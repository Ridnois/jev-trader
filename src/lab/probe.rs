//! Jev as a market reader, replayed over history.
//!
//! A probe says *when* to ask (at a rule's candidates, or every N bars), *what* Jev
//! sees (a state built only from bars already closed) and *which* typed questions it
//! answers. `replay` walks the archive bar by bar, asks, and caches every answer, so a
//! rerun is free and the same answers feed scoring (`score`) and the Jev strategy
//! families (`aux`). The live engine uses the same state builder and cache format.
//!
//! The state is anonymous by default: percent moves, z-scores and ratios, no coin
//! name, no date, no price level. Jev has read the internet; a model shown "ZEC on
//! 2026-10-10" may simply remember what happened next. The `revealed` variant exists
//! to measure that memory, not to trade on it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::panel::{DAY_MS, Panel};
use super::sim::{self, SimCfg};
use super::stat;
use super::strategy::{self, Aux, Spec, rolling_z};
use crate::jev::{JevClient, Question};

#[derive(Debug, Clone, Deserialize)]
pub struct QuestionDef {
    pub id: String,
    /// noul | score | choice
    pub kind: String,
    /// `{with}` and `{against}` become the trade's direction words (higher/lower…).
    pub text: String,
    /// score: levels from lowest to highest. choice: `label = description` pairs.
    #[serde(default)]
    pub levels: Vec<String>,
    #[serde(default)]
    pub options: BTreeMap<String, String>,
    /// What the answer is scored against: `fwd_up` (price higher after the horizon, in
    /// the trade's favour) or `adverse_2pct` (moves 2% against the trade first). Empty = unscored.
    #[serde(default)]
    pub outcome: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Probe {
    pub name: String,
    pub interval: String,
    #[serde(default)]
    pub hypothesis: String,
    /// Ask at the entries and signals of this spec id…
    #[serde(default)]
    pub candidates_of: String,
    /// …or every this many bars per coin (0 = candidates only).
    #[serde(default)]
    pub every: usize,
    /// Bars of recent history shown as a table.
    #[serde(default = "d_rows")]
    pub rows: usize,
    /// Bars ahead the answers are scored on.
    pub horizon_bars: usize,
    pub questions: Vec<QuestionDef>,
}

fn d_rows() -> usize {
    16
}

impl Probe {
    pub fn load(path: &Path) -> Result<Probe> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let p: Probe = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        if p.candidates_of.is_empty() == (p.every == 0) {
            bail!("{}: set exactly one of `candidates_of` and `every`", path.display());
        }
        for q in &p.questions {
            match q.kind.as_str() {
                "noul" => {}
                "score" if q.levels.len() >= 2 => {}
                "choice" if q.options.len() >= 2 => {}
                k => bail!("{}: question `{}` has kind `{k}` without its levels/options", path.display(), q.id),
            }
        }
        Ok(p)
    }

    /// Changes whenever the questions or the state layout change: old answers are
    /// then ignored instead of being mixed with new ones.
    pub fn hash(&self) -> String {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |s: &str| {
            for b in s.bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
            h ^= 0xff;
        };
        eat(STATE_VERSION);
        eat(&self.interval);
        eat(&self.rows.to_string());
        eat(&self.horizon_bars.to_string());
        for q in &self.questions {
            eat(&q.id);
            eat(&q.kind);
            eat(&q.text);
            q.levels.iter().for_each(|l| eat(l));
            q.options.iter().for_each(|(k, v)| {
                eat(k);
                eat(v);
            });
        }
        format!("{h:016x}")
    }

    fn questions_for(&self, side: i8) -> IndexMap<String, Question> {
        let (with, against, w_ing, a_ing) = if side >= 0 { ("higher", "lower", "rising", "falling") } else { ("lower", "higher", "falling", "rising") };
        let fill = |s: &str| s.replace("{with}", with).replace("{against}", against).replace("{with_ing}", w_ing).replace("{against_ing}", a_ing);
        self.questions.iter().map(|q| {
            let text = fill(&q.text);
            let question = match q.kind.as_str() {
                "score" => Question::score(&text, &q.levels.iter().map(String::as_str).collect::<Vec<_>>()),
                "choice" => Question::choice(&text, q.options.iter().map(|(k, v)| (k.clone(), v.clone()))),
                _ => Question::noul(&text),
            };
            (q.id.clone(), question)
        }).collect()
    }
}

/// Bump when `Featurizer::state` changes shape.
const STATE_VERSION: &str = "state-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Variant {
    /// Anonymous state: the only variant strategies may use.
    Anon,
    /// Coin name and date shown: measures memory of history.
    Revealed,
    /// The anonymous state asked a second time: measures answer stability.
    Repeat,
}

impl Variant {
    pub fn parse(s: &str) -> Result<Variant> {
        Ok(match s {
            "anon" => Variant::Anon,
            "revealed" => Variant::Revealed,
            "repeat" => Variant::Repeat,
            _ => bail!("unknown variant `{s}` (anon, revealed, repeat)"),
        })
    }
}

/// One cached answer set (a line of `answers.jsonl`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Answer {
    pub probe: String,
    pub hash: String,
    pub venue: String,
    pub variant: Variant,
    pub coin: String,
    /// Open time of the bar the state was built at (its close is when Jev is asked).
    pub t: i64,
    /// +1 / −1: the trade the questions were phrased for.
    pub side: i8,
    pub model: String,
    /// noul → probability; score → expected level 0..1; choice → the label.
    pub answers: BTreeMap<String, Value>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub ms: u64,
    pub asked_at: chrono::DateTime<chrono::Utc>,
}

