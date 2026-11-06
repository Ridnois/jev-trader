//! Live execution on Hyperliquid perps, shared by the news bot (`run --live`) and
//! the scalper (`scalp --live`). The decision path is the same as paper; only the
//! fills differ: each one is a real order, booked from what the exchange reports.
//!
//! Safety:
//! - armed only by `[exchange] mode = "live"` AND the `--live` flag;
//! - the signer is an API agent wallet (can trade, cannot withdraw);
//! - the live ledger lives in its own data_dir, records its network, and refuses a
//!   paper ledger (and vice versa); it manages at most `max_capital_usd`;
//! - marketable orders are IOC limits capped `max_slippage_bps` through the price;
//!   exits are reduce-only, so they can never open or flip a position;
//! - fills are the source of truth: amounts and fees come from `userFills`;
//! - the exchange's positions are compared with the ledger every check; a mismatch
//!   puts that coin in REVIEW (no entries; exits continue);
//! - an order whose outcome can't be confirmed sets `live_hold`, which blocks all
//!   entries until a person checks the account and runs `jev-trader hl clear-hold`.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use tracing::{error, info, warn};

use crate::config::Config;
use crate::decide::{self, Decision};
use crate::hyperliquid::exchange::{Account, Exchange, OrderError, OrderReq, OrderState, Placed, Tif, sum_fills};
use crate::paper::{Binding, Fill, Plan, Skip, State};

pub const MODE: &str = "live";
/// Hyperliquid rejects orders worth less than $10; stay a little above it.
pub const MIN_ORDER_USD: f64 = 10.5;

pub struct Live {
    pub ex: Exchange,
    pub max_slippage_bps: f64,
    /// Fallback taker fee (fraction) if the fill lookup fails.
    pub taker_fee: f64,
}

/// A confirmed fill in ledger terms.
#[derive(Debug, Clone)]
pub struct Booked {
    pub sz: f64,
    pub price: f64,
    pub notional: f64,
    pub fee_usd: f64,
    pub order_id: String,
}

/// Where a resting post-only order stands.
#[derive(Debug, Clone, Default)]
pub struct MakerProgress {
    pub open: bool,
    pub filled_sz: f64,
    pub avg_px: f64,
    pub fee_usd: f64,
    pub oid: u64,
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

impl Live {
    pub async fn connect(cfg: &Config) -> Result<Self> {
        let ex = Exchange::connect(&cfg.exchange).await?;
        for a in &cfg.assets {
            ex.asset(&a.coin)?;
        }
        if cfg.exchange.leverage > 0 {
            for a in &cfg.assets {
                let lev = cfg.exchange.leverage.min(ex.asset(&a.coin)?.max_leverage);
                if let Err(e) = ex.update_leverage(&a.coin, lev, true).await {
                    bail!("setting {} cross leverage to {lev}×: {e} (set [exchange] leverage = 0 to skip)", a.coin);
                }
            }
        }
        info!(network = ex.network.name(), account = %ex.user, agent = %ex.agent_address(), "Hyperliquid exchange connected");
        Ok(Self {
            ex,
            max_slippage_bps: cfg.exchange.max_slippage_bps,
            taker_fee: cfg.scalp.fee_bps / 1e4,
        })
    }

    pub fn venue(&self) -> &'static str {
        if self.ex.network == crate::config::Network::Testnet { "hyperliquid-testnet" } else { "hyperliquid" }
    }

    /// Rounds a size down to the coin's step.
    pub fn floor_sz(&self, coin: &str, sz: f64) -> f64 {
        self.ex.asset(coin).map(|a| a.floor_sz(sz)).unwrap_or(0.0)
    }

    pub fn step(&self, coin: &str) -> f64 {
        self.ex.asset(coin).map(|a| a.step()).unwrap_or(0.0)
    }

    /// Fills of one order since `since_ms`, retried briefly (fills can lag the response).
    async fn fills_of(&self, oid: u64, since_ms: i64) -> Option<(f64, f64, f64)> {
        for attempt in 0..3u32 {
            match self.ex.fills_since(since_ms).await {
                Ok(f) => {
                    if let Some(s) = sum_fills(&f, oid) {
                        return Some(s);
                    }
                }
                Err(e) => warn!("fill lookup failed: {e:#}"),
            }
            tokio::time::sleep(Duration::from_millis(300 << attempt)).await;
        }
        None
    }

