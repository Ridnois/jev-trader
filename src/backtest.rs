//! Phase 2: replay cached Jev answers (`events.jsonl`) against historical 1h klines
//! through the same decision path as live trading (`decide` + `execute`). No Jev calls.
//!
//! Point-in-time: an event is visible from `first_seen_at`; a decision at the close
//! of hour t fills at the open of hour t+1 with fees and slippage; momentum uses
//! closed candles only; cooldowns, halts and decay run on simulated time.
//! Results are labelled *backtest · replay of cached events*: the event set holds only
//! what the feeds served while the bot was running (survivorship).

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::bot::append_jsonl;
use crate::breaker;
use crate::config::{Config, RegimeMode};
use crate::decide::{self, AssetInput, Inputs, Variant};
use crate::market::{self, Candle};
use crate::news::NewsEvent;
use crate::paper::State;
use crate::settle::{self, Kind, Outcome};
use crate::stats;
use crate::status::{Paths, load_events, write_atomic};

const HOUR_MS: i64 = 3_600_000;
/// Candles before the window used to warm up momentum / ATR / vol.
const WARMUP_H: i64 = 48;
const CHECKPOINT_EVERY: usize = 200;
pub const LABEL: &str = "backtest · replay of cached events";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub variant: String,
    /// (close time, equity) per hour, oldest first.
    pub equity: Vec<(DateTime<Utc>, f64)>,
    pub fills: usize,
    pub turnover_usd: f64,
    pub fees: f64,
    /// Decision clusters (non-overlapping 24h) and their direction-adjusted 24h returns.
    pub decision_t0: Vec<DateTime<Utc>>,
    pub decision_ret24: Vec<f64>,
}

impl RunResult {
    pub fn returns(&self) -> Vec<f64> {
        self.equity.windows(2).map(|w| w[1].1 / w[0].1 - 1.0).collect()
    }

