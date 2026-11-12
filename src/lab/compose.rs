//! Composed rules: strategies built from a small, closed library of blocks instead of
//! a hand-written function, so that new strategies can be written as data (by a
//! person or by `lab propose`) and still run through the one `plan` → simulator /
//! book path.
//!
//! A rule is a set of entry conditions for the long side (all must hold), an optional
//! mirror for the short side, side-agnostic filters, and exit conditions. Every
//! condition reads one **feature**, a causal per-coin series:
//!
//! | feature        | value at bar `i` (window `n`)                                   | signed |
//! |----------------|-----------------------------------------------------------------|--------|
//! | `ret`          | close / close `n` bars ago − 1                                  | yes |
//! | `z`            | z of the close against the previous `n` closes                  | yes |
//! | `resid_z`      | the same on log price minus the cumulative basket return        | yes |
//! | `channel`      | position in the previous `n` bars' range: −1 at the low, +1 at the high, beyond ±1 outside it | yes |
//! | `vol_ratio`    | volume / mean volume of the previous `n` bars                   | no  |
//! | `rvol`         | std of bar log returns over `n` bars, scaled to a day           | no  |
//! | `funding`      | sum of funding prints in the last `n` hours (longs pay when > 0) | yes |
//! | `xs_ret`       | cross-sectional rank of `ret`, −1 (worst) to +1 (best)          | yes |
//! | `xs_funding`   | cross-sectional rank of `funding`                               | yes |
//! | `mkt_ret`      | the basket's log return over `n` bars (same for every coin)     | yes |
//! | `mkt_rvol_pct` | percentile of the basket's `rvol` among the previous 30 days    | no  |
//!
//! **Mirror.** The short side tests the long conditions on the negated value of a
//! signed feature (`z <= -2` becomes `z >= 2`); unsigned features are tested as they
//! are. Exits mirror the same way. Filters are never mirrored.
//!
//! **Parameters.** A number, or a reference into the spec's parameters: `"$n"`, or a
//! product such as `"$n*$out_frac"` or `"$n*0.5"`. Policy parameters keep the names the
//! hand-written families use: `hold` (bars) or `hold_days` (default 30), `stop` (bps),
//! `shorts` (0/1, default 1), `max_positions`.
//!
//! **Identity.** A rule's id is a hash of its canonical form, so the same rule written
//! twice is recognised as already tried, whatever the file it came from is called.
//! The library is closed on purpose: every block added widens the search.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use anyhow::{Result, bail};
use serde::{Deserialize, Deserializer, Serialize};

use super::panel::Panel;
use super::strategy::{Plan, Policy, Spec, empty, prev_extreme, rebalance_bar, rolling_z};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    Ret,
    Z,
    ResidZ,
    Channel,
    VolRatio,
    Rvol,
    Funding,
    XsRet,
    XsFunding,
    MktRet,
    MktRvolPct,
}

pub const FEATURES: [Feature; 11] = [Feature::Ret, Feature::Z, Feature::ResidZ, Feature::Channel, Feature::VolRatio, Feature::Rvol, Feature::Funding, Feature::XsRet, Feature::XsFunding, Feature::MktRet, Feature::MktRvolPct];

impl Feature {
    /// Signed features change sign on the short side; unsigned ones don't.
    pub fn signed(self) -> bool {
        !matches!(self, Feature::VolRatio | Feature::Rvol | Feature::MktRvolPct)
    }

    pub fn name(self) -> &'static str {
        match self {
            Feature::Ret => "ret",
            Feature::Z => "z",
            Feature::ResidZ => "resid_z",
            Feature::Channel => "channel",
            Feature::VolRatio => "vol_ratio",
            Feature::Rvol => "rvol",
            Feature::Funding => "funding",
            Feature::XsRet => "xs_ret",
            Feature::XsFunding => "xs_funding",
            Feature::MktRet => "mkt_ret",
            Feature::MktRvolPct => "mkt_rvol_pct",
        }
    }

    /// Market-state features: the same value for every coin.
    pub fn market(self) -> bool {
        matches!(self, Feature::MktRet | Feature::MktRvolPct)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = "<=")]
    Le,
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = "<")]
    Lt,
}

impl Op {
    fn test(self, x: f64, level: f64) -> bool {
        match self {
            Op::Ge => x >= level,
            Op::Le => x <= level,
            Op::Gt => x > level,
            Op::Lt => x < level,
        }
    }

