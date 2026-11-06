//! Phase 1: settle every headline, decision and blocked entry once its horizons
//! have passed, on closed 1h candles. Records wait in `data/settle_pending.json`
//! and are written exactly once to `data/settlements.jsonl`. Plain statistics, no LLM.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::bot::append_jsonl;
use crate::config::Config;
use crate::decide::AssetDecision;
use crate::market::{self, Candle};
use crate::news::NewsEvent;
use crate::paper::State;
use crate::signals::Target;
use crate::status::{Paths, read_json, write_atomic};

const HOUR_MS: i64 = 3_600_000;
/// Candles of history before entry used for realized vol.
const VOL_LOOKBACK: usize = 24;
const MAX_ATTEMPTS: u32 = 5;
/// |sentiment| below this is a neutral headline (no direction).
const NEUTRAL_SENTIMENT: f64 = 0.15;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Headline,
    Decision,
    /// An entry a gate blocked: settled as if it had been taken.
    Counterfactual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub kind: Kind,
    pub asset: String,
    /// Signal time: `first_seen_at` for headlines (never `published`), tick time for decisions.
    pub t0: DateTime<Utc>,
    /// +1 expects up, −1 expects down (exit), 0 neutral.
    pub direction: i8,
    pub stop_pct: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentiment: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentiment_conf: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_info: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub news_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regime_at_signal: Option<String>,
    #[serde(default)]
    pub attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HorizonResult {
    pub h: u32,
    pub ret: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excess: Option<f64>,
    /// Forward return ÷ (trailing realized vol × √h).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z: Option<f64>,
    pub mfe: f64,
    pub mae: f64,
    /// Hours until the stop would have been hit (long only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_hit_h: Option<u32>,
    pub tag: String,
    /// Counterfactuals: MISSED_OPPORTUNITY or CORRECTLY_BLOCKED.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settlement {
    #[serde(flatten)]
    pub rec: Record,
    /// SETTLED or FAILED.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_time: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vol: Option<f64>,
    #[serde(default)]
    pub horizons: Vec<HorizonResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regime_at_exit: Option<String>,
    /// A failure whose regime changed between signal and exit.
    #[serde(default)]
    pub regime_mismatch: bool,
    pub settled_at: DateTime<Utc>,
}

impl Settlement {
    pub fn horizon(&self, h: u32) -> Option<&HorizonResult> {
        self.horizons.iter().find(|x| x.h == h)
    }
}

pub enum Outcome {
    Pending,
    Settled(Box<Settlement>),
    Failed(String),
}

fn is_failure(tag: &str) -> bool {
    tag.starts_with("FAILED")
}

/// Outcome tag in units of realized vol, from the trade's own direction.
pub fn tag(direction: i8, ret: f64, mfe: f64, mae: f64, stop: bool, s: f64) -> &'static str {
    if direction == 0 {
        return "NEUTRAL";
    }
    let (r, fav, adv) = if direction > 0 { (ret, mfe, mae) } else { (-ret, -mae, -mfe) };
    if stop && direction > 0 {
        "FAILED_STOP"
    } else if r > 0.5 * s {
        "WORKED"
    } else if fav > s && r < 0.0 {
        "FAILED_FADE"
    } else if fav > 0.5 * s && adv < -0.5 * s {
        "CHOPPY"
    } else if r < -0.5 * s {
        "FAILED"
    } else {
        "NEUTRAL"
    }
}