    /// After an unknown outcome: what happened to `cloid`? `Ok(None)` = never reached the book.
    async fn resolve(&self, cloid: &str, since_ms: i64) -> std::result::Result<Option<(OrderState, Option<(f64, f64, f64)>)>, ()> {
        for attempt in 0..4u32 {
            tokio::time::sleep(Duration::from_millis(500 << attempt)).await;
            match self.ex.order_status(cloid).await {
                Ok(OrderState::Unknown) if attempt == 3 => return Ok(None),
                Ok(OrderState::Unknown) => continue,
                Ok(st @ OrderState::Known { oid, .. }) => {
                    let fills = self.fills_of(oid, since_ms).await;
                    return Ok(Some((st, fills)));
                }
                Err(e) => warn!("order lookup failed: {e:#}"),
            }
        }
        Err(())
    }

    /// One marketable order: IOC limit `max_slippage_bps` through `ref_px`.
    /// `Err` starting with "UNCONFIRMED" means it may have executed.
    pub async fn market_fill(&self, coin: &str, buy: bool, sz: f64, ref_px: f64, reduce_only: bool) -> std::result::Result<Booked, String> {
        let a = self.ex.asset(coin).map_err(|e| format!("{e:#}"))?;
        let sz = a.floor_sz(sz);
        if sz <= 0.0 {
            return Err(format!("{coin}: size rounds to zero"));
        }
        if !(ref_px.is_finite() && ref_px > 0.0) {
            return Err(format!("{coin}: no reference price"));
        }
        let slip = self.max_slippage_bps / 1e4;
        let px = a.round_px(if buy { ref_px * (1.0 + slip) } else { ref_px * (1.0 - slip) }, buy);
        let cloid = self.ex.new_cloid();
        let since = now_ms() - 5_000;
        let req = OrderReq { coin: coin.into(), buy, px, sz, reduce_only, tif: Tif::Ioc, cloid: cloid.clone() };
        let book = |sz: f64, px: f64, fee: Option<f64>, oid: u64| Booked {
            sz,
            price: px,
            notional: sz * px,
            fee_usd: fee.unwrap_or(sz * px * self.taker_fee),
            order_id: oid.to_string(),
        };
        match self.ex.order(&req).await {
            Ok(Placed::Filled { oid, sz: fsz, avg_px }) => {
                let fills = self.fills_of(oid, since).await;
                // Prefer the fills (source of truth); fall back to the response.
                Ok(match fills {
                    Some((s, p, fee)) => book(s, p, Some(fee), oid),
                    None => book(fsz, avg_px, None, oid),
                })
            }
            Ok(Placed::Resting { oid }) => {
                // An IOC never rests; be safe and pull it.
                let _ = self.ex.cancel_by_cloid(coin, &cloid).await;
                match self.fills_of(oid, since).await {
                    Some((s, p, fee)) => Ok(book(s, p, Some(fee), oid)),
                    None => Err(format!("{coin}: IOC rested unexpectedly and was cancelled unfilled")),
                }
            }
            Ok(Placed::Error(m)) => Err(format!("{coin}: {m}")),
            Err(OrderError::Rejected(m)) => Err(format!("{coin}: {m}")),
            Err(OrderError::Unknown(m)) => {
                warn!(%cloid, "order outcome unknown ({m}); looking it up");
                match self.resolve(&cloid, since).await {
                    Ok(None) => Err(format!("{coin}: order never reached the book ({m})")),
                    Ok(Some((OrderState::Known { oid, .. }, Some((s, p, fee))))) => Ok(book(s, p, Some(fee), oid)),
                    Ok(Some((OrderState::Known { status, .. }, None))) if status != "open" && status != "filled" => {
                        Err(format!("{coin}: order {status} without a fill"))
                    }
                    _ => Err(format!("UNCONFIRMED order {cloid} on {coin}: {m}")),
                }
            }
        }
    }

