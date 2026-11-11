//! Synthetic panels with a known answer, for checking the gate itself.
//!
//! `edge = 0` is a market with nothing to find: correlated random walks with
//! volatility clustering and fat tails. Any rule's expected return is zero before
//! costs, so every promotion on such panels is a false positive. `edge > 0` mixes in
//! a mean-reverting component a fade rule can really earn from, to measure power.

use super::panel::{DAY_MS, Panel, UniverseRule};
use crate::market::{Candle, interval_ms};
use crate::stats::Rng;

pub fn panel(seed: u64, coins: usize, days: usize, interval: &str, edge: f64) -> Panel {
    let ims = interval_ms(interval).unwrap_or(3_600_000);
    let bpd = (DAY_MS / ims) as usize;
    let n = days * bpd;
    let mut rng = Rng::new(seed.wrapping_mul(0x9E37_79B9).wrapping_add(17));
    // About 4.5% a day per coin, 60% of the variance shared.
    let sigma = 0.045 / (bpd as f64).sqrt();
    // Fat tails: one shock in fifty is three times the size.
    let shock = |rng: &mut Rng| rng.normal() * if rng.below(50) == 0 { 3.0 } else { 1.0 };
    // Slow log-volatility cycle shared by the market.
    let mut lv = 0.0f64;
    let mut vol = Vec::with_capacity(n);
    let mut market = Vec::with_capacity(n);
    for _ in 0..n {
        lv = 0.995 * lv + 0.05 * rng.normal();
        vol.push(lv.exp().clamp(0.3, 4.0));
        market.push(shock(&mut rng));
    }
    let edge = edge.clamp(0.0, 1.0);
    let kappa = 0.04; // the planted stretch halves in about 17 bars
    let series = (0..coins).map(|c| {
        let mut walk = 0.0f64; // log price, random-walk part
        let mut ou = 0.0f64; // log price, mean-reverting part
        let base = 20.0 * (c + 1) as f64;
        let mut prev = base;
        let candles = (0..n).map(|i| {
            let s = sigma * vol[i];
            walk += s * (0.77 * market[i] + 0.64 * (1.0 - edge).sqrt() * shock(&mut rng));
            ou += -kappa * ou + s * 0.64 * edge.sqrt() * 3.0 * shock(&mut rng);
            let close = base * (walk + ou).exp();
            let open = prev;
            let wig = (rng.normal().abs() * s * 0.6).exp();
            prev = close;
            Candle { open_time: i as i64 * ims, open, high: open.max(close) * wig, low: open.min(close) / wig, close, volume: 5e7 / close / bpd as f64, close_time: (i as i64 + 1) * ims - 1 }
        }).collect();
        (format!("S{c}"), candles)
    }).collect();
    Panel::from_candles("synthetic", interval, series, Vec::new(), UniverseRule { min_dollar_volume: 0.0, min_history_days: 0 }).expect("synthetic panel")
}

#[cfg(test)]
mod tests {
    use super::super::rules::{Costs, Sizing};
    use super::super::sim::{SimCfg, simulate};
    use super::super::stat;
    use super::super::strategy::{Aux, Spec, plan};
    use super::*;

    #[test]
    fn no_edge_panels_lose_costs_and_planted_edges_pay() {
        let spec = Spec::new("meanrev", &[("n", 48.0), ("z", 2.0), ("hold", 12.0)]);
        let cfg = SimCfg::new(10_000.0, Costs::default(), Sizing::default());
        let mean_of = |edge: f64| {
            let runs: Vec<f64> = (0..6).map(|s| {
                let p = panel(s, 6, 150, "1h", edge);
                stat::mean(&simulate(&p, &plan(&spec, &p, &Aux::default()).unwrap(), &cfg).daily)
            }).collect();
            stat::mean(&runs)
        };
        let (flat, planted) = (mean_of(0.0), mean_of(0.6));
        assert!(flat < 0.0, "fading a random walk should lose its costs, got {flat}");
        assert!(planted > 0.0 && planted > flat + 0.0005, "planted edge {planted} vs none {flat}");
    }

    #[test]
    fn panels_look_like_markets() {
        let p = panel(3, 4, 120, "1h", 0.0);
        assert_eq!(p.n, 120 * 24);
        let v = p.dvol[0][p.n - 1];
        assert!(v > 0.005 && v < 0.3, "daily vol {v}");
        assert!(p.active.iter().all(|a| a[p.n - 1]));
        assert!((0..p.n).all(|i| p.high[1][i] >= p.low[1][i] && p.high[1][i] >= p.close[1][i]));
    }
}