    pub fn returns_since(&self, from: DateTime<Utc>) -> Vec<f64> {
        self.equity.windows(2).filter(|w| w[0].0 >= from).map(|w| w[1].1 / w[0].1 - 1.0).collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Metrics {
    pub total_return: f64,
    pub max_drawdown: f64,
    pub hours: usize,
    pub fills: usize,
    pub turnover: f64,
    pub fees: f64,
    pub decision_clusters: usize,
    pub decision_hit_rate: Option<f64>,
}

pub fn metrics_of(returns: &[f64], r: &RunResult, from: DateTime<Utc>) -> Metrics {
    let mut eq = 1.0f64;
    let mut peak = 1.0f64;
    let mut mdd = 0.0f64;
    for x in returns {
        eq *= 1.0 + x;
        peak = peak.max(eq);
        mdd = mdd.max(1.0 - eq / peak);
    }
    let avg_eq = stats::mean(&r.equity.iter().map(|e| e.1).collect::<Vec<_>>()).max(1e-9);
    let pts: Vec<(DateTime<Utc>, f64, f64)> = r
        .decision_t0
        .iter()
        .zip(&r.decision_ret24)
        .filter(|(t, _)| **t >= from)
        .map(|(t, x)| (*t, 1.0, *x))
        .collect();
    let clusters = stats::merge_pairs(&pts, Duration::hours(24));
    let hits: Vec<f64> = clusters.iter().map(|c| c.1).collect();
    Metrics {
        total_return: eq - 1.0,
        max_drawdown: mdd,
        hours: returns.len(),
        fills: r.fills,
        turnover: r.turnover_usd / avg_eq,
        fees: r.fees,
        decision_clusters: clusters.len(),
        decision_hit_rate: (!hits.is_empty()).then(|| hits.iter().filter(|x| **x > 0.0).count() as f64 / hits.len() as f64),
    }
}

/// Historical data for one window.
pub struct Market {
    pub candles: BTreeMap<String, Vec<Candle>>,
    /// Hour close times inside [from, to).
    pub hours: Vec<i64>,
}

pub async fn load_market(
    http: &reqwest::Client,
    cfg: &Config,
    dir: &Path,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Market> {
    std::fs::create_dir_all(dir)?;
    let start = from.timestamp_millis() - WARMUP_H * HOUR_MS;
    let end = to.timestamp_millis();
    let mut candles = BTreeMap::new();
    for a in &cfg.assets {
        let cache = dir.join(format!("hl_klines_{}_{}_{}.json", a.coin, start, end));
        let c: Vec<Candle> = match std::fs::read(&cache).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
            Some(c) => c,
            None => {
                let c = market::klines_range(http, cfg.api(), &a.coin, "1h", start, end).await?;
                write_atomic(&cache, &serde_json::to_vec(&c)?)?;
                c
            }
        };
        let now_ms = Utc::now().timestamp_millis();
        let c: Vec<Candle> = c.into_iter().filter(|k| k.is_closed(now_ms)).collect();
        crate::validate::candles(&c).with_context(|| format!("{} candles", a.coin))?;
        candles.insert(a.symbol.clone(), c);
    }
    let first = candles.values().next().context("no assets")?;
    let hours = first
        .iter()
        .map(|c| c.close_time)
        .filter(|t| *t >= from.timestamp_millis() && *t < end)
        .collect();
    Ok(Market { candles, hours })
}

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    key: String,
    step: usize,
    state: State,
    result: RunResult,
    pending: Vec<settle::Record>,
}

/// Replays one variant. Resumable from `dir/checkpoint_<variant>.json`.
pub fn simulate(cfg: &Config, events: &[NewsEvent], m: &Market, variant: Variant, dir: &Path) -> Result<RunResult> {
    let mut cfg = cfg.clone();
    // The regime is not replayed (it needs daily breadth / dominance history).
    cfg.regime.mode = RegimeMode::Off;
    let vname = match variant {
        Variant::Strategy => "strategy",
        Variant::MomentumOnly => "momentum_only",
        Variant::NewsOnly => "news_only",
    }
    .to_string();
    let key = format!("{vname}:{}:{}:{}", m.hours.first().unwrap_or(&0), m.hours.last().unwrap_or(&0), events.len());
    let ck_path = dir.join(format!("checkpoint_{vname}.json"));
    let (mut step, mut state, mut res, mut pending) = match std::fs::read(&ck_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Checkpoint>(&b).ok())
        .filter(|c| c.key == key)
    {
        Some(c) => {
            info!(variant = %vname, step = c.step, "resuming backtest");
            (c.step, c.state, c.result, c.pending)
        }
        None => (
            0,
            State::new(cfg.paper.starting_cash),
            RunResult { variant: vname.clone(), equity: Vec::new(), fills: 0, turnover_usd: 0.0, fees: 0.0, decision_t0: Vec::new(), decision_ret24: Vec::new() },
            Vec::new(),
        ),
    };
    let trades_path = dir.join(format!("trades_{vname}.jsonl"));
    if step == 0 {
        std::fs::remove_file(&trades_path).ok();
    }

    let mut sorted: Vec<&NewsEvent> = events.iter().collect();
    sorted.sort_by_key(|e| e.seen_at());
    let retain = Duration::seconds((cfg.news.retain_hours * 3600.0) as i64);
    let idx: HashMap<&str, HashMap<i64, usize>> = m
        .candles
        .iter()
        .map(|(s, c)| (s.as_str(), c.iter().enumerate().map(|(i, k)| (k.close_time, i)).collect()))
        .collect();

    while step < m.hours.len() {
        let close = m.hours[step];
        let now = DateTime::from_timestamp_millis(close + 1).context("time")?;
        let lo = sorted.partition_point(|e| e.seen_at() < now - retain);
        let hi = sorted.partition_point(|e| e.seen_at() <= now);
        let window: Vec<NewsEvent> = sorted[lo..hi].iter().map(|e| (*e).clone()).collect();

        let mut assets = Vec::new();
        for a in &cfg.assets {
            let c = &m.candles[&a.symbol];
            let mut inp = AssetInput { symbol: a.symbol.clone(), ..Default::default() };
            if let Some(&i) = idx[a.symbol.as_str()].get(&close) {
                let hist = &c[..=i];
                inp.momentum = crate::signals::momentum(hist, cfg.market.momentum_window);
                inp.atr_pct = market::atr_pct(hist, 14);
                inp.mark = Some(c[i].close);
                inp.exec = c.get(i + 1).map(|n| n.open);
            } else {
                inp.review = Some("no candle".into());
            }
            assets.push(inp);
        }
        let marks: HashMap<String, f64> = assets.iter().filter_map(|a| a.mark.map(|p| (a.symbol.clone(), p))).collect();
        let eq = state.equity_verified(&marks);
        let brk = breaker::update(&mut state, eq, now, &cfg.risk);
        let inp = Inputs {
            cfg: &cfg,
            now,
            events: &window,
            assets,
            paused: false,
            flatten: false,
            jev_degraded: None,
            regime: None,
            variant,
            replay: true,
        };
        let mut d = decide::decide(&state, &brk, &inp);
        pending.extend(settle::decision_records(&mut state, &d.assets, &cfg, now, None));
        let fills = decide::execute(&mut state, &mut d, &cfg, now);
        for f in &fills {
            res.fills += 1;
            res.turnover_usd += f.notional;
            res.fees += f.fee;
            append_jsonl(&trades_path, f);
        }
        let ts = DateTime::from_timestamp_millis(close).context("time")?;
        res.equity.push((ts, state.equity(&marks)));
        step += 1;
        if step % CHECKPOINT_EVERY == 0 {
            let ck = Checkpoint { key: key.clone(), step, state: state.clone(), result: res.clone(), pending: pending.clone() };
            write_atomic(&ck_path, &serde_json::to_vec(&ck)?)?;
        }
    }

    // Settle the decisions on the same candles (24h, direction-adjusted).
    let end = DateTime::from_timestamp_millis(m.hours.last().copied().unwrap_or(0) + HOUR_MS).unwrap_or_else(Utc::now);
    for r in pending.iter().filter(|r| r.kind != Kind::Headline) {
        if let Outcome::Settled(s) = settle::settle_one(r, &m.candles[&r.asset], None, &[24], end, None)
            && let Some(h) = s.horizon(24)
        {
            res.decision_t0.push(r.t0);
            res.decision_ret24.push(r.direction as f64 * h.ret);
        }
    }
    std::fs::remove_file(&ck_path).ok();
    for (ts, eq) in &res.equity {
        append_jsonl(&dir.join(format!("equity_{vname}.jsonl")), &(ts, eq));
    }
    Ok(res)
}

/// Buy-and-hold of `weights` from the first hour, with one entry fee
/// (`fees` reported per $10,000 invested).
pub fn buy_and_hold(m: &Market, weights: &[(String, f64)], fee: f64, name: &str) -> RunResult {
    let mut equity = Vec::new();
    for &close in &m.hours {
        let mut v = 0.0;
        for (sym, w) in weights {
            let c = &m.candles[sym];
            let Some(start) = c.iter().find(|k| k.close_time == m.hours[0]) else { continue };
            let Some(now) = c.iter().find(|k| k.close_time == close) else { continue };
            v += w * (1.0 - fee) * now.close / start.close;
        }
        equity.push((DateTime::from_timestamp_millis(close).unwrap_or_default(), v));
    }
    RunResult { variant: name.into(), equity, fills: weights.len(), turnover_usd: 0.0, fees: fee * 10_000.0, decision_t0: Vec::new(), decision_ret24: Vec::new() }
}

#[derive(Debug, Clone, Serialize)]
pub struct Residual {
    pub baseline: String,
    pub ols: Option<stats::Ols>,
    /// alpha ÷ residual sd (not annualised; no Sharpe on residuals).
    pub edge_ratio: Option<f64>,
    /// Share of 4 sub-windows whose alpha has the full-sample sign.
    pub stability: Option<f64>,
}

pub fn residual(y: &[f64], x: &[f64], name: &str) -> Residual {
    let ols = stats::ols_hac(y, x);
    let stability = ols.as_ref().map(|o| {
        let k = y.len() / 4;
        let same = (0..4)
            .filter_map(|i| stats::ols_hac(&y[i * k..(i + 1) * k], &x[i * k..(i + 1) * k]))
            .filter(|w| w.alpha.signum() == o.alpha.signum() && w.alpha != 0.0)
            .count();
        same as f64 / 4.0
    });
    Residual {
        baseline: name.into(),
        edge_ratio: ols.as_ref().filter(|o| o.resid_sd > 0.0).map(|o| o.alpha / o.resid_sd),
        ols,
        stability,
    }
}

/// CTS verdicts.
pub fn verdict(primary: &Residual, alternate: &Residual, hours: usize, clusters: usize) -> &'static str {
    let t = |r: &Residual| r.ols.as_ref().map(|o| o.t_alpha).unwrap_or(0.0);
    if hours < 24 * 7 || clusters < 20 {
        "INSUFFICIENT_EVIDENCE"
    } else if t(primary) > 2.0 && t(alternate) > 2.0 && primary.stability.unwrap_or(0.0) >= 0.75 {
        "RESIDUAL_EDGE"
    } else if t(primary) > 2.0 {
        "RESIDUAL_FRAGILE"
    } else {
        "BASELINE_EXPLAINED"
    }
}

