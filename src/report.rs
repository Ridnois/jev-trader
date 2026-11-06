//! 1.3 / 1.4: statistics over settled records, with noise defences, as a markdown
//! report (`jev-trader report`), a dashboard scorecard, and an improvement backlog.
//! Every figure carries its n; effects carry their percentile within a
//! block-permutation null. Suggestions only: nothing here edits config.toml.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::hypotheses::Hypothesis;
use crate::settle::{Kind, Settlement};
use crate::stats;

/// n below which a weight suggestion is never made.
pub const MIN_SUGGEST: usize = 20;
/// n below which a statistic is shown greyed out.
pub const MIN_SOLID: usize = 30;
/// n below which a cohort is hidden.
pub const MIN_SHOW: usize = 5;
const NULL_ITERS: usize = 1000;

#[derive(Debug, Clone, Serialize)]
pub struct IcRow {
    pub h: u32,
    pub n_raw: usize,
    pub n_clusters: usize,
    pub ic: Option<f64>,
    pub null_pct: Option<f64>,
    pub p: Option<f64>,
    pub hit_rate: Option<f64>,
    pub hit_null_pct: Option<f64>,
    pub low_n: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Cohort {
    pub dim: String,
    pub key: String,
    pub n_clusters: usize,
    pub ic: Option<f64>,
    pub p: Option<f64>,
    pub hit_rate: Option<f64>,
    pub low_n: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GateValue {
    pub gate: String,
    pub n: usize,
    pub missed: usize,
    pub correctly_blocked: usize,
    /// Mean 24h return of the blocked entries (positive = the gate cost money).
    pub mean_ret: f64,
    pub low_n: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecisionStats {
    pub n: usize,
    pub n_clusters: usize,
    pub hit_rate: Option<f64>,
    /// Mean direction-adjusted 24h return per decision.
    pub expectancy: Option<f64>,
    pub tags: BTreeMap<String, usize>,
    pub regime_mismatch: usize,
    pub low_n: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StopCalibration {
    pub stop_pct: f64,
    /// Long entries (taken or blocked) that were up at 24h.
    pub n_winners: usize,
    /// Share of those winners whose MAE first went below the stop.
    pub share_below_stop: Option<f64>,
    pub mae_q50: Option<f64>,
    pub mae_q90: Option<f64>,
    pub low_n: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Scorecard {
    pub label: String,
    pub generated_at: DateTime<Utc>,
    pub n_settled: usize,
    pub n_failed: usize,
    pub headlines: Vec<IcRow>,
    pub cohorts: Vec<Cohort>,
    pub decisions: DecisionStats,
    pub gates: Vec<GateValue>,
    pub stop: StopCalibration,
    pub reaction_fcr: (f64, f64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacklogEntry {
    pub ts: DateTime<Utc>,
    pub pattern: String,
    pub n: usize,
    pub effect: String,
    pub null_percentile: Option<f64>,
    /// low | medium | high
    pub severity: String,
    pub suggestion: String,
}

/// Headline signal strength: signed sentiment × new_info.
fn headline_x(s: &Settlement) -> f64 {
    s.rec.sentiment.unwrap_or(0.0) * s.rec.new_info.unwrap_or(0.0)
}

/// (t0, x, y) per asset → independent clusters (window = horizon), pooled.
fn clustered(
    rows: &[(&Settlement, f64, f64)],
    window: Duration,
) -> (Vec<f64>, Vec<f64>, f64) {
    let mut by_asset: BTreeMap<&str, Vec<(DateTime<Utc>, f64, f64)>> = BTreeMap::new();
    for (s, x, y) in rows {
        by_asset.entry(s.rec.asset.as_str()).or_default().push((s.rec.t0, *x, *y));
    }
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    let (mut first, mut last) = (None::<DateTime<Utc>>, None::<DateTime<Utc>>);
    for (_, mut v) in by_asset {
        v.sort_by_key(|r| r.0);
        first = Some(first.map_or(v[0].0, |f| f.min(v[0].0)));
        last = Some(last.map_or(v[v.len() - 1].0, |l| l.max(v[v.len() - 1].0)));
        for (x, y) in stats::merge_pairs(&v, window) {
            xs.push(x);
            ys.push(y);
        }
    }
    let span_days = match (first, last) {
        (Some(f), Some(l)) => ((l - f).num_hours() as f64 / 24.0).max(1.0),
        _ => 1.0,
    };
    (xs, ys, span_days)
}

/// Block length (in clusters) covering at least 24h.
fn block_len(n: usize, span_days: f64) -> usize {
    ((n as f64 / span_days).ceil() as usize).max(2)
}

fn ic_row(rows: &[(&Settlement, f64, f64)], h: u32) -> IcRow {
    let (x, y, span) = clustered(rows, Duration::hours(h as i64));
    let n = x.len();
    let block = block_len(n, span);
    let ic = stats::spearman(&x, &y);
    let null_pct = if n >= MIN_SHOW { stats::null_percentile(&x, &y, block, NULL_ITERS, stats::spearman) } else { None };
    let hit = stats::hit_rate(&x, &y);
    let hit_null = if n >= MIN_SHOW { stats::null_percentile(&x, &y, block, NULL_ITERS, stats::hit_rate) } else { None };
    IcRow {
        h,
        n_raw: rows.len(),
        n_clusters: n,
        ic,
        null_pct,
        p: null_pct.map(stats::p_two_sided),
        hit_rate: hit,
        hit_null_pct: hit_null,
        low_n: n < MIN_SOLID,
    }
}

pub fn scorecard(settled: &[Settlement], cfg: &Config, label: &str) -> Scorecard {
    let ok: Vec<&Settlement> = settled.iter().filter(|s| s.status == "SETTLED").collect();
    let n_failed = settled.len() - ok.len();
    let heads: Vec<&Settlement> = ok.iter().copied().filter(|s| s.rec.kind == Kind::Headline && s.rec.event.as_deref() != Some("noise")).collect();

    let mut headlines = Vec::new();
    for &h in &cfg.settle.horizons {
        let rows: Vec<(&Settlement, f64, f64)> =
            heads.iter().filter_map(|s| s.horizon(h).and_then(|r| r.z).map(|z| (*s, headline_x(s), z))).collect();
        headlines.push(ic_row(&rows, h));
    }

    // Breakdowns at 4h (or the middle horizon).
    let bh = if cfg.settle.horizons.contains(&4) { 4 } else { cfg.settle.horizons[cfg.settle.horizons.len() / 2] };
    let mut cohorts = Vec::new();
    let dims: [(&str, fn(&Settlement) -> Option<String>); 4] = [
        ("event", |s| s.rec.event.clone()),
        ("source", |s| s.rec.source.clone()),
        ("asset", |s| Some(s.rec.asset.clone())),
        ("regime", |s| s.rec.regime_at_signal.clone()),
    ];
    for (dim, key) in dims {
        let mut groups: BTreeMap<String, Vec<(&Settlement, f64, f64)>> = BTreeMap::new();
        for s in &heads {
            if let (Some(k), Some(z)) = (key(s), s.horizon(bh).and_then(|r| r.z)) {
                groups.entry(k).or_default().push((*s, headline_x(s), z));
            }
        }
        for (k, rows) in groups {
            let r = ic_row(&rows, bh);
            if r.n_clusters < MIN_SHOW {
                continue;
            }
            cohorts.push(Cohort { dim: dim.into(), key: k, n_clusters: r.n_clusters, ic: r.ic, p: r.p, hit_rate: r.hit_rate, low_n: r.low_n });
        }
    }

    // Decisions at 24h (or the longest horizon).
    let dh = cfg.settle.horizons.iter().copied().max().unwrap_or(24);
    let decs: Vec<&Settlement> = ok.iter().copied().filter(|s| s.rec.kind == Kind::Decision).collect();
    let mut tags = BTreeMap::new();
    let mut drets = Vec::new();
    let mut drows = Vec::new();
    for s in &decs {
        if let Some(r) = s.horizon(dh) {
            *tags.entry(r.tag.clone()).or_insert(0) += 1;
            drets.push(s.rec.direction as f64 * r.ret);
            drows.push((*s, s.rec.direction as f64, r.ret));
        }
    }
    let (dx, dy, _) = clustered(&drows, Duration::hours(dh as i64));
    let decisions = DecisionStats {
        n: decs.len(),
        n_clusters: dx.len(),
        hit_rate: stats::hit_rate(&dx, &dy),
        expectancy: (!drets.is_empty()).then(|| stats::mean(&drets)),
        tags,
        regime_mismatch: decs.iter().filter(|s| s.regime_mismatch).count(),
        low_n: dx.len() < MIN_SOLID,
    };

    // Gate values from counterfactuals.
    let mut gmap: BTreeMap<String, Vec<&Settlement>> = BTreeMap::new();
    for s in ok.iter().copied().filter(|s| s.rec.kind == Kind::Counterfactual) {
        gmap.entry(s.rec.blocked_by.clone().unwrap_or_else(|| "?".into())).or_default().push(s);
    }
    let gates = gmap
        .into_iter()
        .map(|(gate, v)| {
            let rs: Vec<&crate::settle::HorizonResult> = v.iter().filter_map(|s| s.horizon(dh)).collect();
            let missed = rs.iter().filter(|r| r.label.as_deref() == Some("MISSED_OPPORTUNITY")).count();
            GateValue {
                gate,
                n: rs.len(),
                missed,
                correctly_blocked: rs.len() - missed,
                mean_ret: stats::mean(&rs.iter().map(|r| r.ret).collect::<Vec<_>>()),
                low_n: rs.len() < MIN_SOLID,
            }
        })
        .collect();

    // Stop calibration from MAE of winners (long entries, taken or blocked).
    let maes: Vec<f64> = ok
        .iter()
        .filter(|s| matches!(s.rec.kind, Kind::Decision | Kind::Counterfactual) && s.rec.direction > 0)
        .filter_map(|s| s.horizon(dh))
        .filter(|r| r.ret > 0.0)
        .map(|r| r.mae)
        .collect();
    let stop_pct = cfg.risk.stop_loss_pct;
    let stop = StopCalibration {
        stop_pct,
        n_winners: maes.len(),
        share_below_stop: (!maes.is_empty()).then(|| maes.iter().filter(|m| **m <= -stop_pct).count() as f64 / maes.len() as f64),
        mae_q50: stats::quantile(&maes, 0.5),
        mae_q90: stats::quantile(&maes, 0.1),
        low_n: maes.len() < MIN_SOLID,
    };

    Scorecard {
        label: label.into(),
        generated_at: Utc::now(),
        n_settled: ok.len(),
        n_failed,
        headlines,
        cohorts,
        decisions,
        gates,
        stop,
        reaction_fcr: stats::false_confirmation_rate(cfg.strategy.reaction_min_clusters.max(1), cfg.strategy.reaction_z, 0.3, 4000),
    }
}

/// Improvement backlog: patterns with enough evidence, as suggestions.
pub fn backlog(sc: &Scorecard) -> Vec<BacklogEntry> {
    let now = sc.generated_at;
    let mut out = Vec::new();
    for c in &sc.cohorts {
        if (c.dim == "event" || c.dim == "source") && c.n_clusters >= MIN_SUGGEST && c.p.is_some_and(|p| p > 0.1) {
            out.push(BacklogEntry {
                ts: now,
                pattern: format!("{} '{}' carries no measurable signal", c.dim, c.key),
                n: c.n_clusters,
                effect: format!("IC {:+.3}", c.ic.unwrap_or(0.0)),
                null_percentile: c.p.map(|p| (1.0 - p / 2.0) * 100.0),
                severity: "medium".into(),
                suggestion: format!("consider news.{}_weight.\"{}\" = 0 (via the review gate)", c.dim, c.key),
            });
        }
    }
    for g in &sc.gates {
        if g.n >= MIN_SUGGEST && g.missed * 2 > g.n && g.mean_ret > 0.0 {
            out.push(BacklogEntry {
                ts: now,
                pattern: format!("gate '{}' blocks more winners than losers", g.gate),
                n: g.n,
                effect: format!("{} missed / {} correctly blocked, mean blocked 24h return {:+.2}%", g.missed, g.correctly_blocked, g.mean_ret * 100.0),
                null_percentile: None,
                severity: "high".into(),
                suggestion: format!("review whether '{}' should be loosened; write a hypothesis first", g.gate),
            });
        }
    }
    if sc.stop.n_winners >= MIN_SUGGEST && sc.stop.share_below_stop.is_some_and(|s| s > 0.3) {
        out.push(BacklogEntry {
            ts: now,
            pattern: "fixed stop cuts off winning trades".into(),
            n: sc.stop.n_winners,
            effect: format!(
                "{:.0}% of winners went below −{:.1}% first; MAE q90 {:.1}%",
                sc.stop.share_below_stop.unwrap_or(0.0) * 100.0,
                sc.stop.stop_pct * 100.0,
                sc.stop.mae_q90.unwrap_or(0.0) * 100.0
            ),
            null_percentile: None,
            severity: "high".into(),
            suggestion: "test risk.stop = \"atr\" with stop_atr_mult set from the MAE q90 (hypothesis H-004)".into(),
        });
    }
    for r in &sc.headlines {
        if r.n_clusters >= MIN_SUGGEST && r.p.is_some_and(|p| p > 0.1) {
            out.push(BacklogEntry {
                ts: now,
                pattern: format!("headline IC at {}h is indistinguishable from noise", r.h),
                n: r.n_clusters,
                effect: format!("IC {:+.3}", r.ic.unwrap_or(0.0)),
                null_percentile: r.null_pct,
                severity: "low".into(),
                suggestion: "do not tune news thresholds for this horizon".into(),
            });
        }
    }
    out
}

fn f(x: Option<f64>, digits: usize) -> String {
    x.map(|v| format!("{v:+.digits$}")).unwrap_or_else(|| "—".into())
}

fn pctf(x: Option<f64>) -> String {
    x.map(|v| format!("{:.0}%", v * 100.0)).unwrap_or_else(|| "—".into())
}

fn grey(low: bool) -> &'static str {
    if low { " *(n<30)*" } else { "" }
}

fn verdict(p: Option<f64>) -> &'static str {
    match p {
        Some(p) if p <= 0.1 => "",
        Some(_) => " · indistinguishable from noise",
        None => " · too few clusters for a null test",
    }
}

#[derive(Debug, Deserialize)]
pub struct ReviewRecord {
    pub ts: DateTime<Utc>,
    pub verdict: String,
    pub candidate_hash: String,
    pub candidate: String,
}

pub fn markdown(
    sc: &Scorecard,
    backlog: &[BacklogEntry],
    hyps: &[Hypothesis],
    reviews: &[ReviewRecord],
    current_hash: &str,
    tokens: (u64, u64, u64),
) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# jev-trader report\n");
    let _ = writeln!(
        s,
        "*{} · generated {} · {} settled records ({} failed).* Statistics with n < 30 are marked; cohorts with n < 5 are hidden. Percentiles are within a block-permutation null ({} shuffles, blocks ≥ 24h).\n",
        sc.label,
        sc.generated_at.format("%Y-%m-%d %H:%M UTC"),
        sc.n_settled,
        sc.n_failed,
        NULL_ITERS
    );
    if let Some(r) = reviews.iter().rev().find(|r| r.candidate_hash == current_hash) {
        if r.verdict == "REJECT" {
            let _ = writeln!(s, "> **The current config was applied against a REJECT verdict** ({}, {}).\n", r.candidate, r.ts.format("%Y-%m-%d"));
        }
    }

    let _ = writeln!(s, "## Headlines: does sentiment predict the move?\n");
    let _ = writeln!(s, "IC = Spearman of sentiment × new_info against the vol-normalised forward return, after merging overlapping windows per asset.\n");
    let _ = writeln!(s, "| Horizon | n (clusters / raw) | IC | null pct | hit rate | hit null pct |\n|---|---|---|---|---|---|");
    for r in &sc.headlines {
        let _ = writeln!(
            s,
            "| {}h | {} / {}{} | {} | {}{} | {} | {} |",
            r.h,
            r.n_clusters,
            r.n_raw,
            grey(r.low_n),
            f(r.ic, 3),
            r.null_pct.map(|p| format!("{p:.0}")).unwrap_or_else(|| "—".into()),
            verdict(r.p),
            pctf(r.hit_rate),
            r.hit_null_pct.map(|p| format!("{p:.0}")).unwrap_or_else(|| "—".into()),
        );
    }
    if !sc.cohorts.is_empty() {
        let _ = writeln!(s, "\n### Breakdown (4h)\n\n| Cohort | n | IC | p | hit rate |\n|---|---|---|---|---|");
        for c in &sc.cohorts {
            let _ = writeln!(s, "| {} = {} | {}{} | {} | {} | {} |", c.dim, c.key, c.n_clusters, grey(c.low_n), f(c.ic, 3), c.p.map(|p| format!("{p:.2}")).unwrap_or_else(|| "—".into()), pctf(c.hit_rate));
        }
    }

    let d = &sc.decisions;
    let _ = writeln!(s, "\n## Decisions\n");
    let _ = writeln!(
        s,
        "{} decisions in {} independent clusters{} · hit rate {} · expectancy {} per decision (24h, direction-adjusted) · {} regime mismatches.\n",
        d.n,
        d.n_clusters,
        grey(d.low_n),
        pctf(d.hit_rate),
        d.expectancy.map(|e| format!("{:+.2}%", e * 100.0)).unwrap_or_else(|| "—".into()),
        d.regime_mismatch
    );
    if !d.tags.is_empty() {
        let _ = writeln!(s, "| Outcome (24h) | n |\n|---|---|");
        for (t, n) in &d.tags {
            let _ = writeln!(s, "| {t} | {n} |");
        }
    }

    let _ = writeln!(s, "\n## Gate values (blocked entries, settled as if taken)\n");
    if sc.gates.is_empty() {
        let _ = writeln!(s, "No blocked entries settled yet.");
    } else {
        let _ = writeln!(s, "| Gate | n | missed | correctly blocked | mean blocked 24h return |\n|---|---|---|---|---|");
        for g in &sc.gates {
            let _ = writeln!(s, "| {} | {}{} | {} | {} | {:+.2}% |", g.gate, g.n, grey(g.low_n), g.missed, g.correctly_blocked, g.mean_ret * 100.0);
        }
    }

    let st = &sc.stop;
    let _ = writeln!(s, "\n## Stop calibration\n");
    let _ = writeln!(
        s,
        "{} winning long entries{}. Share whose MAE first went below −{:.1}%: {}. MAE median {}, 90th percentile {}.",
        st.n_winners,
        grey(st.low_n),
        st.stop_pct * 100.0,
        pctf(st.share_below_stop),
        st.mae_q50.map(|m| format!("{:.2}%", m * 100.0)).unwrap_or_else(|| "—".into()),
        st.mae_q90.map(|m| format!("{:.2}%", m * 100.0)).unwrap_or_else(|| "—".into()),
    );
    let _ = writeln!(
        s,
        "\nNews-reaction gate on pure noise: false confirmation {:.1}% (i.i.d.), {:.1}% (AR(1) φ=0.3).",
        sc.reaction_fcr.0 * 100.0,
        sc.reaction_fcr.1 * 100.0
    );

    let _ = writeln!(s, "\n## Improvement backlog (suggestions only)\n");
    if backlog.is_empty() {
        let _ = writeln!(s, "Nothing has n ≥ {MIN_SUGGEST} and a clear effect yet.");
    }
    for b in backlog {
        let _ = writeln!(s, "- **[{}] {}** — n={}, {}{}. → {}", b.severity, b.pattern, b.n, b.effect, b.null_percentile.map(|p| format!(", null pct {p:.0}")).unwrap_or_default(), b.suggestion);
    }

    let _ = writeln!(s, "\n## Hypotheses\n");
    for status in ["testing", "proposed", "accepted", "rejected"] {
        let v: Vec<&Hypothesis> = hyps.iter().filter(|h| h.status == status).collect();
        if v.is_empty() {
            continue;
        }
        let _ = writeln!(s, "**{status}**\n");
        for h in v {
            let _ = writeln!(s, "- `{}` {} — change `{}`; pass if {}{}", h.id, h.statement, h.change, h.pass_if, if h.result.is_empty() { String::new() } else { format!(" · result: {}", h.result) });
        }
        let _ = writeln!(s);
    }
    let _ = writeln!(s, "## Jev usage today\n\n{} calls · {} input / {} output tokens.", tokens.0, tokens.1, tokens.2);
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settle::{HorizonResult, Record};

    fn settled(i: i64, x: f64, z: f64, kind: Kind) -> Settlement {
        let t0 = DateTime::from_timestamp(i * 7200, 0).unwrap();
        Settlement {
            rec: Record {
                id: format!("{i}"),
                kind,
                asset: "BTC".into(),
                t0,
                direction: if x > 0.0 { 1 } else { -1 },
                stop_pct: 0.04,
                event: Some("macro".into()),
                source: Some("s".into()),
                sentiment: Some(x),
                sentiment_conf: Some(1.0),
                new_info: Some(1.0),
                cluster_id: None,
                news_score: None,
                blocked_by: (kind == Kind::Counterfactual).then(|| "pause".into()),
                regime_at_signal: None,
                attempts: 0,
            },
            status: "SETTLED".into(),
            error: None,
            entry_time: Some(t0),
            entry_price: Some(100.0),
            vol: Some(0.01),
            horizons: [1u32, 4, 24]
                .iter()
                .map(|&h| HorizonResult { h, ret: z * 0.01, excess: None, z: Some(z), mfe: 0.01, mae: -0.05, stop_hit_h: None, tag: "WORKED".into(), label: (kind == Kind::Counterfactual).then(|| "MISSED_OPPORTUNITY".into()) })
                .collect(),
            regime_at_exit: None,
            regime_mismatch: false,
            settled_at: t0,
        }
    }

    #[test]
    fn clusters_overlap_and_reports() {
        let cfg = crate::decide::tests::cfg();
        let mut rng = stats::Rng::new(1);
        let v: Vec<Settlement> = (0..300)
            .map(|i| {
                let x = rng.normal();
                settled(i, x, x + 0.3 * rng.normal(), if i % 10 == 0 { Kind::Counterfactual } else { Kind::Headline })
            })
            .collect();
        let sc = scorecard(&v, &cfg, "test");
        let h1 = &sc.headlines[0];
        assert!(h1.n_clusters <= h1.n_raw);
        let h24 = &sc.headlines[2];
        assert!(h24.n_clusters < h1.n_clusters / 5, "24h windows on 2h spacing merge heavily");
        assert!(h1.ic.unwrap() > 0.5 && h1.null_pct.unwrap() > 95.0);
        assert_eq!(sc.gates[0].gate, "pause");
        assert!(sc.stop.share_below_stop.unwrap() > 0.9);
        let b = backlog(&sc);
        let md = markdown(&sc, &b, &[], &[], "", (0, 0, 0));
        assert!(md.contains("## Gate values"));
    }
}