    /// Posts a post-only (ALO) order. Returns its cloid. A rejection (e.g. it would
    /// have crossed) is an ordinary `Err`; "UNCONFIRMED" means it may be resting.
    pub async fn post_maker(&self, coin: &str, buy: bool, px: f64, usd: f64) -> std::result::Result<(String, f64, f64), String> {
        let a = self.ex.asset(coin).map_err(|e| format!("{e:#}"))?;
        let px = a.round_px(px, !buy); // never round across the book
        let sz = a.floor_sz(usd / px);
        if sz * px < MIN_ORDER_USD {
            return Err(format!("{coin}: ${:.2} is below the exchange minimum", sz * px));
        }
        let cloid = self.ex.new_cloid();
        let since = now_ms() - 5_000;
        let req = OrderReq { coin: coin.into(), buy, px, sz, reduce_only: false, tif: Tif::Alo, cloid: cloid.clone() };
        match self.ex.order(&req).await {
            Ok(Placed::Resting { .. }) | Ok(Placed::Filled { .. }) => Ok((cloid, px, sz)),
            Ok(Placed::Error(m)) => Err(format!("{coin}: {m}")),
            Err(OrderError::Rejected(m)) => Err(format!("{coin}: {m}")),
            Err(OrderError::Unknown(m)) => match self.resolve(&cloid, since).await {
                Ok(None) => Err(format!("{coin}: order never reached the book ({m})")),
                Ok(Some(_)) => Ok((cloid, px, sz)),
                Err(()) => Err(format!("UNCONFIRMED order {cloid} on {coin}: {m}")),
            },
        }
    }

    /// Status and fills of a resting order posted at `since_ms`.
    pub async fn maker_progress(&self, cloid: &str, since_ms: i64) -> Result<MakerProgress> {
        match self.ex.order_status(cloid).await? {
            OrderState::Unknown => Ok(MakerProgress::default()),
            OrderState::Known { status, oid, .. } => {
                let mut p = MakerProgress { open: status == "open", oid, ..Default::default() };
                if let Ok(f) = self.ex.fills_since(since_ms - 5_000).await
                    && let Some((sz, px, fee)) = sum_fills(&f, oid)
                {
                    p.filled_sz = sz;
                    p.avg_px = px;
                    p.fee_usd = fee;
                }
                Ok(p)
            }
        }
    }

    /// Cancels by cloid; "already filled or cancelled" counts as done.
    pub async fn cancel(&self, coin: &str, cloid: &str) -> std::result::Result<(), String> {
        match self.ex.cancel_by_cloid(coin, cloid).await {
            Ok(()) => Ok(()),
            Err(OrderError::Rejected(m)) if m.contains("already canceled") || m.contains("never placed") || m.contains("filled") => Ok(()),
            Err(e) => Err(format!("{e}")),
        }
    }
}

// ---------------------------------------------------------------- news bot

/// Paper commands must never open a live ledger.
pub fn refuse_live_ledger(state: &State) -> Result<()> {
    if state.mode.as_deref() == Some(MODE) {
        bail!("this data_dir holds a LIVE ledger: use the live config and `run --live`");
    }
    Ok(())
}

/// Loads the live ledger, or creates it from the allotted capital.
pub async fn open_ledger(live: &Live, cfg: &Config, path: &Path) -> Result<State> {
    if path.exists() {
        let s = State::load_or_new(path, 0.0)?;
        if s.mode.as_deref() != Some(MODE) {
            bail!("{} is a paper ledger; give the live config its own data_dir", path.display());
        }
        if s.venue.as_deref() != Some(live.venue()) {
            bail!("{} was created for {}, not {}; give this config its own data_dir", path.display(), s.venue.as_deref().unwrap_or("Binance"), live.venue());
        }
        return Ok(s);
    }
    let cap = cfg.exchange.max_capital_usd;
    if !(cap.is_finite() && cap > 0.0) {
        bail!("set [exchange] max_capital_usd to the most USDC the bot may trade with");
    }
    let acct = live.ex.account().await?;
    for a in &cfg.assets {
        let szi = acct.positions.get(&a.coin).copied().unwrap_or(0.0);
        if szi != 0.0 {
            bail!("the account already holds {szi} {} on Hyperliquid; close it or drop {} from the config, so the ledger starts in sync", a.coin, a.symbol);
        }
    }
    let cash = cap.min(acct.withdrawable);
    let min = cfg.risk.min_trade_usd.max(MIN_ORDER_USD);
    if cash < min {
        bail!("only {:.2} USDC withdrawable on Hyperliquid {} (account value {:.2}); need at least {min:.2}", acct.withdrawable, live.ex.network.name(), acct.account_value);
    }
    let mut s = State::new(cash);
    s.mode = Some(MODE.into());
    s.venue = Some(live.venue().into());
    s.save(path)?;
    info!(cash, cap, withdrawable = acct.withdrawable, "live ledger created");
    Ok(s)
}

