//! Local market-data archive for research: candles and funding, append-only CSV
//! under `<lab data dir>/candles/<venue>/<interval>/<coin>.csv` and
//! `funding/<venue>/<coin>.csv`.
//!
//! Two venues:
//! - `hyperliquid`: where the bots trade. Its API keeps only the latest 5000 candles
//!   per interval, so the archive is what makes history grow from now on.
//! - `binance`: USDT-M perpetual klines, years deep. **Research only**: prices are a
//!   proxy (the perps are arbitraged against each other), costs are never taken from it.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::hyperliquid::{self, num};
use crate::market::{Candle, interval_ms};

pub const HL: &str = "hyperliquid";
pub const BINANCE: &str = "binance";
const BINANCE_API: &str = "https://fapi.binance.com";

pub struct Archive {
    root: PathBuf,
}

impl Archive {
    pub fn new(root: &Path) -> Self {
        Self { root: root.to_path_buf() }
    }

    pub fn candle_path(&self, venue: &str, interval: &str, coin: &str) -> PathBuf {
        self.root.join("candles").join(venue).join(interval).join(format!("{coin}.csv"))
    }

    pub fn funding_path(&self, venue: &str, coin: &str) -> PathBuf {
        self.root.join("funding").join(venue).join(format!("{coin}.csv"))
    }

    /// Closed candles, oldest first. Missing file = no data.
    pub fn candles(&self, venue: &str, interval: &str, coin: &str) -> Result<Vec<Candle>> {
        let path = self.candle_path(venue, interval, coin);
        let Ok(text) = std::fs::read_to_string(&path) else { return Ok(Vec::new()) };
        let ims = interval_ms(interval).with_context(|| format!("bad interval {interval}"))?;
        let mut out = Vec::with_capacity(text.len() / 48);
        for (ln, line) in text.lines().enumerate() {
            let mut f = line.split(',');
            let mut next = || f.next().with_context(|| format!("{}:{}: short row", path.display(), ln + 1));
            let t: i64 = next()?.parse().with_context(|| format!("{}:{}: bad time", path.display(), ln + 1))?;
            let mut v = [0f64; 5];
            for x in &mut v {
                *x = next()?.parse().with_context(|| format!("{}:{}: bad number", path.display(), ln + 1))?;
            }
            out.push(Candle { open_time: t, open: v[0], high: v[1], low: v[2], close: v[3], volume: v[4], close_time: t + ims - 1 });
        }
        Ok(out)
    }

    /// Appends candles newer than what is stored (the file stays sorted and unique).
    pub fn append_candles(&self, venue: &str, interval: &str, coin: &str, candles: &[Candle]) -> Result<usize> {
        let path = self.candle_path(venue, interval, coin);
        let last = last_time(&path);
        let fresh: Vec<&Candle> = candles.iter().filter(|c| last.is_none_or(|l| c.open_time > l) && valid(c)).collect();
        if fresh.is_empty() {
            return Ok(0);
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        let mut buf = String::with_capacity(fresh.len() * 56);
        let mut prev = last.unwrap_or(i64::MIN);
        let mut n = 0;
        for c in fresh {
            if c.open_time <= prev {
                continue;
            }
            prev = c.open_time;
            buf.push_str(&format!("{},{},{},{},{},{}\n", c.open_time, c.open, c.high, c.low, c.close, c.volume));
            n += 1;
        }
        f.write_all(buf.as_bytes())?;
        Ok(n)
    }

    /// Funding prints `(time_ms, rate)`: the fraction of notional a long pays (a short
    /// receives) at that time. Hyperliquid settles hourly, Binance every 4 or 8 hours.
    pub fn funding(&self, venue: &str, coin: &str) -> Vec<(i64, f64)> {
        let Ok(text) = std::fs::read_to_string(self.funding_path(venue, coin)) else { return Vec::new() };
        text.lines().filter_map(|l| {
            let (t, r) = l.split_once(',')?;
            Some((t.parse().ok()?, r.parse().ok()?))
        }).collect()
    }

    pub fn append_funding(&self, venue: &str, coin: &str, rows: &[(i64, f64)]) -> Result<usize> {
        let path = self.funding_path(venue, coin);
        let last = last_time(&path);
        let mut prev = last.unwrap_or(i64::MIN);
        let mut buf = String::new();
        let mut n = 0;
        for (t, r) in rows {
            if *t > prev && r.is_finite() {
                prev = *t;
                buf.push_str(&format!("{t},{r}\n"));
                n += 1;
            }
        }
        if n > 0 {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::OpenOptions::new().create(true).append(true).open(&path)?.write_all(buf.as_bytes())?;
        }
        Ok(n)
    }
}

fn valid(c: &Candle) -> bool {
    [c.open, c.high, c.low, c.close].iter().all(|x| x.is_finite() && *x > 0.0) && c.high >= c.low && c.volume.is_finite()
}

/// Time of the last row, read from the file's tail.
fn last_time(path: &Path) -> Option<i64> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(512))).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    s.lines().rev().find(|l| !l.is_empty())?.split(',').next()?.parse().ok()
}

