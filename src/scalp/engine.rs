//! Pure entry/exit rules, mirrored for longs and shorts: fade 15m closes stretched
//! far from their 3-day mean, hold up to `hold_secs`, hard stop. News only tilts
//! the threshold and vetoes (the Jev entry review is applied by the caller).

use serde::{Deserialize, Serialize};

use super::features::Features;
use crate::config::{EntryMode, ScalpConfig};
use crate::status::GateResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// Buy the perp, sell it back.
    #[default]
    Long,
    /// Sell the perp, buy it back.
    Short,
}

impl Side {
    /// +1 for long, −1 for short: a move of `sign × x` is in the trade's favour.
    pub fn sign(self) -> f64 {
        match self {
            Side::Long => 1.0,
            Side::Short => -1.0,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Side::Long => "long",
            Side::Short => "short",
        }
    }
}

/// Background news context for one asset (from the slow news loop).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct NewsCtx {
    /// Decayed news score, −1..1.
    pub score: f64,
    pub hack_alert: bool,
    pub n_events: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntryEval {
    pub side: Side,
    pub gates: Vec<GateResult>,
    /// Bars are fresh and price is stretched past the threshold: worth recording.
    pub candidate: bool,
    /// Every market gate passed (risk gates and the Jev review still apply).
    pub ok: bool,
    pub cost_bps: f64,
    pub tp_bps: f64,
    pub sl_bps: f64,
    /// |z| needed to fade, after the news tilt.
    pub flow_threshold: f64,
}

impl EntryEval {
    pub fn first_block(&self) -> Option<&str> {
        self.gates.iter().find(|g| !g.passed).map(|g| g.gate.as_str())
    }
}

/// Round-trip cost in bps, measured from the mid. Taker entry: two taker fees, two
/// slippages, the spread crossed once (half in, half out). Maker entry: the maker
/// fee in, a taker exit (stops must be certain), half the spread and one slippage.
/// Perps add the funding the side pays over the holding period (received funding
/// is not counted as a credit).
pub fn round_trip_cost_bps(f: &Features, cfg: &ScalpConfig, side: Side) -> f64 {
    let trading = match cfg.entry {
        EntryMode::Taker => 2.0 * cfg.fee_bps + 2.0 * cfg.slippage_bps + f.spread_bps,
        EntryMode::Maker => cfg.maker_fee_bps + cfg.fee_bps + cfg.slippage_bps + 0.5 * f.spread_bps,
    };
    // Funding the side would pay over the hold (perps).
    let funding = (side.sign() * f.funding_bps_1h).max(0.0) * cfg.hold_secs as f64 / 3600.0;
    trading + funding
}

/// Mean reversion on `revert_interval` candles, mirrored for both sides: a close
/// more than `revert_z` standard deviations above the mean of the previous
/// `revert_bars` closes is sold, one as far below is bought. News tone shifts the
/// threshold by `news_tilt` (in z units) toward the side it agrees with; a hack
/// blocks longs. Research and caveats: hypotheses.toml H-R01.
pub fn evaluate_entry(f: &Features, news: &NewsCtx, cfg: &ScalpConfig, side: Side) -> EntryEval {
    let s = side.sign();
    let mut gates = Vec::new();
    let mut gate = |name: &str, pass: bool, detail: String| {
        gates.push(GateResult::new(name, pass, detail));
        pass
    };
    let cost = round_trip_cost_bps(f, cfg, side);
    let threshold = (cfg.revert_z - cfg.news_tilt * s * news.score.clamp(-1.0, 1.0)).max(0.5);
    let bar_ms = crate::market::interval_ms(&cfg.revert_interval).unwrap_or(900_000);
    // How far price sits on the far side of the mean from where this side profits.
    let stretch = -s * f.z_rev;

    let warm = gate("warm-up", f.history_secs >= cfg.warmup_secs, format!("{}s of {}s", f.history_secs, cfg.warmup_secs));
    let fresh = gate("feed", f.book_age_ms <= cfg.feed_stale_ms, format!("book {}ms old", f.book_age_ms));
    let bars = gate(
        "bars",
        f.bars > cfg.revert_bars && f.bar_ms > 0 && f.ts_ms - f.bar_ms <= 3 * bar_ms,
        format!("{} of {} {} bars, last opened {}s ago", f.bars, cfg.revert_bars + 1, cfg.revert_interval, (f.ts_ms - f.bar_ms) / 1000),
    );
    let spread = gate("spread", f.spread_bps <= cfg.max_spread_bps, format!("{:.2} bps", f.spread_bps));
    let stretched = gate("stretch", stretch >= threshold, format!("z {:+.2} (fade needs {} {:.2})", f.z_rev, if s > 0.0 { "≤ −" } else { "≥ +" }, threshold));
    let calm = gate(
        "news",
        side == Side::Short || !news.hack_alert,
        if news.hack_alert { "hack alert".into() } else { format!("score {:+.2}", news.score) },
    );

    let candidate = bars && stretched;
    let ok = warm && fresh && spread && candidate && calm;
    EntryEval {
        side,
        gates,
        candidate,
        ok,
        cost_bps: cost,
        // No take profit: in the research the time stop beat exiting at the mean.
        // 9999 bps keeps the target finite (the ledger is JSON) and out of reach.
        tp_bps: 9999.0,
        sl_bps: cfg.revert_stop_bps,
        flow_threshold: threshold,
    }
}