/// The account, and per-symbol REVIEW reasons when the exchange disagrees with the ledger.
pub async fn reconcile(live: &Live, state: &State, cfg: &Config) -> Result<(Account, HashMap<String, String>)> {
    let acct = live.ex.account().await?;
    let mut review = HashMap::new();
    for a in &cfg.assets {
        if let Some(hold) = &state.live_hold {
            review.insert(a.symbol.clone(), format!("exchange hold: {hold}"));
            continue;
        }
        let want = state.positions.get(&a.symbol).map(|p| p.qty).unwrap_or(0.0);
        let have = acct.positions.get(&a.coin).copied().unwrap_or(0.0);
        if (have - want).abs() > live.step(&a.coin) * 0.5 + 1e-12 {
            review.insert(a.symbol.clone(), format!("Hyperliquid holds {have} {}, ledger expects {want:.6}", a.coin));
        }
    }
    Ok((acct, review))
}

/// Applies a decision with real orders. Returns fills and per-order errors.
pub async fn execute(live: &Live, state: &mut State, d: &mut Decision, cfg: &Config, now: DateTime<Utc>, acct: &mut Account) -> (Vec<Fill>, Vec<String>) {
    let e = d.equity_for_weights;
    let order = decide::prepare(state, d, cfg, now);
    let mut fills = Vec::new();
    let mut errors = Vec::new();
    for i in order {
        let a = &mut d.assets[i];
        let Some(px) = a.exec else { continue };
        let Some(coin) = cfg.assets.iter().find(|x| x.symbol == a.symbol).map(|x| x.coin.clone()) else { continue };
        let stop = decide::stop_for(state, a);
        let skip = |a: &mut decide::AssetDecision, s: Skip| {
            if (a.weight - a.current).abs() > 1e-9 {
                a.binding = Some(Binding::from_skip(s));
            }
        };
        let plan = match state.plan(&a.symbol, a.weight, px, e, cfg.paper.fee_rate, &cfg.risk) {
            Ok(p) => p,
            Err(s) => {
                skip(a, s);
                continue;
            }
        };
        let min_usd = cfg.risk.min_trade_usd.max(MIN_ORDER_USD);
        let (buy, sz, full_exit) = match plan {
            Plan::Buy { notional } => {
                let spend = notional.min(acct.withdrawable);
                let sz = live.floor_sz(&coin, spend / px);
                if sz * px < min_usd {
                    skip(a, Skip::Cash);
                    continue;
                }
                (true, sz, false)
            }
            Plan::Sell { qty, full_exit } => {
                let have = acct.positions.get(&coin).copied().unwrap_or(0.0).max(0.0);
                let q = if full_exit { have } else { live.floor_sz(&coin, qty.min(have)) };
                if q <= 0.0 || (!full_exit && q * px < min_usd) {
                    skip(a, Skip::MinTrade);
                    continue;
                }
                (false, q, full_exit)
            }
        };
        let b = match live.market_fill(&coin, buy, sz, px, !buy).await {
            Ok(b) => b,
            Err(m) => {
                error!("{} order failed: {m}", a.symbol);
                if m.starts_with("UNCONFIRMED") {
                    state.live_hold = Some(format!("{m} · check Hyperliquid, then `jev-trader hl clear-hold`"));
                }
                errors.push(format!("{}: {m}", a.symbol));
                continue;
            }
        };
        let mut f = if buy {
            acct.withdrawable -= b.notional + b.fee_usd;
            *acct.positions.entry(coin.clone()).or_default() += b.sz;
            state.book_buy(&a.symbol, b.sz, b.price, b.notional, b.notional + b.fee_usd, b.fee_usd, &a.reason, now, stop)
        } else {
            acct.withdrawable += b.notional - b.fee_usd;
            *acct.positions.entry(coin.clone()).or_default() -= b.sz;
            // A full exit closes the ledger position once less than one size step is left.
            let dust = if full_exit { (live.step(&coin) * b.price).max(1e-6) * 1.01 } else { 1e-6 };
            state.book_sell(&a.symbol, b.sz, b.price, b.notional, b.notional - b.fee_usd, b.fee_usd, &a.reason, now, &cfg.risk, dust)
        };
        f.order_id = Some(b.order_id.clone());
        f.venue = Some(live.venue().into());
        f.events = a.drivers.clone();
        f.gates = a.gates.clone();
        f.binding = a.binding.map(|b| format!("{b:?}"));
        fills.push(f);
    }
    (fills, errors)
}

