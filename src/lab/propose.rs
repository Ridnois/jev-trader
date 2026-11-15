//! `lab propose`: turn lessons into a few new families, each with a hypothesis and a
//! reason that cites the lessons it acts on.
//!
//! The proposer is deterministic: the same lessons, lineage, ledger and seed give the
//! same proposals. It simulates nothing. Its operators:
//!
//! | operator    | acts on            | makes                                                      |
//! |-------------|--------------------|------------------------------------------------------------|
//! | `condition` | a `regime` lesson  | the parent with a market-state filter for the state it earns in |
//! | `restrict`  | `side`, `hold`     | the parent long-only, or with shorter holds                 |
//! | `invert`    | `broad_loss`       | the opposite side of the parent, as its own hypothesis      |
//! | `recombine` | `redundancy` (low) | the parent's entry confirmed by an unrelated family's entry |
//! | `shrink`    | `params` (flat)    | a hand-written parent as blocks, flat dimensions fixed      |
//! | `repair`    | `gate` (near miss) | the parent with one block aimed at a check it failed        |
//! | `explore`   | budget left over   | a feature pairing not yet in the lineage                    |
//!
//! Lessons are acted on closest parent first: the families that failed the fewest gate
//! checks get the cycle's slots before stronger lessons about families further away.
//!
//! The proposal file format is the interface: a person (or another proposer) can write
//! the same `families/gen/*.toml` files and they go through the same door.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use serde::Serialize;

use super::compose::{self, Cond, Feature, Op, Param, Rule, Short};
use super::learn::{Finding, Lesson, Lessons, Regime, root_of};
use super::lineage::{self, Event};
use super::strategy::{Family, Spec};
use super::EvolveCfg;
use crate::hypotheses::Hypothesis;
use crate::stats::Rng;

/// What the proposer knows about a family it may derive from.
#[derive(Debug, Clone)]
pub struct Parent {
    pub family: Family,
    /// The gate's pick on its study, if it has a verdict.
    pub pick: Option<Spec>,
    /// The pick's mean daily return on the design window, bps.
    pub mean_bps: Option<f64>,
    pub study: String,
    /// Gate checks its verdict failed; None when the gate stopped it before the checks.
    pub failed: Option<Vec<String>>,
}

impl Parent {
    /// Distance to passing: checks failed. Unjudged families sort last.
    pub fn distance(&self) -> usize {
        self.failed.as_ref().map_or(usize::MAX, Vec::len)
    }
}

/// Checks that say the edge is not steady over time.
const TIME_CHECKS: [&str; 3] = ["quarters", "walk-forward", "timing baseline"];
/// Checks that say the profit leans on a few days.
const CONCENTRATION_CHECKS: [&str; 1] = ["best days removed"];
/// Checks that say the edge does not survive slower or dearer execution.
const EXECUTION_CHECKS: [&str; 2] = ["one bar late", "costs ×2"];

#[derive(Debug, Clone, Serialize)]
pub struct Proposal {
    pub family: Family,
    pub hypothesis: Hypothesis,
    pub study: String,
    /// Specs of its grid not yet in the ledger for its study.
    pub new_trials: usize,
    pub specs: usize,
    /// Rule id + grid + fixed: never proposed twice.
    pub key: String,
    /// The parent is a hand-written family re-expressed as blocks only approximately.
    pub approximate: bool,
}

pub struct Inputs<'a> {
    pub lessons: &'a Lessons,
    /// By family name (spec family / rule id).
    pub parents: &'a BTreeMap<String, Parent>,
    pub lineage: &'a [Event],
    /// Spec ids already charged, by study.
    pub charged: &'a BTreeMap<String, HashSet<String>>,
    pub research_venue: &'a str,
    pub generation: u32,
    pub seed: u64,
    /// Auto mode: most trials the loop may charge on one study over all its cycles
    /// (counted from the lineage's `tested` events). None = no such cap.
    pub loop_budget: Option<usize>,
    /// With a budget: count only the loop's trials tested at or after this time (a cap per
    /// run), and ignore the per-study lifetime overrides.
    pub budget_since: Option<chrono::DateTime<chrono::Utc>>,
    /// Start of the ledger's current era: loop spend before it is not counted.
    pub era: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct Outcome {
    pub proposals: Vec<Proposal>,
    /// Why candidates were skipped and why the cycle stopped.
    pub messages: Vec<String>,
}

const POLICY: [&str; 5] = ["hold", "hold_days", "stop", "shorts", "max_positions"];

fn bars_per_day(interval: &str) -> f64 {
    crate::market::interval_ms(interval).map(|ms| (86_400_000 / ms).max(1) as f64).unwrap_or(24.0)
}

/// The parent as blocks, with its pick's parameters. `None` when it has no composed form.
fn base(parent: &Parent) -> Option<(Rule, BTreeMap<String, f64>, bool)> {
    let pick = parent.pick.as_ref()?;
    if let Some(rule) = &parent.family.rule {
        return Some((rule.clone(), pick.params.clone(), true));
    }
    let (mut rule, exact) = compose::template(&parent.family.family)?;
    let mut params = pick.params.clone();
    let p = |k: &str, d: f64| pick.get(k, d);
    match parent.family.family.as_str() {
        "meanrev" | "jev_gate" => {
            if p("resid", 0.0) > 0.5 {
                rule.long[0].feature = Feature::ResidZ;
            }
            if p("exit_mean", 0.0) > 0.5 {
                rule.exit.push(Cond { feature: rule.long[0].feature, n: Param::Ref("$n".into()), op: Op::Ge, level: Param::Num(0.0) });
            }
        }
        "hvt" | "xs_reversal" if p("dir", 1.0) < 0.0 => rule.long[0] = rule.long[0].inverted(),
        _ => {}
    }
    if matches!(parent.family.family.as_str(), "xs_reversal" | "funding_carry") {
        // The k worst of about 20 coins, as a rank cut-off.
        let level = -(1.0 - 2.0 * p("k", 3.0) / 20.0).clamp(0.0, 0.95);
        let inv = matches!(rule.long[0].op, Op::Ge | Op::Gt);
        rule.long[0].level = Param::Num(if inv { -level } else { level });
    }
    let refs = rule.refs();
    params.retain(|k, _| refs.contains(k) || POLICY.contains(&k.as_str()));
    Some((rule, params, exact))
}

/// The spec parameter a rule's first entry window reads, when it is a plain `$name`.
fn main_window(rule: &Rule) -> Option<String> {
    match &rule.long.first()?.n {
        Param::Ref(s) if s.starts_with('$') && !s.contains('*') => Some(s[1..].to_string()),
        _ => None,
    }
}

