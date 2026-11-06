//! Paper-trading account: persisted state, fills with fees/slippage, hard risk rules.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{PaperConfig, RiskConfig};
use crate::news::NewsEvent;
use crate::status::GateResult;

const MAX_SEEN: usize = 5000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub qty: f64,
    pub entry: f64,
    /// Stop distance (fraction below entry) fixed when the position was opened or
    /// last added to. `None` = the configured `stop_loss_pct`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_dist: Option<f64>,
    /// Realized P&L of partial sells so far in this round trip.
    #[serde(default)]
    pub realized: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened_at: Option<DateTime<Utc>>,
}

/// A headline whose Jev analysis failed and will be retried (6.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pending {
    pub first_seen_at: DateTime<Utc>,
    pub attempts: u32,
    pub next_try_at: DateTime<Utc>,
    pub last_error: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenDay {
    pub day: Option<NaiveDate>,
    pub calls: u64,
    pub input: u64,
    pub output: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub cash: f64,
    pub positions: BTreeMap<String, Position>,
    pub day: NaiveDate,
    pub day_start_equity: f64,
    pub halted_on: Option<NaiveDate>,
    pub cooldown_until: BTreeMap<String, DateTime<Utc>>,
    pub seen: VecDeque<String>,
    pub events: Vec<NewsEvent>,
    /// Last dashboard flatten request the bot has executed.
    #[serde(default)]
    pub flatten_handled: Option<DateTime<Utc>>,
    // --- 3.4 circuit breaker (all UTC) ---
    #[serde(default)]
    pub week_start: Option<NaiveDate>,
    #[serde(default)]
    pub week_start_equity: Option<f64>,
    #[serde(default)]
    pub month_start: Option<NaiveDate>,
    #[serde(default)]
    pub month_start_equity: Option<f64>,
    #[serde(default)]
    pub halted_week: Option<NaiveDate>,
    #[serde(default)]
    pub halted_month: Option<NaiveDate>,
    #[serde(default)]
    pub loss_streak: u32,
    #[serde(default)]
    pub streak_until: Option<DateTime<Utc>>,
    // --- 6.1 / 6.2 / 6.4 Jev reliability and cost ---
    #[serde(default)]
    pub pending: BTreeMap<String, Pending>,
    /// (ok, failed) Jev calls per recent tick, newest last.
    #[serde(default)]
    pub jev_window: VecDeque<(u32, u32)>,
    #[serde(default)]
    pub tokens: TokenDay,
    /// Direction of the last raw signal per asset, for settlement (1 = long, 0 = flat).
    #[serde(default)]
    pub last_dir: BTreeMap<String, i8>,
    /// 5.3 drift z-score of reactions to positive news, per asset: (z, clusters).
    #[serde(default)]
    pub reaction: BTreeMap<String, (f64, usize)>,
    /// "live" when this ledger mirrors a real exchange account (never mixed with paper).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Live: "hyperliquid" or "hyperliquid-testnet". A live ledger without it was made for Binance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue: Option<String>,
    /// Perp funding received (+) or paid (−) since the ledger started, already in cash.
    #[serde(default)]
    pub funding_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding_at: Option<DateTime<Utc>>,
    /// Capital the ledger started with (live: the allotted USDC; paper: `starting_cash`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starting_cash: Option<f64>,
    /// Live: an order whose outcome could not be confirmed. Blocks entries until cleared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_hold: Option<String>,
    /// 6.5: index over `seen`, rebuilt on load.
    #[serde(skip)]
    seen_index: HashSet<String>,
}

/// What a rebalance wants to do, before anything is booked.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Plan {
    /// Spend this many quote dollars (fees excluded).
    Buy { notional: f64 },
    /// Sell this quantity; `full_exit` closes the position.
    Sell { qty: f64, full_exit: bool },
}

