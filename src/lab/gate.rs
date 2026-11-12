//! The gate: one verdict for a family of rules, with the search paid for.
//!
//! Search and every check but the last two run on the design window. The lockbox
//! (the newest share of days) is opened only for a candidate that passed everything
//! else, and every opening is logged and makes the next one harder to pass.

use anyhow::Result;
use serde::Serialize;

use super::panel::{DAY_MS, Panel};
use super::search::{self, Trial, WalkForward};
use super::sim::{self, SimCfg};
use super::stat;
use super::strategy::{self, Aux, Family, Plan, Spec};
use super::{GateCfg, LabConfig};
use crate::stats::Rng;

pub const DAYS_PER_YEAR: f64 = 365.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    PromoteToPaper,
    Revise,
    Reject,
    InsufficientEvidence,
}

impl Verdict {
    pub fn word(self) -> &'static str {
        match self {
            Verdict::PromoteToPaper => "PROMOTE_TO_PAPER",
            Verdict::Revise => "REVISE",
            Verdict::Reject => "REJECT",
            Verdict::InsufficientEvidence => "INSUFFICIENT_EVIDENCE",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub value: String,
    pub need: String,
    pub pass: bool,
    /// Failing this one rejects the family outright.
    pub fatal: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Leader {
    pub spec: String,
    pub sharpe_annual: f64,
    pub mean_bps_day: f64,
    pub trades: usize,
    pub win_pct: f64,
    pub mean_trade_bps: f64,
}

/// What a promoted spec is expected to do on paper, fixed before it trades.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Card {
    pub spec: Spec,
    pub interval: String,
    /// Expected daily return (fraction of capital): the lower of walk-forward and lockbox.
    pub mu_daily: f64,
    pub sd_daily: f64,
    /// Sequential test: false-positive rate and miss rate.
    pub alpha: f64,
    pub beta: f64,
    /// Expected days to a decision if the edge is real / if there is none.
    pub days_if_real: f64,
    pub days_if_none: f64,
    /// 99th-percentile drawdown (fraction of capital) over 30, 60 and 90 days.
    pub dd99: [f64; 3],
    pub issued: chrono::DateTime<chrono::Utc>,
    pub study: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Evaluation {
    pub study: String,
    pub family: String,
    pub hypothesis: String,
    pub panel: String,
    pub specs: usize,
    pub trials_ledger: usize,
    pub trials_effective: f64,
    pub design_days: usize,
    pub lockbox_days: usize,
    pub lockbox_opened: bool,
    pub pick: Option<Spec>,
    pub leaders: Vec<Leader>,
    pub checks: Vec<Check>,
    pub walk: WalkForward,
    pub fold_picks: Vec<String>,
    pub verdict: Verdict,
    pub card: Option<Card>,
    pub notes: Vec<String>,
    /// The pick's design-window record, kept so `lab learn` can read where it made and
    /// lost money without simulating again. Never contains lockbox days.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
}

/// One slice of the pick's trades.
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct Slice {
    pub label: String,
    pub trades: usize,
    pub wins: usize,
    /// Dollars, after costs and funding.
    pub pnl: f64,
    pub mean_bps: f64,
}

/// The pick on the design window, from its first trading day on.
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct Evidence {
    /// UTC date of `daily[0]`.
    pub first_day: String,
    pub capital: f64,
    /// Daily return (fraction of capital).
    pub daily: Vec<f64>,
    /// The equal-weight coin basket's log return on the same days (market state).
    pub basket: Vec<f64>,
    pub coins: Vec<Slice>,
    /// long, short.
    pub sides: Vec<Slice>,
    /// Holding time in terciles of the pick's own trades (label: "≤ N bars").
    pub holds: Vec<Slice>,
    /// By exit reason: time, stop, signal, end.
    pub exits: Vec<Slice>,
}

fn slice(label: impl Into<String>, trades: &[&sim::Trade]) -> Slice {
    let bps: Vec<f64> = trades.iter().map(|t| t.bps()).collect();
    Slice { label: label.into(), trades: trades.len(), wins: trades.iter().filter(|t| t.pnl > 0.0).count(), pnl: trades.iter().map(|t| t.pnl).sum(), mean_bps: stat::mean(&bps) }
}

/// Breaks a run on the design panel down for the learner.
pub fn evidence(design: &Panel, run: &sim::Run, first: usize, capital: f64) -> Evidence {
    let days = run.daily.len();
    let mut basket = vec![0.0f64; days];
    for i in 0..design.n {
        let d = design.day_of(i);
        if d < days {
            basket[d] += design.basket_ret[i];
        }
    }
    let t: Vec<&sim::Trade> = run.trades.iter().collect();
    let coins = (0..design.coins.len()).map(|c| slice(design.coins[c].clone(), &t.iter().copied().filter(|x| x.coin == c).collect::<Vec<_>>())).filter(|s| s.trades > 0).collect();
    let sides = vec![slice("long", &t.iter().copied().filter(|x| x.side > 0).collect::<Vec<_>>()), slice("short", &t.iter().copied().filter(|x| x.side < 0).collect::<Vec<_>>())];
    let mut held: Vec<usize> = t.iter().map(|x| x.exit_i.saturating_sub(x.entry_i)).collect();
    held.sort_unstable();
    let mut holds = Vec::new();
    if !held.is_empty() {
        let cuts = [held[held.len() / 3], held[2 * held.len() / 3], usize::MAX];
        let mut lo = 0usize;
        for (k, hi) in cuts.iter().enumerate() {
            let part: Vec<&sim::Trade> = t.iter().copied().filter(|x| {
                let h = x.exit_i.saturating_sub(x.entry_i);
                (k == 0 || h > lo) && h <= *hi
            }).collect();
            if !part.is_empty() {
                holds.push(slice(if *hi == usize::MAX { format!("> {lo} bars") } else { format!("≤ {hi} bars") }, &part));
            }
            lo = *hi;
        }
    }
    let exits = [(sim::Reason::Time, "time"), (sim::Reason::Stop, "stop"), (sim::Reason::Signal, "signal"), (sim::Reason::End, "end")]
        .iter()
        .map(|(r, w)| slice(*w, &t.iter().copied().filter(|x| x.reason == *r).collect::<Vec<_>>()))
        .filter(|s| s.trades > 0)
        .collect();
    let day0 = design.t0.div_euclid(DAY_MS) + first as i64;
    Evidence {
        first_day: chrono::DateTime::from_timestamp_millis(day0 * DAY_MS).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default(),
        capital,
        daily: run.daily[first.min(days)..].to_vec(),
        basket: basket[first.min(days)..].to_vec(),
        coins,
        sides,
        holds,
        exits,
    }
}

pub struct Inputs<'a> {
    /// Research panel, lockbox included.
    pub full: &'a Panel,
    pub aux: &'a Aux,
    /// The trading venue's own candles and funding, for the last check.
    pub venue: Option<(&'a Panel, &'a Aux)>,
    pub fam: &'a Family,
    /// Distinct specs ever tried on this study, before this run (from the ledger).
    pub ledger_trials: usize,
    /// Lockbox openings so far for this study.
    pub lockbox_reads: usize,
    /// Judge this spec instead of the grid's best (it must be in the grid).
    pub force: Option<&'a Spec>,
    /// The grid already simulated on the design window (saves running it twice).
    pub design_trials: Option<Vec<Trial>>,
    /// Daily returns on the design window of every spec simulated in the same `lab
    /// evolve` cycle on this study (this family's included). Proposals chosen together
    /// are judged together: a family that passes its own reality check while the
    /// generation as a whole fails it gets REVISE at best, and the lockbox stays shut.
    pub generation: Option<&'a [Vec<f64>]>,
}

fn bps(x: f64) -> String {
    format!("{:+.2} bps/day", x * 1e4)
}

fn annual(sr_daily: f64) -> f64 {
    sr_daily * DAYS_PER_YEAR.sqrt()
}

/// The same plan with each coin's signals rotated by a random number of bars: the
/// rule's trade count, side mix, clustering and exits survive, its timing doesn't.
fn time_shifted(plan: &Plan, rng: &mut Rng, min_shift: usize) -> Plan {
    let rot = |v: &Vec<i8>, k: usize| {
        let mut out = v.clone();
        out.rotate_right(k);
        out
    };
    let mut entry = Vec::with_capacity(plan.entry.len());
    let mut exit = Vec::with_capacity(plan.exit.len());
    for c in 0..plan.entry.len() {
        let n = plan.entry[c].len();
        let k = if n > 2 * min_shift { min_shift + rng.below(n - 2 * min_shift) } else { n / 2 };
        entry.push(rot(&plan.entry[c], k));
        if !plan.exit.is_empty() {
            exit.push(rot(&plan.exit[c], k));
        }
    }
    Plan { entry, exit, policy: plan.policy }
}

/// (year, quarter) of each panel day.
fn quarter_of(p: &Panel, day: usize) -> (i32, u32) {
    use chrono::Datelike;
    let t = (p.t0.div_euclid(DAY_MS) + day as i64) * DAY_MS;
    let d = chrono::DateTime::from_timestamp_millis(t).unwrap_or_default();
    (d.year(), (d.month() - 1) / 3)
}

pub fn evaluate(inp: Inputs, lab: &LabConfig) -> Result<Evaluation> {
    let g: &GateCfg = &lab.gate;
    let full = inp.full;
    let fam = inp.fam;
    let study = format!("{}-{}", full.venue, full.interval);
    let cfg = SimCfg::new(lab.capital, lab.costs.clone(), lab.sizing.clone());
    let bpd = full.bars_per_day();
    let total_days = full.n_days();
    let design_days = ((total_days as f64) * (1.0 - lab.lockbox_frac)).floor() as usize;
    let design = full.head(design_days * bpd);
    let mut ev = Evaluation {
        study: study.clone(),
        family: fam.family.clone(),
        hypothesis: fam.hypothesis.clone(),
        panel: full.describe(),
        specs: 0,
        trials_ledger: inp.ledger_trials,
        trials_effective: 0.0,
        design_days,
        lockbox_days: total_days - design_days,
        lockbox_opened: false,
        pick: None,
        leaders: Vec::new(),
        checks: Vec::new(),
        walk: WalkForward::default(),
        fold_picks: Vec::new(),
        verdict: Verdict::InsufficientEvidence,
        card: None,
        notes: Vec::new(),
        evidence: None,
    };

    let trials: Vec<Trial> = match inp.design_trials {
        Some(t) if t.first().is_some_and(|x| x.daily.len() == design_days) => t,
        _ => search::run_grid(&design, fam, inp.aux, &cfg, lab.threads)?,
    };
    ev.specs = trials.len();
    // Days before anything could trade (warm-up, coins not listed yet) carry no information.
    let first = (0..design_days).find(|d| trials.iter().any(|t| t.daily[*d] != 0.0)).unwrap_or(design_days);
    let rows: Vec<&[f64]> = trials.iter().map(|t| &t.daily[first..]).collect();
    let t_days = design_days - first;
    let n_trials = inp.ledger_trials.max(trials.len());
    ev.trials_ledger = n_trials;

    let mut order: Vec<usize> = (0..trials.len()).collect();
    order.sort_by(|a, b| stat::sharpe(rows[*b]).total_cmp(&stat::sharpe(rows[*a])));
    ev.leaders = order.iter().take(10).map(|&k| leader(&trials[k], rows[k])).collect();

    let pick = match inp.force {
        Some(s) => trials.iter().position(|t| t.spec.id() == s.id()),
        None => search::select(&rows, 0..t_days, lab.walk.min_active_days),
    };
    let Some(pick) = pick else {
        ev.notes.push(match inp.force {
            Some(s) => format!("{} is not in this family's grid", s.id()),
            None => "no spec traded on enough days to be judged".into(),
        });
        return Ok(ev);
    };
    let best = &trials[pick];
    let r = rows[pick];
    ev.pick = Some(best.spec.clone());
    let mut checks: Vec<Check> = Vec::new();
    let mut add = |name: &'static str, value: String, need: String, pass: bool, fatal: bool| checks.push(Check { name, value, need, pass, fatal });

    // 0. Enough evidence to say anything.
    let enough = t_days >= g.min_design_days && best.trades >= g.min_trades;
    add("evidence", format!("{t_days} days, {} trades", best.trades), format!("≥ {} days and ≥ {} trades", g.min_design_days, g.min_trades), enough, false);

    // 1. Walk-forward: the selection rule on data it had not seen.
    let wf = search::walk_forward(&rows, lab.walk.train_days, lab.walk.test_days, lab.walk.purge_days, lab.walk.min_active_days);
    let wb = stat::boot_mean(&wf.oos, g.block_days, g.boot_iters, lab.seed);
    add(
        "walk-forward",
        format!("{} [{:+.2}, {:+.2}] over {} days, Sharpe {:.2}", bps(wb.mean), wb.lo * 1e4, wb.hi * 1e4, wf.oos.len(), annual(stat::sharpe(&wf.oos))),
        "mean > 0 with the 95% interval above 0".into(),
        !wf.oos.is_empty() && wb.lo > 0.0,
        wf.oos.is_empty() || wb.mean <= 0.0,
    );
    ev.fold_picks = wf.folds.iter().map(|f| trials[f.pick].spec.id()).collect();

    // 2. Reality check over the whole family.
    let rc = stat::reality_check(&rows, g.block_days, g.rc_iters, lab.seed ^ 0xA5);
    let rc_p = rc.as_ref().map(|x| x.p).unwrap_or(1.0);
    add(
        "reality check",
        format!("p = {rc_p:.3} (best of {} specs: t = {:.2})", trials.len(), rc.as_ref().map(|x| x.stat).unwrap_or(0.0)),
        format!("p < {}", g.rc_p_pass),
        rc_p < g.rc_p_pass,
        rc_p > g.rc_p_fatal,
    );

    // 2b. The same check over the whole generation of proposals.
    if let Some(cycle) = inp.generation {
        let grows: Vec<&[f64]> = cycle.iter().filter(|r| r.len() == design_days).map(|r| &r[first..]).collect();
        if grows.len() > rows.len() {
            let g_rc = stat::reality_check(&grows, g.block_days, g.rc_iters, lab.seed ^ 0x6E);
            let gp = g_rc.as_ref().map(|x| x.p).unwrap_or(1.0);
            add("generation reality check", format!("p = {gp:.3} over all {} specs simulated in this cycle", grows.len()), format!("p < {}", g.rc_p_pass), gp < g.rc_p_pass, false);
        }
    }

    // 3. Deflated Sharpe of the pick, given every trial on this data set.
    // The bar is the Sharpe the luckiest of N_eff no-edge rules would show. A no-edge
    // rule's Sharpe over T days has variance ≈ 1/T; the textbook recipe takes the
    // variance across the trials instead, but that also counts real differences
    // between rules (a 3h hold pays four times the costs of a 24h hold), which raised
    // the bar so far that planted edges with Sharpe 3.5 never passed in calibration.
    let (n_eff, rho) = stat::effective_trials(&rows, n_trials, lab.seed ^ 0x17);
    ev.trials_effective = n_eff;
    let sr0 = stat::expected_max_sharpe(1.0 / (t_days.max(2) - 1) as f64, n_eff);
    let (sk, ku) = stat::skew_kurt(r);
    let sr = stat::sharpe(r);
    let dsr = stat::psr(sr, sr0, t_days, sk, ku);
    add(
        "deflated Sharpe",
        format!("{dsr:.3} (Sharpe {:.2} vs {:.2} expected from luck; {n_trials} trials, {n_eff:.0} effective, mean correlation {rho:.2})", annual(sr), annual(sr0)),
        format!("≥ {}", g.dsr_pass),
        dsr >= g.dsr_pass,
        dsr < g.dsr_fatal,
    );

    // 4. Probability of backtest overfitting.
    match stat::pbo_cscv(&rows, g.pbo_blocks) {
        Some(p) => add("overfitting (PBO)", format!("{:.2} (in-sample winner's median out-of-sample rank {:.2}, {} splits)", p.pbo, p.median_rank, p.splits), format!("≤ {}", g.pbo_pass), p.pbo <= g.pbo_pass, p.pbo > g.pbo_fatal),
        None => add("overfitting (PBO)", "not computable (needs ≥ 4 specs and enough days)".into(), format!("≤ {}", g.pbo_pass), trials.len() < 4, false),
    }

    // 5. Plateau: the pick's grid neighbours.
    let nb = if inp.force.is_none() || fam.specs().get(pick).is_some_and(|s| s.id() == best.spec.id()) { fam.neighbours(pick) } else { Vec::new() };
    if nb.is_empty() {
        add("plateau", "no neighbours in the grid".into(), "neighbours positive".into(), true, false);
    } else {
        let mut ns: Vec<f64> = nb.iter().map(|k| stat::sharpe(rows[*k])).collect();
        ns.sort_by(f64::total_cmp);
        let (worst, median) = (ns[0], ns[ns.len() / 2]);
        add(
            "plateau",
            format!("{} neighbours: worst Sharpe {:.2}, median {:.2} (pick {:.2})", nb.len(), annual(worst), annual(median), annual(sr)),
            format!("all > 0 and median ≥ {:.0}% of the pick", g.plateau_frac * 100.0),
            worst > 0.0 && median >= g.plateau_frac * sr,
            false,
        );
    }

    // 6. Costs doubled, and one bar late.
    let plan = strategy::plan(&best.spec, &design, inp.aux)?;
    ev.evidence = Some(evidence(&design, &sim::simulate(&design, &plan, &cfg), first, cfg.capital));
    let stressed = sim::simulate(&design, &plan, &SimCfg { costs: lab.costs.scaled(g.cost_stress), ..cfg.clone() });
    let sm = stat::mean(&stressed.daily[first..]);
    add("costs ×2", bps(sm), "mean > 0".into(), sm > 0.0, false);
    let late = sim::simulate(&design, &plan, &SimCfg { entry_delay: 1, ..cfg.clone() });
    let lm = stat::mean(&late.daily[first..]);
    add("one bar late", bps(lm), "mean > 0".into(), lm > 0.0, false);

    // 7. Breadth: coins, quarters, best days.
    let total: f64 = best.coin_pnl.iter().sum();
    let traded: Vec<usize> = (0..design.coins.len()).filter(|c| best.coin_pnl[*c] != 0.0).collect();
    let carried = traded.iter().filter(|c| total - best.coin_pnl[**c] <= 0.0).map(|c| design.coins[*c].clone()).collect::<Vec<_>>();
    let winners = traded.iter().filter(|c| best.coin_pnl[**c] > 0.0).count();
    add(
        "coins",
        format!("{winners}/{} coins profitable{}", traded.len(), if carried.is_empty() { String::new() } else { format!("; without {} the total is ≤ 0", carried.join(", ")) }),
        "profitable without any single coin".into(),
        carried.is_empty() && total > 0.0 && traded.len() >= 3,
        false,
    );
    let mut quarters: std::collections::BTreeMap<(i32, u32), (f64, usize)> = Default::default();
    for (d, v) in r.iter().enumerate() {
        let q = quarters.entry(quarter_of(&design, first + d)).or_default();
        q.0 += v;
        q.1 += 1;
    }
    let qs: Vec<f64> = quarters.values().filter(|(_, n)| *n >= 20).map(|(s, _)| *s).collect();
    let qpos = qs.iter().filter(|s| **s > 0.0).count();
    add("quarters", format!("{qpos}/{} positive", qs.len()), format!("≥ {:.0}%", g.quarters_pos_frac * 100.0), !qs.is_empty() && qpos as f64 >= g.quarters_pos_frac * qs.len() as f64, false);
    let mut sorted = r.to_vec();
    sorted.sort_by(f64::total_cmp);
    let drop = ((t_days as f64 * g.top_day_frac).ceil() as usize).max(1).min(sorted.len());
    let rest: f64 = sorted[..sorted.len() - drop].iter().sum();
    add("best days removed", format!("{:+.2}% without the best {drop} days ({:+.2}% with)", rest * 100.0, r.iter().sum::<f64>() * 100.0), "still > 0".into(), rest > 0.0, false);

    // 8. Same trades at other times.
    let mut rng = Rng::new(lab.seed ^ 0xB45E);
    let mine = stat::mean(r);
    let min_shift = (5 * bpd).min(design.n / 4);
    let mut below = 0usize;
    for _ in 0..g.baseline_draws {
        let run = sim::simulate(&design, &time_shifted(&plan, &mut rng, min_shift), &cfg);
        if stat::mean(&run.daily[first..]) < mine {
            below += 1;
        }
    }
    let pct = if g.baseline_draws > 0 { below as f64 / g.baseline_draws as f64 } else { 1.0 };
    add("timing baseline", format!("beats {:.0}% of {} time-shifted copies", pct * 100.0, g.baseline_draws), format!("≥ {:.0}%", g.baseline_pct * 100.0), pct >= g.baseline_pct, false);

    // 9. Market exposure (reported; an edge that is only beta is not one).
    let mut mkt = vec![0.0f64; design_days];
    for i in 0..design.n {
        mkt[design.day_of(i)] += design.basket_ret[i];
    }
    if let Some(o) = crate::stats::ols_hac(r, &mkt[first..]) {
        add("market-neutral part", format!("alpha {} (t = {:.2}), beta {:+.2}, R² {:.2}", bps(o.alpha), o.t_alpha, o.beta, o.r2), "alpha > 0".into(), o.alpha > 0.0, false);
    }

    let design_ok = checks.iter().all(|c| c.pass);
    let fatal = checks.iter().any(|c| !c.pass && c.fatal);
    let mut add = |name: &'static str, value: String, need: String, pass: bool, fatal: bool| checks.push(Check { name, value, need, pass, fatal });

    // 10. Lockbox, only for a candidate that passed everything on the design window.
    let mut lock_mean = None;
    if design_ok && ev.lockbox_days >= 20 {
        ev.lockbox_opened = true;
        let run_full = search::run_one(full, &best.spec, inp.aux, &cfg)?;
        let lock = &run_full.daily[design_days..];
        let lb = stat::boot_mean(lock, g.block_days, g.boot_iters, lab.seed ^ 0x10C);
        let alpha = g.lockbox_alpha / (inp.lockbox_reads + 1) as f64;
        let floor = stat::boot_window_means(r, lock.len(), g.block_days, g.boot_iters, lab.seed ^ 0xF100);
        let p5 = floor.get(floor.len() / 20).copied().unwrap_or(0.0);
        let ok = lb.mean > 0.0 && lb.p < alpha && lb.mean >= p5;
        add(
            "lockbox",
            format!("{} over {} unseen days, p = {:.3}, Sharpe {:.2}; design predicted ≥ {:+.2} bps/day; opening #{}", bps(lb.mean), lock.len(), lb.p, annual(stat::sharpe(lock)), p5 * 1e4, inp.lockbox_reads + 1),
            format!("mean > 0, p < {alpha:.3}, not below the predicted range"),
            ok,
            // Losing on unseen data ends it. Positive but not yet convincing is a REVISE.
            lb.mean <= 0.0,
        );
        lock_mean = Some(lb.mean);

        // 11. The venue the bots trade on.
        if let Some((vp, vaux)) = inp.venue {
            let vrun = search::run_one(vp, &best.spec, vaux, &cfg)?;
            // Line the two runs up by calendar day.
            let (d0f, d0v) = (full.t0.div_euclid(DAY_MS), vp.t0.div_euclid(DAY_MS));
            let (mut a, mut b) = (Vec::new(), Vec::new());
            for (dv, x) in vrun.daily.iter().enumerate() {
                let df = d0v + dv as i64 - d0f;
                if df >= 0 && (df as usize) < run_full.daily.len() {
                    a.push(run_full.daily[df as usize]);
                    b.push(*x);
                }
            }
            let corr = crate::stats::pearson(&a, &b).unwrap_or(0.0);
            let vm = stat::mean(&vrun.daily);
            add(
                "trading venue",
                format!("{} on {} ({} trades); daily returns correlate {corr:.2} with the research venue over {} shared days", bps(vm), vp.describe(), vrun.trades, a.len()),
                format!("mean > 0 and correlation ≥ {}", g.venue_corr),
                vm > 0.0 && corr >= g.venue_corr,
                false,
            );
        }
    } else if design_ok {
        ev.notes.push(format!("lockbox has only {} days: not opened", ev.lockbox_days));
    } else {
        ev.notes.push("lockbox not opened: the candidate did not pass the design-window checks, so the unseen data stays unseen".into());
    }

    ev.verdict = if !enough {
        Verdict::InsufficientEvidence
    } else if fatal || checks.iter().any(|c| !c.pass && c.fatal) {
        Verdict::Reject
    } else if checks.iter().all(|c| c.pass) && ev.lockbox_opened {
        Verdict::PromoteToPaper
    } else {
        Verdict::Revise
    };
    if ev.verdict == Verdict::PromoteToPaper {
        let mu = lock_mean.unwrap_or(wb.mean).min(wb.mean);
        let sdv = stat::sd(r);
        let (d1, d0) = stat::sprt_expected_days(mu, sdv, g.paper_alpha, g.paper_beta);
        let dd = |len: usize| stat::boot_drawdown(r, len, g.block_days, g.boot_iters, 0.99, lab.seed ^ len as u64);
        ev.card = Some(Card { spec: best.spec.clone(), interval: fam.interval.clone(), mu_daily: mu, sd_daily: sdv, alpha: g.paper_alpha, beta: g.paper_beta, days_if_real: d1, days_if_none: d0, dd99: [dd(30), dd(60), dd(90)], issued: chrono::Utc::now(), study });
    }
    ev.walk = wf;
    ev.checks = checks;
    Ok(ev)
}

fn leader(t: &Trial, r: &[f64]) -> Leader {
    Leader {
        spec: t.spec.id(),
        sharpe_annual: annual(stat::sharpe(r)),
        mean_bps_day: stat::mean(r) * 1e4,
        trades: t.trades,
        win_pct: if t.trades > 0 { t.wins as f64 / t.trades as f64 * 100.0 } else { 0.0 },
        mean_trade_bps: t.mean_trade_bps,
    }
}

pub fn markdown(ev: &Evaluation) -> String {
    use std::fmt::Write as _;
    let mut md = String::new();
    let _ = writeln!(md, "# {} on {}: {}\n", ev.family, ev.study, ev.verdict.word());
    let _ = writeln!(md, "*{}{}*\n", ev.panel, if ev.hypothesis.is_empty() { String::new() } else { format!(" · hypothesis {}", ev.hypothesis) });
    let _ = writeln!(md, "- Searched **{} specs** on the first {} days. The newest {} days are the lockbox{}.", ev.specs, ev.design_days, ev.lockbox_days, if ev.lockbox_opened { " (opened for this candidate)" } else { " (still sealed)" });
    let _ = writeln!(md, "- Trials on this data set so far: **{}** ({:.0} effective after correlation).", ev.trials_ledger, ev.trials_effective);
    if let Some(p) = &ev.pick {
        let _ = writeln!(md, "- Candidate: `{}`", p.id());
    }
    for n in &ev.notes {
        let _ = writeln!(md, "- {n}");
    }
    if !ev.checks.is_empty() {
        let _ = writeln!(md, "\n| Check | Result | Needs | |\n|---|---|---|---|");
        for c in &ev.checks {
            let mark = if c.pass { "pass" } else if c.fatal { "**FAIL**" } else { "fail" };
            let _ = writeln!(md, "| {} | {} | {} | {mark} |", c.name, c.value, c.need);
        }
    }
    if !ev.walk.folds.is_empty() {
        let _ = writeln!(md, "\n## Walk-forward folds\n\n| Train days | Test days | Picked | Train Sharpe | Test Sharpe | Test mean |\n|---|---|---|---|---|---|");
        for (f, id) in ev.walk.folds.iter().zip(&ev.fold_picks) {
            let _ = writeln!(md, "| {}–{} | {}–{} | `{id}` | {:.2} | {:.2} | {:+.2} bps/day |", f.train.start, f.train.end, f.test.start, f.test.end, annual(f.train_sharpe), annual(f.test_sharpe), f.test_mean * 1e4);
        }
    }
    if !ev.leaders.is_empty() {
        let _ = writeln!(md, "\n## In-sample leaderboard (design window; this is the view that fools people)\n\n| Spec | Sharpe | Mean | Trades | Won | Per trade |\n|---|---|---|---|---|---|");
        for l in &ev.leaders {
            let _ = writeln!(md, "| `{}` | {:.2} | {:+.2} bps/day | {} | {:.0}% | {:+.1} bps |", l.spec, l.sharpe_annual, l.mean_bps_day, l.trades, l.win_pct, l.mean_trade_bps);
        }
    }
    if let Some(c) = &ev.card {
        let _ = writeln!(md, "\n## Prediction card (fixed before paper trading)\n");
        let _ = writeln!(md, "- Expected: {:+.2} bps/day, daily sd {:.2} bps (Sharpe {:.2}).", c.mu_daily * 1e4, c.sd_daily * 1e4, annual(c.mu_daily / c.sd_daily.max(1e-12)));
        let _ = writeln!(md, "- Sequential test (false positives {:.0}%, power {:.0}%): about **{:.0} days** to graduate if the edge is real, **{:.0} days** to be killed if there is none.", c.alpha * 100.0, (1.0 - c.beta) * 100.0, c.days_if_real, c.days_if_none);
        let _ = writeln!(md, "- Kill at a drawdown beyond {:.2}% in 30 days, {:.2}% in 60, {:.2}% in 90 (99th percentile of the backtest's bootstrap).", c.dd99[0] * 100.0, c.dd99[1] * 100.0, c.dd99[2] * 100.0);
    }
    md
}

#[cfg(test)]
mod tests {
    use super::super::panel::testing::*;
    use super::*;

