//! Strategies as data. A `Spec` is a family name plus numeric parameters; `plan`
//! turns it into entry and exit signals for every coin and bar of a panel. The
//! simulator and the live engine both act on those signals, so a rule is written once.
//!
//! Timing contract: the signal at bar `i` uses bars `0..=i` only and is acted on at
//! the open of bar `i + 1`.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::compose::{self, Rule};
use super::panel::Panel;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spec {
    pub family: String,
    pub params: BTreeMap<String, f64>,
}

impl Spec {
    #[cfg(test)]
    pub fn new(family: &str, params: &[(&str, f64)]) -> Self {
        Self { family: family.into(), params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect() }
    }

    /// Stable id: `meanrev:hold=24,n=288,stop=500,z=2`.
    pub fn id(&self) -> String {
        format!("{}:{}", self.family, self.params.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(","))
    }

    pub fn get(&self, key: &str, default: f64) -> f64 {
        self.params.get(key).copied().unwrap_or(default)
    }

    fn need(&self, key: &str) -> Result<f64> {
        self.params.get(key).copied().with_context(|| format!("{}: missing parameter `{key}`", self.family))
    }
}

/// A family file (`families/<name>.toml`): the rule, the bar size, and the grid the
/// search may try. The grid is the declared search space; nothing outside it is tested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Family {
    /// The rule's name. For a composed family this is the rule id (`c…`), filled in
    /// at load when the file leaves it out.
    #[serde(default)]
    pub family: String,
    pub interval: String,
    /// Entry in hypotheses.toml this search belongs to.
    #[serde(default)]
    pub hypothesis: String,
    /// Jev families: the probe whose cached answers the rule reads.
    #[serde(default)]
    pub probe: Option<std::path::PathBuf>,
    /// Run on this venue instead of the research venue (Jev replays live where they were asked).
    #[serde(default)]
    pub venue: Option<String>,
    #[serde(default)]
    pub grid: BTreeMap<String, Vec<f64>>,
    #[serde(default)]
    pub fixed: BTreeMap<String, f64>,
    /// A composed rule (`compose.rs`) instead of a hand-written one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<Rule>,
    /// Lineage of a generated family: a readable name (`g0007`), the family it was
    /// derived from, the operator and lessons that produced it, and why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lessons: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

impl Family {
    pub fn load(path: &Path) -> Result<Family> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Parses and checks a family file; a composed rule is registered so `plan` finds it.
    pub fn parse(text: &str) -> Result<Family> {
        let mut f: Family = toml::from_str(text)?;
        f.check()?;
        Ok(f)
    }

    pub fn check(&mut self) -> Result<()> {
        for (k, v) in &self.grid {
            if v.is_empty() {
                bail!("grid `{k}` is empty");
            }
            if self.fixed.contains_key(k) {
                bail!("`{k}` is both in [grid] and [fixed]");
            }
        }
        if let Some(rule) = &self.rule {
            rule.validate()?;
            let id = rule.id();
            if !self.family.is_empty() && self.family != id {
                bail!("a composed family is named by its rule id: `family` must be `{id}` or left out (got `{}`)", self.family);
            }
            for r in rule.refs() {
                if !self.grid.contains_key(&r) && !self.fixed.contains_key(&r) {
                    bail!("the rule refers to `${r}`, which is in neither [grid] nor [fixed]");
                }
            }
            self.family = compose::register(rule);
        } else if self.family.is_empty() {
            bail!("`family` is missing");
        }
        Ok(())
    }

    /// What a person calls it: the generated name, else the family.
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.family)
    }

    /// Every combination of the grid, in a fixed order (last key varies fastest).
    pub fn specs(&self) -> Vec<Spec> {
        let keys: Vec<&String> = self.grid.keys().collect();
        let total: usize = keys.iter().map(|k| self.grid[*k].len()).product();
        (0..total).map(|mut i| {
            let mut params = self.fixed.clone();
            for k in keys.iter().rev() {
                let vals = &self.grid[*k];
                params.insert((*k).clone(), vals[i % vals.len()]);
                i /= vals.len();
            }
            Spec { family: self.family.clone(), params }
        }).collect()
    }

    /// Indices (into `specs()`) of the specs one grid step away in exactly one parameter.
    pub fn neighbours(&self, index: usize) -> Vec<usize> {
        let sizes: Vec<usize> = self.grid.values().map(Vec::len).collect();
        let mut strides = vec![1usize; sizes.len()];
        for d in (0..sizes.len().saturating_sub(1)).rev() {
            strides[d] = strides[d + 1] * sizes[d + 1];
        }
        let mut out = Vec::new();
        for d in 0..sizes.len() {
            let pos = index / strides[d] % sizes[d];
            if pos > 0 {
                out.push(index - strides[d]);
            }
            if pos + 1 < sizes[d] {
                out.push(index + strides[d]);
            }
        }
        out
    }
}

/// How positions opened by a plan are managed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    /// Time stop: close at the first open at least this many bars after entry.
    pub hold_bars: usize,
    /// Hard stop distance from the entry fill, bps. 0 = none.
    pub stop_bps: f64,
    /// When the time stop comes due and the signal still says the same side, keep the
    /// position instead of closing and reopening it (rebalancing strategies).
    pub extend: bool,
    /// Most positions open at once. 0 = no limit (the gross cap still applies).
    pub max_positions: usize,
}

/// Signals for a whole panel.
#[derive(Debug, Clone)]
pub struct Plan {
    /// `[coin][bar]`: +1 open a long, −1 open a short, 0 nothing.
    pub entry: Vec<Vec<i8>>,
    /// `[coin][bar]`: +1 close a long, −1 close a short, 0 nothing. Empty = no signal exits.
    pub exit: Vec<Vec<i8>>,
    pub policy: Policy,
}

/// Extra inputs some families need.
#[derive(Debug, Default)]
pub struct Aux {
    /// Jev answers by question id: `[coin][bar]`, NaN where Jev was not asked.
    pub jev: HashMap<String, Vec<Vec<f32>>>,
}

pub const FAMILIES: [&str; 8] = ["meanrev", "trend", "xs_reversal", "funding_carry", "jev_gate", "jev_direction", "hvt", "ensemble"];

