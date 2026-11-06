//! 0.5: number validation. Every input that can move money is checked here;
//! a failure means REVIEW (no entries) for that asset, or HALTED for the account.

use anyhow::{Result, bail};

use crate::market::Candle;
use crate::news::{Challenge, NewsEvent};

fn unit(name: &str, x: f64) -> Result<()> {
    if !x.is_finite() || !(0.0..=1.0 + 1e-9).contains(&x) {
        bail!("{name} = {x} is not in 0..1");
    }
    Ok(())
}

pub fn news_event(e: &NewsEvent) -> Result<()> {
    unit("asset_conf", e.asset_conf)?;
    unit("event_conf", e.event_conf)?;
    unit("sentiment_conf", e.sentiment_conf)?;
    unit("new_info", e.new_info)?;
    if !e.sentiment.is_finite() || e.sentiment.abs() > 1.0 + 1e-9 {
        bail!("sentiment = {} is not in -1..1", e.sentiment);
    }
    Ok(())
}

pub fn challenge(c: &Challenge) -> Result<()> {
    unit("priced_in", c.priced_in)?;
    unit("unverified", c.unverified)?;
    unit("bear", c.bear)
}

pub fn candles(c: &[Candle]) -> Result<()> {
    for (i, k) in c.iter().enumerate() {
        let ok = [k.open, k.high, k.low, k.close].iter().all(|v| v.is_finite() && *v > 0.0)
            && k.volume.is_finite()
            && k.volume >= 0.0
            && k.high >= k.low
            && k.close_time > k.open_time;
        if !ok {
            bail!("candle {i} (open_time {}) is invalid", k.open_time);
        }
        if i > 0 && k.open_time <= c[i - 1].open_time {
            bail!("candles out of order at {i}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::test_candle;

    #[test]
    fn candle_checks() {
        let mut c: Vec<Candle> = (0..3).map(|i| test_candle(i, 10.0)).collect();
        assert!(candles(&c).is_ok());
        c[1].close = f64::NAN;
        assert!(candles(&c).is_err());
        c[1].close = 10.0;
        c[1].high = 1.0;
        assert!(candles(&c).is_err());
    }
}