pub fn cache_path(data_dir: &Path, probe: &str) -> PathBuf {
    data_dir.join("jev").join(probe).join("answers.jsonl")
}

pub fn load_answers(data_dir: &Path, probe: &Probe, venue: &str) -> Vec<Answer> {
    let hash = probe.hash();
    let all: Vec<Answer> = crate::status::read_jsonl(&cache_path(data_dir, &probe.name));
    all.into_iter().filter(|a| a.hash == hash && a.venue == venue).collect()
}

/// Per-coin series the state is built from, computed once per panel. All causal.
pub struct Featurizer<'a> {
    p: &'a Panel,
    /// Stretch against the mean of the previous 3 days of closes.
    z: Vec<Vec<f64>>,
    /// Rolling mean volume over 3 days.
    vol_avg: Vec<Vec<f64>>,
    /// Cumulative basket log return.
    basket: Vec<f64>,
}

impl<'a> Featurizer<'a> {
    pub fn new(p: &'a Panel) -> Self {
        let w = 3 * p.bars_per_day();
        let z = p.close.iter().map(|c| rolling_z(c, w)).collect();
        let vol_avg = p.volume.iter().map(|v| {
            let mut out = vec![f64::NAN; v.len()];
            let (mut s, mut k) = (0.0, 0usize);
            for i in 0..v.len() {
                if v[i].is_finite() {
                    s += v[i];
                    k += 1;
                }
                if i >= w && v[i - w].is_finite() {
                    s -= v[i - w];
                    k -= 1;
                }
                if k > w / 2 {
                    out[i] = s / k as f64;
                }
            }
            out
        }).collect();
        let mut acc = 0.0;
        let basket = p.basket_ret.iter().map(|r| {
            acc += r;
            acc
        }).collect();
        Self { p, z, vol_avg, basket }
    }

    /// The state Jev sees for coin `c` at the close of bar `i`, or None when there
    /// isn't enough history. Uses bars `0..=i` only.
    pub fn state(&self, c: usize, i: usize, probe: &Probe, side: i8, reveal: bool) -> Option<Value> {
        let p = self.p;
        let bpd = p.bars_per_day();
        let hour = (3_600_000 / p.interval_ms).max(1) as usize;
        if i < 3 * bpd || i < probe.rows {
            return None;
        }
        let px = p.close[c][i];
        let r2 = |x: f64| (x * 100.0).round() / 100.0;
        let mv = |k: usize| -> Option<f64> {
            let a = p.close[c][i.checked_sub(k)?];
            (a.is_finite() && px.is_finite()).then(|| r2((px / a - 1.0) * 100.0))
        };
        let mkt = |k: usize| r2(((self.basket[i] - self.basket[i - k]).exp() - 1.0) * 100.0);
        let (z, dvol, va) = (self.z[c][i], p.dvol[c][i], self.vol_avg[c][i]);
        if !(px.is_finite() && z.is_finite() && dvol.is_finite() && va.is_finite() && va > 0.0) {
            return None;
        }
        // Latest funding print at or before the bar's close, as bps per hour.
        let t_close = p.time(i) + p.interval_ms;
        let f = &p.funding[c];
        let k = f.partition_point(|(t, _)| *t <= t_close);
        let funding = (k > 0).then(|| {
            let gap_h = if k > 1 { ((f[k - 1].0 - f[k - 2].0) as f64 / 3_600_000.0).round().max(1.0) } else { 1.0 };
            r2(f[k - 1].1 * 1e4 / gap_h)
        });
        let rows = i + 1 - probe.rows..=i;
        let mut rets = Vec::with_capacity(probe.rows);
        let mut ranges = Vec::with_capacity(probe.rows);
        let mut vols = Vec::with_capacity(probe.rows);
        for j in rows {
            let (o, h, l, cl) = (p.open[c][j], p.high[c][j], p.low[c][j], p.close[c][j]);
            if !(o.is_finite() && cl.is_finite()) {
                return None;
            }
            rets.push(r2((cl / o - 1.0) * 100.0));
            ranges.push(r2((h / l - 1.0) * 100.0));
            vols.push(((p.volume[c][j] / va) * 10.0).round() / 10.0);
        }
        let mut now = json!({
            "move_1h_pct": mv(hour),
            "move_6h_pct": mv(6 * hour),
            "move_24h_pct": mv(24 * hour),
            "move_3d_pct": mv(72 * hour),
            "stretch_vs_3d_mean_in_std": r2(z),
            "typical_daily_move_pct": r2(dvol * 100.0),
            "volume_vs_3d_average": ((p.volume[c][i] / va) * 10.0).round() / 10.0,
            "whole_market_move_6h_pct": mkt(6 * hour),
            "whole_market_move_24h_pct": mkt(24 * hour),
            "hour_of_day_utc": (t_close.rem_euclid(DAY_MS) / 3_600_000),
        });
        if let Some(fb) = funding {
            now["funding_bps_per_hour_longs_pay"] = json!(fb);
        }
        let mut state = json!({
            "what": format!("A cryptocurrency perpetual future on {} bars. Moves are in percent. The asset's name and the date are withheld on purpose.", p.interval),
            "now": now,
            "recent_bars_oldest_first": { "return_pct": rets, "high_low_range_pct": ranges, "volume_vs_3d_average": vols },
        });
        if !probe.candidates_of.is_empty() {
            let hold_h = probe.horizon_bars as f64 * p.interval_ms as f64 / 3_600_000.0;
            state["proposed_trade"] = json!(if side >= 0 {
                format!("buy now, sell within {hold_h:.0} hours: profits if the price rises")
            } else {
                format!("sell short now, buy back within {hold_h:.0} hours: profits if the price falls")
            });
        }
        if reveal {
            state["what"] = json!(format!("{} perpetual future on {} bars. Moves are in percent.", p.coins[c], p.interval));
            state["time_utc"] = json!(chrono::DateTime::from_timestamp_millis(t_close).map(|d| d.format("%Y-%m-%d %H:%M").to_string()));
        }
        Some(state)
    }
}

