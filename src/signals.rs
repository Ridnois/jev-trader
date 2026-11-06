//! Turns Jev news features + price data into a target portfolio weight.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use crate::config::{NewsConfig, StrategyConfig};
use crate::market::Candle;
use crate::news::NewsEvent;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct NewsSignal {
    /// Decay-weighted sentiment squashed into -1..1.
    pub score: f64,
    /// Number of events contributing.
    pub n_events: usize,
    /// High-confidence, fresh hack/exploit news about this asset.
    pub hack_alert: bool,
    /// 5.4 stance counts over independent clusters (same story counted once).
    #[serde(default)]
    pub n_clusters: usize,
    #[serde(default)]
    pub pos: usize,
    #[serde(default)]
    pub neg: usize,
    #[serde(default)]
    pub neutral: usize,
}

/// How headlines are combined into a score.
#[derive(Debug, Clone, Default)]
pub struct SignalOpts<'a> {
    pub half_life_hours: f64,
    /// 5.1: one influence per same-story cluster (false = sum over outlets, the original).
    pub dedupe_clusters: bool,
    pub event_weight: Option<&'a BTreeMap<String, f64>>,
    pub source_weight: Option<&'a BTreeMap<String, f64>>,
}

impl<'a> SignalOpts<'a> {
    pub fn from_config(n: &'a NewsConfig) -> Self {
        Self {
            half_life_hours: n.half_life_hours,
            dedupe_clusters: n.dedupe_clusters,
            event_weight: Some(&n.event_weight),
            source_weight: Some(&n.source_weight),
        }
    }
}

/// Extra outlets on the same story add this much confidence each, up to `CLUSTER_BONUS_MAX`.
const CLUSTER_BONUS: f64 = 0.1;
const CLUSTER_BONUS_MAX: f64 = 0.3;
/// |sentiment| below this counts as neutral in stance counts.
const NEUTRAL_BAND: f64 = 0.15;

#[cfg_attr(not(test), allow(dead_code))]
pub fn news_signal(
    events: &[NewsEvent],
    symbol: &str,
    now: DateTime<Utc>,
    half_life_hours: f64,
) -> NewsSignal {
    let opts = SignalOpts { half_life_hours, ..Default::default() };
    news_signal_with_drivers(events, symbol, now, &opts, 0).0
}

/// Also returns the ids of the `top` events with the largest absolute influence
/// (hack alerts first), so decisions can be traced back to headlines.
/// Only events visible at `now` (seen_at ≤ now) count, so replays stay point-in-time.
pub fn news_signal_with_drivers(
    events: &[NewsEvent],
    symbol: &str,
    now: DateTime<Utc>,
    opts: &SignalOpts,
    top: usize,
) -> (NewsSignal, Vec<String>) {
    let mut n = 0;
    let mut hack_alert = false;
    let mut contrib: Vec<(f64, &str)> = Vec::new();
    // cluster → (members, Σ signed weight, strongest signed weight)
    let mut clusters: BTreeMap<&str, (usize, f64, f64)> = BTreeMap::new();
    for e in events {
        if e.seen_at() > now {
            continue;
        }
        let relevance = if e.asset == symbol {
            e.asset_conf
        } else if e.asset == "market" {
            0.5 * e.asset_conf
        } else {
            continue;
        };
        let age_h = (now - e.published).num_seconds().max(0) as f64 / 3600.0;
        if e.asset == symbol && e.event == "hack" && e.event_conf > 0.8 && e.new_info > 0.7 && age_h < 24.0 {
            hack_alert = true;
            contrib.push((f64::INFINITY, &e.id));
        }
        if e.event == "noise" {
            continue;
        }
        let ew = opts.event_weight.and_then(|m| m.get(&e.event)).copied().unwrap_or(1.0);
        let sw = opts.source_weight.and_then(|m| m.get(&e.source)).copied().unwrap_or(1.0);
        let decay = 0.5f64.powf(age_h / opts.half_life_hours);
        let w = relevance * e.sentiment_conf * e.new_info * decay * ew * sw;
        let signed = w * e.sentiment;
        n += 1;
        contrib.push((signed.abs(), &e.id));
        let c = clusters.entry(e.cluster()).or_insert((0, 0.0, 0.0));
        c.0 += 1;
        c.1 += signed;
        if signed.abs() > c.2.abs() {
            c.2 = signed;
        }
    }
    let sum: f64 = if opts.dedupe_clusters {
        clusters
            .values()
            .map(|(k, _, strongest)| strongest * (1.0 + (CLUSTER_BONUS * (*k as f64 - 1.0)).min(CLUSTER_BONUS_MAX)))
            .sum()
    } else {
        clusters.values().map(|(_, s, _)| s).sum()
    };
    let (mut pos, mut neg, mut neutral) = (0, 0, 0);
    for (_, s, strongest) in clusters.values() {
        let v = if opts.dedupe_clusters { *strongest } else { *s };
        // Compare against the decayed weight scale: a cluster is "neutral" if its
        // net contribution is small relative to a fresh, confident headline.
        if v > NEUTRAL_BAND * 0.25 {
            pos += 1;
        } else if v < -NEUTRAL_BAND * 0.25 {
            neg += 1;
        } else {
            neutral += 1;
        }
    }
    contrib.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut drivers: Vec<String> = Vec::new();
    for (w, id) in contrib {
        if drivers.len() >= top || w < 1e-3 {
            break;
        }
        if !drivers.iter().any(|d| d == id) {
            drivers.push(id.to_string());
        }
    }
    (
        NewsSignal { score: sum.tanh(), n_events: n, hack_alert, n_clusters: clusters.len(), pos, neg, neutral },
        drivers,
    )
}

