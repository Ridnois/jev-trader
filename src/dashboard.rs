//! `jev-trader dashboard`: a local web page for monitoring the bot.
//! Reads the files the bot writes in `data/`; writes only `data/control.json`.
//! The page polls `/api/snapshot` every 2s with If-None-Match.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::Result;
use axum::extract::State as AxState;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::config::Config;
use crate::news::NewsEvent;
use crate::regime::Regime;
use crate::report::{self, Scorecard};
use crate::scalp::report::ScalpScorecard;
use crate::settle::Settlement;
use crate::paper::{Fill, State};
use crate::status::{
    Control, EquityRecord, Paths, PipelineRecord, Status, TickRecord, read_json, read_jsonl, tail_jsonl,
};

const INDEX: &str = include_str!("../dashboard/index.html");
const CSS: &str = include_str!("../dashboard/app.css");
const JS: &str = include_str!("../dashboard/app.js");
/// Barlow (sentences) and Barlow Condensed (board caps and labels), served from the binary.
const FONTS: &[(&str, &[u8])] = &[
    ("barlow-400", include_bytes!("../dashboard/fonts/barlow-400.woff2")),
    ("barlow-500", include_bytes!("../dashboard/fonts/barlow-500.woff2")),
    ("barlow-600", include_bytes!("../dashboard/fonts/barlow-600.woff2")),
    ("barlow-700", include_bytes!("../dashboard/fonts/barlow-700.woff2")),
    ("barlow-condensed-500", include_bytes!("../dashboard/fonts/barlow-condensed-500.woff2")),
    ("barlow-condensed-600", include_bytes!("../dashboard/fonts/barlow-condensed-600.woff2")),
    ("barlow-condensed-700", include_bytes!("../dashboard/fonts/barlow-condensed-700.woff2")),
];

struct App {
    cfg: Config,
    paths: Paths,
    scalp_dir: std::path::PathBuf,
    started_at: DateTime<Utc>,
    /// Scorecard is recomputed only when settlements.jsonl changes.
    scorecard: Mutex<Option<((u64, Option<SystemTime>), Arc<Scorecard>)>>,
    /// The scalper's files grow every few seconds: its scorecard is recomputed at most once a minute.
    scalp_scorecard: Mutex<Option<(std::time::Instant, Arc<ScalpScorecard>)>>,
    http: reqwest::Client,
    /// The one backtest started from the page (at most one runs at a time).
    bt_job: Mutex<Option<BtJob>>,
}

#[derive(Clone, Serialize)]
struct BtJob {
    id: String,
    /// "scalp" (scalper rules on candles) or "news" (cached Jev answers).
    kind: &'static str,
    config: String,
    from: String,
    to: Option<String>,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    /// "running" | "done" | "failed"
    state: &'static str,
    error: Option<String>,
    verdict: Option<String>,
    #[serde(skip)]
    dir: std::path::PathBuf,
}

pub async fn serve(cfg: Config, host: String, port: u16, scalp_dir: std::path::PathBuf) -> Result<()> {
    let paths = Paths::new(&cfg.data_dir);
    let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).user_agent("jev-trader/0.1").build()?;
    let app = Arc::new(App {
        cfg,
        paths,
        scalp_dir,
        started_at: Utc::now(),
        scorecard: Mutex::new(None),
        scalp_scorecard: Mutex::new(None),
        http,
        bt_job: Mutex::new(None),
    });
    let router = Router::new()
        .route("/", get(|| async { asset("text/html; charset=utf-8", INDEX.as_bytes()) }))
        .route("/app.css", get(|| async { asset("text/css; charset=utf-8", CSS.as_bytes()) }))
        .route("/app.js", get(|| async { asset("text/javascript; charset=utf-8", JS.as_bytes()) }))
        .route("/fonts/{name}", get(font_handler))
        .route("/api/snapshot", get(snapshot_handler))
        .route("/api/scalp", get(scalp_handler))
        .route("/api/backtests", get(backtests_handler).post(backtest_start_handler))
        .route("/api/backtests/{id}", get(backtest_handler))
        .route("/api/control/pause", post(pause_handler))
        .route("/api/control/flatten", post(flatten_handler))
        .with_state(app);

    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    if !addr.ip().is_loopback() {
        tracing::warn!("dashboard bound to {addr}: the Pause/Flatten controls have no authentication");
    }
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("dashboard on http://{addr}");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await?;
    Ok(())
}