/// `v` halved, as is, and doubled (rounded, at least 2): a small plateau to judge.
fn around(v: f64) -> Vec<f64> {
    let mut out: Vec<f64> = [0.5, 1.0, 2.0].iter().map(|m| (v * m).round().max(2.0)).collect();
    out.dedup();
    out
}

/// Renames `$x` to `$<prefix>x` in every condition.
fn renamed(conds: &[Cond], prefix: &str) -> Vec<Cond> {
    let fix = |p: &Param| match p {
        Param::Ref(s) => Param::Ref(s.split('*').map(|f| match f.trim().strip_prefix('$') {
            Some(n) => format!("${prefix}{n}"),
            None => f.trim().to_string(),
        }).collect::<Vec<_>>().join("*")),
        n => n.clone(),
    };
    conds.iter().map(|c| Cond { n: fix(&c.n), level: fix(&c.level), ..c.clone() }).collect()
}

/// A candidate before it is checked against the budget and the lineage.
struct Draft {
    rule: Rule,
    fixed: BTreeMap<String, f64>,
    grid: BTreeMap<String, Vec<f64>>,
    interval: String,
    parent: Option<String>,
    parent_label: String,
    parent_spec: String,
    operator: &'static str,
    lessons: Vec<String>,
    why: String,
    approximate: bool,
}

fn draft(parent: &Parent, operator: &'static str, lesson: &Lesson, why: String, f: impl FnOnce(&mut Rule, &mut BTreeMap<String, f64>, &mut BTreeMap<String, Vec<f64>>, f64) -> bool) -> Option<Draft> {
    let (mut rule, mut fixed, exact) = base(parent)?;
    let mut grid = BTreeMap::new();
    if !f(&mut rule, &mut fixed, &mut grid, bars_per_day(&parent.family.interval)) {
        return None;
    }
    Some(Draft {
        rule,
        fixed,
        grid,
        interval: parent.family.interval.clone(),
        parent: Some(parent.family.family.clone()),
        parent_label: parent.family.label().to_string(),
        parent_spec: parent.pick.as_ref().map(Spec::id).unwrap_or_default(),
        operator,
        lessons: vec![lesson.id.clone()],
        why,
        approximate: !exact,
    })
}

/// Puts the main window on a small grid around the pick.
fn widen_window(rule: &Rule, fixed: &BTreeMap<String, f64>, grid: &mut BTreeMap<String, Vec<f64>>) {
    if let Some(w) = main_window(rule)
        && let Some(v) = fixed.get(&w)
    {
        grid.insert(w, around(*v));
    }
}

/// A condition with every fixed parameter replaced by its value, so `$n` fixed at 96 and
/// `$b_n` fixed at 96 read the same.
fn cond_key(c: &Cond, fixed: &BTreeMap<String, f64>) -> String {
    let p = |x: &Param| match x {
        Param::Num(v) => format!("{v}"),
        Param::Ref(s) => s
            .split('*')
            .map(|t| {
                let t = t.trim();
                fixed.get(t.trim_start_matches('$')).map(|v| format!("{v}")).unwrap_or_else(|| t.to_string())
            })
            .collect::<Vec<_>>()
            .join("*"),
    };
    format!("{:?}|{}|{:?}|{}", c.feature, p(&c.n), c.op, p(&c.level))
}

/// Drops repeated conditions from each block of a rule (a recombine of a family with one of
/// its own ingredients repeats them). Returns how many were dropped.
fn dedupe(rule: &mut Rule, fixed: &BTreeMap<String, f64>) -> usize {
    let mut dropped = 0;
    for block in [&mut rule.long, &mut rule.filter, &mut rule.exit] {
        let mut seen = HashSet::new();
        let before = block.len();
        block.retain(|c| seen.insert(cond_key(c, fixed)));
        dropped += before - block.len();
    }
    dropped
}

/// The primary-parent chain of a generated family, nearest first (not including itself).
fn ancestors(lineage: &[Event], family: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = family.to_string();
    for _ in 0..64 {
        match lineage.iter().find(|e| e.family == cur && e.event == "proposed").and_then(|e| e.parent.clone()) {
            Some(p) => {
                out.push(p.clone());
                cur = p;
            }
            None => break,
        }
    }
    out
}

fn has_filter(rule: &Rule, f: Feature) -> bool {
    rule.filter.iter().any(|c| c.feature == f)
}

/// Adds a market-state filter for `fav`, its window and level on a small grid.
fn regime_filter(rule: &mut Rule, grid: &mut BTreeMap<String, Vec<f64>>, bpd: f64, fav: Regime) -> bool {
    let (feature, n_key, lvl_key, op, levels, windows): (Feature, &str, &str, Op, Vec<f64>, Vec<f64>) = match fav {
        Regime::MarketUp => (Feature::MktRet, "mkt_n", "mkt_min", Op::Gt, vec![0.0, 0.03], vec![15.0 * bpd, 30.0 * bpd]),
        Regime::MarketDown => (Feature::MktRet, "mkt_n", "mkt_max", Op::Lt, vec![0.0, -0.03], vec![15.0 * bpd, 30.0 * bpd]),
        Regime::VolLow => (Feature::MktRvolPct, "vol_n", "vol_cap", Op::Lt, vec![0.5, 0.7], vec![bpd, 3.0 * bpd]),
        Regime::VolHigh => (Feature::MktRvolPct, "vol_n", "vol_floor", Op::Gt, vec![0.3, 0.5], vec![bpd, 3.0 * bpd]),
    };
    if has_filter(rule, feature) {
        return false;
    }
    rule.filter.push(Cond { feature, n: Param::Ref(format!("${n_key}")), op, level: Param::Ref(format!("${lvl_key}")) });
    grid.insert(n_key.into(), windows);
    grid.insert(lvl_key.into(), levels);
    true
}

