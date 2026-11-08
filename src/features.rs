//! Rolling microstructure features per asset, from best bid/ask updates and trades.
//! Everything is bucketed per second, so memory stays bounded (baseline window only).

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::config::FlowConfig;

#[derive(Debug, Clone, Default)]
struct Sec {
    sec: i64,
    trades: u32,
    buy_usd: f64,
    sell_usd: f64,
    /// Last mid seen in this second (0 = none).
    mid: f64,
    /// Lowest and highest trade price in this second (0 = no trades). Used to decide
    /// whether a resting maker order would have filled.
    lo: f64,
    hi: f64,
}

/// Live market state for one pair.
#[derive(Debug, Clone, Default)]
pub struct AssetMarket {
    pub bid: f64,
    pub ask: f64,
    pub bid_qty: f64,
    pub ask_qty: f64,
    pub last_book_ms: i64,
    pub last_trade_ms: i64,
    secs: VecDeque<Sec>,
    obi_ewma: f64,
    obi_ms: i64,
    first_ms: i64,
    /// Perp context (Hyperliquid activeAssetCtx); zero until the first update.
    pub funding_1h: f64,
    pub open_interest: f64,
    pub mark_px: f64,
    pub oracle_px: f64,
}

/// One evaluation's view of the market.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Features {
    pub ts_ms: i64,
    pub mid: f64,
    pub bid: f64,
    pub ask: f64,
    pub spread_bps: f64,
    /// (microprice − mid) / mid, bps. Positive = book leans toward the ask lifting.
    pub micro_bps: f64,
    /// Smoothed top-of-book imbalance, −1..1.
    pub obi: f64,
    /// Aggressive buy vs sell volume over the flow window, −1..1.
    pub flow: f64,
    /// Trades/sec over the flow window ÷ baseline trades/sec.
    pub activity: f64,
    pub ret_60s_bps: f64,
    pub ret_300s_bps: f64,
    /// Std of 1-second mid log returns over the vol window, bps.
    pub sigma_1s_bps: f64,
    /// σ₁ₛ · √hold: a typical move over the holding period, bps.
    pub exp_move_bps: f64,
    /// Seconds of history available.
    pub history_secs: i64,
    pub book_age_ms: i64,
    /// Hourly funding rate in bps (positive = longs pay shorts).
    #[serde(default)]
    pub funding_bps_1h: f64,
    /// Open interest in dollars at the mark price.
    #[serde(default)]
    pub open_interest_usd: f64,
    /// Perps: (mark − oracle) / oracle, bps.
    #[serde(default)]
    pub premium_bps: f64,
}

impl AssetMarket {
    fn bucket(&mut self, sec: i64) -> &mut Sec {
        if self.secs.back().is_none_or(|b| b.sec < sec) {
            self.secs.push_back(Sec { sec, ..Default::default() });
        }
        // Out-of-order events land in the newest bucket.
        self.secs.back_mut().expect("just pushed")
    }

    fn prune(&mut self, now_sec: i64, keep: i64) {
        while self.secs.front().is_some_and(|b| b.sec < now_sec - keep) {
            self.secs.pop_front();
        }
    }

    pub fn on_book(&mut self, bid: f64, bid_qty: f64, ask: f64, ask_qty: f64, now_ms: i64) {
        if !(bid > 0.0 && ask >= bid && bid_qty >= 0.0 && ask_qty >= 0.0) {
            return;
        }
        if self.first_ms == 0 {
            self.first_ms = now_ms;
        }
        self.bid = bid;
        self.ask = ask;
        self.bid_qty = bid_qty;
        self.ask_qty = ask_qty;
        self.last_book_ms = now_ms;
        self.bucket(now_ms.div_euclid(1000)).mid = (bid + ask) / 2.0;
    }

    /// `buyer_is_maker` = the aggressor sold.
    pub fn on_trade(&mut self, price: f64, qty: f64, buyer_is_maker: bool, now_ms: i64) {
        if !(price > 0.0 && qty > 0.0) {
            return;
        }
        if self.first_ms == 0 {
            self.first_ms = now_ms;
        }
        self.last_trade_ms = now_ms;
        let b = self.bucket(now_ms.div_euclid(1000));
        b.trades += 1;
        b.lo = if b.lo > 0.0 { b.lo.min(price) } else { price };
        b.hi = b.hi.max(price);
        if buyer_is_maker {
            b.sell_usd += price * qty;
        } else {
            b.buy_usd += price * qty;
        }
    }

