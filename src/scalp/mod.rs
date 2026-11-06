//! `jev-trader scalp`: an order-flow scalper.
//!
//! - **Pace:** market events. A websocket keeps best bid/ask and every trade; the
//!   engine evaluates every `eval_ms`.
//! - **Decision:** microstructure (activity burst, trade-flow and book imbalance,
//!   short push, no chasing) plus a cost gate: the typical move over the holding
//!   period must pay for the round trip several times over. The same gates,
//!   mirrored, open shorts: spot can't borrow, so shorts sell from a neutral coin
//!   inventory and buy it back. Trade P&L is reported apart from inventory drift.
//! - **Exits:** take profit, stop (trailing to break-even after fees once in
//!   profit), flow reversal, hack alert (longs), time stop.
//! - **Thought process:** news never starts a trade. The slow news loop tilts the
//!   flow threshold and vetoes on hacks; at entry time Jev reviews the proposed
//!   trade against recent headlines and can block it or halve its size.
//! - **Evidence:** every candidate (taken or not) is logged with its gates and Jev's
//!   review, then marked out at fixed horizons net of costs.

pub mod engine;
pub mod features;

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

use crate::binance::Qty;
use crate::bot::append_jsonl;
use crate::config::{Config, ScalpConfig};
use crate::jev::JevClient;
use crate::live::{Booked, Live};
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
        }
    }
}

// ---------------------------------------------------------------- ledger

/// The neutral coin inventory for one asset: what shorts sell from and buy back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inventory {
    pub qty: f64,
    /// Price when it was allotted (for reporting inventory drift apart from trading).
    pub px: f64,
}

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub mode: Option<String>,
    pub cash: f64,
    /// Cash when the ledger was created.
    pub starting_cash: f64,
    #[serde(default)]
    pub inventory: BTreeMap<String, Inventory>,
    /// Coins the ledger owns now: inventory ± open trades.
    #[serde(default)]
    pub held: BTreeMap<String, f64>,
    pub positions: BTreeMap<String, ScalpPos>,
    pub day: NaiveDate,
    pub day_start_equity: f64,
    /// Trade P&L today; the daily loss limit applies to this, not to inventory drift.
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
    fn new(cash: f64, live: bool) -> Self {
        Self {
            mode: live.then(|| LIVE_MODE.to_string()),
            cash,
            starting_cash: cash,
            inventory: BTreeMap::new(),
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

    /// Cash plus every coin the ledger owns, at the bid.
    fn equity(&self, bids: &HashMap<String, f64>) -> f64 {
        self.cash
            + self
                .held
                .iter()
                .map(|(s, q)| q * bids.get(s).copied().or_else(|| self.inventory.get(s).map(|i| i.px)).unwrap_or(0.0))
                .sum::<f64>()
    }

    /// What the ledger was given: cash plus the inventory at its allotment price.
    fn capital(&self) -> f64 {
        self.starting_cash + self.inventory.values().map(|i| i.qty * i.px).sum::<f64>()
    }

    /// Price drift of the neutral inventory, kept apart from trading results.
    fn inventory_drift(&self, bids: &HashMap<String, f64>) -> f64 {
        self.inventory.iter().map(|(s, i)| i.qty * (bids.get(s).copied().unwrap_or(i.px) - i.px)).sum()
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
            Exec { cash: -b.cash_out, base: b.qty, price: b.price, fee_usd: b.fee_usd, order_id: b.order_id }
        } else {
            Exec { cash: b.cash_in, base: -b.qty, price: b.price, fee_usd: b.fee_usd, order_id: b.order_id }
        }
    }
}

enum Amount {
    /// Spend this much quote (buys only).
    Quote(f64),
    /// Buy or sell this much of the coin.
    Base(f64),
}

enum Venue {
    Paper,
    Binance(Live),
}

