//! `jev-trader lab`: where strategies are searched, tested and killed before they
//! reach the trader. See DESIGN-V3.md.
//!
//! - `fetch`     bring the local candle and funding archive up to date
//! - `search`    run a family's grid on the design window (leaderboard only)
//! - `validate`  search + the gate: one verdict, with the search paid for
//! - `run`       one spec, one simulation, plain numbers
//! - `calibrate` run the gate on synthetic no-edge markets: how often does it lie?
//! - `promote`   hand a validated spec (or a labelled candidate) to the trader
//! - `report`    list the verdicts so far

pub mod data;
pub mod gate;
pub mod panel;
pub mod probe;
pub mod rules;
pub mod search;
pub mod sim;
pub mod stat;
pub mod strategy;
pub mod synth;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde::{Deserialize, Serialize};

use data::{Archive, BINANCE, HL};
use panel::{Panel, UniverseRule};
use rules::{Costs, Sizing, SizingMode};
use strategy::{Aux, Family, Spec};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Universe {
    /// Hyperliquid coin names. Fixed list of perps listed on both venues.
    pub coins: Vec<String>,
    /// Binance symbol when it isn't `<coin>USDT`.
    pub binance: BTreeMap<String, String>,
    /// Research venue: median daily dollar volume over the trailing 30 days.
    pub min_dollar_volume: f64,
    pub min_history_days: usize,
}

