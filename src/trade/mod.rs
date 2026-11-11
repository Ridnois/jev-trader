//! `jev-trader trade`: the v3 paper trader. It runs the specs the lab handed over
//! (`strategies/*.json`, written by `lab promote`) through the same rules the lab
//! simulated: the plan is recomputed at every bar close on Hyperliquid's own candles
//! and acted on at the live book; stops are watched every second.
//!
//! Paper only. A spec earns real orders by graduating in `trade monitor`, and arming
//! them stays a human decision.
//!
//! Files in the trade data dir: `ledger.json` (books and open positions),
//! `trades.jsonl` (round trips), `daily.jsonl` (one row per strategy per UTC day:
//! what `monitor` tests), `status.json`, `maker_exits.jsonl` (H-X01), and
//! `ticks/<coin>/<day>.csv` (1-second rows).

pub mod book;
pub mod monitor;

use std::collections::{BTreeMap, HashMap};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::bot::append_jsonl;
use crate::lab::data::{self, HL};
use crate::lab::panel::DAY_MS;
use crate::lab::sim::SimCfg;
use crate::lab::strategy::{self, Aux};
use crate::lab::{LabConfig, Promoted, probe};
use crate::market::interval_ms;
use crate::scalp::features::AssetMarket;
use crate::scalp::{FeedHealth, Markets, hl_feed_task};
use crate::status::write_atomic;
use book::{Book, Closed, Event, Quote};

#[derive(Subcommand)]
pub enum TradeCmd {
    /// Run the paper trader (Ctrl+C to stop; open paper positions are kept and resumed).
    Run {
        /// Process the latest closed bar of every strategy once, then exit.
        #[arg(long)]
        once: bool,
    },
    /// Judge the paper run against each strategy's prediction card: parity with the
    /// simulator, the sequential test, and the drawdown envelope.
    Monitor,
}

/// A strategy on file with its name (the file stem).
pub struct Loaded {
    pub name: String,
    pub promoted: Promoted,
}

pub fn load_strategies(dir: &Path) -> Result<Vec<Loaded>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = e.path();
        if path.extension().is_some_and(|x| x == "json") {
            let promoted: Promoted = serde_json::from_slice(&std::fs::read(&path)?).with_context(|| format!("parsing {}", path.display()))?;
            out.push(Loaded { name: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), promoted });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    pub started: Option<DateTime<Utc>>,
    pub books: BTreeMap<String, Book>,
    /// Open time of the last bar each strategy has acted on.
    #[serde(default)]
    pub last_bar: BTreeMap<String, i64>,
    /// UTC day number being accumulated, each strategy's equity when it began, and
    /// the seconds the engine was up during it.
    #[serde(default)]
    pub day: i64,
    #[serde(default)]
    pub day_start_equity: BTreeMap<String, f64>,
    #[serde(default)]
    pub day_up_secs: u64,
}

/// One line of `daily.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyRow {
    pub strategy: String,
    pub day: String,
    /// Mark-to-market P&L of the day ÷ the strategy's capital.
    pub ret: f64,
    pub up_secs: u64,
}

#[derive(Serialize)]
struct TradeRow<'a> {
    strategy: &'a str,
    #[serde(flatten)]
    trade: &'a Closed,
}

/// A time-stop exit whose passive alternative is still being watched (H-X01).
struct MakerWatch {
    strategy: String,
    coin: String,
    side: i8,
    /// What the taker exit got, and the touch a resting order would have sat at.
    taker_px: f64,
    passive_px: f64,
    at_sec: i64,
}

pub struct Paths {
    pub ledger: PathBuf,
    pub trades: PathBuf,
    pub daily: PathBuf,
    pub status: PathBuf,
    pub maker: PathBuf,
    pub ticks: PathBuf,
    pub monitor: PathBuf,
}

impl Paths {
    pub fn new(dir: &Path) -> Self {
        Self { ledger: dir.join("ledger.json"), trades: dir.join("trades.jsonl"), daily: dir.join("daily.jsonl"), status: dir.join("status.json"), maker: dir.join("maker_exits.jsonl"), ticks: dir.join("ticks"), monitor: dir.join("monitor.md") }
    }
}