fn parse_day(s: &str) -> Result<DateTime<Utc>> {
    if let Ok(t) = s.parse::<DateTime<Utc>>() {
        return Ok(t);
    }
    let d: NaiveDate = s.parse().with_context(|| format!("bad date '{s}' (use YYYY-MM-DD)"))?;
    Ok(d.and_hms_opt(0, 0, 0).expect("midnight").and_utc())
}

pub fn window(events: &[NewsEvent], from: Option<&str>, to: Option<&str>) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let from = match from {
        Some(s) => parse_day(s)?,
        None => events.iter().map(|e| e.seen_at()).min().context("no cached events: run the bot first or pass --from")?,
    };
    let to = match to {
        Some(s) => parse_day(s)?,
        None => Utc::now() - Duration::hours(1),
    };
    if to <= from + Duration::hours(WARMUP_H / 2) {
        bail!("window {from} .. {to} is too short");
    }
    Ok((from, to))
}

pub struct Backtest {
    pub dir: PathBuf,
    pub markdown: String,
    pub verdict: &'static str,
}

/// `jev-trader backtest`: strategy + ablations + residual-edge verdict.
pub async fn run(http: &reqwest::Client, cfg: &Config, from: Option<&str>, to: Option<&str>, run_id: Option<String>) -> Result<Backtest> {
    let paths = Paths::new(&cfg.data_dir);
    let events = load_events(&paths.events);
    let (from, to) = window(&events, from, to)?;
    let run_id = run_id.unwrap_or_else(|| format!("{}_{}", from.format("%Y%m%d"), to.format("%Y%m%d")));
    let dir = paths.backtests.join(&run_id);
    std::fs::create_dir_all(&dir)?;
    let m = load_market(http, cfg, &paths.backtests.join("klines"), from, to).await?;
    info!(hours = m.hours.len(), events = events.len(), "backtest window loaded");

    let strat = simulate(cfg, &events, &m, Variant::Strategy, &dir)?;
    let mom = simulate(cfg, &events, &m, Variant::MomentumOnly, &dir)?;
    let news = simulate(cfg, &events, &m, Variant::NewsOnly, &dir)?;
    let btc = buy_and_hold(&m, &[(cfg.settle.benchmark.clone(), 1.0)], cfg.paper.fee_rate, "btc_hold");
    let n = cfg.assets.len() as f64;
    let ew: Vec<(String, f64)> = cfg.assets.iter().map(|a| (a.symbol.clone(), 1.0 / n)).collect();
    let basket = buy_and_hold(&m, &ew, cfg.paper.fee_rate, "equal_weight");

    let y = strat.returns();
    let prim = residual(&y, &mom.returns(), "momentum-only (primary, declared)");
    let alt = residual(&y, &btc.returns(), "BTC buy-and-hold (alternate)");
    let sm = metrics_of(&y, &strat, from);
    let v = verdict(&prim, &alt, sm.hours, sm.decision_clusters);

    let mut md = String::new();
    let _ = writeln!(md, "# Backtest {run_id}\n\n*{LABEL} · {} → {} · {} hours · {} cached events.* The event set holds only what the feeds served while the bot was running. The regime gate and the challenge call are not replayed.\n", from.format("%Y-%m-%d %H:%M"), to.format("%Y-%m-%d %H:%M"), m.hours.len(), events.len());
    let _ = writeln!(md, "| Run | Return | Max DD | Fills | Turnover | Fees | Decision clusters | Hit rate (24h) |\n|---|---|---|---|---|---|---|---|");
    for r in [&strat, &mom, &news, &btc, &basket] {
        let mt = metrics_of(&r.returns(), r, from);
        let _ = writeln!(
            md,
            "| {} | {:+.2}% | {:.2}% | {} | {:.1}× | ${:.2} | {} | {} |",
            r.variant,
            mt.total_return * 100.0,
            mt.max_drawdown * 100.0,
            mt.fills,
            mt.turnover,
            mt.fees,
            mt.decision_clusters,
            mt.decision_hit_rate.map(|h| format!("{:.0}%", h * 100.0)).unwrap_or_else(|| "—".into())
        );
    }
    let _ = writeln!(md, "\n## Residual edge: does Jev's news reading add anything beyond momentum?\n");
    for r in [&prim, &alt] {
        match &r.ols {
            Some(o) => {
                let _ = writeln!(
                    md,
                    "- **{}**: alpha {:+.5}/h (HAC t {:+.2}), beta {:.2}, R² {:.2}, residual edge ratio {}, rolling-alpha stability {}",
                    r.baseline,
                    o.alpha,
                    o.t_alpha,
                    o.beta,
                    o.r2,
                    r.edge_ratio.map(|e| format!("{e:+.3}")).unwrap_or_else(|| "—".into()),
                    r.stability.map(|s| format!("{:.0}%", s * 100.0)).unwrap_or_else(|| "—".into())
                );
            }
            None => {
                let _ = writeln!(md, "- **{}**: too few hours for a regression", r.baseline);
            }
        }
    }
    let _ = writeln!(md, "\n**Verdict: {v}**");
    write_atomic(&dir.join("report.md"), md.as_bytes())?;
    let summary = serde_json::json!({
        "label": LABEL, "run_id": run_id, "from": from, "to": to, "verdict": v,
        "strategy": sm, "primary": prim, "alternate": alt,
    });
    write_atomic(&dir.join("summary.json"), &serde_json::to_vec_pretty(&summary)?)?;
    Ok(Backtest { dir, markdown: md, verdict: v })
}

