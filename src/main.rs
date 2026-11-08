mod config;
mod dashboard;
mod engine;
mod features;
mod feed;
mod hyperliquid;
mod intel;
mod jev;
mod ledger;
mod live;
mod market;
mod report;
mod status;
mod strategy;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};

use config::Config;

#[derive(Parser)]
#[command(about = "jev-trader v2: one engine on Hyperliquid perps where Jev's news reading and order flow must agree")]
struct Cli {
    #[arg(long, global = true, default_value = "config.toml")]
    config: PathBuf,
    /// Secrets file loaded before .env.local (its values win), e.g. `.env.hl-testnet`.
    #[arg(long, global = true)]
    env_file: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the engine (paper unless armed).
    Run {
        /// Arm real orders. Needs `[exchange] mode = "live"` in the config too.
        #[arg(long)]
        live: bool,
    },
    /// Fetch every news source once and show how each headline would be handled
    /// (coin tag, noise, duplicate). No Jev calls.
    News {
        /// Also print the items that were filtered out.
        #[arg(long)]
        all: bool,
    },
    /// Score one headline with Jev, the way the engine does.
    Analyze { text: String },
    /// Score the engine's logs: trades, headline edge, flow vs news, Jev spend
    /// (writes <data_dir>/report.md).
    Report,
    /// Serve the monitoring dashboard.
    Dashboard {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8787)]
        port: u16,
    },
    /// Hyperliquid tools.
    Hl {
        #[command(subcommand)]
        cmd: HlCmd,
    },
}

#[derive(Subcommand)]
enum HlCmd {
    /// Rank liquid perps by how often a move over the holding horizon clears the cost.
    Scan {
        #[arg(long, default_value_t = 30)]
        top: usize,
        /// Minimum 24h volume, $M.
        #[arg(long, default_value_t = 10.0)]
        min_volume: f64,
        /// Hours of 1-minute candles (max 83).
        #[arg(long, default_value_t = 72)]
        hours: i64,
        /// Window in minutes (catalyst trades hold up to hours; flow 30 min).
        #[arg(long, default_value_t = 30)]
        window: usize,
        /// Hurdle as a multiple of the round-trip cost.
        #[arg(long, default_value_t = 3.0)]
        mult: f64,
    },
    /// Check keys, agent approval, margin and coins. Sends no orders.
    Check,
    /// Clear the hold set after an unconfirmed order, once you've checked Hyperliquid.
    ClearHold,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(f) = &cli.env_file {
        dotenvy::from_filename(f).map_err(|e| anyhow::anyhow!("reading {}: {e}", f.display()))?;
    }
    let _ = dotenvy::from_filename(".env.local");
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())).init();

    let cfg = Config::load(&cli.config)?;
    let http = reqwest::Client::builder().timeout(Duration::from_secs(15)).user_agent("Mozilla/5.0 (jev-trader)").build()?;
    match cli.cmd {
        Cmd::Run { live } => {
            let live = match (cfg.exchange.mode, live) {
                (config::ExecMode::Live, true) => Some(live::Live::connect(&cfg).await?),
                (config::ExecMode::Paper, false) => None,
                (config::ExecMode::Live, false) => anyhow::bail!("this config sends real orders ([exchange] mode = \"live\"); add --live to confirm"),
                (config::ExecMode::Paper, true) => anyhow::bail!("--live given but the config is paper; use a live config (see config.live.toml)"),
            };
            engine::run(cfg.clone(), jev_client(&cfg)?, live).await?;
        }
        Cmd::News { all } => news_dry_run(&http, &cfg, all).await?,
        Cmd::Analyze { text } => {
            let files = status::Files::new(&std::env::temp_dir());
            let gate = intel::JevGate::new(jev_client(&cfg)?, 10, &files);
            let names: Vec<String> = hyperliquid::perp_universe(&http, cfg.api()).await.map(|p| p.into_iter().map(|x| x.coin).collect()).unwrap_or_else(|_| cfg.watch.clone());
            let tag = intel::tagger::Tagger::new(&names).tag(&text, "");
            println!("local tag: {tag:?}");
            let mut q = indexmap::IndexMap::new();
            q.insert("s0".to_string(), jev::Question::score("h0: effect on the tagged asset's price over the next few hours", &["Strong drop", "Drop", "No effect", "Rise", "Strong rise"]));
            q.insert("n0".to_string(), jev::Question::noul("h0 is a new, concrete, market-moving fact (not opinion, analysis, recap, or a report of price moves)"));
            let subject = tag.coins.first().cloned().unwrap_or_else(|| "crypto market".into());
            let state = serde_json::json!({ "task": "Each item is a crypto news headline tagged with the asset it concerns", "items": { "h0": format!("[{subject}] {text}") } });
            let r = gate.ask("cli", 1, &state, &q).await?;
            let (s, conf) = r.score_expected("s0")?;
            println!("impact {:+.2} (conf {conf:.2}) · new fact {:.2} · tokens {:?}", s * 2.0 - 1.0, r.noul("n0")?, r.usage.map(|u| (u.input_tokens, u.output_tokens)));
        }
        Cmd::Report => {
            let sc = report::scorecard(&cfg.data_dir, cfg.catalyst.min_impact);
            let md = report::markdown(&sc);
            std::fs::create_dir_all(&cfg.data_dir)?;
            status::write_atomic(&status::Files::new(&cfg.data_dir).report, md.as_bytes())?;
            println!("{md}");
        }
        Cmd::Dashboard { host, port } => dashboard::serve(cfg, host, port).await?,
        Cmd::Hl { cmd } => match cmd {
            HlCmd::Scan { top, min_volume, hours, window, mult } => println!("{}", hyperliquid::scan(&http, cfg.api(), &cfg.costs, top, min_volume, hours.clamp(1, 83), window, mult).await?),
            HlCmd::Check => live::check(&cfg).await?,
            HlCmd::ClearHold => live::clear_hold(&cfg)?,
        },
    }
    Ok(())
}

