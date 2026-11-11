//! News sources: RSS/Atom feeds and exchange listing announcements (Binance,
//! Upbit, Bybit, OKX). All keyless. Each fetch returns plain items; tagging,
//! filtering and Jev happen in `intel`.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    News,
    /// An exchange will list (or launch a perp for) a coin.
    Listing,
    /// An exchange will delist a coin, or flags it as risky.
    Delisting,
    /// Central bank / regulator press release.
    Official,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub source: String,
    pub published: Option<DateTime<Utc>>,
    pub kind: Kind,
}

/// A short, stable name for a feed URL ("coindesk.com", "binance").
pub fn source_name(url: &str) -> String {
    let host = url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or(url);
    host.trim_start_matches("www.").trim_start_matches("api.").to_string()
}

pub async fn fetch_rss(http: &reqwest::Client, url: &str) -> Result<Vec<Item>> {
    let bytes = http.get(url).send().await?.error_for_status()?.bytes().await?;
    let feed = feed_rs::parser::parse(&bytes[..]).with_context(|| format!("parsing feed {url}"))?;
    let source = source_name(url);
    let official = source.contains("federalreserve.gov") || source.contains("sec.gov");
    Ok(feed
        .entries
        .into_iter()
        .filter_map(|e| {
            let mut title = e.title.map(|t| t.content)?.trim().to_string();
            // Google News appends " - Outlet" to every title.
            if source.contains("news.google") && let Some(i) = title.rfind(" - ") {
                title.truncate(i);
            }
            let summary = e.summary.map(|s| truncate(&strip_html(&s.content), 240)).unwrap_or_default();
            let id = e.links.first().map(|l| l.href.clone()).unwrap_or(e.id);
            Some(Item { id, title, summary, source: source.clone(), published: e.published.or(e.updated), kind: if official { Kind::Official } else { Kind::News } })
        })
        .collect())
}

/// Exchange items about stocks / tokenized securities are not crypto catalysts.
fn tradfi(title: &str) -> bool {
    let t = title.to_lowercase();
    ["tradfi", "tokenized", "bstocks", "xstock", "stock", "etf", "equit"].iter().any(|w| t.contains(w))
}

fn ms(v: &Value) -> Option<DateTime<Utc>> {
    let n = v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))?;
    Utc.timestamp_millis_opt(n).single()
}

fn listing_kind(title: &str) -> Kind {
    let t = title.to_lowercase();
    if ["delist", "remove", "cease", "suspend", "monitoring tag", "termination"].iter().any(|w| t.contains(w)) { Kind::Delisting } else { Kind::Listing }
}

