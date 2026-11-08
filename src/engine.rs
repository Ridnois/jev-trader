//! `jev-trader run`: the combined engine. One process, one ledger.
//!
//! - **News** (`intel`): every source → local tagger → batched Jev → events and a
//!   decayed bias per coin. Strong fresh events open catalyst *watches*.
//! - **Market** (`feed`): Hyperliquid websocket per coin (watch list + any coin a
//!   catalyst names), evaluated every `flow.eval_ms`.
//! - **Entries**: catalyst (headline confirmed by flow, not chased, Jev review) and
//!   flow (microstructure burst that agrees with the news bias).
//! - **Exits**: resting maker take-profit, stop and time stop (taker), news turned,
//!   hack alert, trailing to break-even after costs. Live positions also carry a
//!   reduce-only stop-market on the exchange in case this process dies.
//! - **Evidence**: every catalyst-grade headline and every flow candidate is marked
//!   out at fixed horizons, taken or not (`markouts.jsonl`), so `report` can say
//!   whether the news reading pays.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::config::{Config, EntryMode};
use crate::features::Features;
use crate::feed::{self, FeedHealth, Markets};
use crate::intel::{self, Bias, Intel, JevGate, NewsEvent, Review, tagger::Tagger};
use crate::jev::JevClient;
use crate::ledger::{Exec, Ledger, Position, close_part};
use crate::live::{Live, MIN_ORDER_USD};
use crate::status::{Control, Files, Gate, append_jsonl, first_block, write_atomic};
use crate::strategy::{self, OpenPos, Plan, Side, TradeKind, Watch};

// ---------------------------------------------------------------- venue

enum Venue {
    Paper,
    Live(Arc<Live>),
}

impl Venue {
    fn name(&self) -> &'static str {
        match self {
            Venue::Paper => "hyperliquid-paper",
            Venue::Live(l) => l.venue(),
        }
    }

    fn live(&self) -> Option<&Live> {
        match self {
            Venue::Live(l) => Some(l),
            Venue::Paper => None,
        }
    }

    /// A handle that doesn't borrow the engine (so the ledger can change meanwhile).
    fn live_arc(&self) -> Option<Arc<Live>> {
        match self {
            Venue::Live(l) => Some(l.clone()),
            Venue::Paper => None,
        }
    }

    fn step(&self, coin: &str) -> f64 {
        self.live().map(|l| l.step(coin)).unwrap_or(0.0)
    }

    /// A marketable order for `qty`; exits pass `reduce_only`.
    async fn taker(&self, coin: &str, buy: bool, qty: f64, reduce_only: bool, f: Option<&Features>, cfg: &Config) -> std::result::Result<Exec, String> {
        let f = f.ok_or("no price for the order")?;
        match self {
            Venue::Paper => {
                let c = &cfg.costs;
                let px = if buy { f.ask * (1.0 + c.slippage_bps / 1e4) } else { f.bid * (1.0 - c.slippage_bps / 1e4) };
                Ok(Exec::fill(buy, qty, px, qty * px * c.taker_bps / 1e4, "paper".into()))
            }
            Venue::Live(l) => {
                l.ensure_leverage(coin).await?;
                l.market_fill(coin, buy, qty, if buy { f.ask } else { f.bid }, reduce_only).await.map(|b| Exec::from_booked(b, buy))
            }
        }
    }
}

// ---------------------------------------------------------------- records

/// A post-only flow entry waiting for a trade-through (paper) or the exchange's fill (live).
struct PendingEntry {
    side: Side,
    px: f64,
    usd: f64,
    sz: f64,
    posted_ms: i64,
    cloid: Option<String>,
    last_poll_ms: i64,
    plan: Plan,
    candidate_id: String,
    why: String,
}

#[derive(Serialize)]
struct TradeRecord<'a> {
    coin: &'a str,
    kind: TradeKind,
    side: Side,
    venue: &'a str,
    opened: DateTime<Utc>,
    closed: DateTime<Utc>,
    hold_secs: i64,
    qty: f64,
    entry_px: f64,
    exit_px: f64,
    notional: f64,
    fees_usd: f64,
    funding_usd: f64,
    pnl_usd: f64,
    pnl_bps: f64,
    reason: &'a str,
    tp_px: f64,
    sl_px: f64,
    init_sl_px: f64,
    stop_bps: f64,
    cost_bps: f64,
    size_mult: f64,
    why: &'a str,
    review: Option<&'a Review>,
    candidate_id: &'a str,
    entry_order: &'a str,
    exit_order: &'a str,
}

/// A catalyst-grade headline marked out at fixed horizons (minutes), traded or not.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EventMarkout {
    id: String,
    coin: String,
    side: Side,
    impact: f64,
    new_info: f64,
    source: String,
    kind: intel::sources::Kind,
    lag_min: f64,
    t0_ms: i64,
    left: Vec<i64>,
    #[serde(default)]
    taken: bool,
}

struct FlowMarkout {
    id: String,
    coin: String,
    side: Side,
    taken: bool,
    bias: f64,
    aligned: bool,
    t0_sec: i64,
    mid0: f64,
    cost_bps: f64,
    left: Vec<i64>,
}

const EVENT_HORIZONS_MIN: [i64; 4] = [5, 15, 60, 240];

// ---------------------------------------------------------------- engine

struct Engine {
    cfg: Config,
    files: Files,
    venue: Venue,
    ledger: Ledger,
    gate: JevGate,
    intel: Intel,
    markets: Markets,
    health: Arc<Mutex<FeedHealth>>,
    sub_tx: mpsc::UnboundedSender<String>,
    subscribed: HashSet<String>,
    /// coin → 24h volume ($), refreshed every 10 minutes.
    universe: HashMap<String, f64>,
    universe_at: Option<std::time::Instant>,
    http: reqwest::Client,
    watches: Vec<Watch>,
    pending: HashMap<String, PendingEntry>,
    event_marks: Vec<EventMarkout>,
    flow_marks: Vec<FlowMarkout>,
    last_candidate_ms: HashMap<String, i64>,
    last_backstop_try: HashMap<String, i64>,
    withdrawable: Option<f64>,
    exch_review: HashMap<String, String>,
    last_reconcile_ms: i64,
    last_funding_ms: i64,
    last_marks_ms: i64,
    blocks: HashMap<String, String>,
    control: Control,
    session: Value,
}