async fn font_handler(axum::extract::Path(name): axum::extract::Path<String>) -> Response {
    let stem = name.strip_suffix(".woff2").unwrap_or(&name);
    match FONTS.iter().find(|(n, _)| *n == stem) {
        Some((_, bytes)) => asset("font/woff2", bytes),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn asset(ct: &'static str, body: &'static [u8]) -> Response {
    ([(header::CONTENT_TYPE, ct), (header::CACHE_CONTROL, "no-cache")], body).into_response()
}

#[derive(Serialize)]
struct PositionView {
    asset: String,
    qty: f64,
    entry: f64,
    price: Option<f64>,
    value: f64,
    pnl_pct: Option<f64>,
    weight: f64,
}

#[derive(Serialize)]
struct AssetView {
    symbol: String,
    name: String,
    latest: Option<TickRecord>,
    /// Oldest first: (ts, price, news score, final weight)
    history: Vec<(DateTime<Utc>, f64, f64, f64)>,
    cooldown_until: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
struct Account {
    cash: f64,
    equity: f64,
    starting_cash: f64,
    day_start_equity: f64,
    halted_on: Option<chrono::NaiveDate>,
    positions: Vec<PositionView>,
    flatten_pending: bool,
}

#[derive(Serialize)]
struct Settings {
    interval_secs: u64,
    news_entry: f64,
    news_exit: f64,
    half_life_hours: f64,
    max_asset_weight: f64,
    max_gross: f64,
    stop_loss_pct: f64,
    daily_loss_limit_pct: f64,
    cooldown_hours: f64,
    feeds: Vec<String>,
    jev_model: String,
    retain_hours: f64,
    sizing: String,
    stop: String,
    max_heat: f64,
    weekly_loss_limit_pct: Option<f64>,
    monthly_loss_limit_pct: Option<f64>,
    losing_streak_n: Option<u32>,
    streak_cooldown_hours: f64,
    regime_mode: String,
    challenge: bool,
    dedupe_clusters: bool,
    min_clusters: usize,
    reaction_gate: bool,
    jev_max_attempts: u32,
    /// Paper round trip in bps: fee and slippage on the buy and on the sell.
    round_trip_cost_bps: f64,
}

#[derive(Serialize)]
struct DashboardProc {
    pid: u32,
    started_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct Snapshot {
    now: DateTime<Utc>,
    /// `data/SYNTHETIC` exists: the directory holds demo data, not real runs.
    synthetic: bool,
    bot_pid_running: Option<bool>,
    status: Option<Status>,
    dashboard: DashboardProc,
    control: Control,
    account: Option<Account>,
    assets: Vec<AssetView>,
    events: Vec<NewsEvent>,
    trades: Vec<Fill>,
    equity: Vec<EquityRecord>,
    pipeline: Vec<PipelineRecord>,
    settings: Settings,
    regime: Option<Regime>,
    scorecard: Option<Arc<Scorecard>>,
    /// Headlines given up on after the Jev retry budget (6.1).
    spiked: usize,
    /// The hypothesis register (`hypotheses.toml`), so the Evidence page can say what each test needs.
    hypotheses: Vec<crate::hypotheses::Hypothesis>,
}

fn file_key(p: &std::path::Path) -> (u64, Option<SystemTime>) {
    std::fs::metadata(p).map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None))
}

fn scorecard(app: &App) -> Option<Arc<Scorecard>> {
    let key = file_key(&app.paths.settlements);
    if key.0 == 0 {
        return None;
    }
    let mut cache = app.scorecard.lock().ok()?;
    if let Some((k, sc)) = cache.as_ref()
        && *k == key
    {
        return Some(sc.clone());
    }
    let settled: Vec<Settlement> = read_jsonl(&app.paths.settlements);
    let sc = Arc::new(report::scorecard(&settled, &app.cfg, "paper · settled"));
    *cache = Some((key, sc.clone()));
    Some(sc)
}

fn build_snapshot(app: &App) -> Snapshot {
    let cfg = &app.cfg;
    let p = &app.paths;
    let status: Option<Status> = read_json(&p.status);
    let control = Control::load(&p.control);
    let state: Option<State> = read_json(&p.state);

    let ticks: Vec<TickRecord> = tail_jsonl(&p.ticks, 4 << 20, 24 * 60 * cfg.assets.len().max(1));
    let mut by_asset: HashMap<&str, Vec<&TickRecord>> = HashMap::new();
    for t in &ticks {
        by_asset.entry(t.asset.as_str()).or_default().push(t);
    }
    let prices: HashMap<String, f64> = cfg
        .assets
        .iter()
        .filter_map(|a| by_asset.get(a.symbol.as_str()).and_then(|v| v.last()).map(|t| (a.symbol.clone(), t.price)))
        .collect();

    let assets = cfg
        .assets
        .iter()
        .map(|a| {
            let hist = by_asset.get(a.symbol.as_str()).cloned().unwrap_or_default();
            // Downsample to at most ~240 points.
            let step = (hist.len() / 240).max(1);
            AssetView {
                symbol: a.symbol.clone(),
                name: a.name.clone(),
                latest: hist.last().map(|t| (*t).clone()),
                history: hist
                    .iter()
                    .step_by(step)
                    .map(|t| (t.ts, t.price, t.news.score, t.final_weight))
                    .collect(),
                cooldown_until: state
                    .as_ref()
                    .and_then(|s| s.cooldown_until.get(&a.symbol).copied())
                    .filter(|t| *t > Utc::now()),
            }
        })
        .collect();

    let account = state.as_ref().map(|s| {
        let equity = s.equity(&prices);
        Account {
            cash: s.cash,
            equity,
            starting_cash: s.starting_cash.unwrap_or(cfg.paper.starting_cash),
            day_start_equity: s.day_start_equity,
            halted_on: s.halted_on,
            flatten_pending: control
                .flatten_requested_at
                .is_some_and(|r| s.flatten_handled.is_none_or(|d| r > d)),
            positions: s
                .positions
                .iter()
                .map(|(sym, pos)| {
                    let price = prices.get(sym).copied();
                    let value = pos.qty * price.unwrap_or(pos.entry);
                    PositionView {
                        asset: sym.clone(),
                        qty: pos.qty,
                        entry: pos.entry,
                        price,
                        value,
                        pnl_pct: price.map(|px| px / pos.entry - 1.0),
                        weight: if equity > 0.0 { value / equity } else { 0.0 },
                    }
                })
                .collect(),
        }
    });

    let mut events: Vec<NewsEvent> = tail_jsonl(&p.events, 2 << 20, 300);
    events.reverse();
    let mut trades: Vec<Fill> = tail_jsonl(&p.trades, 1 << 20, 200);
    trades.reverse();
    let equity_all: Vec<EquityRecord> = tail_jsonl(&p.equity, 2 << 20, 24 * 60);
    let cutoff = Utc::now() - chrono::Duration::hours(24);
    let equity: Vec<EquityRecord> = equity_all.into_iter().filter(|e| e.ts > cutoff).collect();
    let pipeline: Vec<PipelineRecord> = tail_jsonl(&p.pipeline, 1 << 20, 60);

    Snapshot {
        now: Utc::now(),
        synthetic: cfg.data_dir.join("SYNTHETIC").exists(),
        bot_pid_running: status.as_ref().and_then(|s| pid_running(s.pid)),
        status,
        dashboard: DashboardProc { pid: std::process::id(), started_at: app.started_at },
        control,
        account,
        assets,
        events,
        trades,
        equity,
        pipeline,
        settings: Settings {
            interval_secs: cfg.loop_interval_secs,
            news_entry: cfg.strategy.news_entry,
            news_exit: cfg.strategy.news_exit,
            half_life_hours: cfg.news.half_life_hours,
            max_asset_weight: cfg.risk.max_asset_weight,
            max_gross: cfg.risk.max_gross,
            stop_loss_pct: cfg.risk.stop_loss_pct,
            daily_loss_limit_pct: cfg.risk.daily_loss_limit_pct,
            cooldown_hours: cfg.risk.cooldown_hours,
            feeds: cfg.news.feeds.clone(),
            jev_model: cfg.jev.model.clone(),
            retain_hours: cfg.news.retain_hours,
            sizing: format!("{:?}", cfg.risk.sizing).to_lowercase(),
            stop: format!("{:?}", cfg.risk.stop).to_lowercase(),
            max_heat: cfg.risk.heat_cap(),
            weekly_loss_limit_pct: cfg.risk.weekly_loss_limit_pct,
            monthly_loss_limit_pct: cfg.risk.monthly_loss_limit_pct,
            losing_streak_n: cfg.risk.losing_streak_n,
            streak_cooldown_hours: cfg.risk.streak_cooldown_hours,
            regime_mode: format!("{:?}", cfg.regime.mode).to_lowercase(),
            challenge: cfg.challenge.enabled,
            dedupe_clusters: cfg.news.dedupe_clusters,
            min_clusters: cfg.strategy.min_clusters,
            reaction_gate: cfg.strategy.reaction_gate,
            jev_max_attempts: cfg.jev.max_attempts,
            round_trip_cost_bps: 2.0 * (cfg.paper.fee_rate + cfg.paper.slippage_rate) * 1e4,
        },
        regime: read_json(&p.regime),
        scorecard: scorecard(app),
        spiked: std::fs::read_to_string(&p.spiked).map(|s| s.lines().count()).unwrap_or(0),
        hypotheses: crate::hypotheses::load(std::path::Path::new("hypotheses.toml")).unwrap_or_default(),
    }
}

/// Linux: check /proc. Elsewhere: unknown.
fn pid_running(pid: u32) -> Option<bool> {
    if cfg!(target_os = "linux") {
        Some(std::path::Path::new(&format!("/proc/{pid}")).exists())
    } else {
        None
    }
}

/// ETag over the data files plus bot liveness, so polling clients get a cheap 304
/// when nothing changed. Polling (rather than a held-open stream) keeps any number
/// of open tabs under the browser's per-host connection limit.
fn etag(app: &App) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    fingerprint(&app.paths).hash(&mut h);
    let pid = read_json::<Status>(&app.paths.status).and_then(|s| pid_running(s.pid));
    pid.hash(&mut h);
    format!("\"{:016x}\"", h.finish())
}

async fn snapshot_handler(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    let tag = etag(&app);
    if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(tag.as_str()) {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, tag)]).into_response();
    }
    ([(header::ETAG, tag), (header::CACHE_CONTROL, "no-cache".to_string())], Json(build_snapshot(&app))).into_response()
}