pub fn plan(spec: &Spec, p: &Panel, aux: &Aux) -> Result<Plan> {
    match spec.family.as_str() {
        "meanrev" => meanrev(spec, p),
        "trend" => trend(spec, p),
        "xs_reversal" => xs_reversal(spec, p),
        "funding_carry" => funding_carry(spec, p),
        "jev_gate" => jev_gate(spec, p, aux),
        "jev_direction" => jev_direction(spec, p, aux),
        "hvt" => hvt(spec, p),
        "ensemble" => ensemble(spec, p, aux),
        other => match compose::lookup(other) {
            Some(rule) => compose::plan(&rule, spec, p),
            None if compose::is_composed_id(other) => bail!("composed rule `{other}` is not loaded: load its family file or the promoted strategy that embeds it"),
            None => bail!("unknown strategy family `{other}` (known: {}, or a composed rule)", FAMILIES.join(", ")),
        },
    }
}

pub(crate) fn empty(p: &Panel) -> Vec<Vec<i8>> {
    vec![vec![0i8; p.n]; p.coins.len()]
}

/// z of `x[i]` against the `n` values before it (population std), as the scalper
/// computes it (`scalp::features::revert_z`). NaN until `n` finite values precede it.
pub fn rolling_z(x: &[f64], n: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; x.len()];
    if n < 2 {
        return out;
    }
    // Scale by the first finite value: z doesn't change and the sums stay well conditioned.
    let scale = x.iter().copied().find(|v| v.is_finite() && *v != 0.0).unwrap_or(1.0).abs();
    let (mut s, mut q, mut k) = (0.0f64, 0.0f64, 0usize);
    for i in 0..x.len() {
        // The window is x[i-n..i]: complete when all n are finite.
        if k == n && x[i].is_finite() {
            let m = s / n as f64;
            let var = q / n as f64 - m * m;
            if var > 1e-18 {
                out[i] = (x[i] / scale - m) / var.sqrt();
            }
        }
        let v = x[i] / scale;
        if v.is_finite() {
            s += v;
            q += v * v;
            k += 1;
        }
        if i >= n {
            let old = x[i - n] / scale;
            if old.is_finite() {
                s -= old;
                q -= old * old;
                k -= 1;
            }
        }
    }
    out
}

/// Fade a close stretched `z` standard deviations from the mean of the previous `n`
/// closes (H-R01, generalised). `resid = 1` measures the stretch against the basket
/// of all coins instead of the coin's own price, so market-wide moves don't count.
/// `exit_mean = 1` also closes when the stretch is gone (z back across 0).
fn meanrev(spec: &Spec, p: &Panel) -> Result<Plan> {
    let n = spec.need("n")? as usize;
    let z = spec.need("z")?;
    let hold = spec.need("hold")? as usize;
    let (resid, shorts, exit_mean) = (spec.get("resid", 0.0) > 0.5, spec.get("shorts", 1.0) > 0.5, spec.get("exit_mean", 0.0) > 0.5);
    if n < 2 || z <= 0.0 || hold == 0 {
        bail!("meanrev: need n >= 2, z > 0, hold >= 1");
    }
    let mut entry = empty(p);
    let mut exit = if exit_mean { empty(p) } else { Vec::new() };
    let basket: Vec<f64> = if resid {
        let mut acc = 0.0;
        p.basket_ret.iter().map(|r| {
            acc += r;
            acc
        }).collect()
    } else {
        Vec::new()
    };
    for c in 0..p.coins.len() {
        let x: Vec<f64> = if resid { (0..p.n).map(|i| p.close[c][i].ln() - basket[i]).collect() } else { p.close[c].clone() };
        let zs = rolling_z(&x, n);
        for i in 0..p.n {
            let v = zs[i];
            if !v.is_finite() {
                continue;
            }
            if v <= -z {
                entry[c][i] = 1;
            } else if v >= z && shorts {
                entry[c][i] = -1;
            }
            if exit_mean {
                exit[c][i] = if v >= 0.0 { 1 } else { -1 };
            }
        }
    }
    Ok(Plan { entry, exit, policy: Policy { hold_bars: hold, stop_bps: spec.get("stop", 0.0), extend: false, max_positions: spec.get("max_positions", 0.0) as usize } })
}

/// Rolling max of the previous `n` values (not including the current one).
pub(crate) fn prev_extreme(x: &[f64], n: usize, max: bool) -> Vec<f64> {
    let mut out = vec![f64::NAN; x.len()];
    let mut dq: VecDeque<usize> = VecDeque::new();
    let mut finite = 0usize;
    for i in 0..x.len() {
        if i >= n && finite == n {
            if let Some(&f) = dq.front() {
                out[i] = x[f];
            }
        }
        if x[i].is_finite() {
            while dq.back().is_some_and(|&b| if max { x[b] <= x[i] } else { x[b] >= x[i] }) {
                dq.pop_back();
            }
            dq.push_back(i);
            finite += 1;
        }
        if i >= n {
            if x[i - n].is_finite() {
                finite -= 1;
            }
            while dq.front().is_some_and(|&f| f <= i - n) {
                dq.pop_front();
            }
        }
    }
    out
}

/// Donchian breakout: buy a close above the high of the previous `n` bars, sell one
/// below their low; leave on a close through the opposite `n × out_frac` channel.
fn trend(spec: &Spec, p: &Panel) -> Result<Plan> {
    let n = spec.need("n")? as usize;
    let m = ((n as f64 * spec.get("out_frac", 0.5)).round() as usize).max(2);
    let shorts = spec.get("shorts", 1.0) > 0.5;
    if n < 4 {
        bail!("trend: need n >= 4");
    }
    let mut entry = empty(p);
    let mut exit = empty(p);
    for c in 0..p.coins.len() {
        let (hi, lo) = (prev_extreme(&p.high[c], n, true), prev_extreme(&p.low[c], n, false));
        let (xhi, xlo) = (prev_extreme(&p.high[c], m, true), prev_extreme(&p.low[c], m, false));
        for i in 0..p.n {
            let px = p.close[c][i];
            if !px.is_finite() {
                continue;
            }
            if px > hi[i] {
                entry[c][i] = 1;
            } else if px < lo[i] && shorts {
                entry[c][i] = -1;
            }
            if px < xlo[i] {
                exit[c][i] = 1;
            } else if px > xhi[i] {
                exit[c][i] = -1;
            }
        }
    }
    let hold = (spec.get("hold_days", 30.0) * p.bars_per_day() as f64) as usize;
    Ok(Plan { entry, exit, policy: Policy { hold_bars: hold.max(1), stop_bps: spec.get("stop", 0.0), extend: false, max_positions: spec.get("max_positions", 0.0) as usize } })
}

