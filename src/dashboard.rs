//! `jev-trader dashboard`: a local page for watching the engine. Reads the files
//! the engine writes; writes only `control.json` (pause entries, flatten all).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::config::Config;
use crate::report::{self, Scorecard};
use crate::status::{Control, Files, read_json, tail_jsonl};

struct App {
    cfg: Config,
    files: Files,
    score: Mutex<Option<(Instant, Arc<Scorecard>)>>,
}

pub async fn serve(cfg: Config, host: String, port: u16) -> Result<()> {
    let files = Files::new(&cfg.data_dir);
    let app = Arc::new(App { cfg, files, score: Mutex::new(None) });
    let router = Router::new()
        .route("/", get(|| async { asset("text/html; charset=utf-8", include_bytes!("../dashboard/index.html")) }))
        .route("/app.js", get(|| async { asset("text/javascript", include_bytes!("../dashboard/app.js")) }))
        .route("/app.css", get(|| async { asset("text/css", include_bytes!("../dashboard/app.css")) }))
        .route("/fonts/{name}", get(font))
        .route("/api/state", get(state))
        .route("/api/pause", post(pause))
        .route("/api/flatten", post(flatten))
        .with_state(app);
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("dashboard on http://{addr}");
    axum::serve(listener, router).await?;
    Ok(())
}

fn asset(ct: &'static str, body: &'static [u8]) -> Response {
    ([(header::CONTENT_TYPE, ct), (header::CACHE_CONTROL, "no-cache")], body).into_response()
}

async fn font(axum::extract::Path(name): axum::extract::Path<String>) -> Response {
    let body: &'static [u8] = match name.as_str() {
        "barlow-400.woff2" => include_bytes!("../dashboard/fonts/barlow-400.woff2"),
        "barlow-500.woff2" => include_bytes!("../dashboard/fonts/barlow-500.woff2"),
        "barlow-600.woff2" => include_bytes!("../dashboard/fonts/barlow-600.woff2"),
        "barlow-condensed-600.woff2" => include_bytes!("../dashboard/fonts/barlow-condensed-600.woff2"),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    ([(header::CONTENT_TYPE, "font/woff2"), (header::CACHE_CONTROL, "max-age=86400")], body).into_response()
}

fn pid_alive(dir: &std::path::Path) -> bool {
    std::fs::read_to_string(dir.join("engine.pid"))
        .ok()
        .and_then(|t| t.trim().parse::<u32>().ok())
        .is_some_and(|pid| std::fs::read_to_string(format!("/proc/{pid}/cmdline")).is_ok_and(|c| c.contains("jev-trader")))
}

async fn state(State(app): State<Arc<App>>) -> Json<Value> {
    let status: Option<Value> = read_json(&app.files.status);
    let trades: Vec<Value> = tail_jsonl(&app.files.trades, 256 * 1024, 60);
    let candidates: Vec<Value> = tail_jsonl::<Value>(&app.files.candidates, 2 << 20, 400).into_iter().filter(|c| c["kind"] == "catalyst" || c["taken"] == true).rev().take(40).collect();
    let score = {
        let cached = app.score.lock().unwrap().clone();
        match cached {
            Some((t, s)) if t.elapsed().as_secs() < 60 => s,
            _ => {
                let dir = app.cfg.data_dir.clone();
                let min = app.cfg.catalyst.min_impact;
                let s = Arc::new(tokio::task::spawn_blocking(move || report::scorecard(&dir, min)).await.unwrap_or_default());
                *app.score.lock().unwrap() = Some((Instant::now(), s.clone()));
                s
            }
        }
    };
    Json(json!({
        "alive": pid_alive(&app.files.dir),
        "status": status,
        "trades": trades,
        "candidates": candidates,
        "score": &*score,
        "control": Control::load(&app.files.control),
        "data_dir": app.cfg.data_dir,
    }))
}

async fn pause(State(app): State<Arc<App>>, Json(body): Json<Value>) -> Result<Json<Control>, (StatusCode, String)> {
    let mut c = Control::load(&app.files.control);
    c.paused = body["paused"].as_bool().unwrap_or(true);
    c.save(&app.files.control).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    Ok(Json(c))
}

/// Closes every position and pauses new entries.
async fn flatten(State(app): State<Arc<App>>) -> Result<Json<Control>, (StatusCode, String)> {
    let mut c = Control::load(&app.files.control);
    c.paused = true;
    c.flatten_requested_at = Some(chrono::Utc::now());
    c.save(&app.files.control).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    Ok(Json(c))
}