// ---------- 2.5 change review gate ----------

/// FNV-1a over the strategy-relevant config (everything except paths, timing and feeds).
pub fn config_hash(path: &Path) -> Result<String> {
    let v: toml::Value = toml::from_str(&std::fs::read_to_string(path)?)?;
    let flat = flatten(&v);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (k, val) in flat {
        for b in k.bytes().chain(b"=".iter().copied()).chain(val.bytes()).chain(b";".iter().copied()) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    Ok(format!("{h:016x}"))
}

const IGNORED: &[&str] = &["data_dir", "loop_interval_secs", "jev.", "news.feeds", "exchange.", "regime.coingecko_url", "regime.refresh_minutes"];

/// Leaf key → value string, sorted, without ignored keys.
pub fn flatten(v: &toml::Value) -> BTreeMap<String, String> {
    fn walk(prefix: &str, v: &toml::Value, out: &mut BTreeMap<String, String>) {
        match v {
            toml::Value::Table(t) => {
                for (k, x) in t {
                    let p = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                    walk(&p, x, out);
                }
            }
            other => {
                out.insert(prefix.to_string(), other.to_string());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk("", v, &mut out);
    out.retain(|k, _| !IGNORED.iter().any(|i| k == i.trim_end_matches('.') || (i.ends_with('.') && k.starts_with(i))));
    out
}

pub fn changed_params(base: &Path, cand: &Path) -> Result<Vec<String>> {
    let a = flatten(&toml::from_str(&std::fs::read_to_string(base)?)?);
    let b = flatten(&toml::from_str(&std::fs::read_to_string(cand)?)?);
    let mut keys: Vec<String> = a.keys().chain(b.keys()).cloned().collect();
    keys.sort();
    keys.dedup();
    Ok(keys.into_iter().filter(|k| a.get(k) != b.get(k)).collect())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Review {
    pub ts: DateTime<Utc>,
    pub verdict: String,
    pub base: String,
    pub candidate: String,
    pub base_hash: String,
    pub candidate_hash: String,
    pub checks: Vec<(String, bool, String)>,
}

/// `jev-trader backtest --compare base.toml candidate.toml`.
pub async fn compare(
    http: &reqwest::Client,
    base_path: &Path,
    cand_path: &Path,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<(String, String)> {
    let base = Config::load(base_path)?;
    let cand = Config::load(cand_path)?;
    let paths = Paths::new(&base.data_dir);
    let events = load_events(&paths.events);
    let (from, to) = window(&events, from, to)?;
    let dir = paths.backtests.join(format!("compare_{}", Utc::now().format("%Y%m%d_%H%M%S")));
    std::fs::create_dir_all(dir.join("base"))?;
    std::fs::create_dir_all(dir.join("candidate"))?;
    let m = load_market(http, &base, &paths.backtests.join("klines"), from, to).await?;
    let split_i = (m.hours.len() as f64 * 0.7) as usize;
    let oos_from = DateTime::from_timestamp_millis(*m.hours.get(split_i).context("window too short")?).unwrap_or(from);

    let b = simulate(&base, &events, &m, Variant::Strategy, &dir.join("base"))?;
    let c = simulate(&cand, &events, &m, Variant::Strategy, &dir.join("candidate"))?;
    let mom = simulate(&cand, &events, &m, Variant::MomentumOnly, &dir.join("candidate"))?;
    let (rb, rc, rm) = (b.returns_since(oos_from), c.returns_since(oos_from), mom.returns_since(oos_from));
    let (mb, mc) = (metrics_of(&rb, &b, oos_from), metrics_of(&rc, &c, oos_from));

    let mut checks: Vec<(String, bool, String)> = Vec::new();
    checks.push(("sample".into(), mc.decision_clusters >= 20, format!("{} independent decision clusters out-of-sample (need ≥ 20)", mc.decision_clusters)));
    let changed = changed_params(base_path, cand_path)?;
    checks.push(("overfitting".into(), changed.len() <= 2, format!("{} parameters changed: {}", changed.len(), changed.join(", "))));
    // Regime dependency: BTC above/below its 200h SMA (a trend proxy; the full regime is not replayed).
    let btc = &m.candles[&base.settle.benchmark];
    let up: Vec<bool> = m.hours[split_i..]
        .iter()
        .skip(1)
        .map(|t| {
            let i = btc.iter().position(|k| k.close_time == *t).unwrap_or(0);
            let s = &btc[i.saturating_sub(199)..=i];
            btc[i].close > s.iter().map(|k| k.close).sum::<f64>() / s.len() as f64
        })
        .collect();
    let half = |want: bool| -> f64 {
        let (mut eb, mut ec) = (1.0, 1.0);
        for i in 0..rb.len().min(rc.len()).min(up.len()) {
            if up[i] == want {
                eb *= 1.0 + rb[i];
                ec *= 1.0 + rc[i];
            }
        }
        ec - eb
    };
    let (du, dd) = (half(true), half(false));
    checks.push(("regime".into(), du >= 0.0 && dd >= 0.0, format!("candidate − base: {:+.2}% in BTC uptrend hours, {:+.2}% in downtrend hours (200h-SMA proxy)", du * 100.0, dd * 100.0)));
    let gain = mc.total_return - mb.total_return;
    checks.push((
        "costs".into(),
        !(mc.turnover > mb.turnover * 1.2 && gain <= 0.0),
        format!("turnover {:.1}× vs {:.1}×, fees ${:.2} vs ${:.2}, return gain {:+.2}%", mc.turnover, mb.turnover, mc.fees, mb.fees, gain * 100.0),
    ));
    let prim = residual(&rc, &rm, "momentum-only");
    let alt = residual(&rc, &rb, "base config");
    let rv = verdict(&prim, &alt, rc.len(), mc.decision_clusters);
    checks.push(("baseline".into(), rv != "BASELINE_EXPLAINED", format!("candidate vs momentum-only out-of-sample: {rv}")));

    let worse = gain < 0.0 && mc.max_drawdown >= mb.max_drawdown;
    let verdict = if worse {
        "REJECT"
    } else if checks.iter().all(|c| c.1) {
        "PASS"
    } else {
        "REVISE"
    };

    let mut md = String::new();
    let _ = writeln!(md, "# Change review: {} → {}\n\n*{LABEL} · judged only on the last 30% ({} → {}, {} hours).*\n", base_path.display(), cand_path.display(), oos_from.format("%Y-%m-%d %H:%M"), to.format("%Y-%m-%d %H:%M"), rc.len());
    let _ = writeln!(md, "| | Return | Max DD | Turnover | Decision clusters | Hit rate |\n|---|---|---|---|---|---|");
    for (n, x) in [("base", &mb), ("candidate", &mc)] {
        let _ = writeln!(md, "| {n} | {:+.2}% | {:.2}% | {:.1}× | {} | {} |", x.total_return * 100.0, x.max_drawdown * 100.0, x.turnover, x.decision_clusters, x.decision_hit_rate.map(|h| format!("{:.0}%", h * 100.0)).unwrap_or_else(|| "—".into()));
    }
    let _ = writeln!(md, "\n| Check | Result | Detail |\n|---|---|---|");
    for (n, ok, d) in &checks {
        let _ = writeln!(md, "| {n} | {} | {d} |", if *ok { "pass" } else { "**fail**" });
    }
    let _ = writeln!(md, "\n## Verdict: **{verdict}**");
    if verdict == "REJECT" {
        let _ = writeln!(md, "\n> REJECT: the candidate did worse out-of-sample with no drawdown benefit. You can still apply it; `jev-trader report` will record that it was applied against this verdict.");
    }
    write_atomic(&dir.join("compare.md"), md.as_bytes())?;
    let review = Review {
        ts: Utc::now(),
        verdict: verdict.into(),
        base: base_path.display().to_string(),
        candidate: cand_path.display().to_string(),
        base_hash: config_hash(base_path)?,
        candidate_hash: config_hash(cand_path)?,
        checks,
    };
    append_jsonl(&paths.reviews, &review);
    Ok((verdict.to_string(), md))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::test_candle;

    #[test]
    fn verdicts() {
        let mut rng = stats::Rng::new(9);
        let x: Vec<f64> = (0..500).map(|_| rng.normal() * 0.01).collect();
        let explained: Vec<f64> = x.iter().map(|v| v * 0.8).collect();
        let edge: Vec<f64> = x.iter().map(|v| 0.002 + v * 0.5 + rng.normal() * 0.001).collect();
        let z: Vec<f64> = (0..500).map(|_| rng.normal() * 0.01).collect();
        assert_eq!(verdict(&residual(&explained, &x, "p"), &residual(&explained, &z, "a"), 500, 30), "BASELINE_EXPLAINED");
        assert_eq!(verdict(&residual(&edge, &x, "p"), &residual(&edge, &z, "a"), 500, 30), "RESIDUAL_EDGE");
        assert_eq!(verdict(&residual(&edge, &x, "p"), &residual(&edge, &z, "a"), 500, 3), "INSUFFICIENT_EVIDENCE");
    }

    #[test]
    fn flatten_ignores_paths_and_counts_changes() {
        let a: toml::Value = toml::from_str("data_dir = 'a'\n[risk]\nx = 1\ny = 2\n[jev]\nmodel='m'").unwrap();
        let b: toml::Value = toml::from_str("data_dir = 'b'\n[risk]\nx = 1\ny = 3\n[jev]\nmodel='n'").unwrap();
        let (fa, fb) = (flatten(&a), flatten(&b));
        let diff: Vec<_> = fa.keys().filter(|k| fa.get(*k) != fb.get(*k)).collect();
        assert_eq!(diff, vec!["risk.y"]);
    }

    #[test]
    fn replay_is_point_in_time_and_trades() {
        let cfg = crate::decide::tests::cfg();
        let dir = std::env::temp_dir().join(format!("jev-bt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Rising market, 120 hours.
        let mut candles = BTreeMap::new();
        for a in &cfg.assets {
            candles.insert(a.symbol.clone(), (0..120).map(|i| test_candle(i, 100.0 * 1.002f64.powi(i as i32))).collect::<Vec<_>>());
        }
        let hours: Vec<i64> = candles["BTC"].iter().skip(30).map(|c| c.close_time).collect();
        let m = Market { candles, hours };
        let t = |h: i64| DateTime::from_timestamp_millis(h * HOUR_MS + 1).unwrap();
        let mut e = NewsEvent {
            id: "e".into(),
            title: "t".into(),
            source: "s".into(),
            published: t(40),
            analyzed_at: t(40),
            asset: "BTC".into(),
            asset_conf: 1.0,
            event: "macro".into(),
            event_conf: 0.9,
            sentiment: 1.0,
            sentiment_conf: 1.0,
            new_info: 1.0,
            model: "jev".into(),
            first_seen_at: Some(t(40) + Duration::minutes(10)),
            cluster_id: None,
            tokens_in: None,
            tokens_out: None,
        };
        e.published = t(40);
        let r = simulate(&cfg, &[e], &m, Variant::Strategy, &dir).unwrap();
        let trades: Vec<crate::paper::Fill> = crate::status::read_jsonl(&dir.join("trades_strategy.jsonl"));
        assert!(!trades.is_empty());
        // First buy only after the event became visible (seen 40h+10m → decided at the 41h close).
        assert!(trades[0].ts >= t(41) - Duration::milliseconds(1), "{}", trades[0].ts);
        assert_eq!(r.equity.len(), m.hours.len());
        let mom = simulate(&cfg, &[], &m, Variant::MomentumOnly, &dir).unwrap();
        assert!(mom.fills > 0, "momentum-only trades without news");
        std::fs::remove_dir_all(dir).ok();
    }
}
