//! `jev-trader scalp`: a mean-reversion trader on Hyperliquid perps (the module
//! keeps its old name; the order-flow rules were retired 2026-11-10, H-C05/H-R01).
//!
//! - **Pace:** a websocket keeps best bid/ask, trades and the perp context; a bars
//!   task refreshes 15m candles as each closes; the engine evaluates every `eval_ms`.
//! - **Decision:** a 15m close more than `revert_z` std from the mean of the previous
//!   `revert_bars` closes is faded: bought when stretched low, sold when high. One
//!   candidate per coin per bar. 1× notional, a perp short is a plain sell.
//! - **Fills:** paper fills on the real feed, or (`--live`) real orders: post-only
//!   entries that count only once the exchange reports the fill, reduce-only IOC exits.
//! - **Exits:** hard stop (`revert_stop_bps`), hack alert (longs), time stop.
//! - **Thought process:** news never starts a trade. The slow news loop tilts the
//!   flow threshold and vetoes on hacks; at entry time Jev reviews the proposed
//!   trade against recent headlines and can block it or halve its size.
//! - **Evidence:** every candidate (taken or not) is logged with its gates and Jev's
//!   review, then marked out at fixed horizons net of costs.

pub mod backtest;
pub mod engine;
pub mod features;
pub mod report;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, Utc};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{error, info, warn};

use crate::bot::append_jsonl;
use crate::config::{Config, EntryMode, ScalpConfig};
use crate::jev::JevClient;
use crate::live::{Booked, Live, MIN_ORDER_USD};
use crate::news::{self, NewsEvent, ScalpReview};
use crate::signals::{SignalOpts, news_signal_with_drivers};
use crate::status::{GateResult, write_atomic};
use engine::{EntryEval, NewsCtx, OpenPos, Side};
use features::{AssetMarket, Features};

const LIVE_MODE: &str = "live";

struct Files {
    state: PathBuf,
    news: PathBuf,
    status: PathBuf,
    trades: PathBuf,
    candidates: PathBuf,
    markouts: PathBuf,
    events: PathBuf,
    orders: PathBuf,
}

impl Files {
    fn new(dir: &Path) -> Self {
        Self {
            state: dir.join("scalp_state.json"),
            news: dir.join("scalp_news.json"),
            status: dir.join("scalp_status.json"),
            trades: dir.join("scalp_trades.jsonl"),
            candidates: dir.join("scalp_candidates.jsonl"),
            markouts: dir.join("scalp_markouts.jsonl"),
            events: dir.join("scalp_events.jsonl"),
            orders: dir.join("scalp_orders.jsonl"),
        }
    }
}

// ---------------------------------------------------------------- ledger

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScalpPos {
    pub side: Side,
    /// Base bought (long, net of fees) or sold (short) at entry.
    pub qty: f64,
    pub entry_px: f64,
    /// Signed cash and coin changes at entry (long: −cash, +coin; short: +cash, −coin).
    pub entry_cash: f64,
    pub entry_base: f64,
    pub entry_fee: f64,
    pub opened_ms: i64,
    pub tp_px: f64,
    pub sl_px: f64,
    pub init_sl_px: f64,
    /// Best price reached in the trade's favour (bid for longs, ask for shorts).
    pub best_px: f64,
    pub sl_bps: f64,
    pub cost_bps: f64,
    pub deadline_ms: i64,
    pub candidate_id: String,
    pub size_mult: f64,
    #[serde(default)]
    pub review: Option<ScalpReview>,
    pub order_id: String,
    /// Perps: funding received (+) or paid (−) while open, already in cash.
    #[serde(default)]
    pub funding_usd: f64,
    /// An exit was decided but not (fully) filled yet: keep closing, whatever the rules say now.
    #[serde(default)]
    pub closing: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub mode: Option<String>,
    /// "hyperliquid-paper", "hyperliquid" or "hyperliquid-testnet".
    #[serde(default)]
    pub venue: Option<String>,
    /// Collateral plus realized results (a long's notional is spent, a short's received).
    pub cash: f64,
    /// Cash when the ledger was created.
    pub starting_cash: f64,
    /// Signed perp position per coin (positive = long).
    #[serde(default)]
    pub held: BTreeMap<String, f64>,
    pub positions: BTreeMap<String, ScalpPos>,
    pub day: NaiveDate,
    pub day_start_equity: f64,
    /// Trade P&L today (the daily loss limit applies to this).
    #[serde(default)]
    pub day_trade_pnl: f64,
    #[serde(default)]
    pub halted_day: Option<NaiveDate>,
    #[serde(default)]
    pub cooldown_until_ms: BTreeMap<String, i64>,
    #[serde(default)]
    pub entries_ms: VecDeque<i64>,
    #[serde(default)]
    pub wins: u32,
    #[serde(default)]
    pub losses: u32,
    /// Total trade P&L since the ledger started.
    #[serde(default)]
    pub realized: f64,
    #[serde(default)]
    pub live_hold: Option<String>,
}

impl Ledger {
    fn new(cash: f64, live: bool, venue: &str) -> Self {
        Self {
            mode: live.then(|| LIVE_MODE.to_string()),
            venue: Some(venue.to_string()),
            cash,
            starting_cash: cash,
            held: BTreeMap::new(),
            positions: BTreeMap::new(),
            day: Utc::now().date_naive(),
            day_start_equity: cash,
            day_trade_pnl: 0.0,
            halted_day: None,
            cooldown_until_ms: BTreeMap::new(),
            entries_ms: VecDeque::new(),
            wins: 0,
            losses: 0,
            realized: 0.0,
            live_hold: None,
        }
    }

    fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, &serde_json::to_vec_pretty(self)?)
    }

    fn held(&self, sym: &str) -> f64 {
        self.held.get(sym).copied().unwrap_or(0.0)
    }

    /// Cash plus every open position at its mark (entry price when no mark).
    fn equity(&self, marks: &HashMap<String, f64>) -> f64 {
        self.cash + self.held.iter().map(|(s, q)| q * self.mark(s, marks)).sum::<f64>()
    }

    fn mark(&self, sym: &str, marks: &HashMap<String, f64>) -> f64 {
        marks.get(sym).copied().or_else(|| self.positions.get(sym).map(|p| p.entry_px)).unwrap_or(0.0)
    }

    /// Notional of open positions; at 1× it is the collateral already in use.
    fn open_notional(&self, marks: &HashMap<String, f64>) -> f64 {
        self.held.iter().map(|(s, q)| q.abs() * self.mark(s, marks)).sum()
    }

    /// What the ledger was given.
    fn capital(&self) -> f64 {
        self.starting_cash
    }

    fn apply(&mut self, sym: &str, x: &Exec) {
        self.cash += x.cash;
        *self.held.entry(sym.to_string()).or_default() += x.base;
    }
}

// ---------------------------------------------------------------- venue

/// One fill in ledger terms: signed changes to cash and to the coin.
#[derive(Debug, Clone)]
struct Exec {
    cash: f64,
    base: f64,
    price: f64,
    fee_usd: f64,
    order_id: String,
}

impl Exec {
    fn from_booked(b: Booked, buy: bool) -> Self {
        if buy {
            Exec { cash: -(b.notional + b.fee_usd), base: b.sz, price: b.price, fee_usd: b.fee_usd, order_id: b.order_id }
        } else {
            Exec { cash: b.notional - b.fee_usd, base: -b.sz, price: b.price, fee_usd: b.fee_usd, order_id: b.order_id }
        }
    }
}

enum Venue {
    /// Simulated fills against the real Hyperliquid feed, charged real fees.
    Paper,
    Live(Live),
}