/// `repair`: a family close to passing, changed by one block aimed at a check it failed.
/// Only structural changes: a composed family is named by its rule, so the same rule on
/// other parameters would overwrite the parent's verdict. Checks that measure the search
/// itself (deflated Sharpe, reality check, plateau, PBO) have no repair here: the bar they
/// set counts every trial on the study, and only more history moves it.
fn repairs(l: &Lesson, failed: &[String], inp: &Inputs, cfg: &EvolveCfg) -> Vec<Draft> {
    let Some(parent) = inp.parents.get(&l.subject) else { return Vec::new() };
    if failed.is_empty() || failed.len() > cfg.repair_max_failed || parent.pick.is_none() {
        return Vec::new();
    }
    let hit = |set: &[&str]| failed.iter().filter(|f| set.contains(&f.as_str())).cloned().collect::<Vec<_>>();
    let label = parent.family.label().to_string();
    let mut out = Vec::new();

    // Not steady over time: trade only in the market state it earns more in, read from its own evidence.
    let time = hit(&TIME_CHECKS);
    if !time.is_empty() {
        let lean = inp.lessons.lessons.iter().filter(|x| x.subject == l.subject && !x.anecdote).filter_map(|x| match &x.data {
            Finding::Regime { favoured, mean_in_bps, mean_out_bps, t, .. } | Finding::Lean { favoured, mean_in_bps, mean_out_bps, t, .. } => Some((x, *favoured, *mean_in_bps, *mean_out_bps, *t)),
            _ => None,
        }).max_by(|a, b| a.4.total_cmp(&b.4));
        if let Some((lx, fav, a, b, _)) = lean {
            let why = format!("{label} failed only {} ({}), {} of them about consistency over time ({}). It earned {a:+.1} bps/day with {} against {b:+.1} otherwise ({}): trading only in that state tests whether the gaps were the other state.", failed.join(", "), l.id, time.join(", "), time.len(), fav.word(), lx.id);
            // The filter's window and level are the search; the parent's own parameters stay at its pick.
            if let Some(mut d) = draft(parent, "repair", l, why, move |rule, _fixed, grid, bpd| regime_filter(rule, grid, bpd, fav)) {
                d.lessons.push(lx.id.clone());
                out.push(d);
            }
        }
    }

    // Profit leans on a few days: skip the most volatile tenth of days, where outsized days cluster.
    let conc = hit(&CONCENTRATION_CHECKS);
    if !conc.is_empty() {
        let why = format!("{label} failed only {} ({}); {} says its profit leans on a few days. Outsized days cluster in market-wide volatility spikes: skipping the most volatile tenth of days tests whether the edge is there without them.", failed.join(", "), l.id, conc.join(", "));
        if let Some(d) = draft(parent, "repair", l, why, |rule, fixed, grid, bpd| {
            if has_filter(rule, Feature::MktRvolPct) {
                return false;
            }
            rule.filter.push(Cond { feature: Feature::MktRvolPct, n: Param::Num(bpd), op: Op::Lt, level: Param::Num(0.9) });
            widen_window(rule, fixed, grid);
            true
        }) {
            out.push(d);
        }
    }

    // Fragile to execution: hold through a repeated signal instead of closing and reopening, so it trades less.
    let exec = hit(&EXECUTION_CHECKS);
    if !exec.is_empty() {
        let why = format!("{label} failed only {} ({}); {} says it does not survive slower or dearer fills. Extending a position on a repeated signal, instead of closing and reopening it, cuts round trips and the costs they carry.", failed.join(", "), l.id, exec.join(", "));
        if let Some(d) = draft(parent, "repair", l, why, |rule, fixed, grid, _| {
            if rule.extend || rule.every.is_some() {
                return false;
            }
            rule.extend = true;
            widen_window(rule, fixed, grid);
            true
        }) {
            out.push(d);
        }
    }
    out
}

fn from_lesson(l: &Lesson, inp: &Inputs, cfg: &EvolveCfg) -> Vec<Draft> {
    match &l.data {
        Finding::Gate { failed, .. } => repairs(l, failed, inp, cfg),
        _ => from_lesson_one(l, inp).into_iter().collect(),
    }
}

fn from_lesson_one(l: &Lesson, inp: &Inputs) -> Option<Draft> {
    let parent = inp.parents.get(&l.subject)?;
    match &l.data {
        Finding::Regime { favoured, mean_in_bps, mean_out_bps, .. } => {
            let why = format!("{} earned {mean_in_bps:+.1} bps/day with {} and {mean_out_bps:+.1} without ({}). Trading only in that state should keep the first and drop the second, if the state itself is what matters.", parent.family.label(), favoured.word(), l.id);
            let fav = *favoured;
            draft(parent, "condition", l, why, move |rule, _fixed, grid, bpd| regime_filter(rule, grid, bpd, fav))
        }
        Finding::Side { worse, short_bps, long_bps, .. } if worse == "short" => {
            let why = format!("{}'s shorts lost {short_bps:+.1} bps per trade while its longs made {long_bps:+.1} ({}). The long side alone is the hypothesis.", parent.family.label(), l.id);
            draft(parent, "restrict", l, why, |rule, fixed, grid, _| {
                if rule.short == Short::None || fixed.get("shorts").is_some_and(|s| *s < 0.5) {
                    return false;
                }
                rule.short = Short::None;
                fixed.remove("shorts");
                widen_window(rule, fixed, grid);
                true
            })
        }
        Finding::Hold { worse, worse_bps, better, better_bps, .. } => {
            let why = format!("{}'s trades held {worse} lost {worse_bps:+.1} bps each while those held {better} made {better_bps:+.1} ({}). Shorter holds should keep the second.", parent.family.label(), l.id);
            draft(parent, "restrict", l, why, |rule, fixed, grid, _| {
                let (key, v) = match (fixed.get("hold"), fixed.get("hold_days")) {
                    (Some(h), _) => ("hold", *h),
                    (None, Some(d)) => ("hold_days", *d),
                    (None, None) => ("hold_days", 30.0),
                };
                let vals: Vec<f64> = if key == "hold" { vec![(v / 4.0).round().max(1.0), (v / 2.0).round().max(1.0)] } else { vec![v / 4.0, v / 2.0] };
                if vals[0] == vals[1] {
                    return false;
                }
                fixed.remove(key);
                grid.insert(key.into(), vals);
                widen_window(rule, fixed, grid);
                true
            })
        }
        Finding::BroadLoss { mean_bps, coins_positive, coins, .. } => {
            let why = format!("{} lost {mean_bps:+.1} bps/day with {coins_positive} of {coins} coins positive ({}). A rule that loses this broadly may be wrong-way: its opposite is tested as a separate hypothesis, with the search charged again.", parent.family.label(), l.id);
            draft(parent, "invert", l, why, |rule, fixed, grid, _| {
                rule.long = rule.long.iter().map(Cond::inverted).collect();
                rule.exit = rule.exit.iter().map(Cond::inverted).collect();
                widen_window(rule, fixed, grid);
                true
            })
        }
        Finding::Redundancy { other, corr, .. } if corr.abs() <= 0.3 => {
            let b = inp.parents.get(other)?;
            let me = parent.family.family.as_str();
            if ancestors(inp.lineage, me).iter().any(|a| a == other) || ancestors(inp.lineage, other).iter().any(|a| a == me) {
                return None;
            }
            if b.family.interval != parent.family.interval || parent.mean_bps.is_none_or(|m| m <= 0.0) || b.mean_bps.is_none_or(|m| m <= 0.0) {
                return None;
            }
            let (b_rule, b_params, b_exact) = base(b)?;
            let why = format!("{} and {} both made money on the design window with a daily correlation of {corr:+.2} ({}). Entering only when both agree tests whether each confirms the other.", parent.family.label(), b.family.label(), l.id);
            let mut d = draft(parent, "recombine", l, why, |rule, fixed, grid, _| {
                let before = rule.long.len() + rule.filter.len();
                rule.long.extend(renamed(&b_rule.long, "b_"));
                rule.filter.extend(renamed(&b_rule.filter, "b_"));
                for r in b_rule.refs() {
                    if let Some(v) = b_params.get(&r) {
                        fixed.insert(format!("b_{r}"), *v);
                    }
                }
                // Agreement with conditions the parent already has is no agreement at all.
                dedupe(rule, fixed);
                if rule.long.len() + rule.filter.len() == before {
                    return false;
                }
                widen_window(rule, fixed, grid);
                rule.validate().is_ok()
            })?;
            d.approximate |= !b_exact;
            Some(d)
        }
        Finding::Params { flat, moving } if parent.family.rule.is_none() && !flat.is_empty() && !moving.is_empty() => {
            let why = format!("On {}'s grid, {} made no difference ({}). The same rule as blocks with those fixed at the pick searches fewer specs, so the reality check has fewer lucky draws to forgive.", parent.family.label(), flat.join(", "), l.id);
            let pick = parent.pick.clone()?;
            let fam_grid = parent.family.grid.clone();
            let moving: Vec<String> = moving.iter().map(|m| m.name.clone()).collect();
            draft(parent, "shrink", l, why, move |rule, fixed, grid, _| {
                let refs = rule.refs();
                for m in &moving {
                    if !(refs.contains(m) || POLICY.contains(&m.as_str())) {
                        continue;
                    }
                    let (Some(vals), Some(at)) = (fam_grid.get(m), pick.params.get(m)) else { continue };
                    let i = vals.iter().position(|v| v == at).unwrap_or(0);
                    let near: Vec<f64> = vals[i.saturating_sub(1)..(i + 2).min(vals.len())].to_vec();
                    if near.len() > 1 {
                        fixed.remove(m);
                        grid.insert(m.clone(), near);
                    }
                }
                !grid.is_empty()
            })
        }
        _ => None,
    }
}