// ---------------------------------------------------------------- commands

/// `jev-trader hl check`: keys, agent, account, coins, leverage. Sends no orders.
pub async fn check(cfg: &Config) -> Result<()> {
    let ex = Exchange::connect(&cfg.exchange).await?;
    println!("Hyperliquid {} · account {} · agent {}", ex.network.name(), ex.user, ex.agent_address());
    let acct = ex.account().await?;
    println!("perps account value {:.2} USDC · withdrawable {:.2}", acct.account_value, acct.withdrawable);
    if acct.account_value < 1.0 {
        match ex.spot_usdc().await {
            Ok(u) if u > 0.0 => println!("note: {u:.2} USDC sits in the spot account; move it to perps (or enable unified margin) before trading"),
            _ => {}
        }
    }
    println!("bot would manage {:.2} (max_capital_usd {:.2})", cfg.exchange.max_capital_usd.min(acct.withdrawable), cfg.exchange.max_capital_usd);
    for a in &cfg.assets {
        match ex.asset(&a.coin) {
            Ok(info) => println!(
                "{:<7} asset #{:<4} size step {} · max leverage {}× · position {}",
                a.coin,
                info.index,
                info.step(),
                info.max_leverage,
                acct.positions.get(&a.coin).copied().unwrap_or(0.0)
            ),
            Err(e) => println!("{:<7} MISSING: {e:#}", a.coin),
        }
    }
    // The agent must be approved for this account, or every order is rejected.
    // Hyperliquid has no dry-run order; an impossible cancel shows whether the signature is accepted.
    match ex.cancel_by_cloid(&cfg.assets[0].coin, "0x00000000000000000000000000000000").await {
        Err(OrderError::Rejected(m)) if m.to_lowercase().contains("does not exist") || m.to_lowercase().contains("user or api wallet") => {
            println!("agent signature: REJECTED ({m}). Approve the agent for this account at app.hyperliquid.xyz/API")
        }
        Err(OrderError::Rejected(m)) => println!("agent signature: accepted (test cancel answered: {m})"),
        Ok(()) => println!("agent signature: accepted"),
        Err(e) => println!("agent signature: could not tell ({e})"),
    }
    Ok(())
}

/// `jev-trader hl clear-hold`: clears the hold set after an unconfirmed order, in
/// both the news ledger and the scalper ledger of this data_dir.
pub fn clear_hold(cfg: &Config) -> Result<()> {
    let paths = crate::status::Paths::new(&cfg.data_dir);
    let mut any = false;
    if paths.state.exists() {
        let mut s = State::load_or_new(&paths.state, 0.0)?;
        if let Some(h) = s.live_hold.take() {
            s.save(&paths.state)?;
            println!("news ledger: cleared {h}");
            any = true;
        }
    }
    if crate::scalp::clear_hold(&cfg.data_dir)? {
        any = true;
    }
    if !any {
        println!("no hold set");
    }
    Ok(())
}
