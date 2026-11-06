//! Pure entry/exit rules, mirrored for longs and shorts. Market microstructure
//! decides; news only tilts the flow threshold and vetoes (the Jev entry review is
//! applied by the caller).

use serde::{Deserialize, Serialize};

use super::features::Features;
use crate::config::ScalpConfig;
use crate::status::GateResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// Buy above the neutral inventory, sell back down.
    #[default]
    Long,
    /// Sell part of the neutral inventory, buy it back.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryEval {
    pub side: Side,
    pub gates: Vec<GateResult>,
    /// Burst + flow + book + push all agree: a candidate worth recording.
    pub candidate: bool,
    /// Every market gate passed (risk gates and the Jev review still apply).
    pub ok: bool,
    pub cost_bps: f64,
    pub tp_bps: f64,
    pub sl_bps: f64,
    pub flow_threshold: f64,
}

impl EntryEval {
    pub fn first_block(&self) -> Option<&str> {
        self.gates.iter().find(|g| !g.passed).map(|g| g.gate.as_str())
    }
}

/// Round-trip cost in bps: two fees, two slippages, and the spread crossed once.
pub fn round_trip_cost_bps(f: &Features, cfg: &ScalpConfig) -> f64 {
    2.0 * cfg.fee_bps + 2.0 * cfg.slippage_bps + f.spread_bps
}