/// A place to ask: coin, bar and the side the questions are phrased for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Point {
    pub c: usize,
    pub i: usize,
    pub side: i8,
    /// The un-gated rule actually entered here (asked first when the budget is short).
    pub entry: bool,
}

/// Where the probe asks on this panel, in priority order: the rule's real entries
/// first (oldest first), then its other signal bars, or every N-th bar.
pub fn points(p: &Panel, probe: &Probe, cfg: &SimCfg) -> Result<Vec<Point>> {
    let mut out = Vec::new();
    if probe.every > 0 {
        for c in 0..p.coins.len() {
            // Stagger coins so they aren't all asked on the same bars.
            let off = (c * 7) % probe.every;
            for i in (0..p.n).filter(|i| (p.time(*i) / p.interval_ms) as usize % probe.every == off) {
                if p.active[c][i] {
                    out.push(Point { c, i, side: 1, entry: true });
                }
            }
        }
        out.sort_by_key(|x| (x.i, x.c));
        return Ok(out);
    }
    let spec: Spec = super::parse_spec(&probe.candidates_of)?;
    let plan = strategy::plan(&spec, p, &Aux::default())?;
    let run = sim::simulate(p, &plan, cfg);
    let mut seen = HashSet::new();
    for t in &run.trades {
        // A trade filled at the open of `entry_i` was signalled on the bar before.
        if t.entry_i > 0 && seen.insert((t.coin, t.entry_i - 1)) {
            out.push(Point { c: t.coin, i: t.entry_i - 1, side: t.side, entry: true });
        }
    }
    out.sort_by_key(|x| (x.i, x.c));
    let mut rest = Vec::new();
    for c in 0..p.coins.len() {
        for i in 0..p.n {
            let s = plan.entry[c][i];
            if s != 0 && p.active[c][i] && !seen.contains(&(c, i)) {
                rest.push(Point { c, i, side: s, entry: false });
            }
        }
    }
    rest.sort_by_key(|x| (x.i, x.c));
    out.extend(rest);
    Ok(out)
}

pub struct ReplayOpts {
    pub variant: Variant,
    pub max_calls: usize,
    pub concurrency: usize,
    /// Only the rule's real entries (skip its other signal bars).
    pub entries_only: bool,
    /// Ask a random sample of this many of the points already answered as `anon`
    /// (for the revealed and repeat variants).
    pub sample: usize,
    pub seed: u64,
    pub dry_run: bool,
}

#[derive(Debug, Default)]
pub struct ReplayStats {
    pub planned: usize,
    pub cached: usize,
    pub asked: usize,
    pub failed: usize,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub left: usize,
}

fn parse_answers(probe: &Probe, r: &crate::jev::Response) -> Result<BTreeMap<String, Value>> {
    r.validate()?;
    let mut out = BTreeMap::new();
    for q in &probe.questions {
        let v = match q.kind.as_str() {
            "score" => json!(r.score_expected(&q.id)?.0),
            "choice" => json!(r.choice(&q.id)?.0),
            _ => json!(r.noul(&q.id)?),
        };
        out.insert(q.id.clone(), v);
    }
    Ok(out)
}

