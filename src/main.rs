mod backtest;
mod hyperliquid;
mod live;
mod bot;
mod breaker;
mod cluster;
mod decide;
mod hypotheses;
mod regime;
mod scalp;
mod report;
mod settle;
mod stats;
mod config;
mod dashboard;
mod jev;
mod market;
mod news;
mod paper;
mod signals;
mod status;
mod validate;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use clap::{Parser, Subcommand};

use config::Config;
use jev::JevClient;
use news::NewsItem;
use paper::State;
use status::Paths;

#[derive(Parser)]
#[command(about = "Crypto trading bots for Hyperliquid perps, powered by TypeSafe Jev")]
struct Cli {
    #[arg(long, default_value = "config.toml")]
    config: PathBuf,
    /// Secrets file loaded before .env.local (its values win), e.g. `.env.hl-testnet`.
    #[arg(long, global = true)]
    env_file: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run Jev on a single piece of text and print the extracted features.
    Analyze { text: String },
    /// Fetch configured news feeds and print headlines (no Jev calls).
    News,
    /// Run the news bot loop (paper unless armed).
    Run {
        /// Run a single tick and exit.
        #[arg(long)]
        once: bool,
        /// Arm real Hyperliquid orders. Needs `[exchange] mode = "live"` in the config too.
        #[arg(long)]
        live: bool,
    },
    /// Run the order-flow scalper (market data decides; Jev reviews entries against the news).
    Scalp {
        /// Arm real Hyperliquid orders (needs `[exchange] mode = "live"`; network = "testnet" trades fake money).
        #[arg(long)]
        live: bool,
    },
    /// Hyperliquid tools: market scan, account check, clearing a hold.
    Hl {
        #[command(subcommand)]
        cmd: HlCmd,
    },
    /// Show the paper account.
    Status,
    /// Score settled headlines, decisions and blocked trades (writes data/report.md
    /// and data/backlog.jsonl; suggestions only, never edits config.toml).
    Report,
    /// Replay cached Jev answers against historical klines (no Jev calls), with
    /// momentum-only / news-only / buy-and-hold baselines and a residual-edge verdict.
    Backtest {
        /// Start (YYYY-MM-DD or RFC 3339). Default: first cached event.
        #[arg(long)]
        from: Option<String>,
        /// End (YYYY-MM-DD or RFC 3339). Default: now.
        #[arg(long)]
        to: Option<String>,
        /// Output folder name under data/backtests/ (resumable).
        #[arg(long)]
        run_id: Option<String>,
        /// Review a change: `--compare base.toml candidate.toml` → PASS / REVISE / REJECT.
        #[arg(long, num_args = 2, value_names = ["BASE", "CANDIDATE"])]
        compare: Option<Vec<PathBuf>>,
    },
    /// Serve the monitoring dashboard.
    Dashboard {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8787)]
        port: u16,
        /// Scalper data directory shown on the Scalper page.
        #[arg(long, default_value = "data-hl")]
        scalp_dir: PathBuf,
    },
}

#[derive(Subcommand)]
enum HlCmd {
    /// Rank liquid perps by how often a 10-minute move clears the round-trip cost at
    /// this config's [scalp] fees (no key needed).
    Scan {
        /// How many of the highest-volume perps to test.
        #[arg(long, default_value_t = 30)]
        top: usize,
        /// Skip perps with less 24h volume than this, millions of dollars.
        #[arg(long, default_value_t = 5.0)]
        min_volume: f64,
        /// Hours of 1-minute candles (max 83).
        #[arg(long, default_value_t = 72)]
        hours: i64,
    },
    /// Check the account address, agent key, margin, coins and leverage of this
    /// config's [exchange]. Sends no orders.
    Check,
    /// Clear the hold set after an unconfirmed order, once you've checked
    /// Hyperliquid and the live ledger agree.
    ClearHold,
}


#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    // Secrets: real environment wins, then --env-file, then .env.local, then .env (all git-ignored).
    if let Some(f) = &cli.env_file {
        dotenvy::from_filename(f).map_err(|e| anyhow::anyhow!("reading {}: {e}", f.display()))?;
    }
    let _ = dotenvy::from_filename(".env.local");
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = Config::load(&cli.config)?;
    std::fs::create_dir_all(&cfg.data_dir)?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("jev-trader/0.1")
        .build()?;

    match cli.cmd {
        Cmd::News => {
            for item in bot::fetch_all_news(&http, &cfg).await {
                let when = item.published.map(|p| p.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|| "undated         ".into());
                println!("{when}  [{}]  {}", item.source, item.title);
            }
        }
        Cmd::Status => print_status(&http, &cfg).await?,
        Cmd::Analyze { text } => {
            let jev = jev_client(&cfg)?;
            let item = NewsItem {
                id: "cli".into(),
                title: text,
                summary: String::new(),
                source: "cli".into(),
                published: Some(Utc::now()),
            };
            let t0 = std::time::Instant::now();
            let ev = news::analyze(&jev, &item, &cfg.assets, Utc::now()).await?;
            println!("{}", serde_json::to_string_pretty(&ev)?);
            eprintln!("latency: {:?}", t0.elapsed());
        }
        Cmd::Run { once, live } => {
            let paths = Paths::new(&cfg.data_dir);
            let live = arm(&cfg, live).await?;
            let ctx = bot::Ctx { jev: jev_client(&cfg)?, cfg, http, paths, live };
            bot::run(ctx, once).await?;
        }
        Cmd::Scalp { live } => {
            let live = arm(&cfg, live).await?;
            scalp::run(cfg.clone(), jev_client(&cfg)?, live).await?;
        }
        Cmd::Hl { cmd } => match cmd {
            HlCmd::Scan { top, min_volume, hours } => println!("{}", hyperliquid::scan(&http, cfg.api(), &cfg.scalp, top, min_volume, hours.clamp(1, 83)).await?),
            HlCmd::Check => live::check(&cfg).await?,
            HlCmd::ClearHold => live::clear_hold(&cfg)?,
        },
        Cmd::Report => {
            let md = run_report(&cfg, &cli.config)?;
            println!("{md}");
        }
        Cmd::Backtest { from, to, run_id, compare } => match compare {
            Some(p) => {
                let (verdict, md) = backtest::compare(&http, &p[0], &p[1], from.as_deref(), to.as_deref()).await?;
                println!("{md}");
                eprintln!("verdict: {verdict}");
            }
            None => {
                let b = backtest::run(&http, &cfg, from.as_deref(), to.as_deref(), run_id).await?;
                println!("{}", b.markdown);
                eprintln!("verdict: {} · written to {}", b.verdict, b.dir.display());
            }
        },
        Cmd::Dashboard { host, port, scalp_dir } => dashboard::serve(cfg, host, port, scalp_dir).await?,
    }
    Ok(())
}

