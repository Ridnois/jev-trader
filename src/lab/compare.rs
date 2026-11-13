//! `lab compare`: judge a generation of proposals, and set them against what is
//! already there.
//!
//! No new judging code: each proposal goes through `gate::evaluate` exactly as
//! `lab validate` does. Two things are added because the proposals were chosen
//! together: every spec of every proposal is simulated and charged to the ledger
//! before any result is read, and each family is also judged by a reality check over
//! the union of the whole generation's specs on its study.
//!
//! Then each challenger's pick is compared with the incumbents (every strategy on
//! paper, and the pick of every family with a verdict on the same study) on common
//! design-window days: means, a block-bootstrap interval on the daily difference, and
//! the correlation. A challenger 0.9-correlated with an incumbent is a duplicate.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::gate::{self, Evaluation, Verdict};
use super::panel::Panel;
use super::search::{self, Trial, TrialRecord};
use super::sim::{self, SimCfg};
use super::stat;
use super::strategy::{self, Aux, Family, Spec};
use super::LabConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockboxRead {
    pub study: String,
    pub family: String,
    pub spec: String,
    pub at: chrono::DateTime<chrono::Utc>,
    pub passed: bool,
}

/// The trial ledger and the lockbox log: the real files, or memory (calibration).
pub struct Books {
    trials_path: Option<PathBuf>,
    lockbox_path: Option<PathBuf>,
    trials: Vec<TrialRecord>,
    lockbox: Vec<LockboxRead>,
}

impl Books {
    pub fn files(lab: &LabConfig) -> Self {
        Self { trials_path: Some(lab.trials_path()), lockbox_path: Some(lab.lockbox_path()), trials: Vec::new(), lockbox: Vec::new() }
    }

    pub fn memory() -> Self {
        Self { trials_path: None, lockbox_path: None, trials: Vec::new(), lockbox: Vec::new() }
    }

    /// Charges new specs; returns (new, total on record for the study).
    pub fn record(&mut self, study: &str, family: &str, trials: &[Trial]) -> (usize, usize) {
        if let Some(p) = &self.trials_path {
            return search::record_trials(p, study, family, trials);
        }
        let mut seen: HashSet<String> = self.trials.iter().filter(|r| r.study == study).map(|r| r.spec.clone()).collect();
        let mut new = 0;
        for t in trials {
            if seen.insert(t.spec.id()) {
                new += 1;
                self.trials.push(TrialRecord { study: study.into(), family: family.into(), spec: t.spec.id(), at: chrono::Utc::now(), days: t.daily.len(), trades: t.trades, sharpe_daily: stat::sharpe(&t.daily), mean_bps_day: stat::mean(&t.daily) * 1e4 });
            }
        }
        (new, seen.len())
    }

    pub fn rows(&self) -> Vec<TrialRecord> {
        match &self.trials_path {
            Some(p) => crate::status::read_jsonl(p),
            None => self.trials.clone(),
        }
    }

    pub fn total(&self, study: &str) -> usize {
        self.rows().iter().filter(|r| r.study == study).map(|r| r.spec.as_str()).collect::<HashSet<_>>().len()
    }

    /// Spec ids on record, by study.
    pub fn charged(&self) -> BTreeMap<String, HashSet<String>> {
        let mut out: BTreeMap<String, HashSet<String>> = BTreeMap::new();
        for r in self.rows() {
            out.entry(r.study).or_default().insert(r.spec);
        }
        out
    }

    fn lock_reads(&self) -> Vec<LockboxRead> {
        match &self.lockbox_path {
            Some(p) => crate::status::read_jsonl(p),
            None => self.lockbox.clone(),
        }
    }

    fn log_lock(&mut self, r: LockboxRead) {
        match &self.lockbox_path {
            Some(p) => crate::bot::append_jsonl(p, &r),
            None => self.lockbox.push(r),
        }
    }

    pub fn lockbox_openings(&self) -> usize {
        self.lock_reads().len()
    }
}

