//! `jev-trader trade monitor`: is the paper run doing what its prediction card said?
//!
//! Three questions per strategy, each answered from files only:
//! 1. **Parity.** The simulator replays the same days on Hyperliquid's candles. Its
//!    trades are matched to the paper trades by coin, side and the bar that decided
//!    them. Trades that exist on one side only mean the backtest and the engine disagree
//!    (or the engine was down), and nothing else on this page can be trusted until that
//!    is explained.
//! 2. **Sequential test.** Paper daily returns against the card: KILL when "no edge"
//!    has become the better explanation, GRADUATE when the predicted edge has, at the
//!    error rates fixed in lab.toml. The card says up front how long this should take.
//! 3. **Drawdown.** Past the 99th-percentile drawdown of the backtest's own bootstrap
//!    for the days elapsed: KILL.
//!
//! The verdict is written, never applied: nothing here starts or stops trading.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::{DailyRow, Ledger, Paths, load_strategies};
use crate::lab::data::HL;
use crate::lab::sim::{self, SimCfg};
use crate::lab::strategy::{self, Aux};
use crate::lab::{LabConfig, gate, stat};

/// A day counts when the engine was up for at least this share of it.
const MIN_UPTIME: f64 = 0.9;

#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    Kill(String),
    Graduate,
    Continue,
}

/// The sequential verdict for one strategy's paper days.
pub fn sequential(rets: &[f64], card: &gate::Card) -> (Call, f64, f64) {
    let llr = stat::sprt_llr(rets, card.mu_daily, card.sd_daily);
    let (lo, hi) = stat::sprt_bounds(card.alpha, card.beta);
    let (mut cum, mut peak, mut dd) = (0.0f64, 0.0f64, 0.0f64);
    for r in rets {
        cum += r;
        peak = peak.max(cum);
        dd = dd.max(peak - cum);
    }
    // The envelope for the nearest horizon at or past the days elapsed.
    let limit = match rets.len() {
        0..=30 => card.dd99[0],
        31..=60 => card.dd99[1],
        _ => card.dd99[2],
    };
    let call = if limit > 0.0 && dd > limit {
        Call::Kill(format!("drawdown {:.2}% is past the {:.2}% the backtest allows in {} days", dd * 100.0, limit * 100.0, rets.len().clamp(30, 90)))
    } else if card.mu_daily <= 0.0 {
        Call::Kill("the card predicts no positive return".into())
    } else if llr <= lo {
        Call::Kill("the sequential test now favours no edge".into())
    } else if llr >= hi {
        Call::Graduate
    } else {
        Call::Continue
    };
    (call, llr, dd)
}

/// −0.0 prints as "-0.00"; an exact zero should not look like a loss.
fn z(x: f64) -> f64 {
    if x == 0.0 { 0.0 } else { x }
}

/// A decision bar worth comparing: the strategy existed for it (its paper history starts at
/// `started`), the engine has processed it, and the simulator has the bar after it to fill on.
fn comparable(decision: i64, started: Option<i64>, last_processed: i64, sim_last_fill: i64, ims: i64) -> bool {
    match started {
        Some(start) => decision >= start && decision <= last_processed && decision + ims <= sim_last_fill,
        None => false,
    }
}

fn side_word(side: i8) -> &'static str {
    if side > 0 { "long" } else { "short" }
}

fn stamp(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|| ms.to_string())
}

