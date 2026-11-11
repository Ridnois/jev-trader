//! Searching a family's grid, and keeping count. Every spec that is ever simulated
//! for a study is written to the trial ledger; the gate reads the number of trials
//! from there instead of trusting anyone's memory of how many things were tried.

use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::panel::Panel;
use super::sim::{self, SimCfg};
use super::stat;
use super::strategy::{self, Aux, Family, Spec};

#[derive(Debug, Clone)]
pub struct Trial {
    pub spec: Spec,
    /// Daily returns over the panel it was run on.
    pub daily: Vec<f64>,
    pub trades: usize,
    pub wins: usize,
    pub mean_trade_bps: f64,
    pub coin_pnl: Vec<f64>,
}

/// Simulates every spec of the family's grid on `p`, on `threads` threads.
pub fn run_grid(p: &Panel, fam: &Family, aux: &Aux, cfg: &SimCfg, threads: usize) -> Result<Vec<Trial>> {
    let specs = fam.specs();
    let chunk = specs.len().div_ceil(threads.max(1)).max(1);
    let results: Vec<Result<Vec<Trial>>> = std::thread::scope(|s| {
        let handles: Vec<_> = specs.chunks(chunk).map(|part| s.spawn(move || part.iter().map(|spec| run_one(p, spec, aux, cfg)).collect::<Result<Vec<Trial>>>())).collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err(anyhow::anyhow!("a search thread panicked")))).collect()
    });
    let mut out = Vec::with_capacity(specs.len());
    for r in results {
        out.extend(r?);
    }
    Ok(out)
}

pub fn run_one(p: &Panel, spec: &Spec, aux: &Aux, cfg: &SimCfg) -> Result<Trial> {
    let plan = strategy::plan(spec, p, aux)?;
    let run = sim::simulate(p, &plan, cfg);
    Ok(Trial { spec: spec.clone(), trades: run.trades.len(), wins: run.wins(), mean_trade_bps: run.mean_trade_bps(), coin_pnl: run.coin_pnl, daily: run.daily })
}

/// The spec with the best Sharpe ratio over `days`, among those that were in the
/// market on at least `min_active` of them. This is the one selection rule used
/// everywhere: in each walk-forward fold and for the final pick.
pub fn select(rows: &[&[f64]], days: Range<usize>, min_active: usize) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (k, r) in rows.iter().enumerate() {
        let w = &r[days.clone()];
        if w.iter().filter(|v| **v != 0.0).count() < min_active {
            continue;
        }
        let s = stat::sharpe(w);
        if best.is_none_or(|(_, b)| s > b) {
            best = Some((k, s));
        }
    }
    best.map(|(k, _)| k)
}

#[derive(Debug, Clone, Serialize)]
pub struct Fold {
    pub train: Range<usize>,
    pub test: Range<usize>,
    /// Index of the spec picked on the training days.
    pub pick: usize,
    pub train_sharpe: f64,
    pub test_sharpe: f64,
    pub test_mean: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WalkForward {
    /// The picked specs' returns on their test days, in order: what following the
    /// selection rule would have earned on data it had not seen.
    pub oos: Vec<f64>,
    pub folds: Vec<Fold>,
}

/// Rolling walk-forward: pick on `train` days, skip `purge` days (so positions and
/// lookbacks from the training window can't leak), score the pick on the next
/// `test` days, then roll forward by `test`.
pub fn walk_forward(rows: &[&[f64]], train: usize, test: usize, purge: usize, min_active: usize) -> WalkForward {
    let t = rows.first().map(|r| r.len()).unwrap_or(0);
    let mut wf = WalkForward::default();
    let mut start = 0;
    while start + train + purge + test <= t {
        let (tr, te) = (start..start + train, start + train + purge..start + train + purge + test);
        if let Some(pick) = select(rows, tr.clone(), min_active) {
            let w = &rows[pick][te.clone()];
            wf.oos.extend_from_slice(w);
            wf.folds.push(Fold { train_sharpe: stat::sharpe(&rows[pick][tr.clone()]), test_sharpe: stat::sharpe(w), test_mean: stat::mean(w), train: tr, test: te, pick });
        } else {
            // Nothing traded enough to be picked: the rule stays flat for this fold.
            wf.oos.extend(std::iter::repeat_n(0.0, test));
        }
        start += test;
    }
    wf
}

/// One line of `trials.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrialRecord {
    /// `<venue>-<interval>`: trials are counted per data set.
    pub study: String,
    pub family: String,
    pub spec: String,
    pub at: chrono::DateTime<chrono::Utc>,
    pub days: usize,
    pub trades: usize,
    pub sharpe_daily: f64,
    pub mean_bps_day: f64,
}

