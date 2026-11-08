//! `jev-trader report`: is the engine making money, and is the news reading
//! worth its tokens? Reads the engine's logs only; never edits a config.
//!
//! 1. Trades by kind (catalyst / flow): n, win rate, net $ and bps after fees.
//! 2. Headline markouts: does Jev's signed impact predict the move at 5/15/60/240
//!    min after costs, for every catalyst-grade headline (taken or not)? By source
//!    and by kind (listing vs news).
//! 3. Flow markouts: do flow candidates that agree with the news beat those that
//!    don't? (Answers whether the news filter is what makes flow trades pay.)
//! 4. Jev spend: calls, tokens, tokens per headline, by purpose.
//!
//! Means carry 95% intervals with errors clustered by UTC hour (nearby samples are
//! not independent). Figures with fewer than `MIN_N` independent hours are marked thin.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::status::{Files, read_jsonl};

pub const MIN_N: usize = 20;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Est {
    pub n: usize,
    /// Independent clusters (UTC hours).
    pub clusters: usize,
    pub mean: Option<f64>,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
}

impl Est {
    pub fn thin(&self) -> bool {
        self.clusters < MIN_N
    }

    /// "pays", "loses", "unclear", or "too little data".
    pub fn verdict(&self) -> &'static str {
        match (self.thin(), self.lo, self.hi) {
            (true, _, _) => "too little data",
            (_, Some(lo), _) if lo > 0.0 => "pays",
            (_, _, Some(hi)) if hi < 0.0 => "loses",
            _ => "unclear",
        }
    }
}

/// Mean with an hour-clustered 95% interval. `v` = (unix hour, value).
pub fn est(v: &[(i64, f64)]) -> Est {
    let n = v.len();
    if n == 0 {
        return Est::default();
    }
    let mean = v.iter().map(|x| x.1).sum::<f64>() / n as f64;
    let mut by: BTreeMap<i64, f64> = BTreeMap::new();
    for (h, x) in v {
        *by.entry(*h).or_default() += x - mean;
    }
    let g = by.len();
    if g < 2 {
        return Est { n, clusters: g, mean: Some(mean), lo: None, hi: None };
    }
    let var = by.values().map(|s| s * s).sum::<f64>() / (n as f64 * n as f64) * g as f64 / (g as f64 - 1.0);
    let t = t975(g - 1);
    let se = var.sqrt();
    Est { n, clusters: g, mean: Some(mean), lo: Some(mean - t * se), hi: Some(mean + t * se) }
}

fn t975(df: usize) -> f64 {
    const T: [f64; 10] = [12.71, 4.30, 3.18, 2.78, 2.57, 2.45, 2.36, 2.31, 2.26, 2.23];
    match df {
        0 => f64::NAN,
        1..=10 => T[df - 1],
        11..=20 => 2.13,
        21..=40 => 2.04,
        _ => 1.98,
    }
}

