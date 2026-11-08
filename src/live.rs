//! Live execution on Hyperliquid perps. The engine's decision path is the same as
//! paper; only the fills differ: each one is a real order, booked from what the
//! exchange reports.
//!
//! Safety:
//! - armed only by `[exchange] mode = "live"` AND `run --live`;
//! - the signer is an API agent wallet (can trade, cannot withdraw);
//! - the live ledger lives in its own data_dir, records its network, and manages
//!   at most `max_capital_usd`;
//! - IOC orders are capped `max_slippage_bps` through the price; exits, take-profits
//!   and backstop stops are reduce-only, so they can never open or flip a position;
//! - fills are the source of truth: amounts and fees come from `userFills`;
//! - an order whose outcome can't be confirmed sets a hold that blocks all entries
//!   until a person checks the account and runs `jev-trader hl clear-hold`.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Result, bail};
use chrono::Utc;
use tracing::{info, warn};

use crate::config::Config;
use crate::hyperliquid::exchange::{Exchange, OrderError, OrderReq, OrderState, Placed, Tif, sum_fills};

/// Hyperliquid rejects orders worth less than $10; stay a little above it.
pub const MIN_ORDER_USD: f64 = 10.5;

pub struct Live {
    pub ex: Exchange,
    pub max_slippage_bps: f64,
    /// Fallback taker fee (fraction) if the fill lookup fails.
    pub taker_fee: f64,
    leverage: u32,
    /// Coins whose leverage has been set this session.
    levered: Mutex<HashSet<String>>,
}

/// A confirmed fill in ledger terms.
#[derive(Debug, Clone)]
pub struct Booked {
    pub sz: f64,
    pub price: f64,
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
        for c in &cfg.watch {
            ex.asset(c)?;
        }
        let live = Self { ex, max_slippage_bps: cfg.exchange.max_slippage_bps, taker_fee: cfg.costs.taker_bps / 1e4, leverage: cfg.exchange.leverage, levered: Mutex::new(HashSet::new()) };
        for c in &cfg.watch {
            if let Err(e) = live.ensure_leverage(c).await {
                bail!("{e} (set [exchange] leverage = 0 to skip)");
            }
        }
        info!(network = live.ex.network.name(), account = %live.ex.user, agent = %live.ex.agent_address(), "Hyperliquid exchange connected");
        Ok(live)
    }

    /// Sets cross leverage on a coin once per session (catalyst trades can open on
    /// coins outside the watch list).
    pub async fn ensure_leverage(&self, coin: &str) -> std::result::Result<(), String> {
        if self.leverage == 0 || self.levered.lock().unwrap().contains(coin) {
            return Ok(());
        }
        let a = self.ex.asset(coin).map_err(|e| format!("{e:#}"))?;
        let lev = self.leverage.min(a.max_leverage);
        self.ex.update_leverage(coin, lev, true).await.map_err(|e| format!("setting {coin} cross leverage to {lev}×: {e}"))?;
        self.levered.lock().unwrap().insert(coin.to_string());
        Ok(())
    }

    pub fn venue(&self) -> &'static str {
        if self.ex.network == crate::config::Network::Testnet { "hyperliquid-testnet" } else { "hyperliquid" }
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
        let req = OrderReq { coin: coin.into(), buy, px, sz, reduce_only, tif: Tif::Ioc, cloid: cloid.clone(), trigger_px: None };
        let book = |sz: f64, px: f64, fee: Option<f64>, oid: u64| Booked {
            sz,
            price: px,
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
    /// `sz` in coins; `reduce_only` for take-profit exits (no minimum size applies then).
    pub async fn post_maker(&self, coin: &str, buy: bool, px: f64, sz: f64, reduce_only: bool) -> std::result::Result<(String, f64, f64), String> {
        let a = self.ex.asset(coin).map_err(|e| format!("{e:#}"))?;
        let px = a.round_px(px, !buy); // never round across the book
        let sz = a.floor_sz(sz);
        if sz <= 0.0 || (!reduce_only && sz * px < MIN_ORDER_USD) {
            return Err(format!("{coin}: ${:.2} is below the exchange minimum", sz * px));
        }
        let cloid = self.ex.new_cloid();
        let since = now_ms() - 5_000;
        let req = OrderReq { coin: coin.into(), buy, px, sz, reduce_only, tif: Tif::Alo, cloid: cloid.clone(), trigger_px: None };
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

    /// A reduce-only stop-market resting on the exchange: protection if the process
    /// dies. `buy` closes a short. Returns its cloid.
    pub async fn post_backstop(&self, coin: &str, buy: bool, trigger: f64, sz: f64) -> std::result::Result<String, String> {
        let a = self.ex.asset(coin).map_err(|e| format!("{e:#}"))?;
        let trigger = a.round_px(trigger, buy);
        // Worst accepted fill once triggered: 5% beyond the trigger.
        let limit = a.round_px(if buy { trigger * 1.05 } else { trigger * 0.95 }, buy);
        let sz = a.floor_sz(sz);
        if sz <= 0.0 {
            return Err(format!("{coin}: backstop size rounds to zero"));
        }
        let cloid = self.ex.new_cloid();
        let req = OrderReq { coin: coin.into(), buy, px: limit, sz, reduce_only: true, tif: Tif::Ioc, cloid: cloid.clone(), trigger_px: Some(trigger) };
        match self.ex.order(&req).await {
            Ok(Placed::Error(m)) => Err(format!("{coin}: {m}")),
            Ok(_) => Ok(cloid),
            Err(e) => Err(format!("{coin}: {e}")),
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
    for c in &cfg.watch {
        match ex.asset(c) {
            Ok(info) => println!("{c:<7} asset #{:<4} size step {} · max leverage {}× · position {}", info.index, info.step(), info.max_leverage, acct.positions.get(c).copied().unwrap_or(0.0)),
            Err(e) => println!("{c:<7} MISSING: {e:#}"),
        }
    }
    // The agent must be approved for this account, or every order is rejected.
    // Hyperliquid has no dry-run order; an impossible cancel shows whether the signature is accepted.
    match ex.cancel_by_cloid(&cfg.watch[0], "0x00000000000000000000000000000000").await {
        Err(OrderError::Rejected(m)) if m.to_lowercase().contains("does not exist") || m.to_lowercase().contains("user or api wallet") => {
            println!("agent signature: REJECTED ({m}). Approve the agent for this account at app.hyperliquid.xyz/API")
        }
        Err(OrderError::Rejected(m)) => println!("agent signature: accepted (test cancel answered: {m})"),
        Ok(()) => println!("agent signature: accepted"),
        Err(e) => println!("agent signature: could not tell ({e})"),
    }
    Ok(())
}

/// `jev-trader hl clear-hold`: clears the hold set after an unconfirmed order.
pub fn clear_hold(cfg: &Config) -> Result<()> {
    if crate::ledger::clear_hold(&crate::status::Files::new(&cfg.data_dir).ledger)? {
        println!("hold cleared");
    } else {
        println!("no hold set");
    }
    Ok(())
}