/// Fingerprint of the data files.
fn fingerprint(p: &Paths) -> Vec<(u64, Option<SystemTime>)> {
    [&p.status, &p.control, &p.state, &p.events, &p.trades, &p.ticks, &p.pipeline, &p.equity, &p.settlements, &p.regime, &p.spiked]
        .iter()
        .map(|f| match std::fs::metadata(f) {
            Ok(m) => (m.len(), m.modified().ok()),
            Err(_) => (0, None),
        })
        .collect()
}

/// Scalper page: live status, recent candidates and round trips, and markouts net
/// of cost grouped by what happened to each candidate. Read-only.
/// `?lite=1` returns only the status and liveness (for the rail heartbeat on every page).
async fn scalp_handler(AxState(app): AxState<Arc<App>>, axum::extract::RawQuery(q): axum::extract::RawQuery) -> Json<serde_json::Value> {
    use serde_json::{Value, json};
    let d = &app.scalp_dir;
    let status: Option<Value> = read_json(&d.join("scalp_status.json"));
    if status.is_none() {
        return Json(json!({ "available": false, "dir": d.display().to_string() }));
    }
    let running = status.as_ref().and_then(|s| s["pid"].as_u64()).and_then(|p| pid_running(p as u32));
    if q.as_deref().is_some_and(|q| q.contains("lite=1")) {
        return Json(json!({ "available": true, "lite": true, "now": Utc::now(), "running": running, "status": status }));
    }
    let mut candidates: Vec<Value> = tail_jsonl(&d.join("scalp_candidates.jsonl"), 8 << 20, 20_000);
    // Candidates from before the activity baseline existed are not evidence.
    let warmup_ids: std::collections::HashSet<String> =
        candidates.iter().filter(|c| c["blocked_by"] == "warm-up").filter_map(|c| c["id"].as_str().map(String::from)).collect();
    candidates.retain(|c| c["blocked_by"] != "warm-up");
    let mut trades: Vec<Value> = tail_jsonl(&d.join("scalp_trades.jsonl"), 2 << 20, 2_000);
    let markouts: Vec<Value> = tail_jsonl(&d.join("scalp_markouts.jsonl"), 8 << 20, 60_000);

    // Which group each candidate falls in.
    let group: HashMap<&str, &str> = candidates
        .iter()
        .filter_map(|c| {
            let id = c["id"].as_str()?;
            let g = if c["taken"].as_bool() == Some(true) {
                "taken"
            } else {
                match c["blocked_by"].as_str() {
                    Some("jev review") => "jev_veto",
                    Some("cost") => "cost",
                    _ => "other",
                }
            };
            Some((id, g))
        })
        .collect();
    let mut acc: std::collections::BTreeMap<(i64, &str), (usize, f64, f64, usize)> = Default::default();
    for m in &markouts {
        let (Some(id), Some(h), Some(net), Some(ret)) = (m["id"].as_str(), m["h_secs"].as_i64(), m["net_bps"].as_f64(), m["ret_bps"].as_f64()) else { continue };
        if warmup_ids.contains(id) {
            continue;
        }
        // Older markouts have no side: they were all longs.
        let side = if m["side"] == "short" { "short" } else { "long" };
        for g in ["all", side, group.get(id).copied().unwrap_or("other")] {
            let e = acc.entry((h, g)).or_default();
            e.0 += 1;
            e.1 += net;
            e.2 += ret;
            e.3 += (net > 0.0) as usize;
        }
    }
    let markout_rows: Vec<Value> = acc
        .iter()
        .map(|((h, g), (n, net, ret, hits))| json!({ "h": h, "group": g, "n": n, "mean_net_bps": net / *n as f64, "mean_ret_bps": ret / *n as f64, "hit": *hits as f64 / *n as f64 }))
        .collect();
    let mut blocked: std::collections::BTreeMap<String, usize> = Default::default();
    for c in &candidates {
        *blocked.entry(c["blocked_by"].as_str().unwrap_or("taken").to_string()).or_default() += 1;
    }
    let (n_tr, wins, pnl, bps) = trades.iter().fold((0usize, 0usize, 0.0, 0.0), |(n, w, p, b), t| {
        let x = t["pnl_usd"].as_f64().unwrap_or(0.0);
        (n + 1, w + (x > 0.0) as usize, p + x, b + t["pnl_bps"].as_f64().unwrap_or(0.0))
    });
    // Per coin, over every round trip on file (before the list is cut for the page).
    let mut by_coin: std::collections::BTreeMap<String, serde_json::Map<String, Value>> = Default::default();
    for t in &trades {
        let Some(a) = t["asset"].as_str() else { continue };
        let e = by_coin.entry(a.to_string()).or_default();
        let add = |e: &mut serde_json::Map<String, Value>, k: &str, x: f64| {
            let v = e.get(k).and_then(Value::as_f64).unwrap_or(0.0) + x;
            e.insert(k.into(), json!(v));
        };
        let pnl = t["pnl_usd"].as_f64().unwrap_or(0.0);
        add(e, "n", 1.0);
        add(e, "wins", (pnl > 0.0) as u8 as f64);
        add(e, "pnl_usd", pnl);
        add(e, "fees_usd", t["fees_usd"].as_f64().unwrap_or(0.0));
        add(e, "bps_sum", t["pnl_bps"].as_f64().unwrap_or(0.0));
        add(e, "hold_secs_sum", t["hold_secs"].as_f64().unwrap_or(0.0));
        add(e, "notional_sum", t["notional"].as_f64().unwrap_or(0.0));
        let side = if t["side"] == "short" { "short" } else { "long" };
        add(e, &format!("{side}_n"), 1.0);
        add(e, &format!("{side}_pnl"), pnl);
        e.insert("last_closed".into(), t["closed"].clone());
    }
    for c in &candidates {
        let Some(a) = c["asset"].as_str() else { continue };
        let e = by_coin.entry(a.to_string()).or_default();
        let n = e.get("chances").and_then(Value::as_f64).unwrap_or(0.0) + 1.0;
        e.insert("chances".into(), json!(n));
        if c["taken"].as_bool() == Some(true) {
            let n = e.get("taken").and_then(Value::as_f64).unwrap_or(0.0) + 1.0;
            e.insert("taken".into(), json!(n));
        }
    }
    let scorecard = scalp_scorecard(&app).await;
    let n_candidates = candidates.len();
    let session_start = candidates.first().map(|c| c["ts"].clone()).unwrap_or(Value::Null);
    // What the price did after each candidate (raw move and net of cost), keyed by id.
    let mut after: HashMap<String, serde_json::Map<String, Value>> = HashMap::new();
    for m in &markouts {
        let (Some(id), Some(h)) = (m["id"].as_str(), m["h_secs"].as_i64()) else { continue };
        after.entry(id.to_string()).or_default().insert(h.to_string(), json!({ "ret_bps": m["ret_bps"], "net_bps": m["net_bps"] }));
    }
    let by_id: HashMap<String, Value> = candidates
        .iter()
        .filter(|c| c["taken"].as_bool() == Some(true))
        .filter_map(|c| Some((c["id"].as_str()?.to_string(), c.clone())))
        .collect();
    candidates.reverse();
    candidates.truncate(150);
    for c in candidates.iter_mut() {
        if let Some(a) = c["id"].as_str().and_then(|id| after.get(id)) {
            c["after"] = Value::Object(a.clone());
        }
    }
    trades.reverse();
    trades.truncate(500);
    // Each round trip carries the decision that opened it, so the page can tell the whole story.
    for t in trades.iter_mut() {
        if let Some(c) = t["candidate_id"].as_str().and_then(|id| by_id.get(id)) {
            t["candidate"] = c.clone();
        }
    }
    Json(json!({
        "available": true,
        "now": Utc::now(),
        "running": running,
        "status": status,
        "candidates": candidates,
        "trades": trades,
        "scorecard": scorecard,
        "summary": {
            "session_start": session_start,
            "candidates": n_candidates,
            "blocked_by": blocked,
            "markouts": markout_rows,
            "trades": { "n": n_tr, "wins": wins, "pnl_usd": pnl, "mean_bps": if n_tr > 0 { bps / n_tr as f64 } else { 0.0 } },
            "by_coin": by_coin,
        },
    }))
}

