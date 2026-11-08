//! Signed Hyperliquid exchange client (orders, cancels, leverage) and the per-user
//! info queries needed to trust a fill: account state, order status, user fills.
//!
//! Keys: `HL_ACCOUNT_ADDRESS` is the main account (public; positions and margin live
//! there) and `HL_AGENT_PRIVATE_KEY` is an API agent wallet approved for it at
//! app.hyperliquid.xyz/API. An agent can place and cancel orders but cannot
//! withdraw or transfer. Nothing in this file can move funds.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use super::sign::{Mp, Signer, float_to_wire};
use super::{Network, num};
use crate::config::ExchangeConfig;

/// Why an exchange call failed, and whether it might still have executed.
#[derive(Debug)]
pub enum OrderError {
    /// Definitely not executed (rejected by the exchange, or never sent).
    Rejected(String),
    /// May or may not have executed: look the order up by cloid before anything else.
    Unknown(String),
}

impl std::fmt::Display for OrderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderError::Rejected(m) => write!(f, "rejected: {m}"),
            OrderError::Unknown(m) => write!(f, "outcome unknown: {m}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tif {
    /// Post-only: rejected instead of taking liquidity.
    Alo,
    /// Immediate-or-cancel: the nearest thing to a market order.
    Ioc,
}

impl Tif {
    fn wire(self) -> &'static str {
        match self {
            Tif::Alo => "Alo",
            Tif::Ioc => "Ioc",
        }
    }
}

#[derive(Debug, Clone)]
pub struct OrderReq {
    pub coin: String,
    pub buy: bool,
    pub px: f64,
    pub sz: f64,
    pub reduce_only: bool,
    pub tif: Tif,
    pub cloid: String,
    /// Some(px): a stop-market trigger order (tpsl "sl") instead of a limit; `px`
    /// is then the worst fill price accepted once it triggers.
    pub trigger_px: Option<f64>,
}

/// Per-order result of a placement.
#[derive(Debug, Clone, PartialEq)]
pub enum Placed {
    Resting { oid: u64 },
    Filled { oid: u64, sz: f64, avg_px: f64 },
    Error(String),
}

#[derive(Debug, Clone)]
pub struct AssetInfo {
    pub index: u32,
    pub sz_decimals: u32,
    pub max_leverage: u32,
}

impl AssetInfo {
    /// Largest size step multiple not above `sz`.
    pub fn floor_sz(&self, sz: f64) -> f64 {
        let m = 10f64.powi(self.sz_decimals as i32);
        ((sz * m) + 1e-9).floor() / m
    }

    pub fn step(&self) -> f64 {
        10f64.powi(-(self.sz_decimals as i32))
    }

