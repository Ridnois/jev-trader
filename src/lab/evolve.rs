//! `lab evolve`: learn → propose → compare → report, once, on demand.
//!
//! - `lab learn`    lessons from every verdict, the ledger, the paper monitor and the lineage
//! - `lab propose`  new family files under `families/gen/`, each with a hypothesis entry
//! - `lab compare`  a generation through the gate (charged first, judged together) and
//!                  against the incumbents
//! - `lab evolve`   all three, and one report: `lab-data/reports/evolve/gNNN.{json,md}`
//! - `lab lineage`  the tree of what was proposed and what became of it
//!
//! Nothing here promotes. `lab promote` stays a separate, deliberate command.
//!
//! The same loop runs in memory on synthetic markets (`lab calibrate --evolve`): that
//! is the acceptance test for the whole design. A loop that reads results and proposes
//! again is a search, and on markets with nothing to find it must promote nothing.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use super::compare::{self, Books, Judged, Pairing};
use super::data::HL;
use super::gate::{self, Evaluation, Verdict};
use super::learn::{self, Lessons};
use super::lineage::{self, Event};
use super::panel::Panel;
use super::propose::{self, Parent, Proposal};
use super::strategy::{Aux, Family, Spec};
use super::{LabConfig, stat, synth};
use crate::trade::monitor;

/// A family file on disk, with where it came from.
pub struct OnDisk {
    pub path: PathBuf,
    pub family: Family,
}

/// Every family file: `families/*.toml` and the generated ones.
pub fn families(lab: &LabConfig) -> Vec<OnDisk> {
    let gen_dir = lab.evolve.families_dir.clone();
    let root = gen_dir.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("families"));
    let mut out = Vec::new();
    for dir in [root, gen_dir] {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect();
        paths.sort();
        for path in paths {
            match Family::load(&path) {
                Ok(family) => out.push(OnDisk { path, family }),
                Err(e) => tracing::warn!("{}: {e:#}", path.display()),
            }
        }
    }
    out
}

/// Every gate verdict on file: (path, evaluation JSON).
fn verdict_files(lab: &LabConfig) -> Vec<(PathBuf, serde_json::Value)> {
    let mut out = Vec::new();
    let mut studies: Vec<PathBuf> = std::fs::read_dir(lab.data_dir.join("reports")).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    studies.sort();
    for dir in studies {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        files.sort();
        for f in files {
            if let Some(v) = crate::status::read_json::<serde_json::Value>(&f)
                && v["verdict"].is_string()
                && v["family"].is_string()
            {
                out.push((f, v));
            }
        }
    }
    out
}

fn mean_bps(eval: &serde_json::Value) -> Option<f64> {
    let d: Vec<f64> = serde_json::from_value(eval.get("evidence")?.get("daily")?.clone()).ok()?;
    (!d.is_empty()).then(|| stat::mean(&d) * 1e4)
}

/// Parents the proposer may derive from: families with a verdict, a pick, and no Jev
/// probe (their replay cache is not the loop's to spend).
fn parents_of(verdicts: &[(String, serde_json::Value)], fams: &HashMap<String, Family>) -> BTreeMap<String, Parent> {
    let mut out = BTreeMap::new();
    for (_, v) in verdicts {
        let name = v["family"].as_str().unwrap_or("");
        let Some(fam) = fams.get(name) else { continue };
        if fam.probe.is_some() {
            continue;
        }
        let pick: Option<Spec> = serde_json::from_value(v["pick"].clone()).ok();
        out.insert(name.to_string(), Parent { family: fam.clone(), pick, mean_bps: mean_bps(v), study: v["study"].as_str().unwrap_or("").into() });
    }
    out
}

/// What the learner and the proposer read, from files.
pub struct Gathered {
    pub verdicts: Vec<(String, serde_json::Value)>,
    pub families: HashMap<String, Family>,
    pub monitor: Option<monitor::Report>,
    pub lineage: Vec<Event>,
}

pub fn gather(lab: &LabConfig) -> Gathered {
    let families: HashMap<String, Family> = families(lab).into_iter().map(|d| (d.family.family.clone(), d.family)).collect();
    let verdicts = verdict_files(lab).into_iter().map(|(p, v)| (p.display().to_string(), v)).collect();
    let monitor = crate::status::read_json::<monitor::Report>(&lab.trade.data_dir.join("monitor.json"));
    Gathered { verdicts, families, monitor, lineage: lineage::load(&lab.lineage_path()) }
}

fn label_of(fams: &HashMap<String, Family>, name: &str) -> String {
    fams.get(name).map(|f| f.label().to_string()).unwrap_or_else(|| name.to_string())
}

pub fn learn_from(g: &Gathered, ledger: &[super::search::TrialRecord], lab: &LabConfig) -> Lessons {
    let verdicts = g.verdicts.iter().map(|(src, v)| learn::Verdict { source: src.clone(), eval: v, label: label_of(&g.families, v["family"].as_str().unwrap_or("")) }).collect();
    learn::learn(&learn::Inputs { verdicts, ledger, monitor: g.monitor.as_ref(), lineage: &g.lineage }, &lab.evolve)
}