impl Default for Universe {
    fn default() -> Self {
        let coins = ["BTC", "ETH", "SOL", "XRP", "DOGE", "BNB", "ADA", "AVAX", "LINK", "SUI", "NEAR", "LTC", "ZEC", "ENA", "HYPE", "AAVE", "UNI", "WLD", "kPEPE", "BCH"];
        Self {
            coins: coins.iter().map(|s| s.to_string()).collect(),
            binance: BTreeMap::from([("kPEPE".to_string(), "1000PEPEUSDT".to_string())]),
            min_dollar_volume: 50e6,
            min_history_days: 20,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Walk {
    pub train_days: usize,
    pub test_days: usize,
    /// Days skipped between training and test (lookbacks and open positions).
    pub purge_days: usize,
    /// A spec needs this many days in the market to be picked.
    pub min_active_days: usize,
}

impl Default for Walk {
    fn default() -> Self {
        Self { train_days: 180, test_days: 60, purge_days: 5, min_active_days: 20 }
    }
}

/// Thresholds of the gate. Written down before any strategy is tested (DESIGN-V3.md);
/// changing one after seeing a result is a new trial.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct GateCfg {
    pub min_design_days: usize,
    pub min_trades: usize,
    /// Mean block length of the stationary bootstrap, days.
    pub block_days: f64,
    pub boot_iters: usize,
    pub rc_iters: usize,
    pub rc_p_pass: f64,
    pub rc_p_fatal: f64,
    pub dsr_pass: f64,
    pub dsr_fatal: f64,
    pub pbo_blocks: usize,
    pub pbo_pass: f64,
    pub pbo_fatal: f64,
    pub plateau_frac: f64,
    pub cost_stress: f64,
    pub quarters_pos_frac: f64,
    pub top_day_frac: f64,
    pub baseline_draws: usize,
    pub baseline_pct: f64,
    /// One-sided p the lockbox must beat on its first opening; halved on the second, a third on the third…
    pub lockbox_alpha: f64,
    pub venue_corr: f64,
    /// Paper trading's sequential test.
    pub paper_alpha: f64,
    pub paper_beta: f64,
}

impl Default for GateCfg {
    fn default() -> Self {
        Self {
            min_design_days: 250,
            min_trades: 100,
            block_days: 5.0,
            boot_iters: 2000,
            rc_iters: 1000,
            rc_p_pass: 0.05,
            rc_p_fatal: 0.20,
            dsr_pass: 0.95,
            dsr_fatal: 0.50,
            pbo_blocks: 10,
            pbo_pass: 0.30,
            pbo_fatal: 0.50,
            plateau_frac: 0.5,
            cost_stress: 2.0,
            quarters_pos_frac: 0.6,
            top_day_frac: 0.01,
            baseline_draws: 200,
            baseline_pct: 0.95,
            lockbox_alpha: 0.20,
            venue_corr: 0.7,
            paper_alpha: 0.05,
            paper_beta: 0.20,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LabConfig {
    pub data_dir: PathBuf,
    /// Venue whose long history the search runs on. Prices only; never costs.
    pub research_venue: String,
    /// Finest bars kept for the research venue; coarser ones are built from it.
    pub base_interval: String,
    /// Bars fetched from Hyperliquid (it only serves its latest 5000 per interval).
    pub hl_intervals: Vec<String>,
    /// First day fetched for the research venue.
    pub start: String,
    pub capital: f64,
    /// Newest share of days no search may see.
    pub lockbox_frac: f64,
    pub threads: usize,
    pub seed: u64,
    pub universe: Universe,
    pub costs: Costs,
    pub sizing: Sizing,
    pub walk: Walk,
    pub gate: GateCfg,
}

impl Default for LabConfig {
    fn default() -> Self {
        Self {
            data_dir: "lab-data".into(),
            research_venue: BINANCE.into(),
            base_interval: "15m".into(),
            hl_intervals: vec!["15m".into(), "1h".into()],
            start: "2024-01-01".into(),
            capital: 10_000.0,
            lockbox_frac: 0.20,
            threads: 4,
            seed: 20261111,
            universe: Universe::default(),
            costs: Costs::default(),
            sizing: Sizing::default(),
            walk: Walk::default(),
            gate: GateCfg::default(),
        }
    }
}

impl LabConfig {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let cfg: LabConfig = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        if !(0.05..=0.5).contains(&cfg.lockbox_frac) {
            bail!("lockbox_frac must be between 0.05 and 0.5");
        }
        Ok(cfg)
    }

    pub fn archive(&self) -> Archive {
        Archive::new(&self.data_dir)
    }

    pub fn trials_path(&self) -> PathBuf {
        self.data_dir.join("trials.jsonl")
    }

    pub fn lockbox_path(&self) -> PathBuf {
        self.data_dir.join("lockbox.jsonl")
    }

    pub fn report_dir(&self, study: &str) -> PathBuf {
        self.data_dir.join("reports").join(study)
    }

    fn rule(&self, venue: &str) -> UniverseRule {
        if venue == self.research_venue {
            UniverseRule { min_dollar_volume: self.universe.min_dollar_volume, min_history_days: self.universe.min_history_days }
        } else {
            // The trading venue's history is short; every listed coin of the fixed list counts.
            UniverseRule { min_dollar_volume: 0.0, min_history_days: 0 }
        }
    }

    /// The panel for a venue and bar size, built from the archive (coarser bars are
    /// aggregated from the base interval when they aren't stored).
    pub fn panel(&self, venue: &str, interval: &str, coins: &[String]) -> Result<Panel> {
        let ar = self.archive();
        let rule = self.rule(venue);
        let stored = coins.iter().any(|c| ar.candle_path(venue, interval, c).exists());
        if stored {
            return Panel::load(&ar, venue, interval, coins, rule);
        }
        let (want, base) = (crate::market::interval_ms(interval).context("bad interval")?, crate::market::interval_ms(&self.base_interval).context("bad base_interval")?);
        if want <= base || want % base != 0 {
            bail!("no {interval} candles for {venue} in {}; run `jev-trader lab fetch`", self.data_dir.display());
        }
        Panel::load(&ar, venue, &self.base_interval, coins, rule)?.resample((want / base) as usize, interval, rule)
    }
}

#[derive(Subcommand)]
pub enum LabCmd {
    /// Bring the local candle and funding archive up to date (no keys needed).
    Fetch {
        /// binance, hyperliquid or all.
        #[arg(long, default_value = "all")]
        venue: String,
    },
    /// Run a family's grid on the design window and print the leaderboard.
    Search {
        #[arg(long)]
        family: PathBuf,
    },
    /// Search a family and judge it: PROMOTE_TO_PAPER, REVISE, REJECT or INSUFFICIENT_EVIDENCE.
    Validate {
        #[arg(long)]
        family: PathBuf,
        /// Judge this spec id from the grid instead of the grid's best.
        #[arg(long)]
        spec: Option<String>,
    },
    /// Simulate one spec and print plain numbers.
    Run {
        #[arg(long)]
        family: PathBuf,
        /// Spec id, e.g. "meanrev:hold=24,n=288,stop=500,z=2".
        #[arg(long)]
        spec: String,
        /// Venue to run on. Default: the research venue.
        #[arg(long)]
        venue: Option<String>,
        /// First day (YYYY-MM-DD).
        #[arg(long)]
        from: Option<String>,
        /// Comma-separated coins instead of the whole universe.
        #[arg(long)]
        coins: Option<String>,
        /// Reproduce `scalp-backtest`: fixed 10% size, no funding, no stop slippage, 2% daily loss limit.
        #[arg(long)]
        like_scalp: bool,
    },
    /// Run the whole gate on synthetic markets with a known answer.
    Calibrate {
        #[arg(long)]
        family: PathBuf,
        #[arg(long, default_value_t = 50)]
        panels: usize,
        #[arg(long, default_value_t = 600)]
        days: usize,
        #[arg(long, default_value_t = 8)]
        coins: usize,
        /// 0 = nothing to find (false-positive rate); up to 1 = a planted mean-reverting edge (power).
        #[arg(long, default_value_t = 0.0)]
        edge: f64,
    },
    /// Hand a spec to the trader: the validated pick, or (--candidate) the best spec labelled unproven.
    Promote {
        #[arg(long)]
        family: PathBuf,
        #[arg(long)]
        candidate: bool,
    },
    /// List the verdicts on file.
    Report,
    /// Jev as a market reader: replay a probe over history, score its answers.
    Jev {
        #[command(subcommand)]
        cmd: JevCmd,
    },
}

#[derive(Subcommand)]
pub enum JevCmd {
    /// Walk the archive bar by bar and ask Jev at each of the probe's points.
    /// Answers are cached: a rerun only asks what is missing.
    Replay {
        #[arg(long)]
        probe: PathBuf,
        #[arg(long, default_value = "hyperliquid")]
        venue: String,
        /// Comma-separated coins instead of the whole universe.
        #[arg(long)]
        coins: Option<String>,
        /// Most Jev calls this run may make.
        #[arg(long, default_value_t = 200)]
        max_calls: usize,
        /// anon (default), revealed (coin and date shown: memory check) or repeat (stability check).
        #[arg(long, default_value = "anon")]
        variant: String,
        /// revealed / repeat: how many of the anonymous answers to re-ask.
        #[arg(long, default_value_t = 150)]
        sample: usize,
        /// Only the bars where the rule really entered.
        #[arg(long)]
        entries_only: bool,
        /// Keep every N-th point: spreads a small budget over the whole history.
        #[arg(long, default_value_t = 1)]
        stride: usize,
        /// Print the first state and the call count; ask nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Score the cached answers against what happened next.
    Score {
        #[arg(long)]
        probe: PathBuf,
        #[arg(long, default_value = "hyperliquid")]
        venue: String,
        #[arg(long)]
        coins: Option<String>,
    },
}

fn day_ms(s: &str) -> Result<i64> {
    Ok(chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").with_context(|| format!("bad date {s} (want YYYY-MM-DD)"))?.and_hms_opt(0, 0, 0).expect("midnight").and_utc().timestamp_millis())
}

/// `meanrev:hold=24,n=288` → Spec.
pub fn parse_spec(id: &str) -> Result<Spec> {
    let (family, rest) = id.split_once(':').with_context(|| format!("bad spec id `{id}` (want family:key=value,…)"))?;
    let mut params = BTreeMap::new();
    for kv in rest.split(',').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').with_context(|| format!("bad parameter `{kv}` in `{id}`"))?;
        params.insert(k.trim().to_string(), v.trim().parse::<f64>().with_context(|| format!("bad number in `{kv}`"))?);
    }
    Ok(Spec { family: family.to_string(), params })
}

#[derive(Debug, Serialize, Deserialize)]
struct LockboxRead {
    study: String,
    family: String,
    spec: String,
    at: chrono::DateTime<chrono::Utc>,
    passed: bool,
}

pub async fn run(cmd: LabCmd, lab: &LabConfig, http: &reqwest::Client, hl_api: &'static str, jev_cfg: &crate::config::JevConfig) -> Result<()> {
    match cmd {
        LabCmd::Fetch { venue } => fetch(lab, http, hl_api, &venue).await,
        LabCmd::Search { family } => {
            let fam = Family::load(&family)?;
            let full = lab.panel(fam.venue.as_deref().unwrap_or(&lab.research_venue), &fam.interval, &lab.universe.coins)?;
            let design = full.head(design_bars(&full, lab));
            eprintln!("{} · design window: first {} days", full.describe(), design.n_days());
            let cfg = sim::SimCfg::new(lab.capital, lab.costs.clone(), lab.sizing.clone());
            let trials = search::run_grid(&design, &fam, &family_aux(lab, &fam, &full)?, &cfg, lab.threads)?;
            let study = format!("{}-{}", full.venue, full.interval);
            let (new, total) = search::record_trials(&lab.trials_path(), &study, &fam.family, &trials);
            let mut rows: Vec<(f64, &search::Trial)> = trials.iter().map(|t| (stat::sharpe(&t.daily), t)).collect();
            rows.sort_by(|a, b| b.0.total_cmp(&a.0));
            println!("{:<64} {:>7} {:>10} {:>7} {:>9}", "spec", "Sharpe", "bps/day", "trades", "bps/trade");
            for (s, t) in rows.iter().take(15) {
                println!("{:<64} {:>7.2} {:>+10.2} {:>7} {:>+9.1}", t.spec.id(), s * gate::DAYS_PER_YEAR.sqrt(), stat::mean(&t.daily) * 1e4, t.trades, t.mean_trade_bps);
            }
            println!("\n{} specs searched ({new} new). {total} trials are now on record for {study}.\nA leaderboard is the best of many tries; `lab validate` says whether any of it is real.", trials.len());
            Ok(())
        }
        LabCmd::Validate { family, spec } => {
            let fam = Family::load(&family)?;
            let venue_name = fam.venue.clone().unwrap_or_else(|| lab.research_venue.clone());
            let full = lab.panel(&venue_name, &fam.interval, &lab.universe.coins)?;
            let aux = family_aux(lab, &fam, &full)?;
            // The trading venue's own data is the last check for rules searched elsewhere.
            let venue = if venue_name == HL { None } else { lab.panel(HL, &fam.interval, &lab.universe.coins).ok() };
            let study = format!("{}-{}", full.venue, full.interval);
            let force = spec.as_deref().map(parse_spec).transpose()?;
            // Record this grid's trials first, so the count the gate uses includes them.
            let cfg = sim::SimCfg::new(lab.capital, lab.costs.clone(), lab.sizing.clone());
            let design = full.head(design_bars(&full, lab));
            let trials = search::run_grid(&design, &fam, &aux, &cfg, lab.threads)?;
            let (_, ledger_trials) = search::record_trials(&lab.trials_path(), &study, &fam.family, &trials);
            drop(design);
            let reads: Vec<LockboxRead> = crate::status::read_jsonl(&lab.lockbox_path());
            let prior = |spec_id: &str| reads.iter().filter(|r| r.study == study && r.spec != spec_id).map(|r| r.spec.as_str()).collect::<std::collections::HashSet<_>>().len();
            // The pick isn't known before the run; count every earlier opening, then correct below.
            let all_prior = prior("");
            let vaux = Aux::default();
            let inputs = |reads: usize, trials: Option<Vec<search::Trial>>| gate::Inputs { full: &full, aux: &aux, venue: venue.as_ref().map(|v| (v, &vaux)), fam: &fam, ledger_trials, lockbox_reads: reads, force: force.as_ref(), design_trials: trials };
            let mut ev = gate::evaluate(inputs(all_prior, Some(trials)), lab)?;
            if let (true, Some(p)) = (ev.lockbox_opened, ev.pick.clone()) {
                let own = prior(&p.id());
                if own != all_prior {
                    // This same spec opened the lockbox before: looking again is not a new look.
                    ev = gate::evaluate(inputs(own, None), lab)?;
                } else {
                    crate::bot::append_jsonl(&lab.lockbox_path(), &LockboxRead { study: study.clone(), family: fam.family.clone(), spec: p.id(), at: chrono::Utc::now(), passed: ev.verdict == gate::Verdict::PromoteToPaper });
                }
            }
            let md = gate::markdown(&ev);
            let dir = lab.report_dir(&study);
            std::fs::create_dir_all(&dir)?;
            crate::status::write_atomic(&dir.join(format!("{}.md", fam.family)), md.as_bytes())?;
            crate::status::write_atomic(&dir.join(format!("{}.json", fam.family)), &serde_json::to_vec_pretty(&ev)?)?;
            println!("{md}");
            eprintln!("verdict: {} · written to {}", ev.verdict.word(), dir.display());
            Ok(())
        }
        LabCmd::Run { family, spec, venue, from, coins, like_scalp } => {
            let fam = Family::load(&family)?;
            let spec = parse_spec(&spec)?;
            let venue = venue.unwrap_or_else(|| lab.research_venue.clone());
            let coins: Vec<String> = coins.map(|c| c.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_else(|| lab.universe.coins.clone());
            let mut p = lab.panel(&venue, &fam.interval, &coins)?;
            let mut cfg = sim::SimCfg::new(lab.capital, lab.costs.clone(), lab.sizing.clone());
            if like_scalp {
                cfg.sizing = Sizing { mode: SizingMode::Fixed, pos_pct: 0.10, ..lab.sizing.clone() };
                cfg.costs.funding = false;
                cfg.costs.stop_slippage_bps = 0.0;
                cfg.daily_loss_limit_pct = 0.02;
            }
            // Signals need their warm-up, so the plan is built on everything and the
            // result is read from `from` on.
            let from_day = match from {
                Some(f) => p.day_of(p.index_at(day_ms(&f)?).min(p.n.saturating_sub(1))),
                None => 0,
            };
            if venue == lab.research_venue && from_day == 0 {
                // Never show lockbox days from a plain run.
                p = p.head(design_bars(&p, lab));
                eprintln!("research venue: lockbox days are left out of `lab run`");
            }
            let plan = strategy::plan(&spec, &p, &Aux::default())?;
            let mut run = sim::simulate(&p, &plan, &cfg);
            let first_bar = from_day * p.bars_per_day();
            run.trades.retain(|t| t.entry_i >= first_bar);
            let daily = &run.daily[from_day.min(run.daily.len())..];
            println!("{}\n{}", p.describe(), run_summary(&spec, &run, daily, &p, lab));
            Ok(())
        }
        LabCmd::Calibrate { family, panels, days, coins, edge } => {
            let fam = Family::load(&family)?;
            let mut lab = lab.clone();
            lab.gate.baseline_draws = lab.gate.baseline_draws.min(60);
            lab.gate.boot_iters = lab.gate.boot_iters.min(600);
            lab.gate.rc_iters = lab.gate.rc_iters.min(400);
            let mut tally: BTreeMap<&'static str, usize> = BTreeMap::new();
            let mut opened = 0;
            // Per check: (times run, times passed), in the order the gate runs them.
            let mut per_check: Vec<(&'static str, usize, usize)> = Vec::new();
            for s in 0..panels {
                let p = synth::panel(lab.seed + s as u64, coins, days, &fam.interval, edge);
                let aux = Aux::default();
                let ev = gate::evaluate(gate::Inputs { full: &p, aux: &aux, venue: None, fam: &fam, ledger_trials: 0, lockbox_reads: 0, force: None, design_trials: None }, &lab)?;
                *tally.entry(ev.verdict.word()).or_default() += 1;
                opened += ev.lockbox_opened as usize;
                for c in &ev.checks {
                    match per_check.iter_mut().find(|x| x.0 == c.name) {
                        Some(x) => {
                            x.1 += 1;
                            x.2 += c.pass as usize;
                        }
                        None => per_check.push((c.name, 1, c.pass as usize)),
                    }
                }
                let best = ev.leaders.first().map(|l| format!("in-sample best Sharpe {:.2}", l.sharpe_annual)).unwrap_or_default();
                eprintln!("panel {:>3}: {:<22} {best}", s + 1, ev.verdict.word());
            }
            let pct = |n: usize| n as f64 / panels.max(1) as f64 * 100.0;
            println!("# Gate calibration: {} on {panels} synthetic panels ({days} days × {coins} coins, {}, edge {edge})\n", fam.family, fam.interval);
            println!("| Outcome | Panels | Share |\n|---|---|---|");
            for (k, v) in &tally {
                println!("| {k} | {v} | {:.0}% |", pct(*v));
            }
            println!("\n| Check | Run on | Passed |\n|---|---|---|");
            for (name, n, ok) in &per_check {
                println!("| {name} | {n} | {:.0}% |", *ok as f64 / (*n).max(1) as f64 * 100.0);
            }
            println!("\nLockbox opened on {:.0}% of panels.", pct(opened));
            let promoted = tally.get("PROMOTE_TO_PAPER").copied().unwrap_or(0);
            if edge == 0.0 {
                println!("\nThere is nothing to find in these markets, so every promotion is a false positive: **{:.0}%** (target ≤ 5%).", pct(promoted));
            } else {
                println!("\nThese markets hold a real mean-reverting edge: the gate found it in **{:.0}%** of them.", pct(promoted));
            }
            Ok(())
        }
        LabCmd::Promote { family, candidate } => promote(lab, &family, candidate),
        LabCmd::Jev { cmd } => jev(cmd, lab, jev_cfg).await,
        LabCmd::Report => {
            let root = lab.data_dir.join("reports");
            println!("| Study | Family | Verdict | Specs | Trials on record | Candidate |\n|---|---|---|---|---|---|");
            let mut n = 0;
            for study in std::fs::read_dir(&root).into_iter().flatten().flatten() {
                for f in std::fs::read_dir(study.path()).into_iter().flatten().flatten() {
                    let path = f.path();
                    if path.extension().is_some_and(|e| e == "json") && !path.to_string_lossy().ends_with(".card.json") {
                        let Some(v) = crate::status::read_json::<serde_json::Value>(&path) else { continue };
                        let pick = v["pick"]["params"].as_object().map(|m| m.iter().map(|(k, x)| format!("{k}={x}")).collect::<Vec<_>>().join(",")).unwrap_or_default();
                        println!("| {} | {} | {} | {} | {} | `{pick}` |", v["study"].as_str().unwrap_or(""), v["family"].as_str().unwrap_or(""), v["verdict"].as_str().unwrap_or(""), v["specs"], v["trials_ledger"]);
                        n += 1;
                    }
                }
            }
            if n == 0 {
                println!("\nNo verdicts yet: run `jev-trader lab validate --family families/<name>.toml`.");
            }
            Ok(())
        }
    }
}

/// Cached Jev answers for a family that reads a probe; empty for the others.
fn family_aux(lab: &LabConfig, fam: &Family, p: &Panel) -> Result<Aux> {
    match &fam.probe {
        Some(path) => {
            let pr = probe::Probe::load(path)?;
            Ok(probe::aux(p, &pr, &probe::load_answers(&lab.data_dir, &pr, &p.venue)))
        }
        None => Ok(Aux::default()),
    }
}

/// The panel a probe is replayed and scored on. On the research venue that is the
/// design window only: Jev's answers are evidence too, and the lockbox stays sealed.
fn jev_panel(lab: &LabConfig, venue: &str, interval: &str, coins: &[String]) -> Result<Panel> {
    let p = lab.panel(venue, interval, coins)?;
    Ok(if venue == lab.research_venue { p.head(design_bars(&p, lab)) } else { p })
}

async fn jev(cmd: JevCmd, lab: &LabConfig, jev_cfg: &crate::config::JevConfig) -> Result<()> {
    let coins_of = |c: Option<String>| c.map(|c| c.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_else(|| lab.universe.coins.clone());
    let cfg = sim::SimCfg::new(lab.capital, lab.costs.clone(), lab.sizing.clone());
    match cmd {
        JevCmd::Replay { probe, venue, coins, max_calls, variant, sample, entries_only, stride, dry_run } => {
            let pr = probe::Probe::load(&probe)?;
            let p = jev_panel(lab, &venue, &pr.interval, &coins_of(coins))?;
            let client = if dry_run { None } else { Some(crate::jev::JevClient::from_env(&jev_cfg.model, std::time::Duration::from_millis(jev_cfg.timeout_ms.max(8000)))?) };
            let opts = probe::ReplayOpts { variant: probe::Variant::parse(&variant)?, max_calls, concurrency: jev_cfg.concurrency.clamp(1, 8), entries_only, stride, sample, seed: lab.seed, dry_run };
            eprintln!("{} · probe {} ({})", p.describe(), pr.name, variant);
            let st = probe::replay(&p, &pr, &cfg, &lab.data_dir, client.as_ref(), &opts).await?;
            let per = if st.asked > 0 { (st.tokens_in + st.tokens_out) as f64 / st.asked as f64 } else { 0.0 };
            println!("points {} · already cached {} · asked now {} · failed {} · still to ask {}", st.planned, st.cached, st.asked, st.failed, st.left);
            if st.asked > 0 {
                println!("tokens: {} in + {} out = {:.0} per call. Asking the remaining {} would take about {:.1}M tokens.", st.tokens_in, st.tokens_out, per, st.left, st.left as f64 * per / 1e6);
            }
            Ok(())
        }
        JevCmd::Score { probe, venue, coins } => {
            let pr = probe::Probe::load(&probe)?;
            let p = jev_panel(lab, &venue, &pr.interval, &coins_of(coins))?;
            let answers = probe::load_answers(&lab.data_dir, &pr, &venue);
            let sc = probe::score(&p, &pr, &answers);
            let md = probe::markdown(&sc, &pr, &p);
            let dir = lab.data_dir.join("reports").join("jev");
            std::fs::create_dir_all(&dir)?;
            crate::status::write_atomic(&dir.join(format!("{}-{venue}.md", pr.name)), md.as_bytes())?;
            crate::status::write_atomic(&dir.join(format!("{}-{venue}.json", pr.name)), &serde_json::to_vec_pretty(&sc)?)?;
            println!("{md}");
            Ok(())
        }
    }
}

fn design_bars(full: &Panel, lab: &LabConfig) -> usize {
    ((full.n_days() as f64) * (1.0 - lab.lockbox_frac)).floor() as usize * full.bars_per_day()
}

async fn fetch(lab: &LabConfig, http: &reqwest::Client, hl_api: &'static str, venue: &str) -> Result<()> {
    let ar = lab.archive();
    let start_ms = day_ms(&lab.start)?;
    if venue == "all" || venue == BINANCE {
        let coins = lab.universe.coins.iter().map(|c| (c.clone(), lab.universe.binance.get(c).cloned().unwrap_or_else(|| format!("{c}USDT")))).collect();
        let iv = [lab.base_interval.clone()];
        data::fetch(http, &ar, &data::FetchPlan { venue: BINANCE, intervals: &iv, coins, start_ms, hl_api }).await?;
    }
    if venue == "all" || venue == HL {
        let coins = lab.universe.coins.iter().map(|c| (c.clone(), c.clone())).collect();
        data::fetch(http, &ar, &data::FetchPlan { venue: HL, intervals: &lab.hl_intervals, coins, start_ms, hl_api }).await?;
    }
    if !["all", BINANCE, HL].contains(&venue) {
        bail!("unknown venue `{venue}` (binance, hyperliquid or all)");
    }
    Ok(())
}

fn run_summary(spec: &Spec, run: &sim::Run, daily: &[f64], p: &Panel, lab: &LabConfig) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let n = run.trades.len();
    let bps: Vec<f64> = run.trades.iter().map(sim::Trade::bps).collect();
    let (mean, sd) = (stat::mean(&bps), stat::sd(&bps));
    let naive_t = if sd > 0.0 { mean / (sd / (n.max(1) as f64).sqrt()) } else { 0.0 };
    let b = stat::boot_mean(daily, lab.gate.block_days, lab.gate.boot_iters, lab.seed);
    let (mut cum, mut peak, mut dd) = (0.0f64, 0.0f64, 0.0f64);
    for r in daily {
        cum += r;
        peak = peak.max(cum);
        dd = dd.max(peak - cum);
    }
    let _ = writeln!(s, "spec {}", spec.id());
    let _ = writeln!(s, "trades {n} · won {} ({:.0}%) · mean {mean:+.1} bps/trade · sd {sd:.0} · trade-level t {naive_t:+.2} (assumes independent trades: it overstates)", run.wins(), if n > 0 { run.wins() as f64 / n as f64 * 100.0 } else { 0.0 });
    let _ = writeln!(s, "return {:+.2}% over {} days · max drawdown {:.2}% · fees ${:.2} · funding ${:+.2}", cum * 100.0, daily.len(), dd * 100.0, run.fees, run.funding);
    let _ = writeln!(s, "daily: mean {:+.2} bps [{:+.2}, {:+.2}] · Sharpe {:.2} · one-sided p {:.3} (day-clustered bootstrap)", b.mean * 1e4, b.lo * 1e4, b.hi * 1e4, stat::sharpe(daily) * gate::DAYS_PER_YEAR.sqrt(), b.p);
    let mut by: BTreeMap<&str, (usize, f64)> = BTreeMap::new();
    for t in &run.trades {
        let e = by.entry(match t.reason {
            sim::Reason::Time => "time stop",
            sim::Reason::Stop => "stop",
            sim::Reason::Signal => "signal",
            sim::Reason::End => "end of data",
        }).or_default();
        e.0 += 1;
        e.1 += t.bps();
    }
    for (k, (c, sum)) in by {
        let _ = writeln!(s, "  exit {k:<12} {c:>5} trades  {:+.1} bps each", sum / c as f64);
    }
    let mut coins: Vec<(f64, &str)> = run.coin_pnl.iter().zip(&p.coins).map(|(v, c)| (*v, c.as_str())).filter(|(v, _)| *v != 0.0).collect();
    coins.sort_by(|a, b| b.0.total_cmp(&a.0));
    let _ = writeln!(s, "  by coin: {}", coins.iter().map(|(v, c)| format!("{c} ${v:+.0}")).collect::<Vec<_>>().join(" · "));
    s
}

/// A spec the trader may run, with what it is expected to do. `status` says whether
/// it passed the gate ("validated") or is only being watched ("candidate").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Promoted {
    pub status: String,
    pub hypothesis: String,
    pub card: gate::Card,
}

fn promote(lab: &LabConfig, family: &Path, candidate: bool) -> Result<()> {
    let fam = Family::load(family)?;
    let study = format!("{}-{}", lab.research_venue, fam.interval);
    let path = lab.report_dir(&study).join(format!("{}.json", fam.family));
    let ev: serde_json::Value = crate::status::read_json(&path).with_context(|| format!("no verdict at {}: run `lab validate` first", path.display()))?;
    let verdict = ev["verdict"].as_str().unwrap_or("");
    let promoted = if verdict == "PROMOTE_TO_PAPER" {
        Promoted { status: "validated".into(), hypothesis: fam.hypothesis.clone(), card: serde_json::from_value(ev["card"].clone()).context("verdict file has no prediction card")? }
    } else if candidate {
        // Not validated: the card is built from the design window so the monitor can
        // still say when paper results fall outside what the backtest claimed.
        let spec: Spec = serde_json::from_value(ev["pick"].clone()).context("the verdict has no candidate spec")?;
        let full = lab.panel(&lab.research_venue, &fam.interval, &lab.universe.coins)?;
        let design = full.head(design_bars(&full, lab));
        let cfg = sim::SimCfg::new(lab.capital, lab.costs.clone(), lab.sizing.clone());
        let t = search::run_one(&design, &spec, &Aux::default(), &cfg)?;
        let first = t.daily.iter().position(|v| *v != 0.0).unwrap_or(0);
        let r = &t.daily[first..];
        let (mu, sdv) = (stat::mean(r), stat::sd(r));
        let g = &lab.gate;
        let (d1, d0) = stat::sprt_expected_days(mu.max(1e-9), sdv, g.paper_alpha, g.paper_beta);
        let dd = |len: usize| stat::boot_drawdown(r, len, g.block_days, g.boot_iters, 0.99, lab.seed ^ len as u64);
        Promoted {
            status: "candidate".into(),
            hypothesis: fam.hypothesis.clone(),
            card: gate::Card { spec, interval: fam.interval.clone(), mu_daily: mu, sd_daily: sdv, alpha: g.paper_alpha, beta: g.paper_beta, days_if_real: d1, days_if_none: d0, dd99: [dd(30), dd(60), dd(90)], issued: chrono::Utc::now(), study: study.clone() },
        }
    } else {
        bail!("{} on {study} is {verdict}, not PROMOTE_TO_PAPER. Use --candidate to paper-trade it anyway, labelled as unproven.", fam.family);
    };
    std::fs::create_dir_all("strategies")?;
    let out = PathBuf::from("strategies").join(format!("{}.json", fam.family));
    crate::status::write_atomic(&out, &serde_json::to_vec_pretty(&promoted)?)?;
    println!("{} → {} ({}): `{}`", fam.family, out.display(), promoted.status, promoted.card.spec.id());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_ids_round_trip() {
        let s = parse_spec("meanrev:hold=24,n=288,stop=500,z=2.5").unwrap();
        assert_eq!(s.family, "meanrev");
        assert_eq!(s.get("z", 0.0), 2.5);
        assert_eq!(s.id(), "meanrev:hold=24,n=288,stop=500,z=2.5");
        assert!(parse_spec("meanrev").is_err());
        assert!(parse_spec("meanrev:n=abc").is_err());
    }

    #[test]
    fn repo_lab_config_and_families_parse() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let lab = LabConfig::load(&root.join("lab.toml")).unwrap();
        assert!(lab.universe.coins.len() >= 8);
        assert!(lab.gate.rc_p_pass <= lab.gate.rc_p_fatal && lab.gate.dsr_fatal <= lab.gate.dsr_pass);
        let mut n = 0;
        for f in std::fs::read_dir(root.join("families")).unwrap().flatten() {
            let fam = Family::load(&f.path()).unwrap();
            assert!(strategy::FAMILIES.contains(&fam.family.as_str()), "{}", fam.family);
            assert!(crate::market::interval_ms(&fam.interval).is_some());
            let specs = fam.specs();
            assert!(!specs.is_empty() && specs.len() <= 600, "{}: {} specs", fam.family, specs.len());
            n += 1;
        }
        assert!(n >= 4);
    }
}