impl Venue {
    fn name(&self) -> &'static str {
        match self {
            Venue::Paper => "paper",
            Venue::Binance(l) if l.testnet => "binance-testnet",
            Venue::Binance(_) => "binance",
        }
    }

    fn min_notional(&self, sym: &str) -> f64 {
        match self {
            Venue::Paper => 0.0,
            Venue::Binance(l) => l.rules.get(sym).map(|(_, r)| r.min_notional * 1.02).unwrap_or(0.0),
        }
    }

    /// Rounds a coin quantity down to what the venue accepts.
    fn floor_qty(&self, sym: &str, q: f64) -> f64 {
        match self {
            Venue::Paper => q,
            Venue::Binance(l) => l.rules.get(sym).map(|(_, r)| r.floor_qty(q)).unwrap_or(0.0),
        }
    }

    async fn trade(&self, sym: &str, buy: bool, amt: Amount, f: Option<&Features>, cfg: &ScalpConfig, now: DateTime<Utc>) -> std::result::Result<Exec, String> {
        match self {
            Venue::Paper => {
                let f = f.ok_or("no price to fill a paper order")?;
                let fee = cfg.fee_bps / 1e4;
                if buy {
                    let px = f.ask * (1.0 + cfg.slippage_bps / 1e4);
                    let (qty, notional) = match amt {
                        Amount::Quote(usd) => (usd / px, usd),
                        Amount::Base(q) => (q, q * px),
                    };
                    Ok(Exec { cash: -(notional * (1.0 + fee)), base: qty, price: px, fee_usd: notional * fee, order_id: "paper".into() })
                } else {
                    let Amount::Base(q) = amt else { return Err("a sell needs a coin quantity".into()) };
                    let px = f.bid * (1.0 - cfg.slippage_bps / 1e4);
                    Ok(Exec { cash: q * px * (1.0 - fee), base: -q, price: px, fee_usd: q * px * fee, order_id: "paper".into() })
                }
            }
            Venue::Binance(l) => {
                let qty = match amt {
                    Amount::Quote(usd) => Qty::Quote(usd),
                    Amount::Base(q) => {
                        let (_, r) = l.rules.get(sym).ok_or("not configured")?;
                        let q = r.floor_qty(q);
                        if q < r.min_qty {
                            return Err(format!("{q} is below the minimum order size"));
                        }
                        Qty::Base(r.fmt_qty(q))
                    }
                };
                crate::live::market_fill(l, sym, buy, qty, now, cfg.fee_bps / 1e4).await.map(|b| Exec::from_booked(b, buy))
            }
        }
    }
}

// ---------------------------------------------------------------- shared state