/// Settles one record against closed 1h candles. Entry is the close of the first
/// candle closing at or after `t0`; nothing dated later than `t0` decides the entry.
pub fn settle_one(
    rec: &Record,
    candles: &[Candle],
    bench: Option<&[Candle]>,
    horizons: &[u32],
    now: DateTime<Utc>,
    regime_now: Option<&str>,
) -> Outcome {
    let now_ms = now.timestamp_millis();
    let t0 = rec.t0.timestamp_millis();
    let max_h = horizons.iter().copied().max().unwrap_or(24) as usize;
    let Some(idx) = candles.iter().position(|c| c.close_time >= t0) else {
        return if now_ms - t0 > 2 * HOUR_MS { Outcome::Failed("no candle after t0".into()) } else { Outcome::Pending };
    };
    if candles[idx].open_time > t0 + HOUR_MS {
        return Outcome::Failed("gap in candles at t0".into());
    }
    let Some(last) = candles.get(idx + max_h) else { return Outcome::Pending };
    if last.close_time >= now_ms {
        return Outcome::Pending;
    }
    let entry = candles[idx].close;
    let vol = market::realized_vol(&candles[..=idx], VOL_LOOKBACK).filter(|v| *v > 0.0);
    let bench_at = |open_time: i64| bench.and_then(|b| b.iter().find(|c| c.open_time == open_time)).map(|c| c.close);
    let bench_entry = bench_at(candles[idx].open_time);

    let mut out = Vec::new();
    for &h in horizons {
        let h_us = h as usize;
        let win = &candles[idx + 1..=idx + h_us];
        let exit = candles[idx + h_us].close;
        let ret = exit / entry - 1.0;
        let mfe = win.iter().map(|c| c.high).fold(f64::MIN, f64::max) / entry - 1.0;
        let mae = win.iter().map(|c| c.low).fold(f64::MAX, f64::min) / entry - 1.0;
        let stop_hit_h = win.iter().position(|c| c.low <= entry * (1.0 - rec.stop_pct)).map(|k| k as u32 + 1);
        let s = vol.unwrap_or(0.01) * (h as f64).sqrt();
        let excess = match (bench_entry, bench_at(candles[idx + h_us].open_time)) {
            (Some(b0), Some(b1)) => Some(ret - (b1 / b0 - 1.0)),
            _ => None,
        };
        let t = tag(rec.direction, ret, mfe, mae, stop_hit_h.is_some(), s);
        let label = (rec.kind == Kind::Counterfactual)
            .then(|| if t == "WORKED" { "MISSED_OPPORTUNITY" } else { "CORRECTLY_BLOCKED" }.to_string());
        out.push(HorizonResult {
            h,
            ret,
            excess,
            z: vol.map(|v| ret / (v * (h as f64).sqrt())),
            mfe,
            mae,
            stop_hit_h,
            tag: t.into(),
            label,
        });
    }
    let failed = out.iter().any(|r| is_failure(&r.tag));
    let mismatch = failed
        && matches!((rec.regime_at_signal.as_deref(), regime_now), (Some(a), Some(b)) if a != b && a != "UNKNOWN" && b != "UNKNOWN");
    Outcome::Settled(Box::new(Settlement {
        rec: rec.clone(),
        status: "SETTLED".into(),
        error: None,
        entry_time: chrono::DateTime::from_timestamp_millis(candles[idx].close_time + 1),
        entry_price: Some(entry),
        vol,
        horizons: out,
        regime_at_exit: regime_now.map(str::to_string),
        regime_mismatch: mismatch,
        settled_at: now,
    }))
}

// ---------- creating records ----------

pub fn headline_record(e: &NewsEvent, cfg: &Config, regime: Option<&str>) -> Option<Record> {
    let asset = if e.asset == "market" {
        cfg.settle.benchmark.clone()
    } else if cfg.assets.iter().any(|a| a.symbol == e.asset) {
        e.asset.clone()
    } else {
        return None;
    };
    let direction = if e.sentiment > NEUTRAL_SENTIMENT {
        1
    } else if e.sentiment < -NEUTRAL_SENTIMENT {
        -1
    } else {
        0
    };
    Some(Record {
        id: format!("h:{}", e.id),
        kind: Kind::Headline,
        asset,
        t0: e.seen_at(),
        direction,
        stop_pct: cfg.risk.stop_loss_pct,
        event: Some(e.event.clone()),
        source: Some(e.source.clone()),
        sentiment: Some(e.sentiment),
        sentiment_conf: Some(e.sentiment_conf),
        new_info: Some(e.new_info),
        cluster_id: Some(e.cluster().to_string()),
        news_score: None,
        blocked_by: None,
        regime_at_signal: regime.map(str::to_string),
        attempts: 0,
    })
}

/// A record for each change of signal direction: the decision, or the counterfactual
/// when a gate blocked the entry. Updates `state.last_dir`.
pub fn decision_records(
    state: &mut State,
    decisions: &[AssetDecision],
    cfg: &Config,
    now: DateTime<Utc>,
    regime: Option<&str>,
) -> Vec<Record> {
    let mut out = Vec::new();
    for d in decisions {
        let dir: i8 = match d.target {
            Target::Weight(x) if x > 0.0 => 1,
            Target::Weight(_) => 0,
            Target::Hold => continue,
        };
        let prev = state.last_dir.insert(d.symbol.clone(), dir);
        if prev == Some(dir) {
            continue;
        }
        let (kind, direction) = if dir == 1 {
            if d.blocked_by.is_some() && d.weight <= d.current + 1e-9 {
                (Kind::Counterfactual, 1)
            } else {
                (Kind::Decision, 1)
            }
        } else {
            (Kind::Decision, -1)
        };
        out.push(Record {
            id: format!("{}:{}:{}", if kind == Kind::Counterfactual { "c" } else { "d" }, d.symbol, now.timestamp()),
            kind,
            asset: d.symbol.clone(),
            t0: now,
            direction,
            stop_pct: d.stop_dist.unwrap_or(cfg.risk.stop_loss_pct),
            event: None,
            source: None,
            sentiment: None,
            sentiment_conf: None,
            new_info: None,
            cluster_id: None,
            news_score: Some(d.news.score),
            blocked_by: if kind == Kind::Counterfactual { d.blocked_by.clone() } else { None },
            regime_at_signal: regime.map(str::to_string),
            attempts: 0,
        });
    }
    out
}

