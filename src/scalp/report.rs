//! `jev-trader scalp-report`: the scalper's measurement loop. Reads the files the
//! scalper writes (candidates, markouts, orders, trades) and answers: does the
//! signal pay after costs, where does the money go (fills, exits, gates), which
//! inputs relate to the move, and where does each scalper hypothesis stand.
//! Means carry 95% intervals with errors clustered by UTC hour; correlations carry
//! their percentile within a block-permutation null. Suggestions only: nothing here
//! edits a config or `hypotheses.toml`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::engine::Side;
use super::features::Features;
use crate::config::ScalpConfig;
use crate::hypotheses::Hypothesis;
use crate::report::{BacklogEntry, MIN_SHOW, MIN_SOLID, MIN_SUGGEST};
use crate::stats;
use crate::status::read_jsonl;

const HOUR_MS: i64 = 3_600_000;
const NULL_ITERS: usize = 1000;
/// The horizon the H-C hypotheses are written against.
const H_TEST: i64 = 600;

const PASSED: &str = "passed every market check";
const PLACED: &str = "placed as an order";
const FILLED: &str = "filled";
const ALL: &str = "every candidate";

/// Inputs logged with every candidate. `true` = directional: multiplied by the
/// side's sign, so positive means "in the trade's favour" (for funding: the side pays).
const FEATURES: [(&str, bool, fn(&Features) -> f64); 10] = [
    ("flow", true, |f| f.flow),
    ("obi", true, |f| f.obi),
    ("micro_bps", true, |f| f.micro_bps),
    ("ret_60s_bps", true, |f| f.ret_60s_bps),
    ("ret_300s_bps", true, |f| f.ret_300s_bps),
    ("funding_bps_1h", true, |f| f.funding_bps_1h),
    ("premium_bps", true, |f| f.premium_bps),
    ("activity", false, |f| f.activity),
    ("exp_move_bps", false, |f| f.exp_move_bps),
    ("spread_bps", false, |f| f.spread_bps),
];
/// The inputs the entry gates trade on (burst, flow, book, push).
const ENTRY_INPUTS: [&str; 4] = ["activity", "flow", "obi", "ret_60s_bps"];

// ---------------------------------------------------------------- input records

#[derive(Deserialize)]
struct Cand {
    id: String,
    asset: String,
    side: Side,
    features: Features,
    eval: Eval,
    #[serde(default)]
    taken: bool,
    #[serde(default)]
    blocked_by: Option<String>,
}

#[derive(Deserialize)]
struct Eval {
    #[serde(default)]
    ok: bool,
}

#[derive(Deserialize)]
struct Markout {
    id: String,
    h_secs: i64,
    ret_bps: f64,
    net_bps: f64,
    cost_bps: f64,
}

#[derive(Deserialize)]
struct Order {
    candidate_id: String,
    outcome: String,
}

#[derive(Deserialize)]
struct Trade {
    candidate_id: String,
    opened: DateTime<Utc>,
    reason: String,
    pnl_usd: f64,
    pnl_bps: f64,
    fees_usd: f64,
}

pub struct Logs {
    cands: Vec<Cand>,
    marks: Vec<Markout>,
    orders: Vec<Order>,
    trades: Vec<Trade>,
}

pub fn load(dir: &Path) -> Logs {
    Logs {
        cands: read_jsonl(&dir.join("scalp_candidates.jsonl")),
        marks: read_jsonl(&dir.join("scalp_markouts.jsonl")),
        orders: read_jsonl(&dir.join("scalp_orders.jsonl")),
        trades: read_jsonl(&dir.join("scalp_trades.jsonl")),
    }
}

// ---------------------------------------------------------------- output