    pub fn on_ctx(&mut self, funding_1h: f64, open_interest: f64, mark_px: f64, oracle_px: f64) {
        if funding_1h.is_finite() {
            self.funding_1h = funding_1h;
        }
        if open_interest.is_finite() && open_interest >= 0.0 {
            self.open_interest = open_interest;
        }
        if mark_px > 0.0 {
            self.mark_px = mark_px;
        }
        if oracle_px > 0.0 {
            self.oracle_px = oracle_px;
        }
    }

    /// Did any trade print strictly through `px` in a second after `after_sec`?
    /// `below` = a resting buy at `px` (fills when someone sells under it); otherwise
    /// a resting sell. Strictly through, and only in later seconds, so the queue
    /// ahead of us at `px` is assumed to absorb trades at exactly our price.
    pub fn traded_through(&self, px: f64, below: bool, after_sec: i64) -> bool {
        self.secs.iter().rev().take_while(|b| b.sec > after_sec).any(|b| b.lo > 0.0 && if below { b.lo < px } else { b.hi > px })
    }

    /// Mid at the end of second `sec` (forward-filled from earlier seconds).
    pub fn mid_at(&self, sec: i64) -> Option<f64> {
        self.secs.iter().rev().find(|b| b.sec <= sec && b.mid > 0.0).map(|b| b.mid)
    }

