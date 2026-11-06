//! Candles and the indicators computed from them. Data comes from Hyperliquid's
//! public info API (`candleSnapshot`, no key needed).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Candle {
    pub open_time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    /// Last millisecond of the candle (Hyperliquid `T`).
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

/// The last `limit` candles of `interval` (the newest may still be open).
pub async fn klines(http: &reqwest::Client, api: &str, coin: &str, interval: &str, limit: usize) -> Result<Vec<Candle>> {
    let ims = interval_ms(interval).with_context(|| format!("bad interval {interval}"))?;
    let end = chrono::Utc::now().timestamp_millis();
    let start = end - ims * limit as i64;
    let mut c = crate::hyperliquid::candles(http, api, coin, interval, start, end).await?;
    if c.len() > limit {
        c.drain(..c.len() - limit);
    }
    Ok(c)
}

/// All candles opening in [start_ms, end_ms].
pub async fn klines_range(http: &reqwest::Client, api: &str, coin: &str, interval: &str, start_ms: i64, end_ms: i64) -> Result<Vec<Candle>> {
    crate::hyperliquid::candles(http, api, coin, interval, start_ms, end_ms).await
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
