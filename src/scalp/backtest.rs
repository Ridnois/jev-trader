//! Scalper backtest: the live entry rules (`engine::evaluate_entry`) and exits replayed
//! on Hyperliquid's historical `revert_interval` candles. No news, no Jev review.
//!
//! Timing: a bar's close is judged when it closes, so the entry fills at the next bar's
//! open (as the live bot does a few hundred ms after the close). The hard stop is checked
//! on every bar's high/low (filled at the stop, or at the open if price gapped through);
//! the time stop exits at the first bar open at or past the deadline. Costs: taker fee
//! and slippage from the config on both legs, an assumed spread, no funding.
//! Output matches the news backtest's files, so the dashboard reads both.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Result, bail};
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};

use super::engine::{self, NewsCtx, Side};
use super::features::{Features, revert_z};
use crate::bot::append_jsonl;
use crate::config::Config;
use crate::market::{self, Candle};
use crate::status::write_atomic;

pub const LABEL: &str = "backtest · scalper rules on historical candles";
/// Hyperliquid serves only the latest 5000 candles per interval.
const MAX_CANDLES: i64 = 5000;
/// Not in the candles: a typical top-of-book spread on these perps, paid half in, half out.
pub const ASSUMED_SPREAD_BPS: f64 = 2.0;

fn bar_ms(cfg: &Config) -> i64 {
    market::interval_ms(&cfg.scalp.revert_interval).unwrap_or(900_000)
}

/// Earliest start the candles allow, after the `revert_bars` warm-up.
pub fn earliest_start(cfg: &Config, now: DateTime<Utc>) -> DateTime<Utc> {
    let back = (MAX_CANDLES - cfg.scalp.revert_bars as i64 - 4) * bar_ms(cfg);
    now - Duration::milliseconds(back)
}

struct Pos {
    side: Side,
    qty: f64,
    entry_px: f64,
    notional: f64,
    entry_fee: f64,
    sl_px: f64,
    init_sl_px: f64,
    sl_bps: f64,
    cost_bps: f64,
    best_px: f64,
    deadline_ms: i64,
}

impl Pos {
    fn unrealized(&self, mark: f64) -> f64 {
        self.side.sign() * (mark - self.entry_px) * self.qty - self.entry_fee
    }
}

fn progress(dir: &Path, stage: u8) {
    std::fs::write(dir.join("progress.json"), json!({ "stage": stage }).to_string()).ok();
}

/// Closed-trade ledger: P&L, fees, and the fills written for the dashboard.
struct Book {
    realized: f64,
    day_pnl: f64,
    fees: f64,
    turnover: f64,
    fills: Vec<Value>,
    trade_bps: Vec<f64>,
    spread_slip_bps: f64,
    fee: f64,
}

impl Book {
    /// Closes at `px` less half the spread and the slippage; returns the round trip's P&L after both fees.
    fn close(&mut self, sym: &str, p: Pos, px: f64, at: DateTime<Utc>, reason: &str) -> f64 {
        let s = p.side.sign();
        let fill_px = px - s * px * self.spread_slip_bps / 1e4;
        let notional = p.qty * fill_px;
        let exit_fee = notional * self.fee;
        let pnl = s * (fill_px - p.entry_px) * p.qty - p.entry_fee - exit_fee;
        self.realized += pnl;
        self.day_pnl += pnl;
        self.fees += exit_fee;
        self.turnover += notional;
        self.trade_bps.push(pnl / p.notional * 1e4);
        self.fills.push(json!({
            "ts": at, "asset": sym, "side": if p.side == Side::Long { "sell" } else { "buy" }, "position": p.side.word(),
            "qty": p.qty, "price": fill_px, "notional": notional, "fee": exit_fee, "reason": reason, "events": [],
            "realized_pnl": pnl, "round_trip_pnl": pnl, "pnl_bps": pnl / p.notional * 1e4,
        }));
        pnl
    }
}