async fn get_json(http: &reqwest::Client, url: &str) -> Result<Value> {
    for attempt in 0..4u32 {
        match http.get(url).send().await {
            Ok(r) if r.status().is_success() => return Ok(r.json().await?),
            Ok(r) if r.status().as_u16() == 429 || r.status().as_u16() == 418 => {
                tokio::time::sleep(std::time::Duration::from_secs(20 * (attempt as u64 + 1))).await;
            }
            Ok(r) => bail!("{url}: {}: {}", r.status(), r.text().await.unwrap_or_default()),
            Err(e) if attempt == 3 => return Err(e).with_context(|| format!("GET {url}")),
            Err(_) => tokio::time::sleep(std::time::Duration::from_secs(2)).await,
        }
    }
    bail!("{url}: rate limited")
}

/// Binance USDT-M klines opening in [start_ms, end_ms), closed only.
pub async fn binance_klines(http: &reqwest::Client, symbol: &str, interval: &str, start_ms: i64, end_ms: i64) -> Result<Vec<Candle>> {
    let ims = interval_ms(interval).with_context(|| format!("bad interval {interval}"))?;
    let now = chrono::Utc::now().timestamp_millis();
    let mut out = Vec::new();
    let mut from = start_ms;
    while from < end_ms {
        let url = format!("{BINANCE_API}/fapi/v1/klines?symbol={symbol}&interval={interval}&startTime={from}&limit=1500");
        let v = get_json(http, &url).await?;
        let arr = v.as_array().context("klines: not an array")?;
        if arr.is_empty() {
            break;
        }
        let mut last = from;
        for k in arr {
            let t = k[0].as_i64().context("kline time")?;
            last = t;
            if t >= end_ms || t + ims > now {
                continue;
            }
            out.push(Candle { open_time: t, open: num(&k[1]), high: num(&k[2]), low: num(&k[3]), close: num(&k[4]), volume: num(&k[5]), close_time: t + ims - 1 });
        }
        if arr.len() < 1500 || last + ims <= from {
            break;
        }
        from = last + ims;
        tokio::time::sleep(std::time::Duration::from_millis(250)).await; // 10 weight per call, 2400/min allowed
    }
    Ok(out)
}

pub async fn binance_funding(http: &reqwest::Client, symbol: &str, start_ms: i64) -> Result<Vec<(i64, f64)>> {
    let mut out: Vec<(i64, f64)> = Vec::new();
    let mut from = start_ms;
    loop {
        let url = format!("{BINANCE_API}/fapi/v1/fundingRate?symbol={symbol}&startTime={from}&limit=1000");
        let v = get_json(http, &url).await?;
        let arr = v.as_array().context("fundingRate: not an array")?;
        for r in arr {
            if let Some(t) = r["fundingTime"].as_i64() {
                out.push((t, num(&r["fundingRate"])));
            }
        }
        match out.last() {
            Some((t, _)) if arr.len() == 1000 && *t + 1 > from => from = *t + 1,
            _ => break,
        }
        tokio::time::sleep(std::time::Duration::from_millis(700)).await; // this endpoint allows 500 calls / 5 min
    }
    Ok(out)
}

/// A Hyperliquid info call that waits out rate limits (HTTP 429) instead of failing.
async fn hl_info(http: &reqwest::Client, api: &str, body: Value) -> Result<Value> {
    let mut wait = 15;
    for attempt in 0..5 {
        match hyperliquid::info_at(http, api, body.clone()).await {
            Ok(v) => return Ok(v),
            Err(e) if attempt < 4 && format!("{e:#}").contains("429") => {
                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                wait += 15;
            }
            Err(e) => return Err(e),
        }
    }
    bail!("hyperliquid: rate limited")
}

