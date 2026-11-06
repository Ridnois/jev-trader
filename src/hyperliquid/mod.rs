//! Hyperliquid: the only venue. Public market data (no key) lives here: perp
//! universe with volume, funding and open interest, candles for the news bot,
//! settlement, backtests and the regime, and `jev-trader hl scan`, which ranks
//! coins by how often a 10-minute move clears the round-trip cost at these fees.
//! Signed trading is in `exchange` (client) and `crate::live` (ledger glue).
//!
//! Why Hyperliquid: the deepest on-chain perp book at 1.5 bps maker / 4.5 bps taker
//! (tier 0), real shorts without borrowing, and funding / open interest in the same
//! feed. See CRYPTO-REDESIGN.md for the venue comparison.

pub mod exchange;
pub mod sign;

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::{EntryMode, ScalpConfig};
use crate::market::Candle;

/// Which Hyperliquid chain to talk to. Market data and orders always use the same one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Network {
    #[default]
    Mainnet,
    /// Fake money, thin books: for checking the order plumbing, not the strategy.
    Testnet,
}

impl Network {
    pub fn api_url(self) -> &'static str {
        match self {
            Network::Mainnet => "https://api.hyperliquid.xyz",
            Network::Testnet => "https://api.hyperliquid-testnet.xyz",
        }
    }

    pub fn ws_url(self) -> &'static str {
        match self {
            Network::Mainnet => "wss://api.hyperliquid.xyz/ws",
            Network::Testnet => "wss://api.hyperliquid-testnet.xyz/ws",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Network::Mainnet => "mainnet",
            Network::Testnet => "testnet",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PerpCtx {
    pub coin: String,
    pub day_volume_usd: f64,
    pub open_interest_usd: f64,
    /// Open interest in coins.
    pub open_interest: f64,
    /// Hourly funding rate as a fraction (positive = longs pay shorts).
    pub funding_1h: f64,
    pub mark_px: f64,
    /// Spread implied by the impact prices (cost of a typical-size market order), bps.
    pub impact_spread_bps: f64,
}

/// Numbers arrive as strings or JSON numbers; anything else is NaN.
pub fn num(v: &Value) -> f64 {
    match v {
        Value::String(s) => s.parse().unwrap_or(f64::NAN),
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

pub async fn info_at(http: &reqwest::Client, api: &str, body: Value) -> Result<Value> {
    let r = http.post(format!("{api}/info")).json(&body).send().await.context("hyperliquid info request")?;
    let status = r.status();
    if !status.is_success() {
        bail!("hyperliquid info {}: {}", status, r.text().await.unwrap_or_default());
    }
    Ok(r.json().await?)
}

/// coin → asset index, size decimals, max leverage (delisted perps excluded).
pub async fn perp_meta(http: &reqwest::Client, api: &str) -> Result<HashMap<String, exchange::AssetInfo>> {
    let v = info_at(http, api, json!({"type": "meta"})).await?;
    let uni = v["universe"].as_array().context("meta: no universe")?;
    Ok(uni
        .iter()
        .enumerate()
        .filter(|(_, a)| a["isDelisted"].as_bool() != Some(true))
        .map(|(i, a)| {
            (
                a["name"].as_str().unwrap_or_default().to_string(),
                exchange::AssetInfo {
                    index: i as u32,
                    sz_decimals: a["szDecimals"].as_u64().unwrap_or(0) as u32,
                    max_leverage: a["maxLeverage"].as_u64().unwrap_or(1) as u32,
                },
            )
        })
        .collect())
}

/// Every listed (not delisted) perp with its 24h context.
pub async fn perp_universe(http: &reqwest::Client, api: &str) -> Result<Vec<PerpCtx>> {
    let v = info_at(http, api, json!({"type": "metaAndAssetCtxs"})).await?;
    let (Some(meta), Some(ctxs)) = (v.get(0), v.get(1).and_then(Value::as_array)) else { bail!("unexpected metaAndAssetCtxs shape") };
    let uni = meta["universe"].as_array().context("no universe")?;
    let mut out = Vec::new();
    for (a, c) in uni.iter().zip(ctxs) {
        if a["isDelisted"].as_bool() == Some(true) {
            continue;
        }
        let mark = num(&c["markPx"]);
        let spread = match c["impactPxs"].as_array() {
            Some(p) if p.len() == 2 => {
                let (b, k) = (num(&p[0]), num(&p[1]));
                if b > 0.0 && k >= b { (k - b) / ((k + b) / 2.0) * 1e4 } else { f64::NAN }
            }
            _ => f64::NAN,
        };
        out.push(PerpCtx {
            coin: a["name"].as_str().unwrap_or_default().to_string(),
            day_volume_usd: num(&c["dayNtlVlm"]),
            open_interest_usd: num(&c["openInterest"]) * mark,
            open_interest: num(&c["openInterest"]),
            funding_1h: num(&c["funding"]),
            mark_px: mark,
            impact_spread_bps: spread,
        });
    }
    Ok(out)
}

fn parse_candles(v: &Value) -> Result<Vec<Candle>> {
    let arr = v.as_array().context("candles: not an array")?;
    arr.iter()
        .map(|c| {
            Ok(Candle {
                open_time: c["t"].as_i64().context("candle t")?,
                open: num(&c["o"]),
                high: num(&c["h"]),
                low: num(&c["l"]),
                close: num(&c["c"]),
                volume: num(&c["v"]),
                close_time: c["T"].as_i64().context("candle T")?,
            })
        })
        .collect()
}

/// Candles opening in [start_ms, end_ms], oldest first. Hyperliquid keeps only the
/// most recent 5000 candles per interval (about 208 days of 1h candles).
pub async fn candles(http: &reqwest::Client, api: &str, coin: &str, interval: &str, start_ms: i64, end_ms: i64) -> Result<Vec<Candle>> {
    let mut out: Vec<Candle> = Vec::new();
    let mut from = start_ms;
    for _ in 0..20 {
        let v = info_at(http, api, json!({"type": "candleSnapshot", "req": {"coin": coin, "interval": interval, "startTime": from, "endTime": end_ms}}))
            .await
            .with_context(|| format!("candles for {coin}"))?;
        let batch = parse_candles(&v)?;
        let Some(last) = batch.last() else { break };
        let next = last.open_time + 1;
        let n = batch.len();
        out.extend(batch);
        if n < 500 || next > end_ms || next <= from {
            break;
        }
        from = next;
    }
    out.sort_by_key(|c| c.open_time);
    out.dedup_by_key(|c| c.open_time);
    Ok(out)
}

/// 1-minute closes `(open_time_ms, close)` for the last `minutes` (max 5000).
pub async fn closes_1m(http: &reqwest::Client, api: &str, coin: &str, minutes: i64) -> Result<Vec<(i64, f64)>> {
    let end = chrono::Utc::now().timestamp_millis();
    let start = end - minutes.min(5000) * 60_000;
    let c = candles(http, api, coin, "1m", start, end).await?;
    Ok(c.into_iter().map(|k| (k.open_time, k.close)).filter(|(_, c)| c.is_finite() && *c > 0.0).collect())
}

/// Share of non-overlapping `window`-minute moves whose size is at least `hurdle_bps`,
/// and the median move size.
pub fn hurdle_stats(closes: &[(i64, f64)], window: usize, hurdle_bps: f64) -> (f64, f64, usize) {
    let mut moves: Vec<f64> = closes.chunks(window).zip(closes.chunks(window).skip(1)).map(|(a, b)| ((b[b.len() - 1].1 / a[a.len() - 1].1) - 1.0).abs() * 1e4).collect();
    if moves.is_empty() {
        return (0.0, 0.0, 0);
    }
    let hit = moves.iter().filter(|m| **m >= hurdle_bps).count() as f64 / moves.len() as f64;
    moves.sort_by(|a, b| a.total_cmp(b));
    (hit, moves[moves.len() / 2], moves.len())
}

/// Round-trip cost at the config's fees with this spread (no funding), bps.
pub fn trading_cost_bps(sc: &ScalpConfig, spread_bps: f64) -> f64 {
    match sc.entry {
        EntryMode::Taker => 2.0 * sc.fee_bps + 2.0 * sc.slippage_bps + spread_bps,
        EntryMode::Maker => sc.maker_fee_bps + sc.fee_bps + sc.slippage_bps + 0.5 * spread_bps,
    }
}

/// `jev-trader hl scan`: rank liquid perps by how often a 10-minute move clears
/// `cost_mult` × round-trip cost at this config's fees.
pub async fn scan(http: &reqwest::Client, api: &str, sc: &ScalpConfig, top: usize, min_volume_musd: f64, hours: i64) -> Result<String> {
    let mut uni = perp_universe(http, api).await?;
    uni.retain(|p| p.day_volume_usd >= min_volume_musd * 1e6 && p.mark_px > 0.0);
    uni.sort_by(|a, b| b.day_volume_usd.total_cmp(&a.day_volume_usd));
    uni.truncate(top);
    let window = 10usize;
    let mut rows = Vec::new();
    for p in &uni {
        let closes = match closes_1m(http, api, &p.coin, hours * 60).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{}: {e:#}", p.coin);
                continue;
            }
        };
        let spread = if p.impact_spread_bps.is_finite() { p.impact_spread_bps } else { 1.0 };
        let cost = trading_cost_bps(sc, spread);
        let (hit, median, n) = hurdle_stats(&closes, window, sc.cost_mult * cost);
        rows.push((hit, p.clone(), cost, median, n));
        tokio::time::sleep(std::time::Duration::from_millis(120)).await; // stay far under the rate limit
    }
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut out = format!(
        "Hyperliquid perps, last {hours}h of 1m candles, {window}-minute windows. Hurdle = {:.1} × round-trip cost ({} entry: maker {:.1} / taker {:.1} bps, slippage {:.1}).\n\n",
        sc.cost_mult,
        if sc.entry == EntryMode::Maker { "maker" } else { "taker" },
        sc.maker_fee_bps,
        sc.fee_bps,
        sc.slippage_bps
    );
    out += &format!("{:<8} {:>9} {:>8} {:>10} {:>7} {:>6} {:>9} {:>8} {:>5}\n", "coin", "vol24 $M", "OI $M", "fund bp/h", "spread", "cost", "median10m", "clears", "n");
    for (hit, p, cost, median, n) in &rows {
        out += &format!(
            "{:<8} {:>9.1} {:>8.1} {:>+10.3} {:>7.2} {:>6.1} {:>9.1} {:>7.0}% {:>5}\n",
            p.coin,
            p.day_volume_usd / 1e6,
            p.open_interest_usd / 1e6,
            p.funding_1h * 1e4,
            p.impact_spread_bps,
            cost,
            median,
            hit * 100.0,
            n
        );
    }
    out += "\n`clears` = share of 10-minute moves at least the hurdle at these fees.\nIt measures room for an edge, not an edge. Put a universe change in hypotheses.toml before trading it.\n";
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hurdle_counts_non_overlapping_windows() {
        // 30 closes: flat for 10, +1% for 10, back for 10 → moves of +100 and ≈ −99 bps.
        let mut c: Vec<(i64, f64)> = (0..10).map(|i| (i, 100.0)).collect();
        c.extend((10..20).map(|i| (i, 101.0)));
        c.extend((20..30).map(|i| (i, 100.0)));
        let (hit, median, n) = hurdle_stats(&c, 10, 50.0);
        assert_eq!(n, 2);
        assert!((hit - 1.0).abs() < 1e-12);
        assert!(median > 98.0);
        assert_eq!(hurdle_stats(&c, 10, 150.0).0, 0.0);
    }

    #[test]
    fn maker_entry_is_cheaper_than_taker() {
        let mk = ScalpConfig { entry: EntryMode::Maker, maker_fee_bps: 1.5, fee_bps: 4.5, slippage_bps: 1.0, ..Default::default() };
        let tk = ScalpConfig { entry: EntryMode::Taker, ..mk.clone() };
        assert!((trading_cost_bps(&mk, 1.0) - 7.5).abs() < 1e-12);
        assert!((trading_cost_bps(&tk, 1.0) - 12.0).abs() < 1e-12);
    }

    #[test]
    fn parses_candle_snapshot() {
        let v: Value = serde_json::from_str(r#"[{"t":1793991600000,"T":1793995199999,"s":"BTC","i":"1h","o":"75785.0","c":"75881.0","h":"75881.0","l":"75684.0","v":"412.49868","n":7085}]"#).unwrap();
        let c = parse_candles(&v).unwrap();
        assert_eq!(c[0].close_time, 1793995199999);
        assert_eq!(c[0].close, 75881.0);
    }
}
