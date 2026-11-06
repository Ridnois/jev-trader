//! RSS/Atom ingestion and Jev-based feature extraction.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::config::Asset;
use crate::jev::{JevClient, Question};

#[derive(Debug, Clone)]
pub struct NewsItem {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub source: String,
    /// `None` when the feed gave no date: such items never count as fresh (0.2).
    pub published: Option<DateTime<Utc>>,
}

/// Typed features Jev extracted from one news item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsEvent {
    pub id: String,
    pub title: String,
    pub source: String,
    pub published: DateTime<Utc>,
    pub analyzed_at: DateTime<Utc>,
    pub asset: String,
    pub asset_conf: f64,
    pub event: String,
    pub event_conf: f64,
    /// Expected short-term impact, -1 (strongly negative) .. +1 (strongly positive).
    pub sentiment: f64,
    pub sentiment_conf: f64,
    /// Probability this is new, concrete, verifiable information.
    pub new_info: f64,
    pub model: String,
    /// When the bot first saw the headline in a feed. Point-in-time anchor for
    /// settlement and replay (older records fall back to `analyzed_at`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_seen_at: Option<DateTime<Utc>>,
    /// Same-story group (5.1); the id of the cluster's first headline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_in: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_out: Option<u64>,
}

impl NewsEvent {
    /// When this event became visible to the bot.
    pub fn seen_at(&self) -> DateTime<Utc> {
        self.first_seen_at.unwrap_or(self.analyzed_at)
    }

    pub fn cluster(&self) -> &str {
        self.cluster_id.as_deref().unwrap_or(&self.id)
    }
}

pub async fn fetch_feed(http: &reqwest::Client, url: &str) -> Result<Vec<NewsItem>> {
    let bytes = http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let feed = feed_rs::parser::parse(&bytes[..]).with_context(|| format!("parsing feed {url}"))?;
    let source = feed
        .title
        .map(|t| t.content)
        .unwrap_or_else(|| url.to_string());
    Ok(feed
        .entries
        .into_iter()
        .filter_map(|e| {
            let title = e.title.map(|t| t.content)?.trim().to_string();
            let summary = e
                .summary
                .map(|s| truncate(&strip_html(&s.content), 600))
                .unwrap_or_default();
            let id = e
                .links
                .first()
                .map(|l| l.href.clone())
                .unwrap_or(e.id);
            Some(NewsItem {
                id,
                title,
                summary,
                source: source.clone(),
                published: e.published.or(e.updated),
            })
        })
        .collect())
}

fn questions(assets: &[Asset]) -> IndexMap<String, Question> {
    let mut asset_opts: Vec<(String, String)> = assets
        .iter()
        .map(|a| (a.symbol.clone(), format!("{} ({})", a.name, a.symbol)))
        .collect();
    asset_opts.push(("market".into(), "The crypto market broadly, not one specific coin".into()));
    asset_opts.push(("other".into(), "A different coin, or not about crypto".into()));

    let mut q = IndexMap::new();
    q.insert(
        "asset".into(),
        Question::choice("Which asset is this news primarily about", asset_opts),
    );
    q.insert(
        "event".into(),
        Question::choice(
            "What kind of event does this describe",
            [
                ("hack", "Exploit, hack, theft, or protocol failure"),
                ("listing", "Exchange listing or delisting"),
                ("regulation", "Regulatory, legal, or enforcement action"),
                ("macro", "Macro, interest rates, ETF flows, institutional adoption"),
                ("partnership", "Partnership, integration, or product launch"),
                ("noise", "Opinion, price commentary, rumor, or no concrete event"),
            ],
        ),
    );
    q.insert(
        "sentiment".into(),
        Question::score(
            "Likely short-term price impact on that asset",
            &[
                "Strongly negative",
                "Negative",
                "Neutral",
                "Positive",
                "Strongly positive",
            ],
        ),
    );
    q.insert(
        "new_info".into(),
        Question::noul("This reports a new, concrete, verifiable fact rather than opinion or recap"),
    );
    q
}

pub async fn analyze(
    client: &JevClient,
    item: &NewsItem,
    assets: &[Asset],
    first_seen_at: DateTime<Utc>,
) -> Result<NewsEvent> {
    let state = format!(
        "Source: {}\nPublished: {}\nHeadline: {}\nSummary: {}",
        item.source,
        item.published.map(|p| p.to_rfc3339()).unwrap_or_else(|| "unknown".into()),
        item.title,
        item.summary
    );
    let r = client.ask(&state, &questions(assets)).await?;
    r.validate()?;
    let (asset, asset_conf) = r.choice("asset")?;
    let (event, event_conf) = r.choice("event")?;
    let (sent01, sentiment_conf) = r.score_expected("sentiment")?;
    Ok(NewsEvent {
        id: item.id.clone(),
        title: item.title.clone(),
        source: item.source.clone(),
        published: item.published.context("undated item")?,
        analyzed_at: Utc::now(),
        asset: asset.to_string(),
        asset_conf,
        event: event.to_string(),
        event_conf,
        sentiment: sent01 * 2.0 - 1.0,
        sentiment_conf,
        new_info: r.noul("new_info")?,
        model: r.model.clone(),
        first_seen_at: Some(first_seen_at),
        cluster_id: None,
        tokens_in: r.usage.as_ref().map(|u| u.input_tokens),
        tokens_out: r.usage.as_ref().map(|u| u.output_tokens),
    })
}