/// `lab learn`: writes `lessons/<date>.{json,md}` and returns the lessons and the path.
pub fn learn_files(lab: &LabConfig) -> Result<(Lessons, PathBuf)> {
    let g = gather(lab);
    let lessons = learn_from(&g, &Books::files(lab).rows(), lab);
    let dir = lab.lessons_dir();
    std::fs::create_dir_all(&dir)?;
    // One file per generation: proposals cite lesson ids, so a file must never be rewritten.
    let path = dir.join(format!("{}-g{:03}.json", chrono::Utc::now().format("%Y-%m-%d"), lineage::next_generation(&g.lineage)));
    crate::status::write_atomic(&path, &serde_json::to_vec_pretty(&lessons)?)?;
    crate::status::write_atomic(&path.with_extension("md"), learn::markdown(&lessons).as_bytes())?;
    Ok((lessons, path))
}

fn latest_lessons(lab: &LabConfig) -> Option<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(lab.lessons_dir()).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    v.sort();
    v.pop()
}

pub fn read_lessons(path: &Path) -> Result<Lessons> {
    serde_json::from_slice(&std::fs::read(path).with_context(|| format!("reading {}", path.display()))?).with_context(|| format!("parsing {}", path.display()))
}

/// The proposer's view, from files.
pub fn propose_from(g: &Gathered, lessons: &Lessons, books: &Books, lab: &LabConfig, cfg: &super::EvolveCfg, loop_budget: Option<usize>) -> propose::Outcome {
    let parents = parents_of(&g.verdicts, &g.families);
    let charged = books.charged();
    let generation = lineage::next_generation(&g.lineage);
    propose::propose(&propose::Inputs { lessons, parents: &parents, lineage: &g.lineage, charged: &charged, research_venue: &lab.research_venue, generation, seed: lab.seed, loop_budget }, cfg)
}

/// Writes proposals: family files, hypothesis entries, `proposed` events.
pub fn write_proposals(lab: &LabConfig, proposals: &[Proposal]) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(&lab.evolve.families_dir)?;
    let mut paths = Vec::new();
    for p in proposals {
        let path = lab.evolve.families_dir.join(format!("{}.toml", p.family.label()));
        if path.exists() {
            bail!("{} exists: the lineage and the files disagree; nothing was overwritten", path.display());
        }
        std::fs::write(&path, propose::family_toml(p)?)?;
        let mut hyp = std::fs::OpenOptions::new().create(true).append(true).open(&lab.evolve.hypotheses)?;
        use std::io::Write as _;
        write!(hyp, "\n{}", propose::hypothesis_toml(&p.hypothesis)?)?;
        lineage::append(&lab.lineage_path(), &Event {
            at: Some(chrono::Utc::now()),
            event: "proposed".into(),
            family: p.family.family.clone(),
            name: p.family.label().into(),
            parent: p.family.parent.clone(),
            generation: p.family.generation.unwrap_or(0),
            operator: p.family.operator.clone().unwrap_or_default(),
            lessons: p.family.lessons.clone(),
            trials: 0,
            verdict: None,
            study: p.study.clone(),
            key: p.key.clone(),
            note: p.family.reason.clone(),
        });
        paths.push(path);
    }
    Ok(paths)
}

/// What one cycle cost a study.
#[derive(Debug, Clone, Serialize)]
pub struct StudyCost {
    pub study: String,
    pub trials_before: usize,
    pub trials_after: usize,
    pub design_days: usize,
    /// Annualised Sharpe the luckiest of N no-edge rules would show over the design
    /// window, every trial counted as independent (the gate discounts correlated ones).
    pub luck_bar_before: f64,
    pub luck_bar_after: f64,
}

fn luck_bar(trials: usize, days: usize) -> f64 {
    stat::expected_max_sharpe(1.0 / (days.max(3) - 1) as f64, trials as f64) * gate::DAYS_PER_YEAR.sqrt()
}

#[derive(Debug, Clone, Serialize)]
pub struct ProposalRow {
    pub name: String,
    pub family: String,
    pub operator: String,
    pub parent: Option<String>,
    pub lessons: Vec<String>,
    pub reason: String,
    pub rule: String,
    pub specs: usize,
    pub new_trials: usize,
    pub approximate: bool,
    pub verdict: Option<String>,
    pub failed: Vec<String>,
    pub pick: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CycleReport {
    pub generation: u32,
    pub dry_run: bool,
    pub at: chrono::DateTime<chrono::Utc>,
    pub lessons_file: String,
    pub lessons_total: usize,
    pub lessons_usable: usize,
    pub studies: Vec<StudyCost>,
    pub proposals: Vec<ProposalRow>,
    pub pairings: Vec<Pairing>,
    pub forward: Vec<Pairing>,
    pub messages: Vec<String>,
    /// Families the operator might promote (PROMOTE_TO_PAPER and not a duplicate).
    pub promotable: Vec<String>,
    pub lockbox_openings: usize,
}

const WALK_NOTE: &str = "This family was generated after the design window had been looked at, so its walk-forward folds are not truly out of sample. The lockbox and paper days are the only clean evidence for it.";

fn row(p: &Proposal, j: Option<&Judged>) -> ProposalRow {
    ProposalRow {
        name: p.family.label().into(),
        family: p.family.family.clone(),
        operator: p.family.operator.clone().unwrap_or_default(),
        parent: p.family.parent.clone(),
        lessons: p.family.lessons.clone(),
        reason: p.family.reason.clone(),
        rule: p.family.rule.as_ref().map(|r| r.describe()).unwrap_or_default(),
        specs: p.specs,
        new_trials: j.map(|j| j.new_trials).unwrap_or(p.new_trials),
        approximate: p.approximate,
        verdict: j.map(|j| j.eval.verdict.word().to_string()),
        failed: j.map(|j| j.eval.checks.iter().filter(|c| !c.pass).map(|c| c.name.to_string()).collect()).unwrap_or_default(),
        pick: j.and_then(|j| j.eval.pick.as_ref().map(Spec::id)),
    }
}

/// The panels a family is judged on, from the archive (cached by interval).
fn archive_panels(lab: &LabConfig) -> impl FnMut(&Family) -> Result<(Panel, Option<Panel>)> + '_ {
    let mut cache: HashMap<String, (Panel, Option<Panel>)> = HashMap::new();
    move |fam: &Family| {
        if let Some(x) = cache.get(&fam.interval) {
            return Ok(x.clone());
        }
        let full = lab.panel(&lab.research_venue, &fam.interval, &lab.universe.coins)?;
        let venue = if lab.research_venue == HL { None } else { lab.panel(HL, &fam.interval, &lab.universe.coins).ok() };
        cache.insert(fam.interval.clone(), (full.clone(), venue.clone()));
        Ok((full, venue))
    }
}

