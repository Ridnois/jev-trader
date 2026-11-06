//! Phase 4: crypto market regime, 0–100, from six keyless components (adapted from
//! claude-trading-skills' crypto-regime-analyzer). Missing components have their
//! weight redistributed; fewer than 4 of 6 present → UNKNOWN (fails closed as a gate).
//! Computed at most every `refresh_minutes` and cached in `data/regime.json`.
//! Step 4.2: recorded and displayed only, unless `[regime] mode` is set.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::config::{Config, RegimeMode};
use crate::market::{self, Candle};
use crate::status::{Paths, read_json, write_atomic};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Zone {
    RiskOff,
    Neutral,
    RiskOn,
    Unknown,
}

impl Zone {
    pub fn from_score(s: Option<f64>) -> Self {
        match s {
            None => Zone::Unknown,
            Some(x) if x < 40.0 => Zone::RiskOff,
            Some(x) if x < 80.0 => Zone::Neutral,
            Some(_) => Zone::RiskOn,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Zone::RiskOff => "RISK_OFF",
            Zone::Neutral => "NEUTRAL",
            Zone::RiskOn => "RISK_ON",
            Zone::Unknown => "UNKNOWN",
        }
    }

    /// `ceiling` mode: share of max_gross allowed per zone. Heuristic defaults.
    pub fn ceiling_factor(self) -> f64 {
        match self {
            Zone::RiskOn => 1.0,
            Zone::Neutral => 0.66,
            Zone::RiskOff => 0.33,
            Zone::Unknown => 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    pub name: String,
    pub weight: f64,
    pub score: Option<f64>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Regime {
    pub ts: DateTime<Utc>,
    pub score: Option<f64>,
    pub zone: Zone,
    pub components: Vec<Component>,
    /// Phase 7 recorded columns: last funding rate and open interest per asset.
    #[serde(default)]
    pub funding: BTreeMap<String, f64>,
    #[serde(default)]
    pub open_interest: BTreeMap<String, f64>,
}

impl Regime {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn unknown(now: DateTime<Utc>) -> Self {
        Self { ts: now, score: None, zone: Zone::Unknown, components: Vec::new(), funding: BTreeMap::new(), open_interest: BTreeMap::new() }
    }

    /// Effective gross ceiling for this regime under `mode`.
    pub fn ceiling(&self, mode: RegimeMode, max_gross: f64) -> f64 {
        match mode {
            RegimeMode::Ceiling => max_gross * self.zone.ceiling_factor(),
            _ => max_gross,
        }
    }
}

/// Combines component scores; `None` when fewer than 4 are present.
pub fn combine(components: &[Component]) -> Option<f64> {
    let present: Vec<&Component> = components.iter().filter(|c| c.score.is_some()).collect();
    if present.len() < 4 {
        return None;
    }
    let wsum: f64 = present.iter().map(|c| c.weight).sum();
    Some(present.iter().map(|c| c.weight * c.score.unwrap_or(0.0)).sum::<f64>() / wsum)
}

fn sma(c: &[f64], n: usize) -> Option<f64> {
    (c.len() >= n && n > 0).then(|| c[c.len() - n..].iter().sum::<f64>() / n as f64)
}

/// BTC trend structure: close vs 50/200-day SMA plus the 200-day slope (20 days).
pub fn trend_score(closes: &[f64]) -> Option<(f64, String)> {
    let last = *closes.last()?;
    let s50 = sma(closes, 50)?;
    let s200 = sma(closes, 200)?;
    let s200_prev = sma(&closes[..closes.len().checked_sub(20)?], 200);
    let mut score: f64 = match (last > s50, last > s200) {
        (true, true) => 85.0,
        (false, true) => 60.0,
        (true, false) => 40.0,
        (false, false) => 10.0,
    };
    if let Some(p) = s200_prev {
        score += if s200 > p { 10.0 } else { -10.0 };
    }
    Some((score.clamp(0.0, 100.0), format!("close {last:.0} · SMA50 {s50:.0} · SMA200 {s200:.0}")))
}

/// Contrarian funding: crowded longs (high positive funding) score low.
/// Rate is per 8h as a fraction (0.0001 = 1 bp).
pub fn funding_score(rate: f64) -> f64 {
    let bp = rate * 10_000.0;
    // 1bp (normal) → 60; ≥5bp → 20; ≤−3bp → 85.
    if bp >= 1.0 {
        (60.0 - (bp - 1.0) * 10.0).max(20.0)
    } else {
        (60.0 + (1.0 - bp) * 6.25).min(85.0)
    }
}

/// Drawdown from the 1-year high and the 30d realized-vol percentile within the year.
pub fn drawdown_vol_score(closes: &[f64]) -> Option<(f64, String)> {
    if closes.len() < 120 {
        return None;
    }
    let year = &closes[closes.len().saturating_sub(365)..];
    let high = year.iter().cloned().fold(f64::MIN, f64::max);
    let last = *year.last()?;
    let dd = 1.0 - last / high;
    let rets: Vec<f64> = year.windows(2).map(|w| (w[1] / w[0]).ln()).collect();
    let vols: Vec<f64> = rets
        .windows(30)
        .map(|w| {
            let m = w.iter().sum::<f64>() / 30.0;
            (w.iter().map(|x| (x - m).powi(2)).sum::<f64>() / 29.0).sqrt()
        })
        .collect();
    let cur = *vols.last()?;
    let pct = vols.iter().filter(|v| **v <= cur).count() as f64 / vols.len() as f64 * 100.0;
    let dd_score = (1.0 - dd / 0.5).clamp(0.0, 1.0) * 100.0;
    Some((0.5 * dd_score + 0.5 * (100.0 - pct), format!("drawdown {:.0}% · vol pct {pct:.0}", dd * 100.0)))
}

const EXCLUDE: &[&str] = &[
    "USDC", "FDUSD", "TUSD", "USDP", "DAI", "EUR", "USDE", "BUSD", "PYUSD", "AEUR", "XUSD", "USD1",
    "WBTC", "WBETH", "STETH", "BFUSD", "EURI", "PAXG",
];

/// Top-`n` USDT pairs by quote volume, excluding stablecoins, wrapped and leveraged tokens.
pub fn universe(tickers: &[market::Ticker24h], n: usize) -> Vec<String> {
    let mut v: Vec<&market::Ticker24h> = tickers
        .iter()
        .filter(|t| {
            let Some(base) = t.symbol.strip_suffix("USDT") else { return false };
            !base.is_empty()
                && !EXCLUDE.contains(&base)
                && !base.ends_with("UP")
                && !base.ends_with("DOWN")
                && !base.ends_with("BULL")
                && !base.ends_with("BEAR")
        })
        .collect();
    v.sort_by(|a, b| b.quote_volume.total_cmp(&a.quote_volume));
    v.into_iter().take(n).map(|t| t.symbol.clone()).collect()
}

/// Daily-refreshed alt breadth and thrust.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DailyCache {
    day: Option<NaiveDate>,
    breadth: Option<f64>,
    thrust: Option<f64>,
    n: usize,
}

/// One BTC dominance observation per day.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Dominance {
    history: BTreeMap<NaiveDate, f64>,
}

/// Returns the cached regime if fresh, otherwise recomputes it (best effort).
pub async fn refresh(http: &reqwest::Client, cfg: &Config, paths: &Paths, now: DateTime<Utc>) -> Regime {
    if let Some(r) = read_json::<Regime>(&paths.regime)
        && (now - r.ts).num_minutes() < cfg.regime.refresh_minutes as i64
    {
        return r;
    }
    let r = compute(http, cfg, paths, now).await;
    match serde_json::to_vec_pretty(&r) {
        Ok(b) => {
            if let Err(e) = write_atomic(&paths.regime, &b) {
                warn!("could not write regime: {e:#}");
            }
        }
        Err(e) => warn!("could not encode regime: {e:#}"),
    }
    r
}

fn closes(c: &[Candle]) -> Vec<f64> {
    c.iter().map(|k| k.close).collect()
}

async fn compute(http: &reqwest::Client, cfg: &Config, paths: &Paths, now: DateTime<Utc>) -> Regime {
    let base = &cfg.market.base_url;
    let now_ms = now.timestamp_millis();
    let mut comps = Vec::new();

    // BTC daily: trend + drawdown/vol.
    let btc = market::klines(http, base, "BTCUSDT", "1d", 400)
        .await
        .map(|c| c.into_iter().filter(|k| k.is_closed(now_ms)).collect::<Vec<_>>());
    let btc_closes = btc.as_ref().map(|c| closes(c)).unwrap_or_default();
    let trend = trend_score(&btc_closes);
    comps.push(Component {
        name: "BTC trend".into(),
        weight: 25.0,
        score: trend.as_ref().map(|t| t.0),
        detail: trend.map(|t| t.1).unwrap_or_else(|| "unavailable".into()),
    });

    // Alt breadth + thrust, refreshed once per UTC day.
    let today = now.date_naive();
    let mut cache: DailyCache = read_json(&paths.regime_cache).unwrap_or_default();
    if cache.day != Some(today) {
        match breadth_thrust(http, base, now_ms).await {
            Ok((b, t, n)) => {
                cache = DailyCache { day: Some(today), breadth: Some(b), thrust: Some(t), n };
                if let Ok(bytes) = serde_json::to_vec_pretty(&cache) {
                    write_atomic(&paths.regime_cache, &bytes).ok();
                }
            }
            Err(e) => warn!("regime breadth failed: {e:#}"),
        }
    }
    let fresh_cache = cache.day.is_some_and(|d| (today - d).num_days() <= 1);
    comps.push(Component {
        name: "Alt breadth".into(),
        weight: 20.0,
        score: cache.breadth.filter(|_| fresh_cache).map(|b| b * 100.0),
        detail: format!("share of top {} above SMA200", cache.n),
    });

    // BTC dominance: needs 31 days of stored observations.
    let mut dom: Dominance = read_json(&paths.dominance).unwrap_or_default();
    if !dom.history.contains_key(&today) {
        match dominance(http, &cfg.regime.coingecko_url).await {
            Ok(d) => {
                dom.history.insert(today, d);
                if let Ok(bytes) = serde_json::to_vec_pretty(&dom) {
                    write_atomic(&paths.dominance, &bytes).ok();
                }
            }
            Err(e) => warn!("BTC dominance failed: {e:#}"),
        }
    }
    let dom_score = dominance_score(&dom.history, today);
    comps.push(Component {
        name: "BTC dominance".into(),
        weight: 15.0,
        score: dom_score.as_ref().map(|d| d.0),
        detail: dom_score
            .map(|d| d.1)
            .unwrap_or_else(|| format!("{} of 31 days stored", dom.history.len())),
    });

    // Funding, contrarian.
    let mut funding = BTreeMap::new();
    let mut oi = BTreeMap::new();
    for a in &cfg.assets {
        if let Ok(f) = market::funding_rate(http, &cfg.market.futures_url, &a.pair).await
            && f.is_finite()
        {
            funding.insert(a.symbol.clone(), f);
        }
        if let Ok(o) = market::open_interest(http, &cfg.market.futures_url, &a.pair).await
            && o.is_finite()
        {
            oi.insert(a.symbol.clone(), o);
        }
    }
    let avg_f = (!funding.is_empty()).then(|| funding.values().sum::<f64>() / funding.len() as f64);
    comps.push(Component {
        name: "Funding".into(),
        weight: 15.0,
        score: avg_f.map(funding_score),
        detail: avg_f.map(|f| format!("avg {:.2} bp/8h", f * 10_000.0)).unwrap_or_else(|| "unavailable".into()),
    });

    let ddv = drawdown_vol_score(&btc_closes);
    comps.push(Component {
        name: "Drawdown/vol".into(),
        weight: 15.0,
        score: ddv.as_ref().map(|d| d.0),
        detail: ddv.map(|d| d.1).unwrap_or_else(|| "unavailable".into()),
    });
    comps.push(Component {
        name: "Thrust".into(),
        weight: 10.0,
        score: cache.thrust.filter(|_| fresh_cache).map(|t| t * 100.0),
        detail: "share with positive 30d return".into(),
    });

    let score = combine(&comps);
    Regime { ts: now, score, zone: Zone::from_score(score), components: comps, funding, open_interest: oi }
}

fn dominance_score(h: &BTreeMap<NaiveDate, f64>, today: NaiveDate) -> Option<(f64, String)> {
    if h.len() < 31 {
        return None;
    }
    let (_, now) = h.range(..=today).next_back()?;
    let (_, ago) = h.range(..=today - chrono::Duration::days(30)).next_back()?;
    let change = now - ago; // percentage points
    // Rising dominance = money leaving alts = risk-off.
    Some(((50.0 - change * 10.0).clamp(0.0, 100.0), format!("{now:.1}% ({change:+.1}pp 30d)")))
}

async fn dominance(http: &reqwest::Client, url: &str) -> Result<f64> {
    let v: serde_json::Value =
        http.get(format!("{url}/api/v3/global")).send().await?.error_for_status()?.json().await?;
    v["data"]["market_cap_percentage"]["btc"].as_f64().context("no btc dominance")
}

async fn breadth_thrust(http: &reqwest::Client, base: &str, now_ms: i64) -> Result<(f64, f64, usize)> {
    let tickers = market::tickers_24h(http, base).await?;
    let uni = universe(&tickers, 20);
    let (mut above, mut pos, mut n) = (0, 0, 0);
    for pair in &uni {
        let Ok(c) = market::klines(http, base, pair, "1d", 202).await else { continue };
        let c: Vec<f64> = c.iter().filter(|k| k.is_closed(now_ms)).map(|k| k.close).collect();
        let (Some(s200), Some(last)) = (sma(&c, 200), c.last()) else { continue };
        n += 1;
        if *last > s200 {
            above += 1;
        }
        if c.len() > 30 && *last > c[c.len() - 31] {
            pos += 1;
        }
    }
    anyhow::ensure!(n >= 10, "only {n} pairs with 200 days of history");
    Ok((above as f64 / n as f64, pos as f64 / n as f64, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comp(w: f64, s: Option<f64>) -> Component {
        Component { name: "x".into(), weight: w, score: s, detail: String::new() }
    }

    #[test]
    fn redistributes_and_fails_closed() {
        let c = vec![comp(25.0, Some(100.0)), comp(20.0, Some(0.0)), comp(15.0, None), comp(15.0, Some(50.0)), comp(15.0, Some(50.0)), comp(10.0, None)];
        let s = combine(&c).unwrap();
        assert!((s - (2500.0 + 750.0 + 750.0) / 75.0).abs() < 1e-9);
        let few = vec![comp(25.0, Some(100.0)), comp(20.0, Some(0.0)), comp(15.0, Some(1.0))];
        assert!(combine(&few).is_none());
        assert_eq!(Zone::from_score(None), Zone::Unknown);
        assert_eq!(Zone::from_score(Some(85.0)), Zone::RiskOn);
    }

    #[test]
    fn funding_is_contrarian() {
        assert!(funding_score(0.0005) < funding_score(0.0001));
        assert!(funding_score(-0.0003) > funding_score(0.0001));
    }

    #[test]
    fn trend() {
        let up: Vec<f64> = (0..260).map(|i| 100.0 + i as f64).collect();
        assert!(trend_score(&up).unwrap().0 >= 90.0);
        let down: Vec<f64> = (0..260).map(|i| 400.0 - i as f64).collect();
        assert!(trend_score(&down).unwrap().0 <= 10.0);
    }

    #[test]
    fn universe_filters() {
        let t = |s: &str, v: f64| market::Ticker24h { symbol: s.into(), quote_volume: v };
        let u = universe(&[t("BTCUSDT", 9.0), t("USDCUSDT", 99.0), t("ETHBTC", 50.0), t("BNBUPUSDT", 9.0), t("SOLUSDT", 5.0)], 5);
        assert_eq!(u, vec!["BTCUSDT", "SOLUSDT"]);
    }
}