    fn fam() -> Family {
        toml::from_str("family = \"meanrev\"\ninterval = \"1h\"\n[grid]\nn = [24, 48, 96]\nz = [1.5, 2.0, 2.5]\nhold = [6, 12]\nstop = [0, 400]\n").unwrap()
    }

    fn lab() -> LabConfig {
        let mut lab = LabConfig::default();
        lab.threads = 2;
        lab.gate.boot_iters = 300;
        lab.gate.rc_iters = 200;
        lab.gate.baseline_draws = 30;
        lab.gate.min_design_days = 100;
        lab.walk.train_days = 90;
        lab.walk.test_days = 30;
        lab
    }

    #[test]
    fn a_random_walk_is_not_promoted_and_its_lockbox_stays_sealed() {
        let p = random_panel(1234, 6, 300, "1h");
        let f = fam();
        let ev = evaluate(Inputs { full: &p, aux: &Aux::default(), venue: None, fam: &f, ledger_trials: 0, lockbox_reads: 0, force: None, design_trials: None, generation: None }, &lab()).unwrap();
        assert_eq!(ev.specs, 36);
        assert_eq!((ev.design_days, ev.lockbox_days), (240, 60));
        assert_ne!(ev.verdict, Verdict::PromoteToPaper, "{}", markdown(&ev));
        assert!(!ev.lockbox_opened, "a failing candidate must not consume the lockbox");
        assert!(ev.card.is_none());
        let md = markdown(&ev);
        assert!(md.contains("reality check") && md.contains("still sealed"), "{md}");
        // The learner's record of the pick: design days only, and it adds up.
        let e = ev.evidence.as_ref().expect("evidence for the pick");
        assert!(!e.daily.is_empty() && e.daily.len() <= ev.design_days);
        assert_eq!(e.daily.len(), e.basket.len());
        let trades: usize = e.sides.iter().map(|s| s.trades).sum();
        assert_eq!(trades, e.coins.iter().map(|s| s.trades).sum::<usize>());
        assert_eq!(trades, e.holds.iter().map(|s| s.trades).sum::<usize>());
        assert_eq!(trades, e.exits.iter().map(|s| s.trades).sum::<usize>());
        let pnl: f64 = e.coins.iter().map(|s| s.pnl).sum();
        assert!((pnl - e.sides.iter().map(|s| s.pnl).sum::<f64>()).abs() < 1e-6);
        // A spec outside the grid can't be judged as if it had been searched.
        let odd = Spec::new("meanrev", &[("n", 7.0), ("z", 9.0), ("hold", 1.0), ("stop", 0.0)]);
        let ev = evaluate(Inputs { full: &p, aux: &Aux::default(), venue: None, fam: &f, ledger_trials: 0, lockbox_reads: 0, force: Some(&odd), design_trials: None, generation: None }, &lab()).unwrap();
        assert_eq!(ev.verdict, Verdict::InsufficientEvidence);
        assert!(ev.pick.is_none());
    }

    #[test]
    fn time_shift_keeps_the_signals_and_moves_them() {
        let entry = vec![(0..200).map(|i| if [3, 40, 111, 170].contains(&i) { 1i8 } else { 0 }).collect::<Vec<_>>(); 3];
        let plan = Plan { entry, exit: Vec::new(), policy: strategy::Policy { hold_bars: 5, stop_bps: 0.0, extend: false, max_positions: 0 } };
        let mut rng = Rng::new(3);
        let s = time_shifted(&plan, &mut rng, 20);
        for c in 0..3 {
            assert_eq!(s.entry[c].iter().filter(|e| **e != 0).count(), 4);
            assert_ne!(s.entry[c], plan.entry[c]);
        }
    }
}
