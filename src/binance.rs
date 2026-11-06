//! Signed Binance spot REST client: balances, symbol rules, market orders.
//! Keys come from BINANCE_API_KEY / BINANCE_SECRET_KEY. Only spot market orders and
//! read endpoints are used; nothing here can withdraw or transfer.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

use crate::config::ExchangeConfig;

/// Why an order call failed, and whether it might still have executed.
#[derive(Debug)]
pub enum OrderError {
    /// Definitely not executed (rejected, never sent, rate-limited).
    Rejected(String),
    /// Binance may or may not have executed it: query by client id before doing anything else.
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

/// Trading rules for one pair, from exchangeInfo.
#[derive(Debug, Clone)]
pub struct Rules {
    pub base: String,
    pub quote: String,
    pub trading: bool,
    pub step: f64,
    step_decimals: usize,
    pub min_qty: f64,
    pub min_notional: f64,
}

impl Rules {
    /// Largest multiple of the step size not above `qty`.
    pub fn floor_qty(&self, qty: f64) -> f64 {
        if self.step <= 0.0 {
            return qty;
        }
        ((qty / self.step) + 1e-9).floor() * self.step
    }

    pub fn fmt_qty(&self, qty: f64) -> String {
        format!("{:.*}", self.step_decimals, qty)
    }
}

/// What actually executed.
#[derive(Debug, Clone, Default)]
pub struct Execution {
    pub order_id: String,
    pub status: String,
    /// Gross base quantity filled.
    pub executed_qty: f64,
    /// Quote currency spent (buy) or received (sell), before quote-denominated fees.
    pub quote_qty: f64,
    /// Fees, by the asset they were charged in.
    pub commissions: HashMap<String, f64>,
}

impl Execution {
    pub fn avg_price(&self) -> f64 {
        if self.executed_qty > 0.0 { self.quote_qty / self.executed_qty } else { 0.0 }
    }
}

pub enum Qty {
    /// Spend this much quote currency (market buy).
    Quote(f64),
    /// Sell this much base currency (market sell), formatted with `Rules::fmt_qty`.
    Base(String),
}

pub struct Binance {
    http: reqwest::Client,
    base: String,
    key: String,
    secret: String,
    recv_window: u64,
    offset_ms: i64,
}

type HmacSha256 = Hmac<Sha256>;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn num(v: &Value, k: &str) -> f64 {
    match &v[k] {
        Value::String(s) => s.parse().unwrap_or(0.0),
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn decimals(step: &str) -> usize {
    match step.split_once('.') {
        Some((_, frac)) => frac.trim_end_matches('0').len(),
        None => 0,
    }
}

impl Binance {
    pub fn from_env(cfg: &ExchangeConfig) -> Result<Self> {
        let key = std::env::var(&cfg.api_key_env).with_context(|| format!("{} is not set", cfg.api_key_env))?;
        let secret = std::env::var(&cfg.secret_env).with_context(|| format!("{} is not set", cfg.secret_env))?;
        let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).user_agent("jev-trader/0.1").build()?;
        Ok(Self { http, base: cfg.base_url.trim_end_matches('/').into(), key, secret, recv_window: cfg.recv_window_ms, offset_ms: 0 })
    }

    /// Aligns request timestamps with Binance's clock.
    pub async fn sync_time(&mut self) -> Result<()> {
        let t0 = now_ms();
        let v: Value = self.http.get(format!("{}/api/v3/time", self.base)).send().await?.error_for_status()?.json().await?;
        let t1 = now_ms();
        let server = v["serverTime"].as_i64().context("no serverTime")?;
        self.offset_ms = server - (t0 + t1) / 2;
        Ok(())
    }

    fn sign(&self, params: &[(&str, String)]) -> String {
        let mut q: Vec<String> = params.iter().map(|(k, v)| format!("{k}={}", urlencode(v))).collect();
        q.push(format!("recvWindow={}", self.recv_window));
        q.push(format!("timestamp={}", now_ms() + self.offset_ms));
        let q = q.join("&");
        let mut mac = HmacSha256::new_from_slice(self.secret.as_bytes()).expect("hmac accepts any key length");
        mac.update(q.as_bytes());
        format!("{q}&signature={}", hex::encode(mac.finalize().into_bytes()))
    }

    async fn signed(&self, method: reqwest::Method, path: &str, params: &[(&str, String)]) -> std::result::Result<Value, OrderError> {
        let url = format!("{}{path}?{}", self.base, self.sign(params));
        let resp = match self.http.request(method, &url).header("X-MBX-APIKEY", &self.key).send().await {
            Ok(r) => r,
            Err(e) if e.is_connect() || e.is_builder() => return Err(OrderError::Rejected(format!("not sent: {e}"))),
            Err(e) => return Err(OrderError::Unknown(format!("{e}"))),
        };
        let status = resp.status();
        let body = resp.text().await.map_err(|e| OrderError::Unknown(format!("reading body: {e}")))?;
        if status.is_success() {
            return serde_json::from_str(&body).map_err(|e| OrderError::Unknown(format!("bad JSON ({e}): {body}")));
        }
        let code = serde_json::from_str::<Value>(&body).ok().and_then(|v| v["code"].as_i64());
        let msg = format!("HTTP {status}: {body}");
        // 5xx and -1007 ("send status unknown") may have executed; 4xx did not.
        if status.is_server_error() || code == Some(-1007) {
            Err(OrderError::Unknown(msg))
        } else {
            Err(OrderError::Rejected(msg))
        }
    }

    /// Free balance per asset (non-zero only).
    pub async fn balances(&self) -> Result<HashMap<String, f64>> {
        let v = self
            .signed(reqwest::Method::GET, "/api/v3/account", &[("omitZeroBalances", "true".into())])
            .await
            .map_err(|e| anyhow!("account: {e}"))?;
        if v["canTrade"].as_bool() == Some(false) {
            bail!("this API key cannot trade spot (enable Spot & Margin Trading on the key)");
        }
        let mut out = HashMap::new();
        for b in v["balances"].as_array().into_iter().flatten() {
            let free = num(b, "free");
            if free > 0.0 {
                out.insert(b["asset"].as_str().unwrap_or_default().to_string(), free);
            }
        }
        Ok(out)
    }

    /// Trading rules for the given pairs (public endpoint).
    pub async fn rules(&self, pairs: &[String]) -> Result<HashMap<String, Rules>> {
        let symbols = serde_json::to_string(pairs)?;
        let v: Value = self
            .http
            .get(format!("{}/api/v3/exchangeInfo", self.base))
            .query(&[("symbols", symbols)])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let mut out = HashMap::new();
        for s in v["symbols"].as_array().into_iter().flatten() {
            let pair = s["symbol"].as_str().unwrap_or_default().to_string();
            let mut r = Rules {
                base: s["baseAsset"].as_str().unwrap_or_default().into(),
                quote: s["quoteAsset"].as_str().unwrap_or_default().into(),
                trading: s["status"] == "TRADING"
                    && s["orderTypes"].as_array().is_some_and(|t| t.iter().any(|x| x == "MARKET")),
                step: 0.0,
                step_decimals: 8,
                min_qty: 0.0,
                min_notional: 0.0,
            };
            for f in s["filters"].as_array().into_iter().flatten() {
                match f["filterType"].as_str() {
                    Some("LOT_SIZE") | Some("MARKET_LOT_SIZE") => {
                        let step = num(f, "stepSize");
                        if step > r.step {
                            r.step = step;
                            r.step_decimals = decimals(f["stepSize"].as_str().unwrap_or("0.00000001"));
                        }
                        r.min_qty = r.min_qty.max(num(f, "minQty"));
                    }
                    Some("NOTIONAL") if f["applyMinToMarket"].as_bool() != Some(false) => {
                        r.min_notional = r.min_notional.max(num(f, "minNotional"));
                    }
                    Some("MIN_NOTIONAL") if f["applyToMarket"].as_bool() != Some(false) => {
                        r.min_notional = r.min_notional.max(num(f, "minNotional"));
                    }
                    _ => {}
                }
            }
            out.insert(pair, r);
        }
        for p in pairs {
            if !out.contains_key(p) {
                bail!("{p} is not listed on {}", self.base);
            }
        }
        Ok(out)
    }

    /// Market order. `test` uses /order/test: Binance validates and signs it, nothing executes.
    pub async fn market_order(
        &self,
        pair: &str,
        buy: bool,
        qty: Qty,
        client_id: &str,
        test: bool,
    ) -> std::result::Result<Execution, OrderError> {
        let mut p: Vec<(&str, String)> = vec![
            ("symbol", pair.into()),
            ("side", if buy { "BUY" } else { "SELL" }.into()),
            ("type", "MARKET".into()),
            ("newClientOrderId", client_id.into()),
            ("newOrderRespType", "FULL".into()),
        ];
        match qty {
            // Two decimals: quote is a dollar stablecoin; floor so we never overspend.
            Qty::Quote(q) => p.push(("quoteOrderQty", format!("{:.2}", (q * 100.0).floor() / 100.0))),
            Qty::Base(q) => p.push(("quantity", q)),
        }
        let path = if test { "/api/v3/order/test" } else { "/api/v3/order" };
        let v = self.signed(reqwest::Method::POST, path, &p).await?;
        if test {
            return Ok(Execution { status: "TEST".into(), ..Default::default() });
        }
        Ok(parse_execution(&v))
    }

    /// Looks an order up by client id. `Ok(None)` = Binance has no such order.
    pub async fn find_order(&self, pair: &str, client_id: &str) -> Result<Option<Execution>> {
        let r = self
            .signed(reqwest::Method::GET, "/api/v3/order", &[("symbol", pair.into()), ("origClientOrderId", client_id.into())])
            .await;
        let v = match r {
            Ok(v) => v,
            Err(OrderError::Rejected(m)) if m.contains("-2013") => return Ok(None),
            Err(e) => bail!("order lookup: {e}"),
        };
        let mut ex = parse_execution(&v);
        // The order endpoint has no fills: take commissions from the account's trades.
        if ex.executed_qty > 0.0 {
            let trades = self
                .signed(reqwest::Method::GET, "/api/v3/myTrades", &[("symbol", pair.into()), ("orderId", ex.order_id.clone())])
                .await
                .map_err(|e| anyhow!("trade lookup: {e}"))?;
            for t in trades.as_array().into_iter().flatten() {
                *ex.commissions.entry(t["commissionAsset"].as_str().unwrap_or_default().into()).or_default() += num(t, "commission");
            }
        }
        Ok(Some(ex))
    }
}

fn parse_execution(v: &Value) -> Execution {
    let mut commissions: HashMap<String, f64> = HashMap::new();
    for f in v["fills"].as_array().into_iter().flatten() {
        *commissions.entry(f["commissionAsset"].as_str().unwrap_or_default().into()).or_default() += num(f, "commission");
    }
    Execution {
        order_id: match &v["orderId"] {
            Value::Number(n) => n.to_string(),
            other => other.as_str().unwrap_or_default().to_string(),
        },
        status: v["status"].as_str().unwrap_or_default().into(),
        executed_qty: num(v, "executedQty"),
        quote_qty: num(v, "cummulativeQuoteQty"),
        commissions,
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_matches_binance_docs() {
        // Example from the Binance spot API docs (HMAC SHA256 section).
        let secret = "NhqPtmdSJYdKjVHjA7PZj4Mge3R5YNiP1e3UZjInClVN65XAbvqqM6A7H5fATj0j";
        let q = "symbol=LTCBTC&side=BUY&type=LIMIT&timeInForce=GTC&quantity=1&price=0.1&recvWindow=5000&timestamp=1499827319559";
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(q.as_bytes());
        assert_eq!(hex::encode(mac.finalize().into_bytes()), "c8db56825ae71d6d79447849e617115f4a920fa2acdcab2b053c4b2838bd6b71");
    }

    #[test]
    fn floors_to_step() {
        let r = Rules { base: "BTC".into(), quote: "USDT".into(), trading: true, step: 0.00001, step_decimals: 5, min_qty: 0.00001, min_notional: 5.0 };
        assert_eq!(r.fmt_qty(r.floor_qty(0.123456789)), "0.12345");
        assert_eq!(r.fmt_qty(r.floor_qty(0.00003)), "0.00003");
        assert_eq!(decimals("0.00100000"), 3);
        assert_eq!(decimals("1.00000000"), 0);
    }

    #[test]
    fn parses_full_response() {
        let v: Value = serde_json::from_str(r#"{"orderId":28,"status":"FILLED","executedQty":"0.002","cummulativeQuoteQty":"200.0",
            "fills":[{"price":"100000","qty":"0.001","commission":"0.000001","commissionAsset":"BTC"},
                     {"price":"100000","qty":"0.001","commission":"0.000001","commissionAsset":"BTC"}]}"#).unwrap();
        let e = parse_execution(&v);
        assert_eq!(e.order_id, "28");
        assert!((e.avg_price() - 100_000.0).abs() < 1e-6);
        assert!((e.commissions["BTC"] - 0.000002).abs() < 1e-12);
    }
}