    pub fn flipped(self) -> Op {
        match self {
            Op::Ge => Op::Le,
            Op::Le => Op::Ge,
            Op::Gt => Op::Lt,
            Op::Lt => Op::Gt,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Op::Ge => ">=",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Lt => "<",
        }
    }
}

/// A number, or a product of spec parameters and numbers: `"$n"`, `"$n*$out_frac"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Param {
    Num(f64),
    Ref(String),
}

impl Param {
    pub fn resolve(&self, spec: &Spec) -> Result<f64> {
        match self {
            Param::Num(x) => Ok(*x),
            Param::Ref(s) => {
                let mut v = 1.0;
                for f in s.split('*').map(str::trim) {
                    v *= match f.strip_prefix('$') {
                        Some(name) => match spec.params.get(name) {
                            Some(x) => *x,
                            None => bail!("{}: rule needs parameter `{name}`", spec.family),
                        },
                        None => f.parse::<f64>().map_err(|_| anyhow::anyhow!("bad factor `{f}` in parameter `{s}`"))?,
                    };
                }
                Ok(v)
            }
        }
    }

    /// Parameter names this refers to.
    pub fn refs(&self) -> Vec<String> {
        match self {
            Param::Num(_) => Vec::new(),
            Param::Ref(s) => s.split('*').filter_map(|f| f.trim().strip_prefix('$').map(str::to_string)).collect(),
        }
    }

