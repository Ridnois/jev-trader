//! News intelligence for the catalyst recorder, ported from v2 (`src/intel/`, kept on
//! branch `backup/local-main-f26bdc2`): sources (RSS + exchange announcement APIs) →
//! local tagger (coin, noise, price recaps, duplicates; free) → batched Jev scoring
//! (impact, new fact, hack) → events.
//!
//! Token budget, measured on the live API on 2026-11-08: a call carries ~270 input
//! tokens of overhead, so headlines are scored up to `batch_size` per call (~200
//! tokens per headline), and only headlines about something tradable are sent.

pub mod sources;
pub mod tagger;

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};

use crate::bot::append_jsonl;
use crate::jev::{JevClient, Question};
use crate::lab::CatalystCfg;
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

struct Queued {
    item: Item,
    published: DateTime<Utc>,
    first_seen: DateTime<Utc>,
    tag: Tag,
    attempts: u32,
}

const IMPACT_LEVELS: [&str; 5] = ["Strong drop", "Drop", "No effect", "Rise", "Strong rise"];

/// The state and questions for one batch: every headline tagged with its subject.
fn batch_request(batch: &[Queued]) -> (serde_json::Value, IndexMap<String, Question>) {
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
    (serde_json::json!({ "task": "Each item is a crypto news headline tagged with the asset it concerns", "items": items }), q)
}

/// One Jev call for the whole batch.
async fn analyze_batch(jev: &JevClient, batch: &[Queued]) -> Result<Vec<NewsEvent>> {
    let (state, q) = batch_request(batch);
    let r = jev.ask(&state, &q).await?;
    r.validate()?;
    batch.iter().enumerate().map(|(i, b)| {
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
    }).collect()
}

/// Why a fresh item is (not) worth a Jev call.
#[derive(Debug, PartialEq)]
enum Filter {
    Keep,
    Noise,
    Recap,
    Untradable,
}

fn filter(tag: &Tag, kind: Kind) -> Filter {
    if tag.noise && kind == Kind::News {
        Filter::Noise
    } else if tag.recap && kind == Kind::News {
        Filter::Recap
    } else if tag.coins.is_empty() && !tag.market && kind != Kind::Official {
        Filter::Untradable
    } else {
        Filter::Keep
    }
}