async fn scalp_scorecard(app: &Arc<App>) -> Option<Arc<ScalpScorecard>> {
    if let Some((t, sc)) = app.scalp_scorecard.lock().ok()?.as_ref()
        && t.elapsed() < std::time::Duration::from_secs(60)
    {
        return Some(sc.clone());
    }
    let (dir, cfg) = (app.scalp_dir.clone(), app.cfg.scalp.clone());
    let sc = Arc::new(tokio::task::spawn_blocking(move || crate::scalp::report::scorecard(&crate::scalp::report::load(&dir), &cfg)).await.ok()?);
    *app.scalp_scorecard.lock().ok()? = Some((std::time::Instant::now(), sc.clone()));
    Some(sc)
}

/// Settings files in the working directory that parse as a config: each can be backtested.
/// Its cached Jev answers (`<data_dir>/events.jsonl`) are what the replay reads.
fn bt_sources() -> Vec<(String, Config)> {
    let mut v: Vec<(String, Config)> = std::fs::read_dir(".")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".toml") {
                return None;
            }
            Config::load(&e.path()).ok().map(|c| (name, c))
        })
        .collect();
    v.sort_by(|a, b| (a.0 != "config.toml", &a.0).cmp(&(b.0 != "config.toml", &b.0)));
    v
}

