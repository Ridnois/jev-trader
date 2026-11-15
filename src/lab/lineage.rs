//! The lab's memory of what it proposed and what became of it:
//! `lab-data/lineage.jsonl`, append-only, one row per event.
//!
//! Events: `proposed` (a family file was written), `tested` (the gate judged it),
//! `promoted` (`lab promote` handed it to the trader), `killed` / `graduated` (the
//! paper monitor's call). The learner reads it to stop deriving from branches that
//! keep dying; the proposer reads it never to propose the same rule and grid twice.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Event {
    #[serde(default)]
    pub at: Option<chrono::DateTime<chrono::Utc>>,
    pub event: String,
    /// The family (rule id for composed families).
    pub family: String,
    /// Readable name (`g0007`).
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub generation: u32,
    #[serde(default)]
    pub operator: String,
    #[serde(default)]
    pub lessons: Vec<String>,
    /// New specs charged to the ledger by this event.
    #[serde(default)]
    pub trials: usize,
    #[serde(default)]
    pub verdict: Option<String>,
    #[serde(default)]
    pub study: String,
    /// Identity of the proposal (rule id + grid), so it is never proposed twice.
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub note: String,
}

pub fn load(path: &Path) -> Vec<Event> {
    crate::status::read_jsonl(path)
}

pub fn append(path: &Path, e: &Event) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    crate::bot::append_jsonl(path, e);
}

/// The next generation number.
pub fn next_generation(events: &[Event]) -> u32 {
    events.iter().map(|e| e.generation).max().unwrap_or(0) + 1
}

/// The next free `gNNNN` name.
pub fn next_name(events: &[Event], taken: usize) -> String {
    let max = events.iter().filter_map(|e| e.name.strip_prefix('g').and_then(|n| n.parse::<u32>().ok())).max().unwrap_or(0);
    format!("g{:04}", max as usize + 1 + taken)
}

/// `lab lineage`: each root family with its descendants, newest state of each.
pub fn tree(events: &[Event]) -> String {
    let mut s = String::new();
    let mut last: BTreeMap<&str, &Event> = BTreeMap::new();
    for e in events {
        last.insert(e.family.as_str(), e);
    }
    let proposed: Vec<&Event> = events.iter().filter(|e| e.event == "proposed").collect();
    if proposed.is_empty() {
        return "No generations yet: `lab evolve --dry-run` shows what the first would propose.\n".into();
    }
    let mut children: BTreeMap<String, Vec<&Event>> = BTreeMap::new();
    for p in &proposed {
        children.entry(p.parent.clone().unwrap_or_default()).or_default().push(p);
    }
    let known: std::collections::HashSet<&str> = proposed.iter().map(|p| p.family.as_str()).collect();
    let mut roots: Vec<String> = children.keys().filter(|k| !known.contains(k.as_str())).cloned().collect();
    roots.sort();
    let mut charged: BTreeMap<&str, usize> = BTreeMap::new();
    for e in events {
        *charged.entry(e.family.as_str()).or_default() += e.trials;
    }
    fn walk(s: &mut String, node: &str, depth: usize, children: &BTreeMap<String, Vec<&Event>>, last: &BTreeMap<&str, &Event>, charged: &BTreeMap<&str, usize>) {
        for c in children.get(node).into_iter().flatten() {
            let state = last.get(c.family.as_str()).map(|e| match (&e.event[..], &e.verdict) {
                ("tested" | "retested", Some(v)) => v.clone(),
                (ev, _) => ev.to_uppercase(),
            }).unwrap_or_default();
            let _ = writeln!(s, "{}└ {} `{}` gen {} · {} · {} · {} trials", "  ".repeat(depth), c.name, c.family, c.generation, c.operator, state, charged.get(c.family.as_str()).copied().unwrap_or(0));
            if depth < 32 {
                walk(s, &c.family, depth + 1, children, last, charged);
            }
        }
    }
    for r in roots {
        let _ = writeln!(s, "{}", if r.is_empty() { "(no parent)" } else { &r });
        walk(&mut s, &r, 1, &children, &last, &charged);
    }
    let trials: usize = events.iter().map(|e| e.trials).sum();
    let _ = writeln!(s, "\n{} proposals over {} generations, {trials} trials charged by the loop.", proposed.len(), next_generation(events) - 1);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_generations_and_tree() {
        let e = |event: &str, family: &str, name: &str, parent: &str, generation: u32| Event { event: event.into(), family: family.into(), name: name.into(), parent: Some(parent.into()), generation, operator: "condition".into(), trials: 4, ..Event::default() };
        let ev = vec![e("proposed", "c1", "g0001", "trend", 1), e("proposed", "c2", "g0002", "c1", 2), Event { verdict: Some("REJECT".into()), ..e("tested", "c2", "g0002", "c1", 2) }];
        assert_eq!(next_generation(&ev), 3);
        assert_eq!(next_name(&ev, 0), "g0003");
        assert_eq!(next_name(&[], 1), "g0002");
        let t = tree(&ev);
        assert!(t.starts_with("trend\n"), "{t}");
        assert!(t.contains("    └ g0002 `c2` gen 2 · condition · REJECT · 8 trials"), "{t}");
        assert!(tree(&[]).contains("No generations"));
    }
}