/// The news loop: fetch, filter locally, score in batches under an hourly call
/// budget, write every event to `events_path` and send it to the engine.
pub async fn run(cfg: CatalystCfg, jev: JevClient, tagger: Arc<Tagger>, events_path: PathBuf, seen_path: PathBuf, tx: UnboundedSender<NewsEvent>) {
    let http = match reqwest::Client::builder().timeout(Duration::from_secs(10)).user_agent("Mozilla/5.0 (jev-trader)").build() {
        Ok(h) => h,
        Err(e) => return warn!("news http client: {e}"),
    };
    let mut seen: VecDeque<String> = crate::status::read_json(&seen_path).unwrap_or_default();
    let mut seen_set: HashSet<String> = seen.iter().cloned().collect();
    // A first run would otherwise treat everything already in the feeds as fresh.
    let mut priming = seen.is_empty();
    let mut recent: VecDeque<(DateTime<Utc>, String, HashSet<String>)> = VecDeque::new();
    let mut queue: Vec<Queued> = Vec::new();
    let mut calls: VecDeque<Instant> = VecDeque::new();
    let (mut last_rss, mut last_ann) = (None::<Instant>, None::<Instant>);
    let due = |last: Option<Instant>, every: u64| last.is_none_or(|t| t.elapsed() >= Duration::from_secs(every.max(5)));
    let (mut kept, mut dropped) = (0u64, 0u64);
    loop {
        let mut fetched: Vec<(String, Result<Vec<Item>>)> = Vec::new();
        if due(last_ann, cfg.announcements_every_secs) {
            last_ann = Some(Instant::now());
            fetched.extend(futures::future::join_all(cfg.announcements.clone().into_iter().map(|v| {
                let http = http.clone();
                async move {
                    let r = sources::fetch_announcements(&http, &v).await;
                    (v, r)
                }
            })).await);
        }
        if due(last_rss, cfg.rss_every_secs) {
            last_rss = Some(Instant::now());
            let r: Vec<_> = futures::stream::iter(cfg.rss.clone()).map(|u| {
                let http = http.clone();
                async move { (sources::source_name(&u), sources::fetch_rss(&http, &u).await) }
            }).buffer_unordered(8).collect().await;
            fetched.extend(r);
        }
        let now = Utc::now();
        let max_age = chrono::Duration::minutes(cfg.max_age_min);
        let mut new_seen = false;
        for (name, res) in fetched {
            let items = match res {
                Ok(i) => i,
                Err(e) => {
                    tracing::debug!("{name}: {e:#}");
                    continue;
                }
            };
            for item in items {
                if !seen_set.insert(item.id.clone()) {
                    continue;
                }
                seen.push_back(item.id.clone());
                new_seen = true;
                if priming {
                    continue;
                }
                // Undated or stale items are context, not news.
                let Some(published) = item.published.filter(|p| now - *p < max_age && *p <= now + chrono::Duration::minutes(5)) else { continue };
                let tag = tagger.tag(&item.title, &item.summary);
                if filter(&tag, item.kind) != Filter::Keep {
                    dropped += 1;
                    continue;
                }
                let tk = tokens(&item.title);
                let coin = tag.coins.first().cloned().unwrap_or_default();
                if recent.iter().any(|(_, c, t)| *c == coin && jaccard(t, &tk) >= cfg.dedupe_jaccard) {
                    dropped += 1;
                    continue;
                }
                recent.push_back((now, coin, tk));
                kept += 1;
                queue.push(Queued { item, published, first_seen: now, tag, attempts: 0 });
            }
        }
        if priming && new_seen {
            info!("news: {} items already in the feeds marked as seen; only what arrives from now on is scored", seen.len());
            priming = false;
        }
        while seen.len() > 8000 {
            if let Some(old) = seen.pop_front() {
                seen_set.remove(&old);
            }
        }
        while recent.front().is_some_and(|(t, _, _)| now - *t > chrono::Duration::hours(6)) {
            recent.pop_front();
        }
        if new_seen && let Ok(b) = serde_json::to_vec(&seen) {
            let _ = crate::status::write_atomic(&seen_path, &b);
        }
        // Exchange announcements first: they are the time-critical catalysts.
        queue.sort_by_key(|q| (q.item.kind == Kind::News, q.first_seen));
        while calls.front().is_some_and(|t| t.elapsed() > Duration::from_secs(3600)) {
            calls.pop_front();
        }
        while !queue.is_empty() && calls.len() < cfg.max_calls_per_hour {
            let n = queue.len().min(cfg.batch_size.max(1));
            let batch: Vec<Queued> = queue.drain(..n).collect();
            calls.push_back(Instant::now());
            match analyze_batch(&jev, &batch).await {
                Ok(events) => {
                    for e in events {
                        info!(coin = e.coins.first().map(String::as_str).unwrap_or("market"), impact = format!("{:+.2}", e.impact), new = format!("{:.2}", e.new_info), lag = format!("{:.0}m", e.lag_min), "news [{}] {}", e.source, e.title);
                        append_jsonl(&events_path, &e);
                        let _ = tx.send(e);
                    }
                }
                Err(e) => {
                    warn!("Jev batch of {n} failed: {e:#}");
                    queue.extend(batch.into_iter().filter_map(|mut q| {
                        q.attempts += 1;
                        (q.attempts < 3).then_some(q)
                    }));
                    break;
                }
            }
        }
        // Items that waited too long for budget stop being news.
        queue.retain(|q| now - q.published < max_age);
        tracing::debug!(kept, dropped, queued = queue.len(), "news cycle");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(title: &str, coins: &[&str], kind: Kind, hack: bool) -> Queued {
        Queued {
            item: Item { id: title.into(), title: title.into(), summary: "Details follow in the article body".into(), source: "test".into(), published: Some(Utc::now()), kind },
            published: Utc::now(),
            first_seen: Utc::now(),
            tag: Tag { coins: coins.iter().map(|c| c.to_string()).collect(), hack_words: hack, ..Default::default() },
            attempts: 0,
        }
    }

    #[test]
    fn batches_two_questions_per_headline_and_a_third_for_hacks() {
        let batch = [queued("Binance will list ZEC perpetuals", &["ZEC"], Kind::Listing, false), queued("Bridge drained of $40M", &["ETH"], Kind::News, true), queued("Fed holds rates", &[], Kind::Official, false)];
        let (state, q) = batch_request(&batch);
        assert_eq!(q.len(), 2 + 3 + 2);
        assert!(q.contains_key("k1") && !q.contains_key("k0"));
        let items = state["items"].as_object().unwrap();
        assert!(items["h0"].as_str().unwrap().starts_with("[ZEC · exchange listing] Binance will list"));
        assert!(items["h2"].as_str().unwrap().starts_with("[crypto market · official release]"));
    }

    #[test]
    fn local_filter_spares_jev_the_noise() {
        let t = Tagger::new(&["BTC".to_string(), "ZEC".to_string()]);
        assert_eq!(filter(&t.tag("Bitcoin drops 6% to $70,000 as longs get liquidated", ""), Kind::News), Filter::Recap);
        assert_eq!(filter(&t.tag("Will Bitcoin recover this week?", ""), Kind::News), Filter::Noise);
        assert_eq!(filter(&t.tag("Local bakery wins an award", ""), Kind::News), Filter::Untradable);
        assert_eq!(filter(&t.tag("Zcash (ZEC) to be listed on a major exchange", ""), Kind::Listing), Filter::Keep);
    }
}