// ---------- persistence and the per-tick runner ----------

pub fn load_pending(paths: &Paths) -> Vec<Record> {
    read_json(&paths.settle_pending).unwrap_or_default()
}

fn save_pending(paths: &Paths, p: &[Record]) -> Result<()> {
    write_atomic(&paths.settle_pending, &serde_json::to_vec(p)?)
}

pub fn enqueue(paths: &Paths, recs: Vec<Record>) {
    if recs.is_empty() {
        return;
    }
    let mut p = load_pending(paths);
    let have: std::collections::HashSet<String> = p.iter().map(|r| r.id.clone()).collect();
    p.extend(recs.into_iter().filter(|r| !have.contains(&r.id)));
    if let Err(e) = save_pending(paths, &p) {
        warn!("could not save pending settlements: {e:#}");
    }
}

#[derive(Debug, Default)]
pub struct RunSummary {
    pub settled: usize,
    pub failed: usize,
    pub pending: usize,
    pub errors: Vec<String>,
}

/// Settles every due record. A fetch failure leaves records pending (retried next
/// tick) and never blocks trading.
pub async fn run(
    http: &reqwest::Client,
    cfg: &Config,
    paths: &Paths,
    now: DateTime<Utc>,
    regime_now: Option<&str>,
) -> RunSummary {
    let mut sum = RunSummary::default();
    let pending = load_pending(paths);
    let max_h = cfg.settle.horizons.iter().copied().max().unwrap_or(24) as i64;
    let due_at = |r: &Record| r.t0 + Duration::hours(max_h + 1);
    let mut due: BTreeMap<String, Vec<Record>> = BTreeMap::new();
    let mut keep = Vec::new();
    for r in pending {
        if now >= due_at(&r) {
            due.entry(r.asset.clone()).or_default().push(r);
        } else {
            keep.push(r);
        }
    }
    if due.is_empty() {
        sum.pending = keep.len();
        return sum;
    }
    let mut cache: HashMap<String, Vec<Candle>> = HashMap::new();
    let earliest = due.values().flatten().map(|r| r.t0).min().unwrap_or(now);
    let start = (earliest - Duration::hours(VOL_LOOKBACK as i64 + 2)).timestamp_millis();
    let mut fetch_assets: Vec<String> = due.keys().cloned().collect();
    if !fetch_assets.contains(&cfg.settle.benchmark) {
        fetch_assets.push(cfg.settle.benchmark.clone());
    }
    for sym in fetch_assets {
        let Some(a) = cfg.assets.iter().find(|a| a.symbol == sym) else { continue };
        match market::klines_range(http, &cfg.market.base_url, &a.pair, "1h", start, now.timestamp_millis()).await {
            Ok(c) => {
                let c: Vec<Candle> = c.into_iter().filter(|k| k.is_closed(now.timestamp_millis())).collect();
                if crate::validate::candles(&c).is_ok() {
                    cache.insert(sym, c);
                } else {
                    sum.errors.push(format!("{sym}: invalid candles"));
                }
            }
            Err(e) => sum.errors.push(format!("{sym}: {e:#}")),
        }
    }
    let bench = cache.get(&cfg.settle.benchmark).cloned();
    for (sym, recs) in due {
        let Some(candles) = cache.get(&sym) else {
            keep.extend(recs);
            continue;
        };
        for mut r in recs {
            let b = if sym == cfg.settle.benchmark { None } else { bench.as_deref() };
            match settle_one(&r, candles, b, &cfg.settle.horizons, now, regime_now) {
                Outcome::Settled(s) => {
                    append_jsonl(&paths.settlements, &*s);
                    sum.settled += 1;
                }
                Outcome::Pending => keep.push(r),
                Outcome::Failed(e) => {
                    r.attempts += 1;
                    if r.attempts >= MAX_ATTEMPTS {
                        append_jsonl(
                            &paths.settlements,
                            &Settlement {
                                rec: r,
                                status: "FAILED".into(),
                                error: Some(e),
                                entry_time: None,
                                entry_price: None,
                                vol: None,
                                horizons: Vec::new(),
                                regime_at_exit: regime_now.map(str::to_string),
                                regime_mismatch: false,
                                settled_at: now,
                            },
                        );
                        sum.failed += 1;
                    } else {
                        keep.push(r);
                    }
                }
            }
        }
    }
    sum.pending = keep.len();
    if let Err(e) = save_pending(paths, &keep) {
        sum.errors.push(format!("save: {e:#}"));
    }
    sum
}

