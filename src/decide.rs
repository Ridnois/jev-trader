//! 2.1: the decision path shared by live trading and backtests.
//! `decide` is pure: it reads state + inputs and returns targets with a complete
//! gate trace (0.6). `execute` applies a decision to the paper ledger.
//! `now` is always passed in, never read from the clock.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::breaker::Breaker;
use crate::config::{ChallengeAction, Config, RegimeMode, Sizing, StopMode};
use crate::news::{Challenge, NewsEvent};
use crate::paper::{self, Binding, Fill, State};
use crate::regime::{Regime, Zone};
use crate::signals::{self, NewsSignal, SignalOpts, Target};
use crate::status::{GateResult, Posture, PostureKind};

/// 2.3 ablations. `Strategy` is the live strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    Strategy,
    /// News treated as always above the entry threshold (full conviction), no news exits.
    MomentumOnly,
    /// Momentum treated as always confirming.
    NewsOnly,
}

#[derive(Debug, Clone, Default)]
pub struct AssetInput {
    pub symbol: String,
    /// Verified price used for valuation, stops and weights.
    pub mark: Option<f64>,
    /// Execution price (live: latest trade; backtest: next candle's open).
    pub exec: Option<f64>,
    /// From closed candles only.
    pub momentum: Option<f64>,
    pub atr_pct: Option<f64>,
    /// Stale / invalid input: no entries for this asset.
    pub review: Option<String>,
    /// 5.2 result, present only when the challenge was asked for this asset.
    pub challenge: Option<Result<Challenge, String>>,
}

pub struct Inputs<'a> {
    pub cfg: &'a Config,
    pub now: DateTime<Utc>,
    pub events: &'a [NewsEvent],
    pub assets: Vec<AssetInput>,
    pub paused: bool,
    pub flatten: bool,
    /// 6.2 Jev circuit breaker open: reason.
    pub jev_degraded: Option<String>,
    pub regime: Option<&'a Regime>,
    pub variant: Variant,
    /// Backtests cannot replay the challenge call: treat it as passed and say so.
    pub replay: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDecision {
    pub symbol: String,
    pub mark: Option<f64>,
    pub exec: Option<f64>,
    pub momentum: Option<f64>,
    pub atr_pct: Option<f64>,
    pub news: NewsSignal,
    pub drivers: Vec<String>,
    pub target: Target,
    pub current: f64,
    /// Weight wanted by the signal and sizing, before gates.
    pub wanted: f64,
    /// Final weight after gates and caps.
    pub weight: f64,
    pub gates: Vec<GateResult>,
    pub binding: Option<Binding>,
    pub review: Option<String>,
    pub reason: String,
    /// Stop distance recorded on new buys.
    pub stop_dist: Option<f64>,
    pub stop_triggered: bool,
    /// First gate that turned a wanted entry into no (or less) entry.
    pub blocked_by: Option<String>,
    /// Entry still wanted after every gate except the challenge (ask Jev, then decide again).
    pub needs_challenge: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub assets: Vec<AssetDecision>,
    /// Verified equity (None: some held position has no price).
    pub equity: Option<f64>,
    /// Equity used for weights: verified if available, else positions at entry.
    pub equity_for_weights: f64,
    pub breaker: Breaker,
    pub ceiling: f64,
    pub posture: Posture,
    pub account_review: Option<String>,
}

/// Records gates as they are applied to one asset's weight.
struct Trace<'a> {
    gates: &'a mut Vec<GateResult>,
    w: &'a mut f64,
    current: f64,
    reason: &'a mut Option<String>,
    blocked_by: &'a mut Option<String>,
}

impl Trace<'_> {
    /// Forced exit: the weight goes to zero.
    fn force(&mut self, name: &str, hit: bool, detail: String) {
        self.gates.push(GateResult::new(name, !hit, detail));
        if hit {
            *self.w = 0.0;
        }
    }

    /// Entry gate: caps the weight at what is held, never forces a sale.
    fn entry(&mut self, name: &str, blocked: bool, detail: String) {
        self.gates.push(GateResult::new(name, !blocked, detail));
        if blocked && *self.w > self.current + 1e-12 {
            *self.w = self.current;
            self.blocked_by.get_or_insert_with(|| name.to_string());
            self.reason.get_or_insert_with(|| name.to_string());
        }
    }
}

pub fn stop_distance(cfg: &Config, atr_pct: Option<f64>) -> f64 {
    match (cfg.risk.stop, atr_pct) {
        (StopMode::Atr, Some(a)) if a.is_finite() && a > 0.0 => cfg.risk.stop_atr_mult * a,
        _ => cfg.risk.stop_loss_pct,
    }
}

