use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub use crate::hyperliquid::Network;

/// A config that still points at Binance must not load: its `[exchange]` keys
/// would otherwise be silently ignored and fall back to Hyperliquid defaults.
fn reject_binance(text: &str) -> Result<()> {
    let v: toml::Value = toml::from_str(text)?;
    let get = |t: &str, k: &str| v.get(t).and_then(|x| x.get(k));
    let text_of = |t: &str, k: &str| get(t, k).and_then(|x| x.as_str()).unwrap_or_default().to_lowercase();
    // Keys that only ever existed for Binance.
    for (t, k) in [("exchange", "base_url"), ("exchange", "quote"), ("market", "futures_url"), ("scalp", "stream_url"), ("scalp", "inventory_usd")] {
        if get(t, k).is_some() {
            anyhow::bail!("[{t}] {k} is a Binance setting");
        }
    }
    if text_of("market", "base_url").contains("binance") || text_of("scalp", "feed") == "binance" {
        anyhow::bail!("market data points at Binance");
    }
    if get("scalp", "perps").and_then(|x| x.as_bool()) == Some(false) {
        anyhow::bail!("[scalp] perps = false (spot)");
    }
    Ok(())
}

/// Every knob added by the optimization plan has a default that reproduces the
/// original strategy, so an old `config.toml` behaves exactly as before.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub loop_interval_secs: u64,
    pub data_dir: PathBuf,
    pub jev: JevConfig,
    pub market: MarketConfig,
    pub news: NewsConfig,
    pub strategy: StrategyConfig,
    pub risk: RiskConfig,
    pub paper: PaperConfig,
    #[serde(default)]
    pub regime: RegimeConfig,
    #[serde(default)]
    pub challenge: ChallengeConfig,
    #[serde(default)]
    pub settle: SettleConfig,
    #[serde(default)]
    pub exchange: ExchangeConfig,
    #[serde(default)]
    pub scalp: ScalpConfig,
    pub assets: Vec<Asset>,
}

/// The order-flow scalper (`jev-trader scalp`) on Hyperliquid perps. Market
/// microstructure decides; Jev reviews entries against the news; the news context
/// only tilts and vetoes. Shorts are plain perp sells, 1× notional, funding accrued.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ScalpConfig {
    /// How often the engine evaluates, in milliseconds.
    pub eval_ms: u64,
    /// Window for trade-flow imbalance and activity, seconds.
    pub flow_window_secs: i64,
    /// Window for realized volatility, seconds.
    pub vol_window_secs: i64,
    /// Baseline for "normal" activity, seconds (also the warm-up).
    pub baseline_secs: i64,
    /// Seconds of history needed before trading.
    pub warmup_secs: i64,
    /// Trades/sec over the flow window ÷ baseline trades/sec.
    pub min_activity_ratio: f64,
    /// (aggressive buy − sell) / total, over the flow window.
    pub min_flow_imbalance: f64,
    /// Top-of-book (bid qty − ask qty) / total, smoothed.
    pub min_book_imbalance: f64,
    /// Mid must have risen this much over the last 60s, bps.
    pub min_push_bps: f64,
    /// Skip if mid already rose more than this over 5 minutes, bps (no chasing).
    pub max_chase_bps: f64,
    pub max_spread_bps: f64,
    /// Holding period limit (time stop), seconds.
    pub hold_secs: i64,
    /// Cost model, per side: taker fee and slippage, bps. Paper fills are charged
    /// these, so results are not flattered.
    pub fee_bps: f64,
    pub slippage_bps: f64,
    /// Expected move over the hold (σ₁ₛ·√hold) must be ≥ this × round-trip cost.
    pub cost_mult: f64,
    /// Take profit = max(tp_vol_mult × expected move, tp_cost_mult × cost).
    pub tp_vol_mult: f64,
    pub tp_cost_mult: f64,
    /// Stop = max(sl_vol_mult × expected move, sl_cost_mult × cost).
    pub sl_vol_mult: f64,
    pub sl_cost_mult: f64,
    /// Fraction of equity per trade (before Jev's size multiplier).
    pub position_pct: f64,
    pub max_positions: usize,
    pub max_trades_per_hour: usize,
    pub daily_loss_limit_pct: f64,
    pub cooldown_after_loss_secs: i64,
    pub min_trade_usd: f64,
    /// No entries when the newest book update is older than this.
    pub feed_stale_ms: i64,
    /// Paper ledger starting cash (live uses `[exchange] max_capital_usd`).
    pub starting_cash: f64,
    /// News tone shifts the flow threshold by up to this much (score −1..1).
    pub news_tilt: f64,
    /// Jev entry review.
    pub jev_review: bool,
    /// Block when adverse-news or event-risk probability is above this.
    pub jev_veto: f64,
    /// Reuse a review for this long unless a new headline arrives, seconds.
    pub jev_cache_secs: i64,
    /// Headlines older than this are not shown to the review, minutes.
    pub jev_headline_minutes: i64,
    /// Markout horizons for every candidate, seconds.
    pub markouts: Vec<i64>,
    /// Also open shorts (perp sells), judged by the same mirrored gates.
    pub shorts: bool,
    /// Once a trade is this many round-trip costs in profit, the stop moves to
    /// break-even after fees and trails the best price. 0 = off.
    pub trail_after_cost_mult: f64,
    /// Fee for a resting (post-only) order, bps. `fee_bps` is the taker fee.
    pub maker_fee_bps: f64,
    /// "taker": cross the spread at once. "maker": post at the best bid (long) or
    /// ask (short) and wait up to `maker_timeout_secs` for the market to trade
    /// through it; unfilled orders are cancelled and logged as misses.
    pub entry: EntryMode,
    pub maker_timeout_secs: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EntryMode {
    Taker,
    #[default]
    Maker,
}