pub async fn run(cfg: Config, jev: JevClient, live: Option<Live>) -> Result<()> {
    std::fs::create_dir_all(&cfg.data_dir)?;
    let files = Files::new(&cfg.data_dir);
    crate::status::acquire_pid_lock(&files.pid)?;
    let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).user_agent("jev-trader/2").build()?;
    let venue = match live {
        Some(l) => Venue::Live(Arc::new(l)),
        None => Venue::Paper,
    };
    let ledger = open_ledger(&venue, &cfg, &files).await?;
    info!(venue = venue.name(), cash = ledger.cash, positions = ledger.positions.len(), "ledger loaded");

    // Perp universe: the tagger needs every perp name; catalysts need volumes.
    let perps = crate::hyperliquid::perp_universe(&http, cfg.api()).await;
    let universe: HashMap<String, f64> = match &perps {
        Ok(p) => p.iter().map(|x| (x.coin.clone(), x.day_volume_usd)).collect(),
        Err(e) => {
            warn!("perp universe unavailable ({e:#}); news tagging limited to the watch list");
            cfg.watch.iter().map(|c| (c.clone(), f64::INFINITY)).collect()
        }
    };
    let names: Vec<String> = universe.keys().cloned().collect();
    let tagger = Arc::new(Tagger::new(&names));

    let markets: Markets = Arc::new(Mutex::new(HashMap::new()));
    let health = Arc::new(Mutex::new(FeedHealth::default()));
    let (sub_tx, sub_rx) = mpsc::unbounded_channel();
    let mut initial: Vec<String> = cfg.watch.clone();
    initial.extend(ledger.positions.keys().cloned());
    initial.sort();
    initial.dedup();
    tokio::spawn(feed::run(cfg.exchange.network.ws_url(), initial.clone(), sub_rx, markets.clone(), health.clone()));

    let gate = JevGate::new(jev, cfg.jev.max_calls_per_hour, &files);
    let intel = intel::load(&files);
    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<NewsEvent>();
    tokio::spawn(intel::run(cfg.clone(), gate.clone(), tagger, intel.clone(), files.clone(), ev_tx));

    let event_marks: Vec<EventMarkout> = crate::status::read_json(&files.pending_markouts).unwrap_or_default();
    let mut e = Engine {
        session: json!({ "started": Utc::now(), "pid": std::process::id() }),
        cfg,
        files,
        venue,
        ledger,
        gate,
        intel,
        markets,
        health,
        sub_tx,
        subscribed: initial.into_iter().collect(),
        universe,
        universe_at: Some(std::time::Instant::now()),
        http,
        watches: Vec::new(),
        pending: HashMap::new(),
        event_marks,
        flow_marks: Vec::new(),
        last_candidate_ms: HashMap::new(),
        last_backstop_try: HashMap::new(),
        withdrawable: None,
        exch_review: HashMap::new(),
        last_reconcile_ms: 0,
        last_funding_ms: Utc::now().timestamp_millis(),
        last_marks_ms: 0,
        blocks: HashMap::new(),
        control: Control::default(),
    };
    let mut ticker = tokio::time::interval(Duration::from_millis(e.cfg.flow.eval_ms.max(100)));
    let mut last_status = 0i64;
    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = tokio::signal::ctrl_c() => {
                e.shutdown().await;
                return Ok(());
            }
        }
        let now = Utc::now();
        let now_ms = now.timestamp_millis();
        let mut events = Vec::new();
        while let Ok(ev) = ev_rx.try_recv() {
            events.push(ev);
        }
        if let Err(err) = e.tick(now, events).await {
            error!("tick: {err:#}");
        }
        if now_ms - last_status >= 2000 {
            last_status = now_ms;
            e.write_status(now);
        }
    }
}

async fn open_ledger(venue: &Venue, cfg: &Config, files: &Files) -> Result<Ledger> {
    let live = venue.live().is_some();
    if let Some(l) = Ledger::open(&files.ledger, live, venue.name())? {
        return Ok(l);
    }
    let cash = match venue {
        Venue::Paper => cfg.risk.starting_cash,
        Venue::Live(l) => {
            let cap = cfg.exchange.max_capital_usd;
            if !(cap.is_finite() && cap > 0.0) {
                bail!("set [exchange] max_capital_usd to the most USDC the bot may trade with");
            }
            let acct = l.ex.account().await?;
            if let Some((c, q)) = acct.positions.iter().find(|(_, q)| **q != 0.0) {
                bail!("the account already holds {q} {c} on Hyperliquid; close it first so the ledger starts in sync");
            }
            cap.min(acct.withdrawable)
        }
    };
    if cash < cfg.risk.min_trade_usd.max(if live { MIN_ORDER_USD } else { 0.0 }) {
        bail!("only {cash:.2} available");
    }
    let l = Ledger::new(cash, live, venue.name());
    l.save(&files.ledger)?;
    Ok(l)
}

impl Engine {
    fn feats(&self, now_ms: i64) -> HashMap<String, Features> {
        let mut m = self.markets.lock().unwrap();
        self.subscribed.iter().filter_map(|c| m.get_mut(c).and_then(|am| am.features(now_ms, &self.cfg.flow)).map(|f| (c.clone(), f))).collect()
    }

    fn bias(&self, coin: &str, now: DateTime<Utc>) -> Bias {
        intel::bias(&self.intel.lock().unwrap().events, coin, now, self.cfg.news.half_life_min)
    }

    fn subscribe(&mut self, coin: &str) {
        if self.subscribed.insert(coin.to_string()) {
            let _ = self.sub_tx.send(coin.to_string());
        }
    }

    fn save(&self) {
        if let Err(e) = self.ledger.save(&self.files.ledger) {
            error!("saving ledger: {e:#}");
        }
    }

    fn tradable(&self, coin: &str) -> bool {
        self.cfg.watch.iter().any(|c| c == coin) || self.universe.get(coin).is_some_and(|v| *v >= self.cfg.news.min_volume_musd * 1e6)
    }

    async fn shutdown(&mut self) {
        if let Venue::Live(l) = &self.venue {
            for (coin, pe) in &self.pending {
                if let Some(c) = &pe.cloid {
                    match l.cancel(coin, c).await {
                        Ok(()) => info!("{coin}: resting entry cancelled on shutdown"),
                        Err(e) => error!("{coin}: could not cancel resting entry {c} ({e}); cancel it on Hyperliquid"),
                    }
                }
            }
        }
        info!("shutting down; open positions stay in the ledger (live: take-profits and backstop stops keep resting on the exchange)");
        self.save();
    }