/// Writes a judged family's verdict where `lab validate` would.
fn write_verdict(lab: &LabConfig, ev: &Evaluation) -> Result<()> {
    let dir = lab.report_dir(&ev.study);
    std::fs::create_dir_all(&dir)?;
    crate::status::write_atomic(&dir.join(format!("{}.md", ev.family)), gate::markdown(ev).as_bytes())?;
    crate::status::write_atomic(&dir.join(format!("{}.json", ev.family)), &serde_json::to_vec_pretty(ev)?)?;
    Ok(())
}

/// Incumbents on a study: strategies on paper, and every other verdict's pick.
fn incumbents(lab: &LabConfig, study: &str, exclude: &[String]) -> Vec<(String, Spec)> {
    let mut out: Vec<(String, Spec)> = Vec::new();
    for s in crate::trade::load_strategies(&lab.trade.strategies_dir).unwrap_or_default() {
        if s.promoted.card.study == study {
            out.push((format!("{} (paper)", s.name), s.promoted.card.spec.clone()));
        }
    }
    let fams: HashMap<String, Family> = families(lab).into_iter().map(|d| (d.family.family.clone(), d.family)).collect();
    for (_, v) in verdict_files(lab) {
        let name = v["family"].as_str().unwrap_or("");
        if v["study"] != study || exclude.iter().any(|e| e == name) || fams.get(name).is_some_and(|f| f.probe.is_some()) {
            continue;
        }
        if let Ok(spec) = serde_json::from_value::<Spec>(v["pick"].clone()) {
            out.push((label_of(&fams, name), spec));
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|(_, s)| seen.insert(s.id()));
    out
}

/// `lab compare`: judges the proposals of a generation already written to disk.
pub fn compare_files(lab: &LabConfig, generation: u32, report: &mut CycleReport) -> Result<Vec<Judged>> {
    let on_disk: Vec<Family> = families(lab).into_iter().map(|d| d.family).filter(|f| f.generation == Some(generation) && f.rule.is_some()).collect();
    if on_disk.is_empty() {
        bail!("no generated families of generation {generation} in {}", lab.evolve.families_dir.display());
    }
    let mut books = Books::files(lab);
    let before: BTreeMap<String, usize> = on_disk.iter().map(|f| format!("{}-{}", lab.research_venue, f.interval)).map(|s| (s.clone(), books.total(&s))).collect();
    let locks_before = books.lockbox_openings();
    let mut src = archive_panels(lab);
    let earlier = lineage::load(&lab.lineage_path());
    let judged_before = |study: &str| earlier.iter().filter(|e| e.event == "tested" && e.study == study).count();
    let mut judged = compare::judge_generation(lab, &on_disk, &mut src, &judged_before, &mut books)?;
    for j in &mut judged {
        j.eval.notes.push(WALK_NOTE.into());
        write_verdict(lab, &j.eval)?;
        let fam = on_disk.iter().find(|f| f.family == j.family).expect("judged family");
        lineage::append(&lab.lineage_path(), &Event {
            at: Some(chrono::Utc::now()),
            event: "tested".into(),
            family: j.family.clone(),
            name: j.label.clone(),
            parent: fam.parent.clone(),
            generation,
            operator: fam.operator.clone().unwrap_or_default(),
            lessons: fam.lessons.clone(),
            trials: j.new_trials,
            verdict: Some(j.eval.verdict.word().into()),
            study: j.study.clone(),
            key: String::new(),
            note: j.eval.checks.iter().filter(|c| !c.pass).map(|c| c.name).collect::<Vec<_>>().join(", "),
        });
    }
    // Costs, per study.
    for (study, b) in before {
        let days = judged.iter().find(|j| j.study == study).map(|j| j.eval.design_days).unwrap_or(0);
        let after = books.total(&study);
        report.studies.push(StudyCost { study, trials_before: b, trials_after: after, design_days: days, luck_bar_before: luck_bar(b, days), luck_bar_after: luck_bar(after, days) });
    }
    report.lockbox_openings = books.lockbox_openings() - locks_before;
    // Against the incumbents, on each study's design window.
    let names: Vec<String> = judged.iter().map(|j| j.family.clone()).collect();
    for fam in &on_disk {
        let Some(j) = judged.iter().find(|j| j.family == fam.family) else { continue };
        let Some(pick) = j.eval.pick.clone() else { continue };
        let (full, _) = src(fam)?;
        let design = full.head(super::design_bars(&full, lab));
        let inc = incumbents(lab, &j.study, &names);
        report.pairings.extend(compare::versus_incumbents(lab, &design, &[(j.label.clone(), pick)], &inc));
    }
    // Parent and child on paper, where both are.
    let strategies = crate::trade::load_strategies(&lab.trade.strategies_dir).unwrap_or_default();
    let daily: Vec<crate::trade::DailyRow> = crate::status::read_jsonl(&lab.trade.data_dir.join("daily.jsonl"));
    report.forward = compare::forward_pairs(&daily, &strategies, lab);
    report.promotable = judged.iter().filter(|j| j.eval.verdict == Verdict::PromoteToPaper && !report.pairings.iter().any(|p| p.challenger == j.label && p.duplicate)).map(|j| j.label.clone()).collect();
    Ok(judged)
}

pub struct EvolveOpts {
    pub dry_run: bool,
    pub max_trials: Option<usize>,
    pub from_lessons: Option<PathBuf>,
    /// Auto mode's cap on the loop's trials per study (None for a single cycle).
    pub loop_budget: Option<usize>,
}

/// `lab evolve`.
pub fn evolve(lab: &LabConfig, opts: &EvolveOpts) -> Result<CycleReport> {
    let (lessons, lessons_path) = match &opts.from_lessons {
        Some(p) => (read_lessons(p)?, p.clone()),
        None => learn_files(lab)?,
    };
    let mut cfg = lab.evolve.clone();
    if let Some(m) = opts.max_trials {
        cfg.trials_per_cycle = m;
    }
    let g = gather(lab);
    let books = Books::files(lab);
    let out = propose_from(&g, &lessons, &books, lab, &cfg, opts.loop_budget);
    let generation = lineage::next_generation(&g.lineage);
    let mut report = CycleReport {
        generation,
        dry_run: opts.dry_run,
        at: chrono::Utc::now(),
        lessons_file: lessons_path.display().to_string(),
        lessons_total: lessons.lessons.len(),
        lessons_usable: lessons.lessons.iter().filter(|l| !l.anecdote && !l.operator_only).count(),
        studies: Vec::new(),
        proposals: Vec::new(),
        pairings: Vec::new(),
        forward: Vec::new(),
        messages: out.messages.clone(),
        promotable: Vec::new(),
        lockbox_openings: 0,
    };
    if opts.dry_run || out.proposals.is_empty() {
        // What it would cost, from the ledger as it stands.
        let mut studies: Vec<String> = out.proposals.iter().map(|p| p.study.clone()).collect();
        studies.sort();
        studies.dedup();
        for s in studies {
            let b = books.total(&s);
            let new: usize = out.proposals.iter().filter(|p| p.study == s).map(|p| p.new_trials).sum();
            let days = g.verdicts.iter().find(|(_, v)| v["study"] == s.as_str()).and_then(|(_, v)| v["design_days"].as_u64()).unwrap_or(0) as usize;
            report.studies.push(StudyCost { study: s, trials_before: b, trials_after: b + new, design_days: days, luck_bar_before: luck_bar(b, days), luck_bar_after: luck_bar(b + new, days) });
        }
        report.proposals = out.proposals.iter().map(|p| row(p, None)).collect();
        if opts.dry_run {
            // Show the would-be files without writing them where the lab reads families.
            let dir = lab.data_dir.join("reports").join("evolve").join(format!("dry-g{generation:03}"));
            std::fs::create_dir_all(&dir)?;
            for p in &out.proposals {
                std::fs::write(dir.join(format!("{}.toml", p.family.label())), propose::family_toml(p)?)?;
            }
        }
    } else {
        write_proposals(lab, &out.proposals)?;
        let judged = compare_files(lab, generation, &mut report)?;
        report.proposals = out.proposals.iter().map(|p| row(p, judged.iter().find(|j| j.family == p.family.family))).collect();
    }
    write_report(lab, &report)?;
    Ok(report)
}

pub fn write_report(lab: &LabConfig, r: &CycleReport) -> Result<PathBuf> {
    let dir = lab.data_dir.join("reports").join("evolve");
    std::fs::create_dir_all(&dir)?;
    let stem = format!("{}g{:03}", if r.dry_run { "dry-" } else { "" }, r.generation);
    crate::status::write_atomic(&dir.join(format!("{stem}.json")), &serde_json::to_vec_pretty(r)?)?;
    let path = dir.join(format!("{stem}.md"));
    crate::status::write_atomic(&path, markdown(r).as_bytes())?;
    Ok(path)
}

pub fn markdown(r: &CycleReport) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# Generation {}{}\n", r.generation, if r.dry_run { " (dry run: nothing simulated, nothing charged, nothing written to families/)" } else { "" });
    let _ = writeln!(s, "*{} · lessons: `{}`*\n", r.at.format("%Y-%m-%d %H:%M UTC"), r.lessons_file);
    let _ = writeln!(s, "## Cost\n");
    if r.studies.is_empty() {
        let _ = writeln!(s, "No trials {}.\n", if r.dry_run { "would be charged" } else { "were charged" });
    } else {
        let _ = writeln!(s, "| Study | Trials before | {} | Luck bar before | after |\n|---|---|---|---|---|", if r.dry_run { "Would be after" } else { "After" });
        for c in &r.studies {
            let _ = writeln!(s, "| {} | {} | {} (+{}) | {:.2} | {:.2} |", c.study, c.trials_before, c.trials_after, c.trials_after - c.trials_before, c.luck_bar_before, c.luck_bar_after);
        }
        let _ = writeln!(s, "\nThe luck bar is the annualised Sharpe the best of that many no-edge rules would show over the design window ({} days), counting every trial as independent. Every cycle raises it: a cycle has to find something larger than the last one could have.\n", r.studies.first().map(|c| c.design_days).unwrap_or(0));
    }
    if r.lockbox_openings > 0 {
        let _ = writeln!(s, "**The lockbox was opened {} time(s) this cycle.** Each opening makes the next one harder to pass.\n", r.lockbox_openings);
    }
    let _ = writeln!(s, "## Lessons\n\n{} lessons, {} usable (the rest are anecdotes or for the operator). Read them in the lessons file.\n", r.lessons_total, r.lessons_usable);
    let _ = writeln!(s, "## Proposals\n");
    if r.proposals.is_empty() {
        let _ = writeln!(s, "None.\n");
    }
    for p in &r.proposals {
        let _ = writeln!(s, "### {} · {} of {}\n", p.name, p.operator, p.parent.as_deref().unwrap_or("nothing (exploration)"));
        let _ = writeln!(s, "- Family `{}` · {} specs, {} new trials{}", p.family, p.specs, p.new_trials, if p.approximate { " · parent's block form is approximate" } else { "" });
        let _ = writeln!(s, "- Rule: {}", p.rule);
        let _ = writeln!(s, "- Why: {}", p.reason);
        if !p.lessons.is_empty() {
            let _ = writeln!(s, "- Lessons: {}", p.lessons.join(", "));
        }
        if let Some(v) = &p.verdict {
            let _ = writeln!(s, "- **{v}**{}{}", p.pick.as_ref().map(|x| format!(" · pick `{x}`")).unwrap_or_default(), if p.failed.is_empty() { String::new() } else { format!(" · failed: {}", p.failed.join(", ")) });
        }
        let _ = writeln!(s);
    }
    if !r.pairings.is_empty() {
        let _ = writeln!(s, "## Against the incumbents (design window)\n\n| Challenger | Incumbent | Days | Challenger | Incumbent | Difference [95%] | Correlation | |\n|---|---|---|---|---|---|---|---|");
        for p in &r.pairings {
            let _ = writeln!(s, "| {} | {} | {} | {:+.1} | {:+.1} | {:+.1} [{:+.1}, {:+.1}] | {:+.2} | {} |", p.challenger, p.incumbent, p.days, p.challenger_bps, p.incumbent_bps, p.diff_bps, p.lo_bps, p.hi_bps, p.corr, if p.duplicate { "duplicate" } else { "" });
        }
        let _ = writeln!(s, "\nbps/day. In-sample for both sides: a difference here is a reason to look, not evidence.\n");
    }
    if !r.forward.is_empty() {
        let _ = writeln!(s, "## On paper: child against parent\n\n| Child | Parent | Days | Child | Parent | Difference [95%] |\n|---|---|---|---|---|---|");
        for p in &r.forward {
            let _ = writeln!(s, "| {} | {} | **{}** | {:+.1} | {:+.1} | {:+.1} [{:+.1}, {:+.1}] |", p.challenger, p.incumbent, p.days, p.challenger_bps, p.incumbent_bps, p.diff_bps, p.lo_bps, p.hi_bps);
        }
        let _ = writeln!(s);
    }
    if !r.messages.is_empty() {
        let _ = writeln!(s, "## Notes from the proposer\n");
        for m in &r.messages {
            let _ = writeln!(s, "- {m}");
        }
        let _ = writeln!(s);
    }
    let _ = writeln!(s, "## For the operator\n");
    if r.dry_run {
        let _ = writeln!(s, "Dry run. The would-be family files are under `lab-data/reports/evolve/dry-g{:03}/`. `lab evolve` without `--dry-run` writes them, charges their trials and judges them.", r.generation);
    } else if r.promotable.is_empty() {
        let _ = writeln!(s, "Nothing to promote. That is a result.");
    } else {
        let _ = writeln!(s, "Passed the gate and not a duplicate: {}. Promoting is your decision: `lab promote --family families/gen/<name>.toml`.", r.promotable.join(", "));
    }
    s
}