/// What `monitor.json` holds: the same verdicts as `monitor.md`, typed, for `lab learn`
/// and the dashboard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Report {
    pub generated: Option<chrono::DateTime<chrono::Utc>>,
    pub started: Option<chrono::DateTime<chrono::Utc>>,
    pub strategies: Vec<StrategyReport>,
    pub catalyst: Vec<CatalystRow>,
    pub catalyst_days: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StrategyReport {
    pub name: String,
    pub status: String,
    pub spec: String,
    pub interval: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
    pub card_mu_bps: f64,
    pub card_sd_bps: f64,
    pub days_if_real: f64,
    pub days_if_none: f64,
    /// Days with the engine up at least 90% of the time, and the ones left out.
    pub paper_days: usize,
    pub partial_days: usize,
    pub mean_bps: f64,
    pub total_pct: f64,
    pub drawdown_pct: f64,
    pub llr: f64,
    pub kill_at: f64,
    pub graduate_at: f64,
    /// CONTINUE | KILL | GRADUATE
    pub call: String,
    #[serde(default)]
    pub call_reason: String,
    /// Full paper days: (UTC date, return).
    pub daily: Vec<(String, f64)>,
    #[serde(default)]
    pub parity: Option<Parity>,
    pub execution: Execution,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Parity {
    pub both: usize,
    pub late: usize,
    pub only_sim: usize,
    pub only_paper: usize,
    /// Closed pairs, paper minus simulator, bps.
    pub mean_diff_bps: Option<f64>,
}

/// What fills really cost on paper against what the lab charges.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Execution {
    pub trades: usize,
    /// Mean price concession against the mid at entry, bps (the model charges `model_side_bps`).
    pub entry_cost_bps: Option<f64>,
    /// The same at signal and time exits.
    pub exit_cost_bps: Option<f64>,
    pub stop_exits: usize,
    /// At stop exits: concession against the mid when the stop fired (the model adds `model_stop_extra_bps`).
    pub stop_cost_bps: Option<f64>,
    pub model_side_bps: f64,
    pub model_stop_extra_bps: f64,
    pub fees_usd: f64,
    pub funding_usd: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CatalystRow {
    pub h_secs: i64,
    pub triggered_n: usize,
    pub triggered_bps: Option<f64>,
    pub all_n: usize,
    pub all_bps: f64,
}

fn mean_opt(x: &[f64]) -> Option<f64> {
    (!x.is_empty()).then(|| stat::mean(x))
}

/// Realised fill costs of one strategy's closed paper trades.
pub fn execution(trades: &[serde_json::Value], lab: &LabConfig) -> Execution {
    let (mut entry, mut exit, mut stop) = (Vec::new(), Vec::new(), Vec::new());
    let (mut fees, mut funding) = (0.0, 0.0);
    for t in trades {
        let (Some(side), Some(epx), Some(emid), Some(xpx), Some(xmid)) = (t["side"].as_f64(), t["entry_px"].as_f64(), t["entry_mid"].as_f64(), t["exit_px"].as_f64(), t["exit_mid"].as_f64()) else { continue };
        fees += t["fees"].as_f64().unwrap_or(0.0);
        funding += t["funding"].as_f64().unwrap_or(0.0);
        if emid > 0.0 {
            entry.push(side * (epx / emid - 1.0) * 1e4);
        }
        if xmid > 0.0 {
            let c = side * (xmid - xpx) / xmid * 1e4;
            if t["reason"] == "stop" { stop.push(c) } else if t["reason"] != "shutdown" { exit.push(c) }
        }
    }
    Execution {
        trades: trades.len(),
        entry_cost_bps: mean_opt(&entry),
        exit_cost_bps: mean_opt(&exit),
        stop_exits: stop.len(),
        stop_cost_bps: mean_opt(&stop),
        model_side_bps: lab.costs.side_bps(""),
        model_stop_extra_bps: lab.costs.stop_extra_bps(),
        fees_usd: fees,
        funding_usd: funding,
    }
}

pub fn run(lab: &LabConfig) -> Result<()> {
    let (report, md) = build(lab)?;
    let paths = Paths::new(&lab.trade.data_dir);
    record_calls(&lab.lineage_path(), &report);
    crate::status::write_atomic(&paths.monitor, md.as_bytes())?;
    crate::status::write_atomic(&paths.monitor.with_extension("json"), &serde_json::to_vec_pretty(&report)?)?;
    println!("{md}");
    Ok(())
}

/// Writes each strategy's KILL or GRADUATE to the lab's lineage, once per strategy and
/// call. Nothing is stopped or started: the lineage only remembers it.
pub fn record_calls(path: &std::path::Path, report: &Report) {
    let known = crate::lab::lineage::load(path);
    for s in &report.strategies {
        let event = match s.call.as_str() {
            "KILL" => "killed",
            "GRADUATE" => "graduated",
            _ => continue,
        };
        let family = s.spec.split(':').next().unwrap_or("").to_string();
        if known.iter().any(|e| e.event == event && e.name == s.name && e.family == family) {
            continue;
        }
        crate::lab::lineage::append(path, &crate::lab::lineage::Event { at: report.generated, event: event.into(), family, name: s.name.clone(), parent: s.parent.clone(), note: format!("after {} paper days: {}", s.paper_days, s.call_reason), ..Default::default() });
    }
}

/// The monitor's verdicts, as a typed report and as the page `monitor.md` shows.
pub fn build(lab: &LabConfig) -> Result<(Report, String)> {
    let paths = Paths::new(&lab.trade.data_dir);
    let strategies = load_strategies(&lab.trade.strategies_dir)?;
    let ledger: Ledger = crate::status::read_json(&paths.ledger).unwrap_or_default();
    let daily: Vec<DailyRow> = crate::status::read_jsonl(&paths.daily);
    let trades: Vec<serde_json::Value> = crate::status::read_jsonl(&paths.trades);
    let mut md = String::new();
    let mut report = Report { generated: Some(chrono::Utc::now()), started: ledger.started, ..Default::default() };
    let _ = writeln!(md, "# v3 paper monitor\n\n*{} · started {}*\n", chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"), ledger.started.map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_else(|| "never".into()));
    if strategies.is_empty() {
        let _ = writeln!(md, "No strategies in {}.", lab.trade.strategies_dir.display());
    }
    for s in &strategies {
        let card = &s.promoted.card;
        let _ = writeln!(md, "## {} ({})\n\n`{}` on {} bars{}\n", s.name, s.promoted.status, card.spec.id(), card.interval, if s.promoted.status == "validated" { "" } else { " · **not validated by the lab: watched, not trusted**" });
        let rows: Vec<&DailyRow> = daily.iter().filter(|r| r.strategy == s.name).collect();
        let full_rows: Vec<&&DailyRow> = rows.iter().filter(|r| r.up_secs as f64 >= MIN_UPTIME * 86_400.0).collect();
        let full: Vec<f64> = full_rows.iter().map(|r| r.ret).collect();
        let (call, llr, dd) = sequential(&full, card);
        let (lo, hi) = stat::sprt_bounds(card.alpha, card.beta);
        let mine: Vec<serde_json::Value> = trades.iter().filter(|t| t["strategy"] == s.name.as_str()).cloned().collect();
        report.strategies.push(StrategyReport {
            name: s.name.clone(),
            status: s.promoted.status.clone(),
            spec: card.spec.id(),
            interval: card.interval.clone(),
            label: s.promoted.label.clone(),
            parent: s.promoted.parent.clone(),
            card_mu_bps: card.mu_daily * 1e4,
            card_sd_bps: card.sd_daily * 1e4,
            days_if_real: card.days_if_real,
            days_if_none: card.days_if_none,
            paper_days: full.len(),
            partial_days: rows.len() - full.len(),
            mean_bps: z(stat::mean(&full) * 1e4),
            total_pct: z(full.iter().sum::<f64>() * 100.0),
            drawdown_pct: dd * 100.0,
            llr: z(llr),
            kill_at: lo,
            graduate_at: hi,
            call: match &call {
                Call::Kill(_) => "KILL",
                Call::Graduate => "GRADUATE",
                Call::Continue => "CONTINUE",
            }
            .into(),
            call_reason: match &call {
                Call::Kill(why) => why.clone(),
                _ => String::new(),
            },
            daily: full_rows.iter().map(|r| (r.day.clone(), r.ret)).collect(),
            parity: None,
            execution: execution(&mine, lab),
        });
        let _ = writeln!(md, "- Card: {:+.2} bps/day expected, sd {:.1} bps; decision expected in about {:.0} days if real, {:.0} if not.", card.mu_daily * 1e4, card.sd_daily * 1e4, card.days_if_real, card.days_if_none);
        let _ = writeln!(md, "- Paper: {} full days ({} partial days left out): mean {:+.2} bps/day, total {:+.2}%, drawdown {:.2}%.", full.len(), rows.len() - full.len(), z(stat::mean(&full) * 1e4), z(full.iter().sum::<f64>() * 100.0), dd * 100.0);
        let _ = writeln!(md, "- Sequential test: {:+.2} (kill at {lo:.2}, graduate at {hi:+.2}).", z(llr));
        let _ = writeln!(md, "- **{}**", match &call {
            Call::Kill(why) => format!("KILL: {why}"),
            Call::Graduate => "GRADUATE: the paper run supports the predicted edge. Going live is still your decision.".into(),
            Call::Continue => "CONTINUE: not enough days to decide either way.".into(),
        });

        // Parity with the simulator over the paper period.
        if ledger.started.is_none() {
            continue;
        }
        let panel = match lab.panel(HL, &card.interval, &lab.universe.coins) {
            Ok(p) => p,
            Err(e) => {
                let _ = writeln!(md, "- Parity: no Hyperliquid candles on file ({e:#}).\n");
                continue;
            }
        };
        let capital = ledger.books.get(&s.name).map(|b| b.capital).unwrap_or(lab.capital);
        let mut cfg = SimCfg::new(capital, lab.costs.clone(), lab.sizing.clone());
        cfg.costs.funding = true;
        let plan = strategy::plan(&card.spec, &panel, &Aux::default())?;
        let run = sim::simulate(&panel, &plan, &cfg);
        // Trades are matched on the bar that DECIDED them, not the bar that filled them: the
        // engine fills on the live book at the open after that bar, which the archive need not
        // have yet. A strategy's paper history starts at its own first bar, so the bars before
        // it arrived are not compared either.
        let ims = panel.interval_ms;
        let started = ledger.first_bar.get(&s.name).copied().or_else(|| ledger.started.map(|d| d.timestamp_millis()));
        let last_bar = ledger.last_bar.get(&s.name).copied().unwrap_or(0);
        let sim_last_fill = panel.time(panel.n.saturating_sub(1));
        let decided = |t: &sim::Trade| panel.time(t.entry_i) - ims;
        let sim_entries: BTreeMap<(String, i8, i64), f64> = run.trades.iter().filter(|t| comparable(decided(t), started, last_bar, sim_last_fill, ims)).map(|t| ((panel.coins[t.coin].clone(), t.side, decided(t)), t.bps())).collect();
        let mut paper: BTreeMap<(String, i8, i64), Option<f64>> = BTreeMap::new();
        for t in trades.iter().filter(|t| t["strategy"] == s.name.as_str()) {
            if let (Some(c), Some(side), Some(bar)) = (t["coin"].as_str(), t["side"].as_i64(), t["entry_bar_ms"].as_i64()) {
                let k = (c.to_string(), side as i8, bar - ims);
                if comparable(k.2, started, last_bar, sim_last_fill, ims) {
                    paper.insert(k, t["pnl_bps"].as_f64());
                }
            }
        }
        if let Some(b) = ledger.books.get(&s.name) {
            for p in b.positions.values() {
                let k = (p.coin.clone(), p.side, p.entry_bar_ms - ims);
                if comparable(k.2, started, last_bar, sim_last_fill, ims) {
                    paper.insert(k, None);
                }
            }
        }
        let (sk, pk): (HashSet<_>, HashSet<_>) = (sim_entries.keys().cloned().collect(), paper.keys().cloned().collect());
        let both: Vec<_> = sk.intersection(&pk).collect();
        let diffs: Vec<f64> = both.iter().filter_map(|k| Some(paper[*k]? - sim_entries[*k])).collect();
        // A paper entry on a bar where the simulator already held that coin on that side is a
        // late join, not a disagreement: a strategy promoted into an open trend starts flat, so
        // its book can only pick up positions the simulator opened days earlier. Everything else
        // unmatched is a real question.
        let sim_held: Vec<(String, i8, i64, i64)> = run.trades.iter().map(|t| (panel.coins[t.coin].clone(), t.side, panel.time(t.entry_i) - ims, if matches!(t.reason, sim::Reason::End) { i64::MAX } else { panel.time(t.exit_i) })).collect();
        let (mut late, mut only_paper): (Vec<_>, Vec<_>) = (Vec::new(), Vec::new());
        for k in pk.difference(&sk).cloned() {
            if sim_held.iter().any(|(c, s, from, to)| *c == k.0 && *s == k.1 && *from < k.2 && k.2 < *to) {
                late.push(k)
            } else {
                only_paper.push(k)
            }
        }
        let only_sim: Vec<_> = sk.difference(&pk).cloned().collect();
        if let Some(r) = report.strategies.last_mut() {
            r.parity = Some(Parity { both: both.len(), late: late.len(), only_sim: only_sim.len(), only_paper: only_paper.len(), mean_diff_bps: mean_opt(&diffs) });
        }
        let _ = writeln!(md, "- Parity: {} entries in both, {} joined a position the simulator already held, {} only in the simulator, {} only on paper{}.", both.len(), late.len(), only_sim.len(), only_paper.len(), if diffs.is_empty() { String::new() } else { format!("; closed pairs differ by {:+.1} bps on average (paper minus simulator)", stat::mean(&diffs)) });
        if only_sim.is_empty() && only_paper.is_empty() && both.is_empty() && late.is_empty() {
            let _ = writeln!(md, "  No bar is comparable yet: the engine needs the close after the bar that decided a trade before the simulator can fill it.");
        } else if !only_sim.is_empty() || !only_paper.is_empty() {
            for k in only_sim.iter().take(5) {
                let _ = writeln!(md, "  simulator only: {} {} decided at {}", side_word(k.1), k.0, stamp(k.2));
            }
            for k in only_paper.iter().take(5) {
                let _ = writeln!(md, "  paper only: {} {} decided at {}", side_word(k.1), k.0, stamp(k.2));
            }
            let _ = writeln!(md, "  Unmatched entries mean the engine was down at a bar close, a quote was missing, or the two disagree. Explain them before trusting the numbers above.");
        } else if !late.is_empty() {
            let _ = writeln!(md, "  Joined late (the paper book started flat while the simulator was already in these): {}.", late.iter().take(6).map(|k| format!("{} {}", side_word(k.1), k.0)).collect::<Vec<_>>().join(", "));
        }
        let _ = writeln!(md);
    }
    // Children next to their parents, on the days both were up.
    let pairs = crate::lab::compare::forward_pairs(&daily, &strategies, lab);
    if !pairs.is_empty() {
        let _ = writeln!(md, "## Parent and child on paper\n\n| Child | Parent | Days | Child | Parent | Difference [95%] |\n|---|---|---|---|---|---|");
        for p in &pairs {
            let _ = writeln!(md, "| {} | {} | {} | {:+.1} | {:+.1} | {:+.1} [{:+.1}, {:+.1}] |", p.challenger, p.incumbent, p.days, p.challenger_bps, p.incumbent_bps, p.diff_bps, p.lo_bps, p.hi_bps);
        }
        let _ = writeln!(md, "\nbps/day on paper. Each keeps its own card and its own sequential test: the child does not inherit the parent's days.\n");
    }
    // The catalyst recorder: forward evidence only.
    let marks: Vec<super::catalyst::Markout> = crate::status::read_jsonl(&paths.catalyst_markouts);
    let _ = writeln!(md, "## Jev catalyst scalper (record mode, no orders)\n");
    if marks.is_empty() {
        let _ = writeln!(md, "No markouts yet. Each scored headline about a coin is marked out at fixed horizons after it arrives.");
    } else {
        let _ = writeln!(md, "Return of trading each headline in the direction Jev read, after the round-trip cost.\n\n| After | Triggered (strong, new, fresh) | n | Every scored headline | n |\n|---|---|---|---|---|");
        for (h, tn, tmean, an, amean) in super::catalyst::summary(&marks) {
            report.catalyst.push(CatalystRow { h_secs: h, triggered_n: tn, triggered_bps: (tn > 0).then_some(tmean), all_n: an, all_bps: amean });
            let _ = writeln!(md, "| {} min | {} | {tn} | {:+.1} bps | {an} |", h / 60, if tn > 0 { format!("{tmean:+.1} bps") } else { "–".into() }, amean);
        }
        let days = marks.iter().map(|m| m.ts.date_naive()).collect::<HashSet<_>>().len();
        report.catalyst_days = days;
        let _ = writeln!(md, "\n{days} day(s) of evidence. Below 30 triggered candidates across 20 days this is an anecdote, not a result.");
    }
    Ok((report, md))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lab::strategy::Spec;
    use crate::stats::Rng;

    fn card(mu: f64) -> gate::Card {
        gate::Card { spec: Spec::new("trend", &[("n", 96.0)]), interval: "1h".into(), mu_daily: mu, sd_daily: 0.01, alpha: 0.05, beta: 0.20, days_if_real: 40.0, days_if_none: 30.0, dd99: [0.08, 0.11, 0.13], issued: chrono::Utc::now(), study: "t".into() }
    }

    #[test]
    fn sequential_calls() {
        let mut rng = Rng::new(4);
        let c = card(0.003); // 30 bps/day on 100 bps sd
        let real: Vec<f64> = (0..150).map(|_| 0.003 + 0.01 * rng.normal()).collect();
        assert_eq!(sequential(&real, &c).0, Call::Graduate);
        let none: Vec<f64> = (0..150).map(|_| 0.01 * rng.normal()).collect();
        assert!(matches!(sequential(&none, &c).0, Call::Kill(_)));
        assert_eq!(sequential(&real[..5], &c).0, Call::Continue);
        assert_eq!(sequential(&[], &c).0, Call::Continue);
        // A crash past the envelope kills at once, whatever the average says.
        let mut crash = vec![0.004; 10];
        crash.extend([-0.05, -0.05]);
        assert!(matches!(sequential(&crash, &c).0, Call::Kill(why) if why.contains("drawdown")));
        // A candidate whose backtest was negative can't graduate.
        assert!(matches!(sequential(&real, &card(-0.001)).0, Call::Kill(_)));
    }

    #[test]
    fn kills_are_remembered_once() {
        let dir = std::env::temp_dir().join(format!("jev-monitor-lineage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("lineage.jsonl");
        let mut r = Report::default();
        r.strategies.push(StrategyReport { name: "g0007".into(), spec: "c4628412b4bd4:n=96".into(), call: "KILL".into(), call_reason: "drawdown".into(), ..Default::default() });
        r.strategies.push(StrategyReport { name: "trend".into(), spec: "trend:n=384".into(), call: "CONTINUE".into(), ..Default::default() });
        record_calls(&path, &r);
        record_calls(&path, &r);
        let ev = crate::lab::lineage::load(&path);
        assert_eq!(ev.len(), 1);
        assert_eq!((ev[0].event.as_str(), ev[0].family.as_str(), ev[0].name.as_str()), ("killed", "c4628412b4bd4", "g0007"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn parity_compares_only_decision_bars_both_sides_could_have() {
        let ims = 3_600_000i64;
        let start = 100 * ims;
        let (last, sim_last) = (110 * ims, 120 * ims);
        // Decided before the strategy arrived, after the engine's last bar, or on a bar the
        // simulator has no following bar to fill on: not comparable.
        assert!(!comparable(start - ims, Some(start), last, sim_last, ims));
        assert!(!comparable(111 * ims, Some(start), last, sim_last, ims));
        assert!(!comparable(120 * ims, Some(start), 130 * ims, sim_last, ims));
        // The edges that are: the strategy's first bar, and the newest bar the simulator can fill.
        assert!(comparable(start, Some(start), last, sim_last, ims));
        assert!(comparable(119 * ims, Some(start), 130 * ims, sim_last, ims));
        // A strategy with no recorded first bar compares nothing.
        assert!(!comparable(start, None, last, sim_last, ims));
    }
}