/// Appends the trials that are new for this study. Returns (new, total for the study).
pub fn record_trials(path: &Path, study: &str, family: &str, trials: &[Trial]) -> (usize, usize) {
    let known: Vec<TrialRecord> = crate::status::read_jsonl(path);
    let mut seen: HashSet<String> = known.iter().filter(|r| r.study == study).map(|r| r.spec.clone()).collect();
    let mut new = 0;
    for t in trials {
        let id = t.spec.id();
        if seen.insert(id.clone()) {
            new += 1;
            crate::bot::append_jsonl(path, &TrialRecord {
                study: study.into(),
                family: family.into(),
                spec: id,
                at: chrono::Utc::now(),
                days: t.daily.len(),
                trades: t.trades,
                sharpe_daily: stat::sharpe(&t.daily),
                mean_bps_day: stat::mean(&t.daily) * 1e4,
            });
        }
    }
    (new, seen.len())
}

/// Distinct specs ever tried on this study.
pub fn count_trials(path: &Path, study: &str) -> usize {
    let known: Vec<TrialRecord> = crate::status::read_jsonl(path);
    known.iter().filter(|r| r.study == study).map(|r| r.spec.as_str()).collect::<HashSet<_>>().len()
}

#[cfg(test)]
mod tests {
    use super::super::panel::testing::*;
    use super::super::rules::{Costs, Sizing};
    use super::*;

    fn fam() -> Family {
        toml::from_str("family = \"meanrev\"\ninterval = \"1h\"\n[grid]\nn = [24, 48]\nz = [1.5, 2.0]\nhold = [6, 12]\n").unwrap()
    }

    #[test]
    fn grid_runs_in_order_whatever_the_thread_count() {
        let p = random_panel(4, 4, 40, "1h");
        let cfg = SimCfg::new(10_000.0, Costs::default(), Sizing::default());
        let one = run_grid(&p, &fam(), &Aux::default(), &cfg, 1).unwrap();
        let four = run_grid(&p, &fam(), &Aux::default(), &cfg, 4).unwrap();
        assert_eq!(one.len(), 8);
        for (a, b) in one.iter().zip(&four) {
            assert_eq!(a.spec, b.spec);
            assert_eq!(a.daily, b.daily);
        }
        assert_eq!(one[0].daily.len(), 40);
        assert!(one.iter().all(|t| t.trades > 0));
    }

    #[test]
    fn select_prefers_sharpe_and_skips_idle_rules() {
        let busy: Vec<f64> = (0..100).map(|i| if i % 2 == 0 { 0.02 } else { -0.01 }).collect();
        let steady: Vec<f64> = (0..100).map(|i| 0.004 + if i % 2 == 0 { 0.001 } else { -0.001 }).collect();
        let mut lucky = vec![0.0; 100];
        lucky[3] = 0.5; // one great day, idle otherwise
        let rows: Vec<&[f64]> = vec![&busy, &steady, &lucky];
        assert_eq!(select(&rows, 0..100, 20), Some(1));
        assert_eq!(select(&rows, 0..100, 200), None);
    }

    #[test]
    fn walk_forward_only_scores_days_after_the_training_window() {
        // Rule A wins the first 100 days and loses after; rule B the opposite.
        let a: Vec<f64> = (0..300).map(|i| (if i < 100 { 0.01 } else { -0.01 }) + if i % 2 == 0 { 0.002 } else { -0.002 }).collect();
        let b: Vec<f64> = a.iter().map(|v| -v).collect();
        let rows: Vec<&[f64]> = vec![&a, &b];
        let wf = walk_forward(&rows, 100, 50, 10, 10);
        // Folds: train 0..100 test 110..160; train 50..150 test 160..210; train 100..200 test 210..260.
        assert_eq!(wf.folds.len(), 3);
        assert_eq!((wf.folds[0].train.clone(), wf.folds[0].test.clone(), wf.folds[0].pick), (0..100, 110..160, 0));
        assert!(wf.folds[0].test_mean < 0.0, "yesterday's winner loses out of sample");
        assert_eq!(wf.folds[2].pick, 1);
        assert!(wf.folds[2].test_mean > 0.0);
        assert_eq!(wf.oos.len(), 150);
    }

    #[test]
    fn ledger_counts_each_spec_once_per_study() {
        let dir = std::env::temp_dir().join(format!("jev-lab-ledger-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("trials.jsonl");
        let p = random_panel(4, 3, 12, "1h");
        let cfg = SimCfg::new(10_000.0, Costs::default(), Sizing::default());
        let trials = run_grid(&p, &fam(), &Aux::default(), &cfg, 2).unwrap();
        assert_eq!(record_trials(&path, "binance-1h", "meanrev", &trials), (8, 8));
        assert_eq!(record_trials(&path, "binance-1h", "meanrev", &trials), (0, 8), "a rerun adds nothing");
        assert_eq!(record_trials(&path, "hyperliquid-1h", "meanrev", &trials[..3]), (3, 3));
        assert_eq!(count_trials(&path, "binance-1h"), 8);
        assert_eq!(count_trials(&path, "nothing"), 0);
        std::fs::remove_dir_all(dir).ok();
    }
}