pub fn decide(state: &State, breaker: &Breaker, inp: &Inputs) -> Decision {
    let cfg = inp.cfg;
    let risk = &cfg.risk;
    let marks: HashMap<String, f64> =
        inp.assets.iter().filter_map(|a| a.mark.map(|m| (a.symbol.clone(), m))).collect();
    let equity = state.equity_verified(&marks);
    let e = equity.unwrap_or_else(|| state.equity(&marks));
    let account_review = equity.is_none().then(|| "held position without a verified price".to_string());
    let opts = SignalOpts::from_config(&cfg.news);
    let zone = inp.regime.map(|r| r.zone).unwrap_or(Zone::Unknown);
    let regime_mode = cfg.regime.mode;

    let mut out = Vec::new();
    for a in &inp.assets {
        let sym = &a.symbol;
        let current = a.mark.map(|p| state.weight(sym, p, e)).unwrap_or(0.0);
        let (ns, drivers) = signals::news_signal_with_drivers(inp.events, sym, inp.now, &opts, 5);

        // Inputs (0.1 / 0.3 / 0.4 / 0.5): anything missing means REVIEW.
        let review = a
            .review
            .clone()
            .or_else(|| a.mark.is_none().then(|| "no verified price".to_string()))
            .or_else(|| a.momentum.is_none().then(|| "no momentum (too few closed candles)".to_string()))
            .or_else(|| {
                (risk.sizing == Sizing::Vol && a.atr_pct.is_none()).then(|| "no ATR for vol sizing".to_string())
            });

        let (sig_news, sig_mom) = match inp.variant {
            Variant::Strategy => (ns, a.momentum),
            Variant::MomentumOnly => (
                NewsSignal { score: 1.0, hack_alert: false, pos: usize::MAX, n_events: 1, ..ns },
                a.momentum,
            ),
            Variant::NewsOnly => (ns, a.momentum.map(|_| 1.0)),
        };
        let target = match (&review, sig_mom) {
            (None, Some(m)) => signals::target(&sig_news, m, risk.max_asset_weight, &cfg.strategy),
            _ => Target::Hold,
        };
        let entry_stop = stop_distance(cfg, a.atr_pct);
        let wanted = match target {
            Target::Hold => current,
            Target::Weight(x) if x <= 0.0 => 0.0,
            Target::Weight(x) => match risk.sizing {
                Sizing::Fixed => x,
                Sizing::Vol => {
                    let conviction = sig_news.score.clamp(0.0, 1.0);
                    (conviction * risk.risk_per_trade / entry_stop).min(risk.max_asset_weight)
                }
            },
        };

        let mut w = wanted;
        let mut gates = Vec::new();
        let mut reason: Option<String> = None;
        let mut blocked_by: Option<String> = None;

        // Forced exits (original behaviour: these go flat).
        let mut t = Trace { gates: &mut gates, w: &mut w, current, reason: &mut reason, blocked_by: &mut blocked_by };
        t.force("flatten", inp.flatten, String::new());
        t.force("kill switch", breaker.flatten, if breaker.flatten { breaker.reason.clone() } else { String::new() });
        let stop_triggered = a.mark.is_some_and(|p| state.stop_hit(sym, p, risk));
        t.force(
            "stop loss",
            stop_triggered,
            state.stop_dist(sym, risk).map(|d| format!("{:.1}% below entry", d * 100.0)).unwrap_or_default(),
        );
        let in_cd = state.in_cooldown(sym, inp.now);
        t.force(
            "asset cooldown",
            in_cd,
            if in_cd { state.cooldown_until.get(sym).map(|t| format!("until {}", t.format("%H:%M"))).unwrap_or_default() } else { String::new() },
        );
        if t.reason.is_none() && (inp.flatten || breaker.flatten || stop_triggered || in_cd) {
            *t.reason = Some(
                if inp.flatten { "flatten (dashboard)" } else if breaker.flatten { "kill switch" } else if stop_triggered { "stop loss" } else { "cooldown" }
                    .to_string(),
            );
        }

        // Entry gates: they cap the weight at what is held, never force a sale.
        t.entry("inputs", review.is_some(), review.clone().map(|r| format!("REVIEW: {r}")).unwrap_or_default());
        t.entry("account", account_review.is_some(), account_review.clone().unwrap_or_default());
        t.entry(
            "breaker",
            breaker.blocks_entries() && !breaker.flatten,
            if breaker.blocks_entries() { breaker.reason.clone() } else { String::new() },
        );
        t.entry("jev health", inp.jev_degraded.is_some(), inp.jev_degraded.clone().unwrap_or_default());
        let regime_blocks = match regime_mode {
            RegimeMode::Off => false,
            RegimeMode::Block => matches!(zone, Zone::RiskOff | Zone::Unknown),
            RegimeMode::Ceiling => zone == Zone::Unknown,
        };
        t.entry(
            "regime",
            regime_blocks,
            format!(
                "{} · {}",
                zone.label(),
                inp.regime.and_then(|r| r.score).map(|s| format!("{s:.0}")).unwrap_or_else(|| "—".into())
            ) + if regime_mode == RegimeMode::Off { " · record only" } else { "" },
        );
        let reaction = state.reaction.get(sym).copied();
        let reaction_blocks = cfg.strategy.reaction_gate
            && inp.variant == Variant::Strategy
            && reaction.is_some_and(|(z, n)| n >= cfg.strategy.reaction_min_clusters && z < cfg.strategy.reaction_z);
        t.entry(
            "news reaction",
            reaction_blocks,
            reaction.map(|(z, n)| format!("drift z {z:+.2} over {n} clusters")).unwrap_or_default(),
        );

        // 5.2 challenge: only when an entry is still about to happen.
        let mut needs_challenge = false;
        if cfg.challenge.enabled && inp.variant == Variant::Strategy && *t.w > current + 1e-12 {
            if inp.replay {
                t.gates.push(GateResult::new("challenge", true, "not replayed"));
            } else {
                match &a.challenge {
                    None => {
                        needs_challenge = true;
                        t.entry("challenge", true, "pending".into());
                    }
                    Some(Err(e)) => t.entry("challenge", true, format!("failed: {e}")),
                    Some(Ok(c)) => {
                        let bad = c.worst() > cfg.challenge.threshold;
                        let detail = format!("priced in {:.2} · unverified {:.2} · bear {:.2}", c.priced_in, c.unverified, c.bear);
                        if bad && cfg.challenge.action == ChallengeAction::Halve {
                            t.gates.push(GateResult::new("challenge", false, format!("{detail} · halved")));
                            *t.w = current + (*t.w - current) / 2.0;
                            t.blocked_by.get_or_insert_with(|| "challenge".into());
                        } else {
                            t.entry("challenge", bad, detail);
                        }
                    }
                }
            }
        }
        t.entry("pause", inp.paused, String::new());

        let reason = reason.unwrap_or_else(|| match (&review, target) {
            (Some(r), _) => format!("review: {r}"),
            (None, Target::Hold) => "hold".into(),
            (None, Target::Weight(_)) if ns.hack_alert => "hack alert".into(),
            (None, Target::Weight(_)) => format!(
                "news {:+.2}, momentum {:+.2}%",
                ns.score,
                a.momentum.unwrap_or(0.0) * 100.0
            ),
        });

        out.push(AssetDecision {
            symbol: sym.clone(),
            mark: a.mark,
            exec: a.exec,
            momentum: a.momentum,
            atr_pct: a.atr_pct,
            news: ns,
            drivers,
            target,
            current,
            wanted,
            weight: w,
            gates,
            binding: None,
            review,
            reason,
            stop_dist: (risk.stop == StopMode::Atr).then_some(entry_stop),
            stop_triggered,
            blocked_by,
            needs_challenge,
        });
    }

    // Caps (asset, gross, heat), then the regime ceiling on increases only.
    let mut targets: BTreeMap<String, f64> =
        out.iter().filter(|d| d.mark.is_some()).map(|d| (d.symbol.clone(), d.weight)).collect();
    let stop_dists: HashMap<String, f64> = out
        .iter()
        .map(|d| {
            let held = state.positions.contains_key(&d.symbol) && d.weight <= d.current + 1e-12;
            let s = if held { state.stop_dist(&d.symbol, risk) } else { None };
            (d.symbol.clone(), s.unwrap_or_else(|| stop_distance(cfg, d.atr_pct)))
        })
        .collect();
    let bindings = paper::apply_caps(&mut targets, &stop_dists, risk, risk.max_gross);
    let ceiling = inp.regime.map(|r| r.ceiling(regime_mode, risk.max_gross)).unwrap_or(match regime_mode {
        RegimeMode::Ceiling => 0.0,
        _ => risk.max_gross,
    });
    if ceiling < risk.max_gross {
        let held: f64 = out.iter().map(|d| d.current).sum();
        let room = (ceiling - held).max(0.0);
        let inc: f64 = out.iter().map(|d| (targets.get(&d.symbol).copied().unwrap_or(0.0) - d.current).max(0.0)).sum();
        if inc > room + 1e-12 {
            let k = if inc > 0.0 { room / inc } else { 0.0 };
            for d in &out {
                if let Some(t) = targets.get_mut(&d.symbol)
                    && *t > d.current
                {
                    *t = d.current + (*t - d.current) * k;
                }
            }
            for d in out.iter_mut() {
                if targets.get(&d.symbol).is_some_and(|t| *t > d.current - 1e-12) && d.wanted > d.current {
                    d.gates.push(GateResult::new("regime ceiling", false, format!("{:.0}%", ceiling * 100.0)));
                }
            }
        }
    }
    for d in out.iter_mut() {
        if let Some(t) = targets.get(&d.symbol) {
            if (t - d.weight).abs() > 1e-12 && d.gates.iter().any(|g| g.gate == "regime ceiling") {
                d.binding = Some(Binding::RegimeCeiling);
            } else {
                d.binding = bindings.get(&d.symbol).copied();
            }
            d.weight = *t;
        }
    }

    let review_assets: Vec<String> = out.iter().filter(|d| d.review.is_some()).map(|d| d.symbol.clone()).collect();
    let regime_unknown_gate = regime_mode != RegimeMode::Off && zone == Zone::Unknown;
    let posture = if breaker.blocks_entries() {
        Posture { kind: PostureKind::ExitsOnly, ceiling, reason: breaker.reason.clone(), until: breaker.active_until }
    } else if inp.flatten {
        Posture { kind: PostureKind::ExitsOnly, ceiling, reason: "flatten requested".into(), until: None }
    } else if let Some(r) = &account_review {
        Posture { kind: PostureKind::Review, ceiling, reason: r.clone(), until: None }
    } else if let Some(r) = &inp.jev_degraded {
        Posture { kind: PostureKind::Review, ceiling, reason: format!("Jev degraded ({r})"), until: None }
    } else if regime_unknown_gate {
        Posture { kind: PostureKind::Review, ceiling, reason: "regime unknown".into(), until: None }
    } else if inp.paused {
        Posture { kind: PostureKind::ExitsOnly, ceiling, reason: "paused".into(), until: None }
    } else if !review_assets.is_empty() && review_assets.len() == out.len() {
        Posture { kind: PostureKind::Review, ceiling, reason: "all assets in review".into(), until: None }
    } else {
        let reason = if review_assets.is_empty() { String::new() } else { format!("review: {}", review_assets.join(", ")) };
        Posture { kind: PostureKind::EntriesAllowed, ceiling, reason, until: None }
    };

    Decision { assets: out, equity, equity_for_weights: e, breaker: breaker.clone(), ceiling, posture, account_review }
}