/// Feature pairings for `explore`, tried in a seeded order.
fn explorations(bpd: f64) -> Vec<(&'static str, Rule, BTreeMap<String, Vec<f64>>, BTreeMap<String, f64>)> {
    let c = |feature, n: &str, op, level: Param| Cond { feature, n: Param::Ref(n.into()), op, level };
    let r = |long: Vec<Cond>, filter: Vec<Cond>| Rule { long, short: Short::Mirror, filter, exit: Vec::new(), exit_opposite: false, every: None, extend: false };
    let g = |kv: &[(&str, Vec<f64>)]| kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<BTreeMap<_, _>>();
    let f = |kv: &[(&str, f64)]| kv.iter().map(|(k, v)| (k.to_string(), *v)).collect::<BTreeMap<_, _>>();
    vec![
        ("a breakout on a volume surge", r(vec![c(Feature::Channel, "$n", Op::Gt, Param::Num(1.0)), c(Feature::VolRatio, "$n", Op::Ge, Param::Ref("$vol_mult".into()))], Vec::new()), g(&[("n", vec![2.0 * bpd, 4.0 * bpd]), ("vol_mult", vec![1.5, 2.5])]), f(&[("hold_days", 3.0), ("stop", 600.0)])),
        ("a fade against the basket when the market is calm", r(vec![c(Feature::ResidZ, "$n", Op::Le, Param::Ref("$z*-1".into()))], vec![c(Feature::MktRvolPct, "$vol_n", Op::Lt, Param::Num(0.5))]), g(&[("n", vec![bpd, 2.0 * bpd]), ("z", vec![2.0, 2.5])]), f(&[("vol_n", bpd), ("hold", (bpd / 4.0).max(1.0)), ("stop", 500.0)])),
        ("cross-sectional momentum on the clock", {
            let mut x = r(vec![c(Feature::XsRet, "$look", Op::Ge, Param::Ref("$cut".into()))], Vec::new());
            x.every = Some(Param::Ref("$hold".into()));
            x.extend = true;
            x
        }, g(&[("look", vec![3.0 * bpd, 7.0 * bpd]), ("cut", vec![0.6, 0.8])]), f(&[("hold", bpd)])),
        ("crowded funding faded", r(vec![c(Feature::XsFunding, "$look", Op::Le, Param::Ref("$cut*-1".into())), c(Feature::Ret, "$look", Op::Le, Param::Num(0.0))], Vec::new()), g(&[("look", vec![24.0, 72.0]), ("cut", vec![0.6, 0.8])]), f(&[("hold", bpd), ("stop", 800.0)])),
        ("a trend entry only in a rising market", r(vec![c(Feature::Ret, "$n", Op::Gt, Param::Ref("$min_ret".into()))], vec![c(Feature::MktRet, "$mkt_n", Op::Gt, Param::Num(0.0))]), g(&[("n", vec![7.0 * bpd, 14.0 * bpd]), ("min_ret", vec![0.1, 0.2])]), f(&[("mkt_n", 30.0 * bpd), ("hold_days", 7.0), ("shorts", 0.0), ("stop", 1000.0)])),
    ]
}

fn spec_ids(fam: &Family) -> Vec<String> {
    fam.specs().iter().map(Spec::id).collect()
}