/// The gate on one family, with the search charged and the lockbox logged: what
/// `lab validate` does, and what `lab compare` does for each proposal.
#[allow(clippy::too_many_arguments)]
///
/// `looks` counts earlier looks at this study's lockbox that are not openings: for a
/// generated family, every generated family judged on the study before it. Each adds
/// one to the divisor of the lockbox's α, as an opening does.
pub fn validate_family(lab: &LabConfig, fam: &Family, full: &Panel, venue: Option<&Panel>, aux: &Aux, force: Option<&Spec>, design_trials: Option<Vec<Trial>>, generation: Option<&[Vec<f64>]>, looks: usize, books: &mut Books) -> Result<Evaluation> {
    let study = format!("{}-{}", full.venue, full.interval);
    let cfg = SimCfg::new(lab.capital, lab.costs.clone(), lab.sizing.clone());
    // Charge this grid first, so the count the gate uses includes it.
    let trials = match design_trials {
        Some(t) => t,
        None => {
            let design = full.head(super::design_bars(full, lab));
            let t = search::run_grid(&design, fam, aux, &cfg, lab.threads)?;
            books.record(&study, &fam.family, &t);
            t
        }
    };
    let ledger_trials = books.total(&study);
    let reads = books.lock_reads();
    let prior = |spec_id: &str| reads.iter().filter(|r| r.study == study && r.spec != spec_id).map(|r| r.spec.as_str()).collect::<HashSet<_>>().len();
    // The pick isn't known before the run; count every earlier opening, then correct below.
    let all_prior = prior("");
    let vaux = Aux::default();
    let inputs = |reads: usize, trials: Option<Vec<Trial>>| gate::Inputs { full, aux, venue: venue.map(|v| (v, &vaux)), fam, ledger_trials, lockbox_reads: reads + looks, force, design_trials: trials, generation };
    let mut ev = gate::evaluate(inputs(all_prior, Some(trials)), lab)?;
    if let (true, Some(p)) = (ev.lockbox_opened, ev.pick.clone()) {
        let own = prior(&p.id());
        if own != all_prior {
            // This same spec opened the lockbox before: looking again is not a new look.
            ev = gate::evaluate(inputs(own, None), lab)?;
        } else {
            books.log_lock(LockboxRead { study: study.clone(), family: fam.family.clone(), spec: p.id(), at: chrono::Utc::now(), passed: ev.verdict == Verdict::PromoteToPaper });
        }
    }
    if looks > 0 {
        ev.notes.push(format!("{looks} generated families were judged on this study before this one; each counts as a look at the lockbox, so its α is divided by {} plus the openings", looks + 1));
    }
    Ok(ev)
}

/// One proposal after the gate.
#[derive(Debug, Clone, Serialize)]
pub struct Judged {
    pub label: String,
    pub family: String,
    pub study: String,
    pub new_trials: usize,
    pub eval: Evaluation,
}

/// The panels a family is judged on: its research panel and, when there is one, the
/// trading venue's.
pub type PanelSource<'a> = dyn FnMut(&Family) -> Result<(Panel, Option<Panel>)> + 'a;

/// Simulates and charges every proposal, then judges each with the generation-wide
/// reality check of its study.
///
/// `judged_before` gives, per study, how many generated families were judged on it in
/// earlier cycles. A generated family's walk-forward is not truly out of sample (it was
/// shaped after the design window was seen), so the lockbox is its only clean test, and
/// that test has to carry the loop's multiple testing: the k-th generated family on a
/// study meets the lockbox at α / k (on top of the openings already counted).
pub fn judge_generation(lab: &LabConfig, fams: &[Family], panels: &mut PanelSource, judged_before: &dyn Fn(&str) -> usize, books: &mut Books) -> Result<Vec<Judged>> {
    let cfg = SimCfg::new(lab.capital, lab.costs.clone(), lab.sizing.clone());
    // 1. Every grid simulated and charged before any verdict is read.
    let mut runs = Vec::with_capacity(fams.len());
    for fam in fams {
        let (full, venue) = panels(fam)?;
        let study = format!("{}-{}", full.venue, full.interval);
        let design = full.head(super::design_bars(&full, lab));
        let trials = search::run_grid(&design, fam, &Aux::default(), &cfg, lab.threads)?;
        let (new, _) = books.record(&study, &fam.family, &trials);
        runs.push((full, venue, study, trials, new));
    }
    // 2. The union of the generation's specs on each study.
    let mut union: BTreeMap<String, Vec<Vec<f64>>> = BTreeMap::new();
    for (_, _, study, trials, _) in &runs {
        union.entry(study.clone()).or_default().extend(trials.iter().map(|t| t.daily.clone()));
    }
    // 3. The gate on each, as `lab validate` runs it.
    let mut out = Vec::with_capacity(fams.len());
    let mut in_cycle: BTreeMap<String, usize> = BTreeMap::new();
    for (fam, (full, venue, study, trials, new)) in fams.iter().zip(runs) {
        let k = in_cycle.entry(study.clone()).or_default();
        let looks = judged_before(&study) + *k;
        *k += 1;
        let ev = validate_family(lab, fam, &full, venue.as_ref(), &Aux::default(), None, Some(trials), union.get(&study).map(Vec::as_slice), looks, books)?;
        out.push(Judged { label: fam.label().to_string(), family: fam.family.clone(), study, new_trials: new, eval: ev });
    }
    Ok(out)
}

