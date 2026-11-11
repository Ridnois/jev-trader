//! A panel of bars: every coin on one regular time grid, missing bars as NaN.
//!
//! Rules that read a panel must be causal: the value they produce for bar `i` may use
//! bars `0..=i` only (the simulator acts on it at the open of bar `i + 1`).
//! `strategy::tests::plans_do_not_look_ahead` checks that for every family by cutting
//! the future off and comparing.

use anyhow::{Result, bail};

use super::data::Archive;
use crate::market::{Candle, interval_ms};

pub const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone)]
pub struct Panel {
    pub venue: String,
    pub interval: String,
    pub interval_ms: i64,
    /// Open time of bar 0; bar `i` opens at `t0 + i × interval_ms`.
    pub t0: i64,
    pub n: usize,
    pub coins: Vec<String>,
    /// `[coin][bar]`, NaN where the venue has no bar.
    pub open: Vec<Vec<f64>>,
    pub high: Vec<Vec<f64>>,
    pub low: Vec<Vec<f64>>,
    pub close: Vec<Vec<f64>>,
    /// Base-coin volume.
    pub volume: Vec<Vec<f64>>,
    /// `[coin][bar]`: liquid enough to open a trade, judged on days before this one.
    pub active: Vec<Vec<bool>>,
    /// `[coin]`: funding prints `(time_ms, rate)`, sorted.
    pub funding: Vec<Vec<(i64, f64)>>,
    /// `[coin][bar]`: std of bar log returns over the last 3 days, scaled to a day.
    /// NaN until there is a day of history.
    pub dvol: Vec<Vec<f64>>,
    /// `[bar]`: equal-weight mean log return of active coins over this bar.
    pub basket_ret: Vec<f64>,
}

/// Which coins count as tradable on each day.
#[derive(Debug, Clone, Copy)]
pub struct UniverseRule {
    /// Median daily dollar volume over the trailing 30 days must be at least this.
    pub min_dollar_volume: f64,
    /// Days of history a coin needs before it can be traded.
    pub min_history_days: usize,
}

impl Panel {
    pub fn time(&self, i: usize) -> i64 {
        self.t0 + i as i64 * self.interval_ms
    }

    pub fn bars_per_day(&self) -> usize {
        (DAY_MS / self.interval_ms).max(1) as usize
    }

    /// UTC day number of bar `i`, counted from the panel's first day.
    pub fn day_of(&self, i: usize) -> usize {
        (self.time(i).div_euclid(DAY_MS) - self.t0.div_euclid(DAY_MS)) as usize
    }

    pub fn n_days(&self) -> usize {
        if self.n == 0 { 0 } else { self.day_of(self.n - 1) + 1 }
    }

    /// First bar index at or after `t_ms` (clamped to the panel).
    pub fn index_at(&self, t_ms: i64) -> usize {
        (((t_ms - self.t0) as f64 / self.interval_ms as f64).ceil().max(0.0) as usize).min(self.n)
    }

    pub fn coin_index(&self, coin: &str) -> Option<usize> {
        self.coins.iter().position(|c| c == coin)
    }

    /// Builds a panel from per-coin candles (each sorted, same interval).
    pub fn from_candles(venue: &str, interval: &str, series: Vec<(String, Vec<Candle>)>, funding: Vec<Vec<(i64, f64)>>, rule: UniverseRule) -> Result<Panel> {
        let ims = interval_ms(interval).ok_or_else(|| anyhow::anyhow!("bad interval {interval}"))?;
        let first = series.iter().filter_map(|(_, c)| c.first().map(|k| k.open_time)).min();
        let last = series.iter().filter_map(|(_, c)| c.last().map(|k| k.open_time)).max();
        let (Some(first), Some(last)) = (first, last) else { bail!("no candles for {venue} {interval}: run `jev-trader lab fetch` first") };
        // Start on a UTC day boundary so days and bars line up.
        let t0 = first.div_euclid(DAY_MS) * DAY_MS;
        let n = ((last - t0) / ims + 1) as usize;
        let nan = || vec![f64::NAN; n];
        let mut p = Panel {
            venue: venue.into(),
            interval: interval.into(),
            interval_ms: ims,
            t0,
            n,
            coins: series.iter().map(|(c, _)| c.clone()).collect(),
            open: Vec::new(),
            high: Vec::new(),
            low: Vec::new(),
            close: Vec::new(),
            volume: Vec::new(),
            active: Vec::new(),
            funding,
            dvol: Vec::new(),
            basket_ret: Vec::new(),
        };
        for (_, candles) in &series {
            let (mut o, mut h, mut l, mut c, mut v) = (nan(), nan(), nan(), nan(), nan());
            for k in candles {
                let d = k.open_time - t0;
                if d < 0 || d % ims != 0 {
                    continue; // off-grid bar: ignore rather than misplace
                }
                let i = (d / ims) as usize;
                if i < n {
                    (o[i], h[i], l[i], c[i], v[i]) = (k.open, k.high, k.low, k.close, k.volume);
                }
            }
            p.open.push(o);
            p.high.push(h);
            p.low.push(l);
            p.close.push(c);
            p.volume.push(v);
        }
        while p.funding.len() < p.coins.len() {
            p.funding.push(Vec::new());
        }
        p.derive(rule);
        Ok(p)
    }

