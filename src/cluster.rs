//! 5.1: group headlines that report the same story, so three outlets covering one
//! event are not counted as three independent signals. A cheap title-token Jaccard
//! decides clear cases; borderline pairs can be sent to Jev.

use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};

use crate::news::NewsEvent;

/// At or above this, two titles are the same story.
pub const SAME: f64 = 0.5;
/// Between this and `SAME`, ask Jev (if enabled); below, different stories.
pub const BORDERLINE: f64 = 0.25;
/// Only look this far back for a matching story.
pub const WINDOW_HOURS: i64 = 12;

const STOP: &[&str] = &[
    "the", "and", "for", "with", "from", "that", "this", "into", "after", "over", "amid", "says",
    "said", "will", "could", "would", "its", "are", "was", "has", "have", "new", "why", "how",
    "what", "price", "crypto", "news", "today", "report", "reports", "about", "than", "more",
];

pub fn tokens(title: &str) -> HashSet<String> {
    title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '$')
        .filter(|t| t.len() >= 3 && !STOP.contains(t))
        .map(|t| t.trim_end_matches('s').to_string())
        .collect()
}

pub fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 { 0.0 } else { inter / union }
}

pub enum Match<'a> {
    /// Same story as this cluster id.
    Same(String),
    /// Possible duplicates, best first: ask Jev.
    Borderline(Vec<&'a NewsEvent>),
    New,
}

/// Compare a new headline with recent events about the same asset.
pub fn find<'a>(title: &str, asset: &str, now: DateTime<Utc>, recent: &'a [NewsEvent]) -> Match<'a> {
    let t = tokens(title);
    let cutoff = now - Duration::hours(WINDOW_HOURS);
    let mut scored: Vec<(f64, &NewsEvent)> = recent
        .iter()
        .filter(|e| e.asset == asset && e.seen_at() >= cutoff)
        .map(|e| (jaccard(&t, &tokens(&e.title)), e))
        .filter(|(j, _)| *j >= BORDERLINE)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    match scored.first() {
        Some((j, e)) if *j >= SAME => Match::Same(e.cluster().to_string()),
        Some(_) => Match::Borderline(scored.into_iter().take(5).map(|(_, e)| e).collect()),
        None => Match::New,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_story_scores_high() {
        let a = tokens("SEC approves spot Solana ETF from VanEck");
        let b = tokens("VanEck's spot Solana ETF approved by SEC");
        let c = tokens("Bitcoin miners sell reserves as hashprice falls");
        assert!(jaccard(&a, &b) >= BORDERLINE);
        assert!(jaccard(&a, &c) < BORDERLINE);
    }
}
