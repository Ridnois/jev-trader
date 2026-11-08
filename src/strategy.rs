//! Pure trading rules (no I/O), mirrored for longs and shorts.
//!
//! Two ways into a trade, both needing news and the market to agree:
//! - **catalyst**: a fresh, concrete, strong headline (Jev) on a coin, confirmed by
//!   order flow in the same direction before price has run; held up to hours.
//! - **flow**: the v1 scalper's microstructure burst, allowed only when the coin's
//!   decayed news bias points the same way.
//!
//! Exits: stop (taker), take profit (resting maker order), trailing to break-even
//! after costs once +`trail_r` R, time stop, opposite strong news, hack alert.
//! v1's "flow reversed" exit is gone: it was the most-used and worst exit
//! (33 of 56 trades, −12.7 bps each).

use serde::{Deserialize, Serialize};

use crate::config::{CatalystConfig, Costs, EntryMode, FlowConfig, RiskConfig};
use crate::features::Features;
use crate::intel::{Bias, NewsEvent};
use crate::status::Gate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    #[default]
    Long,
    Short,
}

impl Side {
    /// +1 long, −1 short: a move of `sign × x` is in the trade's favour.
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

    pub fn of(x: f64) -> Side {
        if x >= 0.0 { Side::Long } else { Side::Short }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TradeKind {
    #[default]
    Catalyst,
    Flow,
}

/// Round trip in bps from the mid: the entry fee, a taker exit (the stop path, so
/// the cost is not flattered by assuming the maker target), one slippage per taker
/// leg, the spread, and funding the side would pay over the hold.
pub fn round_trip_bps(c: &Costs, entry: EntryMode, spread_bps: f64, funding_bps_1h: f64, side: Side, hold_secs: i64) -> f64 {
    let entry_leg = match entry {
        EntryMode::Taker => c.taker_bps + c.slippage_bps + 0.5 * spread_bps,
        EntryMode::Maker => c.maker_bps,
    };
    let exit_leg = c.taker_bps + c.slippage_bps + 0.5 * spread_bps;
    let funding = (side.sign() * funding_bps_1h).max(0.0) * hold_secs as f64 / 3600.0;
    entry_leg + exit_leg + funding
}

/// Notional from risk: `risk_pct × capital × mult / stop`, capped per coin and by
/// free collateral. Returns the notional and which cap bound.
pub fn size_usd(capital: f64, free: f64, risk_pct: f64, mult: f64, stop_bps: f64, r: &RiskConfig) -> (f64, &'static str) {
    if stop_bps <= 0.0 || !stop_bps.is_finite() {
        return (0.0, "no stop");
    }
    let by_risk = risk_pct * capital * mult / (stop_bps / 1e4);
    let by_coin = r.max_coin_pct * capital;
    let mut out = (by_risk, "risk");
    if by_coin < out.0 {
        out = (by_coin, "coin cap");
    }
    if free < out.0 {
        out = (free.max(0.0), "collateral");
    }
    out
}

/// Stop, target and hold of a planned trade, in bps from entry.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Plan {
    pub side: Side,
    pub stop_bps: f64,
    pub tp_bps: f64,
    pub cost_bps: f64,
    pub hold_secs: i64,
}

// ---------------------------------------------------------------- catalyst

/// Is this headline strong and fresh enough to watch for a catalyst trade?
pub fn catalyst_trigger(e: &NewsEvent, cfg: &CatalystConfig) -> Vec<Gate> {
    vec![
        Gate::new("impact", e.impact.abs() >= cfg.min_impact, format!("{:+.2} (≥ ±{:.2})", e.impact, cfg.min_impact)),
        Gate::new("new fact", e.new_info >= cfg.min_new_info, format!("{:.2} (≥ {:.2})", e.new_info, cfg.min_new_info)),
        Gate::new("fresh", e.lag_min <= cfg.max_lag_min, format!("seen {:.0} min after publication (≤ {:.0})", e.lag_min, cfg.max_lag_min)),
    ]
}

/// A headline waiting for the market to confirm it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watch {
    pub event_id: String,
    pub title: String,
    pub source: String,
    pub coin: String,
    pub side: Side,
    pub impact: f64,
    pub new_info: f64,
    pub created_ms: i64,
    pub expires_ms: i64,
    /// Price when the headline was published (1m candle close), for the chase check.
    pub p_pub: f64,
    /// ATR(14) of 1h candles as a fraction of price.
    pub atr: f64,
    pub last_block: String,
}