pub fn propose(inp: &Inputs, cfg: &EvolveCfg) -> Outcome {
    let mut out = Outcome { proposals: Vec::new(), messages: Vec::new() };
    let proposed_keys: HashSet<&str> = inp.lineage.iter().filter(|e| e.event == "proposed").map(|e| e.key.as_str()).collect();
    // A rule's verdict file is named by its id, so the same rule on another grid would overwrite it.
    let proposed_rules: HashSet<&str> = inp.lineage.iter().filter(|e| e.event == "proposed").map(|e| e.family.as_str()).collect();
    // A rule written differently but trading the same bars is the same strategy: it would
    // charge its grid again and could spend a second lockbox opening on one result.
    let same_as: HashMap<String, String> = inp.parents.values().filter_map(|p| Some((p.family.rule.as_ref()?.same_id(), p.family.label().to_string()))).collect();
    let dying: HashSet<&str> = inp.lessons.lessons.iter().filter(|l| matches!(l.data, Finding::DyingBranch { .. }) && !l.anecdote).map(|l| l.subject.as_str()).collect();
    let mut usable: Vec<&Lesson> = inp.lessons.lessons.iter().filter(|l| !l.anecdote && !l.operator_only).collect();
    // Closest parent first (fewest gate checks failed), then the strongest lesson about it.
    let dist = |l: &Lesson| inp.parents.get(&l.subject).map_or(usize::MAX, Parent::distance);
    usable.sort_by(|a, b| dist(a).cmp(&dist(b)).then(b.strength.total_cmp(&a.strength)).then(a.id.cmp(&b.id)));
    let mut drafts: Vec<Draft> = Vec::new();
    for l in &usable {
        if dying.contains(root_of(inp.lineage, &l.subject).as_str()) {
            continue;
        }
        drafts.extend(from_lesson(l, inp, cfg));
    }
    // One proposal per parent first, then the rest in the same order: a cycle should not
    // spend its budget on one family.
    let mut firsts = Vec::new();
    let mut rest = Vec::new();
    let mut parents_seen: HashSet<String> = HashSet::new();
    for d in drafts {
        if parents_seen.insert(d.parent.clone().unwrap_or_default()) { firsts.push(d) } else { rest.push(d) }
    }
    let drafts: Vec<Draft> = firsts.into_iter().chain(rest).collect();
    // Explore fills what the lessons leave, in an order fixed by the seed and generation.
    let interval = inp.parents.values().next().map(|p| p.family.interval.clone()).unwrap_or_else(|| "1h".into());
    let mut ex = explorations(bars_per_day(&interval));
    let mut rng = Rng::new(inp.seed ^ (inp.generation as u64).wrapping_mul(0x9E37_79B9));
    let k = rng.below(ex.len());
    ex.rotate_left(k);

    let mut trials = 0usize;
    let mut seen: HashSet<String> = HashSet::new();
    let mut taken = 0usize;
    let explore_drafts = ex.into_iter().map(|(what, rule, grid, fixed)| Draft {
        rule,
        fixed,
        grid,
        interval: interval.clone(),
        parent: None,
        parent_label: String::new(),
        parent_spec: String::new(),
        operator: "explore",
        lessons: Vec::new(),
        why: format!("Budget was left after the lessons were used: {what}, a pairing of blocks not yet in the lineage."),
        approximate: false,
    });
    for d in drafts.into_iter().chain(explore_drafts) {
        if out.proposals.len() >= cfg.proposals {
            break;
        }
        let exploring = d.operator == "explore";
        if exploring && out.proposals.len() + 1 > cfg.proposals {
            break;
        }
        let mut fixed = d.fixed.clone();
        for k in d.grid.keys() {
            fixed.remove(k);
        }
        let mut rule = d.rule.clone();
        dedupe(&mut rule, &fixed);
        let refs = rule.refs();
        fixed.retain(|k, _| refs.contains(k) || POLICY.contains(&k.as_str()));
        let mut fam = Family { family: String::new(), interval: d.interval.clone(), hypothesis: String::new(), probe: None, venue: None, grid: d.grid.clone(), fixed, rule: Some(rule.clone()), name: None, parent: d.parent.clone(), generation: Some(inp.generation), operator: Some(d.operator.into()), lessons: d.lessons.clone(), reason: String::new() };
        if let Err(e) = fam.check() {
            out.messages.push(format!("{} from {}: not a valid family ({e:#}); skipped", d.operator, d.lessons.join(", ")));
            continue;
        }
        let specs = fam.specs().len();
        if specs > cfg.specs_per_proposal {
            out.messages.push(format!("{} of {}: {specs} specs is over the {} a proposal may have; skipped", d.operator, d.parent_label, cfg.specs_per_proposal));
            continue;
        }
        let same = rule.same_id();
        if let Some(twin) = same_as.get(&same).cloned().or_else(|| out.proposals.iter().find(|p| p.family.rule.as_ref().is_some_and(|r| r.same_id() == same)).map(|p| p.family.label().to_string())) {
            if !exploring {
                out.messages.push(format!("{} of {} ({}): the same strategy as {twin}, written differently; skipped", d.operator, d.parent_label, d.lessons.join(", ")));
            }
            continue;
        }
        let key = format!("{}|{}|{}", fam.family, serde_json::to_string(&fam.grid).unwrap_or_default(), serde_json::to_string(&fam.fixed).unwrap_or_default());
        if proposed_rules.contains(fam.family.as_str()) || out.proposals.iter().any(|p| p.family.family == fam.family) {
            if !exploring {
                out.messages.push(format!("{} of {} ({}): the same rule was already proposed; another grid on it would overwrite its verdict", d.operator, d.parent_label, d.lessons.join(", ")));
            }
            continue;
        }
        if proposed_keys.contains(key.as_str()) || !seen.insert(key.clone()) {
            if !exploring {
                out.messages.push(format!("{} of {} ({}): already proposed in an earlier generation", d.operator, d.parent_label, d.lessons.join(", ")));
            }
            continue;
        }
        let study = format!("{}-{}", inp.research_venue, fam.interval);
        let charged = inp.charged.get(&study);
        let new = spec_ids(&fam).iter().filter(|id| charged.is_none_or(|c| !c.contains(*id))).count();
        if new == 0 {
            out.messages.push(format!("{} of {}: every spec is already in the ledger, nothing new to learn", d.operator, d.parent_label));
            continue;
        }
        if let Some(budget) = inp.loop_budget.map(|b| if inp.budget_since.is_some() { b } else { cfg.budget_for(&study, b) }) {
            let counts = |e: &&Event| e.event == "tested" && e.study == study && inp.budget_since.is_none_or(|t| e.at.is_some_and(|at| at >= t)) && inp.era.is_none_or(|t| e.at.is_some_and(|at| at >= t));
            let spent = inp.lineage.iter().filter(counts).map(|e| e.trials).sum::<usize>() + out.proposals.iter().filter(|p| p.study == study).map(|p| p.new_trials).sum::<usize>();
            if spent + new > budget {
                let span = if inp.budget_since.is_some() { "this run's" } else { "its" };
                out.messages.push(format!("loop budget: {study} has {spent} of {span} {budget} loop trials spent; {} of {} ({new} more) would pass it", d.operator, if d.parent_label.is_empty() { "explore" } else { &d.parent_label }));
                continue;
            }
        }
        if trials + new > cfg.trials_per_cycle {
            out.messages.push(format!("trial budget: {} of {} would bring this cycle to {} trials, over the {} allowed. The cycle stops at {} proposals; raise `[evolve] trials_per_cycle` only on purpose.", d.operator, if d.parent_label.is_empty() { "explore" } else { &d.parent_label }, trials + new, cfg.trials_per_cycle, out.proposals.len()));
            break;
        }
        trials += new;
        let name = lineage::next_name(inp.lineage, taken);
        taken += 1;
        let hyp_id = format!("H-{}", name.to_uppercase());
        fam.name = Some(name.clone());
        fam.hypothesis = hyp_id.clone();
        let rule_text = rule.describe();
        fam.reason = format!("{} {}", d.why, if d.approximate { "The parent is hand-written and its block form is an approximation, so this child is not a strict variant of it." } else { "" }).trim().to_string();
        let baseline = if d.parent_spec.is_empty() { "no edge: the gate's own null (reality check, deflated Sharpe, time-shifted copies)".to_string() } else { format!("the parent's pick `{}`", d.parent_spec) };
        let hypothesis = Hypothesis {
            id: hyp_id,
            statement: format!("{name} ({}): {rule_text}; earns after the search is paid for", d.operator),
            change: format!("new composed family {} (families/gen/{name}.toml), {specs} specs; {}", fam.family, if d.lessons.is_empty() { "exploration".to_string() } else { format!("lessons {}", d.lessons.join(", ")) }),
            metric: "gate verdict on the design window, including the generation-wide reality check; then the lockbox and paper days".into(),
            baseline,
            pass_if: "PROMOTE_TO_PAPER from `lab compare` and not 0.9-correlated with an incumbent".into(),
            invalidate_if: "REJECT, or REVISE with the generation-wide reality check failing".into(),
            status: "proposed".into(),
            result: String::new(),
        };
        out.proposals.push(Proposal { family: fam, hypothesis, study, new_trials: new, specs, key, approximate: d.approximate });
    }
    if out.proposals.is_empty() {
        out.messages.push("nothing to propose: no usable lesson led to a new family and every exploration is already in the lineage".into());
    }
    out
}