// ---------------------------------------------------------------------------
// `lab evolve --auto`: generation after generation until a stop rule.

/// Why an automatic run ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stop {
    /// A family passed the gate: promoting it is the operator's decision.
    Passed,
    /// Nothing new to propose (lessons used up, lineage covers the rest, or the loop budget is spent).
    NothingNew,
    /// `auto_stall` generations in a row with nothing better than REJECT.
    Stalled,
    /// `auto_max_generations` reached.
    MaxGenerations,
}

impl Stop {
    pub fn words(&self) -> &'static str {
        match self {
            Stop::Passed => "a family passed the gate. Promoting it is your decision",
            Stop::NothingNew => "nothing new to propose: the lessons are used up, the lineage covers the rest, or the loop's trial budget is spent",
            Stop::Stalled => "several generations in a row produced nothing better than REJECT",
            Stop::MaxGenerations => "the most generations one run may take",
        }
    }
}

/// True when a generation produced nothing better than REJECT.
fn stalled(verdicts: &[Option<String>]) -> bool {
    verdicts.iter().all(|v| matches!(v.as_deref(), None | Some("REJECT") | Some("INSUFFICIENT_EVIDENCE")))
}

#[derive(Debug, Clone, Serialize)]
pub struct AutoReport {
    pub started: chrono::DateTime<chrono::Utc>,
    pub finished: chrono::DateTime<chrono::Utc>,
    pub stop: Stop,
    pub generations: Vec<CycleReport>,
    /// Loop trials per study after the run, and the budget.
    pub loop_trials: BTreeMap<String, usize>,
    pub budget: usize,
    /// Each study's own budget (`auto_trial_budgets` overrides, else `budget`).
    pub budgets: BTreeMap<String, usize>,
}