/// Market checks for a watched catalyst: confirmation by flow, not chased, tradable.
pub fn catalyst_entry(w: &Watch, f: &Features, cfg: &CatalystConfig, costs: &Costs, risk: &RiskConfig) -> (Vec<Gate>, Plan) {
    let s = w.side.sign();
    let moved = s * (f.mid / w.p_pub - 1.0);
    let moved_atr = if w.atr > 0.0 { moved / w.atr } else { f64::INFINITY };
    let cost = round_trip_bps(costs, cfg.entry, f.spread_bps, f.funding_bps_1h, w.side, cfg.hold_min * 60);
    let stop_bps = (cfg.stop_atr * w.atr * 1e4).max(cfg.min_stop_cost_mult * cost);
    let who = if w.side == Side::Long { "buyers" } else { "sellers" };
    let gates = vec![
        Gate::new("feed", f.book_age_ms <= risk.feed_stale_ms, format!("book {}ms old", f.book_age_ms)),
        Gate::new("confirm", s * f.flow >= cfg.confirm_flow && s * f.ret_60s_bps >= 0.0, format!("{who} {:+.2} (≥ {:.2}), 60s {:+.1} bps", s * f.flow, cfg.confirm_flow, s * f.ret_60s_bps)),
        Gate::new("not chased", moved_atr <= cfg.max_chase_atr, format!("{:+.2}% since publication = {:.2} ATR (≤ {:.1})", moved * 100.0, moved_atr, cfg.max_chase_atr)),
        Gate::new("funding", s * f.funding_bps_1h <= risk.max_funding_bps_1h, format!("{:+.3} bps/h paid by this side (≤ {:.2})", s * f.funding_bps_1h, risk.max_funding_bps_1h)),
        Gate::new("atr", w.atr > 0.0 && w.atr.is_finite(), format!("{:.2}%", w.atr * 100.0)),
    ];
    (gates, Plan { side: w.side, stop_bps, tp_bps: cfg.tp_r * stop_bps, cost_bps: cost, hold_secs: cfg.hold_min * 60 })
}

// ---------------------------------------------------------------- flow

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FlowEval {
    pub side: Side,
    pub gates: Vec<Gate>,
    /// Burst + flow + book + push agree: worth logging (with markouts) even if blocked.
    pub candidate: bool,
    pub ok: bool,
    pub plan: Plan,
}