/// True on bars whose close lands on a multiple of `every` bars since the epoch, so
/// rebalance times don't depend on where a panel starts.
pub(crate) fn rebalance_bar(p: &Panel, i: usize, every: usize) -> bool {
    ((p.time(i) / p.interval_ms) + 1).rem_euclid(every as i64) == 0
}

/// Longs the `k` lowest and shorts the `k` highest scores (ties broken by coin order).
fn rank_legs(scores: &mut Vec<(f64, usize)>, k: usize, entry: &mut [Vec<i8>], i: usize, long_low: bool, short_side: bool) {
    if scores.len() < 2 * k || k == 0 {
        return;
    }
    scores.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let (low, high) = if long_low { (1i8, -1i8) } else { (-1i8, 1i8) };
    for (j, (_, c)) in scores.iter().enumerate() {
        let side = if j < k { low } else if j >= scores.len() - k { high } else { 0 };
        if side > 0 || (side < 0 && short_side) {
            entry[*c][i] = side;
        }
    }
}

/// Cross-sectional: every `hold` bars rank coins by their return over `look` bars;
/// `dir = 1` buys the `k` losers and sells the `k` winners (reversal), `dir = −1` the
/// opposite (momentum). Market-neutral by construction.
fn xs_reversal(spec: &Spec, p: &Panel) -> Result<Plan> {
    let look = spec.need("look")? as usize;
    let hold = spec.need("hold")? as usize;
    let k = spec.need("k")? as usize;
    let reversal = spec.get("dir", 1.0) > 0.0;
    if look == 0 || hold == 0 || k == 0 {
        bail!("xs_reversal: need look, hold, k >= 1");
    }
    let mut entry = empty(p);
    let mut scores = Vec::new();
    for i in look..p.n {
        if !rebalance_bar(p, i, hold) {
            continue;
        }
        scores.clear();
        for c in 0..p.coins.len() {
            let (a, b) = (p.close[c][i - look], p.close[c][i]);
            if p.active[c][i] && a.is_finite() && b.is_finite() {
                scores.push((b / a - 1.0, c));
            }
        }
        rank_legs(&mut scores, k, &mut entry, i, reversal, true);
    }
    Ok(Plan { entry, exit: Vec::new(), policy: Policy { hold_bars: hold, stop_bps: spec.get("stop", 0.0), extend: true, max_positions: 0 } })
}

/// Funding carry: every `hold` bars rank coins by their mean funding over the last
/// `look` hours; short the `k` that pay shorts the most, and (`mode = 0`) long the
/// `k` that charge longs the least. `mode = 1` trades the short leg only.
fn funding_carry(spec: &Spec, p: &Panel) -> Result<Plan> {
    let look_ms = (spec.need("look")? * 3_600_000.0) as i64;
    let hold = spec.need("hold")? as usize;
    let k = spec.need("k")? as usize;
    let short_only = spec.get("mode", 0.0) > 0.5;
    if look_ms <= 0 || hold == 0 || k == 0 {
        bail!("funding_carry: need look, hold, k >= 1");
    }
    let mut entry = empty(p);
    let nc = p.coins.len();
    // Per coin: prints inside (t − look, t], kept as a running sum with two cursors.
    let (mut head, mut tail, mut sum) = (vec![0usize; nc], vec![0usize; nc], vec![0.0f64; nc]);
    let mut scores = Vec::new();
    for i in 0..p.n {
        let t = p.time(i) + p.interval_ms; // the bar's close
        for c in 0..nc {
            let f = &p.funding[c];
            while head[c] < f.len() && f[head[c]].0 <= t {
                sum[c] += f[head[c]].1;
                head[c] += 1;
            }
            while tail[c] < head[c] && f[tail[c]].0 <= t - look_ms {
                sum[c] -= f[tail[c]].1;
                tail[c] += 1;
            }
        }
        if !rebalance_bar(p, i, hold) {
            continue;
        }
        scores.clear();
        for c in 0..nc {
            if p.active[c][i] && head[c] > tail[c] {
                scores.push((sum[c], c));
            }
        }
        if short_only {
            // Highest positive funding only.
            scores.retain(|(s, _)| *s > 0.0);
            scores.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
            for (_, c) in scores.iter().take(k) {
                entry[*c][i] = -1;
            }
        } else {
            rank_legs(&mut scores, k, &mut entry, i, true, true);
        }
    }
    Ok(Plan { entry, exit: Vec::new(), policy: Policy { hold_bars: hold, stop_bps: spec.get("stop", 0.0), extend: true, max_positions: 0 } })
}

fn jev_series<'a>(aux: &'a Aux, q: &str, p: &Panel) -> Result<&'a Vec<Vec<f32>>> {
    let s = aux.jev.get(q).with_context(|| format!("no Jev answers for question `{q}`: run `jev-trader lab jev replay` first"))?;
    // Longer is fine: a panel cut with `head` keeps its bar numbering.
    if s.len() != p.coins.len() || s.iter().any(|v| v.len() < p.n) {
        bail!("Jev answers for `{q}` don't match this panel");
    }
    Ok(s)
}

/// The mean-reversion rule, taken only when Jev's `p_revert` answer for that bar is
/// at least `min_p`. Bars Jev was never asked about are not traded.
fn jev_gate(spec: &Spec, p: &Panel, aux: &Aux) -> Result<Plan> {
    let min_p = spec.need("min_p")? as f32;
    let ans = jev_series(aux, "p_revert", p)?;
    let mut plan = meanrev(spec, p)?;
    for c in 0..p.coins.len() {
        for i in 0..p.n {
            if plan.entry[c][i] != 0 && !(ans[c][i] >= min_p) {
                plan.entry[c][i] = 0;
            }
        }
    }
    Ok(plan)
}