/// Real orders need both the config (`mode = "live"`) and the `--live` flag.
async fn arm(cfg: &Config, live: bool) -> Result<Option<live::Live>> {
    match (cfg.exchange.mode, live) {
        (config::ExecMode::Live, true) => Ok(Some(live::Live::connect(cfg).await?)),
        (config::ExecMode::Paper, false) => Ok(None),
        (config::ExecMode::Live, false) => anyhow::bail!("this config sends real orders ([exchange] mode = \"live\"); add --live to confirm"),
        (config::ExecMode::Paper, true) => anyhow::bail!("--live given but the config is paper; set [exchange] mode = \"live\" in a separate config"),
    }
}

fn run_report(cfg: &Config, config_path: &std::path::Path) -> Result<String> {
    let paths = Paths::new(&cfg.data_dir);
    let settled: Vec<settle::Settlement> = status::read_jsonl(&paths.settlements);
    let sc = report::scorecard(&settled, cfg, "paper · settled");
    let backlog = report::backlog(&sc);
    let hyp_path = config_path.parent().unwrap_or(std::path::Path::new(".")).join("hypotheses.toml");
    let hyps = hypotheses::load(&hyp_path)?;
    let reviews: Vec<report::ReviewRecord> = status::read_jsonl(&paths.reviews);
    let hash = backtest::config_hash(config_path)?;
    let state = State::load_or_new(&paths.state, cfg.paper.starting_cash)?;
    let tokens = (state.tokens.calls, state.tokens.input, state.tokens.output);
    let md = report::markdown(&sc, &backlog, &hyps, &reviews, &hash, tokens);
    status::write_atomic(&paths.report, md.as_bytes())?;
    let lines: String = backlog.iter().filter_map(|b| serde_json::to_string(b).ok()).map(|l| l + "\n").collect();
    status::write_atomic(&paths.backlog, lines.as_bytes())?;
    Ok(md)
}

fn jev_client(cfg: &Config) -> Result<JevClient> {
    JevClient::from_env(&cfg.jev.model, Duration::from_millis(cfg.jev.timeout_ms))
}

async fn print_status(http: &reqwest::Client, cfg: &Config) -> Result<()> {
    let state = State::load_or_new(&Paths::new(&cfg.data_dir).state, cfg.paper.starting_cash)?;
    let start = state.starting_cash.unwrap_or(cfg.paper.starting_cash);
    if state.mode.as_deref() == Some(live::MODE) {
        println!("LIVE ledger ({}){}", state.venue.as_deref().unwrap_or("unknown venue"), state.live_hold.as_ref().map(|h| format!(" · HOLD: {h}")).unwrap_or_default());
    }
    let mut prices = HashMap::new();
    for a in &cfg.assets {
        if let Ok(c) = market::klines(http, cfg.api(), &a.coin, &cfg.market.interval, 1).await
            && validate::candles(&c).is_ok()
            && let Some(last) = c.last()
        {
            prices.insert(a.symbol.clone(), last.close);
        }
    }
    let verified = state.equity_verified(&prices);
    let equity = state.equity(&prices);
    if verified.is_none() {
        println!("REVIEW: a held position has no price; equity below values it at entry");
    }
    println!("equity  ${equity:.2}  (start ${start:.2}, {:+.2}%)", (equity / start - 1.0) * 100.0);
    println!("cash    ${:.2}", state.cash);
    if state.funding_usd != 0.0 {
        println!("funding ${:+.2} since start", state.funding_usd);
    }
    for (sym, p) in &state.positions {
        let px = prices.get(sym).copied().unwrap_or(p.entry);
        println!(
            "{sym:<6} qty {:.6}  entry {:.2}  now {:.2}  pnl {:+.2}%  weight {:.1}%",
            p.qty,
            p.entry,
            px,
            (px / p.entry - 1.0) * 100.0,
            p.qty * px / equity * 100.0
        );
    }
    let brk = breaker::update(&mut state.clone(), verified, Utc::now(), &cfg.risk);
    if brk.blocks_entries() {
        println!("breaker: {:?} · {}{}", brk.state, brk.reason, brk.active_until.map(|u| format!(" until {u}")).unwrap_or_default());
    }
    if !state.pending.is_empty() {
        println!("{} headlines waiting for a Jev retry", state.pending.len());
    }
    println!("Jev today: {} calls · {}/{} tokens", state.tokens.calls, state.tokens.input, state.tokens.output);
    println!("{} news events in memory", state.events.len());
    Ok(())
}
