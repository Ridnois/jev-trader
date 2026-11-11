//! Statistics that pay for the search: stationary bootstrap, probabilistic and
//! deflated Sharpe ratios, a studentized reality check across a whole family of
//! rules, and the probability of backtest overfitting (CSCV).
//!
//! Every function works on per-period returns (the lab uses UTC days) and is
//! deterministic for a given seed, like `crate::stats`.

use crate::stats::Rng;

pub fn mean(x: &[f64]) -> f64 {
    if x.is_empty() { 0.0 } else { x.iter().sum::<f64>() / x.len() as f64 }
}

/// Sample standard deviation (n − 1).
pub fn sd(x: &[f64]) -> f64 {
    if x.len() < 2 {
        return 0.0;
    }
    let m = mean(x);
    (x.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (x.len() - 1) as f64).sqrt()
}

/// Per-period Sharpe ratio (mean / sd). 0 when the series is flat.
pub fn sharpe(x: &[f64]) -> f64 {
    let s = sd(x);
    if s > 0.0 { mean(x) / s } else { 0.0 }
}

/// Skewness and (non-excess) kurtosis. A flat series reads as normal (0, 3).
pub fn skew_kurt(x: &[f64]) -> (f64, f64) {
    let n = x.len() as f64;
    if x.len() < 3 {
        return (0.0, 3.0);
    }
    let m = mean(x);
    let m2 = x.iter().map(|v| (v - m).powi(2)).sum::<f64>() / n;
    if m2 <= 0.0 {
        return (0.0, 3.0);
    }
    let m3 = x.iter().map(|v| (v - m).powi(3)).sum::<f64>() / n;
    let m4 = x.iter().map(|v| (v - m).powi(4)).sum::<f64>() / n;
    (m3 / m2.powf(1.5), m4 / (m2 * m2))
}

/// Standard normal CDF (Abramowitz–Stegun 7.1.26 on erfc; error < 1.5e-7).
pub fn norm_cdf(x: f64) -> f64 {
    let z = x.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let poly = t * (0.254_829_592 + t * (-0.284_496_736 + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erfc = poly * (-z * z).exp();
    if x >= 0.0 { 1.0 - 0.5 * erfc } else { 0.5 * erfc }
}

/// Inverse standard normal CDF (Acklam; relative error < 1.2e-9).
pub fn norm_inv(p: f64) -> f64 {
    let p = p.clamp(1e-300, 1.0 - 1e-16);
    const A: [f64; 6] = [-3.969683028665376e+01, 2.209460984245205e+02, -2.759285104469687e+02, 1.383577518672690e+02, -3.066479806614716e+01, 2.506628277459239e+00];
    const B: [f64; 5] = [-5.447609879822406e+01, 1.615858368580409e+02, -1.556989798598866e+02, 6.680131188771972e+01, -1.328068155288572e+01];
    const C: [f64; 6] = [-7.784894002430293e-03, -3.223964580411365e-01, -2.400758277161838e+00, -2.549732539343734e+00, 4.374664141464968e+00, 2.938163982698783e+00];
    const D: [f64; 4] = [7.784695709041462e-03, 3.224671290700398e-01, 2.445134137142996e+00, 3.754408661907416e+00];
    let lo = 0.02425;
    if p < lo {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5]) / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - lo {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5]) / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    }
}

/// Probabilistic Sharpe ratio (Bailey & López de Prado): P(true SR > `sr0`) given an
/// observed per-period `sr` over `n` periods with the returns' skew and kurtosis.
pub fn psr(sr: f64, sr0: f64, n: usize, skew: f64, kurt: f64) -> f64 {
    if n < 3 {
        return 0.5;
    }
    let var = (1.0 - skew * sr + (kurt - 1.0) / 4.0 * sr * sr).max(1e-12);
    norm_cdf((sr - sr0) * ((n - 1) as f64).sqrt() / var.sqrt())
}