/// Shared by paper and live execution: cooldowns after stops, then the assets to
/// trade in order (sells first, to free cash).
pub fn prepare(state: &mut State, d: &Decision, cfg: &Config, now: DateTime<Utc>) -> Vec<usize> {
    for a in &d.assets {
        if a.stop_triggered {
            let until = now + Duration::seconds((cfg.risk.cooldown_hours * 3600.0) as i64);
            state.cooldown_until.insert(a.symbol.clone(), until);
        }
    }
    let mut order: Vec<usize> = (0..d.assets.len()).filter(|i| d.assets[*i].exec.is_some()).collect();
    order.sort_by(|&i, &j| {
        let di = d.assets[i].weight - d.assets[i].current;
        let dj = d.assets[j].weight - d.assets[j].current;
        di.total_cmp(&dj)
    });
    order
}

/// Stop distance to record on a fill: the new one on adds, the held one on reductions.
pub fn stop_for(state: &State, a: &AssetDecision) -> Option<f64> {
    if a.weight > a.current { a.stop_dist } else { state.positions.get(&a.symbol).and_then(|p| p.stop_dist) }
}

/// Applies a decision to the paper ledger.
pub fn execute(state: &mut State, d: &mut Decision, cfg: &Config, now: DateTime<Utc>) -> Vec<Fill> {
    let e = d.equity_for_weights;
    let order = prepare(state, d, cfg, now);
    let mut fills = Vec::new();
    for i in order {
        let a = &mut d.assets[i];
        let Some(px) = a.exec else { continue };
        let stop = stop_for(state, a);
        match state.rebalance(&a.symbol, a.weight, px, e, &cfg.paper, &cfg.risk, &a.reason, now, stop) {
            Ok(mut f) => {
                f.events = a.drivers.clone();
                f.gates = a.gates.clone();
                f.binding = a.binding.map(|b| format!("{b:?}"));
                fills.push(f);
            }
            Err(skip) => {
                if (a.weight - a.current).abs() > 1e-9 {
                    a.binding = Some(Binding::from_skip(skip));
                }
            }
        }
    }
    fills
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::breaker;

    pub fn cfg() -> Config {
        let mut c: Config = toml::from_str(include_str!("../config.toml")).unwrap();
        c.data_dir = std::env::temp_dir();
        c
    }

    fn ev(asset: &str, sentiment: f64, now: DateTime<Utc>) -> NewsEvent {
        NewsEvent {
            id: format!("{asset}{sentiment}"),
            title: "t".into(),
            source: "s".into(),
            published: now,
            analyzed_at: now,
            asset: asset.into(),
            asset_conf: 1.0,
            event: "macro".into(),
            event_conf: 0.9,
            sentiment,
            sentiment_conf: 1.0,
            new_info: 1.0,
            model: "jev".into(),
            first_seen_at: Some(now),
            cluster_id: None,
            tokens_in: None,
            tokens_out: None,
        }
    }

    fn input(sym: &str, mom: Option<f64>) -> AssetInput {
        AssetInput { symbol: sym.into(), mark: Some(100.0), exec: Some(100.0), momentum: mom, atr_pct: Some(0.01), ..Default::default() }
    }

    fn inputs<'a>(c: &'a Config, events: &'a [NewsEvent], assets: Vec<AssetInput>, now: DateTime<Utc>) -> Inputs<'a> {
        Inputs { cfg: c, now, events, assets, paused: false, flatten: false, jev_degraded: None, regime: None, variant: Variant::Strategy, replay: false }
    }

    #[test]
    fn enters_on_news_and_momentum_with_full_trace() {
        let c = cfg();
        let now = Utc::now();
        let evs = [ev("BTC", 1.0, now)];
        let s = State::new(10_000.0);
        let d = decide(&s, &Breaker::allowed(), &inputs(&c, &evs, vec![input("BTC", Some(0.01))], now));
        let a = &d.assets[0];
        assert!(a.weight > 0.0 && a.blocked_by.is_none());
        assert!(a.gates.len() >= 10, "every gate is recorded");
        assert_eq!(a.binding, Some(Binding::Signal));
        assert_eq!(d.posture.kind, PostureKind::EntriesAllowed);
    }

    #[test]
    fn missing_momentum_is_review_not_zero() {
        let c = cfg();
        let now = Utc::now();
        let evs = [ev("BTC", 1.0, now)];
        let s = State::new(10_000.0);
        let d = decide(&s, &Breaker::allowed(), &inputs(&c, &evs, vec![input("BTC", None)], now));
        let a = &d.assets[0];
        assert_eq!(a.target, Target::Hold);
        assert!(a.review.as_deref().unwrap().contains("momentum"));
        assert_eq!(a.weight, 0.0);
    }

    #[test]
    fn missing_price_on_held_position_reviews_account() {
        let c = cfg();
        let now = Utc::now();
        let evs = [ev("ETH", 1.0, now)];
        let mut s = State::new(10_000.0);
        s.rebalance("BTC", 0.2, 100.0, 10_000.0, &c.paper, &c.risk, "t", now, None).unwrap();
        let btc = AssetInput { symbol: "BTC".into(), ..Default::default() };
        let d = decide(&s, &Breaker::allowed(), &inputs(&c, &evs, vec![btc, input("ETH", Some(0.01))], now));
        assert!(d.equity.is_none());
        assert_eq!(d.posture.kind, PostureKind::Review);
        let eth = &d.assets[1];
        assert_eq!(eth.weight, 0.0);
        assert_eq!(eth.blocked_by.as_deref(), Some("account"));
    }

    #[test]
    fn pause_and_breaker_block_entries_but_not_exits() {
        let c = cfg();
        let now = Utc::now();
        let evs = [ev("BTC", 1.0, now)];
        let s = State::new(10_000.0);
        let mut i = inputs(&c, &evs, vec![input("BTC", Some(0.01))], now);
        i.paused = true;
        let d = decide(&s, &Breaker::allowed(), &i);
        assert_eq!(d.assets[0].weight, 0.0);
        assert_eq!(d.assets[0].blocked_by.as_deref(), Some("pause"));
        let b = Breaker { state: breaker::BreakerState::Cooldown, reason: "losing streak".into(), active_until: None, flatten: false };
        i.paused = false;
        let d = decide(&s, &b, &i);
        assert_eq!(d.assets[0].blocked_by.as_deref(), Some("breaker"));
        assert_eq!(d.posture.kind, PostureKind::ExitsOnly);
    }

    #[test]
    fn stop_sets_cooldown_and_exits() {
        let c = cfg();
        let now = Utc::now();
        let mut s = State::new(10_000.0);
        s.rebalance("BTC", 0.2, 100.0, 10_000.0, &c.paper, &c.risk, "t", now, None).unwrap();
        let mut a = input("BTC", Some(0.01));
        a.mark = Some(90.0);
        a.exec = Some(90.0);
        let mut d = decide(&s, &Breaker::allowed(), &inputs(&c, &[], vec![a], now));
        assert!(d.assets[0].stop_triggered && d.assets[0].weight == 0.0);
        let fills = execute(&mut s, &mut d, &c, now);
        assert_eq!(fills.len(), 1);
        assert!(s.in_cooldown("BTC", now));
        assert!(fills[0].realized_pnl.unwrap() < 0.0);
    }

    #[test]
    fn vol_sizing_gives_volatile_assets_less() {
        let mut c = cfg();
        c.risk.sizing = Sizing::Vol;
        c.risk.stop = StopMode::Atr;
        let now = Utc::now();
        let evs = [ev("BTC", 1.0, now), ev("SOL", 1.0, now)];
        let s = State::new(10_000.0);
        let mut sol = input("SOL", Some(0.01));
        sol.atr_pct = Some(0.03);
        let mut btc = input("BTC", Some(0.01));
        btc.atr_pct = Some(0.005);
        let d = decide(&s, &Breaker::allowed(), &inputs(&c, &evs, vec![btc, sol], now));
        assert!(d.assets[1].weight < d.assets[0].weight);
        assert!(d.assets[1].stop_dist.unwrap() > d.assets[0].stop_dist.unwrap());
    }

    #[test]
    fn challenge_requested_then_applied() {
        let mut c = cfg();
        c.challenge.enabled = true;
        let now = Utc::now();
        let evs = [ev("BTC", 1.0, now)];
        let s = State::new(10_000.0);
        let mut i = inputs(&c, &evs, vec![input("BTC", Some(0.01))], now);
        let d = decide(&s, &Breaker::allowed(), &i);
        assert!(d.assets[0].needs_challenge && d.assets[0].weight == 0.0, "fails closed until asked");
        i.assets[0].challenge = Some(Ok(Challenge { priced_in: 0.9, unverified: 0.1, bear: 0.2 }));
        let d = decide(&s, &Breaker::allowed(), &i);
        assert_eq!(d.assets[0].blocked_by.as_deref(), Some("challenge"));
        i.assets[0].challenge = Some(Ok(Challenge { priced_in: 0.1, unverified: 0.1, bear: 0.2 }));
        let d = decide(&s, &Breaker::allowed(), &i);
        assert!(d.assets[0].weight > 0.0, "{:?}", d.assets[0].gates);
    }

    #[test]
    fn regime_ceiling_limits_increases_only() {
        let mut c = cfg();
        c.regime.mode = RegimeMode::Ceiling;
        let now = Utc::now();
        let evs = [ev("BTC", 1.0, now)];
        let s = State::new(10_000.0);
        let mut r = Regime::unknown(now);
        r.score = Some(20.0);
        r.zone = Zone::RiskOff;
        let mut i = inputs(&c, &evs, vec![input("BTC", Some(0.01))], now);
        i.regime = Some(&r);
        let d = decide(&s, &Breaker::allowed(), &i);
        assert!(d.assets[0].weight <= 0.6 * 0.33 + 1e-9);
        assert!((d.ceiling - 0.6 * 0.33).abs() < 1e-9);
    }

    #[test]
    fn momentum_only_ignores_news() {
        let c = cfg();
        let now = Utc::now();
        let s = State::new(10_000.0);
        let mut i = inputs(&c, &[], vec![input("BTC", Some(0.01))], now);
        assert_eq!(decide(&s, &Breaker::allowed(), &i).assets[0].weight, 0.0);
        i.variant = Variant::MomentumOnly;
        assert!(decide(&s, &Breaker::allowed(), &i).assets[0].weight > 0.0);
    }
}
