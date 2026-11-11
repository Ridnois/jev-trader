//! `jev-trader trade monitor`: is the paper run doing what its prediction card said?
//!
//! Three questions per strategy, each answered from files only:
//! 1. **Parity.** The simulator replays the same days on Hyperliquid's candles. Its
//!    trades are matched to the paper trades by coin, side and entry bar. Trades that
//!    exist on one side only mean the backtest and the engine disagree (or the engine
//!    was down), and nothing else on this page can be trusted until that is explained.
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
        Call::Kill(format!("drawdown {:.2}% is past the {:.2}% the backtest allows in {} days", dd * 100.0, limit * 100.0, rets.len().max(30).min(90)))
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

pub fn run(lab: &LabConfig) -> Result<()> {
    let paths = Paths::new(&lab.trade.data_dir);
    let strategies = load_strategies(&lab.trade.strategies_dir)?;
    let ledger: Ledger = crate::status::read_json(&paths.ledger).unwrap_or_default();
    let daily: Vec<DailyRow> = crate::status::read_jsonl(&paths.daily);
    let trades: Vec<serde_json::Value> = crate::status::read_jsonl(&paths.trades);
    let mut md = String::new();
    let _ = writeln!(md, "# v3 paper monitor\n\n*{} · started {}*\n", chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"), ledger.started.map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_else(|| "never".into()));
    if strategies.is_empty() {
        let _ = writeln!(md, "No strategies in {}.", lab.trade.strategies_dir.display());
    }
    for s in &strategies {
        let card = &s.promoted.card;
        let _ = writeln!(md, "## {} ({})\n\n`{}` on {} bars{}\n", s.name, s.promoted.status, card.spec.id(), card.interval, if s.promoted.status == "validated" { "" } else { " · **not validated by the lab: watched, not trusted**" });
        let rows: Vec<&DailyRow> = daily.iter().filter(|r| r.strategy == s.name).collect();
        let full: Vec<f64> = rows.iter().filter(|r| r.up_secs as f64 >= MIN_UPTIME * 86_400.0).map(|r| r.ret).collect();
        let (call, llr, dd) = sequential(&full, card);
        let (lo, hi) = stat::sprt_bounds(card.alpha, card.beta);
        let _ = writeln!(md, "- Card: {:+.2} bps/day expected, sd {:.1} bps; decision expected in about {:.0} days if real, {:.0} if not.", card.mu_daily * 1e4, card.sd_daily * 1e4, card.days_if_real, card.days_if_none);
        let _ = writeln!(md, "- Paper: {} full days ({} partial days left out): mean {:+.2} bps/day, total {:+.2}%, drawdown {:.2}%.", full.len(), rows.len() - full.len(), z(stat::mean(&full) * 1e4), z(full.iter().sum::<f64>() * 100.0), dd * 100.0);
        let _ = writeln!(md, "- Sequential test: {:+.2} (kill at {lo:.2}, graduate at {hi:+.2}).", z(llr));
        let _ = writeln!(md, "- **{}**", match &call {
            Call::Kill(why) => format!("KILL: {why}"),
            Call::Graduate => "GRADUATE: the paper run supports the predicted edge. Going live is still your decision.".into(),
            Call::Continue => "CONTINUE: not enough days to decide either way.".into(),
        });

        // Parity with the simulator over the paper period.
        let Some(start) = ledger.started.map(|d| d.timestamp_millis()) else { continue };
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
        let last_bar = ledger.last_bar.get(&s.name).copied().unwrap_or(0);
        // Entries the engine could have made: bars opened after it started, up to the last one it processed.
        let sim_entries: BTreeMap<(String, i8, i64), f64> = run.trades.iter().filter(|t| panel.time(t.entry_i) > start && panel.time(t.entry_i) <= last_bar + panel.interval_ms).map(|t| ((panel.coins[t.coin].clone(), t.side, panel.time(t.entry_i)), t.bps())).collect();
        let mut paper: BTreeMap<(String, i8, i64), Option<f64>> = BTreeMap::new();
        for t in trades.iter().filter(|t| t["strategy"] == s.name.as_str()) {
            if let (Some(c), Some(side), Some(bar)) = (t["coin"].as_str(), t["side"].as_i64(), t["entry_bar_ms"].as_i64()) {
                paper.insert((c.to_string(), side as i8, bar), t["pnl_bps"].as_f64());
            }
        }
        if let Some(b) = ledger.books.get(&s.name) {
            for p in b.positions.values() {
                paper.insert((p.coin.clone(), p.side, p.entry_bar_ms), None);
            }
        }
        let (sk, pk): (HashSet<_>, HashSet<_>) = (sim_entries.keys().cloned().collect(), paper.keys().cloned().collect());
        let both: Vec<_> = sk.intersection(&pk).collect();
        let diffs: Vec<f64> = both.iter().filter_map(|k| Some(paper[*k]? - sim_entries[*k])).collect();
        let _ = writeln!(md, "- Parity: {} entries in both, {} only in the simulator, {} only on paper{}.", both.len(), sk.difference(&pk).count(), pk.difference(&sk).count(), if diffs.is_empty() { String::new() } else { format!("; closed pairs differ by {:+.1} bps on average (paper minus simulator)", stat::mean(&diffs)) });
        if sk.difference(&pk).count() + pk.difference(&sk).count() > 0 {
            let _ = writeln!(md, "  Unmatched entries mean the engine was down at a bar close, a quote was missing, or the two disagree. Explain them before trusting the numbers above.");
        }
        let _ = writeln!(md);
    }
    // The catalyst recorder: forward evidence only.
    let marks: Vec<super::catalyst::Markout> = crate::status::read_jsonl(&paths.catalyst_markouts);
    let _ = writeln!(md, "## Jev catalyst scalper (record mode, no orders)\n");
    if marks.is_empty() {
        let _ = writeln!(md, "No markouts yet. Each scored headline about a coin is marked out at fixed horizons after it arrives.");
    } else {
        let _ = writeln!(md, "Return of trading each headline in the direction Jev read, after the round-trip cost.\n\n| After | Triggered (strong, new, fresh) | n | Every scored headline | n |\n|---|---|---|---|---|");
        for (h, tn, tmean, an, amean) in super::catalyst::summary(&marks) {
            let _ = writeln!(md, "| {} min | {} | {tn} | {:+.1} bps | {an} |", h / 60, if tn > 0 { format!("{tmean:+.1} bps") } else { "–".into() }, amean);
        }
        let days = marks.iter().map(|m| m.ts.date_naive()).collect::<HashSet<_>>().len();
        let _ = writeln!(md, "\n{days} day(s) of evidence. Below 30 triggered candidates across 20 days this is an anecdote, not a result.");
    }
    crate::status::write_atomic(&paths.monitor, md.as_bytes())?;
    println!("{md}");
    Ok(())
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
}