fn day_name(day: i64) -> String {
    DateTime::from_timestamp_millis(day * DAY_MS).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

fn quote_of(m: &HashMap<String, AssetMarket>, coin: &str, now_ms: i64) -> Option<Quote> {
    let a = m.get(coin)?;
    let q = Quote { bid: a.bid, ask: a.ask };
    (q.ok() && now_ms - a.last_book_ms < 15_000).then_some(q)
}

pub async fn run(cmd: TradeCmd, lab: &LabConfig, http: &reqwest::Client, hl_api: &'static str, ws_url: &'static str, jev_cfg: &crate::config::JevConfig) -> Result<()> {
    match cmd {
        TradeCmd::Monitor => monitor::run(lab),
        TradeCmd::Run { once } => engine(lab, http, hl_api, ws_url, jev_cfg, once).await,
    }
}

async fn engine(lab: &LabConfig, http: &reqwest::Client, hl_api: &'static str, ws_url: &'static str, jev_cfg: &crate::config::JevConfig, once: bool) -> Result<()> {
    let tc = &lab.trade;
    let strategies = load_strategies(&tc.strategies_dir)?;
    if strategies.is_empty() {
        bail!("no strategies in {}: run `jev-trader lab validate` and then `lab promote` first", tc.strategies_dir.display());
    }
    std::fs::create_dir_all(&tc.data_dir)?;
    crate::status::acquire_pid_lock(&tc.data_dir.join("trade.pid"))?;
    let paths = Paths::new(&tc.data_dir);
    let coins = lab.universe.coins.clone();
    let ar = lab.archive();
    let share = lab.capital / strategies.len() as f64;
    let cfg = SimCfg::new(share, lab.costs.clone(), lab.sizing.clone());
    let mut ledger: Ledger = crate::status::read_json(&paths.ledger).unwrap_or_default();
    ledger.started.get_or_insert_with(Utc::now);
    for s in &strategies {
        ledger.books.entry(s.name.clone()).or_insert_with(|| Book::new(share));
        info!("strategy {} ({}): {} on {} bars · expects {:+.2} bps/day", s.name, s.promoted.status, s.promoted.card.spec.id(), s.promoted.card.interval, s.promoted.card.mu_daily * 1e4);
    }
    let live_probes: Vec<probe::Probe> = tc.live_probes.iter().map(|p| probe::Probe::load(p)).collect::<Result<_>>()?;
    let jev = if live_probes.is_empty() { None } else { Some(crate::jev::JevClient::from_env(&jev_cfg.model, Duration::from_millis(jev_cfg.timeout_ms.max(8000)))?) };
    let mut intervals: Vec<String> = strategies.iter().map(|s| s.promoted.card.interval.clone()).chain(live_probes.iter().map(|p| p.interval.clone())).collect();
    intervals.sort();
    intervals.dedup();

    let markets: Markets = Arc::new(Mutex::new(HashMap::new()));
    let health = Arc::new(Mutex::new(FeedHealth::default()));
    tokio::spawn(hl_feed_task(ws_url, coins.iter().map(|c| (c.clone(), c.clone())).collect(), markets.clone(), health.clone()));

    let mut bar_tries: HashMap<String, (i64, u32)> = HashMap::new();
    let mut bar_done: HashMap<String, i64> = HashMap::new();
    let mut watches: Vec<MakerWatch> = Vec::new();
    let mut flushed: HashMap<String, i64> = HashMap::new();
    let (mut last_tick_ms, mut last_save_ms, mut last_flush_ms) = (Utc::now().timestamp_millis(), 0i64, 0i64);
    let started_ms = last_tick_ms;
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    info!("v3 paper trader up: {} strategies, {} coins, ${share:.0} each · data in {}", strategies.len(), coins.len(), tc.data_dir.display());

    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = tokio::signal::ctrl_c() => {
                write_atomic(&paths.ledger, &serde_json::to_vec_pretty(&ledger)?)?;
                info!("stopped; {} open paper position(s) are kept in the ledger and resumed on restart", ledger.books.values().map(|b| b.positions.len()).sum::<usize>());
                return Ok(());
            }
        }
        let now = Utc::now();
        let now_ms = now.timestamp_millis();
        let dt_h = (now_ms - last_tick_ms).clamp(0, 60_000) as f64 / 3_600_000.0;
        let up = ((now_ms - last_tick_ms).clamp(0, 5_000) as f64 / 1000.0).round() as u64;
        last_tick_ms = now_ms;
        let mut dirty = false;

        // Quotes, funding rates, stops.
        let (quotes, funding): (HashMap<String, Quote>, HashMap<String, f64>) = {
            let m = markets.lock().unwrap();
            (coins.iter().filter_map(|c| quote_of(&m, c, now_ms).map(|q| (c.clone(), q))).collect(), m.iter().map(|(c, a)| (c.clone(), a.funding_1h)).collect())
        };
        for s in &strategies {
            let book = ledger.books.get_mut(&s.name).expect("book");
            let held: Vec<String> = book.positions.keys().cloned().collect();
            for coin in held {
                let Some(q) = quotes.get(&coin).copied() else { continue };
                book.accrue_funding(&coin, funding.get(&coin).copied().unwrap_or(0.0), q.mid(), dt_h);
                if let Some(t) = book.on_quote(&coin, q, &cfg, now_ms) {
                    warn!("{} {} {} stopped at {:.6}: {:+.2} ({:+.1} bps)", s.name, if t.side > 0 { "long" } else { "short" }, coin, t.exit_px, t.pnl, t.pnl_bps);
                    append_jsonl(&paths.trades, &TradeRow { strategy: &s.name, trade: &t });
                    dirty = true;
                }
            }
        }

        // Day roll: one row per strategy for the day that just ended.
        let day = now_ms.div_euclid(DAY_MS);
        let mid = |c: &str| quotes.get(c).map(Quote::mid);
        if ledger.day != day {
            if ledger.day != 0 {
                for s in &strategies {
                    let book = &ledger.books[&s.name];
                    let eq = book.equity(mid);
                    let start = ledger.day_start_equity.get(&s.name).copied().unwrap_or(eq);
                    append_jsonl(&paths.daily, &DailyRow { strategy: s.name.clone(), day: day_name(ledger.day), ret: (eq - start) / book.capital, up_secs: ledger.day_up_secs });
                }
            }
            ledger.day = day;
            ledger.day_up_secs = 0;
            ledger.day_start_equity = strategies.iter().map(|s| (s.name.clone(), ledger.books[&s.name].equity(mid))).collect();
            dirty = true;
        }
        ledger.day_up_secs += up;

        // Bars: a few seconds after each close, bring the candles in and decide. Right
        // after start-up, wait for the price feed first (at most a minute).
        let feed_ready = quotes.len() * 10 >= coins.len() * 8 || now_ms - started_ms > 60_000;
        for interval in intervals.iter().filter(|_| feed_ready) {
            let ims = interval_ms(interval).context("bad strategy interval")?;
            let closed_open = (now_ms.div_euclid(ims) - 1) * ims;
            if bar_done.get(interval).is_some_and(|t| *t >= closed_open) || now_ms < closed_open + ims + 4_000 {
                continue;
            }
            let tries = bar_tries.entry(interval.clone()).or_insert((closed_open, 0));
            if tries.0 != closed_open {
                *tries = (closed_open, 0);
            }
            // Retry every 5 seconds until most coins have the bar; give up after a minute.
            if tries.1 > 0 && (now_ms - (closed_open + ims + 4_000)) / 5_000 < tries.1 as i64 {
                continue;
            }
            tries.1 += 1;
            let have = data::hl_recent(http, &ar, hl_api, &coins, interval, 8, closed_open).await;
            if have * 10 < coins.len() * 8 && tries.1 < 12 {
                continue;
            }
            bar_done.insert(interval.clone(), closed_open);
            if have == 0 {
                warn!("{interval} bar {closed_open}: no candles arrived; skipped");
                continue;
            }
            let panel = match lab.panel(HL, interval, &coins) {
                Ok(p) => p,
                Err(e) => {
                    warn!("{interval}: cannot build the panel: {e:#}");
                    continue;
                }
            };
            if panel.time(panel.n.saturating_sub(1)) != closed_open {
                warn!("{interval}: newest bar on file is not the one that just closed; skipped");
                continue;
            }
            let now_ms = Utc::now().timestamp_millis();
            let quotes: HashMap<String, Quote> = {
                let m = markets.lock().unwrap();
                coins.iter().filter_map(|c| quote_of(&m, c, now_ms).map(|q| (c.clone(), q))).collect()
            };
            for s in strategies.iter().filter(|s| &s.promoted.card.interval == interval) {
                if ledger.last_bar.get(&s.name).is_some_and(|t| *t >= closed_open) {
                    continue;
                }
                let plan = match strategy::plan(&s.promoted.card.spec, &panel, &Aux::default()) {
                    Ok(p) => p,
                    Err(e) => {
                        warn!("{}: {e:#}", s.name);
                        continue;
                    }
                };
                let book = ledger.books.get_mut(&s.name).expect("book");
                let quote = |c: usize| quotes.get(&panel.coins[c]).copied();
                for ev in book.on_bar(&panel, &plan, &cfg, &quote, now_ms) {
                    match ev {
                        Event::Opened(p) => info!("{} open {} {} ${:.0} @ {:.6}{}", s.name, if p.side > 0 { "long" } else { "short" }, p.coin, p.notional, p.entry_px, p.stop_px.map(|x| format!(" · stop {x:.6}")).unwrap_or_default()),
                        Event::Extended(c) => info!("{} keeps {c}", s.name),
                        Event::Closed(t) => {
                            info!("{} close {} {} @ {:.6} ({}): {:+.2} ({:+.1} bps)", s.name, if t.side > 0 { "long" } else { "short" }, t.coin, t.exit_px, t.reason, t.pnl, t.pnl_bps);
                            append_jsonl(&paths.trades, &TradeRow { strategy: &s.name, trade: &t });
                            if let (true, Some(q)) = (t.reason == "time", quotes.get(&t.coin)) {
                                watches.push(MakerWatch { strategy: s.name.clone(), coin: t.coin.clone(), side: t.side, taker_px: t.exit_px, passive_px: if t.side > 0 { q.ask } else { q.bid }, at_sec: now_ms / 1000 });
                            }
                        }
                    }
                }
                ledger.last_bar.insert(s.name.clone(), closed_open);
                dirty = true;
            }
            if let Some(jev) = &jev {
                for pr in live_probes.iter().filter(|p| &p.interval == interval) {
                    match probe::ask_live(&panel, pr, &lab.data_dir, jev).await {
                        Ok(n) if n > 0 => info!("probe {}: asked Jev about {n} coin(s)", pr.name),
                        Ok(_) => {}
                        Err(e) => warn!("probe {}: {e:#}", pr.name),
                    }
                }
            }
        }

        // H-X01: five minutes after a time-stop exit, would a resting order have filled?
        let now_sec = now_ms / 1000;
        if watches.iter().any(|w| now_sec - w.at_sec >= 300) {
            let m = markets.lock().unwrap();
            watches.retain(|w| {
                if now_sec - w.at_sec < 300 {
                    return true;
                }
                if let Some(a) = m.get(&w.coin) {
                    // A long exits by selling: its resting ask fills when someone buys above it.
                    let filled = a.traded_through(w.passive_px, w.side < 0, w.at_sec);
                    let later = (a.bid + a.ask) / 2.0;
                    append_jsonl(&paths.maker, &serde_json::json!({
                        "ts": now, "strategy": w.strategy, "coin": w.coin, "side": w.side, "taker_px": w.taker_px, "passive_px": w.passive_px,
                        "filled_in_5m": filled, "mid_after_5m": later,
                        "price_gain_bps_if_filled": w.side as f64 * (w.passive_px / w.taker_px - 1.0) * 1e4,
                        "drift_bps_if_missed": w.side as f64 * (later / w.taker_px - 1.0) * 1e4,
                    }));
                }
                false
            });
        }

        // 1-second rows, then forget what is written.
        if now_ms - last_flush_ms >= 10_000 {
            last_flush_ms = now_ms;
            let mut m = markets.lock().unwrap();
            for (coin, a) in m.iter_mut() {
                if tc.record_ticks {
                    let rows = a.seconds(flushed.get(coin).copied().unwrap_or(now_sec - 11), now_sec);
                    if let Some(last) = rows.last().map(|r| r.0) {
                        flushed.insert(coin.clone(), last);
                        let dir = paths.ticks.join(coin);
                        let mut by_day: BTreeMap<i64, String> = BTreeMap::new();
                        for (sec, mid, lo, hi, buy, sell, n) in rows {
                            by_day.entry(sec.div_euclid(86_400)).or_default().push_str(&format!("{sec},{mid},{lo},{hi},{buy:.2},{sell:.2},{n}\n"));
                        }
                        if std::fs::create_dir_all(&dir).is_ok() {
                            for (d, text) in by_day {
                                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(format!("{}.csv", day_name(d)))) {
                                    let _ = f.write_all(text.as_bytes());
                                }
                            }
                        }
                    }
                }
                a.drop_before(now_sec - 900);
            }
        }

        if dirty || now_ms - last_save_ms >= 30_000 {
            last_save_ms = now_ms;
            write_atomic(&paths.ledger, &serde_json::to_vec_pretty(&ledger)?)?;
            let h = health.lock().unwrap().clone();
            let books: Vec<serde_json::Value> = strategies.iter().map(|s| {
                let b = &ledger.books[&s.name];
                serde_json::json!({
                    "strategy": s.name, "status": s.promoted.status, "spec": s.promoted.card.spec.id(), "interval": s.promoted.card.interval,
                    "capital": b.capital, "equity_change": b.equity(mid), "realized": b.realized, "fees": b.fees, "funding": b.funding,
                    "at_risk": b.at_risk(), "positions": b.positions.values().collect::<Vec<_>>(),
                })
            }).collect();
            write_atomic(&paths.status, &serde_json::to_vec_pretty(&serde_json::json!({
                "ts": now, "pid": std::process::id(), "venue": "hyperliquid-paper", "feed": h, "quotes": quotes.len(), "coins": coins.len(), "books": books,
            }))?)?;
        }
        if once && intervals.iter().all(|i| bar_done.contains_key(i)) {
            write_atomic(&paths.ledger, &serde_json::to_vec_pretty(&ledger)?)?;
            info!("--once: latest bar of every strategy processed");
            return Ok(());
        }
        if once && now_ms - started_ms > 180_000 {
            bail!("--once: no bar could be processed in 3 minutes (feed or candles unavailable)");
        }
    }
}