/// Runs cycles until a stop rule. Each cycle is an ordinary `lab evolve`: its own
/// lessons file, family files, verdicts, lineage rows and report.
pub fn auto(lab: &LabConfig, max_trials: Option<usize>) -> Result<AutoReport> {
    let cfg = &lab.evolve;
    let started = chrono::Utc::now();
    let mut gens = Vec::new();
    let mut stall = 0u32;
    let stop = loop {
        if gens.len() as u32 >= cfg.auto_max_generations {
            break Stop::MaxGenerations;
        }
        let r = evolve(lab, &EvolveOpts { dry_run: false, max_trials, from_lessons: None, loop_budget: Some(cfg.auto_trial_budget) })?;
        let empty = r.proposals.is_empty();
        let passed = !r.promotable.is_empty();
        let verdicts: Vec<Option<String>> = r.proposals.iter().map(|p| p.verdict.clone()).collect();
        eprintln!("generation {}: {} proposals · {}", r.generation, r.proposals.len(), r.proposals.iter().map(|p| format!("{} {}", p.name, p.verdict.as_deref().unwrap_or("-"))).collect::<Vec<_>>().join(", "));
        if !empty {
            gens.push(r);
        }
        if empty {
            break Stop::NothingNew;
        }
        if passed {
            break Stop::Passed;
        }
        stall = if stalled(&verdicts) { stall + 1 } else { 0 };
        if stall >= cfg.auto_stall {
            break Stop::Stalled;
        }
    };
    let mut loop_trials: BTreeMap<String, usize> = BTreeMap::new();
    for e in lineage::load(&lab.lineage_path()).iter().filter(|e| e.event == "tested") {
        *loop_trials.entry(e.study.clone()).or_default() += e.trials;
    }
    let budgets = loop_trials.keys().map(|k| (k.clone(), cfg.budget_for(k, cfg.auto_trial_budget))).collect();
    let rep = AutoReport { started, finished: chrono::Utc::now(), stop, generations: gens, loop_trials, budget: cfg.auto_trial_budget, budgets };
    let dir = lab.data_dir.join("reports").join("evolve");
    std::fs::create_dir_all(&dir)?;
    let stem = format!("auto-{}", started.format("%Y%m%d-%H%M%S"));
    crate::status::write_atomic(&dir.join(format!("{stem}.json")), &serde_json::to_vec_pretty(&rep)?)?;
    crate::status::write_atomic(&dir.join(format!("{stem}.md")), auto_markdown(&rep).as_bytes())?;
    Ok(rep)
}