impl Venue {
    fn name(&self) -> &'static str {
        match self {
            Venue::Paper => "hyperliquid-paper",
            Venue::Live(l) => l.venue(),
        }
    }

    fn min_notional(&self) -> f64 {
        match self {
            Venue::Paper => 0.0,
            Venue::Live(_) => MIN_ORDER_USD,
        }
    }

    /// A marketable order for `qty` of the coin. Exits pass `reduce_only`.
    async fn trade(&self, coin: &str, buy: bool, qty: f64, reduce_only: bool, f: Option<&Features>, cfg: &ScalpConfig) -> std::result::Result<Exec, String> {
        let f = f.ok_or("no price for the order")?;
        match self {
            Venue::Paper => {
                let fee = cfg.fee_bps / 1e4;
                let px = if buy { f.ask * (1.0 + cfg.slippage_bps / 1e4) } else { f.bid * (1.0 - cfg.slippage_bps / 1e4) };
                let notional = qty * px;
                let (cash, base) = if buy { (-(notional * (1.0 + fee)), qty) } else { (notional * (1.0 - fee), -qty) };
                Ok(Exec { cash, base, price: px, fee_usd: notional * fee, order_id: "paper".into() })
            }
            Venue::Live(l) => {
                let reference = if buy { f.ask } else { f.bid };
                l.market_fill(coin, buy, qty, reference, reduce_only).await.map(|b| Exec::from_booked(b, buy))
            }
        }
    }
}

/// A post-only entry waiting for the market to trade through its price (paper)
/// or for the exchange to report its fill (live).
struct PendingEntry {
    side: Side,
    px: f64,
    usd: f64,
    posted_ms: i64,
    /// Live only: the resting order and its size.
    cloid: Option<String>,
    sz: f64,
    last_poll_ms: i64,
    eval: EntryEval,
    candidate_id: String,
    size_mult: f64,
    review: Option<ScalpReview>,
}

#[derive(Serialize)]
struct OrderRecord<'a> {
    ts: DateTime<Utc>,
    candidate_id: &'a str,
    asset: &'a str,
    side: Side,
    px: f64,
    usd: f64,
    /// "filled", "partial" (live: part filled before the cancel) or "missed".
    outcome: &'a str,
    wait_ms: i64,
    /// Mid when the order resolved, vs the posted price, signed in the trade's favour.
    mid_move_bps: f64,
}

// ---------------------------------------------------------------- shared state

#[derive(Debug, Default, Clone, Serialize)]
pub(crate) struct FeedHealth {
    pub connected: bool,
    pub reconnects: u32,
    pub last_msg_ms: i64,
    pub last_error: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct NewsStore {
    events: Vec<NewsEvent>,
    seen: VecDeque<String>,
    #[serde(skip)]
    attempts: HashMap<String, u32>,
    #[serde(skip)]
    ok: u64,
    #[serde(skip)]
    failed: u64,
    #[serde(skip)]
    last_run: Option<DateTime<Utc>>,
}

pub(crate) type Markets = Arc<Mutex<HashMap<String, AssetMarket>>>;

// ---------------------------------------------------------------- records

#[derive(Serialize)]
struct CandidateRecord<'a> {
    id: &'a str,
    ts: DateTime<Utc>,
    asset: &'a str,
    side: Side,
    features: &'a Features,
    news: NewsCtx,
    eval: &'a EntryEval,
    risk: &'a [GateResult],
    review: Option<&'a ScalpReview>,
    review_error: Option<&'a str>,
    size_mult: f64,
    taken: bool,
    blocked_by: Option<String>,
}

/// `ret_bps` is signed in the trade's favour: a fall counts as a gain for a short.
#[derive(Serialize)]
struct MarkoutRecord<'a> {
    id: &'a str,
    asset: &'a str,
    side: Side,
    taken: bool,
    h_secs: i64,
    ret_bps: f64,
    net_bps: f64,
    cost_bps: f64,
}

#[derive(Serialize)]
struct TradeRecord<'a> {
    asset: &'a str,
    side: Side,
    venue: &'a str,
    opened: DateTime<Utc>,
    closed: DateTime<Utc>,
    hold_secs: i64,
    qty: f64,
    entry_px: f64,
    exit_px: f64,
    /// Trade size in dollars at entry.
    notional: f64,
    fees_usd: f64,
    pnl_usd: f64,
    pnl_bps: f64,
    reason: &'a str,
    tp_px: f64,
    sl_px: f64,
    init_sl_px: f64,
    size_mult: f64,
    review: Option<&'a ScalpReview>,
    candidate_id: &'a str,
    entry_order: &'a str,
    exit_order: &'a str,
}

struct PendingMarkout {
    id: String,
    asset: String,
    side: Side,
    taken: bool,
    t0_sec: i64,
    mid0: f64,
    cost_bps: f64,
    left: Vec<i64>,
}

// ---------------------------------------------------------------- tasks

