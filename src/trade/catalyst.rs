//! The Jev catalyst scalper, in record mode. A fresh, concrete, strong headline about
//! a coin is a candidate trade in the direction Jev read. Nothing is ordered: every
//! scored headline with a coin is logged with the live mid and marked out at fixed
//! horizons, net of the round-trip cost, whether it triggered or not.
//!
//! Why record mode: a headline strategy can't be backtested honestly (replaying old
//! news lets the model's memory of what happened leak in), so its only real evidence
//! is collected forward. `trade monitor` summarises it; it earns orders the same way
//! any rule does.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::book::Quote;
use super::intel::NewsEvent;
use crate::bot::append_jsonl;
use crate::lab::CatalystCfg;
use crate::status::GateResult;

/// Is this headline strong and fresh enough to count as a catalyst?
pub fn trigger(e: &NewsEvent, cfg: &CatalystCfg) -> Vec<GateResult> {
    vec![
        GateResult::new("impact", e.impact.abs() >= cfg.min_impact, format!("{:+.2} (needs ±{:.2})", e.impact, cfg.min_impact)),
        GateResult::new("new fact", e.new_info >= cfg.min_new_info, format!("{:.2} (needs {:.2})", e.new_info, cfg.min_new_info)),
        GateResult::new("fresh", e.lag_min <= cfg.max_lag_min, format!("seen {:.0} min after publication (limit {:.0})", e.lag_min, cfg.max_lag_min)),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub ts: chrono::DateTime<chrono::Utc>,
    pub coin: String,
    /// +1 buy, −1 sell: the direction of Jev's impact reading.
    pub side: i8,
    pub title: String,
    pub source: String,
    pub impact: f64,
    pub new_info: f64,
    pub lag_min: f64,
    pub triggered: bool,
    pub gates: Vec<GateResult>,
    pub mid: f64,
    pub cost_bps: f64,
}

/// `ret_bps` is signed in the candidate's favour; `net_bps` subtracts the round trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Markout {
    pub id: String,
    pub ts: chrono::DateTime<chrono::Utc>,
    pub coin: String,
    pub side: i8,
    pub triggered: bool,
    pub h_secs: i64,
    pub ret_bps: f64,
    pub net_bps: f64,
}

struct Pending {
    c: Candidate,
    t0_ms: i64,
    left: Vec<i64>,
}

#[derive(Default)]
pub struct Recorder {
    pending: Vec<Pending>,
}

impl Recorder {
    /// Logs the headline as a candidate if it names a coin we have a live quote for.
    pub fn on_event(&mut self, e: &NewsEvent, quotes: &HashMap<String, Quote>, cfg: &CatalystCfg, cost_bps: f64, now_ms: i64, path: &Path) -> Option<Candidate> {
        let coin = e.coins.first()?;
        let q = quotes.get(coin)?;
        if e.impact == 0.0 {
            return None;
        }
        let gates = trigger(e, cfg);
        let c = Candidate {
            id: e.id.clone(),
            ts: chrono::Utc::now(),
            coin: coin.clone(),
            side: if e.impact > 0.0 { 1 } else { -1 },
            title: e.title.clone(),
            source: e.source.clone(),
            impact: e.impact,
            new_info: e.new_info,
            lag_min: e.lag_min,
            triggered: gates.iter().all(|g| g.passed),
            gates,
            mid: q.mid(),
            cost_bps,
        };
        append_jsonl(path, &c);
        let mut left = cfg.markouts_secs.clone();
        left.sort_unstable();
        self.pending.push(Pending { c: c.clone(), t0_ms: now_ms, left });
        Some(c)
    }

    /// Writes the markouts whose horizon has passed.
    pub fn tick(&mut self, quotes: &HashMap<String, Quote>, now_ms: i64, path: &Path) -> usize {
        let mut written = 0;
        for p in &mut self.pending {
            while p.left.first().is_some_and(|h| now_ms - p.t0_ms >= h * 1000) {
                let h = p.left.remove(0);
                // A horizon missed by more than a minute (the engine was down) is not a markout.
                let Some(q) = quotes.get(&p.c.coin).filter(|_| now_ms - p.t0_ms < h * 1000 + 60_000) else { continue };
                let ret = p.c.side as f64 * (q.mid() / p.c.mid - 1.0) * 1e4;
                append_jsonl(path, &Markout { id: p.c.id.clone(), ts: chrono::Utc::now(), coin: p.c.coin.clone(), side: p.c.side, triggered: p.c.triggered, h_secs: h, ret_bps: ret, net_bps: ret - p.c.cost_bps });
                written += 1;
            }
        }
        self.pending.retain(|p| !p.left.is_empty());
        written
    }

