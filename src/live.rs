//! Live execution on Binance spot. The decision path is the same as paper; only the
//! fills differ: each one is a real market order, booked from what Binance reports.
//!
//! Safety:
//! - the live ledger lives in its own data_dir and refuses a paper ledger (and vice versa);
//! - it manages at most `exchange.max_capital_usd`; other balances are never touched;
//! - every tick checks the exchange still holds what the ledger thinks it holds, or
//!   puts that asset in REVIEW (no entries; exits continue);
//! - an order whose outcome can't be confirmed sets `live_hold`, which blocks all
//!   entries until a person checks Binance and runs `jev-trader binance clear-hold`.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use tracing::{error, info, warn};

use crate::binance::{Binance, Execution, OrderError, Qty, Rules};
use crate::config::Config;
use crate::decide::{self, Decision};
use crate::paper::{Binding, Fill, Plan, Skip, State};

pub const MODE: &str = "live";

pub struct Live {
    pub client: Binance,
    /// symbol → (pair, rules)
    pub rules: HashMap<String, (String, Rules)>,
    pub quote: String,
    pub testnet: bool,
}

impl Live {
    pub async fn connect(cfg: &Config) -> Result<Self> {
        let mut client = Binance::from_env(&cfg.exchange)?;
        client.sync_time().await.context("syncing time with Binance")?;
        let pairs: Vec<String> = cfg.assets.iter().map(|a| a.pair.clone()).collect();
        let by_pair = client.rules(&pairs).await?;
        let mut rules = HashMap::new();
        for a in &cfg.assets {
            let r = by_pair[&a.pair].clone();
            if r.quote != cfg.exchange.quote {
                bail!("{} is quoted in {}, not {}", a.pair, r.quote, cfg.exchange.quote);
            }
            if !r.trading {
                bail!("{} is not open for market orders", a.pair);
            }
            rules.insert(a.symbol.clone(), (a.pair.clone(), r));
        }
        Ok(Self { client, rules, quote: cfg.exchange.quote.clone(), testnet: cfg.exchange.is_testnet() })
    }
}

/// Paper commands must never open a live ledger.
pub fn refuse_live_ledger(state: &State) -> Result<()> {
    if state.mode.as_deref() == Some(MODE) {
        bail!("this data_dir holds a LIVE ledger: use the live config (`--config config.live.toml`) and `run --live`");
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
        return Ok(s);
    }
    let cap = cfg.exchange.max_capital_usd;
    if !(cap.is_finite() && cap > 0.0) {
        bail!("set [exchange] max_capital_usd to the most {} the bot may trade with", live.quote);
    }
    let bal = live.client.balances().await?;
    let free = bal.get(&live.quote).copied().unwrap_or(0.0);
    let cash = cap.min(free);
    let min = cfg.risk.min_trade_usd.max(live.rules.values().map(|(_, r)| r.min_notional).fold(0.0, f64::max));
    if cash < min {
        bail!("only {free:.2} {} free on Binance; need at least {min:.2} to place an order", live.quote);
    }
    let mut s = State::new(cash);
    s.mode = Some(MODE.into());
    s.save(path)?;
    info!(cash, cap, free, "live ledger created");
    Ok(s)
}

/// Balances, and per-symbol REVIEW reasons when the exchange holds less than the ledger.
pub async fn reconcile(live: &Live, state: &State) -> Result<(HashMap<String, f64>, HashMap<String, String>)> {
    let bal = live.client.balances().await?;
    let mut review = HashMap::new();
    for (sym, (_, r)) in &live.rules {
        if let Some(hold) = &state.live_hold {
            review.insert(sym.clone(), format!("exchange hold: {hold}"));
            continue;
        }
        if let Some(p) = state.positions.get(sym) {
            let have = bal.get(&r.base).copied().unwrap_or(0.0);
            if have + r.step < p.qty * (1.0 - 1e-6) {
                review.insert(sym.clone(), format!("Binance holds {have} {}, ledger expects {:.8}", r.base, p.qty));
            }
        }
    }
    Ok((bal, review))
}