/// Why a rebalance produced no fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    Band,
    MinTrade,
    Cash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fill {
    pub ts: DateTime<Utc>,
    pub asset: String,
    pub side: String,
    pub qty: f64,
    pub price: f64,
    pub notional: f64,
    pub fee: f64,
    pub reason: String,
    /// Ids (links) of the news events that drove this decision.
    #[serde(default)]
    pub events: Vec<String>,
    /// Sell fills: realized P&L vs average entry, after the sell fee.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realized_pnl: Option<f64>,
    /// Full exits: P&L of the whole round trip (partial sells included).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round_trip_pnl: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gates: Vec<GateResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<String>,
    /// "hyperliquid" / "hyperliquid-testnet" for real exchange fills; absent for paper fills.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,
}

impl State {
    pub fn new(cash: f64) -> Self {
        Self {
            cash,
            positions: BTreeMap::new(),
            day: Utc::now().date_naive(),
            day_start_equity: cash,
            halted_on: None,
            cooldown_until: BTreeMap::new(),
            seen: VecDeque::new(),
            events: Vec::new(),
            flatten_handled: None,
            week_start: None,
            week_start_equity: None,
            month_start: None,
            month_start_equity: None,
            halted_week: None,
            halted_month: None,
            loss_streak: 0,
            streak_until: None,
            pending: BTreeMap::new(),
            jev_window: VecDeque::new(),
            tokens: TokenDay::default(),
            last_dir: BTreeMap::new(),
            reaction: BTreeMap::new(),
            mode: None,
            venue: None,
            funding_usd: 0.0,
            funding_at: None,
            starting_cash: Some(cash),
            live_hold: None,
            seen_index: HashSet::new(),
        }
    }

