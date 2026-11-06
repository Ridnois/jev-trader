//! 4.6: hypothesis register. Every strategy change is written down in
//! `hypotheses.toml` before it is tested; `jev-trader report` lists them.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hypothesis {
    pub id: String,
    pub statement: String,
    pub change: String,
    pub metric: String,
    pub baseline: String,
    pub pass_if: String,
    pub invalidate_if: String,
    /// proposed | testing | accepted | rejected
    pub status: String,
    #[serde(default)]
    pub result: String,
}

#[derive(Debug, Default, Deserialize)]
struct File {
    #[serde(default)]
    hypothesis: Vec<Hypothesis>,
}

pub fn load(path: &Path) -> Result<Vec<Hypothesis>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let f: File = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    for h in &f.hypothesis {
        anyhow::ensure!(
            matches!(h.status.as_str(), "proposed" | "testing" | "accepted" | "rejected"),
            "{}: unknown status '{}'",
            h.id,
            h.status
        );
    }
    Ok(f.hypothesis)
}

#[cfg(test)]
mod tests {
    #[test]
    fn repo_register_parses() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("hypotheses.toml");
        let h = super::load(&p).unwrap();
        assert!(!h.is_empty());
        let mut ids: Vec<&str> = h.iter().map(|x| x.id.as_str()).collect();
        ids.dedup();
        assert_eq!(ids.len(), h.len(), "unique ids");
    }
}