/// 5.2 adversarial check on the headlines that are about to drive an entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Challenge {
    pub priced_in: f64,
    pub unverified: f64,
    /// Strength of the strongest bear case over 24h, 0..1.
    pub bear: f64,
}

impl Challenge {
    pub fn worst(&self) -> f64 {
        self.priced_in.max(self.unverified).max(self.bear)
    }
}

pub async fn challenge(client: &JevClient, asset: &Asset, headlines: &[String]) -> Result<Challenge> {
    let state = format!(
        "Proposed trade: buy {} ({}) now, holding up to 24 hours, because of these headlines:\n- {}",
        asset.name,
        asset.symbol,
        headlines.join("\n- ")
    );
    let mut q = IndexMap::new();
    q.insert("priced_in".into(), Question::noul("This news is already priced in or was widely known before these headlines"));
    q.insert(
        "unverified".into(),
        Question::noul("The headlines contradict each other or rest on a single unverified source"),
    );
    q.insert(
        "bear".into(),
        Question::score(
            "Strength of the strongest case that the price falls over the next 24 hours",
            &["No credible bear case", "Weak", "Moderate", "Strong", "Overwhelming"],
        ),
    );
    let r = client.ask(&state, &q).await?;
    r.validate()?;
    let c = Challenge { priced_in: r.noul("priced_in")?, unverified: r.noul("unverified")?, bear: r.score_expected("bear")?.0 };
    crate::validate::challenge(&c)?;
    Ok(c)
}

/// Scalper entry review: Jev reads the recent headlines next to a plain-language
/// description of the market move and judges the news side only. It cannot start a
/// trade; it can block one or halve its size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScalpReview {
    /// P(recent news makes a sharp drop likely over the holding period).
    pub adverse: f64,
    /// P(a breaking or scheduled event could move the asset sharply right now).
    pub event_risk: f64,
    /// How well the news supports a continued rise, 0 (contradicts) .. 1 (strong support).
    pub support: f64,
    pub headlines: usize,
}

/// `short`: the proposed trade sells and profits from a fall, so "adverse" means a
/// sharp rise and "support" means support for a continued fall.
pub async fn scalp_review(
    client: &JevClient,
    asset: &Asset,
    headlines: &[serde_json::Value],
    market_move: &str,
    hold_minutes: i64,
    short: bool,
) -> Result<ScalpReview> {
    let (trade, against, with) = if short {
        ("sell now and buy back", "rise", "fall")
    } else {
        ("buy now and sell", "drop", "rise")
    };
    let state = serde_json::json!({
        "asset": format!("{} ({})", asset.name, asset.symbol),
        "proposed_trade": format!("{trade} within {hold_minutes} minutes, profiting if the price {}s", with),
        "market_move": market_move,
        "recent_headlines": headlines,
    });
    let mut q = IndexMap::new();
    q.insert(
        "adverse".into(),
        Question::noul(&format!("Something in these headlines makes a sharp {against} in {} over the next {hold_minutes} minutes likely", asset.symbol)),
    );
    q.insert(
        "event_risk".into(),
        Question::noul(&format!(
            "A breaking or scheduled event in these headlines (hack, enforcement, macro release, exchange outage) could move {} sharply right now",
            asset.symbol
        )),
    );
    q.insert(
        "support".into(),
        Question::score(
            &format!("How well the news explains and supports a continued {with} in {} over the next {hold_minutes} minutes", asset.symbol),
            &["Contradicts it", "Unrelated or no relevant news", "Weak support", "Strong support"],
        ),
    );
    let r = client.ask(&state, &q).await?;
    r.validate()?;
    Ok(ScalpReview { adverse: r.noul("adverse")?, event_risk: r.noul("event_risk")?, support: r.score_expected("support")?.0, headlines: headlines.len() })
}

/// 5.1: for borderline duplicates, ask Jev whether `title` reports the same event as
/// one of `candidates` (id, title). Returns the matching id when Jev is confident.
pub async fn same_event(client: &JevClient, title: &str, candidates: &[(String, String)]) -> Result<Option<String>> {
    let mut opts: Vec<(String, String)> =
        candidates.iter().enumerate().map(|(i, (_, t))| (format!("c{i}"), t.clone())).collect();
    opts.push(("none".into(), "None of these: it is a different event".into()));
    let mut q = IndexMap::new();
    q.insert("same".into(), Question::choice("Which earlier headline reports the same underlying event", opts));
    let r = client.ask(&format!("New headline: {title}"), &q).await?;
    r.validate()?;
    let (choice, conf) = r.choice("same")?;
    if conf < 0.6 || choice == "none" {
        return Ok(None);
    }
    let i: usize = choice.trim_start_matches('c').parse().unwrap_or(usize::MAX);
    Ok(candidates.get(i).map(|(id, _)| id.clone()))
}

fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_html() {
        assert_eq!(strip_html("<p>Hello <b>BTC</b></p>\n world"), "Hello BTC world");
    }
}