/// Spearman rank correlation.
pub fn spearman(x: &[f64], y: &[f64]) -> Option<f64> {
    if x.len() != y.len() || x.len() < 5 {
        return None;
    }
    let rank = |v: &[f64]| {
        let mut idx: Vec<usize> = (0..v.len()).collect();
        idx.sort_by(|a, b| v[*a].total_cmp(&v[*b]));
        let mut r = vec![0.0; v.len()];
        let mut i = 0;
        while i < idx.len() {
            let mut j = i;
            while j + 1 < idx.len() && v[idx[j + 1]] == v[idx[i]] {
                j += 1;
            }
            for k in i..=j {
                r[idx[k]] = (i + j) as f64 / 2.0;
            }
            i = j + 1;
        }
        r
    };
    let (rx, ry) = (rank(x), rank(y));
    let n = rx.len() as f64;
    let (mx, my) = (rx.iter().sum::<f64>() / n, ry.iter().sum::<f64>() / n);
    let cov: f64 = rx.iter().zip(&ry).map(|(a, b)| (a - mx) * (b - my)).sum();
    let vx: f64 = rx.iter().map(|a| (a - mx).powi(2)).sum();
    let vy: f64 = ry.iter().map(|b| (b - my).powi(2)).sum();
    (vx > 0.0 && vy > 0.0).then(|| cov / (vx * vy).sqrt())
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Row {
    pub label: String,
    pub est: Est,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Scorecard {
    pub trades: Vec<Row>,
    pub trade_usd: BTreeMap<String, f64>,
    pub win_rate: BTreeMap<String, f64>,
    /// Strong headlines (catalyst-grade), net bps per horizon in the impact's direction.
    pub events: Vec<Row>,
    pub events_ic: BTreeMap<i64, Option<f64>>,
    pub events_by_source: Vec<Row>,
    pub events_by_kind: Vec<Row>,
    pub flow: Vec<Row>,
    pub jev: Value,
    pub summary: Vec<String>,
}

fn hour_of(v: &Value, key: &str) -> i64 {
    v[key].as_str().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map(|d| d.timestamp() / 3600).unwrap_or(0)
}

pub fn scorecard(dir: &Path, min_impact: f64) -> Scorecard {
    let f = Files::new(dir);
    let trades: Vec<Value> = read_jsonl(&f.trades);
    let marks: Vec<Value> = read_jsonl(&f.markouts);
    let usage: Vec<Value> = read_jsonl(&f.jev_usage);
    let news: Vec<Value> = read_jsonl(&f.news);
    let first_seen: BTreeMap<String, i64> = news.iter().filter_map(|e| Some((e["id"].as_str()?.to_string(), hour_of(e, "first_seen")))).collect();
    let mut sc = Scorecard::default();

    // 1. Trades.
    for kind in ["catalyst", "flow", "all"] {
        let rows: Vec<&Value> = trades.iter().filter(|t| kind == "all" || t["kind"] == kind).collect();
        let v: Vec<(i64, f64)> = rows.iter().map(|t| (hour_of(t, "opened"), t["pnl_bps"].as_f64().unwrap_or(0.0))).collect();
        sc.trades.push(Row { label: kind.into(), est: est(&v) });
        sc.trade_usd.insert(kind.into(), rows.iter().map(|t| t["pnl_usd"].as_f64().unwrap_or(0.0)).sum());
        if !rows.is_empty() {
            sc.win_rate.insert(kind.into(), rows.iter().filter(|t| t["pnl_usd"].as_f64().unwrap_or(0.0) > 0.0).count() as f64 / rows.len() as f64);
        }
    }

    // 2. Headlines.
    let ev: Vec<&Value> = marks.iter().filter(|m| m["type"] == "event").collect();
    let hour = |m: &Value| first_seen.get(m["id"].as_str().unwrap_or("")).copied().unwrap_or(0);
    for h in [5, 15, 60, 240] {
        let at: Vec<&&Value> = ev.iter().filter(|m| m["h_min"] == h).collect();
        let strong: Vec<(i64, f64)> = at.iter().filter(|m| m["impact"].as_f64().unwrap_or(0.0).abs() >= min_impact).map(|m| (hour(m), m["net_bps"].as_f64().unwrap_or(0.0))).collect();
        sc.events.push(Row { label: format!("{h} min"), est: est(&strong) });
        // IC: signed strength vs the raw move (unsigned by side).
        let x: Vec<f64> = at.iter().map(|m| m["impact"].as_f64().unwrap_or(0.0) * m["new_info"].as_f64().unwrap_or(0.0)).collect();
        let y: Vec<f64> = at.iter().map(|m| if m["side"] == "short" { -1.0 } else { 1.0 } * m["ret_bps"].as_f64().unwrap_or(0.0)).collect();
        sc.events_ic.insert(h, spearman(&x, &y));
    }
    let strong60: Vec<&&Value> = ev.iter().filter(|m| m["h_min"] == 60 && m["impact"].as_f64().unwrap_or(0.0).abs() >= min_impact).collect();
    for (key, out) in [("source", &mut sc.events_by_source), ("kind", &mut sc.events_by_kind)] {
        let mut g: BTreeMap<String, Vec<(i64, f64)>> = BTreeMap::new();
        for m in &strong60 {
            g.entry(m[key].as_str().unwrap_or("?").to_string()).or_default().push((hour(m), m["net_bps"].as_f64().unwrap_or(0.0)));
        }
        let mut rows: Vec<Row> = g.into_iter().map(|(k, v)| Row { label: k, est: est(&v) }).collect();
        rows.sort_by(|a, b| b.est.n.cmp(&a.est.n));
        *out = rows;
    }

    // 3. Flow: aligned with news vs not, at each horizon.
    let fl: Vec<&Value> = marks.iter().filter(|m| m["type"] == "flow").collect();
    let mut hs: Vec<i64> = fl.iter().filter_map(|m| m["h_secs"].as_i64()).collect();
    hs.sort();
    hs.dedup();
    for h in hs {
        for (label, aligned) in [("agrees with news", true), ("no news support", false)] {
            let v: Vec<(i64, f64)> = fl
                .iter()
                .filter(|m| m["h_secs"] == h && m["aligned"].as_bool() == Some(aligned))
                .map(|m| (m["id"].as_str().and_then(|s| s.rsplit('-').next()).and_then(|t| t.parse::<i64>().ok()).unwrap_or(0) / 3_600_000, m["net_bps"].as_f64().unwrap_or(0.0)))
                .collect();
            sc.flow.push(Row { label: format!("{}s · {label}", h), est: est(&v) });
        }
    }

    // 4. Jev spend.
    let mut by: BTreeMap<String, (u64, u64, u64, u64)> = BTreeMap::new();
    for u in &usage {
        let e = by.entry(u["purpose"].as_str().unwrap_or("?").to_string()).or_default();
        e.0 += 1;
        e.1 += u["input"].as_u64().unwrap_or(0) + u["output"].as_u64().unwrap_or(0);
        e.2 += u["items"].as_u64().unwrap_or(0);
        e.3 += u64::from(u["ok"].as_bool() != Some(true));
    }
    let days = {
        let ts: Vec<i64> = usage.iter().map(|u| hour_of(u, "ts")).filter(|h| *h > 0).collect();
        match (ts.iter().min(), ts.iter().max()) {
            (Some(a), Some(b)) => ((b - a) as f64 / 24.0).max(1.0 / 24.0),
            _ => 0.0,
        }
    };
    let total: u64 = by.values().map(|v| v.1).sum();
    sc.jev = serde_json::json!({
        "by_purpose": by.iter().map(|(k, v)| (k.clone(), serde_json::json!({ "calls": v.0, "tokens": v.1, "items": v.2, "failed": v.3, "tokens_per_item": if v.2 > 0 { Some(v.1 as f64 / v.2 as f64) } else { None } }))).collect::<BTreeMap<_, _>>(),
        "total_tokens": total,
        "days": days,
        "tokens_per_day": if days > 0.0 { Some(total as f64 / days) } else { None },
        "v1_tokens_per_headline": 751,
    });

    // Plain-language summary.
    let all = &sc.trades[2].est;
    sc.summary.push(match all.verdict() {
        "too little data" => format!("Trading result: {} trades over {} hours, too few to judge (need {MIN_N}+ independent hours).", all.n, all.clusters),
        v => format!("Trading result: {} trades, {:+.1} bps per trade after fees ({}).", all.n, all.mean.unwrap_or(0.0), v),
    });
    if let Some(r) = sc.events.iter().find(|r| r.label == "60 min") {
        sc.summary.push(match r.est.verdict() {
            "too little data" => format!("Headline edge: {} strong headlines marked out, too few to judge.", r.est.n),
            v => format!("Headline edge: strong headlines move {:+.1} bps net in Jev's direction after 60 min ({v}).", r.est.mean.unwrap_or(0.0)),
        });
    }
    let fa = sc.flow.iter().rev().find(|r| r.label.contains("agrees"));
    let fb = sc.flow.iter().rev().find(|r| r.label.contains("no news"));
    if let (Some(a), Some(b)) = (fa, fb) {
        sc.summary.push(format!(
            "News filter on flow: candidates that agree with the news {:+.1} bps vs {:+.1} bps without support at the longest horizon (n {} vs {}).",
            a.est.mean.unwrap_or(f64::NAN),
            b.est.mean.unwrap_or(f64::NAN),
            a.est.n,
            b.est.n
        ));
    }
    sc
}

fn fmt_est(e: &Est) -> String {
    let m = e.mean.map(|m| format!("{m:+.1}")).unwrap_or_else(|| "–".into());
    let ci = match (e.lo, e.hi) {
        (Some(lo), Some(hi)) => format!("[{lo:+.1}, {hi:+.1}]"),
        _ => "–".into(),
    };
    format!("{m} | {ci} | {} | {} | {}", e.n, e.clusters, e.verdict())
}

pub fn markdown(sc: &Scorecard) -> String {
    let mut s = String::from("# jev-trader v2 report\n\n");
    for l in &sc.summary {
        s += &format!("- {l}\n");
    }
    s += "\n## Trades (bps per trade after fees)\n\n| kind | mean | 95% CI | n | hours | verdict | net $ | win rate |\n|---|---|---|---|---|---|---|---|\n";
    for r in &sc.trades {
        s += &format!("| {} | {} | {:+.2} | {} |\n", r.label, fmt_est(&r.est), sc.trade_usd.get(&r.label).unwrap_or(&0.0), sc.win_rate.get(&r.label).map(|w| format!("{:.0}%", w * 100.0)).unwrap_or_else(|| "–".into()));
    }
    s += "\n## Strong headlines: net bps in Jev's direction, taken or not\n\n| horizon | mean | 95% CI | n | hours | verdict | IC (all headlines) |\n|---|---|---|---|---|---|---|\n";
    for r in &sc.events {
        let h: i64 = r.label.split(' ').next().and_then(|x| x.parse().ok()).unwrap_or(0);
        s += &format!("| {} | {} | {} |\n", r.label, fmt_est(&r.est), sc.events_ic.get(&h).copied().flatten().map(|x| format!("{x:+.2}")).unwrap_or_else(|| "–".into()));
    }
    for (title, rows) in [("By source (60 min)", &sc.events_by_source), ("By kind (60 min)", &sc.events_by_kind)] {
        s += &format!("\n### {title}\n\n| | mean | 95% CI | n | hours | verdict |\n|---|---|---|---|---|---|\n");
        for r in rows {
            s += &format!("| {} | {} |\n", r.label, fmt_est(&r.est));
        }
    }
    s += "\n## Flow candidates: does agreeing with the news help? (net bps)\n\n| horizon · group | mean | 95% CI | n | hours | verdict |\n|---|---|---|---|---|---|\n";
    for r in &sc.flow {
        s += &format!("| {} | {} |\n", r.label, fmt_est(&r.est));
    }
    s += &format!("\n## Jev spend\n\n```json\n{}\n```\n", serde_json::to_string_pretty(&sc.jev).unwrap_or_default());
    s += "\nIntervals cluster by UTC hour. \"too little data\" = fewer than 20 independent hours. Suggestions only; nothing here changes a config.\n";
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clustered_interval_and_verdicts() {
        let v: Vec<(i64, f64)> = (0..40).map(|i| (i, 10.0 + (i % 3) as f64)).collect();
        let e = est(&v);
        assert_eq!(e.clusters, 40);
        assert_eq!(e.verdict(), "pays");
        let neg: Vec<(i64, f64)> = (0..40).map(|i| (i, -5.0 + (i % 2) as f64)).collect();
        assert_eq!(est(&neg).verdict(), "loses");
        let thin: Vec<(i64, f64)> = (0..100).map(|i| (i / 50, 5.0)).collect();
        assert_eq!(est(&thin).verdict(), "too little data", "100 samples in 2 hours are 2 observations");
    }

    #[test]
    fn spearman_basics() {
        let x = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert!((spearman(&x, &[2.0, 4.0, 6.0, 8.0, 10.0]).unwrap() - 1.0).abs() < 1e-12);
        assert!((spearman(&x, &[5.0, 4.0, 3.0, 2.0, 1.0]).unwrap() + 1.0).abs() < 1e-12);
        assert!(spearman(&x[..3], &x[..3]).is_none());
    }
}