/// Every folder that can hold runs: the dashboard's own, then each settings file's.
fn bt_roots(app: &App, sources: &[(String, Config)]) -> Vec<std::path::PathBuf> {
    let mut roots = vec![app.paths.backtests.clone()];
    for (_, c) in sources {
        let r = Paths::new(&c.data_dir).backtests;
        if !roots.iter().any(|x| x == &r) {
            roots.push(r);
        }
    }
    roots
}

fn bt_safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Which step a running job is on, read from the files `backtest::run` writes in order.
fn bt_stage(dir: &std::path::Path) -> u8 {
    if let Some(p) = read_json::<serde_json::Value>(&dir.join("progress.json")) {
        return p["stage"].as_u64().unwrap_or(0) as u8;
    }
    if dir.join("summary.json").exists() {
        4
    } else if dir.join("equity_news_only.jsonl").exists() {
        3
    } else if dir.join("equity_momentum_only.jsonl").exists() {
        2
    } else if dir.join("equity_strategy.jsonl").exists() {
        1
    } else {
        0
    }
}

/// Backtest runs (newest first), the settings files that can be backtested with a
/// per-day count of their cached headlines, and the job started from the page.
async fn backtests_handler(AxState(app): AxState<Arc<App>>) -> Json<serde_json::Value> {
    use serde_json::json;
    let sources = bt_sources();
    let mut runs: Vec<(Option<SystemTime>, serde_json::Value)> = Vec::new();
    for root in bt_roots(&app, &sources) {
        for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let id = e.file_name().to_string_lossy().into_owned();
            if !e.path().join("equity_strategy.jsonl").exists() || runs.iter().any(|r| r.1["id"] == id.as_str()) {
                continue;
            }
            let summary: Option<serde_json::Value> = read_json(&e.path().join("summary.json"));
            let source: Option<serde_json::Value> = read_json(&e.path().join("source.json"));
            runs.push((e.metadata().and_then(|m| m.modified()).ok(), json!({ "id": id, "kind": summary.as_ref().and_then(|s| s["kind"].as_str()).unwrap_or("news"), "summary": summary, "config": source.and_then(|s| s["config"].as_str().map(String::from)) })));
        }
    }
    runs.sort_by(|a, b| b.0.cmp(&a.0));
    let sources_json: Vec<serde_json::Value> = sources
        .iter()
        .map(|(file, c)| {
            let events = crate::status::load_events(&Paths::new(&c.data_dir).events);
            let mut days: std::collections::BTreeMap<String, usize> = Default::default();
            for e in &events {
                *days.entry(e.seen_at().format("%Y-%m-%d").to_string()).or_default() += 1;
            }
            json!({
                "file": file,
                "data_dir": c.data_dir.display().to_string(),
                "assets": c.assets.iter().map(|a| a.symbol.clone()).collect::<Vec<_>>(),
                "events": events.len(),
                "days": days,
                "scalp": {
                    "interval": c.scalp.revert_interval, "z": c.scalp.revert_z, "bars": c.scalp.revert_bars,
                    "hold_secs": c.scalp.hold_secs, "stop_bps": c.scalp.revert_stop_bps, "position_pct": c.scalp.position_pct,
                    "shorts": c.scalp.shorts, "fee_bps": c.scalp.fee_bps, "slippage_bps": c.scalp.slippage_bps,
                    "earliest": crate::scalp::backtest::earliest_start(c, Utc::now()),
                },
            })
        })
        .collect();
    let job = app.bt_job.lock().ok().and_then(|j| j.clone()).map(|j| {
        let stage = if j.state == "running" { bt_stage(&j.dir) } else { 4 };
        let mut v = json!(j);
        v["stage"] = json!(stage);
        v
    });
    Json(json!({
        "synthetic": app.cfg.data_dir.join("SYNTHETIC").exists(),
        "runs": runs.into_iter().map(|r| r.1).collect::<Vec<_>>(),
        "sources": sources_json,
        "job": job,
        "now": Utc::now(),
    }))
}