/// 5.3: per asset, a drift z-score of how price reacted (4h, vol-normalised) to
/// strongly positive headline clusters, overlapping windows merged. Under the null
/// each cluster's z is ~N(0,1), so Σz/√n is ~N(0,1).
pub fn reaction_drift(settled: &[Settlement], horizon: u32, last_n: usize) -> BTreeMap<String, (f64, usize)> {
    let mut by_asset: BTreeMap<String, Vec<(DateTime<Utc>, f64)>> = BTreeMap::new();
    for s in settled {
        let r = &s.rec;
        if r.kind != Kind::Headline || s.status != "SETTLED" || r.event.as_deref() == Some("noise") {
            continue;
        }
        if r.sentiment.unwrap_or(0.0) < 0.3 || r.new_info.unwrap_or(0.0) < 0.5 {
            continue;
        }
        if let Some(z) = s.horizon(horizon).and_then(|h| h.z) {
            by_asset.entry(r.asset.clone()).or_default().push((r.t0, z));
        }
    }
    by_asset
        .into_iter()
        .map(|(a, mut v)| {
            v.sort_by_key(|x| x.0);
            let clusters = crate::stats::merge_windows(&v, Duration::hours(horizon as i64));
            let tail = &clusters[clusters.len().saturating_sub(last_n)..];
            let n = tail.len();
            let z = if n == 0 { 0.0 } else { tail.iter().sum::<f64>() / (n as f64).sqrt() };
            (a, (z, n))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::test_candle;

    fn rec(kind: Kind, dir: i8, t0_h: i64) -> Record {
        Record {
            id: "x".into(),
            kind,
            asset: "SOL".into(),
            t0: DateTime::from_timestamp_millis(t0_h * HOUR_MS + 60_000).unwrap(),
            direction: dir,
            stop_pct: 0.04,
            event: None,
            source: None,
            sentiment: None,
            sentiment_conf: None,
            new_info: None,
            cluster_id: None,
            news_score: None,
            blocked_by: None,
            regime_at_signal: Some("NEUTRAL".into()),
            attempts: 0,
        }
    }

    #[test]
    fn pending_until_last_candle_closes_then_settles() {
        // Price drifts up 0.5% per hour, with a bit of noise for vol.
        let c: Vec<Candle> = (0..80)
            .map(|i| test_candle(i, 100.0 * 1.005f64.powi(i as i32) * if i % 2 == 0 { 1.001 } else { 0.999 }))
            .collect();
        let r = rec(Kind::Decision, 1, 30);
        let early = DateTime::from_timestamp_millis(40 * HOUR_MS).unwrap();
        assert!(matches!(settle_one(&r, &c, None, &[1, 4, 24], early, None), Outcome::Pending));
        let later = DateTime::from_timestamp_millis(80 * HOUR_MS).unwrap();
        let Outcome::Settled(s) = settle_one(&r, &c, None, &[1, 4, 24], later, Some("NEUTRAL")) else { panic!() };
        // Entry is the close of the candle containing t0 (closes after t0), not earlier.
        assert_eq!(s.entry_price, Some(c[30].close));
        let h24 = s.horizon(24).unwrap();
        assert!(h24.ret > 0.1 && h24.tag == "WORKED" && h24.stop_hit_h.is_none());
        assert!(h24.z.unwrap() > 0.0);
    }

    #[test]
    fn counterfactual_labels() {
        let c: Vec<Candle> = (0..80).map(|i| test_candle(i, 100.0 * 0.99f64.powi(i as i32))).collect();
        let r = rec(Kind::Counterfactual, 1, 30);
        let later = DateTime::from_timestamp_millis(80 * HOUR_MS).unwrap();
        let Outcome::Settled(s) = settle_one(&r, &c, None, &[1, 4, 24], later, Some("RISK_OFF")) else { panic!() };
        let h24 = s.horizon(24).unwrap();
        assert_eq!(h24.tag, "FAILED_STOP");
        assert_eq!(h24.label.as_deref(), Some("CORRECTLY_BLOCKED"));
        assert!(s.regime_mismatch);
    }

    #[test]
    fn tags() {
        assert_eq!(tag(1, 0.03, 0.04, -0.01, false, 0.02), "WORKED");
        assert_eq!(tag(1, -0.01, 0.03, -0.015, false, 0.02), "FAILED_FADE");
        assert_eq!(tag(-1, -0.03, 0.0, -0.04, false, 0.02), "WORKED");
        assert_eq!(tag(0, 0.5, 0.5, 0.0, false, 0.02), "NEUTRAL");
    }
}