/// Walks the panel, asks Jev at each point not yet answered, appends to the cache.
pub async fn replay(p: &Panel, probe: &Probe, cfg: &SimCfg, data_dir: &Path, jev: Option<&JevClient>, o: &ReplayOpts) -> Result<ReplayStats> {
    let path = cache_path(data_dir, &probe.name);
    let hash = probe.hash();
    let known = load_answers(data_dir, probe, &p.venue);
    let done: HashSet<(String, i64)> = known.iter().filter(|a| a.variant == o.variant).map(|a| (a.coin.clone(), a.t)).collect();
    let mut pts = points(p, probe, cfg)?;
    if o.entries_only {
        pts.retain(|x| x.entry);
    }
    if o.variant != Variant::Anon {
        // Re-ask a random sample of what the anonymous run answered, so the pairs line up.
        let anon: HashSet<(String, i64)> = known.iter().filter(|a| a.variant == Variant::Anon).map(|a| (a.coin.clone(), a.t)).collect();
        pts.retain(|x| anon.contains(&(p.coins[x.c].clone(), p.time(x.i))));
        let mut rng = crate::stats::Rng::new(o.seed);
        for k in (1..pts.len()).rev() {
            pts.swap(k, rng.below(k + 1));
        }
        pts.truncate(o.sample.max(1));
    }
    let feat = Featurizer::new(p);
    let reveal = o.variant == Variant::Revealed;
    let mut st = ReplayStats::default();
    let mut todo = Vec::new();
    for x in &pts {
        let Some(state) = feat.state(x.c, x.i, probe, x.side, reveal) else { continue };
        st.planned += 1;
        if done.contains(&(p.coins[x.c].clone(), p.time(x.i))) {
            st.cached += 1;
        } else {
            todo.push((*x, state));
        }
    }
    st.left = todo.len().saturating_sub(o.max_calls);
    todo.truncate(o.max_calls);
    if o.dry_run {
        if let Some((x, state)) = todo.first() {
            println!("First state that would be sent ({} bar {}):\n{}\n", p.coins[x.c], x.i, serde_json::to_string_pretty(state)?);
            println!("Questions:\n{}\n", serde_json::to_string_pretty(&probe.questions_for(x.side))?);
        }
        st.left += todo.len();
        return Ok(st);
    }
    let jev = jev.context("no Jev client")?;
    let total = todo.len();
    let mut results = futures::stream::iter(todo).map(|(x, state)| {
        let q = probe.questions_for(x.side);
        async move {
            let t0 = std::time::Instant::now();
            let mut last = None;
            for attempt in 0..3u64 {
                match jev.ask(&state, &q).await.and_then(|r| parse_answers(probe, &r).map(|a| (a, r))) {
                    Ok(ok) => return (x, Ok((ok, t0.elapsed().as_millis() as u64))),
                    Err(e) => {
                        last = Some(e);
                        tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt + 1))).await;
                    }
                }
            }
            (x, Err(last.unwrap_or_else(|| anyhow::anyhow!("no attempt"))))
        }
    }).buffer_unordered(o.concurrency.max(1));
    let mut fails_in_a_row = 0;
    while let Some((x, res)) = results.next().await {
        match res {
            Ok(((answers, r), ms)) => {
                fails_in_a_row = 0;
                let (ti, to) = r.usage.as_ref().map(|u| (u.input_tokens, u.output_tokens)).unwrap_or((0, 0));
                st.asked += 1;
                st.tokens_in += ti;
                st.tokens_out += to;
                crate::bot::append_jsonl(&path, &Answer { probe: probe.name.clone(), hash: hash.clone(), venue: p.venue.clone(), variant: o.variant, coin: p.coins[x.c].clone(), t: p.time(x.i), side: x.side, model: r.model.clone(), answers, tokens_in: ti, tokens_out: to, ms, asked_at: chrono::Utc::now() });
                if st.asked % 100 == 0 {
                    eprintln!("  {}/{total} asked · {} tokens so far", st.asked, st.tokens_in + st.tokens_out);
                }
            }
            Err(e) => {
                st.failed += 1;
                fails_in_a_row += 1;
                eprintln!("  {} bar {}: {e:#}", p.coins[x.c], x.i);
                if fails_in_a_row >= 12 {
                    bail!("12 Jev calls failed in a row; stopping (answers so far are cached)");
                }
            }
        }
    }
    Ok(st)
}

/// Anonymous answers as series the Jev families read: question id → `[coin][bar]`.
/// A noul phrased for the trade's side is stored as asked (a `p_revert` of 0.7 means
/// "70% this trade's side wins", for longs and shorts alike).
pub fn aux(p: &Panel, probe: &Probe, answers: &[Answer]) -> Aux {
    let mut aux = Aux::default();
    for q in probe.questions.iter().filter(|q| q.kind != "choice") {
        aux.jev.insert(q.id.clone(), vec![vec![f32::NAN; p.n]; p.coins.len()]);
    }
    for a in answers.iter().filter(|a| a.variant == Variant::Anon && a.venue == p.venue) {
        let Some(c) = p.coin_index(&a.coin) else { continue };
        let d = a.t - p.t0;
        if d < 0 || d % p.interval_ms != 0 || (d / p.interval_ms) as usize >= p.n {
            continue;
        }
        let i = (d / p.interval_ms) as usize;
        for (id, v) in &a.answers {
            if let (Some(series), Some(x)) = (aux.jev.get_mut(id), v.as_f64()) {
                series[c][i] = x as f32;
            }
        }
    }
    aux
}

/// What happened after bar `i` for a trade of `side` entered at the next open.
/// (signed return over the horizon as a fraction, went 2% against first)
pub fn outcome(p: &Panel, c: usize, i: usize, side: i8, horizon: usize) -> Option<(f64, bool)> {
    let (a, b) = (i + 1, i + 1 + horizon);
    if b >= p.n {
        return None;
    }
    let (entry, exit) = (p.open[c][a], p.open[c][b]);
    if !(entry.is_finite() && exit.is_finite()) {
        return None;
    }
    let s = side as f64;
    let adverse = (a..b).any(|j| if side >= 0 { p.low[c][j] <= entry * 0.98 } else { p.high[c][j] >= entry * 1.02 });
    Some((s * (exit / entry - 1.0), adverse))
}

/// Area under the ROC curve: P(a positive case got a higher answer than a negative one).
pub fn auc(p: &[f64], y: &[bool]) -> Option<f64> {
    let ranks = crate::stats::ranks(p);
    let pos = y.iter().filter(|v| **v).count();
    let neg = y.len() - pos;
    if pos == 0 || neg == 0 {
        return None;
    }
    let sum: f64 = ranks.iter().zip(y).filter(|(_, v)| **v).map(|(r, _)| r).sum();
    Some((sum - (pos * (pos + 1)) as f64 / 2.0) / (pos * neg) as f64)
}