    async fn tick(&mut self, now: DateTime<Utc>, events: Vec<NewsEvent>) -> Result<()> {
        let now_ms = now.timestamp_millis();
        self.control = Control::load(&self.files.control);
        let feats = self.feats(now_ms);

        if self.universe_at.is_none_or(|t| t.elapsed() > Duration::from_secs(600)) {
            self.universe_at = Some(std::time::Instant::now());
            if let Ok(p) = crate::hyperliquid::perp_universe(&self.http, self.cfg.api()).await {
                self.universe = p.into_iter().map(|x| (x.coin, x.day_volume_usd)).collect();
            }
        }

        // Day roll and the daily loss limit (on trade P&L).
        if self.ledger.day != now.date_naive() {
            self.ledger.day = now.date_naive();
            self.ledger.day_trade_pnl = 0.0;
        }
        if self.ledger.day_trade_pnl < -self.cfg.risk.daily_loss_pct * self.ledger.capital() && self.ledger.halted_day != Some(self.ledger.day) {
            warn!(pnl = self.ledger.day_trade_pnl, "daily loss limit hit: no entries until tomorrow (UTC)");
            self.ledger.halted_day = Some(self.ledger.day);
            self.save();
        }

        self.reconcile(now_ms, &feats).await;
        self.accrue_funding(now_ms, &feats);
        self.resolve_pending_entries(now, &feats).await;
        self.manage_take_profits(now).await;

        // Flatten request from the dashboard (once per request).
        let flatten = self.control.flatten_requested_at.is_some_and(|t| self.ledger.flatten_handled < Some(t));
        let coins: Vec<String> = self.ledger.positions.keys().cloned().collect();
        for coin in coins {
            let b = self.bias(&coin, now);
            let f = feats.get(&coin);
            let reason = {
                let Some(p) = self.ledger.positions.get_mut(&coin) else { continue };
                if let Some(f) = f {
                    p.best_px = match p.side {
                        Side::Long => p.best_px.max(f.bid),
                        Side::Short => p.best_px.min(f.ask),
                    };
                    p.sl_px = strategy::trailed_stop(p.side, p.entry_px, p.best_px, p.sl_px, p.stop_bps, p.cost_bps, p.trail_r);
                }
                match (&p.closing, flatten) {
                    (Some(r), _) => Some(r.clone()),
                    (None, true) => Some("flatten".to_string()),
                    (None, false) => strategy::exit_reason(&OpenPos { side: p.side, sl_px: p.sl_px, deadline_ms: p.deadline_ms }, f, &b, now_ms).map(|r| {
                        if r == "stop" && p.side.sign() * (p.sl_px - p.init_sl_px) > 0.0 { "trailing stop".to_string() } else { r.to_string() }
                    }),
                }
            };
            if let Some(r) = reason {
                self.close(&coin, &r, f, now).await;
            }
        }
        if let Some(t) = self.control.flatten_requested_at.filter(|_| flatten) {
            self.ledger.flatten_handled = Some(t);
            self.save();
            for (coin, pe) in std::mem::take(&mut self.pending) {
                if let (Venue::Live(l), Some(c)) = (&self.venue, &pe.cloid) {
                    let _ = l.cancel(&coin, c).await;
                }
            }
            self.watches.clear();
            info!("flatten request handled");
        }

        self.on_news(events, now).await;
        self.run_watches(now, &feats).await;
        if self.cfg.flow.enabled {
            self.run_flow(now, &feats).await;
        }
        self.flow_markouts(now_ms);
        if now_ms - self.last_marks_ms > 15_000 {
            self.last_marks_ms = now_ms;
            self.event_markouts(now_ms).await;
        }
        Ok(())
    }

    // ------------------------------------------------------------ live upkeep

    async fn reconcile(&mut self, now_ms: i64, _feats: &HashMap<String, Features>) {
        let Some(l) = self.venue.live_arc() else { return };
        if now_ms - self.last_reconcile_ms < 30_000 {
            return;
        }
        self.last_reconcile_ms = now_ms;
        let acct = match l.ex.account().await {
            Ok(a) => a,
            Err(e) => {
                warn!("account check failed: {e:#}");
                self.withdrawable = None;
                return;
            }
        };
        self.withdrawable = Some(acct.withdrawable);
        self.exch_review.clear();
        let mut coins: HashSet<String> = self.ledger.held.keys().cloned().collect();
        coins.extend(acct.positions.keys().filter(|c| self.subscribed.contains(*c)).cloned());
        for coin in coins {
            if self.pending.contains_key(&coin) {
                continue;
            }
            let want = self.ledger.held(&coin);
            let have = acct.positions.get(&coin).copied().unwrap_or(0.0);
            if (have - want).abs() <= l.step(&coin) * 0.5 + 1e-12 {
                continue;
            }
            // Flat on the exchange but open here: did the backstop or take-profit fill while we weren't looking?
            if have == 0.0 && let Some(p) = self.ledger.positions.get(&coin).cloned() {
                for (cloid, reason) in [(p.backstop_cloid.clone(), "backstop stop"), (p.tp_cloid.clone(), "take profit")] {
                    let Some(c) = cloid else { continue };
                    if let Ok(pr) = l.maker_progress(&c, p.opened_ms).await
                        && pr.filled_sz > 0.0
                    {
                        let x = Exec::fill(p.side == Side::Short, pr.filled_sz, pr.avg_px, pr.fee_usd, pr.oid.to_string());
                        self.book_exit(&coin, x, reason, Utc::now());
                    }
                }
                if self.ledger.held(&coin).abs() <= l.step(&coin) * 0.5 + 1e-12 {
                    continue;
                }
            }
            self.exch_review.insert(coin.clone(), format!("Hyperliquid holds {have} {coin}, ledger expects {want:.6}"));
        }
    }

    fn accrue_funding(&mut self, now_ms: i64, feats: &HashMap<String, Features>) {
        let dt_h = (now_ms - self.last_funding_ms).max(0) as f64 / 3_600_000.0;
        self.last_funding_ms = now_ms;
        for (coin, p) in self.ledger.positions.iter_mut() {
            let Some(f) = feats.get(coin) else { continue };
            // Positive funding: longs pay, shorts receive.
            let amt = -p.side.sign() * f.funding_bps_1h / 1e4 * p.qty * f.mid * dt_h;
            p.funding_usd += amt;
            self.ledger.cash += amt;
            self.ledger.funding += amt;
        }
    }

    // ------------------------------------------------------------ entries

    /// Account-level checks shared by both trade kinds.
    fn risk_gates(&self, coin: &str, now_ms: i64) -> Vec<Gate> {
        let l = &self.ledger;
        let busy = l.positions.len() + self.pending.len();
        let exch = l.live_hold.clone().or_else(|| self.exch_review.get(coin).cloned()).or_else(|| (self.venue.live().is_some() && self.withdrawable.is_none()).then(|| "account check pending or failing".into()));
        vec![
            Gate::new("not paused", !self.control.paused, if self.control.paused { "paused from the dashboard" } else { "" }),
            Gate::new("daily limit", l.halted_day != Some(l.day), format!("today {:+.2}", l.day_trade_pnl)),
            Gate::new("flat", !l.positions.contains_key(coin) && !self.pending.contains_key(coin), ""),
            Gate::new("slots", busy < self.cfg.risk.max_positions, format!("{busy}/{}", self.cfg.risk.max_positions)),
            Gate::new("cooldown", l.cooldown_until_ms.get(coin).is_none_or(|t| now_ms >= *t), ""),
            Gate::new("entries/hour", l.entries_ms.len() < self.cfg.risk.max_entries_per_hour, format!("{}/{}", l.entries_ms.len(), self.cfg.risk.max_entries_per_hour)),
            Gate::new("exchange", exch.is_none(), exch.unwrap_or_default()),
        ]
    }

    fn free_collateral(&self, marks: &HashMap<String, f64>) -> f64 {
        let reserved: f64 = self.pending.values().map(|p| p.usd).sum();
        let mut free = (self.ledger.capital() * self.cfg.risk.max_gross).min(self.ledger.equity(marks)) - self.ledger.open_notional(marks) - reserved;
        if self.venue.live().is_some() {
            free = free.min(self.withdrawable.unwrap_or(0.0));
        }
        free.max(0.0)
    }

