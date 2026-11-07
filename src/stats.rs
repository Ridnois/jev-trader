//! Noise-aware statistics (1.3, 2.4): independence clustering, Spearman IC,
//! block-permutation nulls, and OLS with Newey-West (HAC) errors.
//! Deterministic: a fixed-seed SplitMix64 generator, so reports are reproducible.

use chrono::{DateTime, Duration, Utc};

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Standard normal (Box–Muller).
    pub fn normal(&mut self) -> f64 {
        let u = self.uniform().max(1e-300);
        let v = self.uniform();
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

/// Independence guard: observations (time-sorted) whose windows overlap are merged
/// into one cluster; returns the mean value per cluster.
pub fn merge_windows(v: &[(DateTime<Utc>, f64)], window: Duration) -> Vec<f64> {
    merge_pairs(&v.iter().map(|(t, x)| (*t, *x, 0.0)).collect::<Vec<_>>(), window)
        .into_iter()
        .map(|(x, _)| x)
        .collect()
}

/// Same, for (t, x, y) triples: one (mean x, mean y) per cluster. Input must be time-sorted.
pub fn merge_pairs(v: &[(DateTime<Utc>, f64, f64)], window: Duration) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let start = v[i].0;
        let mut end = start + window;
        let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
        while i < v.len() && v[i].0 < end {
            sx += v[i].1;
            sy += v[i].2;
            n += 1.0;
            // A member's own window extends the cluster.
            end = end.max(v[i].0 + window);
            i += 1;
        }
        out.push((sx / n, sy / n));
    }
    out
}