pub fn auto_markdown(r: &AutoReport) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# Automatic run: {} generation(s)\n", r.generations.len());
    let _ = writeln!(s, "*{} → {}*\n", r.started.format("%Y-%m-%d %H:%M UTC"), r.finished.format("%H:%M UTC"));
    let _ = writeln!(s, "**Stopped:** {}.\n", r.stop.words());
    let _ = writeln!(s, "| Generation | Proposals | Verdicts | Trials charged |\n|---|---|---|---|");
    for g in &r.generations {
        let trials: usize = g.proposals.iter().map(|p| p.new_trials).sum();
        let _ = writeln!(s, "| {} | {} | {} | {trials} |", g.generation, g.proposals.len(), g.proposals.iter().map(|p| format!("{} {}", p.name, p.verdict.as_deref().unwrap_or("-"))).collect::<Vec<_>>().join(", "));
    }
    let _ = writeln!(s, "\n| Study | Loop trials so far | Budget |\n|---|---|---|");
    for (study, n) in &r.loop_trials {
        let _ = writeln!(s, "| {study} | {n} | {} |", r.budgets.get(study).copied().unwrap_or(r.budget));
    }
    let passed: Vec<&str> = r.generations.iter().flat_map(|g| g.promotable.iter().map(String::as_str)).collect();
    let _ = writeln!(s, "\n{}", if passed.is_empty() { "Nothing to promote. Each generation's report is in `lab-data/reports/evolve/gNNN.md`.".to_string() } else { format!("Passed the gate and not a duplicate: {}. Read its report, then `lab promote --family families/gen/<name>.toml` if you want it on paper.", passed.join(", ")) });
    s
}

// ---------------------------------------------------------------------------
// The loop in memory, on synthetic markets.

#[derive(Debug, Clone, Default)]
pub struct LoopResult {
    /// Promotions by generation (0 = the seed family).
    pub promoted_by_gen: Vec<usize>,
    pub stop: Option<Stop>,
    pub verdicts: BTreeMap<String, usize>,
    pub proposals: usize,
    pub trials: usize,
    pub lockbox_openings: usize,
    pub generations_run: usize,
}