    pub fn features(&mut self, now_ms: i64, cfg: &FlowConfig) -> Option<Features> {
        if self.bid <= 0.0 {
            return None;
        }
        let now_sec = now_ms.div_euclid(1000);
        self.prune(now_sec, cfg.baseline_secs.max(cfg.vol_window_secs).max(cfg.markouts.iter().copied().max().unwrap_or(0)) + 5);
        let mid = (self.bid + self.ask) / 2.0;
        let spread_bps = (self.ask - self.bid) / mid * 1e4;
        let depth = self.bid_qty + self.ask_qty;
        let (raw_obi, micro) = if depth > 0.0 {
            ((self.bid_qty - self.ask_qty) / depth, (self.ask * self.bid_qty + self.bid * self.ask_qty) / depth)
        } else {
            (0.0, mid)
        };
        // EWMA with a 10s time constant, independent of the evaluation rate.
        let dt = if self.obi_ms == 0 { 10_000 } else { (now_ms - self.obi_ms).max(0) };
        let a = 1.0 - (-(dt as f64) / 10_000.0).exp();
        self.obi_ewma += a * (raw_obi - self.obi_ewma);
        self.obi_ms = now_ms;

        let fw = cfg.flow_window_secs.max(1);
        let (mut buy, mut sell, mut n_flow, mut n_base) = (0.0, 0.0, 0u64, 0u64);
        for b in &self.secs {
            if b.sec > now_sec - fw {
                buy += b.buy_usd;
                sell += b.sell_usd;
                n_flow += b.trades as u64;
            }
            if b.sec > now_sec - cfg.baseline_secs {
                n_base += b.trades as u64;
            }
        }
        let flow = if buy + sell > 0.0 { (buy - sell) / (buy + sell) } else { 0.0 };
        let history_secs = ((now_ms - self.first_ms) / 1000).max(0);
        let base_span = history_secs.min(cfg.baseline_secs).max(fw) as f64;
        let base_rate = n_base as f64 / base_span;
        let activity = if base_rate > 0.0 { (n_flow as f64 / fw as f64) / base_rate } else { 0.0 };

        let ret = |secs: i64| self.mid_at(now_sec - secs).map(|p| (mid / p - 1.0) * 1e4).unwrap_or(0.0);
        // 1s log returns over the vol window, forward-filled.
        let mut rets = Vec::with_capacity(cfg.vol_window_secs as usize);
        let mut prev: Option<f64> = None;
        let mut it = self.secs.iter().peekable();
        let mut last_mid = self.mid_at(now_sec - cfg.vol_window_secs).unwrap_or(0.0);
        for s in (now_sec - cfg.vol_window_secs + 1)..=now_sec {
            while let Some(b) = it.peek() {
                if b.sec > s {
                    break;
                }
                if b.mid > 0.0 {
                    last_mid = b.mid;
                }
                it.next();
            }
            if last_mid > 0.0 {
                if let Some(p) = prev {
                    rets.push((last_mid / p).ln() * 1e4);
                }
                prev = Some(last_mid);
            }
        }
        let sigma = if rets.len() >= 30 {
            let m = rets.iter().sum::<f64>() / rets.len() as f64;
            (rets.iter().map(|r| (r - m).powi(2)).sum::<f64>() / (rets.len() - 1) as f64).sqrt()
        } else {
            0.0
        };
        Some(Features {
            ts_ms: now_ms,
            mid,
            bid: self.bid,
            ask: self.ask,
            spread_bps,
            micro_bps: (micro / mid - 1.0) * 1e4,
            obi: self.obi_ewma,
            flow,
            activity,
            ret_60s_bps: ret(60),
            ret_300s_bps: ret(300),
            sigma_1s_bps: sigma,
            exp_move_bps: sigma * (cfg.hold_secs.max(1) as f64).sqrt(),
            history_secs,
            book_age_ms: now_ms - self.last_book_ms,
            funding_bps_1h: self.funding_1h * 1e4,
            open_interest_usd: self.open_interest * if self.mark_px > 0.0 { self.mark_px } else { mid },
            premium_bps: if self.oracle_px > 0.0 && self.mark_px > 0.0 { (self.mark_px / self.oracle_px - 1.0) * 1e4 } else { 0.0 },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_activity_and_returns() {
        let cfg = FlowConfig { baseline_secs: 600, vol_window_secs: 120, ..Default::default() };
        let mut m = AssetMarket::default();
        let t0 = 1_000_000_000i64;
        // 10 quiet minutes: one trade per second, balanced, flat price with tiny noise.
        for s in 0..600 {
            let p = 100.0 + if s % 2 == 0 { 0.001 } else { -0.001 };
            m.on_book(p - 0.005, 1.0, p + 0.005, 1.0, t0 + s * 1000);
            m.on_trade(p, 1.0, s % 2 == 0, t0 + s * 1000 + 500);
        }
        // 30 busy seconds: 4 buy trades per second, price drifting up, bids heavy.
        for s in 600..630 {
            let p = 100.0 + (s - 600) as f64 * 0.01;
            m.on_book(p - 0.005, 5.0, p + 0.005, 1.0, t0 + s * 1000);
            for k in 0..4 {
                m.on_trade(p, 1.0, false, t0 + s * 1000 + 100 * k);
            }
        }
        let f = m.features(t0 + 629_900, &cfg).unwrap();
        assert!(f.flow > 0.9, "flow {}", f.flow);
        assert!(f.activity > 2.5, "activity {}", f.activity);
        assert!(f.ret_60s_bps > 20.0, "ret {}", f.ret_60s_bps);
        assert!(f.micro_bps > 0.0 && f.obi > 0.0);
        assert!(f.sigma_1s_bps > 0.0 && f.exp_move_bps > f.sigma_1s_bps);
    }

    #[test]
    fn maker_fill_needs_a_trade_through_after_posting() {
        let mut m = AssetMarket::default();
        let t0 = 2_000_000_000i64;
        m.on_book(99.99, 1.0, 100.01, 1.0, t0);
        m.on_trade(99.98, 1.0, true, t0 + 100); // same second as the post: ignored
        assert!(!m.traded_through(99.99, true, t0 / 1000));
        m.on_trade(99.99, 1.0, true, t0 + 1100); // at our price: queue ahead absorbs it
        assert!(!m.traded_through(99.99, true, t0 / 1000));
        m.on_trade(99.985, 1.0, true, t0 + 2100); // through it: filled
        assert!(m.traded_through(99.99, true, t0 / 1000));
        assert!(!m.traded_through(100.01, false, t0 / 1000), "no buyer paid above the ask");
        m.on_trade(100.02, 1.0, false, t0 + 3100);
        assert!(m.traded_through(100.01, false, t0 / 1000));
    }

    #[test]
    fn ignores_bad_quotes() {
        let mut m = AssetMarket::default();
        m.on_book(101.0, 1.0, 100.0, 1.0, 1);
        assert!(m.features(2, &FlowConfig::default()).is_none());
    }
}
