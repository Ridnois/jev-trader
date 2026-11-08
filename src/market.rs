//! Candles from Hyperliquid's public info API (`candleSnapshot`, no key) and the
//! few numbers computed from them: ATR for stop sizing, closes for markouts.

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

/// "1m", "15m", "1h", "4h", "1d" → milliseconds.
pub fn interval_ms(interval: &str) -> Option<i64> {
    let (n, unit) = interval.split_at(interval.len().checked_sub(1)?);
    let n: i64 = n.parse().ok()?;
    let unit_ms = match unit {
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    Some(n * unit_ms)
}

/// The last `limit` candles of `interval` (the newest may still be open).
pub async fn klines(http: &reqwest::Client, api: &str, coin: &str, interval: &str, limit: usize) -> Result<Vec<Candle>> {
    let ims = interval_ms(interval).with_context(|| format!("bad interval {interval}"))?;
    let end = chrono::Utc::now().timestamp_millis();
    let mut c = crate::hyperliquid::candles(http, api, coin, interval, end - ims * limit as i64, end).await?;
    if c.len() > limit {
        c.drain(..c.len() - limit);
    }
    Ok(c)
}

/// Average true range over the last `n` closed candles, as a fraction of the last close.
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
    (last > 0.0 && tr.is_finite()).then_some(tr / last)
}

/// Close of the 1m candle covering `ts_ms` (or the last one before it).
pub fn close_at(candles: &[Candle], ts_ms: i64) -> Option<f64> {
    candles.iter().rev().find(|c| c.open_time <= ts_ms).map(|c| c.close).filter(|c| c.is_finite() && *c > 0.0)
}

#[cfg(test)]
pub fn test_candle(i: i64, close: f64) -> Candle {
    Candle { open_time: i * 3_600_000, open: close, high: close * 1.001, low: close * 0.999, close, volume: 1.0, close_time: (i + 1) * 3_600_000 - 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_atr_and_lookup() {
        assert_eq!(interval_ms("15m"), Some(900_000));
        assert_eq!(interval_ms("x"), None);
        let c: Vec<Candle> = (0..20).map(|i| test_candle(i, 100.0 + i as f64)).collect();
        let a = atr_pct(&c, 14).unwrap();
        assert!(a > 0.009 && a < 0.012, "{a}");
        assert!(atr_pct(&c[..5], 14).is_none());
        assert_eq!(close_at(&c, 3 * 3_600_000 + 5), Some(103.0));
        assert_eq!(close_at(&c, -1), None);
    }
}