/// The seed family judged, then rounds of learn → propose → judge under the same stop
/// rules as `lab evolve --auto` (at most `generations`), all in memory on one panel.
pub fn run_loop(lab: &LabConfig, seed_fam: &Family, p: &Panel, generations: u32, seed: u64) -> Result<LoopResult> {
    let mut books = Books::memory();
    let mut res = LoopResult { promoted_by_gen: vec![0; generations as usize + 1], ..Default::default() };
    let mut families: HashMap<String, Family> = HashMap::new();
    families.insert(seed_fam.family.clone(), seed_fam.clone());
    let ev = compare::validate_family(lab, seed_fam, p, None, &Aux::default(), None, None, None, 0, &mut books)?;
    let mut seen: Vec<(String, serde_json::Value)> = vec![("memory".into(), serde_json::to_value(&ev)?)];
    res.promoted_by_gen[0] += (ev.verdict == Verdict::PromoteToPaper) as usize;
    *res.verdicts.entry(ev.verdict.word().into()).or_default() += 1;
    let mut events: Vec<Event> = Vec::new();
    let mut stall = 0u32;
    res.stop = Some(Stop::MaxGenerations);
    for gen_no in 1..=generations {
        let g = Gathered { verdicts: seen.clone(), families: families.clone(), monitor: None, lineage: events.clone() };
        let lessons = learn_from(&g, &books.rows(), lab);
        let parents = parents_of(&g.verdicts, &g.families);
        let charged = books.charged();
        let out = propose::propose(&propose::Inputs { lessons: &lessons, parents: &parents, lineage: &events, charged: &charged, research_venue: &p.venue, generation: gen_no, seed, loop_budget: Some(lab.evolve.auto_trial_budget) }, &lab.evolve);
        if out.proposals.is_empty() {
            res.stop = Some(Stop::NothingNew);
            break;
        }
        res.generations_run += 1;
        res.proposals += out.proposals.len();
        let fams: Vec<Family> = out.proposals.iter().map(|x| x.family.clone()).collect();
        for x in &out.proposals {
            events.push(Event { event: "proposed".into(), family: x.family.family.clone(), name: x.family.label().into(), parent: x.family.parent.clone(), generation: gen_no, key: x.key.clone(), ..Event::default() });
            families.insert(x.family.family.clone(), x.family.clone());
        }
        let mut src = |_: &Family| Ok((p.clone(), None));
        let before = events.iter().filter(|e| e.event == "tested").count();
        let mut verdicts = Vec::new();
        for j in compare::judge_generation(lab, &fams, &mut src, &|_| before, &mut books)? {
            verdicts.push(Some(j.eval.verdict.word().to_string()));
            res.promoted_by_gen[gen_no as usize] += (j.eval.verdict == Verdict::PromoteToPaper) as usize;
            *res.verdicts.entry(j.eval.verdict.word().into()).or_default() += 1;
            events.push(Event { event: "tested".into(), family: j.family.clone(), name: j.label.clone(), generation: gen_no, verdict: Some(j.eval.verdict.word().into()), trials: j.new_trials, study: j.study.clone(), ..Event::default() });
            seen.push(("memory".into(), serde_json::to_value(&j.eval)?));
        }
        if res.promoted_by_gen[gen_no as usize] > 0 {
            res.stop = Some(Stop::Passed);
            break;
        }
        stall = if stalled(&verdicts) { stall + 1 } else { 0 };
        if stall >= lab.evolve.auto_stall {
            res.stop = Some(Stop::Stalled);
            break;
        }
    }
    res.trials = books.total(&format!("{}-{}", p.venue, p.interval));
    res.lockbox_openings = books.lockbox_openings();
    Ok(res)
}