/// The family file of a proposal.
pub fn family_toml(p: &Proposal) -> Result<String> {
    let mut s = String::new();
    s.push_str(&format!("# {} — generation {} — {}\n", p.family.label(), p.family.generation.unwrap_or(0), p.family.operator.as_deref().unwrap_or("")));
    for line in textwrap(&p.family.reason, 88) {
        s.push_str(&format!("# {line}\n"));
    }
    s.push_str(&format!("# Rule: {}\n# Written by `lab propose`. The family name is the rule's id: editing the rule makes a new family.\n\n", p.family.rule.as_ref().map(Rule::describe).unwrap_or_default()));
    s.push_str(&toml::to_string(&p.family)?);
    Ok(s)
}

fn textwrap(s: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for w in s.split_whitespace() {
        if !line.is_empty() && line.len() + w.len() + 1 > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(w);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// The `[[hypothesis]]` block appended to the register.
pub fn hypothesis_toml(h: &Hypothesis) -> Result<String> {
    #[derive(Serialize)]
    struct File<'a> {
        hypothesis: [&'a Hypothesis; 1],
    }
    Ok(toml::to_string(&File { hypothesis: [h] })?)
}

#[cfg(test)]
mod tests {
    use super::super::learn::{self, Lesson};
    use super::*;

    fn lesson(id: &str, subject: &str, strength: f64, data: Finding) -> Lesson {
        Lesson { id: id.into(), kind: "x".into(), subject: subject.into(), label: subject.into(), study: "binance-1h".into(), finding: String::new(), evidence: Vec::new(), strength, n: 500, anecdote: false, operator_only: false, data }
    }

    fn parents() -> BTreeMap<String, Parent> {
        let trend = Family::parse("family = \"trend\"\ninterval = \"1h\"\n[grid]\nn = [24, 96, 384]\nout_frac = [0.25, 0.5]\nshorts = [0, 1]\nstop = [0, 800]\n").unwrap();
        let mr = Family::parse("family = \"meanrev\"\ninterval = \"1h\"\n[grid]\nn = [48, 96]\nz = [1.5, 2.0]\nhold = [12, 24]\n").unwrap();
        let mut out = BTreeMap::new();
        out.insert("trend".into(), Parent { family: trend, pick: Some(Spec::new("trend", &[("n", 96.0), ("out_frac", 0.25), ("shorts", 1.0), ("stop", 800.0)])), mean_bps: Some(10.0), study: "binance-1h".into(), failed: None });
        out.insert("meanrev".into(), Parent { family: mr, pick: Some(Spec::new("meanrev", &[("n", 96.0), ("z", 2.0), ("hold", 24.0)])), mean_bps: Some(5.0), study: "binance-1h".into(), failed: None });
        out
    }

    fn lessons() -> Lessons {
        Lessons {
            generated: None,
            sources: Vec::new(),
            notes: Vec::new(),
            lessons: vec![
                lesson("L01", "trend", 0.9, Finding::Regime { favoured: Regime::MarketUp, mean_in_bps: 20.0, mean_out_bps: -8.0, days_in: 300, days_out: 300, t: 2.7 }),
                lesson("L02", "trend", 0.6, Finding::Side { worse: "short".into(), long_bps: 30.0, short_bps: -20.0, long_trades: 200, short_trades: 150 }),
                lesson("L03", "meanrev", 0.8, Finding::BroadLoss { mean_bps: -20.0, coins_positive: 0, coins: 20, quarters_positive: 0, quarters: 9 }),
                lesson("L04", "trend", 0.7, Finding::Redundancy { other: "meanrev".into(), corr: 0.1, days: 600 }),
                Lesson { anecdote: true, ..lesson("L05", "trend", 1.0, Finding::Hold { worse: "> 9 bars".into(), better: "≤ 3 bars".into(), worse_bps: -9.0, better_bps: 9.0, worse_trades: 3 }) },
            ],
        }
    }

    fn run(lineage: &[Event], charged: &BTreeMap<String, HashSet<String>>, cfg: &EvolveCfg) -> Outcome {
        let (l, p) = (lessons(), parents());
        propose(&Inputs { lessons: &l, parents: &p, lineage, charged, research_venue: "binance", generation: 1, seed: 7, loop_budget: None, budget_since: None, era: None }, cfg)
    }

    #[test]
    fn lessons_become_valid_families_with_hypotheses() {
        let out = run(&[], &BTreeMap::new(), &EvolveCfg { trials_per_cycle: 100, ..EvolveCfg::default() });
        let ops: Vec<&str> = out.proposals.iter().map(|p| p.family.operator.as_deref().unwrap()).collect();
        assert_eq!(&ops[..4], &["condition", "invert", "recombine", "restrict"], "{:?}", out.messages);
        let cond = &out.proposals[0];
        assert_eq!(cond.family.name.as_deref(), Some("g0001"));
        assert_eq!(cond.family.lessons, vec!["L01".to_string()]);
        assert!(cond.family.rule.as_ref().unwrap().filter.iter().any(|c| c.feature == Feature::MktRet));
        assert!(cond.specs <= 12 && cond.new_trials == cond.specs);
        assert!(!cond.approximate, "trend's block form is exact");
        assert!(out.proposals[1].approximate, "meanrev's is not");
        // The invert child of a fade is a momentum rule.
        assert_eq!(out.proposals[1].family.rule.as_ref().unwrap().long[0].op, Op::Ge);
        // The file round-trips through the loader and registers its rule.
        for p in &out.proposals {
            let text = family_toml(p).unwrap();
            let back = Family::parse(&text).unwrap_or_else(|e| panic!("{e:#}\n{text}"));
            assert_eq!(back.family, p.family.family);
            assert_eq!(back.specs().len(), p.specs);
            assert!(compose::lookup(&back.family).is_some());
            let h = hypothesis_toml(&p.hypothesis).unwrap();
            assert!(h.starts_with("[[hypothesis]]"), "{h}");
        }
        // The anecdote (L05) was not acted on.
        assert!(out.proposals.iter().all(|p| !p.family.lessons.contains(&"L05".to_string())));
    }

    #[test]
    fn deterministic_budgeted_and_never_twice() {
        let cfg = EvolveCfg { trials_per_cycle: 100, ..EvolveCfg::default() };
        let a = run(&[], &BTreeMap::new(), &cfg);
        let b = run(&[], &BTreeMap::new(), &cfg);
        assert_eq!(a.proposals.iter().map(|p| &p.key).collect::<Vec<_>>(), b.proposals.iter().map(|p| &p.key).collect::<Vec<_>>());
        // A tight budget stops the cycle and says so.
        let tight = run(&[], &BTreeMap::new(), &EvolveCfg { trials_per_cycle: 5, ..EvolveCfg::default() });
        assert!(tight.proposals.iter().map(|p| p.new_trials).sum::<usize>() <= 5);
        assert!(tight.messages.iter().any(|m| m.contains("trial budget")), "{:?}", tight.messages);
        // What the lineage already holds is not proposed again.
        let lineage: Vec<Event> = a.proposals.iter().map(|p| Event { event: "proposed".into(), family: p.family.family.clone(), key: p.key.clone(), name: p.family.name.clone().unwrap(), generation: 1, ..Event::default() }).collect();
        let c = run(&lineage, &BTreeMap::new(), &cfg);
        for p in &c.proposals {
            assert!(!a.proposals.iter().any(|q| q.key == p.key));
        }
        assert!(c.proposals.first().is_some_and(|p| p.family.name.as_deref() == Some("g0005")));
        // Specs already charged are not new trials.
        let mut charged = BTreeMap::new();
        charged.insert("binance-1h".to_string(), a.proposals[0].family.specs().iter().map(Spec::id).collect::<HashSet<_>>());
        let d = run(&[], &charged, &cfg);
        assert!(d.proposals.iter().all(|p| p.key != a.proposals[0].key));
    }

    #[test]
    fn the_loop_budget_counts_what_the_lineage_already_spent() {
        let cfg = EvolveCfg { trials_per_cycle: 100, ..EvolveCfg::default() };
        let (l, p) = (lessons(), parents());
        let spent = vec![Event { event: "tested".into(), family: "cx".into(), study: "binance-1h".into(), trials: 195, ..Event::default() }];
        let out = propose(&Inputs { lessons: &l, parents: &p, lineage: &spent, charged: &BTreeMap::new(), research_venue: "binance", generation: 2, seed: 7, loop_budget: Some(200), budget_since: None, era: None }, &cfg);
        assert!(out.proposals.iter().map(|x| x.new_trials).sum::<usize>() <= 5, "{:?}", out.proposals.iter().map(|x| x.new_trials).collect::<Vec<_>>());
        assert!(out.messages.iter().any(|m| m.starts_with("loop budget")), "{:?}", out.messages);
    }

    #[test]
    fn a_cap_per_run_ignores_what_earlier_runs_spent() {
        let cfg = EvolveCfg { trials_per_cycle: 100, ..EvolveCfg::default() };
        let (l, p) = (lessons(), parents());
        let start = chrono::Utc::now();
        let old = Event { event: "tested".into(), family: "cx".into(), study: "binance-1h".into(), trials: 5000, at: Some(start - chrono::Duration::hours(1)), ..Event::default() };
        let run = |lineage: &[Event]| propose(&Inputs { lessons: &l, parents: &p, lineage, charged: &BTreeMap::new(), research_venue: "binance", generation: 2, seed: 7, loop_budget: Some(20), budget_since: Some(start), era: None }, &cfg);
        let fresh = run(std::slice::from_ref(&old));
        let charged = fresh.proposals.iter().filter(|x| x.study == "binance-1h").map(|x| x.new_trials).sum::<usize>();
        assert!(charged > 0 && charged <= 20, "a spent lifetime must not block a new run, and the run stays under its cap: {charged}");
        let this_run = Event { trials: 18, at: Some(start + chrono::Duration::minutes(2)), ..old.clone() };
        let later = run(&[old, this_run]);
        assert!(later.proposals.iter().filter(|x| x.study == "binance-1h").map(|x| x.new_trials).sum::<usize>() <= 2);
    }

    #[test]
    fn dying_branches_are_left_alone() {
        let (mut l, p) = (lessons(), parents());
        l.lessons.push(lesson("L09", "trend", 1.0, Finding::DyingBranch { tested: 3, rejected: 3 }));
        let out = propose(&Inputs { lessons: &l, parents: &p, lineage: &[], charged: &BTreeMap::new(), research_venue: "binance", generation: 1, seed: 7, loop_budget: None, budget_since: None, era: None }, &EvolveCfg { trials_per_cycle: 100, ..EvolveCfg::default() });
        assert!(out.proposals.iter().all(|p| p.family.parent.as_deref() != Some("trend")));
        let _ = learn::root_of;
    }

    fn with_failed(p: &mut BTreeMap<String, Parent>, name: &str, failed: &[&str]) {
        p.get_mut(name).unwrap().failed = Some(failed.iter().map(|x| x.to_string()).collect());
    }

    #[test]
    fn the_closest_parent_gets_the_first_slot() {
        let cfg = EvolveCfg { trials_per_cycle: 100, ..EvolveCfg::default() };
        let l = lessons();
        let mut p = parents();
        // trend's lessons are stronger, but meanrev is one check from passing.
        with_failed(&mut p, "trend", &["walk-forward", "reality check", "deflated Sharpe", "quarters", "coins"]);
        with_failed(&mut p, "meanrev", &["plateau"]);
        let out = propose(&Inputs { lessons: &l, parents: &p, lineage: &[], charged: &BTreeMap::new(), research_venue: "binance", generation: 1, seed: 7, loop_budget: None, budget_since: None, era: None }, &cfg);
        assert_eq!(out.proposals[0].family.parent.as_deref(), Some("meanrev"), "{:?}", out.messages);
        // Unjudged parents go last.
        let mut q = parents();
        with_failed(&mut q, "trend", &["plateau"]);
        let out = propose(&Inputs { lessons: &l, parents: &q, lineage: &[], charged: &BTreeMap::new(), research_venue: "binance", generation: 1, seed: 7, loop_budget: None, budget_since: None, era: None }, &cfg);
        assert_eq!(out.proposals[0].family.parent.as_deref(), Some("trend"));
    }

    #[test]
    fn repair_aims_one_block_at_each_failed_check() {
        let cfg = EvolveCfg { trials_per_cycle: 200, proposals: 8, ..EvolveCfg::default() };
        let failed = ["quarters", "best days removed", "costs ×2"];
        let mut p = parents();
        with_failed(&mut p, "trend", &failed);
        let gate = |id: &str, f: &[&str]| lesson(id, "trend", 1.0, Finding::Gate { verdict: "REVISE".into(), failed: f.iter().map(|x| x.to_string()).collect(), fatal: Vec::new() });
        let lean = lesson("L11", "trend", 0.5, Finding::Lean { favoured: Regime::VolLow, mean_in_bps: 15.0, mean_out_bps: 4.0, days_in: 300, days_out: 300, t: 1.6 });
        let l = Lessons { generated: None, sources: Vec::new(), notes: Vec::new(), lessons: vec![gate("L10", &failed), lean.clone()] };
        let out = propose(&Inputs { lessons: &l, parents: &p, lineage: &[], charged: &BTreeMap::new(), research_venue: "binance", generation: 1, seed: 7, loop_budget: None, budget_since: None, era: None }, &cfg);
        let reps: Vec<&Proposal> = out.proposals.iter().filter(|x| x.family.operator.as_deref() == Some("repair")).collect();
        let base_rule = base(&p["trend"]).unwrap().0;
        assert_eq!(reps.len(), if base_rule.extend { 2 } else { 3 }, "{:?}", out.messages);
        // Time: a calm-market filter from the lean, citing both lessons.
        let time = reps[0];
        assert!(time.family.rule.as_ref().unwrap().filter.iter().any(|c| c.feature == Feature::MktRvolPct && c.op == Op::Lt));
        assert_eq!(time.family.lessons, vec!["L10".to_string(), "L11".to_string()]);
        // Concentration: the most volatile tenth of days skipped.
        assert!(reps[1].family.rule.as_ref().unwrap().filter.iter().any(|c| c.level == Param::Num(0.9)));
        // Every repair is a new rule, so it can never overwrite the parent's verdict.
        assert!(reps.iter().all(|r| r.family.family != "trend" && r.family.parent.as_deref() == Some("trend")));

        // Too far from passing, or only search-cost checks: no repair.
        let mut far = parents();
        with_failed(&mut far, "trend", &["quarters", "coins", "plateau", "walk-forward"]);
        let l2 = Lessons { generated: None, sources: Vec::new(), notes: Vec::new(), lessons: vec![gate("L10", &["quarters", "coins", "plateau", "walk-forward"]), lean.clone()] };
        let out = propose(&Inputs { lessons: &l2, parents: &far, lineage: &[], charged: &BTreeMap::new(), research_venue: "binance", generation: 1, seed: 7, loop_budget: None, budget_since: None, era: None }, &cfg);
        assert!(out.proposals.iter().all(|x| x.family.operator.as_deref() != Some("repair")));
        let mut cost = parents();
        with_failed(&mut cost, "trend", &["deflated Sharpe", "reality check"]);
        let l3 = Lessons { generated: None, sources: Vec::new(), notes: Vec::new(), lessons: vec![gate("L10", &["deflated Sharpe", "reality check"]), lean] };
        let out = propose(&Inputs { lessons: &l3, parents: &cost, lineage: &[], charged: &BTreeMap::new(), research_venue: "binance", generation: 1, seed: 7, loop_budget: None, budget_since: None, era: None }, &cfg);
        assert!(out.proposals.iter().all(|x| x.family.operator.as_deref() != Some("repair")));
    }

    #[test]
    fn repeated_conditions_are_dropped() {
        let c = |lvl: &str| Cond { feature: Feature::XsFunding, n: Param::Ref("$n".into()), op: Op::Le, level: Param::Ref(lvl.into()) };
        let mut rule = compose::template("trend").unwrap().0;
        rule.filter = vec![c("$lvl"), c("$b_lvl")];
        let fixed: BTreeMap<String, f64> = [("lvl".to_string(), -0.5), ("b_lvl".to_string(), -0.5)].into();
        assert_eq!(dedupe(&mut rule, &fixed), 1);
        assert_eq!(rule.filter.len(), 1);
        let other: BTreeMap<String, f64> = [("lvl".to_string(), -0.5), ("b_lvl".to_string(), -1.0)].into();
        rule.filter = vec![c("$lvl"), c("$b_lvl")];
        assert_eq!(dedupe(&mut rule, &other), 0, "different levels are different conditions");
    }

    #[test]
    fn the_same_rule_is_never_proposed_twice_and_budgets_are_per_study() {
        let cfg = EvolveCfg { trials_per_cycle: 100, ..EvolveCfg::default() };
        let (l, p) = (lessons(), parents());
        let a = run(&[], &BTreeMap::new(), &cfg);
        // Same rule ids, any grid: all skipped.
        let lineage: Vec<Event> = a.proposals.iter().map(|p| Event { event: "proposed".into(), family: p.family.family.clone(), key: "other grid".into(), name: "g0001".into(), generation: 1, ..Event::default() }).collect();
        let b = run(&lineage, &BTreeMap::new(), &cfg);
        assert!(b.proposals.iter().all(|q| a.proposals.iter().all(|p| p.family.family != q.family.family)));
        // A per-study override lifts the budget for that study only.
        let spent = vec![Event { event: "tested".into(), family: "cx".into(), study: "binance-1h".into(), trials: 199, ..Event::default() }];
        let tight = propose(&Inputs { lessons: &l, parents: &p, lineage: &spent, charged: &BTreeMap::new(), research_venue: "binance", generation: 2, seed: 7, loop_budget: Some(200), budget_since: None, era: None }, &cfg);
        let mut wide_cfg = cfg.clone();
        wide_cfg.auto_trial_budgets.insert("binance-1h".into(), 300);
        let wide = propose(&Inputs { lessons: &l, parents: &p, lineage: &spent, charged: &BTreeMap::new(), research_venue: "binance", generation: 2, seed: 7, loop_budget: Some(200), budget_since: None, era: None }, &wide_cfg);
        let on_1h = |o: &Outcome| o.proposals.iter().filter(|p| p.study == "binance-1h").map(|p| p.new_trials).sum::<usize>();
        assert!(on_1h(&tight) <= 1);
        assert!(on_1h(&wide) > on_1h(&tight));
    }
}