    /// Loads `coins` from the archive.
    pub fn load(ar: &Archive, venue: &str, interval: &str, coins: &[String], rule: UniverseRule) -> Result<Panel> {
        let mut series = Vec::new();
        let mut funding = Vec::new();
        for c in coins {
            let candles = ar.candles(venue, interval, c)?;
            if candles.is_empty() {
                continue;
            }
            funding.push(ar.funding(venue, c));
            series.push((c.clone(), candles));
        }
        Panel::from_candles(venue, interval, series, funding, rule)
    }

    /// Recomputes everything derived from prices: the tradable mask, daily volatility
    /// and the basket return. All three are causal.
    #[cfg(test)]
    pub fn rederive(&mut self, rule: UniverseRule) {
        self.derive(rule);
    }

    fn derive(&mut self, rule: UniverseRule) {
        let (n, nc, bpd) = (self.n, self.coins.len(), self.bars_per_day());
        let days = self.n_days();
        // Tradable mask from daily dollar volume of *earlier* days.
        self.active = vec![vec![false; n]; nc];
        for c in 0..nc {
            let mut day_usd = vec![0.0f64; days];
            let mut day_has = vec![false; days];
            for i in 0..n {
                if self.close[c][i].is_finite() {
                    let d = self.day_of(i);
                    day_usd[d] += self.volume[c][i] * self.close[c][i];
                    day_has[d] = true;
                }
            }
            let mut ok_day = vec![false; days];
            for d in 0..days {
                let from = d.saturating_sub(30);
                let mut w: Vec<f64> = (from..d).filter(|k| day_has[*k]).map(|k| day_usd[k]).collect();
                if w.len() < rule.min_history_days.min(30) {
                    continue;
                }
                let med = if w.is_empty() {
                    0.0
                } else {
                    w.sort_by(f64::total_cmp);
                    w[w.len() / 2]
                };
                ok_day[d] = med >= rule.min_dollar_volume;
            }
            for i in 0..n {
                self.active[c][i] = ok_day[self.day_of(i)] && self.close[c][i].is_finite();
            }
        }
        // Daily volatility: rolling std of bar log returns over 3 days.
        let w = 3 * bpd;
        self.dvol = vec![vec![f64::NAN; n]; nc];
        let mut rets = vec![vec![f64::NAN; n]; nc];
        for c in 0..nc {
            let mut prev = f64::NAN;
            for i in 0..n {
                let px = self.close[c][i];
                if px.is_finite() {
                    if prev.is_finite() {
                        rets[c][i] = (px / prev).ln();
                    }
                    prev = px;
                }
            }
            let (mut s, mut q, mut k) = (0.0, 0.0, 0usize);
            for i in 0..n {
                let r = rets[c][i];
                if r.is_finite() {
                    s += r;
                    q += r * r;
                    k += 1;
                }
                if i >= w {
                    let old = rets[c][i - w];
                    if old.is_finite() {
                        s -= old;
                        q -= old * old;
                        k -= 1;
                    }
                }
                if k >= bpd.max(8) {
                    let m = s / k as f64;
                    let var = (q / k as f64 - m * m).max(0.0);
                    self.dvol[c][i] = (var * bpd as f64).sqrt();
                }
            }
        }
        // Basket: equal-weight mean bar return of active coins.
        self.basket_ret = vec![0.0; n];
        for i in 0..n {
            let (mut s, mut k) = (0.0, 0usize);
            for c in 0..nc {
                if self.active[c][i] && rets[c][i].is_finite() {
                    s += rets[c][i];
                    k += 1;
                }
            }
            if k > 0 {
                self.basket_ret[i] = s / k as f64;
            }
        }
    }