/// A challenger against one incumbent on common days.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pairing {
    pub challenger: String,
    pub incumbent: String,
    pub incumbent_spec: String,
    pub days: usize,
    pub challenger_bps: f64,
    pub incumbent_bps: f64,
    /// Challenger minus incumbent, bps/day, with its 95% block-bootstrap interval.
    pub diff_bps: f64,
    pub lo_bps: f64,
    pub hi_bps: f64,
    pub corr: f64,
    pub duplicate: bool,
}

/// Paired comparison of two daily series of equal length, from the first day either trades.
pub fn pair(challenger: &str, incumbent: &str, incumbent_spec: &str, a: &[f64], b: &[f64], lab: &LabConfig) -> Option<Pairing> {
    let n = a.len().min(b.len());
    let first = (0..n).find(|d| a[*d] != 0.0 || b[*d] != 0.0)?;
    let (a, b) = (&a[first..n], &b[first..n]);
    if a.len() < 30 {
        return None;
    }
    let diff: Vec<f64> = a.iter().zip(b).map(|(x, y)| x - y).collect();
    let bm = stat::boot_mean(&diff, lab.gate.block_days, lab.gate.boot_iters.min(1000), lab.seed ^ 0xC0);
    let corr = crate::stats::pearson(a, b).unwrap_or(0.0);
    Some(Pairing { challenger: challenger.into(), incumbent: incumbent.into(), incumbent_spec: incumbent_spec.into(), days: a.len(), challenger_bps: stat::mean(a) * 1e4, incumbent_bps: stat::mean(b) * 1e4, diff_bps: bm.mean * 1e4, lo_bps: bm.lo * 1e4, hi_bps: bm.hi * 1e4, corr, duplicate: corr >= 0.9 })
}

/// Each challenger's pick against each incumbent spec, on the design window of the
/// challenger's study. Incumbents are re-simulated (they were charged when found).
pub fn versus_incumbents(lab: &LabConfig, design: &Panel, challengers: &[(String, Spec)], incumbents: &[(String, Spec)]) -> Vec<Pairing> {
    let cfg = SimCfg::new(lab.capital, lab.costs.clone(), lab.sizing.clone());
    let run = |s: &Spec| strategy::plan(s, design, &Aux::default()).ok().map(|p| sim::simulate(design, &p, &cfg).daily);
    let inc: Vec<(String, Spec, Vec<f64>)> = incumbents.iter().filter_map(|(l, s)| run(s).map(|d| (l.clone(), s.clone(), d))).collect();
    let mut out = Vec::new();
    for (label, spec) in challengers {
        let Some(a) = run(spec) else { continue };
        for (il, is, b) in &inc {
            if is.id() == spec.id() {
                continue;
            }
            if let Some(p) = pair(label, il, &is.id(), &a, b, lab) {
                out.push(p);
            }
        }
    }
    out
}

