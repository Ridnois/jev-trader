//! One config file per ledger. Every knob has a default, so a config only lists
//! what it changes. See `config.toml` for the documented paper setup.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub use crate::hyperliquid::Network;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    /// Ledger, logs and status for this config. One process per folder.
    pub data_dir: PathBuf,
    /// Coins always streamed (order flow, funding). Catalyst trades may also open on
    /// any other liquid Hyperliquid perp named by fresh news.
    pub watch: Vec<String>,
    pub exchange: ExchangeConfig,
    pub jev: JevConfig,
    pub news: NewsConfig,
    pub costs: Costs,
    pub risk: RiskConfig,
    pub catalyst: CatalystConfig,
    pub flow: FlowConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: "data-v2".into(),
            watch: ["BTC", "ETH", "SOL", "HYPE", "XRP", "ZEC", "SUI", "DOGE", "ENA", "kPEPE"].iter().map(|s| s.to_string()).collect(),
            exchange: ExchangeConfig::default(),
            jev: JevConfig::default(),
            news: NewsConfig::default(),
            costs: Costs::default(),
            risk: RiskConfig::default(),
            catalyst: CatalystConfig::default(),
            flow: FlowConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecMode {
    /// Simulated fills on the real Hyperliquid feed, charged real fees (default).
    #[default]
    Paper,
    /// Real orders signed by an API agent wallet. Also needs `run --live`.
    Live,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ExchangeConfig {
    pub mode: ExecMode,
    pub network: Network,
    pub account_env: String,
    pub agent_key_env: String,
    /// Live: the most USDC the bot may manage. 0 refuses to start (you choose it).
    pub max_capital_usd: f64,
    /// IOC orders sit at most this far through the reference price.
    pub max_slippage_bps: f64,
    /// Cross leverage set on each traded coin (the ledger sizes at ≤ 1× notional).
    pub leverage: u32,
    /// Live: rest a reduce-only stop-market on the exchange, wider than the bot's own
    /// stop, so a position is protected if the process dies.
    pub backstop_stop: bool,
    /// Backstop distance as a multiple of the bot's stop distance.
    pub backstop_mult: f64,
}

impl Default for ExchangeConfig {
    fn default() -> Self {
        Self {
            mode: ExecMode::Paper,
            network: Network::Mainnet,
            account_env: "HL_ACCOUNT_ADDRESS".into(),
            agent_key_env: "HL_AGENT_PRIVATE_KEY".into(),
            max_capital_usd: 0.0,
            max_slippage_bps: 30.0,
            leverage: 1,
            backstop_stop: true,
            backstop_mult: 1.5,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct JevConfig {
    pub model: String,
    pub timeout_ms: u64,
    /// Headlines per Jev call. Each call carries ~270 tokens of fixed overhead, so
    /// batching is the main token saving (measured 2026-11-08: 178 vs 751 tokens per headline).
    pub batch_size: usize,
    /// Hard ceiling on Jev calls per hour (headline batches + trade reviews).
    pub max_calls_per_hour: u32,
    /// Attempts per headline before it is dropped.
    pub max_attempts: u32,
}

impl Default for JevConfig {
    fn default() -> Self {
        Self { model: "jev-latest".into(), timeout_ms: 10_000, batch_size: 10, max_calls_per_hour: 120, max_attempts: 3 }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct NewsConfig {
    pub rss: Vec<String>,
    /// Exchange listing/delisting announcement APIs: "binance", "upbit", "bybit", "okx".
    pub announcements: Vec<String>,
    pub rss_every_secs: u64,
    pub announcements_every_secs: u64,
    /// Items published longer ago than this are ignored on first sight.
    pub max_age_min: i64,
    /// A headline's influence on a coin's bias halves every this many minutes.
    pub half_life_min: f64,
    pub retain_hours: i64,
    /// Title-token Jaccard at or above this = same story (not sent to Jev again).
    pub dedupe_jaccard: f64,
    /// Catalyst trades on unwatched coins need at least this 24h volume, $M.
    pub min_volume_musd: f64,
}

impl Default for NewsConfig {
    fn default() -> Self {
        Self {
            rss: DEFAULT_RSS.iter().map(|s| s.to_string()).collect(),
            announcements: ["binance", "upbit", "bybit", "okx"].iter().map(|s| s.to_string()).collect(),
            rss_every_secs: 60,
            announcements_every_secs: 20,
            max_age_min: 180,
            half_life_min: 60.0,
            retain_hours: 24,
            dedupe_jaccard: 0.5,
            min_volume_musd: 10.0,
        }
    }
}

/// Checked 2026-11-08: all return 200 without a key.
pub const DEFAULT_RSS: &[&str] = &[
    "https://www.coindesk.com/arc/outboundfeeds/rss",
    "https://cointelegraph.com/rss",
    "https://decrypt.co/feed",
    "https://www.theblock.co/rss.xml",
    "https://blockworks.com/feed",
    "https://www.dlnews.com/arc/outboundfeeds/rss/",
    "https://thedefiant.io/api/feed",
    "https://bitcoinmagazine.com/feed",
    "https://cryptoslate.com/feed/",
    "https://crypto.news/feed/",
    "https://beincrypto.com/feed/",
    "https://cryptonews.com/news/feed/",
    "https://www.newsbtc.com/feed/",
    "https://bitcoinist.com/feed/",
    "https://cryptopotato.com/feed/",
    "https://news.google.com/rss/search?q=crypto+OR+bitcoin+when:1h&hl=en-US&gl=US&ceid=US:en",
    "https://www.federalreserve.gov/feeds/press_all.xml",
    "https://www.sec.gov/news/pressreleases.rss",
];

/// Hyperliquid tier-0 fees (hyperliquid.gitbook.io/hyperliquid-docs/trading/fees, 2026-11-06).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Costs {
    pub taker_bps: f64,
    pub maker_bps: f64,
    pub slippage_bps: f64,
}

impl Default for Costs {
    fn default() -> Self {
        Self { taker_bps: 4.5, maker_bps: 1.5, slippage_bps: 1.0 }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RiskConfig {
    /// Paper ledger starting cash (live uses `max_capital_usd`).
    pub starting_cash: f64,
    pub max_positions: usize,
    /// Max notional per coin, fraction of capital.
    pub max_coin_pct: f64,
    /// Max total notional, fraction of capital (1.0 = 1×).
    pub max_gross: f64,
    /// Trade P&L below −this × capital in a UTC day stops entries until tomorrow.
    pub daily_loss_pct: f64,
    /// No re-entry on a coin for this long after a losing trade.
    pub loss_cooldown_min: i64,
    pub max_entries_per_hour: usize,
    pub min_trade_usd: f64,
    /// Skip longs when hourly funding is above this (crowded); shorts when below −this. bps.
    pub max_funding_bps_1h: f64,
    /// Seconds without a book update before a coin is considered stale.
    pub feed_stale_ms: i64,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            starting_cash: 10_000.0,
            max_positions: 3,
            max_coin_pct: 0.35,
            max_gross: 1.0,
            daily_loss_pct: 0.02,
            loss_cooldown_min: 30,
            max_entries_per_hour: 4,
            min_trade_usd: 11.0,
            max_funding_bps_1h: 0.5,
            feed_stale_ms: 5_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EntryMode {
    /// Cross the spread now (IOC).
    #[default]
    Taker,
    /// Post at the touch (ALO) and wait for a trade-through.
    Maker,
}

/// News-led trades: a fresh, concrete, strong headline, confirmed by order flow.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct CatalystConfig {
    pub enabled: bool,
    /// |impact| (−1..1) needed.
    pub min_impact: f64,
    /// Jev's "new concrete market-moving fact" probability needed.
    pub min_new_info: f64,
    /// Seen within this many minutes of publication (slow outlets are late news).
    pub max_lag_min: f64,
    /// How long to wait for order flow to confirm the headline.
    pub confirm_secs: i64,
    /// Signed trade-flow imbalance needed to confirm (−1..1).
    pub confirm_flow: f64,
    /// Skip if price already moved more than this × ATR(1h) in the news direction since publication.
    pub max_chase_atr: f64,
    /// Stop distance = this × ATR(1h), at least `min_stop_cost_mult` × round-trip cost.
    pub stop_atr: f64,
    pub min_stop_cost_mult: f64,
    /// Take profit at this many stop distances (R).
    pub tp_r: f64,
    /// Once +trail_r R in profit: stop to break-even after costs, then trail.
    pub trail_r: f64,
    pub hold_min: i64,
    /// Equity risked per trade (before Jev's size multiplier).
    pub risk_pct: f64,
    pub entry: EntryMode,
    /// One Jev call per entry: "already priced in?" / "does the move continue?".
    pub review: bool,
    /// Halve size when priced_in is above this; skip when above `review_veto`.
    pub review_halve: f64,
    pub review_veto: f64,
}

impl Default for CatalystConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_impact: 0.45,
            min_new_info: 0.6,
            max_lag_min: 20.0,
            confirm_secs: 600,
            confirm_flow: 0.15,
            max_chase_atr: 1.0,
            stop_atr: 1.0,
            min_stop_cost_mult: 3.0,
            tp_r: 2.0,
            trail_r: 1.0,
            hold_min: 240,
            risk_pct: 0.005,
            entry: EntryMode::Taker,
            review: true,
            review_halve: 0.6,
            review_veto: 0.85,
        }
    }
}

/// Flow-led trades: the v1 scalper's microstructure gates, allowed only when the
/// coin's news bias points the same way (v1 data: flow alone lost money before fees).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct FlowConfig {
    pub enabled: bool,
    /// Signed news bias (−1..1) needed in the trade's direction.
    pub min_bias: f64,
    pub flow_window_secs: i64,
    pub vol_window_secs: i64,
    pub baseline_secs: i64,
    pub warmup_secs: i64,
    pub min_activity_ratio: f64,
    pub min_flow_imbalance: f64,
    pub min_book_imbalance: f64,
    pub min_push_bps: f64,
    pub max_chase_bps: f64,
    pub max_spread_bps: f64,
    /// Time stop, seconds. Also the horizon of the expected-move estimate.
    pub hold_secs: i64,
    /// Expected move over the hold must be ≥ this × round-trip cost.
    pub cost_mult: f64,
    /// Stop = max(stop_vol_mult × expected move, min_stop_cost_mult × cost).
    pub stop_vol_mult: f64,
    pub min_stop_cost_mult: f64,
    pub tp_r: f64,
    pub trail_r: f64,
    pub risk_pct: f64,
    pub entry: EntryMode,
    pub maker_timeout_secs: i64,
    /// Markout horizons for every flow candidate, seconds.
    pub markouts: Vec<i64>,
    pub eval_ms: u64,
}

impl Default for FlowConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_bias: 0.15,
            flow_window_secs: 30,
            vol_window_secs: 300,
            baseline_secs: 1800,
            warmup_secs: 600,
            min_activity_ratio: 1.5,
            min_flow_imbalance: 0.25,
            min_book_imbalance: 0.10,
            min_push_bps: 3.0,
            max_chase_bps: 60.0,
            max_spread_bps: 5.0,
            hold_secs: 1800,
            cost_mult: 3.0,
            stop_vol_mult: 1.0,
            min_stop_cost_mult: 2.5,
            tp_r: 2.0,
            trail_r: 1.0,
            risk_pct: 0.0025,
            entry: EntryMode::Maker,
            maker_timeout_secs: 20,
            markouts: vec![60, 300, 1800],
            eval_ms: 500,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let v: toml::Value = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        // v1 configs (news bot / scalper) have sections this version doesn't read.
        for old in ["scalp", "strategy", "paper", "market", "regime", "challenge", "settle", "assets"] {
            if v.get(old).is_some() {
                bail!("{} is a v1 config ([{old}]); v2 uses one combined engine, see config.toml", path.display());
            }
        }
        let cfg: Config = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        if cfg.watch.is_empty() {
            bail!("watch is empty");
        }
        for c in &cfg.watch {
            if c.ends_with("USDT") || c.ends_with("USDC") {
                bail!("watch coin \"{c}\" looks like a Binance pair; use the Hyperliquid perp name (e.g. \"BTC\")");
            }
        }
        Ok(cfg)
    }

    pub fn api(&self) -> &'static str {
        self.exchange.network.api_url()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_uses_defaults_and_v1_is_refused() {
        let dir = std::env::temp_dir().join(format!("jev-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("c.toml");
        std::fs::write(&p, "data_dir = \"x\"\n").unwrap();
        let c = Config::load(&p).unwrap();
        assert_eq!(c.costs.taker_bps, 4.5);
        assert!(c.watch.contains(&"BTC".to_string()));
        std::fs::write(&p, "[scalp]\neval_ms = 1\n").unwrap();
        assert!(Config::load(&p).is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}
