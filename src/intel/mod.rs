//! News intelligence: one pipeline feeding the whole engine.
//!
//! sources (18 RSS + 4 exchange announcement APIs) → local tagger (coin, noise,
//! duplicates; free) → batched Jev scoring (impact, new fact, hack) → events.
//!
//! Token budget (measured 2026-11-08 against the live API): every Jev call carries
//! ~270 input tokens of fixed overhead. v1 asked 4 questions with long option lists
//! per headline, one headline per call: ~751 tokens per headline, and two processes
//! analyzed the same headlines. v2 tags the coin locally, asks 2 short questions per
//! headline (3 when hack words appear), batches up to `batch_size` per call:
//! ~180 tokens per headline, and only for headlines about something tradable.

pub mod sources;
pub mod tagger;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail};
use chrono::{DateTime, NaiveDate, Utc};
use futures::StreamExt;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};

use crate::config::Config;
use crate::jev::{JevClient, Question, Response};
use crate::status::{Files, append_jsonl, write_atomic};
use sources::{Item, Kind};
use tagger::{Tag, Tagger, jaccard, tokens};

/// One analyzed headline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsEvent {
    pub id: String,
    pub title: String,
    pub source: String,
    pub kind: Kind,
    pub published: DateTime<Utc>,
    pub first_seen: DateTime<Utc>,
    /// Minutes between publication and the bot first seeing it.
    pub lag_min: f64,
    /// Hyperliquid perps named, most likely subject first. Empty = market-wide.
    pub coins: Vec<String>,
    pub market: bool,
    /// Expected effect on the coin's price over the next hours, −1..1.
    pub impact: f64,
    pub impact_conf: f64,
    /// P(a new, concrete, market-moving fact; not opinion, recap or price commentary).
    pub new_info: f64,
    /// P(describes a hack / theft that already happened). Asked only when hack words appear.
    #[serde(default)]
    pub hack: Option<f64>,
    pub model: String,
}

impl NewsEvent {
    pub fn coin(&self) -> Option<&str> {
        self.coins.first().map(String::as_str)
    }

    /// Signed weight of this event: impact × new-fact probability.
    pub fn strength(&self) -> f64 {
        self.impact * self.new_info
    }

    fn age_min(&self, now: DateTime<Utc>) -> f64 {
        (now - self.published).num_seconds().max(0) as f64 / 60.0
    }
}

/// Decayed news bias for one coin, −1..1, plus a hack alert.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Bias {
    pub score: f64,
    pub hack_alert: bool,
    pub n: usize,
}

/// `market` events (no coin) count half for every coin.
pub fn bias(events: &[NewsEvent], coin: &str, now: DateTime<Utc>, half_life_min: f64) -> Bias {
    let mut sum = 0.0;
    let mut n = 0;
    let mut hack_alert = false;
    for e in events {
        if e.first_seen > now {
            continue;
        }
        let rel = if e.coins.iter().any(|c| c == coin) {
            if e.coin() == Some(coin) { 1.0 } else { 0.5 }
        } else if e.coins.is_empty() && e.market {
            0.5
        } else {
            continue;
        };
        let age = e.age_min(now);
        if rel == 1.0 && e.hack.unwrap_or(0.0) >= 0.7 && e.new_info >= 0.5 && age < 12.0 * 60.0 {
            hack_alert = true;
        }
        let w = rel * e.strength() * 0.5f64.powf(age / half_life_min.max(1.0));
        if w.abs() > 1e-4 {
            sum += w;
            n += 1;
        }
    }
    Bias { score: sum.tanh(), hack_alert, n }
}

// ---------------------------------------------------------------- Jev budget

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JevUsage {
    pub day: Option<NaiveDate>,
    pub calls: u64,
    pub input: u64,
    pub output: u64,
    pub headlines: u64,
    pub reviews: u64,
    pub failures: u64,
}

/// Every Jev call goes through here: hourly ceiling, usage log, daily totals.
#[derive(Clone)]
pub struct JevGate {
    pub client: JevClient,
    inner: Arc<Mutex<GateInner>>,
    max_per_hour: u32,
    log: std::path::PathBuf,
}

struct GateInner {
    window: VecDeque<DateTime<Utc>>,
    usage: JevUsage,
}

#[derive(Serialize)]
struct UsageRecord<'a> {
    ts: DateTime<Utc>,
    purpose: &'a str,
    items: usize,
    ok: bool,
    input: u64,
    output: u64,
    ms: u128,
}