#[derive(Debug, Clone, Serialize)]
pub struct QuestionScore {
    pub id: String,
    pub outcome: String,
    pub n: usize,
    pub days: usize,
    pub base_rate: f64,
    pub mean_answer: f64,
    /// 1 − Brier / Brier of always answering the base rate. > 0 = better than no skill.
    pub brier_skill: f64,
    pub auc: Option<f64>,
    /// Spearman of the answer against the signed forward return.
    pub ic: Option<f64>,
    /// Where that IC sits in a null made by shuffling outcomes in day-sized blocks (0–100).
    pub ic_null_pct: Option<f64>,
    /// Answer quintiles: (mean answer, share true, mean signed forward return bps, n).
    pub buckets: Vec<(f64, f64, f64, usize)>,
    pub by_coin: Vec<(String, usize, f64)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Scorecard {
    pub probe: String,
    pub venue: String,
    pub models: Vec<String>,
    pub calls: usize,
    pub tokens_per_call: f64,
    pub median_ms: u64,
    pub questions: Vec<QuestionScore>,
    /// The rule's own one-number predictor (|stretch|) scored the same way, for comparison.
    pub baseline_ic: Option<f64>,
    /// (question, n pairs, IC anonymous, IC revealed) on the same states.
    pub leak: Vec<(String, usize, f64, f64)>,
    /// (question, n pairs, mean absolute difference between two asks).
    pub stability: Vec<(String, usize, f64)>,
    pub choices: Vec<(String, String, usize, f64)>,
}

pub fn score(p: &Panel, probe: &Probe, answers: &[Answer]) -> Scorecard {
    let anon: Vec<&Answer> = answers.iter().filter(|a| a.variant == Variant::Anon).collect();
    let idx = |a: &Answer| -> Option<(usize, usize)> {
        let c = p.coin_index(&a.coin)?;
        let d = a.t - p.t0;
        (d >= 0 && d % p.interval_ms == 0 && ((d / p.interval_ms) as usize) < p.n).then(|| (c, (d / p.interval_ms) as usize))
    };
    // (time, answer row, signed forward return, adverse) for every answer with a known outcome.
    let mut obs: Vec<(i64, &Answer, f64, bool, usize, usize)> = anon.iter().filter_map(|a| {
        let (c, i) = idx(a)?;
        let (ret, adverse) = outcome(p, c, i, a.side, probe.horizon_bars)?;
        Some((a.t, *a, ret, adverse, c, i))
    }).collect();
    obs.sort_by_key(|o| (o.0, o.4));
    let per_day = (obs.len() as f64 / obs.iter().map(|o| o.0 / DAY_MS).collect::<HashSet<_>>().len().max(1) as f64).ceil().max(1.0) as usize;
    let mut questions = Vec::new();
    for q in probe.questions.iter().filter(|q| !q.outcome.is_empty() && q.kind != "choice") {
        let rows: Vec<(f64, bool, f64, usize)> = obs.iter().filter_map(|o| {
            let a = o.1.answers.get(&q.id)?.as_f64()?;
            let y = if q.outcome == "adverse_2pct" { o.3 } else { o.2 > 0.0 };
            Some((a, y, o.2, o.4))
        }).collect();
        let n = rows.len();
        if n < 5 {
            continue;
        }
        let ps: Vec<f64> = rows.iter().map(|r| r.0).collect();
        let ys: Vec<bool> = rows.iter().map(|r| r.1).collect();
        let rets: Vec<f64> = rows.iter().map(|r| r.2).collect();
        let base = ys.iter().filter(|v| **v).count() as f64 / n as f64;
        let brier = rows.iter().map(|r| (r.0 - r.1 as u8 as f64).powi(2)).sum::<f64>() / n as f64;
        let brier_ref = base * (1.0 - base);
        // For "adverse" the answer should rise with a *worse* return, so flip the sign.
        let target: Vec<f64> = if q.outcome == "adverse_2pct" { rets.iter().map(|r| -r).collect() } else { rets.clone() };
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|a, b| ps[*a].total_cmp(&ps[*b]));
        let buckets = order.chunks(n.div_ceil(5).max(1)).map(|ch| {
            let k = ch.len() as f64;
            (ch.iter().map(|i| ps[*i]).sum::<f64>() / k, ch.iter().filter(|i| ys[**i]).count() as f64 / k, ch.iter().map(|i| rets[*i]).sum::<f64>() / k * 1e4, ch.len())
        }).collect();
        let mut by_coin: Vec<(String, usize, f64)> = (0..p.coins.len()).filter_map(|c| {
            let (a, b): (Vec<f64>, Vec<f64>) = rows.iter().zip(&target).filter(|(r, _)| r.3 == c).map(|(r, t)| (r.0, *t)).unzip();
            (a.len() >= 15).then(|| crate::stats::spearman(&a, &b).map(|ic| (p.coins[c].clone(), a.len(), ic))).flatten()
        }).collect();
        by_coin.sort_by(|a, b| b.2.total_cmp(&a.2));
        questions.push(QuestionScore {
            id: q.id.clone(),
            outcome: q.outcome.clone(),
            n,
            days: obs.iter().map(|o| o.0 / DAY_MS).collect::<HashSet<_>>().len(),
            base_rate: base,
            mean_answer: stat::mean(&ps),
            brier_skill: if brier_ref > 0.0 { 1.0 - brier / brier_ref } else { 0.0 },
            auc: auc(&ps, &ys),
            ic: crate::stats::spearman(&ps, &target),
            ic_null_pct: crate::stats::null_percentile(&ps, &target, per_day, 1000, crate::stats::spearman),
            buckets,
            by_coin,
        });
    }
    // Baseline: how stretched the price already was, scored against the same returns.
    let feat = Featurizer::new(p);
    let (bx, by): (Vec<f64>, Vec<f64>) = obs.iter().filter_map(|o| {
        let z = feat.z[o.4][o.5];
        z.is_finite().then_some((z.abs(), o.2))
    }).unzip();
    let baseline_ic = crate::stats::spearman(&bx, &by);
    // Paired variants on the same states.
    let key = |a: &Answer| (a.coin.clone(), a.t);
    let by_key: HashMap<(String, i64), &Answer> = anon.iter().map(|a| (key(a), *a)).collect();
    let ret_of: HashMap<(String, i64), f64> = obs.iter().map(|o| (key(o.1), o.2)).collect();
    let mut leak = Vec::new();
    let mut stability = Vec::new();
    for q in probe.questions.iter().filter(|q| q.kind != "choice") {
        let pairs = |v: Variant| -> Vec<(f64, f64, Option<f64>)> {
            answers.iter().filter(|a| a.variant == v).filter_map(|a| {
                let base = by_key.get(&key(a))?;
                Some((base.answers.get(&q.id)?.as_f64()?, a.answers.get(&q.id)?.as_f64()?, ret_of.get(&key(a)).copied()))
            }).collect()
        };
        let rev = pairs(Variant::Revealed);
        let with_ret: Vec<&(f64, f64, Option<f64>)> = rev.iter().filter(|x| x.2.is_some()).collect();
        if with_ret.len() >= 10 && q.outcome != "adverse_2pct" && !q.outcome.is_empty() {
            let r: Vec<f64> = with_ret.iter().map(|x| x.2.unwrap_or(0.0)).collect();
            let ic_a = crate::stats::spearman(&with_ret.iter().map(|x| x.0).collect::<Vec<_>>(), &r).unwrap_or(0.0);
            let ic_r = crate::stats::spearman(&with_ret.iter().map(|x| x.1).collect::<Vec<_>>(), &r).unwrap_or(0.0);
            leak.push((q.id.clone(), with_ret.len(), ic_a, ic_r));
        }
        let rep = pairs(Variant::Repeat);
        if !rep.is_empty() {
            stability.push((q.id.clone(), rep.len(), rep.iter().map(|x| (x.0 - x.1).abs()).sum::<f64>() / rep.len() as f64));
        }
    }
    let mut choices = Vec::new();
    for q in probe.questions.iter().filter(|q| q.kind == "choice") {
        let mut groups: BTreeMap<String, (usize, f64)> = BTreeMap::new();
        for o in &obs {
            if let Some(label) = o.1.answers.get(&q.id).and_then(Value::as_str) {
                let g = groups.entry(label.to_string()).or_default();
                g.0 += 1;
                g.1 += o.2;
            }
        }
        for (label, (n, sum)) in groups {
            choices.push((q.id.clone(), label, n, sum / n as f64 * 1e4));
        }
    }
    let mut models: Vec<String> = anon.iter().map(|a| a.model.clone()).collect::<HashSet<_>>().into_iter().collect();
    models.sort();
    let mut ms: Vec<u64> = answers.iter().map(|a| a.ms).collect();
    ms.sort_unstable();
    Scorecard {
        probe: probe.name.clone(),
        venue: p.venue.clone(),
        models,
        calls: answers.len(),
        tokens_per_call: if answers.is_empty() { 0.0 } else { answers.iter().map(|a| (a.tokens_in + a.tokens_out) as f64).sum::<f64>() / answers.len() as f64 },
        median_ms: ms.get(ms.len() / 2).copied().unwrap_or(0),
        questions,
        baseline_ic,
        leak,
        stability,
        choices,
    }
}

