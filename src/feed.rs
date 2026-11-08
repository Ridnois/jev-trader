//! Hyperliquid websocket: best bid/ask (`bbo`), every trade (`trades`, side "B" =
//! the taker bought) and perp context (`activeAssetCtx`: funding, OI, mark,
//! oracle). Coins can be added while running (a catalyst on an unwatched coin);
//! after a reconnect every coin is subscribed again.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{info, warn};

use crate::features::AssetMarket;
use crate::hyperliquid::num;

pub type Markets = Arc<Mutex<HashMap<String, AssetMarket>>>;

#[derive(Debug, Default, Clone, Serialize)]
pub struct FeedHealth {
    pub connected: bool,
    pub reconnects: u32,
    pub last_msg_ms: i64,
    pub last_error: String,
    pub coins: usize,
}

fn sub_msgs(coin: &str) -> Vec<String> {
    ["bbo", "trades", "activeAssetCtx"].iter().map(|k| serde_json::json!({"method": "subscribe", "subscription": {"type": k, "coin": coin}}).to_string()).collect()
}

pub async fn run(ws_url: &'static str, initial: Vec<String>, mut add: UnboundedReceiver<String>, markets: Markets, health: Arc<Mutex<FeedHealth>>) {
    use tokio_tungstenite::tungstenite::Message;
    let mut coins: BTreeSet<String> = initial.into_iter().collect();
    let mut backoff = 1u64;
    loop {
        match tokio_tungstenite::connect_async(ws_url).await {
            Ok((mut ws, _)) => {
                let mut ok = true;
                for c in &coins {
                    for m in sub_msgs(c) {
                        ok &= ws.send(Message::Text(m.into())).await.is_ok();
                    }
                }
                if ok {
                    info!("hyperliquid stream connected ({} coins)", coins.len());
                    backoff = 1;
                    let mut h = health.lock().unwrap();
                    h.connected = true;
                    h.coins = coins.len();
                }
                let mut ping = tokio::time::interval(Duration::from_secs(30));
                while ok {
                    let msg = tokio::select! {
                        _ = ping.tick() => {
                            if ws.send(Message::Text(r#"{"method":"ping"}"#.into())).await.is_err() { break; }
                            continue;
                        }
                        Some(c) = add.recv() => {
                            if coins.insert(c.clone()) {
                                for m in sub_msgs(&c) {
                                    if ws.send(Message::Text(m.into())).await.is_err() { ok = false; }
                                }
                                info!("stream: subscribed {c}");
                                health.lock().unwrap().coins = coins.len();
                            }
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
                    let now = chrono::Utc::now().timestamp_millis();
                    let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                    let d = &v["data"];
                    match v["channel"].as_str().unwrap_or_default() {
                        "bbo" => {
                            let Some(coin) = d["coin"].as_str() else { continue };
                            let (b, a) = (&d["bbo"][0], &d["bbo"][1]);
                            if b.is_null() || a.is_null() {
                                continue;
                            }
                            markets.lock().unwrap().entry(coin.to_string()).or_default().on_book(num(&b["px"]), num(&b["sz"]), num(&a["px"]), num(&a["sz"]), now);
                        }
                        "trades" => {
                            let Some(arr) = d.as_array() else { continue };
                            let mut m = markets.lock().unwrap();
                            for t in arr {
                                let Some(coin) = t["coin"].as_str() else { continue };
                                m.entry(coin.to_string()).or_default().on_trade(num(&t["px"]), num(&t["sz"]), t["side"].as_str() == Some("A"), now);
                            }
                        }
                        "activeAssetCtx" => {
                            let Some(coin) = d["coin"].as_str() else { continue };
                            let c = &d["ctx"];
                            markets.lock().unwrap().entry(coin.to_string()).or_default().on_ctx(num(&c["funding"]), num(&c["openInterest"]), num(&c["markPx"]), num(&c["oraclePx"]));
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
        // Coins requested while disconnected are picked up on the next connect.
        while let Ok(c) = add.try_recv() {
            coins.insert(c);
        }
        tokio::time::sleep(Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(30);
    }
}