#[derive(Deserialize)]
struct BtStartReq {
    /// "scalp" (default) or "news".
    strategy: Option<String>,
    config: String,
    /// YYYY-MM-DD, inclusive.
    from: String,
    /// YYYY-MM-DD, exclusive; omitted = up to the last closed hour.
    to: Option<String>,
    name: Option<String>,
}

/// Starts `jev-trader backtest` for one settings file and window, in the background.
/// It writes only under `<data_dir>/backtests/<name>/`; it never trades or calls Jev.
async fn backtest_start_handler(AxState(app): AxState<Arc<App>>, Json(req): Json<BtStartReq>) -> Response {
    let bad = |m: String| (StatusCode::BAD_REQUEST, m).into_response();
    let Some((file, cfg)) = bt_sources().into_iter().find(|(f, _)| *f == req.config) else {
        return bad(format!("{} isn't a settings file in the dashboard's folder", req.config));
    };
    let day = |s: &str| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d");
    let Ok(from) = day(&req.from) else { return bad(format!("bad start date '{}'", req.from)) };
    let to = match req.to.as_deref() {
        Some(s) => match day(s) {
            Ok(d) => Some(d),
            Err(_) => return bad(format!("bad end date '{s}'")),
        },
        None => None,
    };
    let kind: &'static str = match req.strategy.as_deref().unwrap_or("scalp") {
        "scalp" => "scalp",
        "news" => "news",
        other => return bad(format!("unknown strategy '{other}'")),
    };
    let today = Utc::now().date_naive();
    let earliest = if kind == "scalp" { crate::scalp::backtest::earliest_start(&cfg, Utc::now()).date_naive() + chrono::Duration::days(1) } else { today - chrono::Duration::days(205) };
    if from < earliest {
        return bad(format!("Hyperliquid's candles for this strategy reach back to {earliest}: start on or after that"));
    }
    // Same rule as the page: at least 48 closed hours.
    let end = to.map(|d| d.and_hms_opt(0, 0, 0).expect("midnight").and_utc()).unwrap_or_else(|| Utc::now() - chrono::Duration::hours(1));
    if end - from.and_hms_opt(0, 0, 0).expect("midnight").and_utc() < chrono::Duration::hours(48) {
        return bad("the window needs at least two days of closed hours".into());
    }
    let id = match req.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) if bt_safe_id(n) => n.to_string(),
        Some(_) => return bad("names use letters, digits, - and _ only".into()),
        None => format!("{}{}_{}", if kind == "scalp" { "scalp_" } else { "" }, from.format("%Y%m%d"), to.unwrap_or(today).format("%Y%m%d")),
    };
    let dir = Paths::new(&cfg.data_dir).backtests.join(&id);
    let job = {
        let Ok(mut slot) = app.bt_job.lock() else { return StatusCode::INTERNAL_SERVER_ERROR.into_response() };
        if slot.as_ref().is_some_and(|j| j.state == "running") {
            return (StatusCode::CONFLICT, "a backtest is already running".to_string()).into_response();
        }
        // Same name replaces that run: clear what the progress steps read.
        for f in ["equity_strategy.jsonl", "equity_momentum_only.jsonl", "equity_news_only.jsonl", "equity_hold.jsonl", "summary.json", "report.md", "progress.json"] {
            std::fs::remove_file(dir.join(f)).ok();
        }
        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join("source.json"), serde_json::json!({ "config": file, "kind": kind }).to_string())) {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("can't write {}: {e}", dir.display())).into_response();
        }
        let j = BtJob {
            id: id.clone(),
            kind,
            config: file,
            from: from.to_string(),
            to: to.map(|d| d.to_string()),
            started_at: Utc::now(),
            finished_at: None,
            state: "running",
            error: None,
            verdict: None,
            dir,
        };
        *slot = Some(j.clone());
        j
    };
    info!(run = %job.id, config = %job.config, "backtest started from dashboard");
    let (app2, http) = (app.clone(), app.http.clone());
    let job_out = job.clone();
    let (from_s, to_s, run_id) = (job.from.clone(), job.to.clone(), job.id.clone());
    tokio::spawn(async move {
        // The replay is CPU-bound: keep it off the request workers.
        let dir = job.dir.clone();
        let res = tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Handle::current();
            if kind == "scalp" {
                let at = |d: &str| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").map(|d| d.and_hms_opt(0, 0, 0).expect("midnight").and_utc());
                let (Ok(f), to) = (at(&from_s), to_s.as_deref().map(at).transpose().ok().flatten()) else { return Err(anyhow::anyhow!("bad dates")) };
                rt.block_on(crate::scalp::backtest::run(&http, &cfg, f, to, &run_id, &dir)).map(|o| o.verdict)
            } else {
                rt.block_on(crate::backtest::run(&http, &cfg, Some(&from_s), to_s.as_deref(), Some(run_id))).map(|b| b.verdict)
            }
        })
        .await;
        let mut slot = app2.bt_job.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(j) = slot.as_mut() {
            j.finished_at = Some(Utc::now());
            match res {
                Ok(Ok(v)) => {
                    j.state = "done";
                    j.verdict = Some(v.to_string());
                }
                Ok(Err(e)) => {
                    j.state = "failed";
                    j.error = Some(format!("{e:#}"));
                }
                Err(e) => {
                    j.state = "failed";
                    j.error = Some(format!("the backtest stopped unexpectedly: {e}"));
                }
            }
            info!(run = %j.id, state = j.state, "backtest finished");
        }
    });
    (StatusCode::ACCEPTED, Json(job_out)).into_response()
}

