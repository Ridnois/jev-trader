//! Files shared between the bot loop and the dashboard: telemetry the bot writes,
//! and the control file the dashboard writes.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::paper::Binding;
use crate::signals::{NewsSignal, Target};

pub struct Paths {
    pub state: PathBuf,
    pub status: PathBuf,
    pub control: PathBuf,
    pub events: PathBuf,
    pub ticks: PathBuf,
    pub trades: PathBuf,
    pub equity: PathBuf,
    pub pipeline: PathBuf,
    pub settlements: PathBuf,
    pub settle_pending: PathBuf,
    pub backlog: PathBuf,
    pub regime: PathBuf,
    pub regime_cache: PathBuf,
    pub dominance: PathBuf,
    pub spiked: PathBuf,
    pub reviews: PathBuf,
    pub report: PathBuf,
    pub backtests: PathBuf,
    pub macro_calendar: PathBuf,
}

impl Paths {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            state: data_dir.join("state.json"),
            status: data_dir.join("status.json"),
            control: data_dir.join("control.json"),
            events: data_dir.join("events.jsonl"),
            ticks: data_dir.join("ticks.jsonl"),
            trades: data_dir.join("trades.jsonl"),
            equity: data_dir.join("equity.jsonl"),
            pipeline: data_dir.join("pipeline.jsonl"),
            settlements: data_dir.join("settlements.jsonl"),
            settle_pending: data_dir.join("settle_pending.json"),
            backlog: data_dir.join("backlog.jsonl"),
            regime: data_dir.join("regime.json"),
            regime_cache: data_dir.join("regime_cache.json"),
            dominance: data_dir.join("dominance.json"),
            spiked: data_dir.join("spiked.jsonl"),
            reviews: data_dir.join("reviews.jsonl"),
            report: data_dir.join("report.md"),
            backtests: data_dir.join("backtests"),
            macro_calendar: data_dir.join("macro_calendar.toml"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StageState {
    Ok,
    Degraded,
    Error,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stage {
    pub name: String,
    pub state: StageState,
    pub detail: String,
    pub ms: u64,
}

/// Written by the bot after every tick (and at start-up).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub last_tick_at: Option<DateTime<Utc>>,
    pub next_tick_at: Option<DateTime<Utc>>,
    pub tick: u64,
    pub interval_secs: u64,
    pub paused: bool,
    pub halted: bool,
    pub stages: Vec<Stage>,
    /// 3.5: one-line market posture.
    #[serde(default)]
    pub posture: Option<Posture>,
    /// Assets currently in REVIEW (no entries) and why.
    #[serde(default)]
    pub review: std::collections::BTreeMap<String, String>,
    /// 6.2: Jev failures over the breaker window (failed, total).
    #[serde(default)]
    pub jev_window: (u32, u32),
    /// 6.1: headlines waiting for a Jev retry, and spiked in total.
    #[serde(default)]
    pub jev_pending: usize,
    /// 6.4: token use today.
    #[serde(default)]
    pub tokens_today: (u64, u64, u64),
    /// Orders go to Binance (real money).
    #[serde(default)]
    pub live: bool,
    /// ... on Binance's spot testnet (fake money).
    #[serde(default)]
    pub testnet: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PostureKind {
    EntriesAllowed,
    ExitsOnly,
    Review,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Posture {
    pub kind: PostureKind,
    /// Effective gross exposure ceiling.
    pub ceiling: f64,
    pub reason: String,
    pub until: Option<DateTime<Utc>>,
}

impl Posture {
    pub fn line(&self) -> String {
        let head = match self.kind {
            PostureKind::EntriesAllowed => format!("ENTRIES ALLOWED · ceiling {:.0}%", self.ceiling * 100.0),
            PostureKind::ExitsOnly => "EXITS ONLY".to_string(),
            PostureKind::Review => "REVIEW".to_string(),
        };
        let mut s = head;
        if !self.reason.is_empty() {
            s.push_str(" · ");
            s.push_str(&self.reason);
        }
        if let Some(u) = self.until {
            s.push_str(&format!(" until {}", u.format("%a %H:%M UTC")));
        }
        s
    }
}

/// One step of the gate chain (0.6).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateResult {
    pub gate: String,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

impl GateResult {
    pub fn new(gate: &str, passed: bool, detail: impl Into<String>) -> Self {
        Self { gate: gate.into(), passed, detail: detail.into() }
    }
}

/// One line of pipeline.jsonl: compact per-tick stage history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRecord {
    pub ts: DateTime<Utc>,
    pub tick: u64,
    pub stages: Vec<Stage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickRecord {
    pub ts: DateTime<Utc>,
    pub asset: String,
    pub price: f64,
    /// `None` = not enough closed candles (REVIEW, never treated as 0).
    pub momentum: Option<f64>,
    pub news: NewsSignal,
    pub target: Target,
    pub final_weight: f64,
    pub halted: bool,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub drivers: Vec<String>,
    /// Why this asset could not take new entries (missing/stale/invalid input).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gates: Vec<GateResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<Binding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub atr_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regime_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regime_zone: Option<String>,
    /// Phase 7 recorded columns (no effect on trading).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_interest: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macro_event: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquityRecord {
    pub ts: DateTime<Utc>,
    pub equity: f64,
    pub cash: f64,
}

/// Written by the dashboard, read by the bot. The bot never writes this file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Control {
    pub paused: bool,
    pub flatten_requested_at: Option<DateTime<Utc>>,
}

impl Control {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, &serde_json::to_vec_pretty(self)?)
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Every record of a JSONL file (unparseable lines skipped).
pub fn read_jsonl<T: DeserializeOwned>(path: &Path) -> Vec<T> {
    std::fs::read_to_string(path)
        .map(|s| s.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
        .unwrap_or_default()
}

/// events.jsonl deduplicated by id, first occurrence wins (6.3 crash consistency).
pub fn load_events(path: &Path) -> Vec<crate::news::NewsEvent> {
    let mut seen = std::collections::HashSet::new();
    read_jsonl::<crate::news::NewsEvent>(path).into_iter().filter(|e| seen.insert(e.id.clone())).collect()
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Parse up to `max_items` records from the last `max_bytes` of a JSONL file.
pub fn tail_jsonl<T: DeserializeOwned>(path: &Path, max_bytes: u64, max_items: usize) -> Vec<T> {
    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(max_bytes);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut buf = String::new();
    if f.read_to_string(&mut buf).is_err() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = buf.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // partial line
    }
    let skip = lines.len().saturating_sub(max_items);
    lines
        .into_iter()
        .skip(skip)
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tails_jsonl() {
        let dir = std::env::temp_dir().join(format!("jev-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.jsonl");
        let body: String = (0..100).map(|i| format!("{{\"n\":{i}}}\n")).collect();
        std::fs::write(&p, body).unwrap();
        #[derive(Deserialize)]
        struct R {
            n: u32,
        }
        let all: Vec<R> = tail_jsonl(&p, 1 << 20, 5);
        assert_eq!(all.iter().map(|r| r.n).collect::<Vec<_>>(), vec![95, 96, 97, 98, 99]);
        let partial: Vec<R> = tail_jsonl(&p, 30, 100);
        assert!(!partial.is_empty() && partial.last().unwrap().n == 99);
        std::fs::remove_dir_all(dir).ok();
    }
}

/// One process per data folder: refuses to start while another live process holds
/// `path`. The lock is a PID file; a stale one (process gone) is taken over.
pub fn acquire_pid_lock(path: &Path) -> anyhow::Result<()> {
    if let Ok(text) = std::fs::read_to_string(path)
        && let Ok(pid) = text.trim().parse::<u32>()
        && pid != std::process::id()
        && std::fs::read_to_string(format!("/proc/{pid}/cmdline")).is_ok_and(|c| c.contains("jev-trader"))
    {
        anyhow::bail!("another jev-trader (PID {pid}) is already using {}; stop it first", path.parent().unwrap_or(path).display());
    }
    write_atomic(path, std::process::id().to_string().as_bytes())
}