impl JevGate {
    pub fn new(client: JevClient, max_per_hour: u32, files: &Files) -> Self {
        Self { client, inner: Arc::new(Mutex::new(GateInner { window: VecDeque::new(), usage: JevUsage::default() })), max_per_hour, log: files.jev_usage.clone() }
    }

    pub fn usage(&self) -> JevUsage {
        self.inner.lock().unwrap().usage.clone()
    }

    pub fn calls_last_hour(&self) -> usize {
        let now = Utc::now();
        let mut g = self.inner.lock().unwrap();
        while g.window.front().is_some_and(|t| now - *t > chrono::Duration::hours(1)) {
            g.window.pop_front();
        }
        g.window.len()
    }

    pub fn has_budget(&self) -> bool {
        self.calls_last_hour() < self.max_per_hour as usize
    }

    /// `items` = headlines in a batch (0 for a trade review).
    pub async fn ask<S: Serialize + ?Sized>(&self, purpose: &str, items: usize, state: &S, q: &IndexMap<String, Question>) -> Result<Response> {
        if !self.has_budget() {
            bail!("Jev hourly budget used ({} calls)", self.max_per_hour);
        }
        let now = Utc::now();
        self.inner.lock().unwrap().window.push_back(now);
        let t0 = std::time::Instant::now();
        let r = self.client.ask(state, q).await.and_then(|r| r.validate().map(|_| r));
        let (input, output) = r.as_ref().ok().and_then(|r| r.usage.as_ref()).map(|u| (u.input_tokens, u.output_tokens)).unwrap_or((0, 0));
        {
            let mut g = self.inner.lock().unwrap();
            let u = &mut g.usage;
            if u.day != Some(now.date_naive()) {
                *u = JevUsage { day: Some(now.date_naive()), ..Default::default() };
            }
            u.calls += 1;
            u.input += input;
            u.output += output;
            if r.is_ok() {
                if items > 0 { u.headlines += items as u64 } else { u.reviews += 1 }
            } else {
                u.failures += 1;
            }
        }
        append_jsonl(&self.log, &UsageRecord { ts: now, purpose, items, ok: r.is_ok(), input, output, ms: t0.elapsed().as_millis() });
        r
    }
}

// ---------------------------------------------------------------- shared state

#[derive(Debug, Clone, Default, Serialize)]
pub struct SourceHealth {
    pub ok: bool,
    pub items: usize,
    pub last_ok: Option<DateTime<Utc>>,
    pub error: String,
    /// Median minutes from publication to first sight, over recent fresh items.
    pub median_lag_min: Option<f64>,
    #[serde(skip)]
    lags: VecDeque<f64>,
}

/// Why fresh items never reached Jev, counted since start.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Funnel {
    pub fresh: u64,
    pub noise: u64,
    /// Reports of price moves that already happened (momentum, not news).
    pub recap: u64,
    pub untradable: u64,
    pub duplicate: u64,
    pub analyzed: u64,
    pub failed: u64,
    pub dropped: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Persisted {
    events: Vec<NewsEvent>,
    seen: VecDeque<String>,
}

#[derive(Default)]
pub struct IntelState {
    pub events: Vec<NewsEvent>,
    seen: VecDeque<String>,
    seen_set: HashSet<String>,
    pub sources: HashMap<String, SourceHealth>,
    pub funnel: Funnel,
    pub queue_len: usize,
    pub last_cycle: Option<DateTime<Utc>>,
}

impl IntelState {
    fn mark_seen(&mut self, id: &str) {
        if self.seen_set.insert(id.to_string()) {
            self.seen.push_back(id.to_string());
            while self.seen.len() > 20_000 {
                if let Some(old) = self.seen.pop_front() {
                    self.seen_set.remove(&old);
                }
            }
        }
    }
}

pub type Intel = Arc<Mutex<IntelState>>;

/// A queued headline waiting for its Jev batch.
struct Queued {
    item: Item,
    published: DateTime<Utc>,
    first_seen: DateTime<Utc>,
    tag: Tag,
    attempts: u32,
}

pub fn load(files: &Files) -> Intel {
    let p: Persisted = crate::status::read_json(&files.news_state).unwrap_or_default();
    let mut s = IntelState { events: p.events, ..Default::default() };
    for id in p.seen {
        s.mark_seen(&id);
    }
    Arc::new(Mutex::new(s))
}