fn jev_client(cfg: &Config) -> Result<jev::JevClient> {
    jev::JevClient::from_env(&cfg.jev.model, Duration::from_millis(cfg.jev.timeout_ms))
}

/// `news`: every source once, through the local tagger. Shows what Jev would see.
async fn news_dry_run(http: &reqwest::Client, cfg: &Config, all: bool) -> Result<()> {
    let names: Vec<String> = hyperliquid::perp_universe(http, cfg.api()).await.map(|p| p.into_iter().map(|x| x.coin).collect()).unwrap_or_else(|_| cfg.watch.clone());
    let tagger = Arc::new(intel::tagger::Tagger::new(&names));
    let now = chrono::Utc::now();
    let max_age = chrono::Duration::minutes(cfg.news.max_age_min);
    let mut fetched = Vec::new();
    for v in &cfg.news.announcements {
        fetched.push((v.clone(), intel::sources::fetch_announcements(http, v).await));
    }
    for u in &cfg.news.rss {
        fetched.push((intel::sources::source_name(u), intel::sources::fetch_rss(http, u).await));
    }
    let (mut fresh, mut to_jev) = (0, 0);
    for (name, r) in fetched {
        match r {
            Err(e) => println!("!! {name}: {e:#}"),
            Ok(items) => {
                let recent: Vec<_> = items.into_iter().filter(|i| i.published.is_some_and(|p| now - p < max_age)).collect();
                println!("== {name}: {} fresh", recent.len());
                for i in recent {
                    fresh += 1;
                    let t = tagger.tag(&i.title, &i.summary);
                    let verdict = if t.noise && i.kind == intel::sources::Kind::News {
                        "noise"
                    } else if t.recap && i.kind == intel::sources::Kind::News {
                        "recap"
                    } else if t.coins.is_empty() && !t.market && i.kind != intel::sources::Kind::Official {
                        "untradable"
                    } else {
                        to_jev += 1;
                        "→ Jev"
                    };
                    if all || verdict == "→ Jev" {
                        let age = i.published.map(|p| (now - p).num_minutes()).unwrap_or(0);
                        println!("  {verdict:<10} {age:>4}m  [{}{}] {}", t.coins.join(","), if t.market { " mkt" } else { "" }, i.title);
                    }
                }
            }
        }
    }
    println!("\n{fresh} fresh items in the last {} min; {to_jev} would go to Jev (before duplicate removal), at about 180 tokens each in batches.", cfg.news.max_age_min);
    Ok(())
}