/// The v1 scalper's gates plus the news-alignment gate.
pub fn flow_entry(f: &Features, b: &Bias, cfg: &FlowConfig, costs: &Costs, risk: &RiskConfig, side: Side) -> FlowEval {
    let s = side.sign();
    let cost = round_trip_bps(costs, cfg.entry, f.spread_bps, f.funding_bps_1h, side, cfg.hold_secs);
    let (who, dir) = if side == Side::Long { ("buyers", "rising") } else { ("sellers", "falling") };
    let mut gates = Vec::new();
    let mut g = |n: &str, pass: bool, d: String| {
        gates.push(Gate::new(n, pass, d));
        pass
    };
    let warm = g("warm-up", f.history_secs >= cfg.warmup_secs, format!("{}s of {}s", f.history_secs, cfg.warmup_secs));
    let fresh = g("feed", f.book_age_ms <= risk.feed_stale_ms, format!("book {}ms old", f.book_age_ms));
    let spread = g("spread", f.spread_bps <= cfg.max_spread_bps, format!("{:.2} bps", f.spread_bps));
    let burst = g("burst", f.activity >= cfg.min_activity_ratio, format!("activity {:.2}× (≥ {:.2})", f.activity, cfg.min_activity_ratio));
    let flow = g("flow", s * f.flow >= cfg.min_flow_imbalance, format!("{who} {:+.2} (≥ {:.2})", s * f.flow, cfg.min_flow_imbalance));
    let book = g("book", s * f.obi >= cfg.min_book_imbalance, format!("{who} {:+.2} (≥ {:.2})", s * f.obi, cfg.min_book_imbalance));
    let push = g("push", s * f.ret_60s_bps >= cfg.min_push_bps && s * f.micro_bps >= 0.0, format!("{dir} {:+.1} bps in 60s", s * f.ret_60s_bps));
    let chase = g("no chase", s * f.ret_300s_bps <= cfg.max_chase_bps, format!("5m {:+.1} bps (≤ {:.0})", s * f.ret_300s_bps, cfg.max_chase_bps));
    let pays = g("cost", f.exp_move_bps >= cfg.cost_mult * cost, format!("expected move {:.1} bps vs {:.1}× cost {:.1}", f.exp_move_bps, cfg.cost_mult, cost));
    let news = g("news agrees", s * b.score >= cfg.min_bias, format!("bias {:+.2} (needs {}{:.2})", b.score, if side == Side::Long { "≥ +" } else { "≤ −" }, cfg.min_bias));
    let calm = g("no hack", side == Side::Short || !b.hack_alert, if b.hack_alert { "hack alert".into() } else { String::new() });
    let funding = g("funding", s * f.funding_bps_1h <= risk.max_funding_bps_1h, format!("{:+.3} bps/h paid", s * f.funding_bps_1h));
    let candidate = burst && flow && book && push;
    let ok = warm && fresh && spread && candidate && chase && pays && news && calm && funding;
    let stop_bps = (cfg.stop_vol_mult * f.exp_move_bps).max(cfg.min_stop_cost_mult * cost);
    FlowEval { side, gates, candidate, ok, plan: Plan { side, stop_bps, tp_bps: cfg.tp_r * stop_bps, cost_bps: cost, hold_secs: cfg.hold_secs } }
}

// ---------------------------------------------------------------- exits

/// What the exit rules need about an open position.
#[derive(Debug, Clone, Copy)]
pub struct OpenPos {
    pub side: Side,
    pub sl_px: f64,
    pub deadline_ms: i64,
}

/// Exit reasons other than the take profit (which rests as a maker order).
/// Longs are judged on the bid, shorts on the ask.
pub fn exit_reason(p: &OpenPos, f: Option<&Features>, b: &Bias, now_ms: i64) -> Option<&'static str> {
    if b.hack_alert && p.side == Side::Long {
        return Some("hack alert");
    }
    if let Some(f) = f {
        let px = if p.side == Side::Long { f.bid } else { f.ask };
        if p.side.sign() * (px - p.sl_px) <= 0.0 {
            return Some("stop");
        }
    }
    // Strong opposite news: the reason for the trade is gone.
    if p.side.sign() * b.score <= -0.5 {
        return Some("news turned");
    }
    if now_ms >= p.deadline_ms {
        return Some("time stop");
    }
    None
}

