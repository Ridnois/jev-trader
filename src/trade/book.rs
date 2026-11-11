//! One strategy's paper book: the same decisions the simulator makes, taken one bar
//! at a time on live prices.
//!
//! `on_bar` runs at the close of a bar with the plan computed on every bar so far,
//! and acts at once on the live quote (the simulator's "next open"). `on_quote` runs
//! every second for stops. The test at the bottom replays a panel through the book
//! bar by bar and requires the same trades the simulator produces.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::lab::panel::Panel;
use crate::lab::rules;
use crate::lab::sim::SimCfg;
use crate::lab::strategy::Plan;

#[derive(Debug, Clone, Copy)]
pub struct Quote {
    pub bid: f64,
    pub ask: f64,
}

impl Quote {
    pub fn mid(&self) -> f64 {
        (self.bid + self.ask) / 2.0
    }

    pub fn ok(&self) -> bool {
        self.bid > 0.0 && self.ask >= self.bid && self.bid.is_finite() && self.ask.is_finite()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pos {
    pub coin: String,
    /// +1 long, −1 short.
    pub side: i8,
    pub qty: f64,
    pub entry_px: f64,
    pub notional: f64,
    pub entry_fee: f64,
    /// None = no hard stop.
    pub stop_px: Option<f64>,
    /// Open time of the bar this position was opened in (the bar after the signal).
    pub entry_bar_ms: i64,
    /// The time stop: close at the first bar opening at or after this.
    pub deadline_ms: i64,
    pub opened_ms: i64,
    /// Mid when the entry was decided (to measure real slippage against the model).
    pub entry_mid: f64,
    #[serde(default)]
    pub funding: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Closed {
    pub coin: String,
    pub side: i8,
    pub entry_bar_ms: i64,
    pub opened_ms: i64,
    pub closed_ms: i64,
    pub entry_px: f64,
    pub exit_px: f64,
    pub notional: f64,
    pub fees: f64,
    pub funding: f64,
    /// After both fees and funding, dollars.
    pub pnl: f64,
    pub pnl_bps: f64,
    /// time | stop | signal | shutdown
    pub reason: String,
    pub entry_mid: f64,
    pub exit_mid: f64,
}

#[derive(Debug, Clone)]
pub enum Event {
    Opened(Pos),
    Closed(Closed),
    Extended(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Book {
    /// Capital this strategy sizes against (fixed; results are not compounded).
    pub capital: f64,
    /// Closed P&L, fees and funding so far, dollars.
    pub realized: f64,
    pub fees: f64,
    pub funding: f64,
    pub positions: BTreeMap<String, Pos>,
}

impl Book {
    pub fn new(capital: f64) -> Self {
        Self { capital, ..Default::default() }
    }

    pub fn gross(&self) -> f64 {
        self.positions.values().map(|p| p.notional).sum()
    }

    /// Realized plus open positions at their mids (a position without a mid counts at entry).
    pub fn equity(&self, mid: impl Fn(&str) -> Option<f64>) -> f64 {
        self.realized + self.positions.values().map(|p| p.side as f64 * (mid(&p.coin).unwrap_or(p.entry_px) - p.entry_px) * p.qty).sum::<f64>()
    }

    /// Worst case if every stop fills at its price, dollars (positions without a stop are not counted).
    pub fn at_risk(&self) -> f64 {
        self.positions.values().filter_map(|p| p.stop_px.map(|s| (p.side as f64 * (p.entry_px - s) * p.qty).max(0.0))).sum::<f64>() + 0.0
    }

    fn close(&mut self, coin: &str, ref_px: f64, extra_bps: f64, cfg: &SimCfg, now_ms: i64, reason: &str, mid: f64) -> Option<Closed> {
        let p = self.positions.remove(coin)?;
        let side = p.side as f64;
        let fill = rules::exit_fill(ref_px, side, cfg.costs.slippage_bps * cfg.costs.mult + extra_bps);
        let exit_fee = p.qty * fill * cfg.costs.fee();
        self.realized += side * (fill - p.entry_px) * p.qty - exit_fee;
        self.fees += exit_fee;
        let pnl = side * (fill - p.entry_px) * p.qty - p.entry_fee - exit_fee + p.funding;
        Some(Closed { coin: p.coin, side: p.side, entry_bar_ms: p.entry_bar_ms, opened_ms: p.opened_ms, closed_ms: now_ms, entry_px: p.entry_px, exit_px: fill, notional: p.notional, fees: p.entry_fee + exit_fee, funding: p.funding, pnl, pnl_bps: pnl / p.notional * 1e4, reason: reason.into(), entry_mid: p.entry_mid, exit_mid: mid })
    }

    /// Price a market order gets before slippage: the far side of the book.
    fn touch(q: &Quote, buying: bool) -> f64 {
        if buying { q.ask } else { q.bid }
    }

    /// The close of the panel's last bar: signal and time exits, then entries, coin by
    /// coin in panel order (as the simulator does). `quote(c)` is the live book now.
    pub fn on_bar(&mut self, p: &Panel, plan: &Plan, cfg: &SimCfg, quote: &dyn Fn(usize) -> Option<Quote>, now_ms: i64) -> Vec<Event> {
        let mut out = Vec::new();
        if p.n == 0 {
            return out;
        }
        let i = p.n - 1;
        let pol = plan.policy;
        let next_open = p.time(i) + p.interval_ms;
        // Exits first, for every coin, then entries: the same order as the simulator.
        for c in 0..p.coins.len() {
            let coin = &p.coins[c];
            let Some(q) = quote(c).filter(Quote::ok) else { continue };
            if let Some((side, deadline)) = self.positions.get(coin).map(|x| (x.side, x.deadline_ms)) {
                if !plan.exit.is_empty() && plan.exit[c][i] == side {
                    out.extend(self.close(coin, Self::touch(&q, side < 0), 0.0, cfg, now_ms, "signal", q.mid()).map(Event::Closed));
                } else if next_open >= deadline {
                    if pol.extend && plan.entry[c][i] == side {
                        if let Some(x) = self.positions.get_mut(coin) {
                            x.deadline_ms = next_open + pol.hold_bars as i64 * p.interval_ms;
                        }
                        out.push(Event::Extended(coin.clone()));
                    } else {
                        out.extend(self.close(coin, Self::touch(&q, side < 0), 0.0, cfg, now_ms, "time", q.mid()).map(Event::Closed));
                    }
                }
            }
        }
        for c in 0..p.coins.len() {
            let coin = &p.coins[c];
            let Some(q) = quote(c).filter(Quote::ok) else { continue };
            let sig = plan.entry[c][i];
            if sig == 0 || self.positions.contains_key(coin) || !p.active[c][i] || (pol.max_positions > 0 && self.positions.len() >= pol.max_positions) {
                continue;
            }
            let usd = rules::size_usd(&cfg.sizing, self.capital, p.dvol[c][i], self.gross());
            if usd <= 0.0 {
                continue;
            }
            let side = sig as f64;
            let fill = rules::entry_fill(Self::touch(&q, sig > 0), side, cfg.costs.slippage_bps * cfg.costs.mult);
            let entry_fee = usd * cfg.costs.fee();
            self.realized -= entry_fee;
            self.fees += entry_fee;
            let pos = Pos {
                coin: coin.clone(),
                side: sig,
                qty: usd / fill,
                entry_px: fill,
                notional: usd,
                entry_fee,
                stop_px: (pol.stop_bps > 0.0).then(|| rules::stop_px(fill, side, pol.stop_bps)),
                entry_bar_ms: next_open,
                deadline_ms: next_open + pol.hold_bars as i64 * p.interval_ms,
                opened_ms: now_ms,
                entry_mid: q.mid(),
                funding: 0.0,
            };
            self.positions.insert(coin.clone(), pos.clone());
            out.push(Event::Opened(pos));
        }
        out
    }

    /// Live stop: the side we would sell into (or buy from) has reached the stop.
    pub fn on_quote(&mut self, coin: &str, q: Quote, cfg: &SimCfg, now_ms: i64) -> Option<Closed> {
        let p = self.positions.get(coin)?;
        let stop = p.stop_px?;
        let hit = if p.side > 0 { q.bid <= stop } else { q.ask >= stop };
        if !hit || !q.ok() {
            return None;
        }
        let buying = p.side < 0;
        self.close(coin, Self::touch(&q, buying), 0.0, cfg, now_ms, "stop", q.mid())
    }

    /// Bar-replay stop, with the simulator's rules: filled at the stop, or at the open
    /// when price gapped through it, plus the model's stop slippage and half spread.
    #[cfg(test)]
    pub fn on_bar_range(&mut self, coin: &str, open: f64, low: f64, high: f64, cfg: &SimCfg, now_ms: i64) -> Option<Closed> {
        let p = self.positions.get(coin)?;
        let stop = p.stop_px?;
        let ref_px = if p.side > 0 { (low <= stop).then(|| open.min(stop))? } else { (high >= stop).then(|| open.max(stop))? };
        let extra = cfg.costs.side_bps(coin) - cfg.costs.slippage_bps * cfg.costs.mult + cfg.costs.stop_extra_bps();
        self.close(coin, ref_px, extra, cfg, now_ms, "stop", ref_px)
    }

    /// Funding over `dt_h` hours at `rate_1h` (fraction per hour; positive = longs pay).
    pub fn accrue_funding(&mut self, coin: &str, rate_1h: f64, mid: f64, dt_h: f64) {
        if let Some(p) = self.positions.get_mut(coin) {
            let amt = -(p.side as f64) * rate_1h * p.qty * mid * dt_h;
            if amt.is_finite() {
                p.funding += amt;
                self.realized += amt;
                self.funding += amt;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lab::panel::testing::*;
    use crate::lab::rules::{Costs, Sizing};
    use crate::lab::sim::{Reason, simulate};
    use crate::lab::strategy::{Aux, Spec, plan};

    /// Feeds `full` to a book one bar at a time: at the close of bar i the book sees
    /// only bars 0..=i and a quote built from bar i+1's open; then bar i+1's range is
    /// checked for stops. Returns its closed trades.
    fn replay(full: &Panel, spec: &Spec, cfg: &SimCfg, from: usize) -> Vec<Closed> {
        let mut book = Book::new(cfg.capital);
        let mut closed = Vec::new();
        let half = |coin: &str| (cfg.costs.side_bps(coin) - cfg.costs.slippage_bps) / 1e4;
        for i in from..full.n - 1 {
            let mut head = full.head(i + 1);
            head.rederive(NO_RULE);
            let pl = plan(spec, &head, &Aux::default()).unwrap();
            let now = full.time(i + 1);
            let quote = |c: usize| {
                let o = full.open[c][i + 1];
                o.is_finite().then(|| Quote { bid: o * (1.0 - half(&full.coins[c])), ask: o * (1.0 + half(&full.coins[c])) })
            };
            for ev in book.on_bar(&head, &pl, cfg, &quote, now) {
                if let Event::Closed(t) = ev {
                    closed.push(t);
                }
            }
            for c in 0..full.coins.len() {
                closed.extend(book.on_bar_range(&full.coins[c], full.open[c][i + 1], full.low[c][i + 1], full.high[c][i + 1], cfg, now));
            }
        }
        closed
    }

    fn agree(spec: &Spec, seed: u64) {
        let full = random_panel(seed, 5, 16, "1h");
        let mut cfg = SimCfg::new(10_000.0, Costs::default(), Sizing::default());
        cfg.costs.funding = false;
        let sim = simulate(&full, &plan(spec, &full, &Aux::default()).unwrap(), &cfg);
        // The simulator closes what is still open at the end; the book leaves it open.
        let want: Vec<_> = sim.trades.iter().filter(|t| t.reason != Reason::End).collect();
        let got = replay(&full, spec, &cfg, 0);
        assert!(want.len() > 15, "{}: only {} trades to compare", spec.id(), want.len());
        assert_eq!(got.len(), want.len(), "{}: trade count", spec.id());
        let mut got = got;
        got.sort_by(|a, b| (a.closed_ms, &a.coin).cmp(&(b.closed_ms, &b.coin)));
        let mut want = want;
        want.sort_by(|a, b| (a.exit_i, &full.coins[a.coin]).cmp(&(b.exit_i, &full.coins[b.coin])));
        for (g, w) in got.iter().zip(&want) {
            assert_eq!((g.coin.as_str(), g.side, g.entry_bar_ms, g.closed_ms), (full.coins[w.coin].as_str(), w.side, full.time(w.entry_i), full.time(w.exit_i)), "{}", spec.id());
            let reason = match w.reason {
                Reason::Time => "time",
                Reason::Stop => "stop",
                Reason::Signal => "signal",
                Reason::End => "end",
            };
            assert_eq!(g.reason, reason);
            assert!((g.notional - w.notional).abs() < 1e-6);
            // Quotes compound half-spread and slippage instead of adding them: a 1e-8 difference.
            assert!((g.pnl - w.pnl).abs() < 1e-3, "{}: pnl {} vs {}", spec.id(), g.pnl, w.pnl);
        }
    }

    #[test]
    fn book_and_simulator_make_the_same_trades() {
        agree(&Spec::new("meanrev", &[("n", 48.0), ("z", 1.5), ("hold", 6.0), ("stop", 300.0)]), 11);
        agree(&Spec::new("meanrev", &[("n", 24.0), ("z", 2.0), ("hold", 12.0), ("exit_mean", 1.0)]), 12);
        agree(&Spec::new("trend", &[("n", 24.0), ("out_frac", 0.5), ("stop", 500.0)]), 13);
        agree(&Spec::new("xs_reversal", &[("look", 12.0), ("hold", 6.0), ("k", 2.0)]), 14);
    }

    #[test]
    fn live_stop_fills_at_the_touched_side() {
        let mut book = Book::new(10_000.0);
        let cfg = SimCfg::new(10_000.0, Costs::default(), Sizing::default());
        book.positions.insert("ETH".into(), Pos { coin: "ETH".into(), side: 1, qty: 1.0, entry_px: 100.0, notional: 100.0, entry_fee: 0.045, stop_px: Some(95.0), entry_bar_ms: 0, deadline_ms: i64::MAX, opened_ms: 0, entry_mid: 100.0, funding: 0.0 });
        assert!(book.on_quote("ETH", Quote { bid: 95.5, ask: 95.6 }, &cfg, 1).is_none());
        assert!((book.at_risk() - 5.0).abs() < 1e-9);
        let t = book.on_quote("ETH", Quote { bid: 94.9, ask: 95.0 }, &cfg, 2).unwrap();
        assert_eq!(t.reason, "stop");
        assert!((t.exit_px - 94.9 * (1.0 - 1e-4)).abs() < 1e-9, "sold at the bid less 1 bps slippage");
        assert!(book.positions.is_empty());
        assert!(t.pnl < -5.0);
    }

    #[test]
    fn funding_accrues_against_longs_and_equity_marks_to_mid() {
        let mut book = Book::new(10_000.0);
        book.positions.insert("ETH".into(), Pos { coin: "ETH".into(), side: -1, qty: 2.0, entry_px: 100.0, notional: 200.0, entry_fee: 0.09, stop_px: None, entry_bar_ms: 0, deadline_ms: i64::MAX, opened_ms: 0, entry_mid: 100.0, funding: 0.0 });
        book.accrue_funding("ETH", 0.0001, 100.0, 2.0); // +1 bps/hour for two hours: the short receives
        assert!((book.funding - 0.04).abs() < 1e-12);
        assert!((book.equity(|_| Some(98.0)) - (0.04 + 4.0)).abs() < 1e-9);
        assert_eq!(book.at_risk(), 0.0);
    }
}