    /// The first `n` bars: what was knowable before bar `n` opened. Search runs on
    /// `head(lockbox_start)`, so it cannot see the lockbox at all.
    pub fn head(&self, n: usize) -> Panel {
        let n = n.min(self.n);
        let cut = |v: &Vec<Vec<f64>>| v.iter().map(|x| x[..n].to_vec()).collect::<Vec<_>>();
        let end = self.t0 + n as i64 * self.interval_ms;
        Panel {
            venue: self.venue.clone(),
            interval: self.interval.clone(),
            interval_ms: self.interval_ms,
            t0: self.t0,
            n,
            coins: self.coins.clone(),
            open: cut(&self.open),
            high: cut(&self.high),
            low: cut(&self.low),
            close: cut(&self.close),
            volume: cut(&self.volume),
            active: self.active.iter().map(|x| x[..n].to_vec()).collect(),
            funding: self.funding.iter().map(|f| f.iter().copied().filter(|(t, _)| *t < end).collect()).collect(),
            dvol: cut(&self.dvol),
            basket_ret: self.basket_ret[..n].to_vec(),
        }
    }

    /// Aggregates `k` bars into one (15m → 1h with k = 4). A coarse bar exists only
    /// when all `k` fine bars do.
    pub fn resample(&self, k: usize, interval: &str, rule: UniverseRule) -> Result<Panel> {
        let ims = interval_ms(interval).ok_or_else(|| anyhow::anyhow!("bad interval {interval}"))?;
        if k < 2 || ims != self.interval_ms * k as i64 {
            bail!("cannot resample {} into {interval}", self.interval);
        }
        let n = self.n / k;
        let nc = self.coins.len();
        let mut p = Panel {
            venue: self.venue.clone(),
            interval: interval.into(),
            interval_ms: ims,
            t0: self.t0,
            n,
            coins: self.coins.clone(),
            open: vec![vec![f64::NAN; n]; nc],
            high: vec![vec![f64::NAN; n]; nc],
            low: vec![vec![f64::NAN; n]; nc],
            close: vec![vec![f64::NAN; n]; nc],
            volume: vec![vec![f64::NAN; n]; nc],
            active: Vec::new(),
            funding: self.funding.clone(),
            dvol: Vec::new(),
            basket_ret: Vec::new(),
        };
        for c in 0..nc {
            for j in 0..n {
                let r = j * k..(j + 1) * k;
                if self.close[c][r.clone()].iter().any(|x| !x.is_finite()) {
                    continue;
                }
                p.open[c][j] = self.open[c][r.start];
                p.close[c][j] = self.close[c][r.end - 1];
                p.high[c][j] = self.high[c][r.clone()].iter().copied().fold(f64::MIN, f64::max);
                p.low[c][j] = self.low[c][r.clone()].iter().copied().fold(f64::MAX, f64::min);
                p.volume[c][j] = self.volume[c][r].iter().sum();
            }
        }
        p.derive(rule);
        Ok(p)
    }

    /// One line for reports: venue, bar size, coins, span.
    pub fn describe(&self) -> String {
        let day = |t: i64| chrono::DateTime::from_timestamp_millis(t).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default();
        format!("{} {} · {} coins · {} bars · {} → {}", self.venue, self.interval, self.coins.len(), self.n, day(self.t0), day(self.time(self.n.saturating_sub(1))))
    }
}

#[cfg(test)]
pub mod testing {
    use super::*;
    use crate::stats::Rng;

    pub const NO_RULE: UniverseRule = UniverseRule { min_dollar_volume: 0.0, min_history_days: 0 };