fn save(intel: &Intel, files: &Files) {
    let s = intel.lock().unwrap();
    let p = Persisted { events: s.events.clone(), seen: s.seen.clone() };
    if let Ok(b) = serde_json::to_vec(&p) {
        let _ = write_atomic(&files.news_state, &b);
    }
}

/// The news loop. New events are also sent to the engine (catalyst triggers).
pub async fn run(cfg: Config, gate: JevGate, tagger: Arc<Tagger>, intel: Intel, files: Files, tx: UnboundedSender<NewsEvent>) {
    let http = match reqwest::Client::builder().timeout(Duration::from_secs(10)).user_agent("Mozilla/5.0 (jev-trader)").build() {
        Ok(h) => h,
        Err(e) => {
            warn!("news http client: {e}");
            return;
        }
    };
    let mut queue: Vec<Queued> = Vec::new();
    let (mut last_rss, mut last_ann) = (None::<std::time::Instant>, None::<std::time::Instant>);
    let due = |last: Option<std::time::Instant>, every: u64| last.is_none_or(|t| t.elapsed() >= Duration::from_secs(every));
    loop {
        let mut fetched: Vec<(String, Result<Vec<Item>>)> = Vec::new();
        if due(last_ann, cfg.news.announcements_every_secs) {
            last_ann = Some(std::time::Instant::now());
            let r = futures::future::join_all(cfg.news.announcements.clone().into_iter().map(|v| {
                let http = http.clone();
                async move {
                    let r = sources::fetch_announcements(&http, &v).await;
                    (v, r)
                }
            }))
            .await;
            fetched.extend(r);
        }
        if due(last_rss, cfg.news.rss_every_secs) {
            last_rss = Some(std::time::Instant::now());
            let r: Vec<_> = futures::stream::iter(cfg.news.rss.clone())
                .map(|u| {
                    let http = http.clone();
                    async move { (sources::source_name(&u), sources::fetch_rss(&http, &u).await) }
                })
                .buffer_unordered(8)
                .collect()
                .await;
            fetched.extend(r);
        }
        let now = Utc::now();
        if !fetched.is_empty() {
            let mut s = intel.lock().unwrap();
            let max_age = chrono::Duration::minutes(cfg.news.max_age_min);
            let recent_tokens: Vec<(String, HashSet<String>)> = s.events.iter().filter(|e| now - e.first_seen < chrono::Duration::hours(6)).map(|e| (e.coin().unwrap_or("").to_string(), tokens(&e.title))).collect();
            let mut batch_tokens: Vec<(String, HashSet<String>)> = queue.iter().map(|q| (q.tag.coins.first().cloned().unwrap_or_default(), tokens(&q.item.title))).collect();
            for (name, res) in fetched {
                let h = s.sources.entry(name.clone()).or_default();
                let items = match res {
                    Ok(i) => {
                        h.ok = true;
                        h.items = i.len();
                        h.last_ok = Some(now);
                        h.error.clear();
                        i
                    }
                    Err(e) => {
                        h.ok = false;
                        h.error = format!("{e:#}").chars().take(160).collect();
                        continue;
                    }
                };
                for item in items {
                    if s.seen_set.contains(&item.id) {
                        continue;
                    }
                    s.mark_seen(&item.id);
                    // Undated or stale items are context, not news.
                    let Some(published) = item.published.filter(|p| now - *p < max_age && *p <= now + chrono::Duration::minutes(5)) else { continue };
                    let lag = (now - published).num_seconds().max(0) as f64 / 60.0;
                    let h = s.sources.get_mut(&name).expect("inserted");
                    h.lags.push_back(lag);
                    if h.lags.len() > 50 {
                        h.lags.pop_front();
                    }
                    let mut v: Vec<f64> = h.lags.iter().copied().collect();
                    v.sort_by(f64::total_cmp);
                    h.median_lag_min = v.get(v.len() / 2).copied();
                    s.funnel.fresh += 1;
                    let tag = tagger.tag(&item.title, &item.summary);
                    if tag.noise && item.kind == Kind::News {
                        s.funnel.noise += 1;
                        continue;
                    }
                    if tag.recap && item.kind == Kind::News {
                        s.funnel.recap += 1;
                        continue;
                    }
                    if tag.coins.is_empty() && !tag.market && item.kind != Kind::Official {
                        s.funnel.untradable += 1;
                        continue;
                    }
                    let tk = tokens(&item.title);
                    let coin = tag.coins.first().cloned().unwrap_or_default();
                    if recent_tokens.iter().chain(batch_tokens.iter()).any(|(c, t)| *c == coin && jaccard(t, &tk) >= cfg.news.dedupe_jaccard) {
                        s.funnel.duplicate += 1;
                        continue;
                    }
                    batch_tokens.push((coin, tk));
                    queue.push(Queued { item, published, first_seen: now, tag, attempts: 0 });
                }
            }
        }
        // Exchange announcements first: they are the time-critical catalysts.
        queue.sort_by_key(|q| (q.item.kind == Kind::News, q.first_seen));
        let mut new_events = Vec::new();
        while !queue.is_empty() && gate.has_budget() {
            let n = queue.len().min(cfg.jev.batch_size.max(1));
            let batch: Vec<Queued> = queue.drain(..n).collect();
            match analyze_batch(&gate, &batch).await {
                Ok(evs) => new_events.extend(evs),
                Err(e) => {
                    warn!("Jev batch of {n} failed: {e:#}");
                    let mut s = intel.lock().unwrap();
                    for mut q in batch {
                        q.attempts += 1;
                        if q.attempts < cfg.jev.max_attempts {
                            queue.push(q);
                        } else {
                            s.funnel.dropped += 1;
                        }
                    }
                    s.funnel.failed += n as u64;
                    break;
                }
            }
        }
        // Items that waited too long for budget stop being news.
        let max_age = chrono::Duration::minutes(cfg.news.max_age_min);
        let before = queue.len();
        queue.retain(|q| now - q.published < max_age);
        {
            let mut s = intel.lock().unwrap();
            s.funnel.dropped += (before - queue.len()) as u64;
            s.funnel.analyzed += new_events.len() as u64;
            for e in &new_events {
                info!(coin = e.coin().unwrap_or("market"), impact = format!("{:+.2}", e.impact), new = format!("{:.2}", e.new_info), lag = format!("{:.0}m", e.lag_min), "news [{}] {}", e.source, e.title);
                append_jsonl(&files.news, e);
                s.events.push(e.clone());
            }
            let retain = chrono::Duration::hours(cfg.news.retain_hours);
            s.events.retain(|e| now - e.published < retain);
            s.queue_len = queue.len();
            s.last_cycle = Some(now);
        }
        if !new_events.is_empty() {
            save(&intel, &files);
        }
        for e in new_events {
            let _ = tx.send(e);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

const IMPACT_LEVELS: [&str; 5] = ["Strong drop", "Drop", "No effect", "Rise", "Strong rise"];

/// One Jev call for up to `batch_size` headlines.
async fn analyze_batch(gate: &JevGate, batch: &[Queued]) -> Result<Vec<NewsEvent>> {
    let mut items = IndexMap::new();
    let mut q = IndexMap::new();
    for (i, b) in batch.iter().enumerate() {
        let subject = match (b.tag.coins.first(), b.item.kind) {
            (Some(c), Kind::Listing) => format!("{c} · exchange listing"),
            (Some(c), Kind::Delisting) => format!("{c} · exchange delisting/warning"),
            (Some(c), _) => c.clone(),
            (None, Kind::Official) => "crypto market · official release".into(),
            (None, _) => "crypto market".into(),
        };
        let mut text = format!("[{subject}] {}", b.item.title);
        if !b.item.summary.is_empty() {
            text.push_str(" — ");
            text.extend(b.item.summary.chars().take(160));
        }
        items.insert(format!("h{i}"), text);
        q.insert(format!("s{i}"), Question::score(&format!("h{i}: effect on the tagged asset's price over the next few hours"), &IMPACT_LEVELS));
        q.insert(format!("n{i}"), Question::noul(&format!("h{i} is a new, concrete, market-moving fact (not opinion, analysis, recap, or a report of price moves)")));
        if b.tag.hack_words {
            q.insert(format!("k{i}"), Question::noul(&format!("h{i} reports a hack, exploit or theft of funds that already happened")));
        }
    }
    let state = serde_json::json!({ "task": "Each item is a crypto news headline tagged with the asset it concerns", "items": items });
    let r = gate.ask("headlines", batch.len(), &state, &q).await?;
    batch
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let (s01, conf) = r.score_expected(&format!("s{i}"))?;
            let hack = if b.tag.hack_words { Some(r.noul(&format!("k{i}"))?) } else { None };
            Ok(NewsEvent {
                id: b.item.id.clone(),
                title: b.item.title.clone(),
                source: b.item.source.clone(),
                kind: b.item.kind,
                published: b.published,
                first_seen: b.first_seen,
                lag_min: (b.first_seen - b.published).num_seconds().max(0) as f64 / 60.0,
                coins: b.tag.coins.clone(),
                market: b.tag.market || b.item.kind == Kind::Official,
                impact: s01 * 2.0 - 1.0,
                impact_conf: conf,
                new_info: r.noul(&format!("n{i}"))?,
                hack,
                model: r.model.clone(),
            })
        })
        .collect()
}

/// Catalyst entry review: one call, two questions. Jev never starts a trade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review {
    pub priced_in: f64,
    pub continues: f64,
}