pub async fn fetch_announcements(http: &reqwest::Client, venue: &str) -> Result<Vec<Item>> {
    let get = |url: &str| {
        let r = http.get(url.to_string());
        async move { r.send().await?.error_for_status()?.json::<Value>().await.map_err(anyhow::Error::from) }
    };
    let mut out = Vec::new();
    match venue {
        "binance" => {
            // 48 = new listings, 161 = delistings.
            for cat in [48, 161] {
                let v = get(&format!("https://www.binance.com/bapi/composite/v1/public/cms/article/list/query?type=1&catalogId={cat}&pageNo=1&pageSize=10")).await?;
                for a in v["data"]["catalogs"][0]["articles"].as_array().into_iter().flatten() {
                    let title = a["title"].as_str().unwrap_or_default().to_string();
                    if title.is_empty() || tradfi(&title) {
                        continue;
                    }
                    let kind = if cat == 161 { Kind::Delisting } else { listing_kind(&title) };
                    out.push(Item { id: format!("binance:{}", a["code"].as_str().unwrap_or_default()), title: format!("Binance: {title}"), summary: String::new(), source: "binance".into(), published: ms(&a["releaseDate"]), kind });
                }
            }
        }
        "upbit" => {
            let v = get("https://api-manager.upbit.com/api/v1/announcements?os=web&page=1&per_page=20&category=trade").await?;
            for n in v["data"]["notices"].as_array().into_iter().flatten() {
                let ko = n["title"].as_str().unwrap_or_default();
                let Some((kind, en)) = upbit_english(ko) else { continue };
                let published = n["first_listed_at"].as_str().or(n["listed_at"].as_str()).and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|d| d.with_timezone(&Utc));
                out.push(Item { id: format!("upbit:{}", n["id"]), title: en, summary: String::new(), source: "upbit".into(), published, kind });
            }
        }
        "bybit" => {
            let v = get("https://api.bybit.com/v5/announcements/index?locale=en-US&type=new_crypto&limit=15").await?;
            for a in v["result"]["list"].as_array().into_iter().flatten() {
                let title = a["title"].as_str().unwrap_or_default().to_string();
                if title.is_empty() || tradfi(&title) {
                    continue;
                }
                let kind = listing_kind(&title);
                out.push(Item { id: format!("bybit:{}", a["url"].as_str().unwrap_or(&title)), title: format!("Bybit: {title}"), summary: String::new(), source: "bybit".into(), published: ms(&a["publishTime"]), kind });
            }
        }
        "okx" => {
            for t in ["announcements-new-listings", "announcements-delistings"] {
                let v = get(&format!("https://www.okx.com/api/v5/support/announcements?annType={t}")).await?;
                for a in v["data"][0]["details"].as_array().into_iter().flatten() {
                    let title = a["title"].as_str().unwrap_or_default().trim().to_string();
                    if title.is_empty() || tradfi(&title) {
                        continue;
                    }
                    let kind = if t.contains("delist") { Kind::Delisting } else { listing_kind(&title) };
                    out.push(Item { id: format!("okx:{}", a["url"].as_str().unwrap_or(&title)), title: format!("OKX: {title}"), summary: String::new(), source: "okx".into(), published: ms(&a["pTime"]), kind });
                }
            }
        }
        other => bail!("unknown announcement source {other:?} (binance, upbit, bybit, okx)"),
    }
    Ok(out)
}

/// Upbit titles are Korean. Only listing, delisting and warning notices matter;
/// the rest (maintenance, network upgrades) are skipped. "Name(TICKER)" is kept.
fn upbit_english(ko: &str) -> Option<(Kind, String)> {
    let subject = ko.split(" 거래지원").next().unwrap_or(ko).split(" 유의").next().unwrap_or(ko).trim();
    if ko.contains("거래지원 종료") {
        Some((Kind::Delisting, format!("Upbit will delist {subject}")))
    } else if ko.contains("유의 종목 지정") && !ko.contains("해제") {
        Some((Kind::Delisting, format!("Upbit flags {subject} with an investment warning")))
    } else if ko.contains("신규 거래지원") || ko.contains("디지털 자산 추가") || ko.contains("마켓 추가") {
        let s = ko.split(" 신규").next().unwrap_or(ko).split(" (").next().unwrap_or(ko).trim();
        Some((Kind::Listing, format!("Upbit lists {s} for trading")))
    } else {
        None
    }
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
    fn upbit_titles() {
        let (k, t) = upbit_english("하이브달러(HBD) 거래지원 종료 안내 (12/7 15:00)").unwrap();
        assert_eq!(k, Kind::Delisting);
        assert_eq!(t, "Upbit will delist 하이브달러(HBD)");
        let (k, t) = upbit_english("지캐시(ZEC) 신규 거래지원 안내 (KRW, BTC, USDT 마켓)").unwrap();
        assert_eq!(k, Kind::Listing);
        assert!(t.contains("(ZEC)"), "{t}");
        assert!(upbit_english("이더리움(ETH) 네트워크 업그레이드 지원 안내").is_none());
    }

    #[test]
    fn names_and_filters() {
        assert_eq!(source_name("https://www.coindesk.com/arc/outboundfeeds/rss"), "coindesk.com");
        assert!(tradfi("OKX to list TradFi perpetual futures for XLV"));
        assert!(!tradfi("Binance Will List Predict (PREDICT)"));
        assert_eq!(listing_kind("Binance Will Delist ABC"), Kind::Delisting);
        assert_eq!(strip_html("<p>Hello <b>BTC</b></p>"), "Hello BTC");
    }
}
