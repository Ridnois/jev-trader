//! The engine's ledger: cash, signed perp positions, open trades and daily
//! counters. Perps at ≤ 1× notional: a long spends its notional, a short receives
//! it, funding accrues into cash. Persisted after every change.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::intel::Review;
use crate::live::Booked;
use crate::status::write_atomic;
use crate::strategy::{Side, TradeKind};

pub const LIVE_MODE: &str = "live";

/// One fill in ledger terms: signed changes to cash and to the coin.
#[derive(Debug, Clone)]
pub struct Exec {
    pub cash: f64,
    pub base: f64,
    pub price: f64,
    pub fee_usd: f64,
    pub order_id: String,
}

impl Exec {
    /// A fill of `sz` at `px` paying `fee_usd`; `buy` adds coin and spends cash.
    pub fn fill(buy: bool, sz: f64, px: f64, fee_usd: f64, order_id: String) -> Self {
        let n = sz * px;
        if buy { Exec { cash: -(n + fee_usd), base: sz, price: px, fee_usd, order_id } } else { Exec { cash: n - fee_usd, base: -sz, price: px, fee_usd, order_id } }
    }

    pub fn from_booked(b: Booked, buy: bool) -> Self {
        Self::fill(buy, b.sz, b.price, b.fee_usd, b.order_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub kind: TradeKind,
    pub side: Side,
    pub qty: f64,
    pub entry_px: f64,
    /// Signed cash and coin changes at entry, kept pro rata as parts close.
    pub entry_cash: f64,
    pub entry_base: f64,
    pub entry_fee: f64,
    pub opened_ms: i64,
    pub tp_px: f64,
    pub sl_px: f64,
    pub init_sl_px: f64,
    /// Best price reached in the trade's favour (bid for longs, ask for shorts).
    pub best_px: f64,
    pub stop_bps: f64,
    pub cost_bps: f64,
    pub trail_r: f64,
    pub deadline_ms: i64,
    pub candidate_id: String,
    /// The headline behind the trade (catalyst) or the bias at entry (flow).
    pub why: String,
    pub size_mult: f64,
    #[serde(default)]
    pub review: Option<Review>,
    pub entry_order: String,
    #[serde(default)]
    pub funding_usd: f64,
    /// An exit was decided but not fully filled yet: keep closing.
    #[serde(default)]
    pub closing: Option<String>,
    /// Live: the resting reduce-only take-profit and the exchange-side backstop stop.
    #[serde(default)]
    pub tp_cloid: Option<String>,
    #[serde(default)]
    pub backstop_cloid: Option<String>,
    /// Paper: when the take-profit was "posted" (it fills on a later trade-through).
    #[serde(default)]
    pub tp_posted_ms: i64,
    #[serde(default)]
    pub last_poll_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub mode: Option<String>,
    pub venue: String,
    pub cash: f64,
    pub starting_cash: f64,
    /// Signed perp position per coin (positive = long).
    #[serde(default)]
    pub held: BTreeMap<String, f64>,
    pub positions: BTreeMap<String, Position>,
    pub day: NaiveDate,
    #[serde(default)]
    pub day_trade_pnl: f64,
    #[serde(default)]
    pub halted_day: Option<NaiveDate>,
    #[serde(default)]
    pub cooldown_until_ms: BTreeMap<String, i64>,
    #[serde(default)]
    pub entries_ms: VecDeque<i64>,
    #[serde(default)]
    pub wins: u32,
    #[serde(default)]
    pub losses: u32,
    #[serde(default)]
    pub realized: f64,
    #[serde(default)]
    pub fees: f64,
    #[serde(default)]
    pub funding: f64,
    #[serde(default)]
    pub live_hold: Option<String>,
    /// Catalyst events already traded or expired (never traded twice).
    #[serde(default)]
    pub used_events: VecDeque<String>,
    /// The dashboard flatten request already acted on.
    #[serde(default)]
    pub flatten_handled: Option<DateTime<Utc>>,
}

impl Ledger {
    pub fn new(cash: f64, live: bool, venue: &str) -> Self {
        Self {
            mode: live.then(|| LIVE_MODE.to_string()),
            venue: venue.into(),
            cash,
            starting_cash: cash,
            held: BTreeMap::new(),
            positions: BTreeMap::new(),
            day: Utc::now().date_naive(),
            day_trade_pnl: 0.0,
            halted_day: None,
            cooldown_until_ms: BTreeMap::new(),
            entries_ms: VecDeque::new(),
            wins: 0,
            losses: 0,
            realized: 0.0,
            fees: 0.0,
            funding: 0.0,
            live_hold: None,
            used_events: VecDeque::new(),
            flatten_handled: None,
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, &serde_json::to_vec_pretty(self)?)
    }

    /// Opens the ledger at `path`, refusing one made for another mode or venue.
    pub fn open(path: &Path, live: bool, venue: &str) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let l: Ledger = serde_json::from_slice(&std::fs::read(path)?).with_context(|| format!("parsing {}", path.display()))?;
        if (l.mode.as_deref() == Some(LIVE_MODE)) != live {
            bail!("{} was created in {} mode; use a separate data_dir", path.display(), if live { "paper" } else { "live" });
        }
        if l.venue != venue {
            bail!("{} was created for {}, not {venue}; use a separate data_dir", path.display(), l.venue);
        }
        Ok(Some(l))
    }

    pub fn held(&self, coin: &str) -> f64 {
        self.held.get(coin).copied().unwrap_or(0.0)
    }

    pub fn capital(&self) -> f64 {
        self.starting_cash
    }

    fn mark(&self, coin: &str, marks: &HashMap<String, f64>) -> f64 {
        marks.get(coin).copied().or_else(|| self.positions.get(coin).map(|p| p.entry_px)).unwrap_or(0.0)
    }

    pub fn equity(&self, marks: &HashMap<String, f64>) -> f64 {
        self.cash + self.held.iter().map(|(c, q)| q * self.mark(c, marks)).sum::<f64>()
    }

    pub fn open_notional(&self, marks: &HashMap<String, f64>) -> f64 {
        self.held.iter().map(|(c, q)| q.abs() * self.mark(c, marks)).sum()
    }

    pub fn apply(&mut self, coin: &str, x: &Exec) {
        self.cash += x.cash;
        self.fees += x.fee_usd;
        *self.held.entry(coin.to_string()).or_default() += x.base;
    }

    pub fn mark_used(&mut self, event_id: &str) {
        if !self.used_events.iter().any(|e| e == event_id) {
            self.used_events.push_back(event_id.to_string());
            while self.used_events.len() > 2000 {
                self.used_events.pop_front();
            }
        }
    }

    pub fn used(&self, event_id: &str) -> bool {
        self.used_events.iter().any(|e| e == event_id)
    }
}

/// Splits the closed fraction off `p`: returns its P&L and shrinks `p` to the rest.
/// `x` is the exit fill (its base has the opposite sign of the position).
pub fn close_part(p: &mut Position, x: &Exec) -> f64 {
    let frac = (x.base.abs() / p.qty).clamp(0.0, 1.0);
    let pnl = frac * p.entry_cash + x.cash + (frac * p.entry_base + x.base) * x.price + frac * p.funding_usd;
    let keep = 1.0 - frac;
    p.qty -= x.base.abs();
    p.entry_cash *= keep;
    p.entry_base *= keep;
    p.entry_fee *= keep;
    p.funding_usd *= keep;
    pnl
}

/// Clears the hold set after an unconfirmed order. Returns whether one was set.
pub fn clear_hold(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let mut l: Ledger = serde_json::from_slice(&std::fs::read(path)?).with_context(|| format!("parsing {}", path.display()))?;
    let had = l.live_hold.take().is_some();
    if had {
        l.save(path)?;
    }
    Ok(had)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(entry: &Exec, side: Side) -> Position {
        Position {
            kind: TradeKind::Catalyst,
            side,
            qty: entry.base.abs(),
            entry_px: entry.price,
            entry_cash: entry.cash,
            entry_base: entry.base,
            entry_fee: entry.fee_usd,
            opened_ms: 0,
            tp_px: 0.0,
            sl_px: 0.0,
            init_sl_px: 0.0,
            best_px: entry.price,
            stop_bps: 0.0,
            cost_bps: 0.0,
            trail_r: 1.0,
            deadline_ms: 0,
            candidate_id: "c".into(),
            why: String::new(),
            size_mult: 1.0,
            review: None,
            entry_order: "e".into(),
            funding_usd: 0.0,
            closing: None,
            tp_cloid: None,
            backstop_cloid: None,
            tp_posted_ms: 0,
            last_poll_ms: 0,
        }
    }

    #[test]
    fn short_and_long_round_trips() {
        let mut l = Ledger::new(1000.0, false, "paper");
        // Short 0.5 at 100 (fee 0.05), buy back at 98 (fee 0.049).
        let e = Exec::fill(false, 0.5, 100.0, 0.05, "e".into());
        l.apply("BTC", &e);
        assert!((l.held("BTC") + 0.5).abs() < 1e-12);
        let marks: HashMap<String, f64> = [("BTC".to_string(), 100.0)].into();
        assert!((l.equity(&marks) - 999.95).abs() < 1e-9, "only the fee is lost at entry");
        let mut p = pos(&e, Side::Short);
        let x = Exec::fill(true, 0.5, 98.0, 0.049, "x".into());
        l.apply("BTC", &x);
        let pnl = close_part(&mut p, &x);
        assert!((pnl - (1.0 - 0.05 - 0.049)).abs() < 1e-9, "{pnl}");
        assert!(l.held("BTC").abs() < 1e-12);
        assert!((l.cash - (1000.0 + pnl)).abs() < 1e-9);
    }

    #[test]
    fn partial_close_keeps_the_rest() {
        let e = Exec::fill(true, 2.0, 100.0, 0.03, "e".into());
        let mut p = pos(&e, Side::Long);
        let x = Exec::fill(false, 1.0, 101.0, 0.05, "x".into());
        let pnl = close_part(&mut p, &x);
        assert!((pnl - (100.95 - 100.015)).abs() < 1e-9, "{pnl}");
        assert!((p.qty - 1.0).abs() < 1e-12 && (p.entry_cash + 100.015).abs() < 1e-9);
    }

    #[test]
    fn refuses_other_mode() {
        let dir = std::env::temp_dir().join(format!("jev-ledger-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("l.json");
        Ledger::new(100.0, false, "hyperliquid-paper").save(&p).unwrap();
        assert!(Ledger::open(&p, true, "hyperliquid").is_err());
        assert!(Ledger::open(&p, false, "hyperliquid-paper").unwrap().is_some());
        std::fs::remove_dir_all(dir).ok();
    }
}