/// Last close relative to its simple moving average.
pub fn momentum(candles: &[Candle], window: usize) -> Option<f64> {
    if candles.len() < window || window == 0 {
        return None;
    }
    let tail = &candles[candles.len() - window..];
    let sma = tail.iter().map(|c| c.close).sum::<f64>() / window as f64;
    Some(candles.last()?.close / sma - 1.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Target {
    /// Keep whatever we hold (hysteresis zone).
    Hold,
    /// Move to this fraction of equity (before risk caps).
    Weight(f64),
}

/// Long-only spot logic: news must be positive AND price must confirm.
pub fn target(news: &NewsSignal, momentum: f64, max_weight: f64, cfg: &StrategyConfig) -> Target {
    if news.hack_alert || news.score < -cfg.news_exit {
        return Target::Weight(0.0);
    }
    if news.score > cfg.news_entry && momentum > 0.0 && (cfg.min_clusters <= 1 || news.pos >= cfg.min_clusters) {
        return Target::Weight(max_weight * news.score.min(1.0));
    }
    if momentum < 0.0 && news.score <= 0.0 {
        return Target::Weight(0.0);
    }
    Target::Hold
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn ev(asset: &str, event: &str, sentiment: f64, age_h: i64, now: DateTime<Utc>) -> NewsEvent {
        NewsEvent {
            id: "x".into(),
            title: "t".into(),
            source: "s".into(),
            published: now - Duration::hours(age_h),
            analyzed_at: now,
            asset: asset.into(),
            asset_conf: 1.0,
            event: event.into(),
            event_conf: 0.95,
            sentiment,
            sentiment_conf: 1.0,
            new_info: 0.9,
            model: "jev".into(),
            first_seen_at: None,
            cluster_id: None,
            tokens_in: None,
            tokens_out: None,
        }
    }

    #[test]
    fn decays_and_filters() {
        let now = Utc::now();
        let fresh = news_signal(&[ev("BTC", "macro", 1.0, 0, now)], "BTC", now, 2.0);
        let old = news_signal(&[ev("BTC", "macro", 1.0, 4, now)], "BTC", now, 2.0);
        assert!(fresh.score > old.score && old.score > 0.0);
        let other = news_signal(&[ev("ETH", "macro", 1.0, 0, now)], "BTC", now, 2.0);
        assert_eq!(other.n_events, 0);
        let noise = news_signal(&[ev("BTC", "noise", 1.0, 0, now)], "BTC", now, 2.0);
        assert_eq!(noise.score, 0.0);
    }

    #[test]
    fn hack_forces_exit() {
        let now = Utc::now();
        let s = news_signal(
            &[ev("SOL", "hack", -1.0, 1, now), ev("SOL", "macro", 1.0, 0, now)],
            "SOL",
            now,
            2.0,
        );
        assert!(s.hack_alert);
        let cfg = cfg();
        assert_eq!(target(&s, 0.05, 0.2, &cfg), Target::Weight(0.0));
    }

    #[test]
    fn clusters_count_once_when_deduped() {
        let now = Utc::now();
        let mut a = ev("BTC", "macro", 1.0, 0, now);
        let mut b = ev("BTC", "macro", 1.0, 0, now);
        let mut c = ev("BTC", "macro", 1.0, 0, now);
        a.id = "a".into();
        b.id = "b".into();
        c.id = "c".into();
        b.cluster_id = Some("a".into());
        c.cluster_id = Some("a".into());
        let evs = [a, b, c];
        let sum = news_signal(&evs, "BTC", now, 2.0);
        let opts = SignalOpts { half_life_hours: 2.0, dedupe_clusters: true, ..Default::default() };
        let (dd, _) = news_signal_with_drivers(&evs, "BTC", now, &opts, 0);
        assert!(dd.score < sum.score);
        assert_eq!((dd.n_clusters, dd.pos), (1, 1));
        let cfg = StrategyConfig { min_clusters: 2, ..cfg() };
        assert_eq!(target(&dd, 0.05, 0.2, &cfg), Target::Hold, "needs 2 independent clusters");
    }

    #[test]
    fn invisible_until_seen() {
        let now = Utc::now();
        let mut e = ev("BTC", "macro", 1.0, 0, now);
        e.first_seen_at = Some(now + Duration::minutes(5));
        assert_eq!(news_signal(&[e], "BTC", now, 2.0).n_events, 0);
    }

    #[test]
    fn event_weights_apply() {
        let now = Utc::now();
        let w: BTreeMap<String, f64> = [("macro".to_string(), 0.0)].into();
        let opts = SignalOpts { half_life_hours: 2.0, event_weight: Some(&w), ..Default::default() };
        let (s, _) = news_signal_with_drivers(&[ev("BTC", "macro", 1.0, 0, now)], "BTC", now, &opts, 0);
        assert_eq!(s.score, 0.0);
    }

    fn cfg() -> StrategyConfig {
        StrategyConfig { news_entry: 0.3, news_exit: 0.3, min_clusters: 1, reaction_gate: false, reaction_min_clusters: 3, reaction_z: -1.5 }
    }
}