/// One run: hourly equity per simulated variant and the strategy's fills.
async fn backtest_handler(AxState(app): AxState<Arc<App>>, axum::extract::Path(id): axum::extract::Path<String>) -> Response {
    // The id names a folder: no separators, no dots.
    if !bt_safe_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(d) = bt_roots(&app, &bt_sources()).into_iter().map(|r| r.join(&id)).find(|d| d.join("equity_strategy.jsonl").exists()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut equity = serde_json::Map::new();
    for v in ["strategy", "momentum_only", "news_only", "hold"] {
        let e: Vec<(DateTime<Utc>, f64)> = read_jsonl(&d.join(format!("equity_{v}.jsonl")));
        if !e.is_empty() {
            equity.insert(v.into(), serde_json::json!(e));
        }
    }
    let summary: Option<serde_json::Value> = read_json(&d.join("summary.json"));
    let source: Option<serde_json::Value> = read_json(&d.join("source.json"));
    let trades: Vec<serde_json::Value> = read_jsonl(&d.join("trades_strategy.jsonl"));
    Json(serde_json::json!({ "id": id, "summary": summary, "source": source, "equity": equity, "trades": trades }))
    .into_response()
}

#[derive(Deserialize)]
struct PauseReq {
    paused: bool,
}

async fn pause_handler(
    AxState(app): AxState<Arc<App>>,
    Json(req): Json<PauseReq>,
) -> Result<Json<Control>, (StatusCode, String)> {
    let mut c = Control::load(&app.paths.control);
    c.paused = req.paused;
    c.save(&app.paths.control).map_err(internal)?;
    info!(paused = req.paused, "pause set from dashboard");
    Ok(Json(c))
}

async fn flatten_handler(AxState(app): AxState<Arc<App>>) -> Result<Json<Control>, (StatusCode, String)> {
    let mut c = Control::load(&app.paths.control);
    // Flattening also pauses, otherwise the next tick could buy straight back in.
    c.paused = true;
    c.flatten_requested_at = Some(Utc::now());
    c.save(&app.paths.control).map_err(internal)?;
    info!("flatten requested from dashboard");
    Ok(Json(c))
}

fn internal(e: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, format!("could not write control file: {e:#}"))
}
