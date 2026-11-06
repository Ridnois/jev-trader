//! Public Binance market data (no API key). `data-api.binance.vision` is the
//! market-data-only mirror; swap `base_url` for another Binance-compatible host if needed.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Candle {
    pub open_time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    /// Last millisecond of the candle (kline field 6).
    pub close_time: i64,
}

impl Candle {
    pub fn is_closed(&self, now_ms: i64) -> bool {
        self.close_time < now_ms
    }
}

/// "1m", "15m", "1h", "4h", "1d", "1w" → milliseconds.
pub fn interval_ms(interval: &str) -> Option<i64> {
    let (n, unit) = interval.split_at(interval.len().checked_sub(1)?);
    let n: i64 = n.parse().ok()?;
    let unit_ms = match unit {
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 7 * 86_400_000,
        _ => return None,
    };
    Some(n * unit_ms)
}

/// Market data for one asset at one tick.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Closed candles only, oldest first.
    pub closed: Vec<Candle>,
    /// Latest traded price (close of the in-progress candle), for execution.
    pub live_price: Option<f64>,
    /// Newest closed candle is older than 2× the interval.
    pub stale: bool,
}

/// Splits raw klines into closed candles and the in-progress candle's price.
pub fn split_closed(mut candles: Vec<Candle>, now_ms: i64, interval_ms: i64) -> Snapshot {
    let live_price = candles.last().map(|c| c.close);
    candles.retain(|c| c.is_closed(now_ms));
    let stale = candles.last().is_none_or(|c| now_ms - c.close_time > 2 * interval_ms);
    Snapshot { closed: candles, live_price, stale }
}

fn num(v: &Value) -> Result<f64> {
    match v {
        Value::String(s) => Ok(s.parse()?),
        Value::Number(n) => n.as_f64().context("bad number"),
        _ => bail!("unexpected kline field {v}"),
    }
}

fn parse_rows(rows: &[Vec<Value>]) -> Result<Vec<Candle>> {
    rows.iter()
        .map(|r| {
            if r.len() < 7 {
                bail!("short kline row");
            }
            Ok(Candle {
                open_time: r[0].as_i64().context("open_time")?,
                open: num(&r[1])?,
                high: num(&r[2])?,
                low: num(&r[3])?,
                close: num(&r[4])?,
                volume: num(&r[5])?,
                close_time: r[6].as_i64().context("close_time")?,
            })
        })
        .collect()
}

pub async fn klines(
    http: &reqwest::Client,
    base_url: &str,
    pair: &str,
    interval: &str,
    limit: usize,
) -> Result<Vec<Candle>> {
    let rows: Vec<Vec<Value>> = http
        .get(format!("{base_url}/api/v3/klines"))
        .query(&[("symbol", pair), ("interval", interval), ("limit", &limit.to_string())])
        .send()
        .await?
        .error_for_status()
        .with_context(|| format!("klines for {pair}"))?
        .json()
        .await?;
    parse_rows(&rows)
}