/// A mean with a 95% interval whose errors are clustered by UTC hour.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Est {
    pub n: usize,
    /// Distinct UTC hours: the independent units.
    pub hours: usize,
    pub mean: Option<f64>,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    pub low_n: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EdgeRow {
    pub group: String,
    pub h: i64,
    /// Move in the trade's favour from the decision mid, bps.
    pub gross: Est,
    /// Mean round-trip cost charged, bps.
    pub cost: f64,
    pub net: Est,
    /// "pays after costs", "loses after costs" or "undecided" (net interval vs 0).
    pub verdict: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Fills {
    pub orders: usize,
    /// Filled or partly filled.
    pub filled: usize,
    pub missed: usize,
    pub fill_rate: Option<f64>,
    /// Gross markouts at the test horizon of filled vs missed orders.
    pub filled_ret: Est,
    pub missed_ret: Est,
    /// Per posted order at the test horizon: maker as run (filled → net, missed → 0)
    /// vs a taker entry on every one (taker fee, slippage and the whole spread).
    pub maker_ev: Est,
    pub taker_ev: Est,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TradeStats {
    pub n: usize,
    pub wins: usize,
    pub pnl_usd: f64,
    pub fees_usd: f64,
    /// Realized P&L per trade, bps, clustered by the hour it opened.
    pub bps: Est,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExitRow {
    pub reason: String,
    pub n: usize,
    pub realized_bps: f64,
    /// Trades of this kind with a markout at the test horizon.
    pub n_held: usize,
    /// Mean of (net markout at the test horizon − realized): what holding to the time
    /// stop would have added. Approximate: the markout starts from the decision mid.
    pub hold_minus_exit_bps: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GateRow {
    /// The check that stopped the candidate, or "taken".
    pub gate: String,
    pub net: Est,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeatureRow {
    pub feature: String,
    pub signed: bool,
    pub n: usize,
    /// Spearman against the move at the main horizon.
    pub ic: Option<f64>,
    pub null_pct: Option<f64>,
    pub p: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Universe {
    pub days: f64,
    /// Candidates passing every market check.
    pub passes: usize,
    pub per_day: Option<f64>,
    pub per_coin: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Funding {
    /// Side receives funding beyond 0.5 bps/h (against the crowd).
    pub opposing: Est,
    /// Side pays funding beyond 0.5 bps/h (with the crowd).
    pub with: Est,
    /// |funding| < 0.125 bps/h.
    pub calm: Est,
    pub diff: Option<f64>,
    pub null_pct: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub id: String,
    /// "pass", "invalidated" or "testing". The register's status stays a human decision.
    pub verdict: String,
    pub progress: f64,
    pub need: f64,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScalpScorecard {
    pub generated_at: DateTime<Utc>,
    pub first: Option<DateTime<Utc>>,
    pub last: Option<DateTime<Utc>>,
    pub candidates: usize,
    pub hours: usize,
    /// Main horizon (the longest markout), seconds.
    pub h: i64,
    pub edge: Vec<EdgeRow>,
    pub fills: Fills,
    pub trades: TradeStats,
    pub exits: Vec<ExitRow>,
    pub gates: Vec<GateRow>,
    pub features: Vec<FeatureRow>,
    pub universe: Universe,
    pub funding: Funding,
    pub verdicts: Vec<Verdict>,
    pub backlog: Vec<BacklogEntry>,
}

// ---------------------------------------------------------------- statistics

/// 97.5% Student-t quantile: table below 5 df, Cornish–Fisher above (within 0.03).
fn t975(df: usize) -> f64 {
    const TABLE: [f64; 4] = [12.706, 4.303, 3.182, 2.776];
    if (1..=4).contains(&df) {
        return TABLE[df - 1];
    }
    let (z, d) = (1.959_964_f64, df.max(1) as f64);
    z + (z.powi(3) + z) / (4.0 * d) + (5.0 * z.powi(5) + 16.0 * z.powi(3) + 3.0 * z) / (96.0 * d * d)
}

/// (hour, value) → mean with a cluster-robust (CR1) 95% interval, t with hours − 1 df.
fn est(v: &[(i64, f64)]) -> Est {
    let n = v.len();
    if n == 0 {
        return Est { low_n: true, ..Default::default() };
    }
    let mean = v.iter().map(|x| x.1).sum::<f64>() / n as f64;
    let mut g: BTreeMap<i64, f64> = BTreeMap::new();
    for (h, x) in v {
        *g.entry(*h).or_default() += x - mean;
    }
    let k = g.len();
    let (lo, hi) = if k >= 2 {
        let se = (g.values().map(|s| s * s).sum::<f64>() * k as f64 / (k - 1) as f64).sqrt() / n as f64;
        let t = t975(k - 1);
        (Some(mean - t * se), Some(mean + t * se))
    } else {
        (None, None)
    };
    Est { n, hours: k, mean: Some(mean), lo, hi, low_n: k < MIN_SOLID }
}

fn verdict(net: &Est) -> &'static str {
    match (net.lo, net.hi) {
        (Some(lo), _) if lo > 0.0 => "pays after costs",
        (_, Some(hi)) if hi < 0.0 => "loses after costs",
        _ => "undecided",
    }
}

/// Mean of y where x == 1 minus mean where x == 0.
fn diff_means(x: &[f64], y: &[f64]) -> Option<f64> {
    let (mut a, mut na, mut b, mut nb) = (0.0, 0.0, 0.0, 0.0);
    for (xi, yi) in x.iter().zip(y) {
        if *xi > 0.5 {
            a += yi;
            na += 1.0;
        } else {
            b += yi;
            nb += 1.0;
        }
    }
    (na > 0.0 && nb > 0.0).then(|| a / na - b / nb)
}

// ---------------------------------------------------------------- scorecard

struct Row<'a> {
    c: &'a Cand,
    hour: i64,
    /// Some(true) filled or partial, Some(false) missed, None never posted.
    filled: Option<bool>,
}

type Marks<'a> = HashMap<(&'a str, i64), &'a Markout>;

fn joined<'a>(rows: &[&Row<'a>], mark: &Marks<'a>, h: i64) -> Vec<(i64, &'a Markout)> {
    rows.iter().filter_map(|r| mark.get(&(r.c.id.as_str(), h)).map(|m| (r.hour, *m))).collect()
}

fn col(v: &[(i64, &Markout)], f: impl Fn(&Markout) -> f64) -> Vec<(i64, f64)> {
    v.iter().map(|(h, m)| (*h, f(m))).collect()
}

pub fn scorecard(logs: &Logs, cfg: &ScalpConfig) -> ScalpScorecard {
    let order: HashMap<&str, bool> = logs.orders.iter().map(|o| (o.candidate_id.as_str(), o.outcome != "missed")).collect();
    // Candidates from before the activity baseline existed are not evidence.
    let mut rows: Vec<Row> = logs
        .cands
        .iter()
        .filter(|c| c.blocked_by.as_deref() != Some("warm-up"))
        .map(|c| Row { c, hour: c.features.ts_ms.div_euclid(HOUR_MS), filled: order.get(c.id.as_str()).copied() })
        .collect();
    rows.sort_by_key(|r| r.c.features.ts_ms);
    let all: Vec<&Row> = rows.iter().collect();
    let mark: Marks = logs.marks.iter().map(|m| ((m.id.as_str(), m.h_secs), m)).collect();
    let h = cfg.markouts.iter().copied().max().unwrap_or(H_TEST);
    let ts = |r: &Row| DateTime::from_timestamp_millis(r.c.features.ts_ms);

    // Does the signal pay?
    let groups: [(&str, fn(&Row) -> bool); 4] =
        [(PASSED, |r| r.c.eval.ok), (PLACED, |r| r.c.taken), (FILLED, |r| r.filled == Some(true)), (ALL, |_| true)];
    let mut edge = Vec::new();
    for &hz in &cfg.markouts {
        for (g, keep) in groups {
            let sel: Vec<&Row> = all.iter().copied().filter(|r| keep(r)).collect();
            let ms = joined(&sel, &mark, hz);
            if ms.is_empty() {
                continue;
            }
            let net = est(&col(&ms, |m| m.net_bps));
            edge.push(EdgeRow {
                group: g.into(),
                h: hz,
                gross: est(&col(&ms, |m| m.ret_bps)),
                cost: stats::mean(&ms.iter().map(|(_, m)| m.cost_bps).collect::<Vec<_>>()),
                verdict: verdict(&net).into(),
                net,
            });
        }
    }

    // Maker fills: adverse selection and the taker counterfactual.
    let filled_rows: Vec<&Row> = all.iter().copied().filter(|r| r.filled == Some(true)).collect();
    let missed_rows: Vec<&Row> = all.iter().copied().filter(|r| r.filled == Some(false)).collect();
    let posted: Vec<&Row> = all.iter().copied().filter(|r| r.filled.is_some()).collect();
    let n_filled = logs.orders.iter().filter(|o| o.outcome != "missed").count();
    let mut maker_ev = Vec::new();
    let mut taker_ev = Vec::new();
    for r in &posted {
        if let Some(m) = mark.get(&(r.c.id.as_str(), H_TEST)) {
            maker_ev.push((r.hour, if r.filled == Some(true) { m.net_bps } else { 0.0 }));
            // The logged cost is the maker round trip; a taker entry pays the taker fee
            // instead of the maker fee, one more slippage and the other half of the spread.
            let taker_cost = m.cost_bps + cfg.fee_bps - cfg.maker_fee_bps + cfg.slippage_bps + 0.5 * r.c.features.spread_bps;
            taker_ev.push((r.hour, m.ret_bps - taker_cost));
        }
    }
    let fills = Fills {
        orders: logs.orders.len(),
        filled: n_filled,
        missed: logs.orders.len() - n_filled,
        fill_rate: (!logs.orders.is_empty()).then(|| n_filled as f64 / logs.orders.len() as f64),
        filled_ret: est(&col(&joined(&filled_rows, &mark, H_TEST), |m| m.ret_bps)),
        missed_ret: est(&col(&joined(&missed_rows, &mark, H_TEST), |m| m.ret_bps)),
        maker_ev: est(&maker_ev),
        taker_ev: est(&taker_ev),
    };

    // Round trips and exits.
    let trades = TradeStats {
        n: logs.trades.len(),
        wins: logs.trades.iter().filter(|t| t.pnl_usd > 0.0).count(),
        pnl_usd: logs.trades.iter().map(|t| t.pnl_usd).sum(),
        fees_usd: logs.trades.iter().map(|t| t.fees_usd).sum(),
        bps: est(&logs.trades.iter().map(|t| (t.opened.timestamp_millis().div_euclid(HOUR_MS), t.pnl_bps)).collect::<Vec<_>>()),
    };
    let mut by_reason: BTreeMap<&str, (Vec<f64>, Vec<f64>)> = BTreeMap::new();
    for t in &logs.trades {
        let e = by_reason.entry(t.reason.as_str()).or_default();
        e.0.push(t.pnl_bps);
        if let Some(m) = mark.get(&(t.candidate_id.as_str(), H_TEST)) {
            e.1.push(m.net_bps - t.pnl_bps);
        }
    }
    let mut exits: Vec<ExitRow> = by_reason
        .into_iter()
        .map(|(reason, (realized, delta))| ExitRow {
            reason: reason.into(),
            n: realized.len(),
            realized_bps: stats::mean(&realized),
            n_held: delta.len(),
            hold_minus_exit_bps: (!delta.is_empty()).then(|| stats::mean(&delta)),
        })
        .collect();
    exits.sort_by(|a, b| b.n.cmp(&a.n));

    // What each check stopped, and how those candidates did.
    let mut gm: BTreeMap<String, Vec<(i64, f64)>> = BTreeMap::new();
    for (r, m) in all.iter().filter_map(|r| mark.get(&(r.c.id.as_str(), h)).map(|m| (r, m))) {
        let g = if r.c.taken { "taken".to_string() } else { r.c.blocked_by.clone().unwrap_or_else(|| "?".into()) };
        gm.entry(g).or_default().push((r.hour, m.net_bps));
    }
    let mut gates: Vec<GateRow> = gm.into_iter().map(|(gate, v)| GateRow { gate, net: est(&v) }).collect();
    gates.sort_by(|a, b| (b.gate == "taken").cmp(&(a.gate == "taken")).then(b.net.n.cmp(&a.net.n)));

    // Which inputs relate to the move: Spearman via ranks computed once.
    let fr: Vec<(&Row, &Markout)> = all.iter().filter_map(|r| mark.get(&(r.c.id.as_str(), h)).map(|m| (*r, *m))).collect();
    let fr_hours = fr.iter().map(|(r, _)| r.hour).collect::<BTreeSet<_>>().len();
    let block = ((fr.len() as f64 / fr_hours.max(1) as f64).ceil() as usize).max(2);
    let ry = stats::ranks(&fr.iter().map(|(_, m)| m.ret_bps).collect::<Vec<_>>());
    let features = FEATURES
        .iter()
        .map(|(name, signed, get)| {
            let x: Vec<f64> = fr.iter().map(|(r, _)| get(&r.c.features) * if *signed { r.c.side.sign() } else { 1.0 }).collect();
            let rx = stats::ranks(&x);
            let null_pct = if fr.len() >= MIN_SHOW { stats::null_percentile(&rx, &ry, block, NULL_ITERS, stats::pearson) } else { None };
            FeatureRow { feature: name.to_string(), signed: *signed, n: fr.len(), ic: stats::pearson(&rx, &ry), null_pct, p: null_pct.map(stats::p_two_sided) }
        })
        .collect();

    // Enough chances per day?
    let (first, last) = (rows.first().and_then(ts), rows.last().and_then(ts));
    let days = match (rows.first(), rows.last()) {
        (Some(a), Some(b)) => (b.c.features.ts_ms - a.c.features.ts_ms) as f64 / 86_400_000.0,
        _ => 0.0,
    };
    let mut per_coin = BTreeMap::new();
    for r in rows.iter().filter(|r| r.c.eval.ok) {
        *per_coin.entry(r.c.asset.clone()).or_insert(0) += 1;
    }
    let passes = per_coin.values().sum();
    let universe = Universe { days, passes, per_day: (days > 0.0).then(|| passes as f64 / days), per_coin };

    // Funding: against the crowd vs with it.
    let (mut opp, mut with, mut calm, mut ind, mut net) = (vec![], vec![], vec![], vec![], vec![]);
    for (r, m) in all.iter().filter_map(|r| mark.get(&(r.c.id.as_str(), H_TEST)).map(|m| (r, m))) {
        let f = r.c.side.sign() * r.c.features.funding_bps_1h;
        if f <= -0.5 {
            opp.push((r.hour, m.net_bps));
            ind.push(1.0);
            net.push(m.net_bps);
        } else if f >= 0.5 {
            with.push((r.hour, m.net_bps));
            ind.push(0.0);
            net.push(m.net_bps);
        }
        if f.abs() < 0.125 {
            calm.push((r.hour, m.net_bps));
        }
    }
    let fblock = ((ind.len() as f64 / fr_hours.max(1) as f64).ceil() as usize).max(2);
    let funding = Funding {
        diff: diff_means(&ind, &net),
        null_pct: if opp.len() >= MIN_SHOW && with.len() >= MIN_SHOW { stats::null_percentile(&ind, &net, fblock, NULL_ITERS, diff_means) } else { None },
        opposing: est(&opp),
        with: est(&with),
        calm: est(&calm),
    };

    let taken_test = est(&col(&joined(&all.iter().copied().filter(|r| r.c.taken).collect::<Vec<_>>(), &mark, H_TEST), |m| m.net_bps));
    let mut sc = ScalpScorecard {
        generated_at: Utc::now(),
        first,
        last,
        candidates: rows.len(),
        hours: rows.iter().map(|r| r.hour).collect::<BTreeSet<_>>().len(),
        h,
        edge,
        fills,
        trades,
        exits,
        gates,
        features,
        universe,
        funding,
        verdicts: Vec::new(),
        backlog: Vec::new(),
    };
    sc.verdicts = verdicts(&sc, &taken_test);
    sc.backlog = backlog(&sc);
    sc
}

// ---------------------------------------------------------------- hypotheses

fn bps(x: Option<f64>) -> String {
    x.map(|v| format!("{v:+.1}")).unwrap_or_else(|| "—".into())
}

fn ci(e: &Est) -> String {
    match (e.lo, e.hi) {
        (Some(lo), Some(hi)) => format!("{} bps (95% {lo:+.1} to {hi:+.1})", bps(e.mean)),
        _ => format!("{} bps", bps(e.mean)),
    }
}

/// The H-C register's pass/invalidate rules, applied to the data.
// ponytail: thresholds copied by hand from hypotheses.toml; change both together.
fn verdicts(sc: &ScalpScorecard, taken: &Est) -> Vec<Verdict> {
    let v = |id: &str, verdict: &str, progress: f64, need: f64, detail: String| Verdict { id: id.into(), verdict: verdict.into(), progress, need, detail };
    let mut out = Vec::new();

    let c01 = match (taken.n >= 150, taken.mean, taken.lo) {
        (true, Some(m), Some(lo)) if m > 0.0 && lo > 0.0 => "pass",
        (true, Some(m), _) if m <= 0.0 => "invalidated",
        _ => "testing",
    };
    out.push(v("H-C01", c01, taken.n as f64, 150.0, format!("{} of 150 taken · net at 600s {}", taken.n, ci(taken))));

    let f = &sc.fills;
    let gap = match (f.filled_ret.mean, f.missed_ret.mean) {
        (Some(a), Some(b)) => Some(a - b),
        _ => None,
    };
    let fr = f.fill_rate.unwrap_or(0.0);
    let c02 = if f.orders < 100 {
        "testing"
    } else if fr >= 0.40 && gap.is_some_and(|g| g >= -4.5) {
        "pass"
    } else if fr < 0.25 || gap.is_some_and(|g| g < -4.5) {
        "invalidated"
    } else {
        "testing"
    };
    out.push(v("H-C02", c02, f.orders as f64, 100.0, format!("{} of 100 orders · {:.0}% filled (needs ≥ 40%) · filled − missed at 600s {} bps (needs ≥ −4.5)", f.orders, fr * 100.0, bps(gap))));

    let u = &sc.universe;
    let c03 = match (u.days >= 7.0, u.per_day) {
        (true, Some(p)) if p >= 10.0 => "pass",
        (true, Some(p)) if p < 3.0 => "invalidated",
        _ => "testing",
    };
    out.push(v("H-C03", c03, u.days.min(7.0), 7.0, format!("{:.1} of 7 days · {} market-check passes per day (needs ≥ 10)", u.days, u.per_day.map(|p| format!("{p:.0}")).unwrap_or_else(|| "—".into()))));

    let fu = &sc.funding;
    let each = fu.opposing.n.min(fu.with.n);
    let c04 = match (each >= 40, fu.diff, fu.null_pct) {
        (true, Some(d), Some(p)) if d >= 3.0 && p > 90.0 => "pass",
        (true, _, Some(p)) if p <= 90.0 => "invalidated",
        _ => "testing",
    };
    out.push(v(
        "H-C04",
        c04,
        each as f64,
        40.0,
        format!("{} against / {} with the crowd (needs 40 each) · difference {} bps (needs ≥ +3, null pct > 90){}", fu.opposing.n, fu.with.n, bps(fu.diff), fu.null_pct.map(|p| format!(", null pct {p:.0}")).unwrap_or_default()),
    ));
    out
}

// ---------------------------------------------------------------- backlog

/// Patterns with enough evidence, as suggestions.
pub fn backlog(sc: &ScalpScorecard) -> Vec<BacklogEntry> {
    let now = sc.generated_at;
    let mut out = Vec::new();
    let mut push = |severity: &str, pattern: String, n: usize, effect: String, null_percentile: Option<f64>, suggestion: String| {
        out.push(BacklogEntry { ts: now, pattern, n, effect, null_percentile, severity: severity.into(), suggestion });
    };

    if let Some(e) = sc.edge.iter().find(|e| e.group == PASSED && e.h == sc.h)
        && e.net.hours >= MIN_SUGGEST
        && e.net.hi.is_some_and(|hi| hi < 0.0)
    {
        let before = if e.gross.lo.is_some_and(|lo| lo <= 0.0) { "; no edge before costs either" } else { "" };
        push(
            "high",
            format!("the signal loses after costs at {}s", e.h),
            e.net.hours,
            format!("gross {} vs cost {:.1} bps, net {}{before}", ci(&e.gross), e.cost, ci(&e.net)),
            None,
            "don't tune thresholds or go live: find an input whose gross edge clears the cost first".into(),
        );
    }

    let f = &sc.fills;
    if f.filled >= MIN_SUGGEST
        && f.missed >= MIN_SUGGEST
        && let (Some(m), Some(t)) = (f.maker_ev.mean, f.taker_ev.mean)
        && t > m
    {
        push(
            "high",
            "maker entries lose more to adverse selection than the fee they save".into(),
            f.orders,
            format!("filled {} vs missed {} bps at 600s; per order maker {m:+.1} vs taker {t:+.1} bps", bps(f.filled_ret.mean), bps(f.missed_ret.mean)),
            None,
            "write a hypothesis for taker entries or a different post price (see H-C02)".into(),
        );
    }

    if let Some(taken) = sc.gates.iter().find(|g| g.gate == "taken").filter(|g| g.net.n >= MIN_SUGGEST)
        && let Some(tm) = taken.net.mean
    {
        for g in sc.gates.iter().filter(|g| g.gate != "taken" && g.net.n >= MIN_SUGGEST) {
            if let Some(gm) = g.net.mean.filter(|gm| *gm > tm) {
                push(
                    if gm > 0.0 { "medium" } else { "low" },
                    format!("'{}' stops candidates that did better than the ones taken", g.gate),
                    g.net.n,
                    format!("{gm:+.1} vs {tm:+.1} bps net at {}s", sc.h),
                    None,
                    format!("check what '{}' selects on; write a hypothesis before changing it", g.gate),
                );
            }
        }
    }

    for x in &sc.exits {
        if x.n_held >= MIN_SUGGEST
            && let Some(d) = x.hold_minus_exit_bps.filter(|d| *d > 2.0)
        {
            push(
                "medium",
                format!("exit '{}' gives up money vs holding to the time stop", x.reason),
                x.n_held,
                format!("realized {:+.1} bps; holding to 600s would have added {d:+.1} bps", x.realized_bps),
                None,
                format!("write a hypothesis to loosen or drop the '{}' exit", x.reason),
            );
        }
    }

    if sc.hours >= MIN_SUGGEST {
        for r in &sc.features {
            match r.p {
                Some(p) if p <= 0.05 => push(
                    "low",
                    format!("'{}' relates to the {}s move", r.feature, sc.h),
                    r.n,
                    format!("IC {}", r.ic.map(|v| format!("{v:+.3}")).unwrap_or_default()),
                    r.null_pct,
                    "record-only: write a hypothesis before trading on it".into(),
                ),
                Some(p) if p > 0.1 && ENTRY_INPUTS.contains(&r.feature.as_str()) => push(
                    "medium",
                    format!("entry input '{}' shows no relation to the {}s move", r.feature, sc.h),
                    r.n,
                    format!("IC {}", r.ic.map(|v| format!("{v:+.3}")).unwrap_or_default()),
                    r.null_pct,
                    "its gate filters noise: don't tune its threshold".into(),
                ),
                _ => {}
            }
        }
    }
    out
}

// ---------------------------------------------------------------- markdown

fn grey(e: &Est) -> &'static str {
    if e.low_n { " *(< 30 h)*" } else { "" }
}

pub fn markdown(sc: &ScalpScorecard, hyps: &[Hypothesis]) -> String {
    let mut s = String::new();
    let span = match (sc.first, sc.last) {
        (Some(a), Some(b)) => format!("{} → {}", a.format("%Y-%m-%d %H:%M"), b.format("%Y-%m-%d %H:%M UTC")),
        _ => "no data".into(),
    };
    let _ = writeln!(s, "# Scalper report\n");
    let _ = writeln!(
        s,
        "*Generated {} · {span} · {} candidates over {} distinct hours.* Means carry 95% intervals with errors clustered by UTC hour (the independent unit); figures resting on fewer than 30 hours are marked. Correlations carry their percentile within a block-permutation null ({NULL_ITERS} shuffles, ~1h blocks). Moves are in bps in the trade's favour from the decision mid.\n",
        sc.generated_at.format("%Y-%m-%d %H:%M UTC"),
        sc.candidates,
        sc.hours
    );

    let _ = writeln!(s, "## Hypotheses\n\nThe data's verdict on each scalper test. The status in `hypotheses.toml` stays your decision.\n");
    let _ = writeln!(s, "| Test | Data says | Progress | Detail |\n|---|---|---|---|");
    for v in &sc.verdicts {
        let status = hyps.iter().find(|h| h.id == v.id).map(|h| format!(" (register: {})", h.status)).unwrap_or_default();
        let _ = writeln!(s, "| {}{status} | **{}** | {:.0} / {:.0} | {} |", v.id, v.verdict, v.progress, v.need, v.detail);
    }

    let _ = writeln!(s, "\n## Does the signal pay?\n\n| Group | Horizon | Gross | Cost | Net | Verdict |\n|---|---|---|---|---|---|");
    for e in &sc.edge {
        let _ = writeln!(s, "| {} | {}s | {} | {:.1} | {}{} · n {} | {} |", e.group, e.h, ci(&e.gross), e.cost, ci(&e.net), grey(&e.net), e.net.n, e.verdict);
    }

    let f = &sc.fills;
    let _ = writeln!(s, "\n## Maker fills\n");
    let _ = writeln!(
        s,
        "{} orders, {} filled, {} missed ({}). Gross move at 600s: filled {}{}, missed {}{}. Per posted order at 600s: maker as run {}, taker entry on every one {}.",
        f.orders,
        f.filled,
        f.missed,
        f.fill_rate.map(|r| format!("{:.0}% fill rate", r * 100.0)).unwrap_or_else(|| "no orders".into()),
        ci(&f.filled_ret),
        grey(&f.filled_ret),
        ci(&f.missed_ret),
        grey(&f.missed_ret),
        ci(&f.maker_ev),
        ci(&f.taker_ev),
    );

    let t = &sc.trades;
    let _ = writeln!(s, "\n## Round trips and exits\n");
    let _ = writeln!(s, "{} trades, {} won, ${:+.2} after ${:.2} fees · per trade {}{}.\n", t.n, t.wins, t.pnl_usd, t.fees_usd, ci(&t.bps), grey(&t.bps));
    if !sc.exits.is_empty() {
        let _ = writeln!(s, "| Exit | n | Realized bps | Holding to 600s instead would add | n |\n|---|---|---|---|---|");
        for x in &sc.exits {
            let _ = writeln!(s, "| {} | {} | {:+.1} | {} | {} |", x.reason, x.n, x.realized_bps, bps(x.hold_minus_exit_bps), x.n_held);
        }
    }

    let _ = writeln!(s, "\n## What each check stopped ({}s net)\n\nIf the checks select well, \"taken\" beats every row below it.\n\n| Stopped by | Net | n |\n|---|---|---|", sc.h);
    for g in &sc.gates {
        let _ = writeln!(s, "| {} | {}{} | {} |", g.gate, ci(&g.net), grey(&g.net), g.net.n);
    }

    let _ = writeln!(s, "\n## Which inputs relate to the {}s move\n\nSpearman IC over every candidate. Directional inputs are signed by the side (positive = in the trade's favour; funding positive = the side pays).\n\n| Input | IC | null pct | p | n |\n|---|---|---|---|---|", sc.h);
    for r in &sc.features {
        let _ = writeln!(
            s,
            "| {}{} | {} | {} | {} | {} |",
            r.feature,
            if ENTRY_INPUTS.contains(&r.feature.as_str()) { " (entry gate)" } else { "" },
            r.ic.map(|v| format!("{v:+.3}")).unwrap_or_else(|| "—".into()),
            r.null_pct.map(|p| format!("{p:.0}")).unwrap_or_else(|| "—".into()),
            r.p.map(|p| format!("{p:.2}")).unwrap_or_else(|| "—".into()),
            r.n
        );
    }

    let u = &sc.universe;
    let _ = writeln!(s, "\n## Chances per coin\n\n{} market-check passes over {:.2} days.\n", u.passes, u.days);
    for (coin, n) in &u.per_coin {
        let _ = writeln!(s, "- {coin}: {n}");
    }

    let fu = &sc.funding;
    let _ = writeln!(
        s,
        "\n## Funding (600s net)\n\nAgainst the crowd {} · n {} · with the crowd {} · n {} · calm (|funding| < 0.125 bps/h) {} · n {}.",
        ci(&fu.opposing),
        fu.opposing.n,
        ci(&fu.with),
        fu.with.n,
        ci(&fu.calm),
        fu.calm.n
    );

    let _ = writeln!(s, "\n## Backlog (suggestions only)\n");
    if sc.backlog.is_empty() {
        let _ = writeln!(s, "Nothing has n ≥ {MIN_SUGGEST} (hours or cases) and a clear effect yet.");
    }
    for b in &sc.backlog {
        let _ = writeln!(s, "- **[{}] {}** — n={}, {}{}. → {}", b.severity, b.pattern, b.n, b.effect, b.null_percentile.map(|p| format!(", null pct {p:.0}")).unwrap_or_default(), b.suggestion);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 48 hours, 8 candidates an hour, every one placed. The planted edge is +20 bps
    /// gross at 600s, and filled orders move 12 bps worse than missed ones.
    fn logs(edge: f64) -> Logs {
        let mut rng = stats::Rng::new(9);
        let (mut cands, mut marks, mut orders) = (vec![], vec![], vec![]);
        for i in 0..48 * 8 {
            let id = format!("c{i}");
            let side = if i % 2 == 0 { Side::Long } else { Side::Short };
            let filled = i % 3 == 0;
            let ret = edge + if filled { -8.0 } else { 4.0 } + 5.0 * rng.normal();
            let features = Features { ts_ms: i as i64 * 450_000, spread_bps: 1.0, flow: rng.normal(), ..Default::default() };
            cands.push(Cand { id: id.clone(), asset: "SOL".into(), side, features, eval: Eval { ok: true }, taken: true, blocked_by: None });
            marks.push(Markout { id: id.clone(), h_secs: 600, ret_bps: ret, net_bps: ret - 7.5, cost_bps: 7.5 });
            orders.push(Order { candidate_id: id, outcome: if filled { "filled" } else { "missed" }.into() });
        }
        Logs { cands, marks, orders, trades: vec![] }
    }

    #[test]
    fn verdicts_and_backlog_follow_the_data() {
        assert_eq!(t975(1), 12.706);
        assert!((t975(30) - 2.042).abs() < 0.01);

        let cfg = ScalpConfig::default();
        let sc = scorecard(&logs(20.0), &cfg);
        let passed = sc.edge.iter().find(|e| e.group == PASSED && e.h == 600).unwrap();
        assert_eq!(passed.net.hours, 48);
        assert_eq!(passed.verdict, "pays after costs");
        let v = |id: &str| sc.verdicts.iter().find(|v| v.id == id).unwrap().verdict.clone();
        assert_eq!(v("H-C01"), "pass");
        assert_eq!(v("H-C02"), "invalidated", "filled orders run 12 bps worse than missed");
        assert!(sc.backlog.iter().any(|b| b.pattern.starts_with("maker entries")));

        let sc = scorecard(&logs(-2.0), &cfg);
        assert_eq!(sc.verdicts[0].verdict, "invalidated");
        assert!(sc.backlog.iter().any(|b| b.pattern.starts_with("the signal loses")));
        assert!(sc.backlog.iter().any(|b| b.pattern == "entry input 'flow' shows no relation to the 600s move"));
    }
}