/// `lab calibrate --evolve`: the loop on `panels` synthetic markets, in parallel.
pub fn calibrate_loop(lab: &LabConfig, seed_fam: &Family, panels: usize, days: usize, coins: usize, edge: f64, generations: u32) -> Result<String> {
    let mut inner = lab.clone();
    inner.threads = 1;
    let workers = lab.threads.max(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: Vec<(usize, Result<LoopResult>)> = std::thread::scope(|sc| {
        let handles: Vec<_> = (0..workers).map(|_| sc.spawn(|| {
            let mut mine = Vec::new();
            loop {
                let s = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if s >= panels {
                    break;
                }
                let p = synth::panel(lab.seed + s as u64, coins, days, &seed_fam.interval, edge);
                let r = run_loop(&inner, seed_fam, &p, generations, lab.seed ^ s as u64);
                if let Ok(x) = &r {
                    eprintln!("panel {:>3}: promoted by generation {:?} · {} proposals · {} trials", s + 1, x.promoted_by_gen, x.proposals, x.trials);
                }
                mine.push((s, r));
            }
            mine
        })).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    let mut rs: Vec<(usize, LoopResult)> = Vec::new();
    for (s, r) in results {
        rs.push((s, r.with_context(|| format!("panel {}", s + 1))?));
    }
    rs.sort_by_key(|x| x.0);
    let n = rs.len().max(1) as f64;
    let any = rs.iter().filter(|(_, r)| r.promoted_by_gen.iter().sum::<usize>() > 0).count();
    let seed_only = rs.iter().filter(|(_, r)| r.promoted_by_gen[0] > 0).count();
    let loop_only = rs.iter().filter(|(_, r)| r.promoted_by_gen[0] == 0 && r.promoted_by_gen[1..].iter().sum::<usize>() > 0).count();
    let mut s = String::new();
    let _ = writeln!(s, "# Evolve-loop calibration: seed `{}` ({}), {panels} synthetic panels ({days} days × {coins} coins), {generations} generations, edge {edge}\n", seed_fam.label(), seed_fam.interval);
    let _ = writeln!(s, "Budget per cycle: {} proposals, {} specs each, {} trials. Stop rules as `lab evolve --auto`: {} loop trials per study, {} stalled generations, at most {generations} generations.\n", lab.evolve.proposals, lab.evolve.specs_per_proposal, lab.evolve.trials_per_cycle, lab.evolve.auto_trial_budget, lab.evolve.auto_stall);
    let _ = writeln!(s, "| | Panels | Share |\n|---|---|---|");
    let _ = writeln!(s, "| Any promotion (seed or loop) | {any} | {:.0}% |", any as f64 / n * 100.0);
    let _ = writeln!(s, "| Seed family promoted | {seed_only} | {:.0}% |", seed_only as f64 / n * 100.0);
    let _ = writeln!(s, "| Promoted only by the loop | {loop_only} | {:.0}% |", loop_only as f64 / n * 100.0);
    let _ = writeln!(s, "\n| Generation | Promotions |\n|---|---|");
    for g in 0..=generations as usize {
        let _ = writeln!(s, "| {g} | {} |", rs.iter().map(|(_, r)| r.promoted_by_gen.get(g).copied().unwrap_or(0)).sum::<usize>());
    }
    let mut verdicts: BTreeMap<String, usize> = BTreeMap::new();
    for (_, r) in &rs {
        for (k, v) in &r.verdicts {
            *verdicts.entry(k.clone()).or_default() += v;
        }
    }
    let _ = writeln!(s, "\n| Verdict (all families judged) | Count |\n|---|---|");
    for (k, v) in &verdicts {
        let _ = writeln!(s, "| {k} | {v} |");
    }
    let mut stops: BTreeMap<&'static str, usize> = BTreeMap::new();
    for (_, r) in &rs {
        *stops.entry(match r.stop {
            Some(Stop::Passed) => "a family passed",
            Some(Stop::NothingNew) => "nothing new to propose / budget spent",
            Some(Stop::Stalled) => "stalled",
            Some(Stop::MaxGenerations) | None => "generation cap",
        }).or_default() += 1;
    }
    let _ = writeln!(s, "\n| Why the loop stopped | Panels |\n|---|---|");
    for (k, v) in &stops {
        let _ = writeln!(s, "| {k} | {v} |");
    }
    let props: usize = rs.iter().map(|(_, r)| r.proposals).sum();
    let trials: usize = rs.iter().map(|(_, r)| r.trials).sum();
    let gens: usize = rs.iter().map(|(_, r)| r.generations_run).sum();
    let locks: usize = rs.iter().map(|(_, r)| r.lockbox_openings).sum();
    let _ = writeln!(s, "\nPer panel on average: {:.1} generations run, {:.1} proposals, {:.0} trials on the ledger, {:.2} lockbox openings.", gens as f64 / n, props as f64 / n, trials as f64 / n, locks as f64 / n);
    if edge == 0.0 {
        let _ = writeln!(s, "\nThese markets hold nothing to find: every promotion is a false positive. **{any} of {panels}** panels had one (requirement: 0).");
    } else {
        let _ = writeln!(s, "\nThese markets hold a planted mean-reverting edge: the seed family alone found it on {seed_only} of {panels}, the loop on {any}.");
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loop_runs_in_memory_and_charges_every_proposal() {
        let mut lab = LabConfig::default();
        lab.gate.baseline_draws = 20;
        lab.gate.boot_iters = 200;
        lab.gate.rc_iters = 150;
        lab.threads = 1;
        let seed = Family::parse("family = \"meanrev\"\ninterval = \"1h\"\n[grid]\nn = [24, 48]\nz = [1.5, 2.0]\n[fixed]\nhold = 12\nshorts = 1\n").unwrap();
        let p = synth::panel(11, 6, 420, "1h", 0.0);
        let r = run_loop(&lab, &seed, &p, 2, 5).unwrap();
        assert!(r.generations_run >= 1, "{r:?}");
        assert!(r.proposals >= 1);
        assert!(r.trials > 4, "proposals' specs are charged on top of the seed's 4: {}", r.trials);
        assert_eq!(r.promoted_by_gen.iter().sum::<usize>(), 0, "nothing to find, nothing promoted: {r:?}");
        let again = run_loop(&lab, &seed, &p, 2, 5).unwrap();
        assert_eq!((again.proposals, again.trials), (r.proposals, r.trials), "the loop is deterministic");
    }

    #[test]
    fn report_says_what_it_cost() {
        let r = CycleReport { generation: 3, dry_run: true, at: chrono::Utc::now(), lessons_file: "x.json".into(), lessons_total: 9, lessons_usable: 4, studies: vec![StudyCost { study: "binance-1h".into(), trials_before: 202, trials_after: 230, design_days: 836, luck_bar_before: luck_bar(202, 836), luck_bar_after: luck_bar(230, 836) }], proposals: Vec::new(), pairings: Vec::new(), forward: Vec::new(), messages: vec!["trial budget".into()], promotable: Vec::new(), lockbox_openings: 0 };
        let md = markdown(&r);
        assert!(md.contains("| binance-1h | 202 | 230 (+28) |"), "{md}");
        assert!(r.studies[0].luck_bar_after > r.studies[0].luck_bar_before);
        assert!(md.contains("Dry run"));
    }
}


#[cfg(test)]
mod auto_tests {
    use super::*;

    #[test]
    fn only_revise_or_better_breaks_a_stall() {
        let v = |x: &[&str]| x.iter().map(|s| Some(s.to_string())).collect::<Vec<_>>();
        assert!(stalled(&v(&["REJECT", "INSUFFICIENT_EVIDENCE"])));
        assert!(!stalled(&v(&["REJECT", "REVISE"])));
        assert!(!stalled(&v(&["PROMOTE_TO_PAPER"])));
        assert!(stalled(&[]));
    }
}