    /// A valid perp price: 5 significant figures and at most 6 − szDecimals decimals
    /// (integers always allowed). `up` rounds away from the book for a buy limit.
    pub fn round_px(&self, px: f64, up: bool) -> f64 {
        if !(px.is_finite() && px > 0.0) {
            return px;
        }
        // Integer prices are always valid, whatever their significant figures.
        if px >= 10_000.0 {
            return if up { (px - 1e-9).ceil() } else { (px + 1e-9).floor() };
        }
        let mag = px.log10().floor() as i32; // 0 for 1..10, −2 for 0.01..0.1
        let dec = (4 - mag).min(6 - self.sz_decimals as i32).max(0);
        let m = 10f64.powi(dec);
        let r = if up { (px * m - 1e-9).ceil() / m } else { (px * m + 1e-9).floor() / m };
        // Strip binary noise so float_to_wire accepts it.
        format!("{r:.8}").parse().unwrap_or(r)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Account {
    pub account_value: f64,
    pub withdrawable: f64,
    /// coin → signed size (positive = long).
    pub positions: HashMap<String, f64>,
    pub entry_px: HashMap<String, f64>,
}

/// One fill from `userFillsByTime`.
#[derive(Debug, Clone)]
pub struct UserFill {
    pub px: f64,
    pub sz: f64,
    pub oid: u64,
    pub fee: f64,
}

/// What `orderStatus` reports for a cloid.
#[derive(Debug, Clone, PartialEq)]
pub enum OrderState {
    Unknown,
    /// "open", "filled", "canceled", "rejected", "marginCanceled", ...
    Known { status: String, oid: u64, orig_sz: f64, remaining: f64, limit_px: f64 },
}

pub struct Exchange {
    http: reqwest::Client,
    pub network: Network,
    signer: Signer,
    /// Main account the agent trades for (lowercase 0x hex).
    pub user: String,
    pub assets: HashMap<String, AssetInfo>,
    last_nonce: AtomicU64,
    cloid_seq: AtomicU64,
}

fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

impl Exchange {
    pub async fn connect(cfg: &ExchangeConfig) -> Result<Self> {
        let user = std::env::var(&cfg.account_env).with_context(|| format!("{} is not set (your main Hyperliquid address)", cfg.account_env))?;
        let key = std::env::var(&cfg.agent_key_env).with_context(|| format!("{} is not set (an API agent wallet's private key)", cfg.agent_key_env))?;
        let user = user.trim().to_lowercase();
        if !(user.starts_with("0x") && user.len() == 42 && hex::decode(&user[2..]).is_ok()) {
            bail!("{} must be a 0x address", cfg.account_env);
        }
        let signer = Signer::from_hex(&key)?;
        if signer.address == user {
            bail!("{} is the main wallet's key; use an API agent wallet (it can trade but not withdraw)", cfg.agent_key_env);
        }
        let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).user_agent("jev-trader/0.1").build()?;
        let mut ex = Self { http, network: cfg.network, signer, user, assets: HashMap::new(), last_nonce: AtomicU64::new(0), cloid_seq: AtomicU64::new(0) };
        ex.assets = super::perp_meta(&ex.http, cfg.network.api_url()).await?;
        Ok(ex)
    }

    pub fn agent_address(&self) -> &str {
        &self.signer.address
    }

    pub fn asset(&self, coin: &str) -> Result<&AssetInfo> {
        self.assets.get(coin).ok_or_else(|| anyhow!("{coin} is not a perp on Hyperliquid {}", self.network.name()))
    }

    /// Strictly increasing millisecond nonce.
    fn nonce(&self) -> u64 {
        let mut prev = self.last_nonce.load(Ordering::SeqCst);
        loop {
            let n = now_ms().max(prev + 1);
            match self.last_nonce.compare_exchange(prev, n, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return n,
                Err(p) => prev = p,
            }
        }
    }

    /// A fresh 16-byte client order id: millisecond time, process id, sequence.
    pub fn new_cloid(&self) -> String {
        let seq = self.cloid_seq.fetch_add(1, Ordering::SeqCst) & 0xffff_ffff;
        format!("0x{:016x}{:08x}{:08x}", now_ms(), std::process::id(), seq)
    }

    pub async fn info(&self, body: Value) -> Result<Value> {
        super::info_at(&self.http, self.network.api_url(), body).await
    }

    async fn post_action(&self, action: Mp) -> std::result::Result<Value, OrderError> {
        let nonce = self.nonce();
        let sig = self.signer.sign_l1_action(&action, None, nonce, self.network == Network::Mainnet).map_err(|e| OrderError::Rejected(format!("{e:#}")))?;
        let body = json!({ "action": action.to_json(), "nonce": nonce, "signature": sig, "vaultAddress": null });
        let url = format!("{}/exchange", self.network.api_url());
        let resp = match self.http.post(&url).json(&body).send().await {
            Ok(r) => r,
            Err(e) if e.is_connect() || e.is_builder() => return Err(OrderError::Rejected(format!("not sent: {e}"))),
            Err(e) => return Err(OrderError::Unknown(format!("{e}"))),
        };
        let status = resp.status();
        let text = resp.text().await.map_err(|e| OrderError::Unknown(format!("reading body: {e}")))?;
        if status.is_server_error() {
            return Err(OrderError::Unknown(format!("HTTP {status}: {text}")));
        }
        if !status.is_success() {
            return Err(OrderError::Rejected(format!("HTTP {status}: {text}")));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| OrderError::Unknown(format!("bad JSON ({e}): {text}")))?;
        if v["status"] != "ok" {
            return Err(OrderError::Rejected(v["response"].as_str().map(str::to_string).unwrap_or(text)));
        }
        Ok(v)
    }

    fn order_wire(&self, o: &OrderReq) -> Result<Mp> {
        let a = self.asset(&o.coin)?;
        Ok(Mp::map(vec![
            ("a", Mp::UInt(a.index as u64)),
            ("b", Mp::Bool(o.buy)),
            ("p", Mp::str(float_to_wire(o.px)?)),
            ("s", Mp::str(float_to_wire(o.sz)?)),
            ("r", Mp::Bool(o.reduce_only)),
            ("t", match o.trigger_px {
                // Key order matches the SDK's order_type_to_wire (tested in sign.rs).
                Some(t) => Mp::map(vec![("trigger", Mp::map(vec![("isMarket", Mp::Bool(true)), ("triggerPx", Mp::str(float_to_wire(t)?)), ("tpsl", Mp::str("sl"))]))]),
                None => Mp::map(vec![("limit", Mp::map(vec![("tif", Mp::str(o.tif.wire()))]))]),
            }),
            ("c", Mp::str(o.cloid.clone())),
        ]))
    }

    /// Places one order. `Ok(Placed::Error)` = the exchange refused this order (not executed).
    pub async fn order(&self, o: &OrderReq) -> std::result::Result<Placed, OrderError> {
        let wire = self.order_wire(o).map_err(|e| OrderError::Rejected(format!("{e:#}")))?;
        let action = Mp::map(vec![("type", Mp::str("order")), ("orders", Mp::Array(vec![wire])), ("grouping", Mp::str("na"))]);
        let v = self.post_action(action).await?;
        let st = &v["response"]["data"]["statuses"][0];
        Ok(parse_placed(st).unwrap_or_else(|| Placed::Error(format!("unexpected status: {st}"))))
    }

    pub async fn cancel_by_cloid(&self, coin: &str, cloid: &str) -> std::result::Result<(), OrderError> {
        let a = self.asset(coin).map_err(|e| OrderError::Rejected(format!("{e:#}")))?;
        let action = Mp::map(vec![
            ("type", Mp::str("cancelByCloid")),
            ("cancels", Mp::Array(vec![Mp::map(vec![("asset", Mp::UInt(a.index as u64)), ("cloid", Mp::str(cloid))])])),
        ]);
        let v = self.post_action(action).await?;
        match v["response"]["data"]["statuses"][0].clone() {
            Value::String(s) if s == "success" => Ok(()),
            other => Err(OrderError::Rejected(other["error"].as_str().map(str::to_string).unwrap_or_else(|| other.to_string()))),
        }
    }

    pub async fn update_leverage(&self, coin: &str, leverage: u32, cross: bool) -> std::result::Result<(), OrderError> {
        let a = self.asset(coin).map_err(|e| OrderError::Rejected(format!("{e:#}")))?;
        let action = Mp::map(vec![
            ("type", Mp::str("updateLeverage")),
            ("asset", Mp::UInt(a.index as u64)),
            ("isCross", Mp::Bool(cross)),
            ("leverage", Mp::UInt(leverage as u64)),
        ]);
        self.post_action(action).await.map(|_| ())
    }

    pub async fn account(&self) -> Result<Account> {
        let v = self.info(json!({"type": "clearinghouseState", "user": self.user})).await?;
        let mut a = Account { account_value: num(&v["marginSummary"]["accountValue"]), withdrawable: num(&v["withdrawable"]), ..Default::default() };
        if !a.account_value.is_finite() {
            bail!("clearinghouseState without accountValue: {v}");
        }
        for p in v["assetPositions"].as_array().into_iter().flatten() {
            let p = &p["position"];
            let coin = p["coin"].as_str().unwrap_or_default().to_string();
            a.positions.insert(coin.clone(), num(&p["szi"]));
            a.entry_px.insert(coin, num(&p["entryPx"]));
        }
        Ok(a)
    }

    /// Spot USDC (total), to explain an empty perps account under unified margin.
    pub async fn spot_usdc(&self) -> Result<f64> {
        let v = self.info(json!({"type": "spotClearinghouseState", "user": self.user})).await?;
        Ok(v["balances"].as_array().into_iter().flatten().find(|b| b["coin"] == "USDC").map(|b| num(&b["total"])).unwrap_or(0.0))
    }

    pub async fn order_status(&self, cloid: &str) -> Result<OrderState> {
        let v = self.info(json!({"type": "orderStatus", "user": self.user, "oid": cloid})).await?;
        Ok(parse_order_state(&v))
    }

    pub async fn fills_since(&self, start_ms: i64) -> Result<Vec<UserFill>> {
        let v = self.info(json!({"type": "userFillsByTime", "user": self.user, "startTime": start_ms, "aggregateByTime": false})).await?;
        Ok(v.as_array()
            .into_iter()
            .flatten()
            .map(|f| UserFill { px: num(&f["px"]), sz: num(&f["sz"]), oid: f["oid"].as_u64().unwrap_or(0), fee: num(&f["fee"]) })
            .collect())
    }
}

fn parse_placed(st: &Value) -> Option<Placed> {
    // Trigger orders may answer with a bare status string.
    if let Some(t) = st.as_str() {
        return Some(if t.contains("waitingFor") || t == "success" { Placed::Resting { oid: 0 } } else { Placed::Error(t.to_string()) });
    }
    if let Some(e) = st["error"].as_str() {
        return Some(Placed::Error(e.to_string()));
    }
    if let Some(r) = st.get("resting") {
        return Some(Placed::Resting { oid: r["oid"].as_u64()? });
    }
    if let Some(f) = st.get("filled") {
        return Some(Placed::Filled { oid: f["oid"].as_u64()?, sz: num(&f["totalSz"]), avg_px: num(&f["avgPx"]) });
    }
    None
}

fn parse_order_state(v: &Value) -> OrderState {
    if v["status"] != "order" {
        return OrderState::Unknown;
    }
    let o = &v["order"];
    OrderState::Known {
        status: o["status"].as_str().unwrap_or_default().to_string(),
        oid: o["order"]["oid"].as_u64().unwrap_or(0),
        orig_sz: num(&o["order"]["origSz"]),
        remaining: num(&o["order"]["sz"]),
        limit_px: num(&o["order"]["limitPx"]),
    }
}

/// Size-weighted price, total size and fees of the fills of one order.
pub fn sum_fills(fills: &[UserFill], oid: u64) -> Option<(f64, f64, f64)> {
    let mine: Vec<&UserFill> = fills.iter().filter(|f| f.oid == oid).collect();
    let sz: f64 = mine.iter().map(|f| f.sz).sum();
    if sz <= 0.0 {
        return None;
    }
    let px = mine.iter().map(|f| f.px * f.sz).sum::<f64>() / sz;
    Some((sz, px, mine.iter().map(|f| f.fee).sum()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_follow_tick_rules() {
        let btc = AssetInfo { index: 0, sz_decimals: 5, max_leverage: 40 };
        assert_eq!(btc.round_px(75_881.37, true), 75_882.0);
        assert_eq!(btc.round_px(75_881.37, false), 75_881.0);
        assert_eq!(btc.round_px(123_456.7, false), 123_456.0, "integers are always valid");
        assert_eq!(btc.round_px(9_876.543, false), 9_876.5, "5 significant figures");
        let eth = AssetInfo { index: 1, sz_decimals: 4, max_leverage: 25 };
        assert_eq!(eth.round_px(3456.789, false), 3456.7);
        assert_eq!(eth.round_px(3456.721, true), 3456.8);
        let pepe = AssetInfo { index: 2, sz_decimals: 0, max_leverage: 10 };
        assert_eq!(pepe.round_px(0.0123456, false), 0.012345);
        let sol = AssetInfo { index: 3, sz_decimals: 2, max_leverage: 20 };
        assert_eq!(sol.round_px(187.123, false), 187.12);
        assert_eq!(sol.round_px(1.234567, true), 1.2346, "max 4 decimals for szDecimals 2");
        assert_eq!(sol.floor_sz(1.239), 1.23);
        assert!(float_to_wire(sol.round_px(187.129, true)).is_ok());
    }

    #[test]
    fn parses_statuses() {
        let v: Value = serde_json::from_str(r#"{"filled":{"totalSz":"0.02","avgPx":"1891.4","oid":77738308}}"#).unwrap();
        assert_eq!(parse_placed(&v), Some(Placed::Filled { oid: 77738308, sz: 0.02, avg_px: 1891.4 }));
        let v: Value = serde_json::from_str(r#"{"resting":{"oid":77738308}}"#).unwrap();
        assert_eq!(parse_placed(&v), Some(Placed::Resting { oid: 77738308 }));
        let v: Value = serde_json::from_str(r#"{"error":"Post only order would have immediately matched"}"#).unwrap();
        assert!(matches!(parse_placed(&v), Some(Placed::Error(_))));
        let v: Value = serde_json::from_str(r#"{"status":"order","order":{"order":{"coin":"ETH","limitPx":"2000.0","oid":9,"origSz":"0.5","sz":"0.2"},"status":"canceled","statusTimestamp":1}}"#).unwrap();
        assert_eq!(parse_order_state(&v), OrderState::Known { status: "canceled".into(), oid: 9, orig_sz: 0.5, remaining: 0.2, limit_px: 2000.0 });
        assert_eq!(parse_order_state(&serde_json::json!({"status":"unknownOid"})), OrderState::Unknown);
    }

    #[test]
    fn sums_fills_by_order() {
        let f = |oid, px, sz, fee| UserFill { px, sz, oid, fee };
        let (sz, px, fee) = sum_fills(&[f(1, 100.0, 1.0, 0.1), f(1, 102.0, 1.0, 0.1), f(2, 50.0, 9.0, 1.0)], 1).unwrap();
        assert!((sz - 2.0).abs() < 1e-12 && (px - 101.0).abs() < 1e-12 && (fee - 0.2).abs() < 1e-12);
        assert!(sum_fills(&[], 1).is_none());
    }
}