/// Hyperliquid candles opening in [start_ms, end_ms], closed only, with rate-limit patience.
async fn hl_candles(http: &reqwest::Client, api: &str, coin: &str, interval: &str, start_ms: i64, end_ms: i64) -> Result<Vec<Candle>> {
    let ims = interval_ms(interval).with_context(|| format!("bad interval {interval}"))?;
    let mut out: Vec<Candle> = Vec::new();
    let mut from = start_ms;
    for _ in 0..40 {
        let v = hl_info(http, api, serde_json::json!({"type": "candleSnapshot", "req": {"coin": coin, "interval": interval, "startTime": from, "endTime": end_ms}})).await?;
        let arr = v.as_array().context("candles: not an array")?;
        let n = arr.len();
        for c in arr {
            let t = c["t"].as_i64().context("candle t")?;
            if t + ims <= end_ms {
                out.push(Candle { open_time: t, open: num(&c["o"]), high: num(&c["h"]), low: num(&c["l"]), close: num(&c["c"]), volume: num(&c["v"]), close_time: t + ims - 1 });
            }
        }
        let Some(last) = out.last().map(|c| c.open_time) else { break };
        if n < 500 || last + ims >= end_ms || last + 1 <= from {
            break;
        }
        from = last + 1;
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    }
    out.sort_by_key(|c| c.open_time);
    out.dedup_by_key(|c| c.open_time);
    Ok(out)
}

/// Hyperliquid hourly funding prints since `start_ms` (500 per call).
pub async fn hl_funding(http: &reqwest::Client, api: &str, coin: &str, start_ms: i64) -> Result<Vec<(i64, f64)>> {
    let mut out: Vec<(i64, f64)> = Vec::new();
    let mut from = start_ms;
    for _ in 0..200 {
        let v = hl_info(http, api, serde_json::json!({"type": "fundingHistory", "coin": coin, "startTime": from})).await?;
        let arr = v.as_array().context("fundingHistory: not an array")?;
        for r in arr {
            if let Some(t) = r["time"].as_i64() {
                out.push((t, num(&r["fundingRate"])));
            }
        }
        match out.last() {
            Some((t, _)) if arr.len() >= 500 && *t + 1 > from => from = *t + 1,
            _ => break,
        }
        tokio::time::sleep(std::time::Duration::from_secs(4)).await; // 45 weight a page of an IP-wide 1200 a minute the live bots share
    }
    Ok(out)
}