/// Jev's own call: long when its `p_up` answer is at least `long_p`, short when at
/// most `1 − long_p`; held `hold` bars.
fn jev_direction(spec: &Spec, p: &Panel, aux: &Aux) -> Result<Plan> {
    let long_p = spec.need("long_p")? as f32;
    let hold = spec.need("hold")? as usize;
    let shorts = spec.get("shorts", 1.0) > 0.5;
    let ans = jev_series(aux, "p_up", p)?;
    let mut entry = empty(p);
    for c in 0..p.coins.len() {
        for i in 0..p.n {
            let a = ans[c][i];
            if a >= long_p {
                entry[c][i] = 1;
            } else if a <= 1.0 - long_p && shorts {
                entry[c][i] = -1;
            }
        }
    }
    Ok(Plan { entry, exit: Vec::new(), policy: Policy { hold_bars: hold.max(1), stop_bps: spec.get("stop", 0.0), extend: false, max_positions: 0 } })
}

/// HVT (High Volume/Volatility Trading): volume-weighted mean reversion / breakout.
/// - Volume-weighted VWAP as fair value anchor.
/// - Volume-weighted z-score: stretch measured in volume-adjusted std dev.
/// - Volume spike filter: only trade when volume > vol_mult × rolling median volume.
/// - dir = 1: fade (mean-rev) when |z| ≥ z_thresh on high volume.
/// - dir = -1: breakout (momentum) when z ≥ z_thresh on volume surge.
/// - exit_mean = 1: also close when volume-weighted z crosses back through 0.
fn hvt(spec: &Spec, p: &Panel) -> Result<Plan> {
    let vol_look = spec.need("vol_look")? as usize;
    let z_thresh = spec.need("z")?;
    let vol_mult = spec.need("vol_mult")?;
    let hold = spec.need("hold")? as usize;
    let dir = spec.get("dir", 1.0);
    let fade_mode = dir > 0.0;
    let exit_mean = spec.get("exit_mean", 0.0) > 0.5;
    let shorts = spec.get("shorts", 1.0) > 0.5;
    let resid = spec.get("resid", 0.0) > 0.5;

    if vol_look < 8 || z_thresh <= 0.0 || vol_mult <= 1.0 || hold == 0 {
        bail!("hvt: need vol_look >= 8, z > 0, vol_mult > 1, hold >= 1");
    }

    let nc = p.coins.len();
    let n = p.n;
    let mut entry = empty(p);
    let mut exit = if exit_mean && fade_mode { empty(p) } else { Vec::new() };

    // Precompute per-coin: volume median, VWAP, volume-weighted mean & std
    let mut vwap = vec![vec![f64::NAN; n]; nc];
    let mut vw_mean = vec![vec![f64::NAN; n]; nc];
    let mut vw_std = vec![vec![f64::NAN; n]; nc];
    let mut vol_median = vec![vec![f64::NAN; n]; nc];

    for c in 0..nc {
        let mut vol_window: Vec<f64> = Vec::new();
        let mut pv_sum = 0.0; // price * volume
        let mut v_sum = 0.0;
        let mut pvv_sum = 0.0; // (price - vwap)^2 * volume
        let mut pv2_sum = 0.0; // price^2 * volume
        let mut pv_hist: VecDeque<(f64, f64)> = VecDeque::new(); // (price, volume) for rolling VWAP
        let mut vol_hist: VecDeque<f64> = VecDeque::new();

        for i in 0..n {
            let px = p.close[c][i];
            let vol = p.volume[c][i];

            // Rolling VWAP
            if px.is_finite() && vol.is_finite() && vol > 0.0 {
                pv_sum += px * vol;
                v_sum += vol;
                pv_hist.push_back((px, vol));
                pvv_sum += px * px * vol;
            }
            while pv_hist.len() > vol_look {
                let (old_px, old_vol) = pv_hist.pop_front().unwrap();
                pv_sum -= old_px * old_vol;
                v_sum -= old_vol;
                pvv_sum -= old_px * old_px * old_vol;
            }
            if v_sum > 0.0 {
                vwap[c][i] = pv_sum / v_sum;
                // Volume-weighted variance
                let vw_var = (pvv_sum / v_sum) - vwap[c][i] * vwap[c][i];
                if vw_var > 1e-18 {
                    vw_std[c][i] = vw_var.sqrt();
                }
                vw_mean[c][i] = vwap[c][i];
            }

            // Rolling volume median
            if vol.is_finite() && vol > 0.0 {
                vol_hist.push_back(vol);
            }
            while vol_hist.len() > vol_look {
                vol_hist.pop_front();
            }
            if vol_hist.len() >= 8 {
                let mut sorted: Vec<f64> = vol_hist.iter().copied().collect();
                sorted.sort_by(f64::total_cmp);
                vol_median[c][i] = sorted[sorted.len() / 2];
            }
        }
    }

    // Basket for residual mode
    let basket: Vec<f64> = if resid {
        let mut acc = 0.0;
        p.basket_ret.iter().map(|r| { acc += r; acc }).collect()
    } else {
        Vec::new()
    };

    for c in 0..nc {
        for i in 0..n {
            let px = p.close[c][i];
            if !px.is_finite() {
                continue;
            }
            let vw = vwap[c][i];
            let std = vw_std[c][i];
            let vmed = vol_median[c][i];
            let vol = p.volume[c][i];

            if !vw.is_finite() || !std.is_finite() || !vmed.is_finite() || !vol.is_finite() || std <= 1e-12 || vmed <= 0.0 {
                continue;
            }

            // Volume spike condition
            let vol_spike = vol >= vol_mult * vmed;

            // Volume-weighted z-score (or residual vs basket)
            let z = if resid {
                let log_px = px.ln();
                let basket_val = basket[i];
                (log_px - basket_val - vw + vwap[c][i]) / std // simplified: stretch of residual vs own VWAP
            } else {
                (px - vw) / std
            };

            if !z.is_finite() {
                continue;
            }

            if fade_mode {
                // Mean-reversion fade
                if vol_spike && z <= -z_thresh {
                    entry[c][i] = 1;
                } else if vol_spike && z >= z_thresh && shorts {
                    entry[c][i] = -1;
                }
                if exit_mean {
                    exit[c][i] = if z >= 0.0 { 1 } else { -1 };
                }
            } else {
                // Breakout / momentum
                if vol_spike && z >= z_thresh {
                    entry[c][i] = 1;
                } else if vol_spike && z <= -z_thresh && shorts {
                    entry[c][i] = -1;
                }
            }
        }
    }

    Ok(Plan {
        entry,
        exit,
        policy: Policy {
            hold_bars: hold,
            stop_bps: spec.get("stop", 0.0),
            extend: false,
            max_positions: spec.get("max_positions", 0.0) as usize,
        },
    })
}