/// The Sharpe ratio the best of `n_trials` no-edge rules is expected to show, given
/// the variance of the trials' Sharpe ratios (the "false strategy" theorem).
pub fn expected_max_sharpe(var_sr: f64, n_trials: f64) -> f64 {
    if n_trials <= 1.0 || var_sr <= 0.0 {
        return 0.0;
    }
    const GAMMA: f64 = 0.577_215_664_901_532_9;
    let a = norm_inv(1.0 - 1.0 / n_trials);
    let b = norm_inv(1.0 - 1.0 / (n_trials * std::f64::consts::E));
    var_sr.sqrt() * ((1.0 - GAMMA) * a + GAMMA * b)
}

/// Effective number of independent trials from the mean pairwise correlation of
/// the trials' return series: N_eff = 1 + (N − 1)(1 − ρ̄). Pairs are sampled.
pub fn effective_trials<R: AsRef<[f64]>>(rows: &[R], n_total: usize, seed: u64) -> (f64, f64) {
    let k = rows.len();
    if k < 2 {
        return (n_total.max(1) as f64, 0.0);
    }
    let mut rng = Rng::new(seed);
    let (mut sum, mut cnt) = (0.0, 0usize);
    let pairs = (k * (k - 1) / 2).min(3000);
    for _ in 0..pairs {
        let (i, mut j) = (rng.below(k), rng.below(k));
        if i == j {
            j = (j + 1) % k;
        }
        if let Some(r) = crate::stats::pearson(rows[i].as_ref(), rows[j].as_ref()) {
            sum += r;
            cnt += 1;
        }
    }
    let rho = if cnt > 0 { (sum / cnt as f64).clamp(0.0, 1.0) } else { 0.0 };
    (1.0 + (n_total.max(1) as f64 - 1.0) * (1.0 - rho), rho)
}