/// Once the best price is `trail_r` stop distances in profit, the stop moves to
/// break-even after costs, then follows the best price at the original distance.
/// It never moves against the trade.
pub fn trailed_stop(side: Side, entry: f64, best: f64, sl_px: f64, stop_bps: f64, cost_bps: f64, trail_r: f64) -> f64 {
    let s = side.sign();
    let gained = s * (best / entry - 1.0) * 1e4;
    if trail_r <= 0.0 || gained < trail_r * stop_bps {
        return sl_px;
    }
    let breakeven = entry * (1.0 + s * cost_bps / 1e4);
    let trail = best * (1.0 - s * stop_bps / 1e4);
    match side {
        Side::Long => sl_px.max(breakeven).max(trail),
        Side::Short => sl_px.min(breakeven).min(trail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intel::sources::Kind;

    fn hot() -> Features {
        Features {
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
            exp_move_bps: 2.5 * 1800f64.sqrt(),
            history_secs: 3600,
            book_age_ms: 100,
            ..Default::default()
        }
    }

    fn cold() -> Features {
        Features { micro_bps: -0.2, obi: -0.3, flow: -0.5, ret_60s_bps: -8.0, ret_300s_bps: -20.0, ..hot() }
    }

    fn event(impact: f64, new_info: f64, lag: f64) -> NewsEvent {
        let now = chrono::Utc::now();
        NewsEvent { id: "e".into(), title: "t".into(), source: "s".into(), kind: Kind::News, published: now, first_seen: now, lag_min: lag, coins: vec!["SOL".into()], market: false, impact, impact_conf: 1.0, new_info, hack: None, model: "m".into() }
    }

    fn watch(side: Side, p_pub: f64) -> Watch {
        Watch { event_id: "e".into(), title: "t".into(), source: "s".into(), coin: "SOL".into(), side, impact: side.sign() * 0.7, new_info: 0.9, created_ms: 0, expires_ms: 1, p_pub, atr: 0.006, last_block: String::new() }
    }

    #[test]
    fn costs() {
        let c = Costs::default();
        // Taker in: 4.5 + 1 + 0.5; taker out the same; no funding paid by a short when funding > 0.
        assert!((round_trip_bps(&c, EntryMode::Taker, 1.0, 0.5, Side::Short, 3600) - 12.0).abs() < 1e-9);
        assert!((round_trip_bps(&c, EntryMode::Taker, 1.0, 0.5, Side::Long, 3600) - 12.5).abs() < 1e-9);
        assert!((round_trip_bps(&c, EntryMode::Maker, 1.0, 0.0, Side::Long, 3600) - 7.5).abs() < 1e-9);
    }

    #[test]
    fn sizing_caps() {
        let r = RiskConfig::default();
        // 0.5% of 10k at a 50 bps stop = $10k, capped at 35% = $3.5k.
        assert_eq!(size_usd(10_000.0, 10_000.0, 0.005, 1.0, 50.0, &r), (3500.0, "coin cap"));
        assert_eq!(size_usd(10_000.0, 10_000.0, 0.005, 0.5, 200.0, &r), (1250.0, "risk"));
        assert_eq!(size_usd(10_000.0, 800.0, 0.005, 1.0, 50.0, &r), (800.0, "collateral"));
    }

    #[test]
    fn catalyst_needs_strength_freshness_and_confirmation() {
        let cfg = CatalystConfig::default();
        assert!(catalyst_trigger(&event(0.7, 0.9, 3.0), &cfg).iter().all(|g| g.passed));
        assert!(!catalyst_trigger(&event(0.2, 0.9, 3.0), &cfg).iter().all(|g| g.passed));
        assert!(!catalyst_trigger(&event(0.7, 0.2, 3.0), &cfg).iter().all(|g| g.passed), "opinion is not a catalyst");
        assert!(!catalyst_trigger(&event(0.7, 0.9, 90.0), &cfg).iter().all(|g| g.passed), "late news");
        let (c, r) = (Costs::default(), RiskConfig::default());
        let (g, plan) = catalyst_entry(&watch(Side::Long, 99.9), &hot(), &cfg, &c, &r);
        assert!(g.iter().all(|g| g.passed), "{g:?}");
        assert!((plan.stop_bps - 60.0).abs() < 1e-9 && (plan.tp_bps - 120.0).abs() < 1e-9);
        // Sellers in control: no confirmation for a long.
        assert!(!catalyst_entry(&watch(Side::Long, 99.9), &cold(), &cfg, &c, &r).0.iter().all(|g| g.passed));
        // Mirrored short confirms on the cold tape.
        assert!(catalyst_entry(&watch(Side::Short, 100.1), &cold(), &cfg, &c, &r).0.iter().all(|g| g.passed));
        // Already up 1.5% on a 0.6% ATR: chased.
        let (g, _) = catalyst_entry(&watch(Side::Long, 98.5), &hot(), &cfg, &c, &r);
        assert_eq!(crate::status::first_block(&g), Some("not chased"));
    }

    #[test]
    fn flow_needs_news_agreement() {
        let (cfg, c, r) = (FlowConfig::default(), Costs::default(), RiskConfig::default());
        let none = Bias::default();
        let bull = Bias { score: 0.4, ..Default::default() };
        let e = flow_entry(&hot(), &none, &cfg, &c, &r, Side::Long);
        assert!(e.candidate && !e.ok);
        assert_eq!(crate::status::first_block(&e.gates), Some("news agrees"));
        assert!(flow_entry(&hot(), &bull, &cfg, &c, &r, Side::Long).ok);
        assert!(!flow_entry(&cold(), &bull, &cfg, &c, &r, Side::Short).ok, "bullish news blocks a short");
        let bear = Bias { score: -0.4, hack_alert: true, n: 1 };
        assert!(flow_entry(&cold(), &bear, &cfg, &c, &r, Side::Short).ok, "a hack doesn't block shorts");
        let mut quiet = hot();
        quiet.exp_move_bps = 10.0;
        assert_eq!(crate::status::first_block(&flow_entry(&quiet, &bull, &cfg, &c, &r, Side::Long).gates), Some("cost"));
    }

    #[test]
    fn exits() {
        let none = Bias::default();
        let long = OpenPos { side: Side::Long, sl_px: 99.5, deadline_ms: 1000 };
        let short = OpenPos { side: Side::Short, sl_px: 100.5, deadline_ms: 1000 };
        let at = |bid: f64, ask: f64| Features { bid, ask, ..hot() };
        assert_eq!(exit_reason(&long, Some(&at(100.0, 100.01)), &none, 0), None);
        assert_eq!(exit_reason(&long, Some(&at(99.4, 99.41)), &none, 0), Some("stop"));
        assert_eq!(exit_reason(&short, Some(&at(100.55, 100.6)), &none, 0), Some("stop"));
        assert_eq!(exit_reason(&short, None, &none, 1000), Some("time stop"));
        assert_eq!(exit_reason(&long, None, &Bias { score: -0.6, ..none }, 0), Some("news turned"));
        assert_eq!(exit_reason(&short, None, &Bias { score: -0.6, ..none }, 0), None);
        assert_eq!(exit_reason(&long, None, &Bias { hack_alert: true, ..none }, 0), Some("hack alert"));
    }

    #[test]
    fn trailing() {
        // Long from 100, stop 60 bps (99.40), cost 12 bps, trail after 1R.
        assert_eq!(trailed_stop(Side::Long, 100.0, 100.5, 99.4, 60.0, 12.0, 1.0), 99.4);
        assert!((trailed_stop(Side::Long, 100.0, 100.6, 99.4, 60.0, 12.0, 1.0) - 100.12).abs() < 1e-9, "break-even after costs");
        assert!((trailed_stop(Side::Long, 100.0, 101.5, 99.4, 60.0, 12.0, 1.0) - 100.891).abs() < 1e-9, "then trails");
        assert!((trailed_stop(Side::Short, 100.0, 99.39, 100.6, 60.0, 12.0, 1.0) - 99.88).abs() < 1e-9);
        assert_eq!(trailed_stop(Side::Long, 100.0, 100.6, 100.5, 60.0, 12.0, 1.0), 100.5, "never loosens");
    }
}