pub struct Outcome {
    pub verdict: &'static str,
    pub markdown: String,
}

/// Replays `[from, to)` (to = now when None) and writes the run into `dir`.
pub async fn run(http: &reqwest::Client, cfg: &Config, from: DateTime<Utc>, to: Option<DateTime<Utc>>, run_id: &str, dir: &Path) -> Result<Outcome> {
    let sc = &cfg.scalp;
    let bar = bar_ms(cfg);
    let now = Utc::now();
    let to = to.unwrap_or(now).min(now);
    let n = sc.revert_bars;
    if from < earliest_start(cfg, now) {
        bail!(
            "Hyperliquid keeps only the last {MAX_CANDLES} {} candles; with the {n}-bar warm-up the earliest start is {}",
            sc.revert_interval,
            earliest_start(cfg, now).format("%Y-%m-%d %H:%M UTC")
        );
    }
    if to - from < Duration::hours(24) {
        bail!("the window needs at least a day");
    }
    std::fs::create_dir_all(dir)?;
    for f in ["equity_strategy.jsonl", "equity_hold.jsonl", "trades_strategy.jsonl"] {
        std::fs::remove_file(dir.join(f)).ok();
    }

    // 1. Candles, with the warm-up before the window.
    progress(dir, 0);
    let start = from.timestamp_millis() - (n as i64 + 2) * bar;
    let end = to.timestamp_millis();
    let mut candles: BTreeMap<String, Vec<Candle>> = BTreeMap::new();
    for a in &cfg.assets {
        // The candle endpoint is slow on big ranges: up to three tries per coin.
        let mut got = market::klines_range(http, cfg.api(), &a.coin, &sc.revert_interval, start, end).await;
        for _ in 0..2 {
            if got.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            got = market::klines_range(http, cfg.api(), &a.coin, &sc.revert_interval, start, end).await;
        }
        let c: Vec<Candle> = got?
            .into_iter()
            .filter(|k| k.is_closed(now.timestamp_millis()) && k.open_time < end)
            .collect();
        if c.len() <= n + 1 {
            bail!("{}: only {} {} candles came back, the rules need more than {}", a.symbol, c.len(), sc.revert_interval, n + 1);
        }
        candles.insert(a.symbol.clone(), c);
    }

    // 2. Replay, bar by bar across every coin.
    progress(dir, 1);
    let from_ms = from.timestamp_millis();
    let mut times: Vec<i64> = candles.values().flat_map(|c| c.iter().map(|k| k.open_time)).filter(|t| *t >= from_ms).collect();
    times.sort_unstable();
    times.dedup();
    let idx: HashMap<&str, HashMap<i64, usize>> =
        candles.iter().map(|(s, c)| (s.as_str(), c.iter().enumerate().map(|(i, k)| (k.open_time, i)).collect())).collect();

    let capital = sc.starting_cash;
    let half_cost = |p: f64| p * (ASSUMED_SPREAD_BPS / 2.0 + sc.slippage_bps) / 1e4;
    let fee = sc.fee_bps / 1e4;
    let mut pos: BTreeMap<String, Pos> = BTreeMap::new();
    let mut last_close: HashMap<String, f64> = HashMap::new();
    let mut cooldown: HashMap<String, i64> = HashMap::new();
    let mut entries_ms: Vec<i64> = Vec::new();
    let (mut day, mut halted) = (None, false);
    let mut equity: Vec<(DateTime<Utc>, f64)> = Vec::new();

    let ts = |ms: i64| DateTime::from_timestamp_millis(ms).unwrap_or(now);
    let mut bk = Book { realized: 0.0, day_pnl: 0.0, fees: 0.0, turnover: 0.0, fills: Vec::new(), trade_bps: Vec::new(), spread_slip_bps: ASSUMED_SPREAD_BPS / 2.0 + sc.slippage_bps, fee };

    for &t in &times {
        // Day roll (UTC): the daily loss limit is on closed-trade P&L.
        let d = ts(t).date_naive();
        if day != Some(d) {
            day = Some(d);
            bk.day_pnl = 0.0;
            halted = false;
        }
        // a. Time stops at this bar's open.
        let due: Vec<String> = pos.iter().filter(|(s, p)| t >= p.deadline_ms && idx[s.as_str()].contains_key(&t)).map(|(s, _)| s.clone()).collect();
        for sym in due {
            let open = candles[&sym][idx[sym.as_str()][&t]].open;
            let p = pos.remove(&sym).expect("open");
            let pnl = bk.close(&sym, p, open, ts(t), "time stop");
            if pnl < 0.0 && sc.cooldown_after_loss_secs > 0 {
                cooldown.insert(sym, t + sc.cooldown_after_loss_secs * 1000);
            }
        }
        if bk.day_pnl < -sc.daily_loss_limit_pct * capital {
            halted = true;
        }
        entries_ms.retain(|e| t - e < 3_600_000);

        // b. Entries at this bar's open, judged on the bar that just closed.
        for (sym, c) in &candles {
            let Some(&i) = idx[sym.as_str()].get(&t) else { continue };
            if i <= n {
                continue;
            }
            let closes: Vec<f64> = c[i - 1 - n..i].iter().map(|k| k.close).collect();
            let Some(z) = revert_z(&closes, n) else { continue };
            let open = c[i].open;
            let f = Features {
                ts_ms: t,
                mid: open,
                bid: open - open * ASSUMED_SPREAD_BPS / 2e4,
                ask: open + open * ASSUMED_SPREAD_BPS / 2e4,
                spread_bps: ASSUMED_SPREAD_BPS,
                history_secs: sc.warmup_secs,
                z_rev: z,
                bars: n + 1,
                bar_ms: c[i - 1].open_time,
                ..Default::default()
            };
            let sides: &[Side] = if sc.shorts { &[Side::Long, Side::Short] } else { &[Side::Long] };
            let Some(ev) = sides.iter().map(|s| engine::evaluate_entry(&f, &NewsCtx::default(), sc, *s)).find(|e| e.ok) else { continue };
            // Risk checks, as live: one trade per coin, free slot, cooldown, hourly cap, daily limit.
            if pos.contains_key(sym) || pos.len() >= sc.max_positions || halted || entries_ms.len() >= sc.max_trades_per_hour {
                continue;
            }
            if cooldown.get(sym).is_some_and(|u| t < *u) {
                continue;
            }
            let marks_equity = capital + bk.realized + pos.iter().map(|(s, p)| p.unrealized(*last_close.get(s).unwrap_or(&p.entry_px))).sum::<f64>();
            let open_notional: f64 = pos.iter().map(|(s, p)| p.qty * last_close.get(s).unwrap_or(&p.entry_px)).sum();
            let usd = (capital * sc.position_pct).min((marks_equity - open_notional).max(0.0));
            if usd < sc.min_trade_usd {
                continue;
            }
            let s = ev.side.sign();
            let px = open + s * half_cost(open);
            let qty = usd / px;
            let entry_fee = usd * fee;
            bk.fees += entry_fee;
            bk.turnover += usd;
            let sl_px = px * (1.0 - s * ev.sl_bps / 1e4);
            entries_ms.push(t);
            bk.fills.push(json!({
                "ts": ts(t), "asset": sym, "side": if ev.side == Side::Long { "buy" } else { "sell" }, "position": ev.side.word(),
                "qty": qty, "price": px, "notional": usd, "fee": entry_fee, "events": [],
                "reason": format!("{sym} stretched {} its {}-day mean: z {:+.2}, fades past {:.2}", if z < 0.0 { "below" } else { "above" }, (n as i64 * bar) / 86_400_000, z, ev.flow_threshold),
            }));
            pos.insert(sym.clone(), Pos { side: ev.side, qty, entry_px: px, notional: usd, entry_fee, sl_px, init_sl_px: sl_px, sl_bps: ev.sl_bps, cost_bps: ev.cost_bps, best_px: px, deadline_ms: t + sc.hold_secs * 1000 });
        }

        // c. Stops inside this bar, then the trail, on its high and low.
        let open_syms: Vec<String> = pos.keys().cloned().collect();
        for sym in open_syms {
            let Some(&i) = idx[sym.as_str()].get(&t) else { continue };
            let k = &candles[&sym][i];
            let p = pos.get_mut(&sym).expect("open");
            let hit = match p.side {
                Side::Long => (k.low <= p.sl_px).then(|| k.open.min(p.sl_px)),
                Side::Short => (k.high >= p.sl_px).then(|| k.open.max(p.sl_px)),
            };
            if let Some(px) = hit {
                let reason = if p.side.sign() * (p.sl_px - p.init_sl_px) > 0.0 { "trailing stop" } else { "stop" };
                let p = pos.remove(&sym).expect("open");
                let pnl = bk.close(&sym, p, px, ts(t + bar - 1), reason);
                if pnl < 0.0 && sc.cooldown_after_loss_secs > 0 {
                    cooldown.insert(sym.clone(), t + sc.cooldown_after_loss_secs * 1000);
                }
                continue;
            }
            p.best_px = match p.side {
                Side::Long => p.best_px.max(k.high),
                Side::Short => p.best_px.min(k.low),
            };
            p.sl_px = engine::trailed_stop(p.side, p.entry_px, p.best_px, p.sl_px, p.sl_bps, p.cost_bps, sc);
        }

        // d. Mark at this bar's close.
        for (sym, c) in &candles {
            if let Some(&i) = idx[sym.as_str()].get(&t) {
                last_close.insert(sym.clone(), c[i].close);
            }
        }
        let eq = capital + bk.realized + pos.iter().map(|(s, p)| p.unrealized(*last_close.get(s).unwrap_or(&p.entry_px))).sum::<f64>();
        equity.push((ts(t + bar - 1), eq));
    }
    // Whatever is still open closes at the last price, so every trade counts.
    let end_t = times.last().copied().unwrap_or(from_ms) + bar - 1;
    for (sym, p) in std::mem::take(&mut pos) {
        let px = *last_close.get(&sym).unwrap_or(&p.entry_px);
        bk.close(&sym, p, px, ts(end_t), "end of window");
    }
    if let Some(last) = equity.last_mut() {
        last.1 = capital + bk.realized;
    }

    // Baseline: an equal share of every coin bought at the window's first open, held.
    let first: HashMap<&str, f64> = candles
        .iter()
        .filter_map(|(s, c)| c.iter().find(|k| k.open_time >= from_ms).map(|k| (s.as_str(), k.open)))
        .collect();
    let share = capital / first.len().max(1) as f64 * (1.0 - fee);
    let mut held: HashMap<&str, f64> = HashMap::new();
    let mut hold: Vec<(DateTime<Utc>, f64)> = Vec::new();
    for &t in &times {
        for (s, c) in &candles {
            if let Some(&i) = idx[s.as_str()].get(&t) {
                held.insert(s.as_str(), c[i].close);
            }
        }
        let v: f64 = first.iter().map(|(s, p0)| share / p0 * held.get(s).unwrap_or(p0)).sum();
        hold.push((ts(t + bar - 1), v));
    }

    // 3. Score and write.
    progress(dir, 2);
    let mut peak = capital;
    let mut mdd = 0.0f64;
    for (_, e) in &equity {
        peak = peak.max(*e);
        mdd = mdd.max(1.0 - e / peak);
    }
    let (fills, trade_bps, fees_paid, turnover, realized) = (bk.fills, bk.trade_bps, bk.fees, bk.turnover, bk.realized);
    let nt = trade_bps.len();
    let mean = trade_bps.iter().sum::<f64>() / nt.max(1) as f64;
    let sd = if nt > 1 { (trade_bps.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (nt - 1) as f64).sqrt() } else { 0.0 };
    let t_stat = if sd > 0.0 { mean / (sd / (nt as f64).sqrt()) } else { 0.0 };
    let wins = trade_bps.iter().filter(|x| **x > 0.0).count();
    let verdict = if nt < 30 {
        "INSUFFICIENT_EVIDENCE"
    } else if t_stat > 2.0 {
        "PAYS_AFTER_FEES"
    } else if t_stat < -2.0 {
        "LOSES_AFTER_FEES"
    } else {
        "UNCLEAR"
    };
    let total_return = (capital + realized) / capital - 1.0;
    let hold_ret = hold.last().map(|h| h.1 / capital - 1.0).unwrap_or(0.0);
    for f in &fills {
        append_jsonl(&dir.join("trades_strategy.jsonl"), f);
    }
    let lines = |v: &[(DateTime<Utc>, f64)]| v.iter().filter_map(|x| serde_json::to_string(x).ok()).map(|l| l + "\n").collect::<String>();
    write_atomic(&dir.join("equity_strategy.jsonl"), lines(&equity).as_bytes())?;
    write_atomic(&dir.join("equity_hold.jsonl"), lines(&hold).as_bytes())?;
    let assumptions = vec![
        format!("Entry at the open of the {} bar after the stretched close; no Jev review, no news tilt.", sc.revert_interval),
        format!("Taker fee {} bps and slippage {} bps each way, plus an assumed {} bps spread.", sc.fee_bps, sc.slippage_bps, ASSUMED_SPREAD_BPS),
        "Funding isn't charged; stops fill at the stop price (or the open if it gapped through).".to_string(),
    ];
    let summary = json!({
        "label": LABEL, "kind": "scalp", "run_id": run_id, "from": from, "to": to, "verdict": verdict,
        "interval": sc.revert_interval, "coins": cfg.assets.iter().map(|a| a.symbol.clone()).collect::<Vec<_>>(),
        "series": { "strategy": "Strategy", "hold": "Hold every coin" },
        "strategy": {
            "total_return": total_return, "max_drawdown": mdd, "hours": (to - from).num_hours(), "fills": fills.len(),
            "trades": nt, "wins": wins, "mean_bps": mean, "t_stat": t_stat, "fees": fees_paid, "turnover": turnover / capital,
        },
        "hold_return": hold_ret,
        "assumptions": assumptions,
    });
    write_atomic(&dir.join("summary.json"), &serde_json::to_vec_pretty(&summary)?)?;
    let mut md = String::new();
    let _ = writeln!(md, "# Scalper backtest {run_id}\n\n*{LABEL} · {} → {} · {} candles · {}*\n", from.format("%Y-%m-%d %H:%M"), to.format("%Y-%m-%d %H:%M"), sc.revert_interval, cfg.assets.iter().map(|a| a.symbol.as_str()).collect::<Vec<_>>().join(", "));
    let _ = writeln!(md, "| Return | Max DD | Trades | Won | Mean per trade | t | Fees | Hold every coin |\n|---|---|---|---|---|---|---|---|");
    let _ = writeln!(md, "| {:+.2}% | {:.2}% | {nt} | {wins} | {mean:+.1} bps | {t_stat:+.2} | ${fees_paid:.2} | {:+.2}% |", total_return * 100.0, mdd * 100.0, hold_ret * 100.0);
    let _ = writeln!(md, "\n**Verdict: {verdict}**\n");
    for a in &assumptions {
        let _ = writeln!(md, "- {a}");
    }
    write_atomic(&dir.join("report.md"), md.as_bytes())?;
    progress(dir, 3);
    Ok(Outcome { verdict, markdown: md })
}