pub async fn review(gate: &JevGate, e: &NewsEvent, coin: &str, long: bool, move_pct: f64, move_atr: f64, funding_bps_1h: f64) -> Result<Review> {
    let dir = if long { "rise" } else { "fall" };
    let state = serde_json::json!({
        "asset": coin,
        "headline": e.title,
        "source": e.source,
        "minutes_since_published": ((Utc::now() - e.published).num_seconds() as f64 / 60.0).round(),
        "price_change_since_published_pct": (move_pct * 100.0).round() / 100.0,
        "price_change_in_typical_hourly_ranges": (move_atr * 100.0).round() / 100.0,
        "hourly_funding_bps": (funding_bps_1h * 100.0).round() / 100.0,
    });
    let mut q = IndexMap::new();
    q.insert("priced_in".into(), Question::noul("The price has already fully reacted to this headline"));
    q.insert("continues".into(), Question::noul(&format!("The price of {coin} will {dir} further over the next hour because of this headline")));
    let r = gate.ask("review", 0, &state, &q).await?;
    Ok(Review { priced_in: r.noul("priced_in")?, continues: r.noul("continues")? })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(coins: &[&str], market: bool, impact: f64, new_info: f64, age_min: i64, hack: Option<f64>) -> NewsEvent {
        let now = Utc::now() - chrono::Duration::seconds(1);
        let t = now - chrono::Duration::minutes(age_min);
        NewsEvent {
            id: format!("{coins:?}{age_min}{impact}"),
            title: "t".into(),
            source: "s".into(),
            kind: Kind::News,
            published: t,
            first_seen: t,
            lag_min: 0.0,
            coins: coins.iter().map(|s| s.to_string()).collect(),
            market,
            impact,
            impact_conf: 1.0,
            new_info,
            hack,
            model: "m".into(),
        }
    }

    #[test]
    fn bias_decays_and_weights_relevance() {
        let now = Utc::now();
        let fresh = bias(&[ev(&["SOL"], false, 0.8, 1.0, 0, None)], "SOL", now, 60.0);
        let old = bias(&[ev(&["SOL"], false, 0.8, 1.0, 60, None)], "SOL", now, 60.0);
        assert!((fresh.score - 0.8f64.tanh()).abs() < 1e-3);
        assert!((old.score - 0.4f64.tanh()).abs() < 1e-2, "one half-life halves the weight");
        let mkt = bias(&[ev(&[], true, 0.8, 1.0, 0, None)], "SOL", now, 60.0);
        assert!((mkt.score - 0.4f64.tanh()).abs() < 1e-3, "market news counts half");
        assert_eq!(bias(&[ev(&["ETH"], false, 0.8, 1.0, 0, None)], "SOL", now, 60.0).n, 0);
        // Opinion (low new_info) barely moves it.
        assert!(bias(&[ev(&["SOL"], false, 0.8, 0.05, 0, None)], "SOL", now, 60.0).score < 0.05);
    }

    #[test]
    fn hack_alert_needs_the_coin_itself() {
        let now = Utc::now();
        assert!(bias(&[ev(&["SUI"], false, -0.6, 0.9, 10, Some(0.9))], "SUI", now, 60.0).hack_alert);
        assert!(!bias(&[ev(&["ETH", "SUI"], false, -0.6, 0.9, 10, Some(0.9))], "SUI", now, 60.0).hack_alert);
        assert!(!bias(&[ev(&["SUI"], false, -0.6, 0.9, 10, Some(0.2))], "SUI", now, 60.0).hack_alert);
    }
}