/// Appends each coin's newest closed Hyperliquid candles (the last `bars` of them):
/// what the live engine calls right after a bar closes. Returns how many coins now
/// have the bar opening at `want_open_ms`.
pub async fn hl_recent(http: &reqwest::Client, ar: &Archive, api: &str, coins: &[String], interval: &str, bars: i64, want_open_ms: i64) -> usize {
    let Some(ims) = interval_ms(interval) else { return 0 };
    let now = chrono::Utc::now().timestamp_millis();
    let mut have = 0;
    for coin in coins {
        let path = ar.candle_path(HL, interval, coin);
        if last_time(&path).is_some_and(|t| t >= want_open_ms) {
            have += 1;
            continue;
        }
        // Never leave a hole: start from the bar after the last one stored.
        let from = last_time(&path).map(|t| t + ims).unwrap_or(now - bars * ims).max(now - 4990 * ims);
        if (now - from) / ims > 100 {
            // Catching up after downtime is a heavy call: stay well inside the rate limit the other bots share.
            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        }
        match hl_candles(http, api, coin, interval, from, now).await {
            Ok(c) => {
                if ar.append_candles(HL, interval, coin, &c).is_ok() && c.iter().any(|k| k.open_time >= want_open_ms) {
                    have += 1;
                }
            }
            Err(e) => tracing::warn!("{coin}: {interval} candles failed: {e:#}"),
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    have
}

/// What `lab fetch` should bring up to date.
pub struct FetchPlan<'a> {
    pub venue: &'a str,
    pub intervals: &'a [String],
    /// (archive name = Hyperliquid coin, venue symbol).
    pub coins: Vec<(String, String)>,
    pub start_ms: i64,
    pub hl_api: &'a str,
}

/// Brings the archive up to now for one venue. Returns one line per coin.
pub async fn fetch(http: &reqwest::Client, ar: &Archive, plan: &FetchPlan<'_>) -> Result<Vec<String>> {
    let now = chrono::Utc::now().timestamp_millis();
    let mut lines = Vec::new();
    for (coin, symbol) in &plan.coins {
        let mut parts = Vec::new();
        for interval in plan.intervals {
            let ims = interval_ms(interval).with_context(|| format!("bad interval {interval}"))?;
            let from = last_time(&ar.candle_path(plan.venue, interval, coin)).map(|t| t + ims).unwrap_or(plan.start_ms);
            let got = match plan.venue {
                BINANCE => binance_klines(http, symbol, interval, from, now).await,
                _ => {
                    // Hyperliquid serves at most its latest 5000 candles.
                    let from = from.max(now - 4990 * ims);
                    // A full pull weighs ~100 of the 1200 a minute this IP shares with the running bots.
                    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
                    hl_candles(http, plan.hl_api, symbol, interval, from, now).await
                }
            };
            match got {
                Ok(c) => parts.push(format!("{interval} +{}", ar.append_candles(plan.venue, interval, coin, &c)?)),
                Err(e) => parts.push(format!("{interval} FAILED ({e:#})")),
            }
        }
        // Funding only matters where there are candles: on Hyperliquid that is the
        // reach of its coarsest interval.
        let reach = plan.intervals.iter().filter_map(|i| interval_ms(i)).max().unwrap_or(3_600_000);
        let fstart = if plan.venue == BINANCE { plan.start_ms } else { plan.start_ms.max(now - 4990 * reach) };
        let ffrom = last_time(&ar.funding_path(plan.venue, coin)).map(|t| t + 1).unwrap_or(fstart);
        let fund = match plan.venue {
            BINANCE => binance_funding(http, symbol, ffrom).await,
            _ => hl_funding(http, plan.hl_api, symbol, ffrom).await,
        };
        match fund {
            Ok(r) => parts.push(format!("funding +{}", ar.append_funding(plan.venue, coin, &r)?)),
            Err(e) => parts.push(format!("funding FAILED ({e:#})")),
        }
        let line = format!("{:<12} {:<8} {}", plan.venue, coin, parts.join(" · "));
        eprintln!("{line}");
        lines.push(line);
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(t: i64, px: f64) -> Candle {
        Candle { open_time: t, open: px, high: px * 1.01, low: px * 0.99, close: px, volume: 10.0, close_time: t + 899_999 }
    }

    #[test]
    fn archive_appends_only_newer_valid_rows() {
        let dir = std::env::temp_dir().join(format!("jev-lab-archive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ar = Archive::new(&dir);
        assert!(ar.candles(HL, "15m", "ETH").unwrap().is_empty());
        assert_eq!(ar.append_candles(HL, "15m", "ETH", &[c(0, 100.0), c(900_000, 101.0)]).unwrap(), 2);
        // Overlap, a duplicate, and a broken candle: only the new valid one lands.
        let mut bad = c(2_700_000, 103.0);
        bad.low = f64::NAN;
        assert_eq!(ar.append_candles(HL, "15m", "ETH", &[c(900_000, 101.0), c(1_800_000, 102.0), c(1_800_000, 102.0), bad]).unwrap(), 1);
        let got = ar.candles(HL, "15m", "ETH").unwrap();
        assert_eq!(got.iter().map(|k| k.open_time).collect::<Vec<_>>(), vec![0, 900_000, 1_800_000]);
        assert_eq!(got[2].close, 102.0);
        assert_eq!(got[2].close_time, 1_800_000 + 899_999);
        assert_eq!(ar.append_funding(HL, "ETH", &[(10, 0.0000125), (20, -0.00002), (20, 0.5)]).unwrap(), 2);
        assert_eq!(ar.append_funding(HL, "ETH", &[(20, 0.1), (30, 0.00001)]).unwrap(), 1);
        assert_eq!(ar.funding(HL, "ETH"), vec![(10, 0.0000125), (20, -0.00002), (30, 0.00001)]);
        std::fs::remove_dir_all(dir).ok();
    }
}