fn client_id(sym: &str, buy: bool, now: DateTime<Utc>) -> String {
    format!("jev-{}-{}-{}", sym, if buy { "b" } else { "s" }, now.timestamp_millis())
}

/// Sends the order; on an unknown outcome, looks it up before giving up.
async fn place(live: &Live, pair: &str, buy: bool, qty: Qty, id: &str) -> std::result::Result<Execution, String> {
    match live.client.market_order(pair, buy, qty, id, false).await {
        Ok(e) => Ok(e),
        Err(OrderError::Rejected(m)) => Err(m),
        Err(OrderError::Unknown(m)) => {
            warn!(id, "order outcome unknown ({m}); looking it up");
            for attempt in 0..4u32 {
                tokio::time::sleep(Duration::from_millis(500 << attempt)).await;
                match live.client.find_order(pair, id).await {
                    Ok(Some(e)) => return Ok(e),
                    Ok(None) => continue,
                    Err(e) => warn!("lookup failed: {e:#}"),
                }
            }
            Err(format!("UNCONFIRMED order {id} on {pair}: {m}"))
        }
    }
}

/// A confirmed market fill, converted to ledger terms.
#[derive(Debug, Clone)]
pub struct Booked {
    /// Buy: base received net of base-denominated fees. Sell: base that left the account.
    pub qty: f64,
    pub price: f64,
    pub notional: f64,
    /// Quote that left (buy) / entered (sell) the account.
    pub cash_out: f64,
    pub cash_in: f64,
    pub fee_usd: f64,
    pub order_id: String,
    pub venue: &'static str,
}

/// Sends one market order and converts what Binance filled into ledger terms.
/// `Err` starting with "UNCONFIRMED" means the order may have executed.
pub async fn market_fill(live: &Live, sym: &str, buy: bool, qty: Qty, now: DateTime<Utc>, fee_rate: f64) -> std::result::Result<Booked, String> {
    let (pair, r) = live.rules.get(sym).ok_or_else(|| format!("{sym} is not configured"))?;
    let ex = place(live, pair, buy, qty, &client_id(sym, buy, now)).await?;
    if ex.executed_qty <= 0.0 {
        return Err(format!("order {} {} with nothing filled", ex.order_id, ex.status));
    }
    let avg = ex.avg_price();
    let fee_base = ex.commissions.get(&r.base).copied().unwrap_or(0.0);
    let fee_quote = ex.commissions.get(&live.quote).copied().unwrap_or(0.0);
    // Fees in another asset (BNB): estimate in dollars at the configured rate.
    let fee_other = if ex.commissions.iter().any(|(k, v)| *v > 0.0 && k != &r.base && k != &live.quote) { ex.quote_qty * fee_rate } else { 0.0 };
    let fee_usd = fee_base * avg + fee_quote + fee_other;
    let venue = if live.testnet { "binance-testnet" } else { "binance" };
    Ok(if buy {
        Booked { qty: ex.executed_qty - fee_base, price: avg, notional: ex.quote_qty, cash_out: ex.quote_qty + fee_quote, cash_in: 0.0, fee_usd, order_id: ex.order_id, venue }
    } else {
        Booked { qty: ex.executed_qty + fee_base, price: avg, notional: ex.quote_qty, cash_out: 0.0, cash_in: ex.quote_qty - fee_quote, fee_usd, order_id: ex.order_id, venue }
    })
}