/// The same gates for both sides, with every directional reading multiplied by the
/// side's sign. News tone helps the side it agrees with and hinders the other.
pub fn evaluate_entry(f: &Features, news: &NewsCtx, cfg: &ScalpConfig, side: Side) -> EntryEval {
    let s = side.sign();
    let mut gates = Vec::new();
    let mut gate = |name: &str, pass: bool, detail: String| {
        gates.push(GateResult::new(name, pass, detail));
        pass
    };
    let cost = round_trip_cost_bps(f, cfg);
    let flow_threshold = (cfg.min_flow_imbalance - cfg.news_tilt * s * news.score.clamp(-1.0, 1.0)).max(0.05);
    let (buyers, rising) = match side {
        Side::Long => ("buyers", "rising"),
        Side::Short => ("sellers", "falling"),
    };

    let warm = gate("warm-up", f.history_secs >= cfg.warmup_secs, format!("{}s of {}s", f.history_secs, cfg.warmup_secs));
    let fresh = gate("feed", f.book_age_ms <= cfg.feed_stale_ms, format!("book {}ms old", f.book_age_ms));
    let spread = gate("spread", f.spread_bps <= cfg.max_spread_bps, format!("{:.2} bps", f.spread_bps));
    let burst = gate("burst", f.activity >= cfg.min_activity_ratio, format!("activity {:.2}× (≥ {:.2})", f.activity, cfg.min_activity_ratio));
    let flow = gate("flow", s * f.flow >= flow_threshold, format!("{buyers} {:+.2} (≥ {:+.2})", s * f.flow, flow_threshold));
    let book = gate("book", s * f.obi >= cfg.min_book_imbalance, format!("{buyers} {:+.2} (≥ {:+.2})", s * f.obi, cfg.min_book_imbalance));
    let push = gate(
        "push",
        s * f.ret_60s_bps >= cfg.min_push_bps && s * f.micro_bps >= 0.0,
        format!("{rising} {:+.1} bps in 60s, micro {:+.2} bps", s * f.ret_60s_bps, s * f.micro_bps),
    );
    let chase = gate("no chase", s * f.ret_300s_bps <= cfg.max_chase_bps, format!("5m {:+.1} bps (≤ {:.0})", s * f.ret_300s_bps, cfg.max_chase_bps));
    let pays = gate(
        "cost",
        f.exp_move_bps >= cfg.cost_mult * cost,
        format!("expected move {:.1} bps vs {:.1}× cost {:.1}", f.exp_move_bps, cfg.cost_mult, cost),
    );
    // A fresh hack blocks buying the coin; it doesn't block selling it.
    let calm = gate(
        "news",
        side == Side::Short || !news.hack_alert,
        if news.hack_alert { "hack alert".into() } else { format!("score {:+.2}", news.score) },
    );

    let candidate = burst && flow && book && push;
    let ok = warm && fresh && spread && candidate && chase && pays && calm;
    EntryEval {
        side,
        gates,
        candidate,
        ok,
        cost_bps: cost,
        tp_bps: (cfg.tp_vol_mult * f.exp_move_bps).max(cfg.tp_cost_mult * cost),
        sl_bps: (cfg.sl_vol_mult * f.exp_move_bps).max(cfg.sl_cost_mult * cost),
        flow_threshold,
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

/// Longs are judged on the bid (what a sell gets), shorts on the ask (what buying back costs).
pub fn evaluate_exit(p: &OpenPos, f: Option<&Features>, news: &NewsCtx, now_ms: i64, cfg: &ScalpConfig) -> Option<String> {
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
        if -s * f.flow >= cfg.min_flow_imbalance && -s * f.obi >= cfg.min_book_imbalance && f.activity >= 1.0 {
            return Some("flow reversed".into());
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

    pub fn hot() -> Features {
        Features {
            ts_ms: 0,
            mid: 100.0,
            bid: 99.995,
            ask: 100.005,
            spread_bps: 1.0,
            micro_bps: 0.2,
            obi: 0.3,
            flow: 0.5,
            activity: 2.5,
            ret_60s_bps: 8.0,
            ret_300s_bps: 20.0,
            sigma_1s_bps: 2.5,
            exp_move_bps: 2.5 * 600f64.sqrt(),
            history_secs: 3600,
            book_age_ms: 100,
        }
    }

    /// The same market, mirrored: sellers in control, price falling.
    fn cold() -> Features {
        let mut f = hot();
        f.micro_bps = -0.2;
        f.obi = -0.3;
        f.flow = -0.5;
        f.ret_60s_bps = -8.0;
        f.ret_300s_bps = -20.0;
        f
    }

    #[test]
    fn hot_market_passes_long_only() {
        let cfg = ScalpConfig::default();
        let e = evaluate_entry(&hot(), &NewsCtx::default(), &cfg, Side::Long);
        assert!(e.ok, "{:?}", e.first_block());
        assert!((e.cost_bps - 23.0).abs() < 1e-9);
        assert!(e.tp_bps > e.sl_bps && e.sl_bps >= e.cost_bps);
        assert!(!evaluate_entry(&hot(), &NewsCtx::default(), &cfg, Side::Short).candidate);
    }

    #[test]
    fn mirrored_market_passes_short_only() {
        let cfg = ScalpConfig::default();
        let e = evaluate_entry(&cold(), &NewsCtx::default(), &cfg, Side::Short);
        assert!(e.ok, "{:?}", e.first_block());
        assert!(!evaluate_entry(&cold(), &NewsCtx::default(), &cfg, Side::Long).candidate);
    }

    #[test]
    fn cost_gate_blocks_quiet_markets() {
        let cfg = ScalpConfig::default();
        let mut f = hot();
        f.exp_move_bps = 600f64.sqrt(); // ≈ 24.5 bps < 2 × 23
        let e = evaluate_entry(&f, &NewsCtx::default(), &cfg, Side::Long);
        assert!(e.candidate && !e.ok);
        assert_eq!(e.first_block(), Some("cost"));
    }

    #[test]
    fn news_tilts_each_side_but_never_triggers() {
        let cfg = ScalpConfig::default();
        let good = NewsCtx { score: 1.0, ..Default::default() };
        let bad = NewsCtx { score: -1.0, ..Default::default() };
        let mut f = hot();
        f.flow = 0.2; // below 0.25, above 0.25 − 0.10
        assert!(!evaluate_entry(&f, &NewsCtx::default(), &cfg, Side::Long).ok);
        assert!(evaluate_entry(&f, &good, &cfg, Side::Long).ok);
        let mut g = cold();
        g.flow = -0.2;
        assert!(evaluate_entry(&g, &bad, &cfg, Side::Short).ok, "bad news lowers the bar for shorts");
        assert!(!evaluate_entry(&g, &good, &cfg, Side::Short).ok, "good news raises it");
        // Great news with no order flow does nothing.
        let mut quiet = hot();
        quiet.activity = 0.8;
        quiet.flow = 0.0;
        assert!(!evaluate_entry(&quiet, &good, &cfg, Side::Long).candidate);
    }

    #[test]
    fn hack_blocks_longs_not_shorts() {
        let cfg = ScalpConfig::default();
        let hack = NewsCtx { hack_alert: true, ..Default::default() };
        assert_eq!(evaluate_entry(&hot(), &hack, &cfg, Side::Long).first_block(), Some("news"));
        assert!(evaluate_entry(&cold(), &hack, &cfg, Side::Short).ok);
    }

    #[test]
    fn exits_both_sides() {
        let cfg = ScalpConfig::default();
        let n = NewsCtx::default();
        let long = OpenPos { side: Side::Long, tp_px: 100.5, sl_px: 99.5, deadline_ms: 1000 };
        let short = OpenPos { side: Side::Short, tp_px: 99.5, sl_px: 100.5, deadline_ms: 1000 };
        let at = |bid: f64, ask: f64| Features { bid, ask, flow: 0.0, obi: 0.0, ..hot() };
        assert_eq!(evaluate_exit(&long, Some(&at(100.0, 100.01)), &n, 0, &cfg), None);
        assert_eq!(evaluate_exit(&long, Some(&at(100.6, 100.61)), &n, 0, &cfg).as_deref(), Some("take profit"));
        assert_eq!(evaluate_exit(&long, Some(&at(99.4, 99.41)), &n, 0, &cfg).as_deref(), Some("stop"));
        assert_eq!(evaluate_exit(&short, Some(&at(99.40, 99.45)), &n, 0, &cfg).as_deref(), Some("take profit"));
        assert_eq!(evaluate_exit(&short, Some(&at(100.55, 100.6)), &n, 0, &cfg).as_deref(), Some("stop"));
        assert_eq!(evaluate_exit(&long, Some(&cold()), &n, 0, &cfg).as_deref(), Some("flow reversed"));
        assert_eq!(evaluate_exit(&short, Some(&hot()), &n, 0, &cfg).as_deref(), Some("flow reversed"));
        assert_eq!(evaluate_exit(&short, None, &n, 1000, &cfg).as_deref(), Some("time stop"));
        let hack = NewsCtx { hack_alert: true, ..Default::default() };
        assert_eq!(evaluate_exit(&long, None, &hack, 0, &cfg).as_deref(), Some("hack alert"));
        assert_eq!(evaluate_exit(&short, None, &hack, 0, &cfg), None, "a hack doesn't close a short");
    }

    #[test]
    fn trailing_stop_locks_breakeven_then_follows() {
        let cfg = ScalpConfig::default(); // trail after 1.5 × cost
        let (cost, sl_bps) = (23.0, 30.0);
        // Long from 100, stop at 99.70. Best 100.20 (+20 bps < 34.5): unchanged.
        assert_eq!(trailed_stop(Side::Long, 100.0, 100.2, 99.7, sl_bps, cost, &cfg), 99.7);
        // Best 100.50 (+50 bps): max(break-even 100.23, trail 100.50 × 0.997 = 100.1985) = 100.23.
        assert!((trailed_stop(Side::Long, 100.0, 100.5, 99.7, sl_bps, cost, &cfg) - 100.23).abs() < 1e-9);
        // Best 101.00: the trail (100.697) is above break-even.
        assert!((trailed_stop(Side::Long, 100.0, 101.0, 99.7, sl_bps, cost, &cfg) - 100.697).abs() < 1e-9);
        // Mirrored for a short from 100 with its stop at 100.30.
        assert!((trailed_stop(Side::Short, 100.0, 99.5, 100.3, sl_bps, cost, &cfg) - 99.77).abs() < 1e-9);
        assert!((trailed_stop(Side::Short, 100.0, 99.0, 100.3, sl_bps, cost, &cfg) - 99.297).abs() < 1e-9);
        // Never loosens: an already tighter stop stays.
        assert_eq!(trailed_stop(Side::Long, 100.0, 100.5, 100.4, sl_bps, cost, &cfg), 100.4);
    }
}