/// All candles opening in [start_ms, end_ms], paging 1000 at a time.
pub async fn klines_range(
    http: &reqwest::Client,
    base_url: &str,
    pair: &str,
    interval: &str,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<Candle>> {
    let mut out: Vec<Candle> = Vec::new();
    let mut from = start_ms;
    loop {
        let rows: Vec<Vec<Value>> = http
            .get(format!("{base_url}/api/v3/klines"))
            .query(&[
                ("symbol", pair),
                ("interval", interval),
                ("startTime", &from.to_string()),
                ("endTime", &end_ms.to_string()),
                ("limit", "1000"),
            ])
            .send()
            .await?
            .error_for_status()
            .with_context(|| format!("klines range for {pair}"))?
            .json()
            .await?;
        let batch = parse_rows(&rows)?;
        let n = batch.len();
        let Some(last) = batch.last() else { break };
        from = last.open_time + 1;
        out.extend(batch);
        if n < 1000 || from > end_ms {
            break;
        }
    }
    out.dedup_by_key(|c| c.open_time);
    Ok(out)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ticker24h {
    pub symbol: String,
    #[serde(deserialize_with = "de_f64")]
    pub quote_volume: f64,
}

fn de_f64<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<f64, D::Error> {
    let v = Value::deserialize(d)?;
    num(&v).map_err(serde::de::Error::custom)
}

pub async fn tickers_24h(http: &reqwest::Client, base_url: &str) -> Result<Vec<Ticker24h>> {
    Ok(http
        .get(format!("{base_url}/api/v3/ticker/24hr"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// Last funding rate (per 8h, as a fraction) from USDⓈ-M futures.
pub async fn funding_rate(http: &reqwest::Client, futures_url: &str, pair: &str) -> Result<f64> {
    let v: Value = http
        .get(format!("{futures_url}/fapi/v1/premiumIndex"))
        .query(&[("symbol", pair)])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    num(v.get("lastFundingRate").context("lastFundingRate")?)
}

/// Open interest in contracts (base asset units).
pub async fn open_interest(http: &reqwest::Client, futures_url: &str, pair: &str) -> Result<f64> {
    let v: Value = http
        .get(format!("{futures_url}/fapi/v1/openInterest"))
        .query(&[("symbol", pair)])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    num(v.get("openInterest").context("openInterest")?)
}

/// Average true range over the last `n` candles, as a fraction of the last close.
pub fn atr_pct(candles: &[Candle], n: usize) -> Option<f64> {
    if n == 0 || candles.len() < n + 1 {
        return None;
    }
    let tail = &candles[candles.len() - n - 1..];
    let tr: f64 = tail
        .windows(2)
        .map(|w| {
            let (prev, c) = (&w[0], &w[1]);
            (c.high - c.low).max((c.high - prev.close).abs()).max((c.low - prev.close).abs())
        })
        .sum::<f64>()
        / n as f64;
    let last = candles.last()?.close;
    (last > 0.0).then_some(tr / last)
}

/// Standard deviation of 1-candle log returns over the last `n` returns.
pub fn realized_vol(candles: &[Candle], n: usize) -> Option<f64> {
    if candles.len() < n + 1 || n < 2 {
        return None;
    }
    let tail = &candles[candles.len() - n - 1..];
    let r: Vec<f64> = tail.windows(2).map(|w| (w[1].close / w[0].close).ln()).collect();
    let m = r.iter().sum::<f64>() / r.len() as f64;
    let var = r.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (r.len() - 1) as f64;
    Some(var.sqrt())
}

#[cfg(test)]
pub fn test_candle(i: i64, close: f64) -> Candle {
    Candle {
        open_time: i * 3_600_000,
        open: close,
        high: close * 1.001,
        low: close * 0.999,
        close,
        volume: 1.0,
        close_time: (i + 1) * 3_600_000 - 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals() {
        assert_eq!(interval_ms("1h"), Some(3_600_000));
        assert_eq!(interval_ms("15m"), Some(900_000));
        assert_eq!(interval_ms("x"), None);
    }

    #[test]
    fn drops_open_candle_and_flags_stale() {
        let c: Vec<Candle> = (0..5).map(|i| test_candle(i, 100.0 + i as f64)).collect();
        // now is inside candle 4
        let s = split_closed(c.clone(), 4 * 3_600_000 + 10, 3_600_000);
        assert_eq!(s.closed.len(), 4);
        assert_eq!(s.live_price, Some(104.0));
        assert!(!s.stale);
        let s = split_closed(c, 9 * 3_600_000, 3_600_000);
        assert!(s.stale, "newest candle closed 4h before now");
    }

    #[test]
    fn atr_and_vol() {
        let c: Vec<Candle> = (0..20).map(|i| test_candle(i, 100.0)).collect();
        let a = atr_pct(&c, 14).unwrap();
        assert!((a - 0.002).abs() < 1e-9);
        assert!(realized_vol(&c, 10).unwrap() < 1e-12);
        assert!(atr_pct(&c[..5], 14).is_none());
    }
}