/// Stationary bootstrap (Politis–Romano): `n` indices into a series of length `n`,
/// in blocks of geometric length with mean `block`, wrapping around the end.
pub fn stationary_indices(rng: &mut Rng, n: usize, block: f64, out: &mut Vec<usize>) {
    out.clear();
    if n == 0 {
        return;
    }
    let p = 1.0 / block.max(1.0);
    let mut i = rng.below(n);
    for _ in 0..n {
        out.push(i);
        i = if rng.uniform() < p { rng.below(n) } else { (i + 1) % n };
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BootMean {
    pub mean: f64,
    /// 95% percentile interval of the mean.
    pub lo: f64,
    pub hi: f64,
    /// One-sided p-value of "mean ≤ 0" (bootstrap means recentred on zero).
    pub p: f64,
}

/// Mean of `x` with a stationary-bootstrap interval and a one-sided p-value.
pub fn boot_mean(x: &[f64], block: f64, iters: usize, seed: u64) -> BootMean {
    let m = mean(x);
    if x.len() < 5 {
        return BootMean { mean: m, lo: f64::NEG_INFINITY, hi: f64::INFINITY, p: 1.0 };
    }
    let mut rng = Rng::new(seed);
    let mut idx = Vec::with_capacity(x.len());
    let mut means = Vec::with_capacity(iters);
    for _ in 0..iters {
        stationary_indices(&mut rng, x.len(), block, &mut idx);
        means.push(idx.iter().map(|&i| x[i]).sum::<f64>() / x.len() as f64);
    }
    means.sort_by(f64::total_cmp);
    let q = |p: f64| means[((means.len() - 1) as f64 * p).round() as usize];
    let ge = means.iter().filter(|b| **b - m >= m).count();
    BootMean { mean: m, lo: q(0.025), hi: q(0.975), p: (ge as f64 + 1.0) / (iters as f64 + 1.0) }
}

/// Bootstrap means of windows of `len` periods drawn from `x`: the distribution a
/// fresh, shorter sample of the same process would show. Sorted ascending.
pub fn boot_window_means(x: &[f64], len: usize, block: f64, iters: usize, seed: u64) -> Vec<f64> {
    let mut rng = Rng::new(seed);
    let mut idx = Vec::with_capacity(x.len());
    let mut out = Vec::with_capacity(iters);
    if x.is_empty() || len == 0 {
        return out;
    }
    for _ in 0..iters {
        stationary_indices(&mut rng, x.len(), block, &mut idx);
        let take = len.min(idx.len());
        out.push(idx[..take].iter().map(|&i| x[i]).sum::<f64>() / take as f64);
    }
    out.sort_by(f64::total_cmp);
    out
}

/// Worst peak-to-trough drawdown (in return units, summed not compounded) over
/// bootstrap paths of `len` periods: the `q` quantile of that distribution.
pub fn boot_drawdown(x: &[f64], len: usize, block: f64, iters: usize, q: f64, seed: u64) -> f64 {
    let mut rng = Rng::new(seed);
    let mut idx = Vec::with_capacity(x.len());
    let mut dds = Vec::with_capacity(iters);
    if x.is_empty() || len == 0 {
        return 0.0;
    }
    for _ in 0..iters {
        let (mut cum, mut peak, mut dd) = (0.0f64, 0.0f64, 0.0f64);
        let mut left = len;
        while left > 0 {
            stationary_indices(&mut rng, x.len(), block, &mut idx);
            for &i in idx.iter().take(left) {
                cum += x[i];
                peak = peak.max(cum);
                dd = dd.max(peak - cum);
            }
            left -= left.min(idx.len());
        }
        dds.push(dd);
    }
    dds.sort_by(f64::total_cmp);
    dds[((dds.len() - 1) as f64 * q).round() as usize]
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RealityCheck {
    /// P(the best of this many no-edge rules scores at least as well), 0..1.
    pub p: f64,
    /// Row of the best rule by studentized mean.
    pub best: usize,
    /// Its studentized mean (a t-statistic).
    pub stat: f64,
}

/// Studentized reality check (White 2000, with Hansen's studentization): is the
/// best rule of the family better than zero once the whole family is accounted for?
/// `rows[k]` is rule k's return series; all rows share the same periods.
pub fn reality_check<R: AsRef<[f64]>>(rows: &[R], block: f64, iters: usize, seed: u64) -> Option<RealityCheck> {
    let t = rows.first()?.as_ref().len();
    if t < 20 {
        return None;
    }
    let root = (t as f64).sqrt();
    // Rules that never trade (sd = 0) can't be the best and are left out.
    let stats: Vec<(usize, f64, f64)> = rows.iter().enumerate().filter_map(|(k, r)| {
        let r = r.as_ref();
        let s = sd(r);
        (s > 0.0).then(|| (k, mean(r), s))
    }).collect();
    let (best, stat) = stats.iter().map(|(k, m, s)| (*k, root * m / s)).max_by(|a, b| a.1.total_cmp(&b.1))?;
    let mut rng = Rng::new(seed);
    let mut idx = Vec::with_capacity(t);
    let mut ge = 0usize;
    for _ in 0..iters {
        stationary_indices(&mut rng, t, block, &mut idx);
        let mut vmax = f64::NEG_INFINITY;
        for (k, m, s) in &stats {
            let r = rows[*k].as_ref();
            let bm = idx.iter().map(|&i| r[i]).sum::<f64>() / t as f64;
            vmax = vmax.max(root * (bm - m) / s);
        }
        if vmax >= stat {
            ge += 1;
        }
    }
    Some(RealityCheck { p: (ge as f64 + 1.0) / (iters as f64 + 1.0), best, stat })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Pbo {
    /// Share of train/test splits where the in-sample winner lands in the bottom
    /// half out of sample.
    pub pbo: f64,
    pub splits: usize,
    /// Median out-of-sample rank of the in-sample winner (0 = worst, 1 = best).
    pub median_rank: f64,
}

/// Combinatorially symmetric cross-validation (Bailey, Borwein, López de Prado, Zhu):
/// the periods are cut into `s` blocks (even), every half/half split picks the rule
/// with the best in-sample Sharpe and looks up its out-of-sample rank.
pub fn pbo_cscv<R: AsRef<[f64]>>(rows: &[R], s: usize) -> Option<Pbo> {
    let k = rows.len();
    let t = rows.first()?.as_ref().len();
    let s = s - s % 2;
    if k < 4 || s < 4 || t < s * 5 {
        return None;
    }
    let bl = t / s;
    // Per rule, per block: sum and sum of squares.
    let sums: Vec<Vec<(f64, f64)>> = rows.iter().map(|r| (0..s).map(|b| {
        let w = &r.as_ref()[b * bl..if b == s - 1 { t } else { (b + 1) * bl }];
        (w.iter().sum(), w.iter().map(|v| v * v).sum())
    }).collect()).collect();
    let lens: Vec<f64> = (0..s).map(|b| (if b == s - 1 { t - b * bl } else { bl }) as f64).collect();
    let sr = |k: usize, mask: u32, want: bool| -> f64 {
        let (mut a, mut q, mut n) = (0.0, 0.0, 0.0);
        for b in 0..s {
            if ((mask >> b) & 1 == 1) == want {
                a += sums[k][b].0;
                q += sums[k][b].1;
                n += lens[b];
            }
        }
        let m = a / n;
        let var = (q / n - m * m).max(0.0);
        if var > 0.0 { m / var.sqrt() } else { 0.0 }
    };
    let mut ranks = Vec::new();
    for mask in 0u32..(1 << s) {
        if mask.count_ones() as usize != s / 2 {
            continue;
        }
        let best = (0..k).max_by(|a, b| sr(*a, mask, true).total_cmp(&sr(*b, mask, true)))?;
        let oos_best = sr(best, mask, false);
        let below = (0..k).filter(|j| sr(*j, mask, false) < oos_best).count();
        ranks.push((below as f64 + 0.5) / k as f64);
    }
    if ranks.is_empty() {
        return None;
    }
    let pbo = ranks.iter().filter(|r| **r <= 0.5).count() as f64 / ranks.len() as f64;
    ranks.sort_by(f64::total_cmp);
    Some(Pbo { pbo, splits: ranks.len(), median_rank: ranks[ranks.len() / 2] })
}

/// Wald's sequential test for paper trading: H0 "no edge" against H1 "the predicted
/// daily mean `mu`", daily sd `sigma`. Returns the log-likelihood ratio after `x`.
pub fn sprt_llr(x: &[f64], mu: f64, sigma: f64) -> f64 {
    if sigma <= 0.0 {
        return 0.0;
    }
    x.iter().map(|v| mu * v / (sigma * sigma) - mu * mu / (2.0 * sigma * sigma)).sum()
}

/// (kill below, graduate above) bounds on the log-likelihood ratio for false-positive
/// rate `alpha` and power `1 − beta`.
pub fn sprt_bounds(alpha: f64, beta: f64) -> (f64, f64) {
    ((beta / (1.0 - alpha)).ln(), ((1.0 - beta) / alpha).ln())
}

/// Expected days until the sequential test decides, if the edge is real (first) or
/// absent (second).
pub fn sprt_expected_days(mu: f64, sigma: f64, alpha: f64, beta: f64) -> (f64, f64) {
    let (lo, hi) = sprt_bounds(alpha, beta);
    let step = mu * mu / (2.0 * sigma * sigma);
    if step <= 0.0 {
        return (f64::INFINITY, f64::INFINITY);
    }
    let h1 = ((1.0 - beta) * hi + beta * lo) / step;
    let h0 = (alpha * hi + (1.0 - alpha) * lo) / -step;
    (h1, h0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(rng: &mut Rng, n: usize, mu: f64) -> Vec<f64> {
        (0..n).map(|_| mu + rng.normal()).collect()
    }

    #[test]
    fn normal_functions_match_known_values() {
        assert!((norm_cdf(0.0) - 0.5).abs() < 1e-7);
        assert!((norm_cdf(1.96) - 0.975).abs() < 1e-4);
        assert!((norm_cdf(-1.0) - 0.158_655).abs() < 1e-5);
        assert!((norm_inv(0.975) - 1.959_964).abs() < 1e-5);
        assert!((norm_inv(0.5)).abs() < 1e-9);
        assert!((norm_inv(0.001) + 3.090_232).abs() < 1e-5);
    }

    #[test]
    fn psr_matches_the_paper_and_moves_the_right_way() {
        // SR 0.1/day over 500 days, normal returns: z = 0.1·√499 / √(1 + 0.005) ≈ 2.228.
        let p = psr(0.1, 0.0, 500, 0.0, 3.0);
        assert!((p - norm_cdf(2.2283)).abs() < 1e-3, "{p}");
        // Negative skew and fat tails lower it; a higher benchmark lowers it.
        assert!(psr(0.1, 0.0, 500, -1.0, 8.0) < p);
        assert!(psr(0.1, 0.05, 500, 0.0, 3.0) < p);
    }

    #[test]
    fn expected_max_grows_with_trials() {
        // Sharpe ratios of no-edge rules over 400 days have sd ≈ 1/√400 = 0.05.
        let e10 = expected_max_sharpe(0.05 * 0.05, 10.0);
        let e380 = expected_max_sharpe(0.05 * 0.05, 380.0);
        assert!(e10 > 0.06 && e10 < 0.09, "{e10}");
        assert!(e380 > 0.13 && e380 < 0.16, "{e380}");
        assert_eq!(expected_max_sharpe(0.01, 1.0), 0.0);
    }

    #[test]
    fn expected_max_matches_simulation() {
        let mut rng = Rng::new(5);
        let (n, t) = (200usize, 400usize);
        let mut maxes = Vec::new();
        for _ in 0..200 {
            let m = (0..n).map(|_| sharpe(&noise(&mut rng, t, 0.0))).fold(f64::NEG_INFINITY, f64::max);
            maxes.push(m);
        }
        let sim = mean(&maxes);
        let theory = expected_max_sharpe(1.0 / t as f64, n as f64);
        assert!((sim - theory).abs() < 0.01, "simulated {sim} vs theory {theory}");
    }

    #[test]
    fn bootstrap_interval_covers_and_rejects() {
        let mut rng = Rng::new(9);
        let flat = noise(&mut rng, 400, 0.0);
        let b = boot_mean(&flat, 5.0, 800, 1);
        assert!(b.lo < 0.0 && b.hi > 0.0 && b.p > 0.05, "{b:?}");
        let edge = noise(&mut rng, 400, 0.3);
        let b = boot_mean(&edge, 5.0, 800, 1);
        assert!(b.lo > 0.0 && b.p < 0.01, "{b:?}");
    }

    #[test]
    fn reality_check_is_calibrated_on_noise_and_finds_an_edge() {
        let mut rng = Rng::new(21);
        let trials = 80;
        let mut rejects = 0;
        for s in 0..trials {
            let rows: Vec<Vec<f64>> = (0..40).map(|_| noise(&mut rng, 250, 0.0)).collect();
            if reality_check(&rows, 4.0, 150, s).unwrap().p < 0.10 {
                rejects += 1;
            }
        }
        let rate = rejects as f64 / trials as f64;
        assert!(rate < 0.20, "false-positive rate {rate} at a 10% level");
        // One real rule among 40: found, and significant.
        let mut rows: Vec<Vec<f64>> = (0..40).map(|_| noise(&mut rng, 250, 0.0)).collect();
        rows[17] = noise(&mut rng, 250, 0.35);
        let rc = reality_check(&rows, 4.0, 500, 3).unwrap();
        assert_eq!(rc.best, 17);
        assert!(rc.p < 0.02, "{rc:?}");
    }

    #[test]
    fn best_of_many_noise_rules_looks_good_alone_but_not_to_the_check() {
        // The trap the lab exists for: 380 no-edge rules, pick the best.
        let mut rng = Rng::new(77);
        let rows: Vec<Vec<f64>> = (0..380).map(|_| noise(&mut rng, 300, 0.0)).collect();
        let rc = reality_check(&rows, 4.0, 300, 4).unwrap();
        assert!(rc.stat > 2.0, "the best of 380 has t = {}", rc.stat);
        assert!(rc.p > 0.10, "but the family-wide p is {}", rc.p);
        let best = &rows[rc.best];
        let (sk, ku) = skew_kurt(best);
        let var_sr = sd(&rows.iter().map(|r| sharpe(r)).collect::<Vec<_>>()).powi(2);
        let dsr = psr(sharpe(best), expected_max_sharpe(var_sr, 380.0), 300, sk, ku);
        assert!(dsr < 0.9, "deflated Sharpe {dsr}");
    }

    #[test]
    fn pbo_is_high_on_noise_and_low_with_a_real_winner() {
        let mut rng = Rng::new(31);
        let mut pbos = Vec::new();
        for _ in 0..12 {
            let rows: Vec<Vec<f64>> = (0..30).map(|_| noise(&mut rng, 400, 0.0)).collect();
            pbos.push(pbo_cscv(&rows, 10).unwrap().pbo);
        }
        let avg = mean(&pbos);
        assert!(avg > 0.35 && avg < 0.65, "noise PBO {avg}");
        let mut rows: Vec<Vec<f64>> = (0..30).map(|_| noise(&mut rng, 400, 0.0)).collect();
        rows[4] = noise(&mut rng, 400, 0.4);
        let p = pbo_cscv(&rows, 10).unwrap();
        assert_eq!(p.splits, 252);
        assert!(p.pbo < 0.05 && p.median_rank > 0.9, "{p:?}");
    }

    #[test]
    fn effective_trials_shrink_when_rules_are_copies() {
        let mut rng = Rng::new(2);
        let base = noise(&mut rng, 300, 0.0);
        let copies: Vec<Vec<f64>> = (0..20).map(|_| base.iter().map(|v| v + 0.1 * rng.normal()).collect()).collect();
        let (n_eff, rho) = effective_trials(&copies, 20, 1);
        assert!(rho > 0.9 && n_eff < 3.5, "rho {rho}, n_eff {n_eff}");
        let indep: Vec<Vec<f64>> = (0..20).map(|_| noise(&mut rng, 300, 0.0)).collect();
        assert!(effective_trials(&indep, 20, 1).0 > 17.0);
    }

    #[test]
    fn sequential_test_bounds_and_timing() {
        let (lo, hi) = sprt_bounds(0.05, 0.20);
        assert!((hi - 2.7726).abs() < 1e-3 && (lo + 1.5581).abs() < 1e-3);
        // Daily Sharpe 0.3: about two months to confirm, about a month to kill.
        let (h1, h0) = sprt_expected_days(0.3, 1.0, 0.05, 0.20);
        assert!(h1 > 35.0 && h1 < 50.0, "{h1}");
        assert!(h0 > 25.0 && h0 < 35.0, "{h0}");
        let mut rng = Rng::new(8);
        let good = noise(&mut rng, 200, 0.3);
        assert!(sprt_llr(&good, 0.3, 1.0) > hi);
        let none = noise(&mut rng, 200, 0.0);
        assert!(sprt_llr(&none, 0.3, 1.0) < lo);
    }

    #[test]
    fn drawdown_envelope_grows_with_horizon() {
        let mut rng = Rng::new(4);
        let x = noise(&mut rng, 500, 0.05);
        let d30 = boot_drawdown(&x, 30, 5.0, 400, 0.99, 1);
        let d120 = boot_drawdown(&x, 120, 5.0, 400, 0.99, 1);
        assert!(d30 > 5.0 && d120 > d30, "{d30} {d120}");
    }
}