    /// Opens a position (taker) or rests a maker entry. Returns whether it went out.
    #[allow(clippy::too_many_arguments)]
    async fn enter(&mut self, coin: &str, kind: TradeKind, plan: Plan, usd: f64, mode: EntryMode, f: &Features, candidate_id: &str, why: String, size_mult: f64, review: Option<Review>, now: DateTime<Utc>) -> std::result::Result<(), String> {
        let now_ms = now.timestamp_millis();
        let buy = plan.side == Side::Long;
        match mode {
            EntryMode::Maker => {
                let px = if buy { f.bid } else { f.ask };
                let (cloid, px, sz) = match &self.venue {
                    Venue::Paper => (None, px, usd / px),
                    Venue::Live(l) => {
                        l.ensure_leverage(coin).await?;
                        l.post_maker(coin, buy, px, usd / px, false).await.map(|(c, p, s)| (Some(c), p, s))?
                    }
                };
                info!("{} post {} {coin} ${:.0} @ {px} ({kind:?}, maker)", self.venue.name(), plan.side.word(), sz * px);
                self.pending.insert(coin.to_string(), PendingEntry { side: plan.side, px, usd: sz * px, sz, posted_ms: now_ms, cloid, last_poll_ms: now_ms, plan, candidate_id: candidate_id.into(), why });
                self.ledger.entries_ms.push_back(now_ms);
                Ok(())
            }
            EntryMode::Taker => {
                let x = self.venue.taker(coin, buy, usd / if buy { f.ask } else { f.bid }, false, Some(f), &self.cfg).await?;
                self.ledger.entries_ms.push_back(now_ms);
                self.open_from_fill(coin, kind, plan, x, candidate_id, why, size_mult, review, now).await;
                Ok(())
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn open_from_fill(&mut self, coin: &str, kind: TradeKind, plan: Plan, x: Exec, candidate_id: &str, why: String, size_mult: f64, review: Option<Review>, now: DateTime<Utc>) {
        let now_ms = now.timestamp_millis();
        let s = plan.side.sign();
        self.ledger.apply(coin, &x);
        let trail_r = match kind {
            TradeKind::Catalyst => self.cfg.catalyst.trail_r,
            TradeKind::Flow => self.cfg.flow.trail_r,
        };
        let mut p = Position {
            kind,
            side: plan.side,
            qty: x.base.abs(),
            entry_px: x.price,
            entry_cash: x.cash,
            entry_base: x.base,
            entry_fee: x.fee_usd,
            opened_ms: now_ms,
            tp_px: x.price * (1.0 + s * plan.tp_bps / 1e4),
            sl_px: x.price * (1.0 - s * plan.stop_bps / 1e4),
            init_sl_px: x.price * (1.0 - s * plan.stop_bps / 1e4),
            best_px: x.price,
            stop_bps: plan.stop_bps,
            cost_bps: plan.cost_bps,
            trail_r,
            deadline_ms: now_ms + plan.hold_secs * 1000,
            candidate_id: candidate_id.into(),
            why,
            size_mult,
            review,
            entry_order: x.order_id.clone(),
            funding_usd: 0.0,
            closing: None,
            tp_cloid: None,
            backstop_cloid: None,
            tp_posted_ms: now_ms,
            last_poll_ms: 0,
        };
        info!("{} OPEN {kind:?} {} {coin} {:.6} @ {} (${:.0}) · stop {:.5} · target {:.5} · {}", self.venue.name(), plan.side.word(), p.qty, x.price, p.qty * x.price, p.sl_px, p.tp_px, p.why);
        if let Venue::Live(l) = &self.venue {
            post_protection(l, coin, &mut p, &self.cfg, true).await;
        }
        self.ledger.positions.insert(coin.to_string(), p);
        self.save();
    }

    async fn resolve_pending_entries(&mut self, now: DateTime<Utc>, feats: &HashMap<String, Features>) {
        let now_ms = now.timestamp_millis();
        let timeout = self.cfg.flow.maker_timeout_secs * 1000;
        for coin in self.pending.keys().cloned().collect::<Vec<_>>() {
            let pe = &self.pending[&coin];
            let expired = now_ms - pe.posted_ms >= timeout;
            // (filled size, price, fee, order id) once resolved; None = still resting.
            let resolved: Option<(f64, f64, f64, String)> = match (&self.venue, pe.cloid.clone()) {
                (Venue::Live(l), Some(cloid)) => {
                    if !expired && now_ms - pe.last_poll_ms < 1000 {
                        continue;
                    }
                    let (posted, sz) = (pe.posted_ms, pe.sz);
                    self.pending.get_mut(&coin).expect("listed").last_poll_ms = now_ms;
                    match l.maker_progress(&cloid, posted).await {
                        Err(e) => {
                            warn!("{coin}: order status failed: {e:#}");
                            None
                        }
                        Ok(p) if p.open && !expired => None,
                        Ok(p) if p.oid == 0 && !expired && now_ms - posted < 10_000 => None,
                        Ok(p) if !p.open && p.filled_sz >= sz - l.step(&coin) * 0.5 => Some((p.filled_sz, p.avg_px, p.fee_usd, p.oid.to_string())),
                        Ok(_) => match l.cancel(&coin, &cloid).await {
                            Err(e) => {
                                warn!("{coin}: cancel failed ({e}); retrying");
                                None
                            }
                            Ok(()) => match l.maker_progress(&cloid, posted).await {
                                Ok(p) if p.open => None,
                                Ok(p) => Some((p.filled_sz, p.avg_px, p.fee_usd, p.oid.to_string())),
                                Err(_) => None,
                            },
                        },
                    }
                }
                _ => {
                    let filled = self.markets.lock().unwrap().get(&coin).is_some_and(|am| am.traded_through(pe.px, pe.side == Side::Long, pe.posted_ms.div_euclid(1000)));
                    if filled {
                        Some((pe.sz, pe.px, pe.usd * self.cfg.costs.maker_bps / 1e4, "paper-maker".into()))
                    } else if expired {
                        Some((0.0, pe.px, 0.0, String::new()))
                    } else {
                        None
                    }
                }
            };
            let Some((fsz, fpx, fee, oid)) = resolved else { continue };
            let pe = self.pending.remove(&coin).expect("listed");
            let mid = feats.get(&coin).map(|f| f.mid).unwrap_or(pe.px);
            append_jsonl(
                &self.files.orders,
                &json!({ "ts": now, "candidate_id": pe.candidate_id, "coin": coin, "side": pe.side, "px": pe.px, "usd": pe.usd,
                         "outcome": if fsz <= 0.0 { "missed" } else if fsz < pe.sz * 0.999 { "partial" } else { "filled" },
                         "wait_ms": now_ms - pe.posted_ms, "mid_move_bps": pe.side.sign() * (mid / pe.px - 1.0) * 1e4 }),
            );
            if fsz <= 0.0 {
                continue;
            }
            let x = Exec::fill(pe.side == Side::Long, fsz, fpx, fee, oid);
            self.open_from_fill(&coin, TradeKind::Flow, pe.plan, x, &pe.candidate_id, pe.why, 1.0, None, now).await;
        }
    }

    // ------------------------------------------------------------ exits

    /// Take-profits rest as maker orders: paper fills on a strict trade-through,
    /// live polls the resting order. Also (live) re-posts missing protection.
    async fn manage_take_profits(&mut self, now: DateTime<Utc>) {
        let now_ms = now.timestamp_millis();
        for coin in self.ledger.positions.keys().cloned().collect::<Vec<_>>() {
            let p = self.ledger.positions[&coin].clone();
            if p.closing.is_some() {
                continue;
            }
            match self.venue.live_arc() {
                None => {
                    // A resting sell (long TP) fills when trades print above it; a resting buy below it.
                    let hit = self.markets.lock().unwrap().get(&coin).is_some_and(|am| am.traded_through(p.tp_px, p.side == Side::Short, p.tp_posted_ms.div_euclid(1000)));
                    if hit {
                        let x = Exec::fill(p.side == Side::Short, p.qty, p.tp_px, p.qty * p.tp_px * self.cfg.costs.maker_bps / 1e4, "paper-tp".into());
                        self.book_exit(&coin, x, "take profit", now);
                    }
                }
                Some(l) => {
                    if now_ms - p.last_poll_ms < 2000 {
                        continue;
                    }
                    if let Some(pp) = self.ledger.positions.get_mut(&coin) {
                        pp.last_poll_ms = now_ms;
                    }
                    match &p.tp_cloid {
                        Some(c) => match l.maker_progress(c, p.tp_posted_ms).await {
                            Ok(pr) if !pr.open && pr.filled_sz >= p.qty - l.step(&coin) * 0.5 => {
                                let x = Exec::fill(p.side == Side::Short, pr.filled_sz, pr.avg_px, pr.fee_usd, pr.oid.to_string());
                                self.book_exit(&coin, x, "take profit", now);
                                if let Some(b) = &p.backstop_cloid {
                                    let _ = l.cancel(&coin, b).await;
                                }
                            }
                            Ok(pr) if !pr.open && pr.oid != 0 => {
                                // Cancelled by the exchange (or partly filled then cancelled): book what filled, re-post later.
                                if pr.filled_sz > 0.0 {
                                    let x = Exec::fill(p.side == Side::Short, pr.filled_sz, pr.avg_px, pr.fee_usd, pr.oid.to_string());
                                    self.book_exit(&coin, x, "take profit (partial)", now);
                                }
                                if let Some(pp) = self.ledger.positions.get_mut(&coin) {
                                    pp.tp_cloid = None;
                                }
                            }
                            _ => {}
                        },
                        None => {
                            let mut pp = p.clone();
                            let retry_backstop = self.last_backstop_try.get(&coin).is_none_or(|t| now_ms - t > 60_000);
                            if retry_backstop {
                                self.last_backstop_try.insert(coin.clone(), now_ms);
                            }
                            post_protection(&l, &coin, &mut pp, &self.cfg, retry_backstop).await;
                            if let Some(slot) = self.ledger.positions.get_mut(&coin) {
                                slot.tp_cloid = pp.tp_cloid;
                                slot.tp_posted_ms = pp.tp_posted_ms;
                                slot.backstop_cloid = pp.backstop_cloid;
                                slot.closing = pp.closing;
                            }
                            self.save();
                        }
                    }
                }
            }
        }
    }

    /// Closes a whole position: cancels the resting take-profit (booking any part
    /// that filled), sends a reduce-only taker exit for the rest, then cancels the
    /// backstop. A failed exit is retried next tick.
    async fn close(&mut self, coin: &str, reason: &str, f: Option<&Features>, now: DateTime<Utc>) {
        let Some(p) = self.ledger.positions.get(coin).cloned() else { return };
        if let (Some(l), Some(tp)) = (self.venue.live_arc(), &p.tp_cloid) {
            if let Err(e) = l.cancel(coin, tp).await {
                warn!("{coin}: cancelling take-profit failed ({e}); retrying next tick");
                if let Some(pp) = self.ledger.positions.get_mut(coin) {
                    pp.closing = Some(reason.into());
                }
                return;
            }
            if let Ok(pr) = l.maker_progress(tp, p.tp_posted_ms).await
                && pr.filled_sz > 0.0
            {
                let x = Exec::fill(p.side == Side::Short, pr.filled_sz, pr.avg_px, pr.fee_usd, pr.oid.to_string());
                self.book_exit(coin, x, "take profit (partial)", now);
            }
            if let Some(pp) = self.ledger.positions.get_mut(coin) {
                pp.tp_cloid = None;
            }
        }
        let Some(p) = self.ledger.positions.get(coin).cloned() else { return };
        match self.venue.taker(coin, p.side == Side::Short, p.qty, true, f, &self.cfg).await {
            Ok(x) => {
                self.book_exit(coin, x, reason, now);
                if !self.ledger.positions.contains_key(coin)
                    && let (Some(l), Some(b)) = (self.venue.live_arc(), &p.backstop_cloid)
                    && let Err(e) = l.cancel(coin, b).await
                {
                    warn!("{coin}: cancelling the backstop stop failed ({e}); it is reduce-only, cancel it on Hyperliquid");
                }
            }
            Err(m) => {
                error!("{coin} exit ({reason}) failed: {m}");
                if let Some(pp) = self.ledger.positions.get_mut(coin) {
                    pp.closing = Some(reason.into());
                }
                if m.starts_with("UNCONFIRMED") {
                    self.ledger.live_hold = Some(format!("{m} · check Hyperliquid, then `jev-trader hl clear-hold`"));
                }
                self.save();
            }
        }
    }

    /// Books an exit fill against the open position; records the round trip.
    fn book_exit(&mut self, coin: &str, x: Exec, reason: &str, now: DateTime<Utc>) {
        let Some(mut p) = self.ledger.positions.remove(coin) else { return };
        let now_ms = now.timestamp_millis();
        self.ledger.apply(coin, &x);
        let orig = p.clone();
        let closed_qty = x.base.abs().min(orig.qty);
        let notional = closed_qty * orig.entry_px;
        let pnl = close_part(&mut p, &x);
        let flat = p.qty <= self.venue.step(coin) * 0.5 + 1e-9;
        self.ledger.realized += pnl;
        self.ledger.day_trade_pnl += pnl;
        if flat {
            self.ledger.held.insert(coin.to_string(), 0.0);
            if pnl < 0.0 {
                self.ledger.losses += 1;
                self.ledger.cooldown_until_ms.insert(coin.to_string(), now_ms + self.cfg.risk.loss_cooldown_min * 60_000);
            } else {
                self.ledger.wins += 1;
            }
        } else {
            if !reason.contains("partial") {
                p.closing = Some(reason.to_string());
            }
            self.ledger.positions.insert(coin.to_string(), p);
        }
        let bps = if notional > 0.0 { pnl / notional * 1e4 } else { 0.0 };
        info!("{} CLOSE {:?} {} {coin} @ {} — {reason} · pnl {pnl:+.2} ({bps:+.1} bps)", self.venue.name(), orig.kind, orig.side.word(), x.price);
        append_jsonl(
            &self.files.trades,
            &TradeRecord {
                coin,
                kind: orig.kind,
                side: orig.side,
                venue: self.venue.name(),
                opened: DateTime::from_timestamp_millis(orig.opened_ms).unwrap_or(now),
                closed: now,
                hold_secs: (now_ms - orig.opened_ms) / 1000,
                qty: closed_qty,
                entry_px: orig.entry_px,
                exit_px: x.price,
                notional,
                fees_usd: orig.entry_fee * (closed_qty / orig.qty) + x.fee_usd,
                funding_usd: orig.funding_usd * (closed_qty / orig.qty),
                pnl_usd: pnl,
                pnl_bps: bps,
                reason,
                tp_px: orig.tp_px,
                sl_px: orig.sl_px,
                init_sl_px: orig.init_sl_px,
                stop_bps: orig.stop_bps,
                cost_bps: orig.cost_bps,
                size_mult: orig.size_mult,
                why: &orig.why,
                review: orig.review.as_ref(),
                candidate_id: &orig.candidate_id,
                entry_order: &orig.entry_order,
                exit_order: &x.order_id,
            },
        );
        self.save();
    }

    // ------------------------------------------------------------ news → watches

    async fn on_news(&mut self, events: Vec<NewsEvent>, now: DateTime<Utc>) {
        for e in events {
            // Market-wide news trades as BTC.
            let coin = match e.coin() {
                Some(c) => c.to_string(),
                None if e.market => "BTC".to_string(),
                None => continue,
            };
            if !self.tradable(&coin) {
                continue;
            }
            if e.impact.abs() >= 0.2 && e.new_info >= 0.3 {
                self.event_marks.push(EventMarkout {
                    id: e.id.clone(),
                    coin: coin.clone(),
                    side: Side::of(e.impact),
                    impact: e.impact,
                    new_info: e.new_info,
                    source: e.source.clone(),
                    kind: e.kind,
                    lag_min: e.lag_min,
                    t0_ms: e.first_seen.timestamp_millis(),
                    left: EVENT_HORIZONS_MIN.to_vec(),
                    taken: false,
                });
                self.save_event_marks();
            }
            if !self.cfg.catalyst.enabled || self.ledger.used(&e.id) {
                continue;
            }
            let gates = strategy::catalyst_trigger(&e, &self.cfg.catalyst);
            if first_block(&gates).is_some() {
                continue;
            }
            if self.watches.iter().any(|w| w.coin == coin) {
                continue;
            }
            self.subscribe(&coin);
            let (p_pub, atr) = self.reference(&coin, e.published).await;
            info!("catalyst watch {} {coin}: {} (impact {:+.2}, new {:.2})", Side::of(e.impact).word(), e.title, e.impact, e.new_info);
            self.watches.push(Watch {
                event_id: e.id.clone(),
                title: e.title.clone(),
                source: e.source.clone(),
                coin,
                side: Side::of(e.impact),
                impact: e.impact,
                new_info: e.new_info,
                created_ms: now.timestamp_millis(),
                expires_ms: now.timestamp_millis() + self.cfg.catalyst.confirm_secs * 1000,
                p_pub,
                atr,
                last_block: "waiting for the feed".into(),
            });
        }
    }

    /// Price at publication (1m close) and ATR(14) of 1h candles, from REST.
    async fn reference(&self, coin: &str, published: DateTime<Utc>) -> (f64, f64) {
        let api = self.cfg.api();
        let pub_ms = published.timestamp_millis();
        let p = crate::hyperliquid::candles(&self.http, api, coin, "1m", pub_ms - 180_000, pub_ms + 60_000).await.ok().and_then(|c| crate::market::close_at(&c, pub_ms)).unwrap_or(0.0);
        let atr = crate::market::klines(&self.http, api, coin, "1h", 16).await.ok().map(|mut c| {
            c.pop(); // the open candle
            crate::market::atr_pct(&c, 14).unwrap_or(0.0)
        });
        (p, atr.unwrap_or(0.0))
    }

    async fn run_watches(&mut self, now: DateTime<Utc>, feats: &HashMap<String, Features>) {
        let now_ms = now.timestamp_millis();
        let mut done: Vec<(usize, String)> = Vec::new();
        for i in 0..self.watches.len() {
            let w = self.watches[i].clone();
            if now_ms >= w.expires_ms {
                done.push((i, format!("expired: {}", w.last_block)));
                continue;
            }
            let Some(f) = feats.get(&w.coin) else { continue };
            if f.history_secs < self.cfg.flow.flow_window_secs {
                self.watches[i].last_block = "reading order flow".into();
                continue;
            }
            if w.p_pub <= 0.0 {
                // Publication price unknown (candles failed): use the first mid seen.
                self.watches[i].p_pub = f.mid;
            }
            let w = self.watches[i].clone();
            let (mut gates, plan) = strategy::catalyst_entry(&w, f, &self.cfg.catalyst, &self.cfg.costs, &self.cfg.risk);
            gates.extend(self.risk_gates(&w.coin, now_ms));
            if let Some(b) = first_block(&gates) {
                self.watches[i].last_block = b.to_string();
                continue;
            }
            // Jev reviews the trade it would take (one call). It can shrink or stop it, never start it.
            let mut size_mult = 1.0;
            let mut review = None;
            if self.cfg.catalyst.review {
                let ev = self.intel.lock().unwrap().events.iter().find(|e| e.id == w.event_id).cloned();
                if let Some(ev) = ev {
                    let moved = w.side.sign() * (f.mid / w.p_pub - 1.0);
                    match intel::review(&self.gate, &ev, &w.coin, w.side == Side::Long, moved * 100.0, if w.atr > 0.0 { moved / w.atr } else { 0.0 }, f.funding_bps_1h).await {
                        Ok(r) => {
                            if r.priced_in > self.cfg.catalyst.review_veto {
                                gates.push(Gate::new("jev review", false, format!("priced in {:.2} (> {:.2})", r.priced_in, self.cfg.catalyst.review_veto)));
                                self.log_catalyst(&w, &gates, "vetoed by Jev", Some(&r), now);
                                done.push((i, "vetoed".into()));
                                continue;
                            }
                            if r.priced_in > self.cfg.catalyst.review_halve {
                                size_mult = 0.5;
                            }
                            gates.push(Gate::new("jev review", true, format!("priced in {:.2} · continues {:.2} · size ×{size_mult}", r.priced_in, r.continues)));
                            review = Some(r);
                        }
                        Err(e) => gates.push(Gate::new("jev review", true, format!("unavailable ({e:#}); full size"))),
                    }
                }
            }
            let marks: HashMap<String, f64> = feats.iter().map(|(k, f)| (k.clone(), f.mid)).collect();
            let free = self.free_collateral(&marks);
            let (usd, bound) = strategy::size_usd(self.ledger.capital(), free, self.cfg.catalyst.risk_pct, size_mult, plan.stop_bps, &self.cfg.risk);
            let min = self.cfg.risk.min_trade_usd.max(if self.venue.live().is_some() { MIN_ORDER_USD } else { 0.0 });
            gates.push(Gate::new("size", usd >= min, format!("${usd:.0} ({bound})")));
            if usd < min {
                self.watches[i].last_block = "size".into();
                continue;
            }
            let id = format!("cat-{}-{now_ms}", w.coin);
            let why = format!("{} [{}]", w.title, w.source);
            match self.enter(&w.coin, TradeKind::Catalyst, plan, usd, self.cfg.catalyst.entry, f, &id, why, size_mult, review.clone(), now).await {
                Ok(()) => {
                    self.log_catalyst(&w, &gates, "entered", review.as_ref(), now);
                    for m in self.event_marks.iter_mut().filter(|m| m.id == w.event_id) {
                        m.taken = true;
                    }
                    done.push((i, "entered".into()));
                }
                Err(m) => {
                    warn!("{} catalyst entry failed: {m}", w.coin);
                    if m.starts_with("UNCONFIRMED") {
                        self.ledger.live_hold = Some(format!("{m} · check Hyperliquid, then `jev-trader hl clear-hold`"));
                    }
                    self.watches[i].last_block = format!("order failed: {m}");
                }
            }
        }
        done.sort_by_key(|(i, _)| std::cmp::Reverse(*i));
        for (i, outcome) in done {
            let w = self.watches.remove(i);
            if outcome.starts_with("expired") {
                self.log_catalyst(&w, &[], &outcome, None, now);
            }
            self.ledger.mark_used(&w.event_id);
        }
        self.save();
    }

    fn log_catalyst(&self, w: &Watch, gates: &[Gate], outcome: &str, review: Option<&Review>, now: DateTime<Utc>) {
        append_jsonl(&self.files.candidates, &json!({ "ts": now, "kind": "catalyst", "coin": w.coin, "side": w.side, "event_id": w.event_id, "title": w.title, "source": w.source,
            "impact": w.impact, "new_info": w.new_info, "atr": w.atr, "waited_secs": (now.timestamp_millis() - w.created_ms) / 1000, "gates": gates, "review": review, "outcome": outcome }));
    }

    // ------------------------------------------------------------ flow

    async fn run_flow(&mut self, now: DateTime<Utc>, feats: &HashMap<String, Features>) {
        let now_ms = now.timestamp_millis();
        while self.ledger.entries_ms.front().is_some_and(|t| *t < now_ms - 3_600_000) {
            self.ledger.entries_ms.pop_front();
        }
        for coin in self.cfg.watch.clone() {
            let Some(f) = feats.get(&coin) else { continue };
            let b = self.bias(&coin, now);
            let evals = [Side::Long, Side::Short].map(|s| strategy::flow_entry(f, &b, &self.cfg.flow, &self.cfg.costs, &self.cfg.risk, s));
            let lean = if f.flow < 0.0 { 1 } else { 0 };
            self.blocks.insert(coin.clone(), first_block(&evals[lean].gates).unwrap_or("").to_string());
            let Some(ev) = evals.into_iter().find(|e| e.candidate) else { continue };
            if f.history_secs < self.cfg.flow.warmup_secs || self.last_candidate_ms.get(&coin).is_some_and(|t| now_ms - t < self.cfg.flow.flow_window_secs * 1000) {
                continue;
            }
            self.last_candidate_ms.insert(coin.clone(), now_ms);
            let mut gates = ev.gates.clone();
            let mut ok = ev.ok;
            if ok {
                gates.extend(self.risk_gates(&coin, now_ms));
                ok = first_block(&gates).is_none();
            }
            let id = format!("flow-{coin}-{now_ms}");
            let mut taken = false;
            if ok {
                let marks: HashMap<String, f64> = feats.iter().map(|(k, f)| (k.clone(), f.mid)).collect();
                let (usd, bound) = strategy::size_usd(self.ledger.capital(), self.free_collateral(&marks), self.cfg.flow.risk_pct, 1.0, ev.plan.stop_bps, &self.cfg.risk);
                let min = self.cfg.risk.min_trade_usd.max(if self.venue.live().is_some() { MIN_ORDER_USD } else { 0.0 });
                gates.push(Gate::new("size", usd >= min, format!("${usd:.0} ({bound})")));
                if usd >= min {
                    let why = format!("order-flow burst with news bias {:+.2}", b.score);
                    match self.enter(&coin, TradeKind::Flow, ev.plan, usd, self.cfg.flow.entry, f, &id, why, 1.0, None, now).await {
                        Ok(()) => taken = true,
                        Err(m) => {
                            warn!("{coin} flow entry failed: {m}");
                            gates.push(Gate::new("order", false, m));
                        }
                    }
                }
            }
            append_jsonl(&self.files.candidates, &json!({ "ts": now, "kind": "flow", "id": id, "coin": coin, "side": ev.side, "bias": b.score, "features": f, "gates": gates, "plan": ev.plan, "taken": taken, "blocked_by": if taken { None } else { first_block(&gates) } }));
            self.flow_marks.push(FlowMarkout {
                id,
                coin: coin.clone(),
                side: ev.side,
                taken,
                bias: b.score,
                aligned: ev.side.sign() * b.score >= self.cfg.flow.min_bias,
                t0_sec: now_ms / 1000,
                mid0: f.mid,
                cost_bps: ev.plan.cost_bps,
                left: self.cfg.flow.markouts.clone(),
            });
        }
    }

    // ------------------------------------------------------------ markouts

    fn flow_markouts(&mut self, now_ms: i64) {
        if self.flow_marks.is_empty() {
            return;
        }
        let m = self.markets.lock().unwrap();
        let now_sec = now_ms / 1000;
        for p in self.flow_marks.iter_mut() {
            let Some(am) = m.get(&p.coin) else { continue };
            p.left.retain(|&h| {
                if now_sec < p.t0_sec + h {
                    return true;
                }
                if let Some(mid) = am.mid_at(p.t0_sec + h) {
                    let ret = p.side.sign() * (mid / p.mid0 - 1.0) * 1e4;
                    append_jsonl(&self.files.markouts, &json!({ "type": "flow", "id": p.id, "coin": p.coin, "side": p.side, "taken": p.taken, "bias": p.bias, "aligned": p.aligned, "h_secs": h, "ret_bps": ret, "cost_bps": p.cost_bps, "net_bps": ret - p.cost_bps }));
                }
                false
            });
        }
        self.flow_marks.retain(|p| !p.left.is_empty());
    }

    /// Headline markouts from 1m candles (works for any coin, survives restarts).
    async fn event_markouts(&mut self, now_ms: i64) {
        let cost = strategy::round_trip_bps(&self.cfg.costs, self.cfg.catalyst.entry, 1.0, 0.0, Side::Long, 0);
        let mut changed = false;
        let mut fetched = 0;
        for i in 0..self.event_marks.len() {
            let m = self.event_marks[i].clone();
            let due: Vec<i64> = m.left.iter().copied().filter(|h| m.t0_ms + h * 60_000 + 90_000 <= now_ms).collect();
            if due.is_empty() || fetched >= 3 {
                continue;
            }
            fetched += 1;
            let end = m.t0_ms + due.iter().max().copied().unwrap_or(0) * 60_000 + 60_000;
            let Ok(c) = crate::hyperliquid::candles(&self.http, self.cfg.api(), &m.coin, "1m", m.t0_ms - 120_000, end).await else { continue };
            let Some(p0) = crate::market::close_at(&c, m.t0_ms) else {
                // No data at all for the start (delisted / too old): give up on it.
                self.event_marks[i].left.clear();
                changed = true;
                continue;
            };
            for h in &due {
                if let Some(p1) = crate::market::close_at(&c, m.t0_ms + h * 60_000) {
                    let ret = m.side.sign() * (p1 / p0 - 1.0) * 1e4;
                    append_jsonl(&self.files.markouts, &json!({ "type": "event", "id": m.id, "coin": m.coin, "side": m.side, "impact": m.impact, "new_info": m.new_info, "source": m.source, "kind": m.kind, "lag_min": m.lag_min, "taken": m.taken, "h_min": h, "ret_bps": ret, "cost_bps": cost, "net_bps": ret - cost }));
                }
            }
            self.event_marks[i].left.retain(|h| !due.contains(h));
            changed = true;
        }
        self.event_marks.retain(|m| !m.left.is_empty());
        if changed {
            self.save_event_marks();
        }
    }

    fn save_event_marks(&self) {
        if let Ok(b) = serde_json::to_vec(&self.event_marks) {
            let _ = write_atomic(&self.files.pending_markouts, &b);
        }
    }

    // ------------------------------------------------------------ status

    fn write_status(&self, now: DateTime<Utc>) {
        let now_ms = now.timestamp_millis();
        let feats = self.feats(now_ms);
        let marks: HashMap<String, f64> = feats.iter().map(|(k, f)| (k.clone(), f.mid)).collect();
        let h = self.health.lock().unwrap().clone();
        let equity = self.ledger.equity(&marks);
        let (news, funnel, sources, queue, last_cycle, events_n) = {
            let s = self.intel.lock().unwrap();
            let mut recent: Vec<&NewsEvent> = s.events.iter().collect();
            recent.sort_by_key(|e| std::cmp::Reverse(e.first_seen));
            let news: Vec<Value> = recent.iter().take(40).map(|e| json!(e)).collect();
            (news, json!(s.funnel), json!(s.sources), s.queue_len, s.last_cycle, s.events.len())
        };
        let mut coins: Vec<String> = self.cfg.watch.clone();
        for c in self.ledger.positions.keys().chain(self.watches.iter().map(|w| &w.coin)) {
            if !coins.contains(c) {
                coins.push(c.clone());
            }
        }
        let coin_rows: Vec<Value> = coins
            .iter()
            .map(|c| {
                let f = feats.get(c);
                let b = self.bias(c, now);
                json!({ "coin": c, "mid": f.map(|f| f.mid), "flow": f.map(|f| f.flow), "activity": f.map(|f| f.activity), "spread_bps": f.map(|f| f.spread_bps),
                        "ret_300s_bps": f.map(|f| f.ret_300s_bps), "funding_bps_1h": f.map(|f| f.funding_bps_1h), "warm": f.map(|f| f.history_secs >= self.cfg.flow.warmup_secs),
                        "bias": b.score, "bias_n": b.n, "hack": b.hack_alert, "block": self.blocks.get(c), "exchange_review": self.exch_review.get(c) })
            })
            .collect();
        let positions: Vec<Value> = self
            .ledger
            .positions
            .iter()
            .map(|(c, p)| {
                let px = feats.get(c).map(|f| if p.side == Side::Long { f.bid } else { f.ask }).unwrap_or(p.entry_px);
                let upnl = p.side.sign() * (px - p.entry_px) * p.qty + p.funding_usd - p.entry_fee;
                json!({ "coin": c, "pos": p, "mark": px, "unrealized": upnl, "risk_usd": (p.side.sign() * (p.entry_px - p.sl_px) * p.qty).max(0.0), "protected": p.backstop_cloid.is_some() || self.venue.live().is_none() })
            })
            .collect();
        let usage = self.gate.usage();
        let status = json!({
            "ts": now,
            "session": self.session,
            "venue": self.venue.name(),
            "live": self.venue.live().is_some(),
            "network": self.cfg.exchange.network.name(),
            "feed": { "connected": h.connected, "last_msg_ms_ago": now_ms - h.last_msg_ms, "reconnects": h.reconnects, "last_error": h.last_error, "coins": h.coins },
            "account": { "equity": equity, "cash": self.ledger.cash, "capital": self.ledger.capital(), "open_notional": self.ledger.open_notional(&marks), "withdrawable": self.withdrawable,
                         "realized": self.ledger.realized, "fees": self.ledger.fees, "funding": self.ledger.funding, "day_trade_pnl": self.ledger.day_trade_pnl,
                         "wins": self.ledger.wins, "losses": self.ledger.losses, "halted_today": self.ledger.halted_day == Some(self.ledger.day), "live_hold": self.ledger.live_hold,
                         "paused": self.control.paused, "entries_last_hour": self.ledger.entries_ms.len() },
            "positions": positions,
            "resting": self.pending.iter().map(|(c, p)| json!({ "coin": c, "side": p.side, "px": p.px, "usd": p.usd, "age_ms": now_ms - p.posted_ms })).collect::<Vec<_>>(),
            "watches": self.watches.iter().map(|w| json!({ "coin": w.coin, "side": w.side, "title": w.title, "source": w.source, "impact": w.impact, "new_info": w.new_info, "secs_left": (w.expires_ms - now_ms) / 1000, "waiting_for": w.last_block, "p_pub": w.p_pub, "atr": w.atr })).collect::<Vec<_>>(),
            "coins": coin_rows,
            "news": news,
            "intel": { "funnel": funnel, "sources": sources, "queue": queue, "last_cycle": last_cycle, "events": events_n },
            "jev": { "today": usage, "calls_last_hour": self.gate.calls_last_hour(), "max_calls_per_hour": self.cfg.jev.max_calls_per_hour,
                     "tokens_per_headline": if usage.headlines > 0 { Some((usage.input + usage.output) as f64 / usage.headlines as f64) } else { None } },
            "pending_markouts": self.event_marks.len() + self.flow_marks.len(),
            "settings": { "catalyst": self.cfg.catalyst, "flow": { "enabled": self.cfg.flow.enabled, "min_bias": self.cfg.flow.min_bias, "hold_secs": self.cfg.flow.hold_secs, "entry": self.cfg.flow.entry }, "risk": self.cfg.risk, "costs": self.cfg.costs },
        });
        let _ = write_atomic(&self.files.status, &serde_json::to_vec(&status).unwrap_or_default());
    }
}

/// Live: rests the reduce-only take-profit (ALO) and, if enabled, the backstop
/// stop-market. A take-profit the book has already passed turns into an exit.
async fn post_protection(l: &Live, coin: &str, p: &mut Position, cfg: &Config, backstop: bool) {
    let close_buy = p.side == Side::Short;
    if p.tp_cloid.is_none() {
        match l.post_maker(coin, close_buy, p.tp_px, p.qty, true).await {
            Ok((c, _, _)) => {
                p.tp_cloid = Some(c);
                p.tp_posted_ms = Utc::now().timestamp_millis();
            }
            Err(m) if m.to_lowercase().contains("immediately match") => p.closing = Some("take profit".into()),
            Err(m) => warn!("{coin}: take-profit not posted ({m}); retrying"),
        }
    }
    if backstop && cfg.exchange.backstop_stop && p.backstop_cloid.is_none() {
        let dist = cfg.exchange.backstop_mult * p.stop_bps / 1e4;
        let trigger = p.entry_px * (1.0 - p.side.sign() * dist);
        match l.post_backstop(coin, close_buy, trigger, p.qty).await {
            Ok(c) => p.backstop_cloid = Some(c),
            Err(m) => warn!("{coin}: backstop stop not posted ({m}); the position is protected only while this process runs"),
        }
    }
}