/// Hyperliquid perp feed: best bid/ask (`bbo`, on change per block), every trade
/// (`side` "B" = the taker bought), and funding / open interest (`activeAssetCtx`).
pub(crate) async fn hl_feed_task(ws_url: &'static str, coins: Vec<(String, String)>, markets: Markets, health: Arc<Mutex<FeedHealth>>) {
    use tokio_tungstenite::tungstenite::Message;
    let by_coin: HashMap<String, String> = coins.iter().map(|(c, s)| (c.clone(), s.clone())).collect();
    let num = |v: &Value| match v {
        Value::String(s) => s.parse::<f64>().unwrap_or(f64::NAN),
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        _ => f64::NAN,
    };
    let mut backoff = 1u64;
    loop {
        match tokio_tungstenite::connect_async(ws_url).await {
            Ok((mut ws, _)) => {
                let mut sub_ok = true;
                for (coin, _) in &coins {
                    for kind in ["bbo", "trades", "activeAssetCtx"] {
                        let m = serde_json::json!({"method": "subscribe", "subscription": {"type": kind, "coin": coin}});
                        if ws.send(Message::Text(m.to_string().into())).await.is_err() {
                            sub_ok = false;
                        }
                    }
                }
                if sub_ok {
                    info!("hyperliquid stream connected ({} coins)", coins.len());
                    backoff = 1;
                    health.lock().unwrap().connected = true;
                }
                let mut ping = tokio::time::interval(Duration::from_secs(30));
                while sub_ok {
                    let msg = tokio::select! {
                        _ = ping.tick() => {
                            if ws.send(Message::Text(r#"{"method":"ping"}"#.into())).await.is_err() { break; }
                            continue;
                        }
                        m = tokio::time::timeout(Duration::from_secs(45), ws.next()) => m,
                    };
                    let msg = match msg {
                        Ok(Some(Ok(m))) => m,
                        Ok(Some(Err(e))) => {
                            health.lock().unwrap().last_error = format!("{e}");
                            break;
                        }
                        Ok(None) => break,
                        Err(_) => {
                            health.lock().unwrap().last_error = "no data for 45s".into();
                            break;
                        }
                    };
                    let text = match msg {
                        Message::Text(t) => t,
                        Message::Ping(p) => {
                            let _ = ws.send(Message::Pong(p)).await;
                            continue;
                        }
                        Message::Close(_) => break,
                        _ => continue,
                    };
                    let now = Utc::now().timestamp_millis();
                    let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                    let d = &v["data"];
                    match v["channel"].as_str().unwrap_or_default() {
                        "bbo" => {
                            let Some(sym) = d["coin"].as_str().and_then(|c| by_coin.get(c)) else { continue };
                            let (b, a) = (&d["bbo"][0], &d["bbo"][1]);
                            if b.is_null() || a.is_null() {
                                continue;
                            }
                            markets.lock().unwrap().entry(sym.clone()).or_default().on_book(num(&b["px"]), num(&b["sz"]), num(&a["px"]), num(&a["sz"]), now);
                        }
                        "trades" => {
                            let Some(arr) = d.as_array() else { continue };
                            let mut m = markets.lock().unwrap();
                            for t in arr {
                                let Some(sym) = t["coin"].as_str().and_then(|c| by_coin.get(c)) else { continue };
                                let taker_sold = t["side"].as_str() == Some("A");
                                m.entry(sym.clone()).or_default().on_trade(num(&t["px"]), num(&t["sz"]), taker_sold, now);
                            }
                        }
                        "activeAssetCtx" => {
                            let Some(sym) = d["coin"].as_str().and_then(|c| by_coin.get(c)) else { continue };
                            let c = &d["ctx"];
                            markets.lock().unwrap().entry(sym.clone()).or_default().on_ctx(num(&c["funding"]), num(&c["openInterest"]), num(&c["markPx"]), num(&c["oraclePx"]));
                        }
                        "error" => {
                            warn!("hyperliquid stream error: {text}");
                            health.lock().unwrap().last_error = text.to_string();
                            continue;
                        }
                        _ => continue,
                    }
                    health.lock().unwrap().last_msg_ms = now;
                }
            }
            Err(e) => health.lock().unwrap().last_error = format!("connect: {e}"),
        }
        {
            let mut h = health.lock().unwrap();
            h.connected = false;
            h.reconnects += 1;
            warn!("hyperliquid stream down ({}); reconnecting in {backoff}s", h.last_error);
        }
        tokio::time::sleep(Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(30);
    }
}

async fn news_task(cfg: Config, jev: JevClient, store: Arc<Mutex<NewsStore>>, files_news: PathBuf, files_events: PathBuf) {
    let http = match reqwest::Client::builder().timeout(Duration::from_secs(15)).user_agent("jev-trader/0.1").build() {
        Ok(h) => h,
        Err(e) => {
            error!("news http client: {e}");
            return;
        }
    };
    loop {
        let now = Utc::now();
        let max_age = chrono::Duration::seconds((cfg.news.max_age_hours * 3600.0) as i64);
        let items = crate::bot::fetch_all_news(&http, &cfg).await;
        let fresh: Vec<_> = {
            let s = store.lock().unwrap();
            let seen: HashSet<&String> = s.seen.iter().collect();
            items.into_iter().filter(|i| !seen.contains(&i.id) && i.published.is_some_and(|p| now - p < max_age)).collect()
        };
        let results: Vec<_> = futures::stream::iter(fresh)
            .map(|item| {
                let jev = &jev;
                let assets = &cfg.assets;
                async move {
                    let r = news::analyze(jev, &item, assets, now).await.and_then(|ev| crate::validate::news_event(&ev).map(|_| ev));
                    (item, r)
                }
            })
            .buffer_unordered(cfg.jev.concurrency.max(1))
            .collect()
            .await;
        {
            let mut s = store.lock().unwrap();
            for (item, r) in results {
                match r {
                    Ok(ev) => {
                        info!(asset = %ev.asset, event = %ev.event, sentiment = format!("{:+.2}", ev.sentiment), "news: {}", ev.title);
                        append_jsonl(&files_events, &ev);
                        s.ok += 1;
                        s.seen.push_back(item.id.clone());
                        s.events.push(ev);
                    }
                    Err(e) => {
                        s.failed += 1;
                        let n = s.attempts.entry(item.id.clone()).or_default();
                        *n += 1;
                        warn!("news analysis failed ({n}/3) for '{}': {e:#}", item.title);
                        if *n >= 3 {
                            s.seen.push_back(item.id.clone());
                        }
                    }
                }
            }
            let retain = chrono::Duration::seconds((cfg.news.retain_hours * 3600.0) as i64);
            s.events.retain(|e| now - e.published < retain);
            while s.seen.len() > 5000 {
                s.seen.pop_front();
            }
            s.last_run = Some(now);
            if let Ok(b) = serde_json::to_vec(&*s) {
                let _ = write_atomic(&files_news, &b);
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

/// Refreshes each coin's mean-reversion z just after every `revert_interval`
/// candle closes (retrying every 20s on failure). Stale bars block entries.
async fn bars_task(api: &'static str, sc: ScalpConfig, coins: Vec<(String, String)>, markets: Markets) {
    let http = match reqwest::Client::builder().timeout(Duration::from_secs(15)).user_agent("jev-trader/0.1").build() {
        Ok(h) => h,
        Err(e) => return error!("bars task: http client: {e}"),
    };
    let Some(ims) = crate::market::interval_ms(&sc.revert_interval) else {
        return error!("bars task: bad revert_interval {:?}", sc.revert_interval);
    };
    loop {
        let mut failed = false;
        for (coin, sym) in &coins {
            let now_ms = Utc::now().timestamp_millis();
            match crate::market::klines(&http, api, coin, &sc.revert_interval, sc.revert_bars + 3).await {
                Ok(c) => {
                    let snap = crate::market::split_closed(c, now_ms, ims);
                    let closes: Vec<f64> = snap.closed.iter().map(|c| c.close).collect();
                    let z = features::revert_z(&closes, sc.revert_bars);
                    let mut m = markets.lock().unwrap();
                    let am = m.entry(sym.clone()).or_default();
                    am.z_rev = z.unwrap_or(0.0);
                    am.bars = if z.is_some() { closes.len() } else { 0 };
                    am.bar_ms = snap.closed.last().map(|c| c.open_time).unwrap_or(0);
                }
                Err(e) => {
                    warn!("{sym}: {} candles failed: {e:#}", sc.revert_interval);
                    failed = true;
                }
            }
        }
        let now_ms = Utc::now().timestamp_millis();
        let next_close = (now_ms / ims + 1) * ims + 3_000;
        let wait = if failed { 20_000 } else { next_close - now_ms };
        tokio::time::sleep(Duration::from_millis(wait.max(1_000) as u64)).await;
    }
}

// ---------------------------------------------------------------- main loop

struct ReviewCache {
    at_ms: i64,
    newest_event: Option<DateTime<Utc>>,
    review: Option<ScalpReview>,
}

/// Counters for the status file (this session only).
#[derive(Default)]
struct Counts {
    candidates: u64,
    reviews: u64,
    vetoes: u64,
    review_fail: u64,
}


/// A position opened from a fill, with targets from the entry evaluation.
#[allow(clippy::too_many_arguments)]
fn new_pos(side: Side, x: &Exec, ev: &EntryEval, now_ms: i64, sc: &ScalpConfig, candidate_id: &str, size_mult: f64, review: Option<ScalpReview>) -> ScalpPos {
    let s = side.sign();
    ScalpPos {
        side,
        qty: x.base.abs(),
        entry_px: x.price,
        entry_cash: x.cash,
        entry_base: x.base,
        entry_fee: x.fee_usd,
        opened_ms: now_ms,
        tp_px: x.price * (1.0 + s * ev.tp_bps / 1e4),
        sl_px: x.price * (1.0 - s * ev.sl_bps / 1e4),
        init_sl_px: x.price * (1.0 - s * ev.sl_bps / 1e4),
        best_px: x.price,
        sl_bps: ev.sl_bps,
        cost_bps: ev.cost_bps,
        deadline_ms: now_ms + sc.hold_secs * 1000,
        candidate_id: candidate_id.to_string(),
        size_mult,
        review,
        order_id: x.order_id.clone(),
        funding_usd: 0.0,
        closing: None,
    }
}

/// A maker fill in ledger terms: `sz` at `px`, paying `fee_usd`.
/// Prices with 5 significant digits: `{:.4}` printed kPEPE's 0.005213 as 0.0052.
fn fmt_px(x: f64) -> String {
    let d = if x.abs() > 0.0 { (4 - x.abs().log10().floor() as i32).max(0) } else { 0 };
    format!("{x:.*}", d as usize)
}

fn maker_exec(buy: bool, sz: f64, px: f64, fee_usd: f64, order_id: String) -> Exec {
    let n = sz * px;
    if buy {
        Exec { cash: -(n + fee_usd), base: sz, price: px, fee_usd, order_id }
    } else {
        Exec { cash: n - fee_usd, base: -sz, price: px, fee_usd, order_id }
    }
}

/// Splits the closed fraction of a position off: returns its P&L and shrinks `p`
/// to what is still open. `x` is the exit fill (its base has the opposite sign).
fn close_part(p: &mut ScalpPos, x: &Exec) -> f64 {
    let frac = (x.base.abs() / p.qty).clamp(0.0, 1.0);
    let pnl = frac * p.entry_cash + x.cash + (frac * p.entry_base + x.base) * x.price + frac * p.funding_usd;
    let keep = 1.0 - frac;
    p.qty -= x.base.abs();
    p.entry_cash *= keep;
    p.entry_base *= keep;
    p.entry_fee *= keep;
    p.funding_usd *= keep;
    pnl
}

pub async fn run(cfg: Config, jev: JevClient, live: Option<Live>) -> Result<()> {
    let sc = cfg.scalp.clone();
    let files = Files::new(&cfg.data_dir);
    crate::status::acquire_pid_lock(&cfg.data_dir.join("scalp.pid"))?;
    let venue = match live {
        Some(l) => Venue::Live(l),
        None => Venue::Paper,
    };
    let is_live = matches!(venue, Venue::Live(_));
    let coin_of: HashMap<String, String> = cfg.assets.iter().map(|a| (a.symbol.clone(), a.coin.clone())).collect();
    let mut ledger = open_ledger(&venue, &cfg, &files.state).await?;
    info!(venue = venue.name(), cash = ledger.cash, positions = ledger.positions.len(), shorts = sc.shorts, "scalper ledger loaded");

    let markets: Markets = Arc::new(Mutex::new(HashMap::new()));
    let health = Arc::new(Mutex::new(FeedHealth::default()));
    let store: Arc<Mutex<NewsStore>> = Arc::new(Mutex::new(
        std::fs::read(&files.news).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default(),
    ));
    let coins: Vec<(String, String)> = cfg.assets.iter().map(|a| (a.coin.clone(), a.symbol.clone())).collect();
    tokio::spawn(hl_feed_task(cfg.exchange.network.ws_url(), coins, markets.clone(), health.clone()));
    tokio::spawn(bars_task(cfg.api(), sc.clone(), cfg.assets.iter().map(|a| (a.coin.clone(), a.symbol.clone())).collect(), markets.clone()));
    tokio::spawn(news_task(cfg.clone(), jev.clone(), store.clone(), files.news.clone(), files.events.clone()));

    let mut reviews: HashMap<(String, Side), ReviewCache> = HashMap::new();
    let mut last_candidate_bar: HashMap<String, i64> = HashMap::new();
    let mut pending: Vec<PendingMarkout> = Vec::new();
    // Live: free margin on the exchange and per-coin position mismatches.
    let mut withdrawable: Option<f64> = None;
    let mut exch_review: HashMap<String, String> = HashMap::new();
    let mut last_reconcile_ms = 0i64;
    let mut last_status_ms = 0i64;
    let mut blocks: HashMap<(String, Side), String> = HashMap::new();
    let mut n = Counts::default();
    let mut pending_entries: HashMap<String, PendingEntry> = HashMap::new();
    let (mut maker_filled, mut maker_missed) = (0u32, 0u32);
    let mut last_funding_ms = Utc::now().timestamp_millis();
    let mut ticker = tokio::time::interval(Duration::from_millis(sc.eval_ms.max(100)));
    // Set by Ctrl+C when flatten_on_exit: every position is closed, then the loop returns.
    let mut shutdown_at: Option<i64> = None;

    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = tokio::signal::ctrl_c() => {
                if shutdown_at.is_some() {
                    warn!("second Ctrl+C: exiting now; open positions stay in the ledger");
                    ledger.save(&files.state)?;
                    return Ok(());
                }
                if let Venue::Live(l) = &venue {
                    for (sym, pe) in &pending_entries {
                        if let Some(c) = &pe.cloid {
                            match l.cancel(&coin_of[sym], c).await {
                                Ok(()) => info!("{sym}: resting entry cancelled on shutdown"),
                                Err(e) => error!("{sym}: could not cancel resting entry {c} on shutdown ({e}); cancel it on Hyperliquid"),
                            }
                        }
                    }
                }
                pending_entries.clear();
                if !sc.flatten_on_exit || ledger.positions.is_empty() {
                    info!("shutting down; open positions stay in the ledger and are managed on restart");
                    ledger.save(&files.state)?;
                    return Ok(());
                }
                info!("shutting down: closing {} open position(s) (Ctrl+C again to leave them open)", ledger.positions.len());
                shutdown_at = Some(Utc::now().timestamp_millis());
            }
        }
        let now = Utc::now();
        let now_ms = now.timestamp_millis();

        // Snapshot features and news context.
        let feats: HashMap<String, Features> = {
            let mut m = markets.lock().unwrap();
            cfg.assets.iter().filter_map(|a| m.get_mut(&a.symbol).and_then(|am| am.features(now_ms, &sc)).map(|f| (a.symbol.clone(), f))).collect()
        };
        let (ctx, events): (HashMap<String, NewsCtx>, Vec<NewsEvent>) = {
            let s = store.lock().unwrap();
            let opts = SignalOpts::from_config(&cfg.news);
            let ctx = cfg
                .assets
                .iter()
                .map(|a| {
                    let (n, _) = news_signal_with_drivers(&s.events, &a.symbol, now, &opts, 0);
                    (a.symbol.clone(), NewsCtx { score: n.score, hack_alert: n.hack_alert, n_events: n.n_events })
                })
                .collect();
            (ctx, s.events.clone())
        };
        let marks: HashMap<String, f64> = feats.iter().map(|(k, f)| (k.clone(), f.mid)).collect();

        // Day roll; the daily limit is on trade P&L.
        if ledger.day != now.date_naive() {
            ledger.day = now.date_naive();
            ledger.day_start_equity = ledger.equity(&marks);
            ledger.day_trade_pnl = 0.0;
        }
        if ledger.day_trade_pnl < -sc.daily_loss_limit_pct * ledger.capital() && ledger.halted_day != Some(ledger.day) {
            warn!(pnl = ledger.day_trade_pnl, "daily loss limit hit: no entries until tomorrow (UTC)");
            ledger.halted_day = Some(ledger.day);
        }

        // Live: the exchange must hold exactly what the ledger holds (every 30s).
        if let Venue::Live(l) = &venue
            && now_ms - last_reconcile_ms > 30_000
        {
            last_reconcile_ms = now_ms;
            match l.ex.account().await {
                Ok(acct) => {
                    exch_review.clear();
                    for a in &cfg.assets {
                        // A resting entry may be part-filled; judge it once it resolves.
                        if pending_entries.contains_key(&a.symbol) {
                            continue;
                        }
                        let want = ledger.held(&a.symbol);
                        let have = acct.positions.get(&a.coin).copied().unwrap_or(0.0);
                        if (have - want).abs() > l.step(&a.coin) * 0.5 + 1e-12 {
                            exch_review.insert(a.symbol.clone(), format!("Hyperliquid holds {have} {}, ledger expects {want:.6}", a.coin));
                        }
                    }
                    withdrawable = Some(acct.withdrawable);
                }
                Err(e) => {
                    warn!("account check failed: {e:#}");
                    withdrawable = None;
                }
            }
        }

        // Funding accrues continuously on open positions (Hyperliquid settles hourly;
        // accrued here per tick at the current hourly rate). Live, the exchange
        // charges the real amount; this keeps the ledger's estimate close to it.
        let dt_h = (now_ms - last_funding_ms).max(0) as f64 / 3_600_000.0;
        last_funding_ms = now_ms;
        for (sym, p) in ledger.positions.iter_mut() {
            let Some(f) = feats.get(sym) else { continue };
            // Positive funding: longs pay, shorts receive.
            let amt = -p.side.sign() * f.funding_bps_1h / 1e4 * p.qty * f.mid * dt_h;
            p.funding_usd += amt;
            ledger.cash += amt;
        }

        // 0. Resting maker entries.
        let waiting: Vec<String> = pending_entries.keys().cloned().collect();
        for sym in waiting {
            let pe = &pending_entries[&sym];
            let expired = now_ms - pe.posted_ms >= sc.maker_timeout_secs * 1000;
            // (filled size, price, fee, order id) once resolved; None = still resting.
            let resolved: Option<(f64, f64, f64, String)> = match (&venue, &pe.cloid) {
                (Venue::Paper, _) | (Venue::Live(_), None) => {
                    let filled = {
                        let m = markets.lock().unwrap();
                        m.get(&sym).is_some_and(|am| am.traded_through(pe.px, pe.side == Side::Long, pe.posted_ms.div_euclid(1000)))
                    };
                    if filled {
                        let sz = pe.usd / pe.px;
                        Some((sz, pe.px, pe.usd * sc.maker_fee_bps / 1e4, "paper-maker".into()))
                    } else if expired {
                        Some((0.0, pe.px, 0.0, String::new()))
                    } else {
                        None
                    }
                }
                (Venue::Live(l), Some(cloid)) => {
                    if !expired && now_ms - pe.last_poll_ms < 1000 {
                        continue;
                    }
                    let coin = &coin_of[&sym];
                    let (cloid, posted_ms, sz) = (cloid.clone(), pe.posted_ms, pe.sz);
                    pending_entries.get_mut(&sym).expect("listed").last_poll_ms = now_ms;
                    match l.maker_progress(&cloid, posted_ms).await {
                        Err(e) => {
                            warn!("{sym}: order status failed: {e:#}");
                            None
                        }
                        Ok(p) if p.open && !expired => None,
                        Ok(p) if p.oid == 0 && !expired && now_ms - posted_ms < 10_000 => None,
                        Ok(p) if !p.open && p.filled_sz >= sz - l.step(coin) * 0.5 => Some((p.filled_sz, p.avg_px, p.fee_usd, p.oid.to_string())),
                        Ok(_) => {
                            // Expired, cancelled by the exchange, or never seen: pull it, then count what filled.
                            if let Err(e) = l.cancel(coin, &cloid).await {
                                warn!("{sym}: cancel failed ({e}); retrying next tick");
                                None
                            } else {
                                match l.maker_progress(&cloid, posted_ms).await {
                                    Ok(p) if p.open => None,
                                    Ok(p) => Some((p.filled_sz, if p.filled_sz > 0.0 { p.avg_px } else { 0.0 }, p.fee_usd, p.oid.to_string())),
                                    Err(e) => {
                                        warn!("{sym}: order status after cancel failed: {e:#}");
                                        None
                                    }
                                }
                            }
                        }
                    }
                }
            };
            let Some((fsz, fpx, ffee, oid)) = resolved else { continue };
            let pe = pending_entries.remove(&sym).expect("listed");
            let mid = feats.get(&sym).map(|f| f.mid).unwrap_or(pe.px);
            let full = fsz > 0.0 && (pe.cloid.is_none() || fsz >= pe.sz * (1.0 - 1e-9));
            append_jsonl(
                &files.orders,
                &OrderRecord {
                    ts: now,
                    candidate_id: &pe.candidate_id,
                    asset: &sym,
                    side: pe.side,
                    px: pe.px,
                    usd: pe.usd,
                    outcome: if full { "filled" } else if fsz > 0.0 { "partial" } else { "missed" },
                    wait_ms: now_ms - pe.posted_ms,
                    mid_move_bps: pe.side.sign() * (mid / pe.px - 1.0) * 1e4,
                },
            );
            if fsz <= 0.0 {
                maker_missed += 1;
                info!("{sym} maker {} at {} not filled in {}s: cancelled", pe.side.word(), fmt_px(pe.px), sc.maker_timeout_secs);
                continue;
            }
            maker_filled += 1;
            let x = maker_exec(pe.side == Side::Long, fsz, fpx, ffee, oid);
            ledger.apply(&sym, &x);
            let pos = new_pos(pe.side, &x, &pe.eval, now_ms, &sc, &pe.candidate_id, pe.size_mult, pe.review.clone());
            info!("{} maker fill {} {sym} {:.6} @ {} (${:.2}) after {}ms · tp {} · stop {}", venue.name().to_uppercase(), pe.side.word(), pos.qty, fmt_px(x.price), pos.qty * x.price, now_ms - pe.posted_ms, fmt_px(pos.tp_px), fmt_px(pos.sl_px));
            ledger.positions.insert(sym.clone(), pos);
            ledger.save(&files.state)?;
        }

        // 1. Exits (trailing the stop first). Once decided, an exit is retried until flat.
        let open: Vec<String> = ledger.positions.keys().cloned().collect();
        for sym in open {
            let news = ctx.get(&sym).copied().unwrap_or_default();
            if let Some(f) = feats.get(&sym)
                && let Some(p) = ledger.positions.get_mut(&sym)
            {
                p.best_px = match p.side {
                    Side::Long => p.best_px.max(f.bid),
                    Side::Short => p.best_px.min(f.ask),
                };
                p.sl_px = engine::trailed_stop(p.side, p.entry_px, p.best_px, p.sl_px, p.sl_bps, p.cost_bps, &sc);
            }
            let p = ledger.positions[&sym].clone();
            let reason = match &p.closing {
                Some(r) => r.clone(),
                None if shutdown_at.is_some() => "shutdown".into(),
                None => {
                    let op = OpenPos { side: p.side, tp_px: p.tp_px, sl_px: p.sl_px, deadline_ms: p.deadline_ms };
                    let Some(mut reason) = engine::evaluate_exit(&op, feats.get(&sym), &news, now_ms, &sc) else { continue };
                    if reason == "stop" && p.side.sign() * (p.sl_px - p.init_sl_px) > 0.0 {
                        reason = "trailing stop".into();
                    }
                    reason
                }
            };
            // Close what the long bought / buy back what the short sold; reduce-only live.
            let res = venue.trade(&coin_of[&sym], p.side == Side::Short, p.qty, true, feats.get(&sym), &sc).await;
            match res {
                Ok(x) => {
                    ledger.apply(&sym, &x);
                    let step = if let Venue::Live(l) = &venue { l.step(&coin_of[&sym]) } else { 0.0 };
                    let mut rest = p.clone();
                    let notional = x.base.abs() * p.entry_px;
                    let pnl = close_part(&mut rest, &x);
                    let flat = rest.qty <= step * 0.5 + 1e-12;
                    ledger.realized += pnl;
                    ledger.day_trade_pnl += pnl;
                    if flat {
                        ledger.positions.remove(&sym);
                        // Dust below one size step can't be traded; drop it from the ledger.
                        ledger.held.insert(sym.clone(), 0.0);
                        if pnl < 0.0 {
                            ledger.losses += 1;
                            ledger.cooldown_until_ms.insert(sym.clone(), now_ms + sc.cooldown_after_loss_secs * 1000);
                        } else {
                            ledger.wins += 1;
                        }
                    } else {
                        warn!("{sym}: exit filled {:.6} of {:.6}; closing the rest next tick", x.base.abs(), p.qty);
                        rest.closing = Some(reason.clone());
                        ledger.positions.insert(sym.clone(), rest);
                    }
                    let shown = if flat { reason.clone() } else { format!("{reason} (partial)") };
                    info!("{} close {} {sym} @ {} — {shown} · pnl {pnl:+.2} ({:+.1} bps)", venue.name().to_uppercase(), p.side.word(), fmt_px(x.price), pnl / notional * 1e4);
                    append_jsonl(
                        &files.trades,
                        &TradeRecord {
                            asset: &sym,
                            side: p.side,
                            venue: venue.name(),
                            opened: DateTime::from_timestamp_millis(p.opened_ms).unwrap_or(now),
                            closed: now,
                            hold_secs: (now_ms - p.opened_ms) / 1000,
                            qty: x.base.abs(),
                            entry_px: p.entry_px,
                            exit_px: x.price,
                            notional,
                            fees_usd: p.entry_fee * (x.base.abs() / p.qty) + x.fee_usd,
                            pnl_usd: pnl,
                            pnl_bps: pnl / notional * 1e4,
                            reason: &shown,
                            tp_px: p.tp_px,
                            sl_px: p.sl_px,
                            init_sl_px: p.init_sl_px,
                            size_mult: p.size_mult,
                            review: p.review.as_ref(),
                            candidate_id: &p.candidate_id,
                            entry_order: &p.order_id,
                            exit_order: &x.order_id,
                        },
                    );
                    ledger.save(&files.state)?;
                }
                Err(m) => {
                    error!("{sym} exit ({reason}) failed: {m}");
                    if let Some(pp) = ledger.positions.get_mut(&sym) {
                        pp.closing = Some(reason.clone());
                    }
                    if m.starts_with("UNCONFIRMED") {
                        ledger.live_hold = Some(format!("{m} · check Hyperliquid, then `jev-trader hl clear-hold`"));
                    }
                    ledger.save(&files.state)?;
                }
            }
        }

        if let Some(t0) = shutdown_at {
            let flat = ledger.positions.is_empty();
            // Exits are retried each tick; give up after 30s (no price, exchange down).
            if flat || now_ms - t0 > 30_000 {
                if flat {
                    info!("shutting down; all positions closed");
                } else {
                    error!("shutting down with {} position(s) still open: close them on the exchange or restart", ledger.positions.len());
                }
                ledger.save(&files.state)?;
                return Ok(());
            }
            continue;
        }

        // 2. Entries, each side judged by the same mirrored gates.
        let macro_event = crate::bot::macro_window(&sc.macro_calendar, now, sc.macro_before_mins, sc.macro_after_mins);
        while ledger.entries_ms.front().is_some_and(|t| *t < now_ms - 3_600_000) {
            ledger.entries_ms.pop_front();
        }
        let sides: &[Side] = if sc.shorts { &[Side::Long, Side::Short] } else { &[Side::Long] };
        for a in &cfg.assets {
            let sym = &a.symbol;
            let Some(f) = feats.get(sym) else { continue };
            let news = ctx.get(sym).copied().unwrap_or_default();
            let evals: Vec<EntryEval> = sides.iter().map(|&s| engine::evaluate_entry(f, &news, &sc, s)).collect();
            for e in &evals {
                blocks.insert((sym.clone(), e.side), e.first_block().unwrap_or("").to_string());
            }
            // z has one sign at a time, so at most one side is a candidate.
            let Some(ev) = evals.into_iter().find(|e| e.candidate) else { continue };
            if f.history_secs < sc.warmup_secs {
                continue;
            }
            // One candidate per asset per closed bar, as in the research.
            if last_candidate_bar.get(sym) == Some(&f.bar_ms) {
                continue;
            }
            last_candidate_bar.insert(sym.clone(), f.bar_ms);
            n.candidates += 1;
            let side = ev.side;

            // 1× notional: free collateral = equity − open notional − resting orders.
            let reserved: f64 = pending_entries.values().map(|p| p.usd).sum();
            let mut free = (ledger.equity(&marks) - ledger.open_notional(&marks) - reserved).max(0.0);
            if is_live {
                free = free.min(withdrawable.unwrap_or(0.0));
            }
            let base_usd = (ledger.capital() * sc.position_pct).min(free);
            let min_usd = sc.min_trade_usd.max(venue.min_notional());
            let mut risk = Vec::new();
            let mut rg = |name: &str, pass: bool, detail: String| {
                risk.push(GateResult::new(name, pass, detail));
                pass
            };
            let mut ok = ev.ok;
            ok &= rg("flat", !ledger.positions.contains_key(sym) && !pending_entries.contains_key(sym), if pending_entries.contains_key(sym) { "maker order resting".into() } else { String::new() });
            let busy = ledger.positions.len() + pending_entries.len();
            ok &= rg("max positions", busy < sc.max_positions, format!("{busy}/{}", sc.max_positions));
            ok &= rg("cooldown", ledger.cooldown_until_ms.get(sym).is_none_or(|t| now_ms >= *t), String::new());
            ok &= rg("trades/hour", ledger.entries_ms.len() < sc.max_trades_per_hour, format!("{}/{}", ledger.entries_ms.len(), sc.max_trades_per_hour));
            ok &= rg("daily limit", ledger.halted_day != Some(ledger.day), String::new());
            ok &= rg("macro", macro_event.is_none(), macro_event.clone().unwrap_or_default());
            let exch_block = ledger.live_hold.clone().or_else(|| exch_review.get(sym).cloned()).or_else(|| (is_live && withdrawable.is_none()).then(|| "account check pending or failing".to_string()));
            ok &= rg("exchange", exch_block.is_none(), exch_block.unwrap_or_default());
            ok &= rg("cash", base_usd >= min_usd, format!("{base_usd:.2} free collateral (≥ {min_usd:.2})"));

            // Jev reviews only trades that would otherwise happen.
            let (mut review, mut review_err, mut size_mult) = (None, None::<String>, 1.0);
            if ok && sc.jev_review {
                let cutoff = now - chrono::Duration::minutes(sc.jev_headline_minutes);
                let mut rel: Vec<&NewsEvent> =
                    events.iter().filter(|e| (e.asset == *sym || e.asset == "market") && e.published > cutoff && e.event != "noise").collect();
                rel.sort_by_key(|e| std::cmp::Reverse(e.published));
                rel.truncate(8);
                let newest = rel.first().map(|e| e.seen_at());
                let key = (sym.clone(), side);
                let cached = reviews.get(&key).filter(|c| now_ms - c.at_ms < sc.jev_cache_secs * 1000 && c.newest_event == newest);
                if let Some(c) = cached {
                    review = c.review.clone();
                } else if rel.is_empty() {
                    reviews.insert(key, ReviewCache { at_ms: now_ms, newest_event: None, review: None });
                } else {
                    let heads: Vec<Value> = rel
                        .iter()
                        .map(|e| serde_json::json!({"title": e.title, "source": e.source, "minutes_ago": (now - e.published).num_minutes()}))
                        .collect();
                    let (who, lean) = if side == Side::Long { ("buyers", "buyers") } else { ("sellers", "sellers") };
                    let share = if side == Side::Long { (f.flow + 1.0) * 50.0 } else { (1.0 - f.flow) * 50.0 };
                    let mv = format!(
                        "price {:+.2}% over the last minute and {:+.2}% over 5 minutes; aggressive {who} are {share:.0}% of traded volume; trading activity is {:.1}x normal; the order book leans to {lean}",
                        f.ret_60s_bps / 100.0,
                        f.ret_300s_bps / 100.0,
                        f.activity
                    );
                    n.reviews += 1;
                    match news::scalp_review(&jev, a, &heads, &mv, sc.hold_secs / 60, side == Side::Short).await {
                        Ok(r) => {
                            reviews.insert(key, ReviewCache { at_ms: now_ms, newest_event: newest, review: Some(r.clone()) });
                            review = Some(r);
                        }
                        Err(e) => {
                            n.review_fail += 1;
                            review_err = Some(format!("{e:#}"));
                        }
                    }
                }
                match (&review, &review_err) {
                    (_, Some(e)) => {
                        ok &= rg("jev review", false, format!("failed: {e}"));
                    }
                    (Some(r), None) => {
                        let veto = r.adverse > sc.jev_veto || r.event_risk > sc.jev_veto;
                        if veto {
                            n.vetoes += 1;
                        }
                        size_mult = if r.support < 0.25 { 0.5 } else { 1.0 };
                        ok &= rg(
                            "jev review",
                            !veto,
                            format!("adverse {:.2} · event risk {:.2} · support {:.2} · {} headlines · size ×{size_mult}", r.adverse, r.event_risk, r.support, r.headlines),
                        );
                    }
                    (None, None) => {
                        rg("jev review", true, "no relevant headlines".into());
                    }
                }
            }

            let id = format!("{sym}-{}-{now_ms}", side.word());
            let usd = base_usd * size_mult;
            let mut blocked_by = ev.first_block().map(str::to_string).or_else(|| risk.iter().find(|g| !g.passed).map(|g| g.gate.clone()));
            let mut taken = false;
            if ok && usd >= min_usd && sc.entry == EntryMode::Maker {
                // Post at our side of the book; resolved in step 0 of a later tick.
                let px = if side == Side::Long { f.bid } else { f.ask };
                let posted = match &venue {
                    Venue::Paper => Ok((None, px, usd / px)),
                    Venue::Live(l) => l.post_maker(&a.coin, side == Side::Long, px, usd).await.map(|(c, p, sz)| (Some(c), p, sz)),
                };
                match posted {
                    Ok((cloid, px, sz)) => {
                        info!("{} post {} {sym} ${:.2} @ {} (maker, {}s timeout) · flow {:+.2} book {:+.2} activity {:.1}×", venue.name().to_uppercase(), side.word(), sz * px, fmt_px(px), sc.maker_timeout_secs, f.flow, f.obi, f.activity);
                        pending_entries.insert(
                            sym.clone(),
                            PendingEntry { side, px, usd: sz * px, posted_ms: now_ms, cloid, sz, last_poll_ms: now_ms, eval: ev.clone(), candidate_id: id.clone(), size_mult, review: review.clone() },
                        );
                        ledger.entries_ms.push_back(now_ms);
                        taken = true;
                    }
                    Err(m) => {
                        warn!("{sym} {} maker post failed: {m}", side.word());
                        if m.starts_with("UNCONFIRMED") {
                            ledger.live_hold = Some(format!("{m} · check Hyperliquid, then `jev-trader hl clear-hold`"));
                            ledger.save(&files.state)?;
                        }
                        blocked_by = Some("exchange".into());
                    }
                }
            } else if ok && usd >= min_usd {
                let qty = usd / if side == Side::Long { f.ask } else { f.bid };
                match venue.trade(&a.coin, side == Side::Long, qty, false, Some(f), &sc).await {
                    Ok(x) => {
                        taken = true;
                        ledger.apply(sym, &x);
                        ledger.entries_ms.push_back(now_ms);
                        let pos = new_pos(side, &x, &ev, now_ms, &sc, &id, size_mult, review.clone());
                        info!(
                            "{} open {} {sym} {:.6} @ {} (${:.2}) · stop {} · {}h time stop · z {:+.2}",
                            venue.name().to_uppercase(),
                            side.word(),
                            pos.qty,
                            fmt_px(x.price),
                            pos.qty * x.price,
                            fmt_px(pos.sl_px),
                            sc.hold_secs / 3600,
                            f.z_rev
                        );
                        ledger.positions.insert(sym.clone(), pos);
                        ledger.save(&files.state)?;
                    }
                    Err(m) => {
                        error!("{sym} {} entry failed: {m}", side.word());
                        if m.starts_with("UNCONFIRMED") {
                            ledger.live_hold = Some(format!("{m} · check Hyperliquid, then `jev-trader hl clear-hold`"));
                            ledger.save(&files.state)?;
                        }
                        blocked_by = Some("exchange".into());
                    }
                }
            }
            append_jsonl(
                &files.candidates,
                &CandidateRecord {
                    id: &id,
                    ts: now,
                    asset: sym,
                    side,
                    features: f,
                    news,
                    eval: &ev,
                    risk: &risk,
                    review: review.as_ref(),
                    review_error: review_err.as_deref(),
                    size_mult,
                    taken,
                    blocked_by: if taken { None } else { blocked_by },
                },
            );
            pending.push(PendingMarkout {
                id,
                asset: sym.clone(),
                side,
                taken,
                t0_sec: now_ms / 1000,
                mid0: f.mid,
                cost_bps: ev.cost_bps,
                left: sc.markouts.clone(),
            });
        }

        // 3. Markouts: what each candidate was worth after costs, in its own direction.
        if !pending.is_empty() {
            let m = markets.lock().unwrap();
            let now_sec = now_ms / 1000;
            for p in pending.iter_mut() {
                let Some(am) = m.get(&p.asset) else { continue };
                p.left.retain(|&h| {
                    if now_sec < p.t0_sec + h {
                        return true;
                    }
                    if let Some(mid) = am.mid_at(p.t0_sec + h) {
                        let ret = p.side.sign() * (mid / p.mid0 - 1.0) * 1e4;
                        append_jsonl(
                            &files.markouts,
                            &MarkoutRecord { id: &p.id, asset: &p.asset, side: p.side, taken: p.taken, h_secs: h, ret_bps: ret, net_bps: ret - p.cost_bps, cost_bps: p.cost_bps },
                        );
                    }
                    false
                });
            }
            pending.retain(|p| !p.left.is_empty());
        }

        // 4. Status for the operator.
        if now_ms - last_status_ms >= 2000 {
            last_status_ms = now_ms;
            let h = health.lock().unwrap().clone();
            let (news_ok, news_failed, news_last, n_events) = {
                let s = store.lock().unwrap();
                (s.ok, s.failed, s.last_run, s.events.len())
            };
            let equity = ledger.equity(&marks);
            let assets: Vec<Value> = cfg
                .assets
                .iter()
                .map(|a| {
                    let f = feats.get(&a.symbol);
                    let lb = blocks.get(&(a.symbol.clone(), Side::Long)).cloned().unwrap_or_default();
                    let sb = blocks.get(&(a.symbol.clone(), Side::Short)).cloned();
                    // The side the flow currently leans to, for a one-line "waiting because".
                    let leaning = if sc.shorts && f.is_some_and(|f| f.z_rev > 0.0) { Side::Short } else { Side::Long };
                    let block = if leaning == Side::Short { sb.clone().unwrap_or_default() } else { lb.clone() };
                    serde_json::json!({
                        "symbol": a.symbol,
                        "coin": a.coin,
                        "features": f,
                        "news": ctx.get(&a.symbol),
                        "position": ledger.positions.get(&a.symbol),
                        "leaning": leaning,
                        "blocked_by": if block.is_empty() { None } else { Some(block) },
                        "long_block": lb,
                        "short_block": sb,
                        "held": ledger.held(&a.symbol),
                        "exchange_review": exch_review.get(&a.symbol),
                        "resting": pending_entries.get(&a.symbol).map(|p| serde_json::json!({"side": p.side, "px": p.px, "usd": p.usd, "age_ms": now_ms - p.posted_ms})),
                    })
                })
                .collect();
            let status = serde_json::json!({
                "ts": now,
                "pid": std::process::id(),
                "venue": venue.name(),
                "live": is_live,
                "network": cfg.exchange.network.name(),
                "feed": { "connected": h.connected, "last_msg_ms_ago": now_ms - h.last_msg_ms, "reconnects": h.reconnects, "last_error": h.last_error },
                "equity": equity,
                "cash": ledger.cash,
                "starting_cash": ledger.starting_cash,
                "capital": ledger.capital(),
                "open_notional": ledger.open_notional(&marks),
                "withdrawable": withdrawable,
                "day_trade_pnl": ledger.day_trade_pnl,
                "day_pnl_pct": ledger.day_trade_pnl / ledger.capital() * 100.0,
                "realized": ledger.realized,
                "wins": ledger.wins,
                "losses": ledger.losses,
                "halted_today": ledger.halted_day == Some(ledger.day),
                "live_hold": ledger.live_hold,
                "candidates_session": n.candidates,
                "feed_source": "hyperliquid",
                "perps": true,
                "entry_mode": sc.entry,
                "maker": { "filled": maker_filled, "missed": maker_missed, "resting": pending_entries.len() },
                "jev": { "reviews": n.reviews, "vetoes": n.vetoes, "failures": n.review_fail, "news_ok": news_ok, "news_failed": news_failed, "news_last_run": news_last, "events": n_events },
                "assets": assets,
                "settings": &sc,
            });
            let _ = write_atomic(&files.status, &serde_json::to_vec_pretty(&status).unwrap_or_default());
        }
    }
}

async fn open_ledger(venue: &Venue, cfg: &Config, path: &Path) -> Result<Ledger> {
    let live = matches!(venue, Venue::Live(_));
    if path.exists() {
        let l: Ledger = serde_json::from_slice(&std::fs::read(path)?).with_context(|| format!("parsing {}", path.display()))?;
        if (l.mode.as_deref() == Some(LIVE_MODE)) != live {
            bail!("{} was created in {} mode; use a separate data_dir", path.display(), if live { "paper" } else { "live" });
        }
        let made_for = l.venue.as_deref().unwrap_or(if live { "binance" } else { "hyperliquid-paper" });
        if made_for != venue.name() {
            bail!("{} was created for {made_for}, not {}; use a separate data_dir", path.display(), venue.name());
        }
        return Ok(l);
    }
    let cash = match venue {
        Venue::Paper => cfg.scalp.starting_cash,
        Venue::Live(l) => {
            let cap = cfg.exchange.max_capital_usd;
            if !(cap.is_finite() && cap > 0.0) {
                bail!("set [exchange] max_capital_usd");
            }
            let acct = l.ex.account().await?;
            for a in &cfg.assets {
                let szi = acct.positions.get(&a.coin).copied().unwrap_or(0.0);
                if szi != 0.0 {
                    bail!("the account already holds {szi} {} on Hyperliquid; close it or drop {} from the config, so the ledger starts in sync", a.coin, a.symbol);
                }
            }
            cap.min(acct.withdrawable)
        }
    };
    if cash < cfg.scalp.min_trade_usd.max(venue.min_notional()) {
        bail!("only {cash:.2} available to the scalper");
    }
    let l = Ledger::new(cash, live, venue.name());
    l.save(path)?;
    Ok(l)
}

/// Clears the scalper's hold (after an unconfirmed order). Returns whether one was set.
pub fn clear_hold(data_dir: &Path) -> Result<bool> {
    let path = Files::new(data_dir).state;
    if !path.exists() {
        return Ok(false);
    }
    let mut l: Ledger = serde_json::from_slice(&std::fs::read(&path)?).with_context(|| format!("parsing {}", path.display()))?;
    match l.live_hold.take() {
        Some(h) => {
            l.save(&path)?;
            println!("scalper ledger: cleared {h}");
            Ok(true)
        }
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(bid: f64, ask: f64) -> Features {
        Features { bid, ask, mid: (bid + ask) / 2.0, ..Default::default() }
    }

    #[tokio::test]
    async fn paper_perp_round_trips() {
        let cfg = ScalpConfig { fee_bps: 10.0, slippage_bps: 0.0, ..Default::default() };
        let mut l = Ledger::new(1000.0, false, "hyperliquid-paper");
        let v = Venue::Paper;

        // Short 0.5 at 100, buy back at 98: gross +1.00, fees 0.05 + 0.049.
        let e = v.trade("BTC", false, 0.5, false, Some(&at(100.0, 100.0)), &cfg).await.unwrap();
        l.apply("BTC", &e);
        assert!((l.held("BTC") + 0.5).abs() < 1e-12, "a short is a negative position");
        let marks: HashMap<String, f64> = [("BTC".to_string(), 100.0)].into();
        assert!((l.equity(&marks) - (1000.0 - 0.05)).abs() < 1e-9, "only the fee is lost at entry");
        let x = v.trade("BTC", true, 0.5, true, Some(&at(98.0, 98.0)), &cfg).await.unwrap();
        l.apply("BTC", &x);
        let pnl = e.cash + x.cash + (e.base + x.base) * x.price;
        assert!((pnl - (1.0 - 0.05 - 0.049)).abs() < 1e-9, "pnl {pnl}");
        assert!(l.held("BTC").abs() < 1e-12, "flat again");

        // Long 1 at 100, sell at 101: gross +1.00, fees 0.10 + 0.101.
        let e = v.trade("BTC", true, 1.0, false, Some(&at(100.0, 100.0)), &cfg).await.unwrap();
        l.apply("BTC", &e);
        let x = v.trade("BTC", false, 1.0, true, Some(&at(101.0, 101.0)), &cfg).await.unwrap();
        l.apply("BTC", &x);
        let pnl2 = e.cash + x.cash + (e.base + x.base) * x.price;
        assert!((pnl2 - (1.0 - 0.1 - 0.101)).abs() < 1e-9, "pnl {pnl2}");
        assert!((l.equity(&marks) - (1000.0 + pnl + pnl2)).abs() < 1e-9);
    }

    #[test]
    fn prices_keep_five_significant_digits() {
        assert_eq!(fmt_px(0.0052134), "0.0052134");
        assert_eq!(fmt_px(3.63091), "3.6309");
        assert_eq!(fmt_px(1174.5), "1174.5");
        assert_eq!(fmt_px(98765.4), "98765");
    }

    #[test]
    fn partial_exit_keeps_the_rest_open() {
        let ev = EntryEval { side: Side::Long, tp_bps: 20.0, sl_bps: 10.0, cost_bps: 7.5, ..Default::default() };
        let entry = maker_exec(true, 2.0, 100.0, 0.03, "e".into());
        let mut p = new_pos(Side::Long, &entry, &ev, 0, &ScalpConfig::default(), "c", 1.0, None);
        // Sell 1 of 2 at 101 with a 0.05 fee: half the entry cost (100.015) against 100.95.
        let x = Exec { cash: 101.0 - 0.05, base: -1.0, price: 101.0, fee_usd: 0.05, order_id: "x".into() };
        let pnl = close_part(&mut p, &x);
        assert!((pnl - (100.95 - 100.015)).abs() < 1e-9, "pnl {pnl}");
        assert!((p.qty - 1.0).abs() < 1e-12);
        assert!((p.entry_cash + 100.015).abs() < 1e-9, "the open half keeps half the entry cost");
    }
}