#[derive(Debug, Default, Clone, Serialize)]
struct FeedHealth {
    connected: bool,
    reconnects: u32,
    last_msg_ms: i64,
    last_error: String,
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

type Markets = Arc<Mutex<HashMap<String, AssetMarket>>>;

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

async fn feed_task(url: String, pairs: Vec<(String, String)>, markets: Markets, health: Arc<Mutex<FeedHealth>>) {
    let by_pair: HashMap<String, String> = pairs.iter().map(|(p, s)| (p.to_uppercase(), s.clone())).collect();
    let streams: Vec<String> = pairs.iter().flat_map(|(p, _)| [format!("{}@bookTicker", p.to_lowercase()), format!("{}@aggTrade", p.to_lowercase())]).collect();
    let full = format!("{}/stream?streams={}", url.trim_end_matches('/'), streams.join("/"));
    let mut backoff = 1u64;
    loop {
        match tokio_tungstenite::connect_async(&full).await {
            Ok((mut ws, _)) => {
                info!("market stream connected ({} streams)", streams.len());
                backoff = 1;
                health.lock().unwrap().connected = true;
                loop {
                    let msg = match tokio::time::timeout(Duration::from_secs(30), ws.next()).await {
                        Ok(Some(Ok(m))) => m,
                        Ok(Some(Err(e))) => {
                            health.lock().unwrap().last_error = format!("{e}");
                            break;
                        }
                        Ok(None) => break,
                        Err(_) => {
                            health.lock().unwrap().last_error = "no data for 30s".into();
                            break;
                        }
                    };
                    let text = match msg {
                        tokio_tungstenite::tungstenite::Message::Text(t) => t,
                        tokio_tungstenite::tungstenite::Message::Ping(p) => {
                            let _ = ws.send(tokio_tungstenite::tungstenite::Message::Pong(p)).await;
                            continue;
                        }
                        tokio_tungstenite::tungstenite::Message::Close(_) => break,
                        _ => continue,
                    };
                    let now = Utc::now().timestamp_millis();
                    let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                    let d = &v["data"];
                    let num = |k: &str| d[k].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                    let Some(sym) = d["s"].as_str().and_then(|p| by_pair.get(p)) else { continue };
                    let stream = v["stream"].as_str().unwrap_or_default();
                    let mut m = markets.lock().unwrap();
                    let am = m.entry(sym.clone()).or_default();
                    if stream.ends_with("@bookTicker") {
                        am.on_book(num("b"), num("B"), num("a"), num("A"), now);
                    } else if stream.ends_with("@aggTrade") {
                        am.on_trade(num("p"), num("q"), d["m"].as_bool().unwrap_or(false), now);
                    }
                    drop(m);
                    health.lock().unwrap().last_msg_ms = now;
                }
            }
            Err(e) => health.lock().unwrap().last_error = format!("connect: {e}"),
        }
        {
            let mut h = health.lock().unwrap();
            h.connected = false;
            h.reconnects += 1;
            warn!("market stream down ({}); reconnecting in {backoff}s", h.last_error);
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

pub async fn run(cfg: Config, jev: JevClient, live: Option<Live>) -> Result<()> {
    let sc = cfg.scalp.clone();
    let files = Files::new(&cfg.data_dir);
    crate::status::acquire_pid_lock(&cfg.data_dir.join("scalp.pid"))?;
    let venue = match live {
        Some(l) => Venue::Binance(l),
        None => Venue::Paper,
    };
    let mut ledger = open_ledger(&venue, &cfg, &files.state).await?;
    info!(venue = venue.name(), cash = ledger.cash, positions = ledger.positions.len(), shorts = sc.shorts, "scalper ledger loaded");

    let markets: Markets = Arc::new(Mutex::new(HashMap::new()));
    let health = Arc::new(Mutex::new(FeedHealth::default()));
    let store: Arc<Mutex<NewsStore>> = Arc::new(Mutex::new(
        std::fs::read(&files.news).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default(),
    ));
    let pairs: Vec<(String, String)> = cfg.assets.iter().map(|a| (a.pair.clone(), a.symbol.clone())).collect();
    tokio::spawn(feed_task(sc.stream_url.clone(), pairs, markets.clone(), health.clone()));
    tokio::spawn(news_task(cfg.clone(), jev.clone(), store.clone(), files.news.clone(), files.events.clone()));

    let mut reviews: HashMap<(String, Side), ReviewCache> = HashMap::new();
    let mut last_candidate_ms: HashMap<String, i64> = HashMap::new();
    let mut pending: Vec<PendingMarkout> = Vec::new();
    let mut balances: Option<HashMap<String, f64>> = None;
    let mut exch_review: HashMap<String, String> = HashMap::new();
    let mut inventory_note: HashMap<String, String> = HashMap::new();
    let mut last_reconcile_ms = 0i64;
    let mut last_status_ms = 0i64;
    let mut blocks: HashMap<(String, Side), String> = HashMap::new();
    let mut n = Counts::default();
    let mut ticker = tokio::time::interval(Duration::from_millis(sc.eval_ms.max(100)));

    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = tokio::signal::ctrl_c() => {
                info!("shutting down; open positions stay in the ledger and are managed on restart");
                ledger.save(&files.state)?;
                return Ok(());
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
        let bids: HashMap<String, f64> = feats.iter().map(|(k, f)| (k.clone(), f.bid)).collect();

        // Day roll; the daily limit is on trade P&L, never on inventory drift.
        if ledger.day != now.date_naive() {
            ledger.day = now.date_naive();
            ledger.day_start_equity = ledger.equity(&bids);
            ledger.day_trade_pnl = 0.0;
        }
        if ledger.day_trade_pnl < -sc.daily_loss_limit_pct * ledger.capital() && ledger.halted_day != Some(ledger.day) {
            warn!(pnl = ledger.day_trade_pnl, "daily loss limit hit: no entries until tomorrow (UTC)");
            ledger.halted_day = Some(ledger.day);
        }

        // Live: the exchange must hold every coin the ledger owns (every 60s).
        if let Venue::Binance(l) = &venue
            && now_ms - last_reconcile_ms > 60_000
        {
            last_reconcile_ms = now_ms;
            match l.client.balances().await {
                Ok(b) => {
                    exch_review.clear();
                    for (sym, (_, r)) in &l.rules {
                        let want = ledger.held(sym);
                        let have = b.get(&r.base).copied().unwrap_or(0.0);
                        if want > 0.0 && have + r.step < want * (1.0 - 1e-6) {
                            exch_review.insert(sym.clone(), format!("Binance holds {have} {}, ledger expects {want:.8}", r.base));
                        }
                    }
                    balances = Some(b);
                }
                Err(e) => {
                    warn!("balance check failed: {e:#}");
                    balances = None;
                }
            }
        }

        // Allot the neutral inventory that shorts sell from (once per asset).
        if sc.shorts && sc.inventory_usd > 0.0 {
            let mut changed = false;
            for a in &cfg.assets {
                let sym = &a.symbol;
                if ledger.inventory.contains_key(sym) {
                    continue;
                }
                let Some(f) = feats.get(sym) else { continue };
                let want = sc.inventory_usd / f.mid;
                let qty = match (&venue, &balances) {
                    (Venue::Paper, _) => want,
                    (Venue::Binance(l), Some(b)) => {
                        let base = &l.rules[sym].1.base;
                        let spare = b.get(base).copied().unwrap_or(0.0) - ledger.held(sym);
                        venue.floor_qty(sym, want.min(spare.max(0.0)))
                    }
                    (Venue::Binance(_), None) => continue,
                };
                if qty * f.mid < venue.min_notional(sym).max(sc.min_trade_usd) {
                    inventory_note.insert(sym.clone(), format!("no {sym} inventory to sell: shorts off for {sym}"));
                    ledger.inventory.insert(sym.clone(), Inventory { qty: 0.0, px: f.mid });
                } else {
                    info!(qty, px = f.mid, "allotted {sym} inventory for shorts (${:.0})", qty * f.mid);
                    ledger.inventory.insert(sym.clone(), Inventory { qty, px: f.mid });
                    *ledger.held.entry(sym.clone()).or_default() += qty;
                }
                changed = true;
            }
            if changed {
                ledger.save(&files.state)?;
            }
        }

        // 1. Exits (trailing the stop first).
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
            let op = OpenPos { side: p.side, tp_px: p.tp_px, sl_px: p.sl_px, deadline_ms: p.deadline_ms };
            let Some(mut reason) = engine::evaluate_exit(&op, feats.get(&sym), &news, now_ms, &sc) else { continue };
            if reason == "stop" && p.side.sign() * (p.sl_px - p.init_sl_px) > 0.0 {
                reason = "trailing stop".into();
            }
            let res = match p.side {
                // Sell what the long bought (never more than the ledger holds).
                Side::Long => venue.trade(&sym, false, Amount::Base(p.qty.min(ledger.held(&sym))), feats.get(&sym), &sc, now).await,
                // Buy back what the short sold, plus the fee Binance takes in the coin.
                Side::Short => {
                    let q = if matches!(venue, Venue::Paper) { p.qty } else { p.qty * (1.0 + sc.fee_bps / 1e4) };
                    venue.trade(&sym, true, Amount::Base(q), feats.get(&sym), &sc, now).await
                }
            };
            match res {
                Ok(x) => {
                    ledger.apply(&sym, &x);
                    let pnl = p.entry_cash + x.cash + (p.entry_base + x.base) * x.price;
                    let notional = p.qty * p.entry_px;
                    ledger.realized += pnl;
                    ledger.day_trade_pnl += pnl;
                    ledger.positions.remove(&sym);
                    if pnl < 0.0 {
                        ledger.losses += 1;
                        ledger.cooldown_until_ms.insert(sym.clone(), now_ms + sc.cooldown_after_loss_secs * 1000);
                    } else {
                        ledger.wins += 1;
                    }
                    info!(
                        "{} close {} {sym} @ {:.2} — {reason} · pnl {pnl:+.2} ({:+.1} bps)",
                        venue.name().to_uppercase(),
                        p.side.word(),
                        x.price,
                        pnl / notional * 1e4
                    );
                    append_jsonl(
                        &files.trades,
                        &TradeRecord {
                            asset: &sym,
                            side: p.side,
                            venue: venue.name(),
                            opened: DateTime::from_timestamp_millis(p.opened_ms).unwrap_or(now),
                            closed: now,
                            hold_secs: (now_ms - p.opened_ms) / 1000,
                            qty: p.qty,
                            entry_px: p.entry_px,
                            exit_px: x.price,
                            notional,
                            fees_usd: p.entry_fee + x.fee_usd,
                            pnl_usd: pnl,
                            pnl_bps: pnl / notional * 1e4,
                            reason: &reason,
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
                    if m.starts_with("UNCONFIRMED") {
                        ledger.live_hold = Some(m);
                        ledger.save(&files.state)?;
                    }
                }
            }
        }

        // 2. Entries, each side judged by the same mirrored gates.
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
            // Flow can only point one way at a time, so at most one side is a candidate.
            let Some(ev) = evals.into_iter().find(|e| e.candidate) else { continue };
            // Before the baseline exists, activity and volatility are unreliable.
            if f.history_secs < sc.warmup_secs {
                continue;
            }
            // One candidate per asset per flow window, so a burst isn't counted 60 times.
            if last_candidate_ms.get(sym).is_some_and(|t| now_ms - t < sc.flow_window_secs * 1000) {
                continue;
            }
            last_candidate_ms.insert(sym.clone(), now_ms);
            n.candidates += 1;
            let side = ev.side;

            let capital = ledger.capital();
            let free_quote = match (&venue, &balances) {
                (Venue::Binance(l), Some(b)) => b.get(&l.quote).copied().unwrap_or(0.0),
                (Venue::Binance(_), None) => 0.0,
                (Venue::Paper, _) => f64::INFINITY,
            };
            // Longs spend cash; shorts sell from the inventory (never below zero).
            let room_usd = match side {
                Side::Long => ledger.cash.min(free_quote / (1.0 + sc.fee_bps / 1e4)),
                Side::Short => ledger.held(sym) * f.bid,
            };
            let base_usd = (capital * sc.position_pct).min(room_usd);
            let min_usd = sc.min_trade_usd.max(venue.min_notional(sym));
            let mut risk = Vec::new();
            let mut rg = |name: &str, pass: bool, detail: String| {
                risk.push(GateResult::new(name, pass, detail));
                pass
            };
            let mut ok = ev.ok;
            ok &= rg("flat", !ledger.positions.contains_key(sym), String::new());
            ok &= rg("max positions", ledger.positions.len() < sc.max_positions, format!("{}/{}", ledger.positions.len(), sc.max_positions));
            ok &= rg("cooldown", ledger.cooldown_until_ms.get(sym).is_none_or(|t| now_ms >= *t), String::new());
            ok &= rg("trades/hour", ledger.entries_ms.len() < sc.max_trades_per_hour, format!("{}/{}", ledger.entries_ms.len(), sc.max_trades_per_hour));
            ok &= rg("daily limit", ledger.halted_day != Some(ledger.day), String::new());
            ok &= rg("exchange", ledger.live_hold.is_none() && !exch_review.contains_key(sym), ledger.live_hold.clone().or_else(|| exch_review.get(sym).cloned()).unwrap_or_default());
            ok &= rg(
                if side == Side::Long { "cash" } else { "inventory" },
                base_usd >= min_usd,
                format!("{base_usd:.2} (≥ {min_usd:.2})"),
            );

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
            let blocked_by = ev.first_block().map(str::to_string).or_else(|| risk.iter().find(|g| !g.passed).map(|g| g.gate.clone()));
            let mut taken = false;
            if ok && usd >= min_usd {
                let res = match side {
                    Side::Long => venue.trade(sym, true, Amount::Quote(usd), Some(f), &sc, now).await,
                    Side::Short => venue.trade(sym, false, Amount::Base(usd / f.bid), Some(f), &sc, now).await,
                };
                match res {
                    Ok(x) => {
                        taken = true;
                        ledger.apply(sym, &x);
                        ledger.entries_ms.push_back(now_ms);
                        let s = side.sign();
                        let pos = ScalpPos {
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
                            candidate_id: id.clone(),
                            size_mult,
                            review: review.clone(),
                            order_id: x.order_id.clone(),
                        };
                        info!(
                            "{} open {} {sym} {:.6} @ {:.2} (${:.2}) · tp {:.2} · stop {:.2} · flow {:+.2} book {:+.2} activity {:.1}×",
                            venue.name().to_uppercase(),
                            side.word(),
                            pos.qty,
                            x.price,
                            pos.qty * x.price,
                            pos.tp_px,
                            pos.sl_px,
                            f.flow,
                            f.obi,
                            f.activity
                        );
                        ledger.positions.insert(sym.clone(), pos);
                        ledger.save(&files.state)?;
                    }
                    Err(m) => {
                        error!("{sym} {} entry failed: {m}", side.word());
                        if m.starts_with("UNCONFIRMED") {
                            ledger.live_hold = Some(m);
                            ledger.save(&files.state)?;
                        }
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
            let equity = ledger.equity(&bids);
            let assets: Vec<Value> = cfg
                .assets
                .iter()
                .map(|a| {
                    let f = feats.get(&a.symbol);
                    let lb = blocks.get(&(a.symbol.clone(), Side::Long)).cloned().unwrap_or_default();
                    let sb = blocks.get(&(a.symbol.clone(), Side::Short)).cloned();
                    // The side the flow currently leans to, for a one-line "waiting because".
                    let leaning = if sc.shorts && f.is_some_and(|f| f.flow < 0.0) { Side::Short } else { Side::Long };
                    let block = if leaning == Side::Short { sb.clone().unwrap_or_default() } else { lb.clone() };
                    serde_json::json!({
                        "symbol": a.symbol,
                        "features": f,
                        "news": ctx.get(&a.symbol),
                        "position": ledger.positions.get(&a.symbol),
                        "leaning": leaning,
                        "blocked_by": if block.is_empty() { None } else { Some(block) },
                        "long_block": lb,
                        "short_block": sb,
                        "inventory": ledger.inventory.get(&a.symbol),
                        "held": ledger.held(&a.symbol),
                        "inventory_note": inventory_note.get(&a.symbol),
                    })
                })
                .collect();
            let status = serde_json::json!({
                "ts": now,
                "pid": std::process::id(),
                "venue": venue.name(),
                "feed": { "connected": h.connected, "last_msg_ms_ago": now_ms - h.last_msg_ms, "reconnects": h.reconnects, "last_error": h.last_error },
                "equity": equity,
                "cash": ledger.cash,
                "starting_cash": ledger.starting_cash,
                "capital": ledger.capital(),
                "inventory_drift": ledger.inventory_drift(&bids),
                "day_trade_pnl": ledger.day_trade_pnl,
                "day_pnl_pct": ledger.day_trade_pnl / ledger.capital() * 100.0,
                "realized": ledger.realized,
                "wins": ledger.wins,
                "losses": ledger.losses,
                "halted_today": ledger.halted_day == Some(ledger.day),
                "live_hold": ledger.live_hold,
                "candidates_session": n.candidates,
                "jev": { "reviews": n.reviews, "vetoes": n.vetoes, "failures": n.review_fail, "news_ok": news_ok, "news_failed": news_failed, "news_last_run": news_last, "events": n_events },
                "assets": assets,
                "settings": &sc,
            });
            let _ = write_atomic(&files.status, &serde_json::to_vec_pretty(&status).unwrap_or_default());
        }
    }
}

async fn open_ledger(venue: &Venue, cfg: &Config, path: &Path) -> Result<Ledger> {
    let live = matches!(venue, Venue::Binance(_));
    if path.exists() {
        let l: Ledger = serde_json::from_slice(&std::fs::read(path)?).with_context(|| format!("parsing {}", path.display()))?;
        if (l.mode.as_deref() == Some(LIVE_MODE)) != live {
            bail!("{} was created in {} mode; use a separate data_dir", path.display(), if live { "paper" } else { "live" });
        }
        return Ok(l);
    }
    let cash = match venue {
        Venue::Paper => cfg.scalp.starting_cash,
        Venue::Binance(l) => {
            let cap = cfg.exchange.max_capital_usd;
            if !(cap.is_finite() && cap > 0.0) {
                bail!("set [exchange] max_capital_usd");
            }
            let free = l.client.balances().await?.get(&l.quote).copied().unwrap_or(0.0);
            cap.min(free)
        }
    };
    if cash < cfg.scalp.min_trade_usd {
        bail!("only {cash:.2} available to the scalper");
    }
    let l = Ledger::new(cash, live);
    l.save(path)?;
    Ok(l)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(bid: f64, ask: f64) -> Features {
        Features { bid, ask, mid: (bid + ask) / 2.0, ..Default::default() }
    }

    #[tokio::test]
    async fn paper_round_trips_book_trade_pnl_apart_from_inventory() {
        let cfg = ScalpConfig { fee_bps: 10.0, slippage_bps: 0.0, ..Default::default() };
        let now = Utc::now();
        let mut l = Ledger::new(1000.0, false);
        l.inventory.insert("BTC".into(), Inventory { qty: 1.0, px: 100.0 });
        l.held.insert("BTC".into(), 1.0);

        // Short 0.5 at 100, buy back at 98: gross +1.00, fees 0.05 + 0.049.
        let v = Venue::Paper;
        let e = v.trade("BTC", false, Amount::Base(0.5), Some(&at(100.0, 100.0)), &cfg, now).await.unwrap();
        l.apply("BTC", &e);
        assert!((l.held("BTC") - 0.5).abs() < 1e-12, "sold half the inventory");
        let x = v.trade("BTC", true, Amount::Base(0.5), Some(&at(98.0, 98.0)), &cfg, now).await.unwrap();
        l.apply("BTC", &x);
        let pnl = e.cash + x.cash + (e.base + x.base) * x.price;
        assert!((pnl - (1.0 - 0.05 - 0.049)).abs() < 1e-9, "pnl {pnl}");
        assert!((l.held("BTC") - 1.0).abs() < 1e-12, "inventory restored");

        // Long $100 at 100, sell at 101: gross +1.00, fees 0.10 + 0.101.
        let e = v.trade("BTC", true, Amount::Quote(100.0), Some(&at(100.0, 100.0)), &cfg, now).await.unwrap();
        l.apply("BTC", &e);
        let x = v.trade("BTC", false, Amount::Base(e.base), Some(&at(101.0, 101.0)), &cfg, now).await.unwrap();
        l.apply("BTC", &x);
        let pnl2 = e.cash + x.cash + (e.base + x.base) * x.price;
        assert!((pnl2 - (1.0 - 0.1 - 0.101)).abs() < 1e-9, "pnl {pnl2}");

        // Equity = capital + trade P&L + inventory drift (price now 101 vs 100 at allotment).
        let bids: HashMap<String, f64> = [("BTC".to_string(), 101.0)].into();
        assert!((l.inventory_drift(&bids) - 1.0).abs() < 1e-9);
        assert!((l.equity(&bids) - (l.capital() + pnl + pnl2 + 1.0)).abs() < 1e-9);
    }
}