pub fn ranks(x: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..x.len()).collect();
    idx.sort_by(|&a, &b| x[a].total_cmp(&x[b]));
    let mut r = vec![0.0; x.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && x[idx[j + 1]] == x[idx[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0;
        for k in i..=j {
            r[idx[k]] = avg;
        }
        i = j + 1;
    }
    r
}

pub fn pearson(x: &[f64], y: &[f64]) -> Option<f64> {
    let n = x.len();
    if n < 3 || n != y.len() {
        return None;
    }
    let mx = x.iter().sum::<f64>() / n as f64;
    let my = y.iter().sum::<f64>() / n as f64;
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for i in 0..n {
        sxy += (x[i] - mx) * (y[i] - my);
        sxx += (x[i] - mx).powi(2);
        syy += (y[i] - my).powi(2);
    }
    (sxx > 0.0 && syy > 0.0).then(|| sxy / (sxx * syy).sqrt())
}

pub fn spearman(x: &[f64], y: &[f64]) -> Option<f64> {
    pearson(&ranks(x), &ranks(y))
}

/// Share of pairs where sign(x) == sign(y), ignoring x == 0 (Hold has no hit rate).
pub fn hit_rate(x: &[f64], y: &[f64]) -> Option<f64> {
    let pairs: Vec<bool> = x.iter().zip(y).filter(|(a, _)| **a != 0.0).map(|(a, b)| (*a > 0.0) == (*b > 0.0)).collect();
    (!pairs.is_empty()).then(|| pairs.iter().filter(|h| **h).count() as f64 / pairs.len() as f64)
}

/// Block-permutation null: shuffles `y` against `x` in contiguous blocks (preserving
/// autocorrelation) and returns the percentile (0–100) of the observed statistic.
pub fn null_percentile(x: &[f64], y: &[f64], block: usize, iters: usize, stat: impl Fn(&[f64], &[f64]) -> Option<f64>) -> Option<f64> {
    let obs = stat(x, y)?;
    let n = y.len();
    let block = block.clamp(1, n.max(1));
    let blocks: Vec<&[f64]> = y.chunks(block).collect();
    let mut rng = Rng::new(0x5EED ^ n as u64);
    let mut below = 0usize;
    let mut total = 0usize;
    let mut order: Vec<usize> = (0..blocks.len()).collect();
    let mut shuffled = Vec::with_capacity(n);
    for _ in 0..iters {
        for i in (1..order.len()).rev() {
            order.swap(i, rng.below(i + 1));
        }
        // Random circular offset as well, so a single block still varies.
        shuffled.clear();
        for &b in &order {
            shuffled.extend_from_slice(blocks[b]);
        }
        let off = rng.below(n);
        shuffled.rotate_left(off);
        if let Some(s) = stat(x, &shuffled) {
            total += 1;
            if s < obs {
                below += 1;
            }
        }
    }
    (total > 0).then(|| below as f64 / total as f64 * 100.0)
}

/// Two-sided p-value from a null percentile.
pub fn p_two_sided(pct: f64) -> f64 {
    (2.0 * (pct / 100.0).min(1.0 - pct / 100.0)).clamp(0.0, 1.0)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Ols {
    pub n: usize,
    pub alpha: f64,
    pub beta: f64,
    pub t_alpha: f64,
    pub t_beta: f64,
    pub r2: f64,
    pub resid_sd: f64,
}

/// y = α + βx + ε, with Newey-West standard errors (Bartlett kernel,
/// lag = ⌊4(n/100)^(2/9)⌋).
pub fn ols_hac(y: &[f64], x: &[f64]) -> Option<Ols> {
    let n = y.len();
    if n < 10 || x.len() != n {
        return None;
    }
    let nf = n as f64;
    let mx = x.iter().sum::<f64>() / nf;
    let my = y.iter().sum::<f64>() / nf;
    let sxx: f64 = x.iter().map(|v| (v - mx).powi(2)).sum();
    let beta = if sxx > 0.0 { x.iter().zip(y).map(|(a, b)| (a - mx) * (b - my)).sum::<f64>() / sxx } else { 0.0 };
    let alpha = my - beta * mx;
    let e: Vec<f64> = (0..n).map(|i| y[i] - alpha - beta * x[i]).collect();
    let sse: f64 = e.iter().map(|v| v * v).sum();
    let sst: f64 = y.iter().map(|v| (v - my).powi(2)).sum();
    let r2 = if sst > 0.0 { 1.0 - sse / sst } else { 0.0 };
    // (X'X)^-1 for X = [1, x]
    let sx: f64 = x.iter().sum();
    let sxx_raw: f64 = x.iter().map(|v| v * v).sum();
    let det = nf * sxx_raw - sx * sx;
    let inv = if det.abs() > 1e-18 {
        [[sxx_raw / det, -sx / det], [-sx / det, nf / det]]
    } else {
        [[1.0 / nf, 0.0], [0.0, 0.0]]
    };
    // Meat: S = Σ_l w_l Σ_t (x_t e_t)(x_{t-l} e_{t-l})' + transpose
    let lag = (4.0 * (nf / 100.0).powf(2.0 / 9.0)).floor() as usize;
    let g: Vec<[f64; 2]> = (0..n).map(|t| [e[t], x[t] * e[t]]).collect();
    let mut s = [[0.0; 2]; 2];
    for l in 0..=lag.min(n - 1) {
        let w = if l == 0 { 1.0 } else { 1.0 - l as f64 / (lag as f64 + 1.0) };
        let mut gamma = [[0.0; 2]; 2];
        for t in l..n {
            for a in 0..2 {
                for b in 0..2 {
                    gamma[a][b] += g[t][a] * g[t - l][b];
                }
            }
        }
        for a in 0..2 {
            for b in 0..2 {
                s[a][b] += w * if l == 0 { gamma[a][b] } else { gamma[a][b] + gamma[b][a] };
            }
        }
    }
    let mul = |a: [[f64; 2]; 2], b: [[f64; 2]; 2]| {
        let mut c = [[0.0; 2]; 2];
        for i in 0..2 {
            for j in 0..2 {
                c[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j];
            }
        }
        c
    };
    let v = mul(mul(inv, s), inv);
    let se_a = v[0][0].max(0.0).sqrt();
    let se_b = v[1][1].max(0.0).sqrt();
    Some(Ols {
        n,
        alpha,
        beta,
        t_alpha: if se_a > 0.0 { alpha / se_a } else { 0.0 },
        t_beta: if se_b > 0.0 { beta / se_b } else { 0.0 },
        r2,
        resid_sd: (sse / (nf - 2.0)).sqrt(),
    })
}

/// 5.3: Monte Carlo false-confirmation rate of "Σz/√n < threshold over the last
/// `n` clusters" on pure noise: i.i.d. N(0,1) and AR(1) with coefficient `phi`.
pub fn false_confirmation_rate(n: usize, threshold: f64, phi: f64, trials: usize) -> (f64, f64) {
    let mut rng = Rng::new(42);
    let (mut iid, mut ar) = (0, 0);
    for _ in 0..trials {
        let z: Vec<f64> = (0..n).map(|_| rng.normal()).collect();
        if z.iter().sum::<f64>() / (n as f64).sqrt() < threshold {
            iid += 1;
        }
        let mut prev = rng.normal();
        let mut s = 0.0;
        for _ in 0..n {
            prev = phi * prev + (1.0 - phi * phi).sqrt() * rng.normal();
            s += prev;
        }
        if s / (n as f64).sqrt() < threshold {
            ar += 1;
        }
    }
    (iid as f64 / trials as f64, ar as f64 / trials as f64)
}

pub fn mean(v: &[f64]) -> f64 {
    if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 }
}

pub fn quantile(v: &[f64], q: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    Some(s[((s.len() - 1) as f64 * q).round() as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spearman_and_ranks() {
        let x = [1.0, 2.0, 3.0, 4.0, 5.0];
        let y = [10.0, 20.0, 25.0, 100.0, 1000.0];
        assert!((spearman(&x, &y).unwrap() - 1.0).abs() < 1e-12);
        assert_eq!(ranks(&[3.0, 1.0, 3.0]), vec![2.5, 1.0, 2.5]);
    }

    #[test]
    fn merges_overlapping_windows() {
        let t = |h: i64| DateTime::from_timestamp(h * 3600, 0).unwrap();
        let v = [(t(0), 1.0), (t(1), 3.0), (t(30), 5.0)];
        assert_eq!(merge_windows(&v, Duration::hours(24)), vec![2.0, 5.0]);
    }

    #[test]
    fn null_flags_noise_and_signal() {
        let mut rng = Rng::new(7);
        let x: Vec<f64> = (0..200).map(|_| rng.normal()).collect();
        let noise: Vec<f64> = (0..200).map(|_| rng.normal()).collect();
        let signal: Vec<f64> = x.iter().map(|v| v + 0.5 * rng.normal()).collect();
        let p_noise = null_percentile(&x, &noise, 5, 500, spearman).unwrap();
        let p_sig = null_percentile(&x, &signal, 5, 500, spearman).unwrap();
        assert!(p_sig > 99.0);
        assert!(p_noise > 2.0 && p_noise < 98.0, "noise percentile {p_noise}");
    }

    #[test]
    fn hac_recovers_alpha() {
        let mut rng = Rng::new(3);
        let x: Vec<f64> = (0..500).map(|_| rng.normal()).collect();
        let y: Vec<f64> = x.iter().map(|v| 0.5 + 2.0 * v + 0.1 * rng.normal()).collect();
        let o = ols_hac(&y, &x).unwrap();
        assert!((o.alpha - 0.5).abs() < 0.05 && (o.beta - 2.0).abs() < 0.05);
        assert!(o.t_alpha > 10.0 && o.r2 > 0.99);
    }

    #[test]
    fn hac_t_is_calibrated_on_noise() {
        let mut rng = Rng::new(11);
        let trials = 400;
        let mut big = 0;
        for _ in 0..trials {
            let x: Vec<f64> = (0..300).map(|_| rng.normal()).collect();
            let y: Vec<f64> = x.iter().map(|v| 0.8 * v + rng.normal()).collect();
            if ols_hac(&y, &x).unwrap().t_alpha.abs() > 2.0 {
                big += 1;
            }
        }
        let rate = big as f64 / trials as f64;
        assert!(rate < 0.10, "false-positive rate {rate}");
    }

    #[test]
    fn false_confirmation_below_ten_percent() {
        let (iid, ar) = false_confirmation_rate(3, -1.5, 0.3, 4000);
        assert!(iid < 0.10, "iid {iid}");
        assert!(ar < 0.15, "ar {ar}");
    }
}