impl Default for ScalpConfig {
    fn default() -> Self {
        Self {
            eval_ms: 500,
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
            hold_secs: 600,
            fee_bps: 4.5,
            slippage_bps: 1.0,
            cost_mult: 2.0,
            tp_vol_mult: 1.5,
            tp_cost_mult: 1.5,
            sl_vol_mult: 1.0,
            sl_cost_mult: 1.0,
            position_pct: 0.10,
            max_positions: 2,
            max_trades_per_hour: 6,
            daily_loss_limit_pct: 0.02,
            cooldown_after_loss_secs: 900,
            min_trade_usd: 10.0,
            feed_stale_ms: 3000,
            starting_cash: 10_000.0,
            news_tilt: 0.10,
            jev_review: true,
            jev_veto: 0.6,
            jev_cache_secs: 120,
            jev_headline_minutes: 180,
            markouts: vec![60, 300, 600],
            shorts: true,
            trail_after_cost_mult: 1.5,
            maker_fee_bps: 1.5,
            entry: EntryMode::Maker,
            maker_timeout_secs: 20,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecMode {
    /// Simulated fills on real Hyperliquid prices (the default).
    #[default]
    Paper,
    /// Real Hyperliquid orders, signed by an API agent wallet. Also needs `--live`.
    Live,
}

/// Where fills happen and which Hyperliquid chain supplies prices. Paper unless
/// explicitly set to live.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ExchangeConfig {
    pub mode: ExecMode,
    /// "mainnet" or "testnet". Market data, the scalper's feed and orders all use it.
    pub network: Network,
    /// Environment variable holding the main account address (public).
    pub account_env: String,
    /// Environment variable holding the API agent wallet's private key. Approve the
    /// agent at app.hyperliquid.xyz/API: it can trade but cannot withdraw.
    pub agent_key_env: String,
    /// Live: the most USDC the bot may manage. The ledger starts with
    /// min(this, account value) and never knows about the rest. 0 = refuse to start.
    pub max_capital_usd: f64,
    /// Marketable orders are IOC limits at most this far through the reference price.
    pub max_slippage_bps: f64,
    /// Cross leverage set on every configured coin at start (0 = leave it alone).
    /// The ledgers size at 1× notional; 1 also makes the exchange enforce that.
    pub leverage: u32,
}

impl ExchangeConfig {
    pub fn is_testnet(&self) -> bool {
        self.network == Network::Testnet
    }
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
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct JevConfig {
    pub model: String,
    pub concurrency: usize,
    pub timeout_ms: u64,
    /// Attempts per headline before it is spiked as `jev_failed` (6.1).
    #[serde(default = "d_max_attempts")]
    pub max_attempts: u32,
    /// Jev circuit breaker: look back over this many ticks (6.2).
    #[serde(default = "d_breaker_ticks")]
    pub breaker_ticks: usize,
    /// ... and block entries when more than this share of calls failed.
    #[serde(default = "d_breaker_fail_ratio")]
    pub breaker_fail_ratio: f64,
    /// ... once at least this many calls were made in the window.
    #[serde(default = "d_breaker_min_calls")]
    pub breaker_min_calls: u32,
}

/// Candles for the news bot (from Hyperliquid on `[exchange] network`).
#[derive(Debug, Clone, Deserialize)]
pub struct MarketConfig {
    pub interval: String,
    pub momentum_window: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewsConfig {
    pub feeds: Vec<String>,
    /// Ignore items older than this when first seen.
    pub max_age_hours: f64,
    /// Influence of a news event halves every `half_life_hours`.
    pub half_life_hours: f64,
    /// Drop stored events older than this.
    pub retain_hours: f64,
    /// 5.1: count a cluster of same-story headlines once (false = original sum over outlets).
    #[serde(default)]
    pub dedupe_clusters: bool,
    /// 5.1: ask Jev about borderline duplicates (Jaccard between the two thresholds).
    #[serde(default = "d_true")]
    pub cluster_jev_check: bool,
    /// 5.5: multiplier per event type / source title (missing = 1.0).
    #[serde(default)]
    pub event_weight: BTreeMap<String, f64>,
    #[serde(default)]
    pub source_weight: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StrategyConfig {
    /// News score (-1..1) required to enter a long.
    pub news_entry: f64,
    /// News score below -news_exit forces an exit.
    pub news_exit: f64,
    /// 5.4: positive independent clusters required before an entry (1 = original).
    #[serde(default = "d_one")]
    pub min_clusters: usize,
    /// 5.3: block entries when positive news has repeatedly failed to move price.
    #[serde(default)]
    pub reaction_gate: bool,
    #[serde(default = "d_reaction_min")]
    pub reaction_min_clusters: usize,
    #[serde(default = "d_reaction_z")]
    pub reaction_z: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Sizing {
    #[default]
    Fixed,
    Vol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StopMode {
    #[default]
    FixedPct,
    Atr,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RiskConfig {
    pub max_asset_weight: f64,
    pub max_gross: f64,
    pub stop_loss_pct: f64,
    pub daily_loss_limit_pct: f64,
    pub cooldown_hours: f64,
    pub min_trade_usd: f64,
    /// Skip rebalances smaller than this fraction of equity.
    pub rebalance_band: f64,
    /// 3.1: "fixed" (max_asset_weight × news score) or "vol" (risk-per-trade / stop distance).
    #[serde(default)]
    pub sizing: Sizing,
    #[serde(default = "d_risk_per_trade")]
    pub risk_per_trade: f64,
    /// 3.2: "fixed_pct" (stop_loss_pct) or "atr" (stop_atr_mult × ATR14%).
    #[serde(default)]
    pub stop: StopMode,
    #[serde(default = "d_stop_atr_mult")]
    pub stop_atr_mult: f64,
    /// 3.3: cap on Σ weight × stop distance. Default = max_gross × stop_loss_pct (never binds).
    #[serde(default)]
    pub max_heat: Option<f64>,
    /// 3.4: drawdown halts (off unless set).
    #[serde(default)]
    pub weekly_loss_limit_pct: Option<f64>,
    #[serde(default)]
    pub monthly_loss_limit_pct: Option<f64>,
    /// 3.4: N losing round trips in a row → no entries for `streak_cooldown_hours` (off unless set).
    #[serde(default)]
    pub losing_streak_n: Option<u32>,
    #[serde(default = "d_streak_hours")]
    pub streak_cooldown_hours: f64,
}

impl RiskConfig {
    pub fn heat_cap(&self) -> f64 {
        self.max_heat.unwrap_or(self.max_gross * self.stop_loss_pct)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaperConfig {
    pub starting_cash: f64,
    pub fee_rate: f64,
    pub slippage_rate: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RegimeMode {
    /// Record and display only (4.2).
    #[default]
    Off,
    /// Scale max_gross by zone.
    Ceiling,
    /// No new entries in RISK_OFF.
    Block,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegimeConfig {
    #[serde(default = "d_true")]
    pub enabled: bool,
    #[serde(default)]
    pub mode: RegimeMode,
    #[serde(default = "d_regime_minutes")]
    pub refresh_minutes: u64,
    #[serde(default = "d_coingecko")]
    pub coingecko_url: String,
}

impl Default for RegimeConfig {
    fn default() -> Self {
        Self { enabled: true, mode: RegimeMode::Off, refresh_minutes: d_regime_minutes(), coingecko_url: d_coingecko() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeAction {
    #[default]
    Block,
    Halve,
}

/// 5.2: one adversarial Jev call before an entry.
#[derive(Debug, Clone, Deserialize)]
pub struct ChallengeConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "d_challenge_threshold")]
    pub threshold: f64,
    #[serde(default)]
    pub action: ChallengeAction,
}

impl Default for ChallengeConfig {
    fn default() -> Self {
        Self { enabled: false, threshold: d_challenge_threshold(), action: ChallengeAction::Block }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettleConfig {
    #[serde(default = "d_true")]
    pub enabled: bool,
    /// Horizons in hours, on closed 1h candles.
    #[serde(default = "d_horizons")]
    pub horizons: Vec<u32>,
    /// Benchmark asset for excess returns.
    #[serde(default = "d_benchmark")]
    pub benchmark: String,
}

impl Default for SettleConfig {
    fn default() -> Self {
        Self { enabled: true, horizons: d_horizons(), benchmark: d_benchmark() }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub symbol: String,
    pub name: String,
    /// Hyperliquid perp name, e.g. "BTC" or "kPEPE".
    #[serde(alias = "pair")]
    pub coin: String,
}

fn d_true() -> bool { true }
fn d_one() -> usize { 1 }
fn d_max_attempts() -> u32 { 5 }
fn d_breaker_ticks() -> usize { 10 }
fn d_breaker_fail_ratio() -> f64 { 0.5 }
fn d_breaker_min_calls() -> u32 { 4 }
fn d_coingecko() -> String { "https://api.coingecko.com".into() }
fn d_reaction_min() -> usize { 3 }
fn d_reaction_z() -> f64 { -1.5 }
fn d_risk_per_trade() -> f64 { 0.005 }
fn d_stop_atr_mult() -> f64 { 2.0 }
fn d_streak_hours() -> f64 { 24.0 }
fn d_regime_minutes() -> u64 { 60 }
fn d_challenge_threshold() -> f64 { 0.7 }
fn d_horizons() -> Vec<u32> { vec![1, 4, 24] }
fn d_benchmark() -> String { "BTC".into() }

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        reject_binance(&text).with_context(|| format!("{} was written for Binance; this build trades only on Hyperliquid (see README)", path.display()))?;
        let cfg: Config = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        for a in &cfg.assets {
            if a.coin.ends_with("USDT") || a.coin.ends_with("USDC") {
                anyhow::bail!("asset {}: coin \"{}\" looks like a Binance pair; use the Hyperliquid perp name (e.g. \"BTC\")", a.symbol, a.coin);
            }
        }
        Ok(cfg)
    }

    /// Hyperliquid API base for market data and orders.
    pub fn api(&self) -> &'static str {
        self.exchange.network.api_url()
    }

    pub fn interval_ms(&self) -> i64 {
        crate::market::interval_ms(&self.market.interval).unwrap_or(3_600_000)
    }
}