    /// A random-walk panel: `coins` coins sharing a market factor, `days` days of
    /// `interval` bars, with plausible highs and lows.
    pub fn random_panel(seed: u64, coins: usize, days: usize, interval: &str) -> Panel {
        let ims = interval_ms(interval).unwrap();
        let n = days * (DAY_MS / ims) as usize;
        let mut rng = Rng::new(seed);
        let sigma = 0.04 / ((DAY_MS / ims) as f64).sqrt();
        let market: Vec<f64> = (0..n).map(|_| rng.normal() * sigma * 0.6).collect();
        let series = (0..coins).map(|c| {
            let mut px = 100.0 * (c + 1) as f64;
            let candles = (0..n).map(|i| {
                let open = px;
                px *= (market[i] + rng.normal() * sigma * 0.8).exp();
                let wig = (rng.normal().abs() * sigma * 0.5).exp();
                Candle { open_time: i as i64 * ims, open, high: open.max(px) * wig, low: open.min(px) / wig, close: px, volume: 1e6 / open, close_time: (i as i64 + 1) * ims - 1 }
            }).collect();
            (format!("C{c}"), candles)
        }).collect();
        Panel::from_candles("test", interval, series, Vec::new(), NO_RULE).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    fn flat(coin: &str, from: usize, n: usize, px: f64, vol: f64) -> (String, Vec<Candle>) {
        let c = (from..from + n).map(|i| Candle { open_time: i as i64 * 3_600_000, open: px, high: px * 1.01, low: px * 0.99, close: px, volume: vol, close_time: (i as i64 + 1) * 3_600_000 - 1 }).collect();
        (coin.to_string(), c)
    }

    #[test]
    fn grid_days_and_missing_bars() {
        // B starts two days late: NaN before, real after.
        let p = Panel::from_candles("t", "1h", vec![flat("A", 0, 96, 100.0, 1.0), flat("B", 48, 48, 50.0, 1.0)], Vec::new(), NO_RULE).unwrap();
        assert_eq!((p.n, p.n_days(), p.bars_per_day()), (96, 4, 24));
        assert!(p.close[1][47].is_nan() && p.close[1][48] == 50.0);
        assert_eq!(p.day_of(23), 0);
        assert_eq!(p.day_of(24), 1);
        assert_eq!(p.index_at(p.time(30)), 30);
        assert_eq!(p.index_at(p.time(30) + 1), 31);
    }

    #[test]
    fn universe_uses_only_earlier_days() {
        // $2.4M a day (100 × 1000 × 24): passes a $1M bar once 3 earlier days exist.
        let rule = UniverseRule { min_dollar_volume: 1e6, min_history_days: 3 };
        let p = Panel::from_candles("t", "1h", vec![flat("A", 0, 24 * 6, 100.0, 1000.0), flat("THIN", 0, 24 * 6, 100.0, 1.0)], Vec::new(), rule).unwrap();
        assert!(!p.active[0][24 * 3 - 1], "day 2 has only two earlier days");
        assert!(p.active[0][24 * 3], "day 3 has three");
        assert!(p.active[1].iter().all(|a| !a), "the thin coin never qualifies");
    }

    #[test]
    fn head_matches_a_panel_built_from_less_data() {
        let full = random_panel(1, 3, 20, "1h");
        let cut = 24 * 12;
        let mut head = full.head(cut);
        assert_eq!(head.n, cut);
        // Derived series are causal: recomputed without the future, nothing before it changes.
        head.rederive(NO_RULE);
        for c in 0..3 {
            for i in 0..cut {
                assert!(head.dvol[c][i] == full.dvol[c][i] || (head.dvol[c][i].is_nan() && full.dvol[c][i].is_nan()));
                assert_eq!(head.active[c][i], full.active[c][i]);
            }
        }
        assert_eq!(head.basket_ret, full.basket_ret[..cut]);
    }

    #[test]
    fn resample_builds_hourly_bars_from_quarters() {
        let q = random_panel(3, 2, 4, "15m");
        let h = q.resample(4, "1h", NO_RULE).unwrap();
        assert_eq!(h.n, q.n / 4);
        assert_eq!(h.open[0][5], q.open[0][20]);
        assert_eq!(h.close[0][5], q.close[0][23]);
        assert!((h.high[0][5] - q.high[0][20..24].iter().copied().fold(f64::MIN, f64::max)).abs() < 1e-12);
        assert!(q.resample(3, "1h", NO_RULE).is_err());
    }

    #[test]
    fn daily_vol_is_about_right() {
        let p = random_panel(7, 2, 30, "15m");
        let v = p.dvol[0][p.n - 1];
        assert!(v > 0.02 && v < 0.07, "daily vol {v}");
    }
}
