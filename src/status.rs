//! Small file helpers shared by the engine, the report and the dashboard.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Every file the engine writes in its data_dir.
#[derive(Debug, Clone)]
pub struct Files {
    pub dir: PathBuf,
    pub ledger: PathBuf,
    pub status: PathBuf,
    pub control: PathBuf,
    /// Analyzed headlines (one per Jev answer set).
    pub news: PathBuf,
    /// Rolling news memory (recent events, seen ids) for restarts.
    pub news_state: PathBuf,
    /// Every Jev call: purpose, items, tokens.
    pub jev_usage: PathBuf,
    pub candidates: PathBuf,
    pub markouts: PathBuf,
    pub pending_markouts: PathBuf,
    pub orders: PathBuf,
    pub trades: PathBuf,
    pub report: PathBuf,
    pub pid: PathBuf,
}

impl Files {
    pub fn new(dir: &Path) -> Self {
        let j = |n: &str| dir.join(n);
        Self {
            dir: dir.to_path_buf(),
            ledger: j("ledger.json"),
            status: j("status.json"),
            control: j("control.json"),
            news: j("news.jsonl"),
            news_state: j("news_state.json"),
            jev_usage: j("jev_usage.jsonl"),
            candidates: j("candidates.jsonl"),
            markouts: j("markouts.jsonl"),
            pending_markouts: j("markouts_pending.json"),
            orders: j("orders.jsonl"),
            trades: j("trades.jsonl"),
            report: j("report.md"),
            pid: j("engine.pid"),
        }
    }
}

/// One step of a gate chain: which check, whether it passed, the reading behind it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Gate {
    pub gate: String,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

impl Gate {
    pub fn new(gate: &str, passed: bool, detail: impl Into<String>) -> Self {
        Self { gate: gate.into(), passed, detail: detail.into() }
    }
}

pub fn first_block(gates: &[Gate]) -> Option<&str> {
    gates.iter().find(|g| !g.passed).map(|g| g.gate.as_str())
}

/// Written by the dashboard, read by the engine (pause entries, flatten all).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Control {
    pub paused: bool,
    pub flatten_requested_at: Option<DateTime<Utc>>,
}

impl Control {
    pub fn load(path: &Path) -> Self {
        read_json(path).unwrap_or_default()
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

pub fn append_jsonl<T: Serialize>(path: &Path, value: &T) {
    let Ok(line) = serde_json::to_string(value) else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Every record of a JSONL file (unparseable lines skipped).
pub fn read_jsonl<T: DeserializeOwned>(path: &Path) -> Vec<T> {
    std::fs::read_to_string(path).map(|s| s.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
}

/// Up to `max_items` records from the last `max_bytes` of a JSONL file.
pub fn tail_jsonl<T: DeserializeOwned>(path: &Path, max_bytes: u64, max_items: usize) -> Vec<T> {
    let Ok(mut f) = std::fs::File::open(path) else { return Vec::new() };
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
    lines.into_iter().skip(skip).filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// One process per data folder. A stale PID file (process gone) is taken over.
pub fn acquire_pid_lock(path: &Path) -> Result<()> {
    if let Ok(text) = std::fs::read_to_string(path)
        && let Ok(pid) = text.trim().parse::<u32>()
        && pid != std::process::id()
        && std::fs::read_to_string(format!("/proc/{pid}/cmdline")).is_ok_and(|c| c.contains("jev-trader"))
    {
        anyhow::bail!("another jev-trader (PID {pid}) is already using {}; stop it first", path.parent().unwrap_or(path).display());
    }
    write_atomic(path, std::process::id().to_string().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tails_jsonl() {
        let dir = std::env::temp_dir().join(format!("jev-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.jsonl");
        for i in 0..100 {
            append_jsonl(&p, &serde_json::json!({ "n": i }));
        }
        #[derive(Deserialize)]
        struct R {
            n: u32,
        }
        let all: Vec<R> = tail_jsonl(&p, 1 << 20, 5);
        assert_eq!(all.iter().map(|r| r.n).collect::<Vec<_>>(), vec![95, 96, 97, 98, 99]);
        assert_eq!(read_jsonl::<R>(&p).len(), 100);
        std::fs::remove_dir_all(dir).ok();
    }
}