    pub fn text(&self) -> String {
        match self {
            Param::Num(x) => format!("{x}"),
            Param::Ref(s) => s.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cond {
    pub feature: Feature,
    pub n: Param,
    pub op: Op,
    pub level: Param,
}

impl Cond {
    pub fn text(&self) -> String {
        format!("{}({}) {} {}", self.feature.name(), self.n.text(), self.op.word(), self.level.text())
    }

    /// The same condition seen from the other side (what `mirror` tests for shorts).
    pub fn inverted(&self) -> Cond {
        let level = match &self.level {
            Param::Num(x) if self.feature.signed() => Param::Num(-x),
            Param::Ref(s) if self.feature.signed() => Param::Ref(match s.strip_suffix("*-1") {
                Some(base) => base.to_string(),
                None => format!("{s}*-1"),
            }),
            other => other.clone(),
        };
        Cond { op: self.op.flipped(), level, ..self.clone() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Short {
    /// Shorts test the long conditions mirrored (still subject to `shorts = 0`).
    #[default]
    Mirror,
    /// Long only.
    None,
}

fn one_or_many<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<Cond>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(Cond),
        Many(Vec<Cond>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(c) => vec![c],
        OneOrMany::Many(v) => v,
    })
}

/// A composed rule (the `[rule]` table of a family file).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Open a long when every one of these holds.
    #[serde(deserialize_with = "one_or_many")]
    pub long: Vec<Cond>,
    #[serde(default)]
    pub short: Short,
    /// Must also hold for either side to open. Never mirrored.
    #[serde(default, deserialize_with = "one_or_many")]
    pub filter: Vec<Cond>,
    /// Close a long when any of these holds; mirrored to close a short.
    #[serde(default, deserialize_with = "one_or_many")]
    pub exit: Vec<Cond>,
    /// Also close when the opposite entry signal fires.
    #[serde(default)]
    pub exit_opposite: bool,
    /// Entries only every this many bars (on the clock, as the cross-sectional families do).
    #[serde(default)]
    pub every: Option<Param>,
    /// At the time stop, keep a position whose entry signal still holds.
    #[serde(default)]
    pub extend: bool,
}

/// FNV-1a: stable across Rust versions, unlike `DefaultHasher`.
fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl Rule {
    /// Canonical text: the serialised rule with fixed field order and list form.
    pub fn canonical(&self) -> String {
        serde_json::to_string(self).expect("a rule always serialises")
    }

    /// `c` + 12 hex digits of the canonical form's hash. This is the family name a
    /// composed rule's specs carry, so trials are counted by structure, not by file name.
    pub fn id(&self) -> String {
        format!("c{:012x}", fnv(&self.canonical()) & 0xffff_ffff_ffff)
    }

    pub fn conds(&self) -> impl Iterator<Item = &Cond> {
        self.long.iter().chain(&self.filter).chain(&self.exit)
    }

    /// Every spec parameter the rule refers to.
    pub fn refs(&self) -> Vec<String> {
        let mut out: Vec<String> = self.conds().flat_map(|c| c.n.refs().into_iter().chain(c.level.refs())).chain(self.every.iter().flat_map(Param::refs)).collect();
        out.sort();
        out.dedup();
        out
    }

    pub fn validate(&self) -> Result<()> {
        if self.long.is_empty() {
            bail!("a rule needs at least one `long` condition");
        }
        if self.conds().count() > 8 {
            bail!("a rule may have at most 8 conditions");
        }
        Ok(())
    }

    /// One line a person can read.
    pub fn describe(&self) -> String {
        let j = |v: &[Cond]| v.iter().map(Cond::text).collect::<Vec<_>>().join(" and ");
        let mut s = format!("long when {}", j(&self.long));
        s.push_str(match self.short {
            Short::Mirror => "; short on the mirror",
            Short::None => "; long only",
        });
        if !self.filter.is_empty() {
            s.push_str(&format!("; only when {}", j(&self.filter)));
        }
        if !self.exit.is_empty() {
            s.push_str(&format!("; close a long when {}", self.exit.iter().map(Cond::text).collect::<Vec<_>>().join(" or ")));
        }
        if self.exit_opposite {
            s.push_str("; close on the opposite signal");
        }
        if let Some(e) = &self.every {
            s.push_str(&format!("; entries every {} bars", e.text()));
        }
        s
    }
}

// ---------------------------------------------------------------------------
// Registry: `strategy::plan` looks composed families up here by id.

fn registry() -> &'static RwLock<HashMap<String, Arc<Rule>>> {
    static R: OnceLock<RwLock<HashMap<String, Arc<Rule>>>> = OnceLock::new();
    R.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Makes `rule` known under its id and returns the id. Idempotent.
pub fn register(rule: &Rule) -> String {
    let id = rule.id();
    registry().write().expect("rule registry").entry(id.clone()).or_insert_with(|| Arc::new(rule.clone()));
    id
}

pub fn lookup(id: &str) -> Option<Arc<Rule>> {
    registry().read().expect("rule registry").get(id).cloned()
}

pub fn is_composed_id(id: &str) -> bool {
    id.len() == 13 && id.starts_with('c') && id[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

// ---------------------------------------------------------------------------
// Features.

fn window(n: f64, min: usize) -> usize {
    (n.round().max(0.0) as usize).max(min)
}

/// `[coin][bar]` values of one feature. Causal: bar `i` uses bars `0..=i` only.
pub fn feature(p: &Panel, f: Feature, n: f64) -> Vec<Vec<f64>> {
    let nc = p.coins.len();
    let nan = || vec![f64::NAN; p.n];
    match f {
        Feature::Ret => {
            let k = window(n, 1);
            (0..nc).map(|c| (0..p.n).map(|i| if i >= k { p.close[c][i] / p.close[c][i - k] - 1.0 } else { f64::NAN }).collect()).collect()
        }
        Feature::Z => (0..nc).map(|c| rolling_z(&p.close[c], window(n, 2))).collect(),
        Feature::ResidZ => {
            let mut acc = 0.0;
            let basket: Vec<f64> = p.basket_ret.iter().map(|r| {
                acc += r;
                acc
            }).collect();
            (0..nc).map(|c| rolling_z(&(0..p.n).map(|i| p.close[c][i].ln() - basket[i]).collect::<Vec<_>>(), window(n, 2))).collect()
        }
        Feature::Channel => {
            let k = window(n, 2);
            (0..nc).map(|c| {
                let (hi, lo) = (prev_extreme(&p.high[c], k, true), prev_extreme(&p.low[c], k, false));
                (0..p.n).map(|i| channel_pos(p.close[c][i], hi[i], lo[i])).collect()
            }).collect()
        }
        Feature::VolRatio => {
            let k = window(n, 2);
            (0..nc).map(|c| {
                let v = &p.volume[c];
                let mut out = nan();
                let (mut s, mut cnt) = (0.0f64, 0usize);
                for i in 0..p.n {
                    if i >= k && cnt == k && s > 0.0 && v[i].is_finite() {
                        out[i] = v[i] / (s / k as f64);
                    }
                    if v[i].is_finite() {
                        s += v[i];
                        cnt += 1;
                    }
                    if i >= k && v[i - k].is_finite() {
                        s -= v[i - k];
                        cnt -= 1;
                    }
                }
                out
            }).collect()
        }
        Feature::Rvol => (0..nc).map(|c| rvol(&log_returns(&p.close[c]), window(n, 2), p.bars_per_day())).collect(),
        Feature::Funding => funding_sums(p, n),
        Feature::XsRet => xs_rank(p, &feature(p, Feature::Ret, n)),
        Feature::XsFunding => xs_rank(p, &funding_sums(p, n)),
        Feature::MktRet => {
            let k = window(n, 1);
            let mut cum = vec![0.0f64; p.n + 1];
            for i in 0..p.n {
                cum[i + 1] = cum[i] + p.basket_ret[i];
            }
            let row: Vec<f64> = (0..p.n).map(|i| if i + 1 >= k { cum[i + 1] - cum[i + 1 - k] } else { f64::NAN }).collect();
            vec![row; nc]
        }
        Feature::MktRvolPct => {
            let vol = rvol(&p.basket_ret, window(n, 2), p.bars_per_day());
            let w = 30 * p.bars_per_day();
            let row: Vec<f64> = (0..p.n).map(|i| {
                if !vol[i].is_finite() || i < w {
                    return f64::NAN;
                }
                let prev = &vol[i - w..i];
                let finite = prev.iter().filter(|x| x.is_finite()).count();
                if finite * 2 < w {
                    return f64::NAN;
                }
                prev.iter().filter(|x| x.is_finite() && **x < vol[i]).count() as f64 / finite as f64
            }).collect();
            vec![row; nc]
        }
    }
}

/// −1 at the low of the previous bars, +1 at their high; outside the range strictly
/// beyond ±1, so `channel > 1` is exactly "closed above the previous high".
fn channel_pos(px: f64, hi: f64, lo: f64) -> f64 {
    if !(px.is_finite() && hi.is_finite() && lo.is_finite()) {
        return f64::NAN;
    }
    let range = hi - lo;
    if px > hi {
        1.0 + ((px - hi) / range).max(4.0 * f64::EPSILON)
    } else if px < lo {
        -1.0 - ((lo - px) / range).max(4.0 * f64::EPSILON)
    } else {
        (2.0 * (px - lo) / range - 1.0).clamp(-1.0, 1.0)
    }
}

fn log_returns(close: &[f64]) -> Vec<f64> {
    let mut out = vec![f64::NAN; close.len()];
    let mut prev = f64::NAN;
    for (i, px) in close.iter().enumerate() {
        if px.is_finite() {
            if prev.is_finite() {
                out[i] = (px / prev).ln();
            }
            prev = *px;
        }
    }
    out
}

/// Std of the last `k` finite returns, scaled to a day; NaN until `k` are in the window.
fn rvol(r: &[f64], k: usize, bpd: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; r.len()];
    let (mut s, mut q, mut cnt) = (0.0f64, 0.0f64, 0usize);
    for i in 0..r.len() {
        if r[i].is_finite() {
            s += r[i];
            q += r[i] * r[i];
            cnt += 1;
        }
        if i >= k && r[i - k].is_finite() {
            s -= r[i - k];
            q -= r[i - k] * r[i - k];
            cnt -= 1;
        }
        if cnt == k {
            let m = s / k as f64;
            out[i] = ((q / k as f64 - m * m).max(0.0) * bpd as f64).sqrt();
        }
    }
    out
}

/// Funding printed in `[close − n hours, close)` of each bar (a print stamped exactly at
/// the close counts from the next bar, as `Panel::head` treats it); NaN before the first.
fn funding_sums(p: &Panel, hours: f64) -> Vec<Vec<f64>> {
    let look = (hours.max(1.0) * 3_600_000.0) as i64;
    p.funding.iter().take(p.coins.len()).map(|f| {
        let (mut head, mut tail, mut sum) = (0usize, 0usize, 0.0f64);
        (0..p.n).map(|i| {
            let t = p.time(i) + p.interval_ms;
            while head < f.len() && f[head].0 < t {
                sum += f[head].1;
                head += 1;
            }
            while tail < head && f[tail].0 < t - look {
                sum -= f[tail].1;
                tail += 1;
            }
            if head == 0 { f64::NAN } else { sum }
        }).collect()
    }).chain(std::iter::repeat_with(|| vec![f64::NAN; p.n]))
        .take(p.coins.len())
        .collect()
}

/// Rank among active coins with a value, scaled to −1 (lowest) … +1 (highest).
fn xs_rank(p: &Panel, x: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let nc = p.coins.len();
    let mut out = vec![vec![f64::NAN; p.n]; nc];
    let mut v: Vec<(f64, usize)> = Vec::with_capacity(nc);
    for i in 0..p.n {
        v.clear();
        v.extend((0..nc).filter(|c| p.active[*c][i] && x[*c][i].is_finite()).map(|c| (x[c][i], c)));
        if v.len() < 2 {
            continue;
        }
        v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let m = (v.len() - 1) as f64;
        for (j, (_, c)) in v.iter().enumerate() {
            out[*c][i] = -1.0 + 2.0 * j as f64 / m;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The plan.

struct Resolved<'a> {
    cond: &'a Cond,
    level: f64,
    values: &'a [Vec<f64>],
}

impl Resolved<'_> {
    /// Does the condition hold for `side` at coin `c`, bar `i`?
    fn holds(&self, c: usize, i: usize, side: i8) -> bool {
        let v = self.values[c][i];
        let x = if side < 0 && self.cond.feature.signed() { -v } else { v };
        self.cond.op.test(x, self.level)
    }
}

pub fn plan(rule: &Rule, spec: &Spec, p: &Panel) -> Result<Plan> {
    rule.validate()?;
    // Each (feature, window) is computed once per plan.
    let mut cache: HashMap<(Feature, u64), Vec<Vec<f64>>> = HashMap::new();
    let mut key_of = |c: &Cond| -> Result<((Feature, u64), f64)> {
        let n = c.n.resolve(spec)?;
        if !n.is_finite() || n <= 0.0 {
            bail!("{}: window {} of {} must be positive", spec.id(), c.n.text(), c.feature.name());
        }
        let key = (c.feature, n.to_bits());
        cache.entry(key).or_insert_with(|| feature(p, c.feature, n));
        Ok((key, c.level.resolve(spec)?))
    };
    let lk = rule.long.iter().map(&mut key_of).collect::<Result<Vec<_>>>()?;
    let fk = rule.filter.iter().map(&mut key_of).collect::<Result<Vec<_>>>()?;
    let xk = rule.exit.iter().map(&mut key_of).collect::<Result<Vec<_>>>()?;
    let cache = &cache;
    let long: Vec<Resolved> = rule.long.iter().zip(&lk).map(|(cond, (k, level))| Resolved { cond, level: *level, values: &cache[k] }).collect();
    let filter: Vec<Resolved> = rule.filter.iter().zip(&fk).map(|(cond, (k, level))| Resolved { cond, level: *level, values: &cache[k] }).collect();
    let exit: Vec<Resolved> = rule.exit.iter().zip(&xk).map(|(cond, (k, level))| Resolved { cond, level: *level, values: &cache[k] }).collect();

    let shorts = rule.short == Short::Mirror && spec.get("shorts", 1.0) > 0.5;
    let every = match &rule.every {
        Some(e) => {
            let k = e.resolve(spec)?.round();
            if k < 1.0 {
                bail!("{}: `every` must be at least 1 bar", spec.id());
            }
            Some(k as usize)
        }
        None => None,
    };
    let has_exit = !exit.is_empty() || rule.exit_opposite;
    let mut entry = empty(p);
    let mut exits = if has_exit { empty(p) } else { Vec::new() };
    let all = |v: &[Resolved], c: usize, i: usize, side: i8| v.iter().all(|r| r.holds(c, i, side));
    let any = |v: &[Resolved], c: usize, i: usize, side: i8| v.iter().any(|r| r.holds(c, i, side));
    for c in 0..p.coins.len() {
        for i in 0..p.n {
            let long_sig = all(&long, c, i, 1);
            let short_sig = all(&long, c, i, -1);
            if every.is_none_or(|k| rebalance_bar(p, i, k)) && all(&filter, c, i, 1) {
                if long_sig {
                    entry[c][i] = 1;
                } else if short_sig && shorts {
                    entry[c][i] = -1;
                }
            }
            if has_exit {
                if any(&exit, c, i, 1) || (rule.exit_opposite && short_sig) {
                    exits[c][i] = 1;
                } else if any(&exit, c, i, -1) || (rule.exit_opposite && long_sig) {
                    exits[c][i] = -1;
                }
            }
        }
    }
    let hold = match spec.params.get("hold") {
        Some(h) => h.round().max(1.0) as usize,
        None => ((spec.get("hold_days", 30.0) * p.bars_per_day() as f64) as usize).max(1),
    };
    Ok(Plan { entry, exit: exits, policy: Policy { hold_bars: hold, stop_bps: spec.get("stop", 0.0), extend: rule.extend, max_positions: spec.get("max_positions", 0.0) as usize } })
}

// ---------------------------------------------------------------------------
// Templates: hand-written families re-expressed in blocks, so `lab propose` can
// derive children from them. Only `trend` is exact (a test requires identical
// trades); the others are approximations and are labelled so in every proposal.

/// The composed form of a hand-written family, and whether it is exact.
pub fn template(family: &str) -> Option<(Rule, bool)> {
    let c = |feature: Feature, n: &str, op: Op, level: Param| Cond { feature, n: Param::Ref(n.into()), op, level };
    let r = |long: Vec<Cond>, exit: Vec<Cond>| Rule { long, short: Short::Mirror, filter: Vec::new(), exit, exit_opposite: false, every: None, extend: false };
    match family {
        "trend" => Some((r(vec![c(Feature::Channel, "$n", Op::Gt, Param::Num(1.0))], vec![c(Feature::Channel, "$n*$out_frac", Op::Lt, Param::Num(-1.0))]), true)),
        // Fade a stretch: z at or below −z (H-R01). `resid` maps to `resid_z`.
        "meanrev" | "jev_gate" => Some((r(vec![c(Feature::Z, "$n", Op::Le, Param::Ref("$z*-1".into()))], Vec::new()), false)),
        // Buy the losers on the clock (rank instead of the k worst).
        "xs_reversal" => {
            let mut rule = r(vec![c(Feature::XsRet, "$look", Op::Le, Param::Num(-0.6))], Vec::new());
            rule.every = Some(Param::Ref("$hold".into()));
            rule.extend = true;
            Some((rule, false))
        }
        // Long the coins that pay longs (lowest funding), short the mirror.
        "funding_carry" => {
            let mut rule = r(vec![c(Feature::XsFunding, "$look", Op::Le, Param::Num(-0.6))], Vec::new());
            rule.every = Some(Param::Ref("$hold".into()));
            rule.extend = true;
            Some((rule, false))
        }
        // Fade a stretch on a volume surge.
        "hvt" => Some((r(vec![c(Feature::Z, "$vol_look", Op::Le, Param::Ref("$z*-1".into())), c(Feature::VolRatio, "$vol_look", Op::Ge, Param::Ref("$vol_mult".into()))], Vec::new()), false)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::panel::testing::*;
    use super::super::rules::{Costs, Sizing};
    use super::super::sim::{SimCfg, simulate};
    use super::super::strategy::{self, Aux};
    use super::*;

    fn trend_rule() -> Rule {
        template("trend").unwrap().0
    }

    #[test]
    fn ids_are_stable_and_structural() {
        let a = trend_rule();
        let b: Rule = toml::from_str("long = { feature = \"channel\", n = \"$n\", op = \">\", level = 1.0 }\nexit = [{ feature = \"channel\", n = \"$n*$out_frac\", op = \"<\", level = -1.0 }]\n").unwrap();
        assert_eq!(a, b, "a single condition and a one-element list are the same rule");
        assert_eq!(a.id(), b.id());
        assert!(is_composed_id(&a.id()));
        let mut c = a.clone();
        c.short = Short::None;
        assert_ne!(a.id(), c.id());
        // Pinned: an id that changes silently would make old trials look new.
        assert_eq!(a.id(), "c4628412b4bd4");
        assert_eq!(a.refs(), vec!["n".to_string(), "out_frac".to_string()]);
    }

    #[test]
    fn params_resolve_products_and_report_missing() {
        let s = Spec::new("x", &[("n", 96.0), ("out_frac", 0.25)]);
        assert_eq!(Param::Ref("$n*$out_frac".into()).resolve(&s).unwrap(), 24.0);
        assert_eq!(Param::Ref("$n*0.5".into()).resolve(&s).unwrap(), 48.0);
        assert_eq!(Param::Num(3.0).resolve(&s).unwrap(), 3.0);
        assert!(Param::Ref("$k".into()).resolve(&s).is_err());
        assert!(Param::Ref("$n*abc".into()).resolve(&s).is_err());
    }

    /// The block library is honest: `trend` written as blocks makes the same plan and
    /// the same trades as the hand-written rule.
    #[test]
    fn composed_trend_equals_the_hand_written_one() {
        let rule = trend_rule();
        let id = register(&rule);
        let p = random_panel(21, 5, 60, "1h");
        let cfg = SimCfg::new(10_000.0, Costs::default(), Sizing::default());
        for (n, out_frac, shorts, stop) in [(24.0, 0.5, 1.0, 0.0), (48.0, 0.25, 0.0, 800.0), (96.0, 0.5, 1.0, 300.0), (7.0, 0.25, 1.0, 0.0)] {
            let params = [("n", n), ("out_frac", out_frac), ("shorts", shorts), ("stop", stop)];
            let hand = strategy::plan(&Spec::new("trend", &params), &p, &Aux::default()).unwrap();
            let comp = strategy::plan(&Spec::new(&id, &params), &p, &Aux::default()).unwrap();
            assert_eq!(hand.entry, comp.entry, "entries, n={n}");
            assert_eq!(hand.exit, comp.exit, "exits, n={n}");
            assert_eq!(hand.policy, comp.policy);
            let (a, b) = (simulate(&p, &hand, &cfg), simulate(&p, &comp, &cfg));
            assert!(!a.trades.is_empty());
            assert_eq!(a.trades.len(), b.trades.len());
            assert_eq!(a.daily, b.daily);
        }
    }

    #[test]
    fn mirror_and_filters() {
        let p = random_panel(3, 4, 30, "1h");
        let rule: Rule = toml::from_str("long = { feature = \"z\", n = 24, op = \"<=\", level = -1.5 }\nfilter = { feature = \"mkt_ret\", n = 24, op = \">\", level = 0.0 }\n").unwrap();
        let spec = Spec::new(&register(&rule), &[("hold", 6.0)]);
        let pl = plan(&rule, &spec, &p).unwrap();
        let z = feature(&p, Feature::Z, 24.0);
        let m = feature(&p, Feature::MktRet, 24.0);
        let (mut longs, mut shorts) = (0, 0);
        for c in 0..4 {
            for i in 0..p.n {
                match pl.entry[c][i] {
                    1 => {
                        assert!(z[c][i] <= -1.5 && m[c][i] > 0.0);
                        longs += 1;
                    }
                    -1 => {
                        assert!(z[c][i] >= 1.5 && m[c][i] > 0.0, "the filter is not mirrored");
                        shorts += 1;
                    }
                    _ => {}
                }
            }
        }
        assert!(longs > 3 && shorts > 3, "{longs} {shorts}");
        let mut long_only = rule.clone();
        long_only.short = Short::None;
        assert!(plan(&long_only, &spec, &p).unwrap().entry.iter().flatten().all(|e| *e >= 0));
        let no_shorts = Spec::new(&spec.family, &[("hold", 6.0), ("shorts", 0.0)]);
        assert!(plan(&rule, &no_shorts, &p).unwrap().entry.iter().flatten().all(|e| *e >= 0));
    }

    #[test]
    fn features_are_causal_and_in_range() {
        let mut full = random_panel(9, 5, 45, "1h");
        full.funding = (0..5).map(|c| (0..full.n as i64).map(|i| (i * 3_600_000, 0.00001 * (c as f64 - 2.0 + (i as f64 / 30.0).sin()))).collect()).collect();
        let cut = 24 * 37 + 5;
        let mut head = full.head(cut);
        head.rederive(NO_RULE);
        for f in FEATURES {
            let (a, b) = (feature(&full, f, 24.0), feature(&head, f, 24.0));
            for c in 0..5 {
                for i in 0..cut {
                    assert!(a[c][i].to_bits() == b[c][i].to_bits() || (a[c][i] - b[c][i]).abs() < 1e-9, "{} coin {c} bar {i}: {} vs {}", f.name(), a[c][i], b[c][i]);
                }
            }
            assert!(a.iter().flatten().any(|x| x.is_finite()), "{} is never defined", f.name());
        }
        for x in feature(&full, Feature::XsRet, 12.0).iter().flatten().filter(|x| x.is_finite()) {
            assert!((-1.0..=1.0).contains(x));
        }
        for x in feature(&full, Feature::MktRvolPct, 24.0).iter().flatten().filter(|x| x.is_finite()) {
            assert!((0.0..=1.0).contains(x));
        }
        assert_eq!(channel_pos(10.0, 10.0, 5.0), 1.0);
        assert!(channel_pos(10.0 + 1e-12, 10.0, 5.0) > 1.0);
        assert!(channel_pos(f64::NAN, 10.0, 5.0).is_nan());
    }
}