/// What the exit rules need about an open position.
#[derive(Debug, Clone, Copy)]
pub struct OpenPos {
    pub side: Side,
    pub tp_px: f64,
    pub sl_px: f64,
    pub deadline_ms: i64,
}

/// Longs are judged on the bid (what a sell gets), shorts on the ask (what buying
/// back costs). Hack alert (longs), stop, take profit, then the time stop.
pub fn evaluate_exit(p: &OpenPos, f: Option<&Features>, news: &NewsCtx, now_ms: i64, _cfg: &ScalpConfig) -> Option<String> {
    if news.hack_alert && p.side == Side::Long {
        return Some("hack alert".into());
    }
    if let Some(f) = f {
        let (px, s) = match p.side {
            Side::Long => (f.bid, 1.0),
            Side::Short => (f.ask, -1.0),
        };
        if s * (px - p.tp_px) >= 0.0 {
            return Some("take profit".into());
        }
        if s * (px - p.sl_px) <= 0.0 {
            // "stop" rather than "stop loss": after trailing, a stop can lock in a gain.
            return Some("stop".into());
        }
    }
    if now_ms >= p.deadline_ms {
        return Some("time stop".into());
    }
    None
}

/// Trailing stop. Once the best price has run `trail_after_cost_mult × cost` in the
/// trade's favour, the stop moves to break-even after fees and then follows the best
/// price at the original stop distance. It never moves against the trade.
pub fn trailed_stop(side: Side, entry: f64, best: f64, sl_px: f64, sl_bps: f64, cost_bps: f64, cfg: &ScalpConfig) -> f64 {
    let s = side.sign();
    let favourable = s * (best / entry - 1.0) * 1e4;
    if cfg.trail_after_cost_mult <= 0.0 || favourable < cfg.trail_after_cost_mult * cost_bps {
        return sl_px;
    }
    let breakeven = entry * (1.0 + s * cost_bps / 1e4);
    let trail = best * (1.0 - s * sl_bps / 1e4);
    match side {
        Side::Long => sl_px.max(breakeven).max(trail),
        Side::Short => sl_px.min(breakeven).min(trail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A liquid market 3σ below its 3-day mean on fresh 15m bars.
    fn low() -> Features {
        Features {
            ts_ms: 1_000_000_000,
            mid: 100.0,
            bid: 99.995,
            ask: 100.005,
            spread_bps: 1.0,
            history_secs: 3600,
            book_age_ms: 100,
            z_rev: -3.0,
            bars: 290,
            bar_ms: 1_000_000_000 - 960_000,
            ..Default::default()
        }
    }

    fn high() -> Features {
        Features { z_rev: 3.0, ..low() }
    }

    #[test]
    fn stretched_low_is_bought_and_high_is_sold() {
        let cfg = ScalpConfig::default();
        let n = NewsCtx::default();
        let e = evaluate_entry(&low(), &n, &cfg, Side::Long);
        assert!(e.ok, "{:?}", e.first_block());
        assert_eq!(e.sl_bps, 300.0);
        assert!(!evaluate_entry(&low(), &n, &cfg, Side::Short).candidate);
        assert!(evaluate_entry(&high(), &n, &cfg, Side::Short).ok);
        assert!(!evaluate_entry(&high(), &n, &cfg, Side::Long).candidate);
        let mut mild = low();
        mild.z_rev = -1.9;
        assert_eq!(evaluate_entry(&mild, &n, &cfg, Side::Long).first_block(), Some("stretch"));
    }

    #[test]
    fn stale_or_short_bar_history_blocks() {
        let cfg = ScalpConfig::default();
        let n = NewsCtx::default();
        let mut f = low();
        f.bars = 200;
        assert_eq!(evaluate_entry(&f, &n, &cfg, Side::Long).first_block(), Some("bars"));
        let mut f = low();
        f.bar_ms = f.ts_ms - 4 * 900_000;
        assert_eq!(evaluate_entry(&f, &n, &cfg, Side::Long).first_block(), Some("bars"));
    }

    #[test]
    fn news_tilts_the_threshold() {
        let cfg = ScalpConfig { news_tilt: 0.2, ..Default::default() };
        let mut f = low();
        f.z_rev = -1.9;
        let good = NewsCtx { score: 1.0, ..Default::default() };
        assert!(evaluate_entry(&f, &good, &cfg, Side::Long).ok, "good news lowers the bar to 1.8");
        assert!(!evaluate_entry(&f, &NewsCtx { score: -1.0, ..Default::default() }, &cfg, Side::Long).ok);
    }

    #[test]
    fn maker_perps_cost_model() {
        // Hyperliquid tier 0: maker 1.5, taker 4.5 bps; 1 bps spread; 1 bps slippage.
        let cfg = ScalpConfig { entry: EntryMode::Maker, maker_fee_bps: 1.5, fee_bps: 4.5, slippage_bps: 1.0, hold_secs: 3600, ..Default::default() };
        let mut f = low();
        f.funding_bps_1h = 0.5; // longs pay 0.5 bps/hour
        assert!((round_trip_cost_bps(&f, &cfg, Side::Long) - (1.5 + 4.5 + 1.0 + 0.5 + 0.5)).abs() < 1e-9);
        assert!((round_trip_cost_bps(&f, &cfg, Side::Short) - 7.5).abs() < 1e-9, "received funding is not a credit");
        let taker = ScalpConfig { entry: EntryMode::Taker, ..cfg };
        assert!((round_trip_cost_bps(&f, &taker, Side::Short) - (9.0 + 2.0 + 1.0)).abs() < 1e-9);
    }

    #[test]
    fn hack_blocks_longs_not_shorts() {
        let cfg = ScalpConfig::default();
        let hack = NewsCtx { hack_alert: true, ..Default::default() };
        assert_eq!(evaluate_entry(&low(), &hack, &cfg, Side::Long).first_block(), Some("news"));
        assert!(evaluate_entry(&high(), &hack, &cfg, Side::Short).ok);
    }

    #[test]
    fn exits_both_sides() {
        let cfg = ScalpConfig::default();
        let n = NewsCtx::default();
        let long = OpenPos { side: Side::Long, tp_px: 100.5, sl_px: 99.5, deadline_ms: 1000 };
        let short = OpenPos { side: Side::Short, tp_px: 99.5, sl_px: 100.5, deadline_ms: 1000 };
        let at = |bid: f64, ask: f64| Features { bid, ask, ..low() };
        assert_eq!(evaluate_exit(&long, Some(&at(100.0, 100.01)), &n, 0, &cfg), None);
        assert_eq!(evaluate_exit(&long, Some(&at(100.6, 100.61)), &n, 0, &cfg).as_deref(), Some("take profit"));
        assert_eq!(evaluate_exit(&long, Some(&at(99.4, 99.41)), &n, 0, &cfg).as_deref(), Some("stop"));
        assert_eq!(evaluate_exit(&short, Some(&at(99.40, 99.45)), &n, 0, &cfg).as_deref(), Some("take profit"));
        assert_eq!(evaluate_exit(&short, Some(&at(100.55, 100.6)), &n, 0, &cfg).as_deref(), Some("stop"));
        assert_eq!(evaluate_exit(&short, None, &n, 1000, &cfg).as_deref(), Some("time stop"));
        let hack = NewsCtx { hack_alert: true, ..Default::default() };
        assert_eq!(evaluate_exit(&long, None, &hack, 0, &cfg).as_deref(), Some("hack alert"));
        assert_eq!(evaluate_exit(&short, None, &hack, 0, &cfg), None, "a hack doesn't close a short");
    }

    #[test]
    fn far_target_never_triggers() {
        let cfg = ScalpConfig::default();
        let e = evaluate_entry(&high(), &NewsCtx::default(), &cfg, Side::Short);
        let tp = 100.0 * (1.0 - e.tp_bps / 1e4);
        assert!(tp > 0.0 && tp < 0.1, "short target {tp} is finite and unreachable");
    }

    #[test]
    fn trailing_stop_locks_breakeven_then_follows() {
        let cfg = ScalpConfig::default(); // trail after 1.5 × cost
        let (cost, sl_bps) = (23.0, 30.0);
        assert_eq!(trailed_stop(Side::Long, 100.0, 100.2, 99.7, sl_bps, cost, &cfg), 99.7);
        assert!((trailed_stop(Side::Long, 100.0, 100.5, 99.7, sl_bps, cost, &cfg) - 100.23).abs() < 1e-9);
        assert!((trailed_stop(Side::Long, 100.0, 101.0, 99.7, sl_bps, cost, &cfg) - 100.697).abs() < 1e-9);
        assert!((trailed_stop(Side::Short, 100.0, 99.5, 100.3, sl_bps, cost, &cfg) - 99.77).abs() < 1e-9);
        assert!((trailed_stop(Side::Short, 100.0, 99.0, 100.3, sl_bps, cost, &cfg) - 99.297).abs() < 1e-9);
        assert_eq!(trailed_stop(Side::Long, 100.0, 100.5, 100.4, sl_bps, cost, &cfg), 100.4);
    }
}
