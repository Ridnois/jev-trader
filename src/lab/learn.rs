//! `lab learn`: read every verdict, the trial ledger, the paper monitor and the
//! lineage, and write down what they say as typed lessons.
//!
//! The learner reads files only (or, in calibration, the same records held in memory)
//! and simulates nothing. Its inputs never contain lockbox days: the gate's evidence
//! record is cut at the design window, and paper days are forward data.
//!
//! A lesson backed by fewer observations than `[evolve]` asks for is kept, labelled
//! an anecdote, and the proposer may not act on it. Two rules for forward evidence:
//! paper days never tune a running strategy (they may only kill it or motivate a
//! child), and execution lessons are addressed to the operator, because changing the
//! cost model re-prices every trial on record.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use super::gate::Evidence;
use super::lineage::Event;
use super::search::TrialRecord;
use super::stat;
use super::strategy::Spec;
use super::EvolveCfg;
use crate::trade::monitor;

/// What a lesson found, in a form the proposer can act on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Finding {
    /// The gate's verdict and the checks that failed.
    Gate { verdict: String, failed: Vec<String>, fatal: Vec<String> },
    /// Which grid dimensions move the result, and which are flat.
    Params { flat: Vec<String>, moving: Vec<ParamMove> },
    /// The pick earns in one market state and loses in the other.
    Regime { favoured: Regime, mean_in_bps: f64, mean_out_bps: f64, days_in: usize, days_out: usize, t: f64 },
    /// One side carries the losses.
    Side { worse: String, long_bps: f64, short_bps: f64, long_trades: usize, short_trades: usize },
    /// The longest-held trades carry the losses.
    Hold { worse: String, better: String, worse_bps: f64, better_bps: f64, worse_trades: usize },
    /// Loses broadly: most coins and most quarters.
    BroadLoss { mean_bps: f64, coins_positive: usize, coins: usize, quarters_positive: usize, quarters: usize },
    /// Profitable only because of one or two coins.
    Coins { carried_by: Vec<String>, positive: usize, traded: usize },
    /// The walk-forward pick kept changing.
    Instability { distinct: usize, folds: usize },
    /// Two families' picks move together (or don't).
    Redundancy { other: String, corr: f64, days: usize },
    /// The paper run's sequential call.
    Paper { call: String, days: usize, mean_bps: f64, card_bps: f64 },
    /// Paper fills against the cost model. For the operator.
    Execution { entry_cost_bps: f64, model_side_bps: f64, trades: usize },
    /// Catalyst recorder markouts at one horizon.
    Catalyst { h_secs: i64, triggered: usize, triggered_bps: f64, days: usize },
    /// Every tested descendant of a root family was rejected.
    DyingBranch { tested: usize, rejected: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Regime {
    MarketUp,
    MarketDown,
    VolHigh,
    VolLow,
}

impl Regime {
    pub fn word(self) -> &'static str {
        match self {
            Regime::MarketUp => "the coin basket up over the last 30 days",
            Regime::MarketDown => "the coin basket down over the last 30 days",
            Regime::VolHigh => "basket volatility above its median",
            Regime::VolLow => "basket volatility below its median",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamMove {
    pub name: String,
    pub best: f64,
    pub worst: f64,
    /// Spread of the per-value mean Sharpe (annualised).
    pub spread: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lesson {
    pub id: String,
    pub kind: String,
    /// Family name (spec family / rule id) or paper strategy name.
    pub subject: String,
    /// What a person calls it (generated name, or the family).
    pub label: String,
    pub study: String,
    pub finding: String,
    /// Files (or records) the lesson rests on.
    pub evidence: Vec<String>,
    /// 0..1: how much the proposer should weigh it.
    pub strength: f64,
    /// Observations behind it (days, trades, specs).
    pub n: usize,
    /// Below the stated minimum: written down, not acted on.
    pub anecdote: bool,
    /// For the operator only (execution, catalyst): never turned into a proposal.
    pub operator_only: bool,
    pub data: Finding,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Lessons {
    pub generated: Option<chrono::DateTime<chrono::Utc>>,
    pub sources: Vec<String>,
    pub lessons: Vec<Lesson>,
    pub notes: Vec<String>,
}

/// One gate verdict as the learner sees it.
pub struct Verdict<'a> {
    /// Where it came from (a file path, or "memory").
    pub source: String,
    /// The evaluation, as written to `<study>/<family>.json`.
    pub eval: &'a serde_json::Value,
    pub label: String,
}

/// Everything the learner reads.
pub struct Inputs<'a> {
    pub verdicts: Vec<Verdict<'a>>,
    pub ledger: &'a [TrialRecord],
    pub monitor: Option<&'a monitor::Report>,
    pub lineage: &'a [Event],
}

fn quarter(date: &str, offset: usize) -> Option<(i32, u32)> {
    use chrono::Datelike;
    let d = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()? + chrono::Days::new(offset as u64);
    Some((d.year(), (d.month() - 1) / 3))
}

/// Mean of `x` where `mask` holds and where it doesn't, with a t-stat on the
/// difference that counts blocks of `block` days, not days (regimes persist).
fn split(x: &[f64], mask: &[Option<bool>], block: f64) -> Option<(f64, f64, usize, usize, f64)> {
    let a: Vec<f64> = x.iter().zip(mask).filter(|(_, m)| **m == Some(true)).map(|(v, _)| *v).collect();
    let b: Vec<f64> = x.iter().zip(mask).filter(|(_, m)| **m == Some(false)).map(|(v, _)| *v).collect();
    if a.len() < 2 || b.len() < 2 {
        return None;
    }
    let (ma, mb) = (stat::mean(&a), stat::mean(&b));
    let se = ((stat::sd(&a).powi(2) / a.len() as f64 + stat::sd(&b).powi(2) / b.len() as f64) * block).sqrt();
    let t = if se > 0.0 { (ma - mb) / se } else { 0.0 };
    Some((ma, mb, a.len(), b.len(), t))
}

/// Market state of each evidence day, from the 30 days before it (causal).
fn regimes(basket: &[f64]) -> (Vec<Option<bool>>, Vec<Option<bool>>) {
    let w = 30;
    let n = basket.len();
    let mut up = vec![None; n];
    let mut vol = vec![f64::NAN; n];
    for d in w..n {
        let win = &basket[d - w..d];
        up[d] = Some(win.iter().sum::<f64>() > 0.0);
        vol[d] = stat::sd(win);
    }
    let mut finite: Vec<f64> = vol.iter().copied().filter(|v| v.is_finite()).collect();
    finite.sort_by(f64::total_cmp);
    let med = finite.get(finite.len() / 2).copied().unwrap_or(f64::NAN);
    let high = vol.iter().map(|v| v.is_finite().then(|| *v > med)).collect();
    (up, high)
}

struct Builder<'c> {
    cfg: &'c EvolveCfg,
    out: Vec<Lesson>,
}

impl Builder<'_> {
    #[allow(clippy::too_many_arguments)]
    fn add(&mut self, kind: &str, subject: &str, label: &str, study: &str, finding: String, evidence: &str, strength: f64, n: usize, anecdote: bool, data: Finding) {
        let operator_only = matches!(data, Finding::Execution { .. } | Finding::Catalyst { .. });
        self.out.push(Lesson { id: String::new(), kind: kind.into(), subject: subject.into(), label: label.into(), study: study.into(), finding, evidence: vec![evidence.into()], strength: strength.clamp(0.0, 1.0), n, anecdote, operator_only, data });
    }
}

/// Reads one verdict.
fn from_verdict(b: &mut Builder, v: &Verdict, ledger: &[TrialRecord]) {
    let e = v.eval;
    let (family, study) = (e["family"].as_str().unwrap_or(""), e["study"].as_str().unwrap_or(""));
    let label = v.label.as_str();
    let src = v.source.as_str();
    let checks = e["checks"].as_array().cloned().unwrap_or_default();
    let failed: Vec<String> = checks.iter().filter(|c| c["pass"] == false).filter_map(|c| c["name"].as_str().map(str::to_string)).collect();
    let fatal: Vec<String> = checks.iter().filter(|c| c["pass"] == false && c["fatal"] == true).filter_map(|c| c["name"].as_str().map(str::to_string)).collect();
    let verdict = e["verdict"].as_str().unwrap_or("").to_string();
    // Breadth checks say more with their numbers.
    let breadth: Vec<String> = checks.iter().filter(|c| c["pass"] == false && (c["name"] == "quarters" || c["name"] == "coins")).filter_map(|c| Some(format!("{}: {}", c["name"].as_str()?, c["value"].as_str()?.split(';').next()?))).collect();
    b.add("gate", family, label, study, format!("{label} on {study}: {verdict}{}{}", if failed.is_empty() { String::new() } else { format!("; failed {}", failed.join(", ")) }, if breadth.is_empty() { String::new() } else { format!(" ({})", breadth.join("; ")) }), src, 1.0, e["specs"].as_u64().unwrap_or(0) as usize, false, Finding::Gate { verdict, failed, fatal });

    // Parameter response, from every spec of this family charged on this study.
    let rows: Vec<(Spec, f64)> = ledger.iter().filter(|r| r.study == study && r.family == family).filter_map(|r| super::parse_spec(&r.spec).ok().map(|s| (s, r.sharpe_daily * super::gate::DAYS_PER_YEAR.sqrt()))).collect();
    if rows.len() >= 4 {
        let all: Vec<f64> = rows.iter().map(|r| r.1).collect();
        let sd_all = stat::sd(&all);
        let keys: BTreeSet<&String> = rows.iter().flat_map(|(s, _)| s.params.keys()).collect();
        let (mut flat, mut moving) = (Vec::new(), Vec::new());
        for k in keys {
            let mut by: BTreeMap<u64, Vec<f64>> = BTreeMap::new();
            for (s, sh) in &rows {
                if let Some(v) = s.params.get(k) {
                    by.entry(v.to_bits()).or_default().push(*sh);
                }
            }
            if by.len() < 2 {
                continue;
            }
            let means: Vec<(f64, f64)> = by.iter().map(|(v, x)| (f64::from_bits(*v), stat::mean(x))).collect();
            let best = means.iter().max_by(|a, b| a.1.total_cmp(&b.1)).expect("two values");
            let worst = means.iter().min_by(|a, b| a.1.total_cmp(&b.1)).expect("two values");
            let spread = best.1 - worst.1;
            if spread < 0.25 * sd_all || spread < 0.05 {
                flat.push(k.clone());
            } else {
                moving.push(ParamMove { name: k.clone(), best: best.0, worst: worst.0, spread });
            }
        }
        moving.sort_by(|a, b| b.spread.total_cmp(&a.spread));
        let text = format!(
            "{label}: {} over {} specs{}",
            if moving.is_empty() { "no parameter moves the result".to_string() } else { moving.iter().map(|m| format!("`{}` moves it (best {}, worst {}, spread {:.2} Sharpe)", m.name, m.best, m.worst, m.spread)).collect::<Vec<_>>().join("; ") },
            rows.len(),
            if flat.is_empty() { String::new() } else { format!("; flat: {}", flat.join(", ")) }
        );
        b.add("params", family, label, study, text, "lab-data/trials.jsonl", 0.5 * (moving.first().map(|m| m.spread).unwrap_or(0.0) / 2.0).min(1.0), rows.len(), false, Finding::Params { flat, moving });
    }

    // Walk-forward stability.
    if let Some(folds) = e["fold_picks"].as_array().filter(|f| f.len() >= 3) {
        let distinct = folds.iter().filter_map(|f| f.as_str()).collect::<BTreeSet<_>>().len();
        if distinct * 2 >= folds.len() {
            b.add("instability", family, label, study, format!("{label}: the walk-forward pick changed in most folds ({distinct} distinct picks in {} folds): the in-sample best is not a stable choice", folds.len()), src, distinct as f64 / folds.len() as f64, folds.len(), false, Finding::Instability { distinct, folds: folds.len() });
        }
    }

    let Some(ev) = e.get("evidence").and_then(|x| serde_json::from_value::<Evidence>(x.clone()).ok()) else {
        if e["pick"].is_object() {
            b.out.push(Lesson { id: String::new(), kind: "note".into(), subject: family.into(), label: label.into(), study: study.into(), finding: format!("{label}: this verdict predates the evidence record; re-run `lab validate` to learn where it made and lost money"), evidence: vec![src.into()], strength: 0.0, n: 0, anecdote: true, operator_only: true, data: Finding::Gate { verdict: String::new(), failed: Vec::new(), fatal: Vec::new() } });
        }
        return;
    };
    let cfg = b.cfg;
    let mean_bps = stat::mean(&ev.daily) * 1e4;

    // Market states.
    let (up, high) = regimes(&ev.basket);
    for (mask, yes, no) in [(&up, Regime::MarketUp, Regime::MarketDown), (&high, Regime::VolHigh, Regime::VolLow)] {
        let Some((m_in, m_out, n_in, n_out, t)) = split(&ev.daily, mask, 5.0) else { continue };
        // Earns in one state, loses in the other.
        let (fav, a, bb, na, nb) = if m_in > 0.0 && m_out < 0.0 { (yes, m_in, m_out, n_in, n_out) } else if m_out > 0.0 && m_in < 0.0 { (no, m_out, m_in, n_out, n_in) } else { continue };
        let t = t.abs();
        if t < 1.0 {
            continue;
        }
        let anecdote = na.min(nb) < cfg.min_regime_days;
        b.add("regime", family, label, study, format!("{label} earns {:+.1} bps/day with {} ({na} days) and {:+.1} without ({nb} days), t = {t:.1} counting 5-day blocks", a * 1e4, fav.word(), bb * 1e4), src, t / 3.0, na + nb, anecdote, Finding::Regime { favoured: fav, mean_in_bps: a * 1e4, mean_out_bps: bb * 1e4, days_in: na, days_out: nb, t });
    }

    // Sides.
    if let (Some(l), Some(s)) = (ev.sides.iter().find(|x| x.label == "long"), ev.sides.iter().find(|x| x.label == "short"))
        && l.trades > 0
        && s.trades > 0
    {
        let (worse, w, other) = if l.pnl < s.pnl { ("long", l, s) } else { ("short", s, l) };
        if w.pnl < 0.0 && other.pnl > 0.0 {
            let anecdote = w.trades < cfg.min_side_trades;
            b.add("side", family, label, study, format!("{label}: {worse}s lose ({:+.1} bps per trade over {}) while {}s make {:+.1}", w.mean_bps, w.trades, other.label, other.mean_bps), src, ((other.mean_bps - w.mean_bps) / 50.0).min(1.0), w.trades, anecdote, Finding::Side { worse: worse.into(), long_bps: l.mean_bps, short_bps: s.mean_bps, long_trades: l.trades, short_trades: s.trades });
        }
    }

    // Holding time: only "the longest holds lose" is acted on (shortening is a real change;
    // the shortest trades are mostly the stopped-out ones).
    if ev.holds.len() >= 2 {
        let (last, best) = (ev.holds.last().expect("holds"), ev.holds.iter().max_by(|a, b| a.mean_bps.total_cmp(&b.mean_bps)).expect("holds"));
        if last.mean_bps < 0.0 && best.mean_bps > 0.0 && best.label != last.label {
            b.add("hold", family, label, study, format!("{label}: trades held {} lose {:+.1} bps each while those held {} make {:+.1}", last.label, last.mean_bps, best.label, best.mean_bps), src, ((best.mean_bps - last.mean_bps) / 100.0).min(1.0), last.trades, last.trades < cfg.min_side_trades, Finding::Hold { worse: last.label.clone(), better: best.label.clone(), worse_bps: last.mean_bps, better_bps: best.mean_bps, worse_trades: last.trades });
        }
    }

    // Breadth.
    let positive = ev.coins.iter().filter(|c| c.pnl > 0.0).count();
    let total: f64 = ev.coins.iter().map(|c| c.pnl).sum();
    let mut quarters: BTreeMap<(i32, u32), f64> = BTreeMap::new();
    for (d, r) in ev.daily.iter().enumerate() {
        if let Some(q) = quarter(&ev.first_day, d) {
            *quarters.entry(q).or_default() += r;
        }
    }
    let qpos = quarters.values().filter(|v| **v > 0.0).count();
    if mean_bps < 0.0 && positive * 4 <= ev.coins.len() && qpos * 3 <= quarters.len() && !ev.coins.is_empty() {
        let trades: usize = ev.coins.iter().map(|c| c.trades).sum();
        let anecdote = ev.daily.len() < 2 * cfg.min_regime_days || trades < cfg.min_side_trades;
        b.add("broad_loss", family, label, study, format!("{label} loses broadly: {mean_bps:+.1} bps/day over {trades} trades, {positive} of {} coins and {qpos} of {} quarters positive. The opposite side may be the edge, as its own hypothesis", ev.coins.len(), quarters.len()), src, (1.0 - positive as f64 / ev.coins.len() as f64).min(1.0), trades, anecdote, Finding::BroadLoss { mean_bps, coins_positive: positive, coins: ev.coins.len(), quarters_positive: qpos, quarters: quarters.len() });
    } else if total > 0.0 {
        let carried: Vec<String> = ev.coins.iter().filter(|c| total - c.pnl <= 0.0).map(|c| c.label.clone()).collect();
        if !carried.is_empty() {
            b.add("coins", family, label, study, format!("{label} is profitable only because of {}", carried.join(", ")), src, 0.5, ev.coins.len(), false, Finding::Coins { carried_by: carried, positive, traded: ev.coins.len() });
        }
    }
}

/// Correlation of two picks' daily returns over their common dates.
fn aligned_corr(a: &Evidence, b: &Evidence) -> Option<(f64, usize)> {
    let (da, db) = (chrono::NaiveDate::parse_from_str(&a.first_day, "%Y-%m-%d").ok()?, chrono::NaiveDate::parse_from_str(&b.first_day, "%Y-%m-%d").ok()?);
    let off = (db - da).num_days();
    let (mut x, mut y) = (Vec::new(), Vec::new());
    for (i, v) in a.daily.iter().enumerate() {
        let j = i as i64 - off;
        if j >= 0 && (j as usize) < b.daily.len() {
            x.push(*v);
            y.push(b.daily[j as usize]);
        }
    }
    (x.len() >= 60).then(|| crate::stats::pearson(&x, &y).map(|c| (c, x.len()))).flatten()
}

/// The root of a family in the lineage (itself when it has no recorded parent).
pub fn root_of(lineage: &[Event], family: &str) -> String {
    let mut cur = family.to_string();
    for _ in 0..64 {
        match lineage.iter().find(|e| e.family == cur && e.event == "proposed").and_then(|e| e.parent.clone()) {
            Some(p) => cur = p,
            None => break,
        }
    }
    cur
}

pub fn learn(inp: &Inputs, cfg: &EvolveCfg) -> Lessons {
    let mut b = Builder { cfg, out: Vec::new() };
    let mut sources: Vec<String> = Vec::new();
    for v in &inp.verdicts {
        from_verdict(&mut b, v, inp.ledger);
        sources.push(v.source.clone());
    }

    // Redundancy between picks (any two verdicts with an evidence record).
    let evs: Vec<(&Verdict, Evidence)> = inp.verdicts.iter().filter_map(|v| v.eval.get("evidence").and_then(|x| serde_json::from_value::<Evidence>(x.clone()).ok()).map(|e| (v, e))).collect();
    for i in 0..evs.len() {
        for j in i + 1..evs.len() {
            let Some((c, n)) = aligned_corr(&evs[i].1, &evs[j].1) else { continue };
            if c.abs() >= 0.7 || c.abs() <= 0.3 {
                let (a, bb) = (evs[i].0, evs[j].0);
                let fam = a.eval["family"].as_str().unwrap_or("");
                let other = bb.eval["family"].as_str().unwrap_or("").to_string();
                let words = if c.abs() >= 0.7 { "move together: one adds little to the other" } else { "are nearly unrelated: parts of each may combine" };
                b.add("redundancy", fam, &a.label, a.eval["study"].as_str().unwrap_or(""), format!("{} and {} {words} (daily correlation {c:+.2} over {n} days)", a.label, bb.label), &a.source, if c.abs() >= 0.7 { c.abs() } else { 1.0 - c.abs() }, n, false, Finding::Redundancy { other, corr: c, days: n });
            }
        }
    }

    // Paper and the operator's lessons.
    if let Some(m) = inp.monitor {
        sources.push("data-v3/monitor.json".into());
        for s in &m.strategies {
            let anecdote = s.paper_days < cfg.min_paper_days;
            b.add("paper", &s.spec.split(':').next().unwrap_or("").to_string(), &s.name, "paper", format!("{} on paper: {} after {} full days ({:+.1} bps/day against {:+.1} on the card){}", s.name, s.call, s.paper_days, s.mean_bps, s.card_mu_bps, if s.call_reason.is_empty() { String::new() } else { format!(": {}", s.call_reason) }), "data-v3/monitor.json", if s.call == "KILL" { 1.0 } else { 0.3 }, s.paper_days, anecdote, Finding::Paper { call: s.call.clone(), days: s.paper_days, mean_bps: s.mean_bps, card_bps: s.card_mu_bps });
            if let Some(cost) = s.execution.entry_cost_bps
                && (cost - s.execution.model_side_bps).abs() > 1.0
            {
                let anecdote = s.execution.trades < cfg.min_side_trades;
                b.add("execution", &s.name, &s.name, "paper", format!("{}: paper entries cost {cost:.1} bps against the mid; the lab charges {:.1}. If this holds, `[costs]` in lab.toml should change. That is your decision: it re-prices every trial on record", s.name, s.execution.model_side_bps), "data-v3/trades.jsonl", 0.0, s.execution.trades, anecdote, Finding::Execution { entry_cost_bps: cost, model_side_bps: s.execution.model_side_bps, trades: s.execution.trades });
            }
        }
        for r in &m.catalyst {
            if let Some(tb) = r.triggered_bps {
                let anecdote = r.triggered_n < cfg.min_catalyst || m.catalyst_days < cfg.min_paper_days;
                b.add("catalyst", "catalyst", "Jev catalyst recorder", "paper", format!("catalyst candidates {} min after the headline: {tb:+.1} bps net over {} triggered ({} days)", r.h_secs / 60, r.triggered_n, m.catalyst_days), "data-v3/catalyst_markouts.jsonl", 0.0, r.triggered_n, anecdote, Finding::Catalyst { h_secs: r.h_secs, triggered: r.triggered_n, triggered_bps: tb, days: m.catalyst_days });
            }
        }
    }

    // Branches that keep dying.
    let mut by_root: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for e in inp.lineage.iter().filter(|e| e.event == "tested") {
        let r = by_root.entry(root_of(inp.lineage, &e.family)).or_default();
        r.0 += 1;
        r.1 += (e.verdict.as_deref() == Some("REJECT")) as usize;
    }
    for (root, (tested, rejected)) in by_root {
        if tested >= 3 && rejected == tested {
            b.add("dying_branch", &root, &root, "", format!("all {tested} tested descendants of {root} were rejected: stop deriving from it"), "lab-data/lineage.jsonl", 1.0, tested, false, Finding::DyingBranch { tested, rejected });
        }
    }

    // Stable order and ids.
    let order = |k: &str| ["gate", "regime", "side", "hold", "broad_loss", "coins", "params", "instability", "redundancy", "paper", "dying_branch", "execution", "catalyst", "note"].iter().position(|x| *x == k).unwrap_or(99);
    b.out.sort_by(|a, c| (a.study.as_str(), a.label.as_str(), order(&a.kind)).cmp(&(c.study.as_str(), c.label.as_str(), order(&c.kind))).then(a.finding.cmp(&c.finding)));
    for (i, l) in b.out.iter_mut().enumerate() {
        l.id = format!("L{:02}", i + 1);
    }
    let mut notes = Vec::new();
    if inp.verdicts.is_empty() {
        notes.push("no verdicts on file: run `lab validate` on a family first".into());
    }
    Lessons { generated: Some(chrono::Utc::now()), sources, lessons: b.out, notes }
}

pub fn markdown(l: &Lessons) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# Lessons\n\n*{} · from {} sources*\n", l.generated.map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_default(), l.sources.len());
    let _ = writeln!(s, "Anecdotes rest on fewer observations than `[evolve]` asks for: written down, not acted on. Lessons marked *operator* are for you, never turned into a proposal.\n");
    let _ = writeln!(s, "| Id | Subject | Kind | Finding | n | Strength | |\n|---|---|---|---|---|---|---|");
    for x in &l.lessons {
        let tag = if x.operator_only { "operator" } else if x.anecdote { "anecdote" } else { "" };
        let _ = writeln!(s, "| {} | {} | {} | {} | {} | {:.2} | {tag} |", x.id, x.label, x.kind, x.finding.replace('|', "/"), x.n, x.strength);
    }
    for n in &l.notes {
        let _ = writeln!(s, "\n- {n}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::super::gate::Slice;
    use super::*;

    fn slice(label: &str, trades: usize, pnl: f64, bps: f64) -> Slice {
        Slice { label: label.into(), trades, wins: trades / 2, pnl, mean_bps: bps }
    }

    fn cfg() -> EvolveCfg {
        EvolveCfg::default()
    }

    /// A pick that makes money when the basket rose over the month before, loses when it fell,
    /// whose shorts lose, and whose longest holds lose.
    fn eval() -> serde_json::Value {
        let mut rng = crate::stats::Rng::new(5);
        let n = 600;
        let mut basket = Vec::new();
        let mut regime = 1.0;
        for d in 0..n {
            if d % 90 == 0 {
                regime = -regime;
            }
            basket.push(0.004 * regime + 0.01 * rng.normal());
        }
        let (up, _) = regimes(&basket);
        let daily: Vec<f64> = (0..n).map(|d| match up[d] {
            Some(true) => 0.003 + 0.01 * rng.normal(),
            Some(false) => -0.002 + 0.01 * rng.normal(),
            None => 0.0,
        }).collect();
        let ev = Evidence {
            first_day: "2024-01-01".into(),
            capital: 10_000.0,
            daily,
            basket,
            coins: vec![slice("BTC", 50, 300.0, 20.0), slice("ETH", 40, 100.0, 10.0), slice("SOL", 40, -50.0, -5.0)],
            sides: vec![slice("long", 100, 600.0, 30.0), slice("short", 40, -250.0, -40.0)],
            holds: vec![slice("≤ 5 bars", 50, 100.0, 10.0), slice("≤ 20 bars", 50, 400.0, 40.0), slice("> 20 bars", 40, -150.0, -25.0)],
            exits: vec![slice("time", 140, 350.0, 12.0)],
        };
        serde_json::json!({
            "study": "binance-1h", "family": "trend", "verdict": "REJECT", "specs": 8,
            "checks": [{"name": "walk-forward", "pass": false, "fatal": true}, {"name": "coins", "pass": true, "fatal": false}],
            "fold_picks": ["a", "b", "c", "a"],
            "pick": {"family": "trend", "params": {"n": 96.0}},
            "evidence": ev,
        })
    }

    fn ledger() -> Vec<TrialRecord> {
        let mut out = Vec::new();
        for n in [24.0, 96.0] {
            for shorts in [0.0, 1.0] {
                for stop in [0.0, 800.0] {
                    let sharpe = if shorts == 0.0 { 0.06 } else { -0.03 } + if n == 96.0 { 0.001 } else { 0.0 };
                    out.push(TrialRecord { study: "binance-1h".into(), family: "trend".into(), spec: format!("trend:n={n},shorts={shorts},stop={stop}"), at: chrono::Utc::now(), days: 600, trades: 300, sharpe_daily: sharpe, mean_bps_day: 0.0 });
                }
            }
        }
        out
    }

    #[test]
    fn reads_regimes_sides_holds_params_and_instability() {
        let e = eval();
        let ledger = ledger();
        let inp = Inputs { verdicts: vec![Verdict { source: "t.json".into(), eval: &e, label: "trend".into() }], ledger: &ledger, monitor: None, lineage: &[] };
        let l = learn(&inp, &cfg());
        let kinds: Vec<&str> = l.lessons.iter().map(|x| x.kind.as_str()).collect();
        for k in ["gate", "regime", "side", "hold", "params", "instability"] {
            assert!(kinds.contains(&k), "missing {k}: {kinds:?}");
        }
        let reg = l.lessons.iter().find(|x| matches!(x.data, Finding::Regime { favoured: Regime::MarketUp, .. })).expect("the market-up lesson");
        assert!(!reg.anecdote && reg.strength > 0.5, "{reg:?}");
        assert!(matches!(&l.lessons.iter().find(|x| x.kind == "side").unwrap().data, Finding::Side { worse, .. } if worse == "short"));
        match &l.lessons.iter().find(|x| x.kind == "params").unwrap().data {
            Finding::Params { flat, moving } => {
                assert_eq!(moving[0].name, "shorts");
                assert_eq!(moving[0].best, 0.0);
                assert!(flat.contains(&"stop".to_string()) && flat.contains(&"n".to_string()), "{flat:?}");
            }
            other => panic!("{other:?}"),
        }
        // Ids are stable and ordered.
        assert_eq!(l.lessons[0].id, "L01");
        let again = learn(&inp, &cfg());
        assert_eq!(l.lessons.iter().map(|x| (&x.id, &x.finding)).collect::<Vec<_>>(), again.lessons.iter().map(|x| (&x.id, &x.finding)).collect::<Vec<_>>());
        assert!(markdown(&l).contains("| L01 |"));
    }

    #[test]
    fn broad_losers_and_thin_evidence() {
        let mut e = eval();
        let ev: &mut serde_json::Value = &mut e["evidence"];
        let n = ev["daily"].as_array().unwrap().len();
        ev["daily"] = serde_json::json!((0..n).map(|i| -0.002 + if i % 2 == 0 { 0.001 } else { -0.001 }).collect::<Vec<f64>>());
        ev["coins"] = serde_json::json!([slice("BTC", 50, -300.0, -20.0), slice("ETH", 40, -100.0, -10.0), slice("SOL", 40, -50.0, -5.0), slice("XRP", 30, -10.0, -1.0)]);
        ev["sides"] = serde_json::json!([slice("long", 100, 60.0, 3.0), slice("short", 5, -250.0, -40.0)]);
        let inp = Inputs { verdicts: vec![Verdict { source: "t.json".into(), eval: &e, label: "meanrev".into() }], ledger: &[], monitor: None, lineage: &[] };
        let l = learn(&inp, &cfg());
        assert!(l.lessons.iter().any(|x| x.kind == "broad_loss" && !x.anecdote));
        // Five losing shorts are an anecdote.
        assert!(l.lessons.iter().find(|x| x.kind == "side").is_some_and(|x| x.anecdote));
    }

    #[test]
    fn dying_branches_and_roots() {
        let ev = |event: &str, family: &str, parent: Option<&str>, verdict: Option<&str>| Event { event: event.into(), family: family.into(), parent: parent.map(str::to_string), verdict: verdict.map(str::to_string), ..Event::default() };
        let lin = vec![
            ev("proposed", "c1", Some("trend"), None),
            ev("proposed", "c2", Some("c1"), None),
            ev("proposed", "c3", Some("trend"), None),
            ev("tested", "c1", None, Some("REJECT")),
            ev("tested", "c2", None, Some("REJECT")),
            ev("tested", "c3", None, Some("REJECT")),
        ];
        assert_eq!(root_of(&lin, "c2"), "trend");
        assert_eq!(root_of(&lin, "meanrev"), "meanrev");
        let inp = Inputs { verdicts: Vec::new(), ledger: &[], monitor: None, lineage: &lin };
        let l = learn(&inp, &cfg());
        assert!(l.lessons.iter().any(|x| x.kind == "dying_branch" && x.subject == "trend"));
    }
}