/// Two paper strategies, parent and child, on the days both were up.
pub fn forward_pairs(daily: &[crate::trade::DailyRow], strategies: &[crate::trade::Loaded], lab: &LabConfig) -> Vec<Pairing> {
    let mut out = Vec::new();
    for child in strategies {
        let Some(parent) = child.promoted.parent.as_deref() else { continue };
        for p in strategies.iter().filter(|s| s.name != child.name && (s.promoted.card.spec.family == parent || s.promoted.label.as_deref() == Some(parent))) {
            let days: BTreeMap<&str, (Option<f64>, Option<f64>)> = daily.iter().filter(|r| r.strategy == child.name || r.strategy == p.name).fold(BTreeMap::new(), |mut m, r| {
                let e = m.entry(r.day.as_str()).or_insert((None, None));
                if r.strategy == child.name { e.0 = Some(r.ret) } else { e.1 = Some(r.ret) }
                m
            });
            let (a, b): (Vec<f64>, Vec<f64>) = days.values().filter_map(|(x, y)| Some((((*x)?), (*y)?))).unzip();
            if a.is_empty() {
                continue;
            }
            let diff: Vec<f64> = a.iter().zip(&b).map(|(x, y)| x - y).collect();
            let bm = stat::boot_mean(&diff, lab.gate.block_days, 1000, lab.seed ^ 0xF0);
            out.push(Pairing { challenger: child.name.clone(), incumbent: p.name.clone(), incumbent_spec: p.promoted.card.spec.id(), days: a.len(), challenger_bps: stat::mean(&a) * 1e4, incumbent_bps: stat::mean(&b) * 1e4, diff_bps: bm.mean * 1e4, lo_bps: bm.lo * 1e4, hi_bps: bm.hi * 1e4, corr: crate::stats::pearson(&a, &b).unwrap_or(0.0), duplicate: false });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::synth;
    use super::*;

    fn lab() -> LabConfig {
        let mut lab = LabConfig::default();
        lab.gate.baseline_draws = 30;
        lab.gate.boot_iters = 300;
        lab.gate.rc_iters = 200;
        lab.threads = 2;
        lab
    }

    fn fam(text: &str) -> Family {
        Family::parse(text).unwrap()
    }

    /// One proposal that can find the planted mean-reverting edge, three that can't.
    /// Everything is charged before anything is judged, the generation-wide check is
    /// reported for each, and the comparison sees the duplicate.
    #[test]
    fn a_generation_is_charged_first_and_judged_together() {
        let lab = lab();
        let p = synth::panel(3, 6, 500, "1h", 0.9);
        let fade = fam("interval = \"1h\"\n[rule]\nlong = { feature = \"z\", n = \"$n\", op = \"<=\", level = \"$z*-1\" }\n[grid]\nn = [24, 48]\nz = [1.5, 2.0]\n[fixed]\nhold = 12\n");
        let noise = [
            fam("interval = \"1h\"\n[rule]\nlong = { feature = \"ret\", n = \"$n\", op = \">\", level = 0.02 }\n[grid]\nn = [24, 72]\n[fixed]\nhold = 24\n"),
            fam("interval = \"1h\"\n[rule]\nlong = { feature = \"channel\", n = \"$n\", op = \">\", level = 1.0 }\n[grid]\nn = [48, 96]\n[fixed]\nhold = 48\n"),
            fam("interval = \"1h\"\n[rule]\nlong = { feature = \"xs_ret\", n = \"$n\", op = \">=\", level = 0.6 }\nevery = 24\n[grid]\nn = [24, 72]\n[fixed]\nhold = 24\n"),
        ];
        let fams: Vec<Family> = std::iter::once(fade.clone()).chain(noise).collect();
        let mut books = Books::memory();
        let mut src = |_: &Family| Ok((p.clone(), None));
        let judged = judge_generation(&lab, &fams, &mut src, &|_| 0, &mut books).unwrap();
        assert_eq!(judged.len(), 4);
        assert_eq!(books.total("synthetic-1h"), 10, "every spec of every proposal is charged once");
        assert_eq!(judged.iter().map(|j| j.new_trials).sum::<usize>(), 10);
        for j in &judged {
            assert!(j.eval.checks.iter().any(|c| c.name == "generation reality check") || j.eval.pick.is_none(), "{}: {:?}", j.label, j.eval.checks.iter().map(|c| c.name).collect::<Vec<_>>());
        }
        // The fade earns on these panels; the others don't.
        let fade_mean = judged[0].eval.evidence.as_ref().map(|e| stat::mean(&e.daily)).unwrap();
        assert!(fade_mean > 0.0, "{fade_mean}");
        // A rerun charges nothing new.
        let again = judge_generation(&lab, &fams[..1], &mut src, &|_| 4, &mut books).unwrap();
        assert!(again[0].eval.notes.iter().any(|n| n.contains("4 generated families")), "{:?}", again[0].eval.notes);
        assert_eq!(again[0].new_trials, 0);
        // Comparison: the fade against itself with a different hold is near-duplicate; against noise, not.
        let design = p.head(super::super::design_bars(&p, &lab));
        let pick = judged[0].eval.pick.clone().unwrap();
        let mut twin = pick.clone();
        twin.params.insert("hold".into(), 13.0);
        let pairs = versus_incumbents(&lab, &design, &[("fade".into(), pick)], &[("twin".into(), twin), ("noise".into(), judged[1].eval.pick.clone().unwrap())]);
        assert_eq!(pairs.len(), 2);
        assert!(pairs[0].duplicate && pairs[0].corr > 0.9, "{:?}", pairs[0]);
        assert!(!pairs[1].duplicate);
    }

    #[test]
    fn pairing_needs_days_and_measures_the_difference() {
        let lab = lab();
        let a: Vec<f64> = (0..200).map(|i| 0.002 + if i % 2 == 0 { 0.01 } else { -0.01 }).collect();
        let b: Vec<f64> = (0..200).map(|i| if i % 3 == 0 { 0.01 } else { -0.005 }).collect();
        let p = pair("a", "b", "b:x=1", &a, &b, &lab).unwrap();
        assert_eq!(p.days, 200);
        assert!(p.diff_bps > 0.0 && p.lo_bps < p.diff_bps && p.diff_bps < p.hi_bps);
        assert!(pair("a", "b", "", &a[..10], &b[..10], &lab).is_none());
    }
}