    pub fn waiting(&self) -> usize {
        self.pending.len()
    }
}

/// Mean net markout per horizon, for triggered candidates and for all of them.
/// (horizon secs, triggered n, triggered mean net bps, all n, all mean net bps)
pub fn summary(markouts: &[Markout]) -> Vec<(i64, usize, f64, usize, f64)> {
    let mut hs: Vec<i64> = markouts.iter().map(|m| m.h_secs).collect();
    hs.sort_unstable();
    hs.dedup();
    hs.into_iter().map(|h| {
        let all: Vec<f64> = markouts.iter().filter(|m| m.h_secs == h).map(|m| m.net_bps).collect();
        let trig: Vec<f64> = markouts.iter().filter(|m| m.h_secs == h && m.triggered).map(|m| m.net_bps).collect();
        (h, trig.len(), crate::lab::stat::mean(&trig), all.len(), crate::lab::stat::mean(&all))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::super::intel::sources::Kind;
    use super::*;

    fn event(coin: Option<&str>, impact: f64, new_info: f64, lag: f64) -> NewsEvent {
        NewsEvent { id: format!("e{impact}{lag}"), title: "Exchange lists the coin".into(), source: "binance".into(), kind: Kind::Listing, published: chrono::Utc::now(), first_seen: chrono::Utc::now(), lag_min: lag, coins: coin.iter().map(|c| c.to_string()).collect(), market: false, impact, impact_conf: 0.8, new_info, hack: None, model: "jev-test".into() }
    }

    #[test]
    fn records_every_coin_headline_and_marks_it_out() {
        let dir = std::env::temp_dir().join(format!("jev-catalyst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (cands, marks) = (dir.join("c.jsonl"), dir.join("m.jsonl"));
        let cfg = CatalystCfg { markouts_secs: vec![300, 60], ..CatalystCfg::default() };
        let mut quotes = HashMap::from([("ZEC".to_string(), Quote { bid: 99.9, ask: 100.1 })]);
        let mut r = Recorder::default();
        // Strong, new, fresh: triggered, a buy.
        let c = r.on_event(&event(Some("ZEC"), 0.8, 0.9, 3.0), &quotes, &cfg, 13.0, 0, &cands).unwrap();
        assert!(c.triggered && c.side == 1 && c.mid == 100.0);
        // Weak and stale, negative: recorded as a sell, not triggered.
        let c = r.on_event(&event(Some("ZEC"), -0.2, 0.9, 90.0), &quotes, &cfg, 13.0, 0, &cands).unwrap();
        assert!(!c.triggered && c.side == -1);
        assert_eq!(c.gates.iter().filter(|g| !g.passed).count(), 2);
        // No coin, or no quote for it: nothing to record.
        assert!(r.on_event(&event(None, 0.9, 0.9, 1.0), &quotes, &cfg, 13.0, 0, &cands).is_none());
        assert!(r.on_event(&event(Some("DOGE"), 0.9, 0.9, 1.0), &quotes, &cfg, 13.0, 0, &cands).is_none());
        assert_eq!(r.waiting(), 2);
        // Price rises 1%: +100 bps for the buy, −100 for the sell, each less 13 bps of cost.
        quotes.insert("ZEC".into(), Quote { bid: 100.9, ask: 101.1 });
        assert_eq!(r.tick(&quotes, 59_000, &marks), 0);
        assert_eq!(r.tick(&quotes, 61_000, &marks), 2);
        assert_eq!(r.tick(&quotes, 301_000, &marks), 2);
        assert_eq!(r.waiting(), 0);
        let m: Vec<Markout> = crate::status::read_jsonl(&marks);
        assert_eq!(m.len(), 4);
        assert!((m[0].ret_bps - 100.0).abs() < 1e-6 && (m[0].net_bps - 87.0).abs() < 1e-6 && m[0].h_secs == 60);
        assert!((m[1].net_bps + 113.0).abs() < 1e-6);
        let s = summary(&m);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].0, s[0].1, s[0].3), (60, 1, 2));
        assert!((s[0].2 - 87.0).abs() < 1e-6 && (s[0].4 + 13.0).abs() < 1e-6);
        // A horizon that passed while the engine was down is dropped, not faked.
        r.on_event(&event(Some("ZEC"), 0.8, 0.9, 3.0), &quotes, &cfg, 13.0, 1_000_000, &cands);
        assert_eq!(r.tick(&quotes, 1_000_000 + 3_600_000, &marks), 0);
        assert_eq!(r.waiting(), 0);
        std::fs::remove_dir_all(dir).ok();
    }
}