    pub fn load_or_new(path: &Path, cash: f64) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::new(cash));
        }
        let text = std::fs::read_to_string(path)?;
        let mut s: State =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        s.seen_index = s.seen.iter().cloned().collect();
        Ok(s)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// 0.5 / 3.4: corrupt ledger data halts entries.
    pub fn validate(&self) -> Result<()> {
        let fin = |name: &str, x: f64| -> Result<()> {
            if !x.is_finite() {
                bail!("{name} is not finite");
            }
            Ok(())
        };
        fin("cash", self.cash)?;
        if self.cash < -1e-6 {
            bail!("cash is negative ({:.2})", self.cash);
        }
        fin("day_start_equity", self.day_start_equity)?;
        if self.day_start_equity <= 0.0 {
            bail!("day_start_equity is not positive");
        }
        for v in [self.week_start_equity, self.month_start_equity].into_iter().flatten() {
            fin("period start equity", v)?;
        }
        for (a, p) in &self.positions {
            if !p.qty.is_finite() || p.qty <= 0.0 || !p.entry.is_finite() || p.entry <= 0.0 {
                bail!("position {a} has invalid qty/entry ({}, {})", p.qty, p.entry);
            }
            if p.stop_dist.is_some_and(|d| !d.is_finite() || d <= 0.0) || !p.realized.is_finite() {
                bail!("position {a} has invalid stop/realized");
            }
        }
        let mut ids = HashSet::new();
        for e in &self.events {
            if !ids.insert(e.id.as_str()) {
                bail!("duplicated event {}", e.id);
            }
        }
        Ok(())
    }

    pub fn is_seen(&self, id: &str) -> bool {
        self.seen_index.contains(id)
    }

    pub fn mark_seen(&mut self, id: String) {
        if !self.seen_index.insert(id.clone()) {
            return;
        }
        self.seen.push_back(id);
        while self.seen.len() > MAX_SEEN {
            if let Some(old) = self.seen.pop_front() {
                self.seen_index.remove(&old);
            }
        }
    }

    /// Equity valued only at real prices; `None` if any held position has no price (0.4).
    pub fn equity_verified(&self, prices: &HashMap<String, f64>) -> Option<f64> {
        let mut eq = self.cash;
        for (a, p) in &self.positions {
            eq += p.qty * prices.get(a).copied().filter(|x| x.is_finite() && *x > 0.0)?;
        }
        Some(eq)
    }

    /// Display-only equity: positions without a price are valued at entry.
    /// Never feed this to the kill switch or to sizing of entries.
    pub fn equity(&self, prices: &HashMap<String, f64>) -> f64 {
        self.cash
            + self
                .positions
                .iter()
                .map(|(a, p)| p.qty * prices.get(a).copied().unwrap_or(p.entry))
                .sum::<f64>()
    }

    /// Perps: accrues funding on long positions since the last call, at each coin's
    /// current hourly rate (positive rate = longs pay). Returns the amount booked.
    /// Hyperliquid settles hourly; accruing per tick spreads the same total.
    pub fn accrue_funding(&mut self, rates_1h: &HashMap<String, f64>, prices: &HashMap<String, f64>, now: DateTime<Utc>) -> f64 {
        let Some(last) = self.funding_at.replace(now) else { return 0.0 };
        let dt_h = ((now - last).num_milliseconds().max(0) as f64 / 3_600_000.0).min(2.0);
        let mut total = 0.0;
        for (a, p) in &self.positions {
            if let (Some(r), Some(px)) = (rates_1h.get(a), prices.get(a))
                && r.is_finite()
                && px.is_finite()
            {
                total -= r * p.qty * px * dt_h;
            }
        }
        self.cash += total;
        self.funding_usd += total;
        total
    }

    pub fn weight(&self, asset: &str, price: f64, equity: f64) -> f64 {
        if equity <= 0.0 {
            return 0.0;
        }
        self.positions.get(asset).map(|p| p.qty * price / equity).unwrap_or(0.0)
    }

    pub fn stop_dist(&self, asset: &str, risk: &RiskConfig) -> Option<f64> {
        self.positions.get(asset).map(|p| p.stop_dist.unwrap_or(risk.stop_loss_pct))
    }

    pub fn stop_hit(&self, asset: &str, price: f64, risk: &RiskConfig) -> bool {
        self.positions
            .get(asset)
            .is_some_and(|p| price < p.entry * (1.0 - p.stop_dist.unwrap_or(risk.stop_loss_pct)))
    }

    pub fn in_cooldown(&self, asset: &str, now: DateTime<Utc>) -> bool {
        self.cooldown_until.get(asset).is_some_and(|t| now < *t)
    }

    /// Records a Jev tick result for the circuit breaker (6.2).
    pub fn record_jev(&mut self, ok: u32, failed: u32, keep: usize) {
        self.jev_window.push_back((ok, failed));
        while self.jev_window.len() > keep.max(1) {
            self.jev_window.pop_front();
        }
    }

    /// (failed, total) over the window.
    pub fn jev_failures(&self) -> (u32, u32) {
        self.jev_window.iter().fold((0, 0), |(f, t), (o, x)| (f + x, t + o + x))
    }

    pub fn add_tokens(&mut self, now: DateTime<Utc>, calls: u64, input: u64, output: u64) {
        let today = now.date_naive();
        if self.tokens.day != Some(today) {
            self.tokens = TokenDay { day: Some(today), ..Default::default() };
        }
        self.tokens.calls += calls;
        self.tokens.input += input;
        self.tokens.output += output;
    }

    /// Move `asset` toward `target_w` of `equity`. Full exits always execute;
    /// other changes are skipped inside the rebalance band / minimum size.
    /// `stop_dist` is recorded on buys (None = configured fixed stop).
    /// Paper fills: slippage and `fee_rate` from `PaperConfig`.
    #[allow(clippy::too_many_arguments)]
    pub fn rebalance(
        &mut self,
        asset: &str,
        target_w: f64,
        price: f64,
        equity: f64,
        paper: &PaperConfig,
        risk: &RiskConfig,
        reason: &str,
        now: DateTime<Utc>,
        stop_dist: Option<f64>,
    ) -> std::result::Result<Fill, Skip> {
        match self.plan(asset, target_w, price, equity, paper.fee_rate, risk)? {
            Plan::Buy { notional } => {
                let px = price * (1.0 + paper.slippage_rate);
                let fee = notional * paper.fee_rate;
                Ok(self.book_buy(asset, notional / px, px, notional, notional + fee, fee, reason, now, stop_dist))
            }
            Plan::Sell { qty, .. } => {
                let px = price * (1.0 - paper.slippage_rate);
                let notional = qty * px;
                let fee = notional * paper.fee_rate;
                Ok(self.book_sell(asset, qty, px, notional, notional - fee, fee, reason, now, risk, 1e-6))
            }
        }
    }

    /// Sizes the move toward `target_w` without changing anything.
    pub fn plan(
        &self,
        asset: &str,
        target_w: f64,
        price: f64,
        equity: f64,
        fee_rate: f64,
        risk: &RiskConfig,
    ) -> std::result::Result<Plan, Skip> {
        let cur_qty = self.positions.get(asset).map(|p| p.qty).unwrap_or(0.0);
        let delta = target_w * equity - cur_qty * price;
        let full_exit = target_w <= 0.0 && cur_qty > 0.0;
        if !full_exit {
            if delta.abs() / equity.max(1e-9) < risk.rebalance_band {
                return Err(Skip::Band);
            }
            if delta.abs() < risk.min_trade_usd {
                return Err(Skip::MinTrade);
            }
        }
        if delta > 0.0 {
            let notional = delta.min(self.cash / (1.0 + fee_rate));
            if notional < risk.min_trade_usd {
                return Err(Skip::Cash);
            }
            Ok(Plan::Buy { notional })
        } else {
            let qty = if full_exit { cur_qty } else { (-delta / price).min(cur_qty) };
            Ok(Plan::Sell { qty, full_exit })
        }
    }

    fn blank(asset: &str, side: &str, qty: f64, px: f64, notional: f64, fee: f64, reason: &str, now: DateTime<Utc>) -> Fill {
        Fill {
            ts: now,
            asset: asset.into(),
            side: side.into(),
            qty,
            price: px,
            notional,
            fee,
            reason: reason.into(),
            events: Vec::new(),
            realized_pnl: None,
            round_trip_pnl: None,
            gates: Vec::new(),
            binding: None,
            venue: None,
            order_id: None,
        }
    }

    /// Books a buy: `qty` received (net of any fee taken in the asset) at `px`,
    /// `cash_out` leaves the ledger, `fee` is the fee in quote dollars.
    #[allow(clippy::too_many_arguments)]
    pub fn book_buy(
        &mut self,
        asset: &str,
        qty: f64,
        px: f64,
        notional: f64,
        cash_out: f64,
        fee: f64,
        reason: &str,
        now: DateTime<Utc>,
        stop_dist: Option<f64>,
    ) -> Fill {
        self.cash -= cash_out;
        let pos = self.positions.entry(asset.to_string()).or_insert(Position {
            qty: 0.0,
            entry: 0.0,
            stop_dist: None,
            realized: 0.0,
            opened_at: Some(now),
        });
        pos.entry = (pos.entry * pos.qty + px * qty) / (pos.qty + qty);
        pos.qty += qty;
        pos.realized -= fee;
        pos.stop_dist = stop_dist;
        Self::blank(asset, "buy", qty, px, notional, fee, reason, now)
    }

    /// Books a sell of `qty` at `px`; `cash_in` enters the ledger. The position is
    /// closed when what is left is worth less than `dust_usd`.
    #[allow(clippy::too_many_arguments)]
    pub fn book_sell(
        &mut self,
        asset: &str,
        qty: f64,
        px: f64,
        notional: f64,
        cash_in: f64,
        fee: f64,
        reason: &str,
        now: DateTime<Utc>,
        risk: &RiskConfig,
        dust_usd: f64,
    ) -> Fill {
        self.cash += cash_in;
        let mut fill = Self::blank(asset, "sell", qty, px, notional, fee, reason, now);
        if let Some(pos) = self.positions.get_mut(asset) {
            let realized = qty * (px - pos.entry) - fee;
            fill.realized_pnl = Some(realized);
            pos.realized += realized;
            pos.qty -= qty;
            if pos.qty * px < dust_usd {
                let rt = pos.realized;
                fill.round_trip_pnl = Some(rt);
                self.positions.remove(asset);
                if rt < 0.0 {
                    self.loss_streak += 1;
                } else {
                    self.loss_streak = 0;
                }
                if let Some(n) = risk.losing_streak_n
                    && self.loss_streak >= n
                {
                    let secs = (risk.streak_cooldown_hours * 3600.0) as i64;
                    self.streak_until = Some(now + chrono::Duration::seconds(secs));
                    self.loss_streak = 0;
                }
            }
        }
        fill
    }
}