/// Ensemble: combines signals from multiple sub-families with learned weights.
/// - Sub-families are listed in the family file's `sub_families` (fixed, not in grid).
/// - On each rebalance bar, trains weights on the previous `train_days` days.
/// - Weighting methods: "vote" (weighted sign), "sharpe" (proportional to in-sample Sharpe).
/// - Final signal = sign(sum(weight_f * signal_f)) per coin/bar.
fn ensemble(spec: &Spec, p: &Panel, aux: &Aux) -> Result<Plan> {
    // Which sub-families to include. Each is a 0/1 flag `use_<family>` in [fixed].
    // When any flag is present, only flagged families are used. When none is present we
    // fall back to every data-only family — never the Jev ones, whose signals need a
    // replay cache the caller may not have populated.
    let known = [
        "meanrev",
        "trend",
        "xs_reversal",
        "funding_carry",
        "jev_gate",
        "jev_direction",
        "hvt",
    ];
    let has_flags = known
        .iter()
        .any(|f| spec.params.contains_key(&format!("use_{f}")));
    let sub_families: Vec<String> = if has_flags {
        known
            .iter()
            .filter(|f| spec.get(&format!("use_{f}"), 0.0) > 0.5)
            .map(|f| (*f).to_string())
            .collect()
    } else {
        vec![
            "meanrev".into(),
            "trend".into(),
            "xs_reversal".into(),
            "funding_carry".into(),
            "hvt".into(),
        ]
    };
    if sub_families.is_empty() {
        bail!("ensemble: no sub-families selected (set use_<family> = 1 in [fixed])");
    }

    let train_days = spec.need("train_days")? as usize;
    let rebalance = spec.need("rebalance")? as usize;
    let min_weight = spec.get("min_weight", 0.1);
    let combine = spec.get("combine", 1.0); // 1 = vote, 2 = sharpe
    let hold = spec.need("hold")? as usize;
    let shorts = spec.get("shorts", 1.0) > 0.5;
    let exit_on_flip = spec.get("exit_on_flip", 1.0) > 0.5;
    // Diagnostic: invert the consensus. Used to tell a reliably-wrong edge (flipping it
    // turns a persistent loss into a persistent gain) from a broken simulator.
    let flip = spec.get("flip", 0.0) > 0.5;
    // How many sub-families must agree on a side before the ensemble opens a trade.
    let min_agree = spec.get("min_agree", 1.0).max(1.0) as usize;

    if train_days == 0 || rebalance == 0 || hold == 0 {
        bail!("ensemble: need train_days > 0, rebalance > 0, hold >= 1");
    }

    // Build plans for each sub-family using their fixed params from this ensemble spec
    // (In practice, sub-families have their own fixed params; here we use defaults from the ensemble's fixed)
    let mut sub_plans: Vec<Plan> = Vec::new();
    for fam_name in &sub_families {
        // Create a spec for this sub-family using the ensemble's fixed params as defaults
        // This is a simplification; real use would load each family's own fixed params
        let mut sub_spec = Spec {
            family: fam_name.clone(),
            params: BTreeMap::new(),
        };
        // Copy relevant fixed params from ensemble spec
        for (k, v) in &spec.params {
            if !["sub_families", "train_days", "rebalance", "min_weight", "combine", "hold", "stop", "max_positions", "shorts", "exit_on_flip"].contains(&k.as_str()) {
                sub_spec.params.insert(k.clone(), *v);
            }
        }
        // Ensure required params exist (use sensible defaults)
        match fam_name.as_str() {
            "meanrev" => {
                sub_spec.params.entry("n".to_string()).or_insert(288.0);
                sub_spec.params.entry("z".to_string()).or_insert(2.0);
                sub_spec.params.entry("hold".to_string()).or_insert(24.0);
                sub_spec.params.entry("stop".to_string()).or_insert(500.0);
                sub_spec.params.entry("shorts".to_string()).or_insert(1.0);
                sub_spec.params.entry("exit_mean".to_string()).or_insert(1.0);
                sub_spec.params.entry("resid".to_string()).or_insert(0.0);
                sub_spec.params.entry("max_positions".to_string()).or_insert(0.0);
            }
            "trend" => {
                sub_spec.params.entry("n".to_string()).or_insert(96.0);
                sub_spec.params.entry("out_frac".to_string()).or_insert(0.5);
                sub_spec.params.entry("stop".to_string()).or_insert(0.0);
                sub_spec.params.entry("shorts".to_string()).or_insert(1.0);
                sub_spec.params.entry("hold_days".to_string()).or_insert(30.0);
                sub_spec.params.entry("max_positions".to_string()).or_insert(0.0);
            }
            "xs_reversal" => {
                sub_spec.params.entry("look".to_string()).or_insert(24.0);
                sub_spec.params.entry("hold".to_string()).or_insert(12.0);
                sub_spec.params.entry("k".to_string()).or_insert(2.0);
                sub_spec.params.entry("dir".to_string()).or_insert(1.0);
                sub_spec.params.entry("stop".to_string()).or_insert(0.0);
            }
            "funding_carry" => {
                sub_spec.params.entry("look".to_string()).or_insert(24.0);
                sub_spec.params.entry("hold".to_string()).or_insert(8.0);
                sub_spec.params.entry("k".to_string()).or_insert(2.0);
                sub_spec.params.entry("mode".to_string()).or_insert(0.0);
                sub_spec.params.entry("stop".to_string()).or_insert(0.0);
            }
            "jev_gate" => {
                sub_spec.params.entry("n".to_string()).or_insert(288.0);
                sub_spec.params.entry("z".to_string()).or_insert(2.0);
                sub_spec.params.entry("hold".to_string()).or_insert(24.0);
                sub_spec.params.entry("stop".to_string()).or_insert(500.0);
                sub_spec.params.entry("min_p".to_string()).or_insert(0.6);
            }
            "jev_direction" => {
                sub_spec.params.entry("long_p".to_string()).or_insert(0.7);
                sub_spec.params.entry("hold".to_string()).or_insert(24.0);
                sub_spec.params.entry("stop".to_string()).or_insert(0.0);
                sub_spec.params.entry("shorts".to_string()).or_insert(1.0);
            }
            "hvt" => {
                sub_spec.params.entry("vol_look".to_string()).or_insert(96.0);
                sub_spec.params.entry("z".to_string()).or_insert(2.0);
                sub_spec.params.entry("vol_mult".to_string()).or_insert(2.0);
                sub_spec.params.entry("hold".to_string()).or_insert(24.0);
                sub_spec.params.entry("stop".to_string()).or_insert(300.0);
                sub_spec.params.entry("dir".to_string()).or_insert(1.0);
                sub_spec.params.entry("exit_mean".to_string()).or_insert(1.0);
                sub_spec.params.entry("resid".to_string()).or_insert(0.0);
                sub_spec.params.entry("shorts".to_string()).or_insert(1.0);
                sub_spec.params.entry("max_positions".to_string()).or_insert(0.0);
            }
            _ => {}
        }
        let plan = crate::lab::strategy::plan(&sub_spec, p, aux)?;
        sub_plans.push(plan);
    }

    let nc = p.coins.len();
    let n = p.n;
    let bpd = p.bars_per_day();
    let train_bars = train_days * bpd;
    let mut entry = empty(p);
    let mut exit = if exit_on_flip { empty(p) } else { Vec::new() };

    // Current weights per sub-family
    let mut weights = vec![1.0 / sub_plans.len() as f64; sub_plans.len()];

    for i in 0..n {
        // Rebalance weights every `rebalance` bars (after we have training data)
        if i >= train_bars && i % rebalance == 0 {
            // Score each sub-family on the training window [train_start, i). A trade
            // opened at bar b is scored on its close-to-close return over `hold` bars;
            // only bars whose exit lands at or before `i` are used, so the weights never
            // see a return past the rebalance bar.
            let train_start = i - train_bars;
            let last_scored = i.saturating_sub(hold + 1);
            let mut perf = vec![0.0; sub_plans.len()];

            for (si, splan) in sub_plans.iter().enumerate() {
                let mut sum = 0.0;
                let mut sum_sq = 0.0;
                let mut cnt = 0usize;
                for b in train_start..=last_scored {
                    let mut bar_return = 0.0;
                    let mut legs = 0usize;
                    for c in 0..nc {
                        let e = splan.entry[c][b];
                        if e == 0 {
                            continue;
                        }
                        let exit_idx = b + hold;
                        let (px0, px1) = (p.close[c][b], p.close[c][exit_idx]);
                        if px0.is_finite() && px1.is_finite() && px0 > 0.0 {
                            bar_return += (px1 / px0 - 1.0) * e as f64;
                            legs += 1;
                        }
                    }
                    if legs > 0 {
                        let r = bar_return / legs as f64;
                        sum += r;
                        sum_sq += r * r;
                        cnt += 1;
                    }
                }
                if cnt > 1 {
                    let mean = sum / cnt as f64;
                    let var = (sum_sq / cnt as f64 - mean * mean).max(0.0);
                    let sd = var.sqrt();
                    perf[si] = if sd > 1e-12 { mean / sd } else { 0.0 };
                }
            }

            // Update weights
            if combine >= 1.5 {
                // Sharpe-weighting: positive Sharpe gets weight proportional to Sharpe
                let pos_sum: f64 = perf.iter().filter(|x| **x > 0.0).sum();
                if pos_sum > 0.0 {
                    for si in 0..sub_plans.len() {
                        weights[si] = if perf[si] > 0.0 { perf[si] / pos_sum } else { 0.0 };
                    }
                }
            } else {
                // Vote weighting: equal weight to positive performers
                let pos_count = perf.iter().filter(|x| **x > 0.0).count().max(1) as f64;
                for si in 0..sub_plans.len() {
                    weights[si] = if perf[si] > 0.0 { 1.0 / pos_count } else { 0.0 };
                }
            }

            // Apply min_weight floor and renormalize
            let mut total = 0.0;
            for w in &mut weights {
                if *w < min_weight {
                    *w = 0.0;
                }
                total += *w;
            }
            if total > 0.0 {
                for w in &mut weights {
                    *w /= total;
                }
            } else {
                // All zero: fall back to equal
                for w in &mut weights {
                    *w = 1.0 / sub_plans.len() as f64;
                }
            }
        }

        // Generate ensemble signal at this bar. The consensus is a position target:
        // enter only when it switches on (or flips side) so the hold/stop policy owns
        // the trade instead of re-entering every bar. `min_agree` demands that at least
        // that many sub-families point the same way — without it a single noisy family
        // can open a trade, and the turnover eats the edge.
        for c in 0..nc {
            let mut weighted_sum = 0.0;
            let mut prev_sum = 0.0;
            let (mut long_n, mut short_n) = (0usize, 0usize);
            for (si, splan) in sub_plans.iter().enumerate() {
                let e = splan.entry[c][i];
                if e > 0 {
                    long_n += 1;
                    weighted_sum += weights[si];
                } else if e < 0 {
                    short_n += 1;
                    weighted_sum -= weights[si];
                }
                if i > 0 {
                    let pv = splan.entry[c][i - 1];
                    if pv != 0 {
                        prev_sum += weights[si] * pv as f64;
                    }
                }
            }
            if flip {
                weighted_sum = -weighted_sum;
                prev_sum = -prev_sum;
            }
            let cur = if weighted_sum > 0.0 {
                1i8
            } else if weighted_sum < 0.0 {
                -1i8
            } else {
                0i8
            };
            let prev = if prev_sum > 0.0 {
                1i8
            } else if prev_sum < 0.0 {
                -1i8
            } else {
                0i8
            };
            let agree = if cur > 0 {
                long_n
            } else if cur < 0 {
                short_n
            } else {
                0
            };

            if cur != 0 && cur != prev && agree >= min_agree && (cur > 0 || shorts) {
                entry[c][i] = cur;
            }
            if exit_on_flip && prev != 0 && cur != prev {
                exit[c][i] = prev;
            }
        }
    }

    Ok(Plan {
        entry,
        exit,
        policy: Policy {
            hold_bars: hold,
            stop_bps: spec.get("stop", 0.0),
            extend: false,
            max_positions: spec.get("max_positions", 0.0) as usize,
        },
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::panel::testing::*;
    use super::*;
    use crate::scalp::features::revert_z;

    fn family(text: &str) -> Family {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn grid_enumerates_and_finds_neighbours() {
        let f = family("family = \"meanrev\"\ninterval = \"15m\"\n[grid]\nn = [96, 288]\nz = [1.5, 2.0, 2.5]\n[fixed]\nhold = 24\n");
        let specs = f.specs();
        assert_eq!(specs.len(), 6);
        assert_eq!(specs[0].id(), "meanrev:hold=24,n=96,z=1.5");
        assert_eq!(specs[5].id(), "meanrev:hold=24,n=288,z=2.5");
        // (n=96, z=2.0) is index 1: neighbours are z=1.5, z=2.5 and n=288.
        let mut nb = f.neighbours(1);
        nb.sort_unstable();
        assert_eq!(nb, vec![0, 2, 4]);
        assert_eq!(f.neighbours(0).len(), 2);
        let ids: std::collections::HashSet<String> = specs.iter().map(Spec::id).collect();
        assert_eq!(ids.len(), 6, "ids are unique");
    }

    #[test]
    fn rolling_z_matches_the_scalper() {
        let p = random_panel(11, 1, 6, "15m");
        let x = &p.close[0];
        let zs = rolling_z(x, 96);
        for i in [96usize, 97, 200, 400, x.len() - 1] {
            let want = revert_z(&x[..=i], 96).unwrap();
            assert!((zs[i] - want).abs() < 1e-7, "bar {i}: {} vs {want}", zs[i]);
        }
        assert!(zs[95].is_nan());
        // A gap blanks the window until it is whole again.
        let mut gap = x.clone();
        gap[150] = f64::NAN;
        let zg = rolling_z(&gap, 96);
        assert!(zg[150].is_nan() && zg[151].is_nan() && zg[246].is_nan());
        assert!(zg[247].is_finite());
    }

    #[test]
    fn previous_extremes_exclude_the_current_bar() {
        let x = [1.0, 5.0, 3.0, 4.0, 9.0, 2.0];
        let hi = prev_extreme(&x, 3, true);
        assert!(hi[2].is_nan());
        assert_eq!(&hi[3..], &[5.0, 5.0, 9.0]);
        let lo = prev_extreme(&x, 3, false);
        assert_eq!(&lo[3..], &[1.0, 3.0, 3.0]);
    }

    #[test]
    fn meanrev_fades_and_respects_shorts() {
        let p = random_panel(5, 3, 20, "15m");
        let s = Spec::new("meanrev", &[("n", 96.0), ("z", 2.0), ("hold", 24.0), ("stop", 500.0)]);
        let pl = plan(&s, &p, &Aux::default()).unwrap();
        let z = rolling_z(&p.close[1], 96);
        let (mut longs, mut shorts) = (0, 0);
        for i in 0..p.n {
            match pl.entry[1][i] {
                1 => {
                    assert!(z[i] <= -2.0);
                    longs += 1;
                }
                -1 => {
                    assert!(z[i] >= 2.0);
                    shorts += 1;
                }
                _ => assert!(!(z[i].abs() >= 2.0)),
            }
        }
        assert!(longs > 5 && shorts > 5, "{longs} longs, {shorts} shorts");
        assert_eq!(pl.policy, Policy { hold_bars: 24, stop_bps: 500.0, extend: false, max_positions: 0 });
        let long_only = Spec::new("meanrev", &[("n", 96.0), ("z", 2.0), ("hold", 24.0), ("shorts", 0.0)]);
        assert!(plan(&long_only, &p, &Aux::default()).unwrap().entry.iter().flatten().all(|e| *e >= 0));
    }

    #[test]
    fn cross_section_is_balanced_and_rebalances_on_the_clock() {
        let p = random_panel(8, 8, 20, "1h");
        let s = Spec::new("xs_reversal", &[("look", 24.0), ("hold", 12.0), ("k", 2.0), ("dir", 1.0)]);
        let pl = plan(&s, &p, &Aux::default()).unwrap();
        let mut rebalances = 0;
        for i in 0..p.n {
            let longs = (0..8).filter(|c| pl.entry[*c][i] == 1).count();
            let shorts = (0..8).filter(|c| pl.entry[*c][i] == -1).count();
            if longs + shorts > 0 {
                rebalances += 1;
                assert_eq!((longs, shorts), (2, 2));
                assert_eq!((p.time(i) / p.interval_ms + 1) % 12, 0);
                // Longs are the losers.
                let ret = |c: usize| p.close[c][i] / p.close[c][i - 24] - 1.0;
                let worst_long = (0..8).filter(|c| pl.entry[*c][i] == 1).map(ret).fold(f64::MIN, f64::max);
                let best_short = (0..8).filter(|c| pl.entry[*c][i] == -1).map(ret).fold(f64::MAX, f64::min);
                assert!(worst_long < best_short);
            }
        }
        assert!(rebalances >= 35, "{rebalances}");
        // Momentum is the mirror image.
        let mom = plan(&Spec::new("xs_reversal", &[("look", 24.0), ("hold", 12.0), ("k", 2.0), ("dir", -1.0)]), &p, &Aux::default()).unwrap();
        for c in 0..8 {
            for i in 0..p.n {
                assert_eq!(mom.entry[c][i], -pl.entry[c][i]);
            }
        }
    }

    #[test]
    fn carry_shorts_the_coin_that_pays_shorts() {
        let mut p = random_panel(2, 4, 10, "1h");
        // Coin 0 charges longs heavily, coin 3 pays them; hourly prints.
        p.funding = (0..4).map(|c| (0..p.n as i64).map(|i| (i * 3_600_000, [0.0005, 0.0001, 0.00005, -0.0003][c])).collect()).collect();
        let pl = plan(&Spec::new("funding_carry", &[("look", 24.0), ("hold", 8.0), ("k", 1.0)]), &p, &Aux::default()).unwrap();
        let i = (0..p.n).find(|i| pl.entry[0][*i] != 0).unwrap();
        assert_eq!((pl.entry[0][i], pl.entry[3][i], pl.entry[1][i]), (-1, 1, 0));
        let short_only = plan(&Spec::new("funding_carry", &[("look", 24.0), ("hold", 8.0), ("k", 1.0), ("mode", 1.0)]), &p, &Aux::default()).unwrap();
        assert!(short_only.entry.iter().flatten().all(|e| *e <= 0));
        assert!(short_only.entry[0].iter().any(|e| *e == -1));
    }

    #[test]
    fn jev_gate_only_keeps_answered_bars() {
        let p = random_panel(5, 2, 20, "15m");
        let base = Spec::new("meanrev", &[("n", 96.0), ("z", 2.0), ("hold", 24.0)]);
        let raw = plan(&base, &p, &Aux::default()).unwrap();
        let mut aux = Aux::default();
        // Jev was asked on coin 0 only, and liked every other candidate.
        let mut ans = vec![vec![f32::NAN; p.n]; 2];
        let mut flip = false;
        for i in 0..p.n {
            if raw.entry[0][i] != 0 {
                ans[0][i] = if flip { 0.8 } else { 0.3 };
                flip = !flip;
            }
        }
        aux.jev.insert("p_revert".into(), ans);
        let gated = plan(&Spec::new("jev_gate", &[("n", 96.0), ("z", 2.0), ("hold", 24.0), ("min_p", 0.6)]), &p, &aux).unwrap();
        let count = |pl: &Plan, c: usize| pl.entry[c].iter().filter(|e| **e != 0).count();
        assert!(count(&gated, 0) > 0 && count(&gated, 0) <= count(&raw, 0) / 2 + 1);
        assert_eq!(count(&gated, 1), 0, "unanswered bars are not traded");
        assert!(plan(&Spec::new("jev_gate", &[("n", 96.0), ("z", 2.0), ("hold", 24.0), ("min_p", 0.6)]), &p, &Aux::default()).is_err());
    }

    /// Every family file on disk (`families/` and `families/gen/`), hand-written or
    /// composed. Families that read Jev answers are left out: they need a replay cache.
    pub(crate) fn repo_families() -> Vec<(std::path::PathBuf, Family)> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("families");
        let mut out = Vec::new();
        for dir in [root.clone(), root.join("gen")] {
            let mut paths: Vec<_> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect();
            paths.sort();
            for path in paths {
                let fam = Family::load(&path).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
                out.push((path, fam));
            }
        }
        out
    }

    /// The contract every family must keep: signals up to a bar don't change when the
    /// future is cut off. A rule that peeks ahead fails here. It runs over every family
    /// file, so a generated family is covered the moment it is written.
    #[test]
    fn plans_do_not_look_ahead() {
        let mut full = random_panel(42, 6, 24, "1h");
        full.funding = (0..6).map(|c| (0..full.n as i64).map(|i| (i * 3_600_000, 0.00001 * ((c as f64 + 1.0) * (i as f64 / 50.0).sin()))).collect()).collect();
        // Volume that moves, so volume rules get to fire.
        let mut rng = crate::stats::Rng::new(7);
        for v in full.volume.iter_mut().flatten() {
            *v *= (0.6 * rng.normal()).exp();
        }
        full.rederive(NO_RULE);
        let mut specs = vec![
            Spec::new("meanrev", &[("n", 48.0), ("z", 1.5), ("hold", 12.0)]),
            Spec::new("meanrev", &[("n", 48.0), ("z", 1.5), ("hold", 12.0), ("resid", 1.0), ("exit_mean", 1.0)]),
            Spec::new("trend", &[("n", 48.0), ("out_frac", 0.5)]),
            Spec::new("xs_reversal", &[("look", 24.0), ("hold", 6.0), ("k", 2.0)]),
            Spec::new("funding_carry", &[("look", 48.0), ("hold", 8.0), ("k", 2.0)]),
        ];
        // A few specs spread over each family's grid.
        let mut families = Vec::new();
        for (path, fam) in repo_families() {
            if fam.probe.is_some() {
                continue;
            }
            let all = fam.specs();
            let pick: Vec<Spec> = (0..4).map(|k| all[k * (all.len() - 1) / 3].clone()).collect();
            families.push((path.display().to_string(), specs.len(), pick.len()));
            specs.extend(pick);
        }
        assert!(families.len() >= 4);
        let mut traded = vec![false; specs.len()];
        for cut in [24 * 9 + 7, 24 * 17] {
            let mut head = full.head(cut);
            head.rederive(NO_RULE);
            for (k, s) in specs.iter().enumerate() {
                let (a, b) = (plan(s, &full, &Aux::default()).unwrap(), plan(s, &head, &Aux::default()).unwrap());
                for c in 0..6 {
                    assert_eq!(&a.entry[c][..cut], &b.entry[c][..], "{} entry, coin {c}, cut {cut}", s.id());
                    if !a.exit.is_empty() {
                        assert_eq!(&a.exit[c][..cut], &b.exit[c][..], "{} exit, coin {c}", s.id());
                    }
                }
                traded[k] |= a.entry.iter().flatten().any(|e| *e != 0);
            }
        }
        for (k, s) in specs.iter().enumerate().take(5) {
            assert!(traded[k], "{} never trades in the test panel", s.id());
        }
        // A family whose sampled specs never trade here was not really checked.
        for (path, from, n) in families {
            assert!(traded[from..from + n].iter().any(|t| *t), "{path}: no sampled spec trades in the test panel, so look-ahead was not checked");
        }
    }
}
