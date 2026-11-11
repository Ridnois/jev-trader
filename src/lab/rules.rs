//! Money rules shared by the simulator and the live engine: what a fill costs, how
//! big a position is, where its stop sits. Kept in one place so a backtest and the
//! paper run it predicts cannot drift apart.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Trading costs, per side, in bps of notional. Defaults are Hyperliquid perps tier 0
/// (taker 4.5 bps) with a 2 bps spread and 1 bps of slippage, as the scalper assumes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Costs {
    pub fee_bps: f64,
    pub slippage_bps: f64,
    /// Full top-of-book spread; half is paid on each side.
    pub spread_bps: f64,
    /// Per-coin spread overrides.
    pub spread: BTreeMap<String, f64>,
    /// Extra slippage when a stop fills (fast market).
    pub stop_slippage_bps: f64,
    /// Whether funding is charged from the archive.
    pub funding: bool,
    /// Stress multiplier on fee, spread and slippage.
    pub mult: f64,
}

impl Default for Costs {
    fn default() -> Self {
        Self { fee_bps: 4.5, slippage_bps: 1.0, spread_bps: 2.0, spread: BTreeMap::new(), stop_slippage_bps: 5.0, funding: true, mult: 1.0 }
    }
}

impl Costs {
    /// Price concession on one side (half spread + slippage), bps.
    pub fn side_bps(&self, coin: &str) -> f64 {
        (self.spread.get(coin).copied().unwrap_or(self.spread_bps) / 2.0 + self.slippage_bps) * self.mult
    }

    /// Fee on one side as a fraction of notional.
    pub fn fee(&self) -> f64 {
        self.fee_bps * self.mult / 1e4
    }

    pub fn stop_extra_bps(&self) -> f64 {
        self.stop_slippage_bps * self.mult
    }

    /// Fees, spread and slippage for a full round trip, bps (no funding).
    pub fn round_trip_bps(&self, coin: &str) -> f64 {
        2.0 * (self.fee_bps * self.mult + self.side_bps(coin))
    }

    pub fn scaled(&self, mult: f64) -> Costs {
        Costs { mult: self.mult * mult, ..self.clone() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SizingMode {
    /// The same share of capital in every trade.
    Fixed,
    /// The same expected daily swing in every trade: size ∝ 1 / the coin's volatility.
    #[default]
    Vol,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Sizing {
    pub mode: SizingMode,
    /// `fixed`: share of capital per trade.
    pub pos_pct: f64,
    /// `vol`: a one-standard-deviation day in the coin moves the account this much.
    pub risk_pct: f64,
    /// Largest single position, share of capital.
    pub max_pct: f64,
    /// Total open notional allowed, share of capital (1.0 = no leverage).
    pub max_gross: f64,
    pub min_usd: f64,
}

impl Default for Sizing {
    fn default() -> Self {
        Self { mode: SizingMode::Vol, pos_pct: 0.10, risk_pct: 0.004, max_pct: 0.15, max_gross: 1.0, min_usd: 10.0 }
    }
}

/// Dollars for a new position, or 0 when it can't be opened. `daily_vol` is the
/// coin's daily return std (fraction); `gross_open` the notional already open.
pub fn size_usd(s: &Sizing, capital: f64, daily_vol: f64, gross_open: f64) -> f64 {
    let want = match s.mode {
        SizingMode::Fixed => capital * s.pos_pct,
        SizingMode::Vol => {
            if !(daily_vol.is_finite() && daily_vol > 0.0) {
                return 0.0;
            }
            (capital * s.risk_pct / daily_vol).min(capital * s.max_pct)
        }
    };
    let room = (capital * s.max_gross - gross_open).max(0.0);
    let usd = want.min(room);
    if usd >= s.min_usd { usd } else { 0.0 }
}

/// Price paid entering (`side` +1 long, −1 short) at reference `px`.
pub fn entry_fill(px: f64, side: f64, side_bps: f64) -> f64 {
    px * (1.0 + side * side_bps / 1e4)
}

/// Price received leaving at reference `px`.
pub fn exit_fill(px: f64, side: f64, side_bps: f64) -> f64 {
    px * (1.0 - side * side_bps / 1e4)
}

/// Stop price `stop_bps` against the entry fill.
pub fn stop_px(entry: f64, side: f64, stop_bps: f64) -> f64 {
    entry * (1.0 - side * stop_bps / 1e4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn costs_add_up_like_the_scalper() {
        let c = Costs::default();
        // Scalper taker round trip: 2 × 4.5 fee + 2 × 1 slippage + 2 spread = 13 bps.
        assert!((c.round_trip_bps("ETH") - 13.0).abs() < 1e-12);
        assert!((c.scaled(2.0).round_trip_bps("ETH") - 26.0).abs() < 1e-12);
        let mut wide = Costs::default();
        wide.spread.insert("ZEC".into(), 6.0);
        assert!((wide.side_bps("ZEC") - 4.0).abs() < 1e-12);
        assert!((entry_fill(100.0, 1.0, 2.0) - 100.02).abs() < 1e-9);
        assert!((entry_fill(100.0, -1.0, 2.0) - 99.98).abs() < 1e-9);
        assert!((exit_fill(100.0, 1.0, 2.0) - 99.98).abs() < 1e-9);
        assert!((stop_px(100.0, 1.0, 500.0) - 95.0).abs() < 1e-9);
        assert!((stop_px(100.0, -1.0, 500.0) - 105.0).abs() < 1e-9);
    }

    #[test]
    fn volatile_coins_get_smaller_positions() {
        let s = Sizing::default(); // 0.4% of capital per daily sigma, 15% cap
        let calm = size_usd(&s, 10_000.0, 0.02, 0.0);
        let wild = size_usd(&s, 10_000.0, 0.08, 0.0);
        assert!((wild - 500.0).abs() < 1e-9);
        assert!((calm - 1500.0).abs() < 1e-9, "20% wanted, capped at 15%");
        assert_eq!(size_usd(&s, 10_000.0, f64::NAN, 0.0), 0.0, "no volatility yet: no trade");
        // The gross cap leaves $300 of room; under the minimum it is no trade.
        assert!((size_usd(&s, 10_000.0, 0.08, 9_700.0) - 300.0).abs() < 1e-9);
        assert_eq!(size_usd(&s, 10_000.0, 0.08, 9_995.0), 0.0);
        let fixed = Sizing { mode: SizingMode::Fixed, ..Sizing::default() };
        assert!((size_usd(&fixed, 10_000.0, f64::NAN, 0.0) - 1000.0).abs() < 1e-9);
    }
}