/// Applies a decision with real orders. Returns fills and per-order errors.
pub async fn execute(
    live: &Live,
    state: &mut State,
    d: &mut Decision,
    cfg: &Config,
    now: DateTime<Utc>,
    balances: &mut HashMap<String, f64>,
) -> (Vec<Fill>, Vec<String>) {
    let e = d.equity_for_weights;
    let order = decide::prepare(state, d, cfg, now);
    let mut fills = Vec::new();
    let mut errors = Vec::new();
    for i in order {
        let a = &mut d.assets[i];
        let Some(px) = a.exec else { continue };
        let Some((_, r)) = live.rules.get(&a.symbol) else { continue };
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
        let min_usd = cfg.risk.min_trade_usd.max(r.min_notional * 1.02);
        let (buy, qty, full_exit) = match plan {
            Plan::Buy { notional } => {
                let free = balances.get(&live.quote).copied().unwrap_or(0.0);
                let spend = notional.min(free);
                if spend < min_usd {
                    skip(a, Skip::Cash);
                    continue;
                }
                (true, Qty::Quote(spend), false)
            }
            Plan::Sell { qty, full_exit } => {
                let have = balances.get(&r.base).copied().unwrap_or(0.0);
                let q = r.floor_qty(qty.min(have));
                if q < r.min_qty || q * px < r.min_notional {
                    if full_exit {
                        errors.push(format!("{}: position ({q} {}) is below Binance's minimum order; left as is", a.symbol, r.base));
                    }
                    skip(a, Skip::MinTrade);
                    continue;
                }
                (false, Qty::Base(r.fmt_qty(q)), full_exit)
            }
        };
        let b = match market_fill(live, &a.symbol, buy, qty, now, cfg.paper.fee_rate).await {
            Ok(b) => b,
            Err(m) => {
                error!("{} order failed: {m}", a.symbol);
                if m.starts_with("UNCONFIRMED") {
                    state.live_hold = Some(format!("{m} · check Binance, then `jev-trader binance clear-hold`"));
                }
                errors.push(format!("{}: {m}", a.symbol));
                continue;
            }
        };
        let mut f = if buy {
            *balances.entry(live.quote.clone()).or_default() -= b.cash_out;
            *balances.entry(r.base.clone()).or_default() += b.qty;
            state.book_buy(&a.symbol, b.qty, b.price, b.notional, b.cash_out, b.fee_usd, &a.reason, now, stop)
        } else {
            *balances.entry(live.quote.clone()).or_default() += b.cash_in;
            *balances.entry(r.base.clone()).or_default() -= b.qty;
            let dust = if full_exit { r.min_notional.max(1e-6) } else { 1e-6 };
            state.book_sell(&a.symbol, b.qty, b.price, b.notional, b.cash_in, b.fee_usd, &a.reason, now, &cfg.risk, dust)
        };
        f.order_id = Some(b.order_id.clone());
        f.venue = Some(b.venue.into());
        f.events = a.drivers.clone();
        f.gates = a.gates.clone();
        f.binding = a.binding.map(|b| format!("{b:?}"));
        fills.push(f);
    }
    (fills, errors)
}

/// `jev-trader binance check`: keys, permissions, balances, rules, and one signed
/// test order per pair (validated by Binance, never executed).
pub async fn check(cfg: &Config) -> Result<()> {
    let live = Live::connect(cfg).await?;
    println!("Binance {} · clock synced", cfg.exchange.base_url);
    let bal = live.client.balances().await?;
    println!("key can trade spot · {} non-zero balances", bal.len());
    let quote = bal.get(&live.quote).copied().unwrap_or(0.0);
    println!("{} free: {quote:.2}  (bot would manage {:.2} of it)", live.quote, cfg.exchange.max_capital_usd.min(quote));
    let mut syms: Vec<_> = live.rules.iter().collect();
    syms.sort_by_key(|(s, _)| s.as_str());
    for (sym, (pair, r)) in syms {
        let t = live
            .client
            .market_order(pair, true, Qty::Quote(r.min_notional.max(cfg.risk.min_trade_usd).max(10.0)), &format!("jev-check-{sym}"), true)
            .await;
        println!(
            "{pair:<9} step {} · min qty {} · min notional {:.2} · held {} {} · test order {}",
            r.fmt_qty(r.step),
            r.min_qty,
            r.min_notional,
            bal.get(&r.base).copied().unwrap_or(0.0),
            r.base,
            match t {
                Ok(_) => "OK".to_string(),
                Err(e) => format!("FAILED ({e})"),
            }
        );
    }
    Ok(())
}