pub fn markdown(sc: &Scorecard, probe: &Probe, p: &Panel) -> String {
    use std::fmt::Write as _;
    let mut md = String::new();
    let _ = writeln!(md, "# Jev probe `{}` on {}\n", sc.probe, sc.venue);
    let _ = writeln!(md, "*{}{} · horizon {} bars · model {}*\n", p.describe(), if probe.hypothesis.is_empty() { String::new() } else { format!(" · hypothesis {}", probe.hypothesis) }, probe.horizon_bars, if sc.models.is_empty() { "none".into() } else { sc.models.join(", ") });
    let _ = writeln!(md, "- {} answers on file · {:.0} tokens per call · median {} ms per call.", sc.calls, sc.tokens_per_call, sc.median_ms);
    if sc.models.len() > 1 {
        let _ = writeln!(md, "- **More than one Jev version answered.** Answers can shift between versions; score them separately before trusting the mix.");
    }
    if sc.questions.is_empty() {
        let _ = writeln!(md, "\nNo scored answers yet (the horizon has to pass before an answer can be judged).");
        return md;
    }
    let _ = writeln!(md, "\n| Question | Scored on | n | Days | Base rate | Mean answer | Brier skill | AUC | IC | IC vs shuffled |\n|---|---|---|---|---|---|---|---|---|---|");
    for q in &sc.questions {
        let _ = writeln!(md, "| `{}` | {} | {} | {} | {:.0}% | {:.2} | {:+.3} | {} | {} | {} |", q.id, q.outcome, q.n, q.days, q.base_rate * 100.0, q.mean_answer, q.brier_skill, q.auc.map(|a| format!("{a:.3}")).unwrap_or("–".into()), q.ic.map(|a| format!("{a:+.3}")).unwrap_or("–".into()), q.ic_null_pct.map(|a| format!("{a:.0}th pct")).unwrap_or("–".into()));
    }
    let _ = writeln!(md, "\nBrier skill > 0 and AUC > 0.5 mean Jev beats always answering the average. \"IC vs shuffled\" is where the IC sits among shuffles that keep each day's answers together: above the 95th percentile is evidence, below is noise.");
    if let Some(b) = sc.baseline_ic {
        let _ = writeln!(md, "\nFor comparison, the size of the stretch alone (|z|) has IC {b:+.3} against the same returns.");
    }
    for q in &sc.questions {
        let _ = writeln!(md, "\n## `{}` by answer quintile\n\n| Mean answer | Came true | Trade return after {} bars | n |\n|---|---|---|---|", q.id, probe.horizon_bars);
        for (a, hit, ret, n) in &q.buckets {
            let _ = writeln!(md, "| {a:.2} | {:.0}% | {ret:+.1} bps | {n} |", hit * 100.0);
        }
        if !q.by_coin.is_empty() {
            let _ = writeln!(md, "\nIC by coin: {}", q.by_coin.iter().map(|(c, n, ic)| format!("{c} {ic:+.2} (n={n})")).collect::<Vec<_>>().join(" · "));
        }
    }
    if !sc.choices.is_empty() {
        let _ = writeln!(md, "\n## Choice answers\n\n| Question | Answer | n | Trade return |\n|---|---|---|---|");
        for (q, label, n, ret) in &sc.choices {
            let _ = writeln!(md, "| `{q}` | {label} | {n} | {ret:+.1} bps |");
        }
    }
    if !sc.leak.is_empty() {
        let _ = writeln!(md, "\n## Memory check (same states, coin and date revealed)\n\n| Question | Pairs | IC anonymous | IC revealed |\n|---|---|---|---|");
        for (q, n, a, r) in &sc.leak {
            let _ = writeln!(md, "| `{q}` | {n} | {a:+.3} | {r:+.3} |");
        }
        let _ = writeln!(md, "\nA revealed IC clearly above the anonymous one means Jev is recalling history, and only anonymous or live answers count.");
    }
    if !sc.stability.is_empty() {
        let _ = writeln!(md, "\n## Stability (same state asked twice)\n");
        for (q, n, d) in &sc.stability {
            let _ = writeln!(md, "- `{q}`: mean absolute difference {d:.3} over {n} pairs.");
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::super::panel::testing::*;
    use super::super::rules::{Costs, Sizing};
    use super::*;

    fn probe() -> Probe {
        toml::from_str(
            r#"
name = "fade_judge"
interval = "15m"
candidates_of = "meanrev:hold=24,n=96,z=2"
horizon_bars = 24
[[questions]]
id = "p_revert"
kind = "noul"
text = "Six hours from now the price is {with} than it is now"
outcome = "fwd_up"
[[questions]]
id = "kind"
kind = "choice"
text = "What kind of move is this"
[questions.options]
spike = "A sudden spike"
grind = "A steady grind"
"#,
        )
        .unwrap()
    }

    fn cfg() -> SimCfg {
        SimCfg::new(10_000.0, Costs::default(), Sizing::default())
    }

    #[test]
    fn state_is_anonymous_and_causal() {
        let full = random_panel(6, 3, 12, "15m");
        let pr = probe();
        let f = Featurizer::new(&full);
        let i = 96 * 8 + 5;
        let s = f.state(1, i, &pr, -1, false).unwrap();
        let text = s.to_string();
        assert!(!text.contains("C1") && !text.contains("time_utc"), "no coin name or date: {text}");
        assert!(text.contains("sell short now"));
        assert_eq!(s["recent_bars_oldest_first"]["return_pct"].as_array().unwrap().len(), 16);
        // No price levels: every number is a move, a ratio or a count of hours.
        let px = full.close[1][i];
        assert!(!text.contains(&format!("{px:.1}")));
        // Cutting the future off changes nothing.
        let mut head = full.head(i + 1);
        head.rederive(NO_RULE);
        assert_eq!(Featurizer::new(&head).state(1, i, &pr, -1, false).unwrap(), s);
        let revealed = f.state(1, i, &pr, 1, true).unwrap().to_string();
        assert!(revealed.contains("C1") && revealed.contains("time_utc") && revealed.contains("buy now"));
        assert!(f.state(1, 50, &pr, 1, false).is_none(), "not enough history");
    }

    #[test]
    fn questions_follow_the_side_and_the_hash_follows_the_questions() {
        let pr = probe();
        let long = serde_json::to_string(&pr.questions_for(1)).unwrap();
        let short = serde_json::to_string(&pr.questions_for(-1)).unwrap();
        assert!(long.contains("price is higher") && short.contains("price is lower"));
        let mut other = pr.clone();
        other.questions[0].text.push('?');
        assert_ne!(pr.hash(), other.hash());
        assert_eq!(pr.hash(), probe().hash());
    }

    #[test]
    fn points_put_real_entries_first() {
        let p = random_panel(6, 3, 12, "15m");
        let pts = points(&p, &probe(), &cfg()).unwrap();
        let entries = pts.iter().take_while(|x| x.entry).count();
        assert!(entries > 10 && entries < pts.len());
        assert!(pts[entries..].iter().all(|x| !x.entry));
        let uniq: HashSet<(usize, usize)> = pts.iter().map(|x| (x.c, x.i)).collect();
        assert_eq!(uniq.len(), pts.len());
        let every = Probe { candidates_of: String::new(), every: 24, ..probe() };
        let pts = points(&p, &every, &cfg()).unwrap();
        assert_eq!(pts.len(), 3 * 12 * 4, "one in 24 bars per coin");
    }

    fn fake(p: &Panel, pr: &Probe, skill: f64, variant: Variant) -> Vec<Answer> {
        let mut rng = crate::stats::Rng::new(5);
        points(p, pr, &cfg()).unwrap().iter().filter_map(|x| {
            let (ret, _) = outcome(p, x.c, x.i, x.side, pr.horizon_bars)?;
            // An oracle blended with noise.
            let a = (0.5 + skill * ret.signum() * 0.2 + 0.15 * rng.normal()).clamp(0.01, 0.99);
            let mut answers = BTreeMap::new();
            answers.insert("p_revert".to_string(), json!(a));
            answers.insert("kind".to_string(), json!(if rng.below(2) == 0 { "spike" } else { "grind" }));
            Some(Answer { probe: pr.name.clone(), hash: pr.hash(), venue: p.venue.clone(), variant, coin: p.coins[x.c].clone(), t: p.time(x.i), side: x.side, model: "jev-test".into(), answers, tokens_in: 700, tokens_out: 60, ms: 400, asked_at: chrono::Utc::now() })
        }).collect()
    }

    #[test]
    fn scoring_tells_skill_from_noise() {
        let p = random_panel(6, 6, 40, "15m");
        let pr = probe();
        let good = score(&p, &pr, &fake(&p, &pr, 1.0, Variant::Anon));
        let q = &good.questions[0];
        assert!(q.n > 300, "{}", q.n);
        assert!(q.brier_skill > 0.05 && q.auc.unwrap() > 0.7 && q.ic.unwrap() > 0.3 && q.ic_null_pct.unwrap() > 99.0, "{q:?}");
        assert!(q.buckets.first().unwrap().1 < q.buckets.last().unwrap().1, "higher answers come true more often");
        let none = score(&p, &pr, &fake(&p, &pr, 0.0, Variant::Anon));
        let q = &none.questions[0];
        assert!(q.brier_skill < 0.01 && (q.auc.unwrap() - 0.5).abs() < 0.07, "{q:?}");
        assert!(q.ic_null_pct.unwrap() < 99.5);
        assert_eq!(good.tokens_per_call, 760.0);
        assert_eq!(good.choices.len(), 2);
        let md = markdown(&good, &pr, &p);
        assert!(md.contains("p_revert") && md.contains("quintile"), "{md}");
    }

    #[test]
    fn answers_become_series_and_paired_variants_are_compared() {
        let p = random_panel(6, 4, 30, "15m");
        let pr = probe();
        let mut all = fake(&p, &pr, 1.0, Variant::Anon);
        let ax = aux(&p, &pr, &all);
        let series = &ax.jev["p_revert"];
        let answered = series.iter().flatten().filter(|v| v.is_finite()).count();
        assert_eq!(answered, all.len());
        assert!(!ax.jev.contains_key("kind"), "choices are not series");
        // The gated family runs on these answers and trades less than the raw rule.
        let raw = strategy::plan(&crate::lab::parse_spec("meanrev:hold=24,n=96,z=2").unwrap(), &p, &Aux::default()).unwrap();
        let gated = strategy::plan(&crate::lab::parse_spec("jev_gate:hold=24,min_p=0.6,n=96,z=2").unwrap(), &p, &ax).unwrap();
        let count = |pl: &strategy::Plan| pl.entry.iter().flatten().filter(|e| **e != 0).count();
        assert!(count(&gated) > 0 && count(&gated) < count(&raw));
        // Revealed answers that know the future show up as a leak; repeats as stability.
        let mut revealed = fake(&p, &pr, 1.0, Variant::Revealed);
        for a in &mut revealed {
            let (c, i) = (p.coin_index(&a.coin).unwrap(), ((a.t - p.t0) / p.interval_ms) as usize);
            let (ret, _) = outcome(&p, c, i, a.side, pr.horizon_bars).unwrap();
            a.answers.insert("p_revert".into(), json!(if ret > 0.0 { 0.95 } else { 0.05 }));
        }
        let mut repeat = fake(&p, &pr, 1.0, Variant::Repeat);
        repeat.truncate(40);
        all.extend(revealed);
        all.extend(repeat);
        let sc = score(&p, &pr, &all);
        let (_, n, ic_a, ic_r) = &sc.leak[0];
        assert!(*n > 100 && ic_r > ic_a && *ic_r > 0.8, "{:?}", sc.leak);
        assert_eq!(sc.stability[0].1, 40);
        assert!(sc.stability[0].2 < 1e-9, "the fake is deterministic");
    }

    #[test]
    fn auc_of_perfect_and_useless_answers() {
        assert_eq!(auc(&[0.1, 0.2, 0.8, 0.9], &[false, false, true, true]), Some(1.0));
        assert_eq!(auc(&[0.5, 0.5, 0.5, 0.5], &[false, true, false, true]), Some(0.5));
        assert_eq!(auc(&[0.5, 0.6], &[true, true]), None);
    }
}