/// Which limit set an asset's final weight (0.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Binding {
    Signal,
    AssetCap,
    GrossCap,
    HeatCap,
    RegimeCeiling,
    RebalanceBand,
    MinTrade,
    Cash,
}

impl Binding {
    pub fn from_skip(s: Skip) -> Self {
        match s {
            Skip::Band => Binding::RebalanceBand,
            Skip::MinTrade => Binding::MinTrade,
            Skip::Cash => Binding::Cash,
        }
    }
}

/// Clamp each weight to `max_asset_weight`, scale so the total ≤ `max_gross`,
/// then so total heat (Σ weight × stop distance) ≤ the heat cap.
/// Returns the binding constraint per asset.
pub fn apply_caps(
    targets: &mut BTreeMap<String, f64>,
    stop_dists: &HashMap<String, f64>,
    risk: &RiskConfig,
    max_gross: f64,
) -> BTreeMap<String, Binding> {
    let mut binding: BTreeMap<String, Binding> =
        targets.keys().map(|k| (k.clone(), Binding::Signal)).collect();
    for (k, w) in targets.iter_mut() {
        if *w > risk.max_asset_weight {
            binding.insert(k.clone(), Binding::AssetCap);
        }
        *w = w.clamp(0.0, risk.max_asset_weight);
    }
    let gross: f64 = targets.values().sum();
    if gross > max_gross + 1e-12 {
        let k = max_gross / gross;
        for (a, w) in targets.iter_mut() {
            if *w > 0.0 {
                *w *= k;
                binding.insert(a.clone(), Binding::GrossCap);
            }
        }
    }
    let heat: f64 = targets
        .iter()
        .map(|(k, w)| w * stop_dists.get(k).copied().unwrap_or(risk.stop_loss_pct))
        .sum();
    let cap = risk.heat_cap();
    if heat > cap + 1e-12 {
        let k = cap / heat;
        for (a, w) in targets.iter_mut() {
            if *w > 0.0 {
                *w *= k;
                binding.insert(a.clone(), Binding::HeatCap);
            }
        }
    }
    binding
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfgs() -> (PaperConfig, RiskConfig) {
        (
            PaperConfig { starting_cash: 10_000.0, fee_rate: 0.001, slippage_rate: 0.0005 },
            RiskConfig {
                max_asset_weight: 0.25,
                max_gross: 0.6,
                stop_loss_pct: 0.04,
                daily_loss_limit_pct: 0.03,
                cooldown_hours: 6.0,
                min_trade_usd: 10.0,
                rebalance_band: 0.02,
                sizing: Default::default(),
                risk_per_trade: 0.005,
                stop: Default::default(),
                stop_atr_mult: 2.0,
                max_heat: None,
                weekly_loss_limit_pct: None,
                monthly_loss_limit_pct: None,
                losing_streak_n: Some(2),
                streak_cooldown_hours: 24.0,
            },
        )
    }

    #[test]
    fn round_trip_costs_fees() {
        let (p, r) = cfgs();
        let now = Utc::now();
        let mut s = State::new(10_000.0);
        let buy = s.rebalance("BTC", 0.2, 50_000.0, 10_000.0, &p, &r, "t", now, None).unwrap();
        assert_eq!(buy.side, "buy");
        assert!((s.weight("BTC", 50_000.0, 10_000.0) - 0.2).abs() < 0.001);
        s.rebalance("BTC", 0.0, 50_000.0, 10_000.0, &p, &r, "t", now, None).unwrap();
        assert!(s.positions.is_empty());
        assert!(s.cash < 10_000.0 && s.cash > 9_990.0);
    }

    #[test]
    fn band_skips_small_changes() {
        let (p, r) = cfgs();
        let mut s = State::new(10_000.0);
        assert!(s.rebalance("BTC", 0.01, 50_000.0, 10_000.0, &p, &r, "t", Utc::now(), None).err() == Some(Skip::Band));
    }

    #[test]
    fn caps() {
        let (_, r) = cfgs();
        let mut t: BTreeMap<String, f64> =
            [("A".into(), 0.5), ("B".into(), 0.25), ("C".into(), 0.25)].into();
        let b = apply_caps(&mut t, &HashMap::new(), &r, r.max_gross);
        assert!((t.values().sum::<f64>() - 0.6).abs() < 1e-9);
        assert!(t.values().all(|w| *w <= 0.25));
        assert_eq!(b["A"], Binding::GrossCap);
    }

    #[test]
    fn heat_cap_binds_on_wide_stops() {
        let (_, mut r) = cfgs();
        r.max_heat = Some(0.01);
        let mut t: BTreeMap<String, f64> = [("A".into(), 0.2)].into();
        let stops: HashMap<String, f64> = [("A".into(), 0.1)].into();
        let b = apply_caps(&mut t, &stops, &r, r.max_gross);
        assert!((t["A"] - 0.1).abs() < 1e-9);
        assert_eq!(b["A"], Binding::HeatCap);
    }

    #[test]
    fn default_heat_cap_never_binds_with_fixed_stop() {
        let (_, r) = cfgs();
        let mut t: BTreeMap<String, f64> = [("A".into(), 0.25), ("B".into(), 0.25), ("C".into(), 0.1)].into();
        let b = apply_caps(&mut t, &HashMap::new(), &r, r.max_gross);
        assert!(b.values().all(|x| *x != Binding::HeatCap));
    }

    #[test]
    fn realized_pnl_and_streak() {
        let (p, r) = cfgs();
        let now = Utc::now();
        let mut s = State::new(10_000.0);
        for _ in 0..2 {
            s.rebalance("BTC", 0.2, 100.0, 10_000.0, &p, &r, "t", now, None).unwrap();
            let f = s.rebalance("BTC", 0.0, 90.0, 10_000.0, &p, &r, "t", now, None).unwrap();
            assert!(f.realized_pnl.unwrap() < 0.0 && f.round_trip_pnl.unwrap() < 0.0);
        }
        assert!(s.streak_until.is_some_and(|t| t > now), "2 losers → cooldown");
    }

    #[test]
    fn verified_equity_and_validation() {
        let (p, r) = cfgs();
        let mut s = State::new(10_000.0);
        s.rebalance("BTC", 0.2, 100.0, 10_000.0, &p, &r, "t", Utc::now(), None).unwrap();
        assert!(s.equity_verified(&HashMap::new()).is_none());
        assert!(s.equity_verified(&[("BTC".to_string(), 100.0)].into()).is_some());
        assert!(s.validate().is_ok());
        s.cash = f64::NAN;
        assert!(s.validate().is_err());
    }

    #[test]
    fn seen_index() {
        let mut s = State::new(1.0);
        s.mark_seen("a".into());
        s.mark_seen("a".into());
        assert!(s.is_seen("a") && s.seen.len() == 1);
    }

}
