//! The news bot loop: I/O (feeds, Jev, Hyperliquid candles and funding, regime) around the shared decision path
//! (`decide::decide` + `decide::execute`), then settlement. Per-stage telemetry for
//! the dashboard, and dashboard controls (pause, flatten).

use std::collections::{BTreeMap, HashMap};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::breaker;
use crate::cluster::{self, Match};
use crate::config::Config;
use crate::decide::{self, AssetInput, Inputs, Variant};
use crate::jev::JevClient;
use crate::market;
use crate::news::{self, NewsEvent, NewsItem};
use crate::paper::{Pending, State};
use crate::regime::{self, Regime};
use crate::settle;
use crate::status::{
    Control, EquityRecord, Paths, PipelineRecord, Stage, StageState, Status, TickRecord, read_jsonl, write_atomic,
};

pub struct Ctx {
    pub cfg: Config,
    pub http: reqwest::Client,
    pub jev: JevClient,
    pub paths: Paths,
    /// Some = real orders on Hyperliquid (config `mode = "live"` and `run --live`).
    pub live: Option<crate::live::Live>,
}

pub async fn run(ctx: Ctx, once: bool) -> Result<()> {
    crate::status::acquire_pid_lock(&ctx.cfg.data_dir.join("bot.pid"))?;
    let mut state = match &ctx.live {
        Some(l) => {
            let s = crate::live::open_ledger(l, &ctx.cfg, &ctx.paths.state).await?;
            if ctx.cfg.exchange.is_testnet() {
                info!(cash = s.cash, positions = s.positions.len(), "Hyperliquid TESTNET account loaded (fake money)");
            } else {
                warn!(cash = s.cash, positions = s.positions.len(), "LIVE Hyperliquid account loaded: orders are real");
            }
            s
        }
        None => {
            let s = State::load_or_new(&ctx.paths.state, ctx.cfg.paper.starting_cash)?;
            crate::live::refuse_live_ledger(&s)?;
            info!(cash = s.cash, positions = s.positions.len(), "paper account loaded (Hyperliquid prices and fees)");
            s
        }
    };

    let mut status = Status {
        pid: std::process::id(),
        started_at: Utc::now(),
        last_tick_at: None,
        next_tick_at: None,
        tick: 0,
        interval_secs: ctx.cfg.loop_interval_secs,
        paused: Control::load(&ctx.paths.control).paused,
        halted: false,
        stages: Vec::new(),
        posture: None,
        review: BTreeMap::new(),
        jev_window: (0, 0),
        jev_pending: 0,
        tokens_today: (0, 0, 0),
        live: ctx.live.is_some(),
        testnet: ctx.live.is_some() && ctx.cfg.exchange.is_testnet(),
    };
    write_status(&ctx.paths, &status);

    loop {
        status.tick += 1;
        let started = Utc::now();
        let stages = match tick(&ctx, &mut state, &mut status).await {
            Ok(stages) => stages,
            Err(e) => {
                error!("tick failed: {e:#}");
                vec![Stage {
                    name: "Tick".into(),
                    state: StageState::Error,
                    detail: format!("{e:#}"),
                    ms: (Utc::now() - started).num_milliseconds().max(0) as u64,
                }]
            }
        };
        state.save(&ctx.paths.state)?;
        append_jsonl(&ctx.paths.pipeline, &PipelineRecord { ts: started, tick: status.tick, stages: stages.clone() });
        status.stages = stages;
        status.last_tick_at = Some(started);
        if once {
            status.next_tick_at = None;
            write_status(&ctx.paths, &status);
            break;
        }
        let next = Utc::now() + ChronoDuration::seconds(ctx.cfg.loop_interval_secs as i64);
        status.next_tick_at = Some(next);
        write_status(&ctx.paths, &status);

        // Sleep until the next tick, but wake early for a flatten request or a pause change.
        let mut stop = false;
        while Utc::now() < next {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                _ = tokio::signal::ctrl_c() => { stop = true; break; }
            }
            let control = Control::load(&ctx.paths.control);
            if control.paused != status.paused {
                status.paused = control.paused;
                info!(paused = control.paused, "pause changed from dashboard");
                write_status(&ctx.paths, &status);
            }
            if flatten_pending(&control, &state) {
                info!("flatten requested from dashboard: running tick now");
                break;
            }
        }
        if stop {
            info!("shutting down");
            break;
        }
    }
    status.next_tick_at = None;
    write_status(&ctx.paths, &status);
    state.save(&ctx.paths.state)?;
    Ok(())
}

fn flatten_pending(control: &Control, state: &State) -> bool {
    match control.flatten_requested_at {
        Some(req) => state.flatten_handled.is_none_or(|done| req > done),
        None => false,
    }
}

fn stage(name: &str, state: StageState, detail: impl Into<String>, t0: Instant) -> Stage {
    Stage { name: name.into(), state, detail: detail.into(), ms: t0.elapsed().as_millis() as u64 }
}

/// A headline the bot gave up on (6.1).
#[derive(Serialize)]
struct Spiked<'a> {
    ts: DateTime<Utc>,
    id: &'a str,
    title: &'a str,
    reason: &'static str,
    attempts: u32,
    last_error: &'a str,
}

async fn tick(ctx: &Ctx, state: &mut State, status: &mut Status) -> Result<Vec<Stage>> {
    let cfg = &ctx.cfg;
    let now = Utc::now();
    let mut stages = Vec::new();
    let control = Control::load(&ctx.paths.control);
    status.paused = control.paused;

    // 1. Feeds.
    let t0 = Instant::now();
    let (items, failed) = fetch_all_news_counted(&ctx.http, cfg).await;
    let max_age = ChronoDuration::seconds((cfg.news.max_age_hours * 3600.0) as i64);
    let undated = items.iter().filter(|i| i.published.is_none()).count();
    let fresh: Vec<NewsItem> = items
        .iter()
        .filter(|i| {
            !state.is_seen(&i.id)
                && i.published.is_some_and(|p| now - p < max_age)
                && state.pending.get(&i.id).is_none_or(|p| p.next_try_at <= now)
        })
        .cloned()
        .collect();
    let n_feeds = cfg.news.feeds.len();
    let feed_state = match failed {
        0 => StageState::Ok,
        f if f == n_feeds => StageState::Error,
        _ => StageState::Degraded,
    };
    stages.push(stage(
        "Feeds",
        feed_state,
        format!(
            "{}/{} sources · {} items · {} new{}",
            n_feeds - failed,
            n_feeds,
            items.len(),
            fresh.len(),
            if undated > 0 { format!(" · {undated} undated (ignored)") } else { String::new() }
        ),
        t0,
    ));

    // 2. Jev, with a retry budget (6.1).
    let t0 = Instant::now();
    if !fresh.is_empty() {
        info!(count = fresh.len(), "analyzing new headlines with Jev");
    }
    let firsts: HashMap<String, DateTime<Utc>> =
        fresh.iter().map(|i| (i.id.clone(), state.pending.get(&i.id).map(|p| p.first_seen_at).unwrap_or(now))).collect();
    let results: Vec<(NewsItem, u64, Result<NewsEvent>)> = stream::iter(fresh)
        .map(|item| {
            let first = firsts[&item.id];
            async move {
                let t = Instant::now();
                let r = news::analyze(&ctx.jev, &item, &cfg.assets, first)
                    .await
                    .and_then(|ev| crate::validate::news_event(&ev).map(|_| ev));
                (item, t.elapsed().as_millis() as u64, r)
            }
        })
        .buffer_unordered(cfg.jev.concurrency.max(1))
        .collect()
        .await;
    let total = results.len();
    let mut latencies = Vec::new();
    let mut failures = 0u32;
    let mut spiked = 0;
    let mut last_err = String::new();
    let mut new_events = Vec::new();
    let (mut tin, mut tout) = (0u64, 0u64);
    for (item, ms, r) in results {
        match r {
            Ok(ev) => {
                latencies.push(ms);
                tin += ev.tokens_in.unwrap_or(0);
                tout += ev.tokens_out.unwrap_or(0);
                state.pending.remove(&item.id);
                new_events.push(ev);
            }
            Err(e) => {
                failures += 1;
                last_err = format!("{e:#}");
                warn!("Jev failed for '{}': {last_err}", item.title);
                let p = state.pending.entry(item.id.clone()).or_insert(Pending {
                    first_seen_at: firsts[&item.id],
                    attempts: 0,
                    next_try_at: now,
                    last_error: String::new(),
                });
                p.attempts += 1;
                p.last_error = truncate(&last_err, 200);
                p.next_try_at = now + ChronoDuration::minutes(1i64 << p.attempts.min(6));
                if p.attempts >= cfg.jev.max_attempts {
                    append_jsonl(
                        &ctx.paths.spiked,
                        &Spiked { ts: now, id: &item.id, title: &item.title, reason: "jev_failed", attempts: p.attempts, last_error: &p.last_error },
                    );
                    state.pending.remove(&item.id);
                    state.mark_seen(item.id.clone());
                    spiked += 1;
                }
            }
        }
    }
    // 5.1: assign same-story clusters (Jaccard, then Jev on borderline cases).
    let mut jev_cluster_calls = 0;
    for mut ev in new_events {
        let found = cluster::find(&ev.title, &ev.asset, now, &state.events);
        ev.cluster_id = match found {
            Match::Same(id) => Some(id),
            Match::Borderline(cands) if cfg.news.cluster_jev_check => {
                let c: Vec<(String, String)> = cands.iter().map(|e| (e.cluster().to_string(), e.title.clone())).collect();
                jev_cluster_calls += 1;
                match news::same_event(&ctx.jev, &ev.title, &c).await {
                    Ok(m) => m,
                    Err(e) => {
                        warn!("Jev cluster check failed: {e:#}");
                        None
                    }
                }
            }
            _ => None,
        };
        info!(
            asset = %ev.asset, event = %ev.event, sentiment = format!("{:+.2}", ev.sentiment),
            new_info = format!("{:.2}", ev.new_info), "{}", ev.title
        );
        append_jsonl(&ctx.paths.events, &ev);
        state.mark_seen(ev.id.clone());
        state.events.push(ev);
    }
    let ok = total as u32 - failures;
    state.record_jev(ok, failures, cfg.jev.breaker_ticks);
    state.add_tokens(now, ok as u64 + jev_cluster_calls, tin, tout);
    // 6.3: persist right after the Jev stage, so a crash never re-analyzes these.
    if let Err(e) = state.save(&ctx.paths.state) {
        warn!("could not save state after Jev: {e:#}");
    }
    latencies.sort_unstable();
    let p50 = latencies.get(latencies.len() / 2).copied();
    let (wf, wt) = state.jev_failures();
    let jev_degraded = (wt >= cfg.jev.breaker_min_calls && wf as f64 / wt as f64 > cfg.jev.breaker_fail_ratio)
        .then(|| format!("{wf}/{wt} calls failed over {} ticks", state.jev_window.len()));
    let tokens = if tin + tout > 0 { format!(" · {tin}/{tout} tok") } else { String::new() };
    stages.push(stage(
        "Jev",
        match (total, failures, &jev_degraded) {
            (_, _, Some(_)) => StageState::Error,
            (0, _, _) => StageState::Skipped,
            (_, 0, _) => StageState::Ok,
            (t, f, _) if f as usize == t => StageState::Error,
            _ => StageState::Degraded,
        },
        match (total, failures) {
            (0, _) => "no new headlines".to_string(),
            (_, 0) => format!("{total} analyzed · p50 {}ms{tokens}", p50.unwrap_or(0)),
            _ => format!(
                "{failures}/{total} failed{} · {}",
                if spiked > 0 { format!(", {spiked} spiked") } else { ", retrying with backoff".into() },
                truncate(&last_err, 80)
            ),
        } + &jev_degraded.as_ref().map(|d| format!(" · BREAKER OPEN: {d}")).unwrap_or_default(),
        t0,
    ));
    let retain = ChronoDuration::seconds((cfg.news.retain_hours * 3600.0) as i64);
    state.events.retain(|e| now - e.published < retain);

    // 3. Prices: closed candles only, validated, staleness checked (0.3, 0.5).
    let t0 = Instant::now();
    let limit = cfg.market.momentum_window.max(15) + 2;
    let ims = cfg.interval_ms();
    let mut inputs: Vec<AssetInput> = Vec::new();
    let mut price_errs = Vec::new();
    let mut live: HashMap<String, f64> = HashMap::new();
    for a in &cfg.assets {
        let mut inp = AssetInput { symbol: a.symbol.clone(), ..Default::default() };
        match market::klines(&ctx.http, cfg.api(), &a.coin, &cfg.market.interval, limit).await {
            Ok(raw) => match crate::validate::candles(&raw) {
                Ok(()) => {
                    let snap = market::split_closed(raw, now.timestamp_millis(), ims);
                    inp.momentum = crate::signals::momentum(&snap.closed, cfg.market.momentum_window);
                    inp.atr_pct = market::atr_pct(&snap.closed, 14);
                    let px = snap.live_price.filter(|p| p.is_finite() && *p > 0.0);
                    inp.mark = px;
                    inp.exec = px;
                    if let Some(p) = px {
                        live.insert(a.symbol.clone(), p);
                    }
                    if snap.stale {
                        inp.review = Some("stale candles".into());
                        price_errs.push(format!("{}: stale", a.coin));
                    }
                }
                Err(e) => {
                    inp.review = Some(format!("invalid candles: {e}"));
                    price_errs.push(format!("{}: invalid", a.coin));
                }
            },
            Err(e) => {
                warn!("price fetch failed for {}: {e:#}", a.coin);
                price_errs.push(a.coin.clone());
            }
        }
        inputs.push(inp);
    }
    // Perps funding on open longs, at Hyperliquid's current hourly rates.
    let mut funding_note = String::new();
    match crate::hyperliquid::perp_universe(&ctx.http, cfg.api()).await {
        Ok(u) => {
            let rates: HashMap<String, f64> =
                cfg.assets.iter().filter_map(|a| u.iter().find(|p| p.coin == a.coin).map(|p| (a.symbol.clone(), p.funding_1h))).collect();
            let amt = state.accrue_funding(&rates, &live, now);
            if !state.positions.is_empty() {
                funding_note = format!(" · funding {amt:+.4} (total {:+.2})", state.funding_usd);
            }
        }
        Err(e) => {
            warn!("funding rates unavailable: {e:#}");
            funding_note = " · funding unavailable".into();
        }
    }
    stages.push(stage(
        "Prices",
        if price_errs.is_empty() {
            StageState::Ok
        } else if live.is_empty() {
            StageState::Error
        } else {
            StageState::Degraded
        },
        if price_errs.is_empty() {
            format!("{} perps · {} closed candles{funding_note}", live.len(), cfg.market.interval)
        } else {
            format!("failed: {}{funding_note}", price_errs.join(", "))
        },
        t0,
    ));

    // 3b. Live: the exchange must still hold what the ledger holds.
    let mut account: Option<crate::hyperliquid::exchange::Account> = None;
    if let Some(l) = &ctx.live {
        let t0 = Instant::now();
        match crate::live::reconcile(l, state, cfg).await {
            Ok((acct, review)) => {
                for inp in inputs.iter_mut() {
                    if let Some(r) = review.get(&inp.symbol)
                        && inp.review.is_none()
                    {
                        inp.review = Some(r.clone());
                    }
                }
                stages.push(stage(
                    "Exchange",
                    if review.is_empty() { StageState::Ok } else { StageState::Degraded },
                    if review.is_empty() {
                        format!("Hyperliquid in sync · {:.2} USDC withdrawable · ledger cash {:.2}", acct.withdrawable, state.cash)
                    } else {
                        let mut v: Vec<_> = review.iter().map(|(k, r)| format!("{k}: {r}")).collect();
                        v.sort();
                        v.join(" · ")
                    },
                    t0,
                ));
                account = Some(acct);
            }
            Err(e) => {
                warn!("Hyperliquid account check failed: {e:#}");
                for inp in inputs.iter_mut() {
                    inp.review.get_or_insert_with(|| "Hyperliquid account unavailable".into());
                }
                stages.push(stage("Exchange", StageState::Error, format!("account check failed, no orders this tick: {e:#}"), t0));
            }
        }
    }

    // 4. Regime (record and display; a gate only if configured).
    let t0 = Instant::now();
    let regime: Option<Regime> = if cfg.regime.enabled {
        let r = regime::refresh(&ctx.http, cfg, &ctx.paths, now).await;
        let present = r.components.iter().filter(|c| c.score.is_some()).count();
        stages.push(stage(
            "Regime",
            if r.score.is_some() { StageState::Ok } else { StageState::Degraded },
            format!(
                "{} {} · {present}/{} components{}",
                r.zone.label(),
                r.score.map(|s| format!("{s:.0}")).unwrap_or_else(|| "—".into()),
                r.components.len(),
                if cfg.regime.mode == crate::config::RegimeMode::Off { " · record only" } else { "" }
            ),
            t0,
        ));
        Some(r)
    } else {
        None
    };
    let zone = regime.as_ref().map(|r| r.zone.label());

    // 5. Breaker + decision (+ challenge on entries about to happen).
    let t0 = Instant::now();
    let equity = state.equity_verified(&live);
    let brk = breaker::update(state, equity, now, &cfg.risk);
    status.halted = brk.flatten;
    if brk.blocks_entries() {
        warn!(reason = %brk.reason, "circuit breaker: {:?}", brk.state);
    }
    let flatten = flatten_pending(&control, state);
    let events = state.events.clone();
    let mut inp = Inputs {
        cfg,
        now,
        events: &events,
        assets: inputs,
        paused: control.paused,
        flatten,
        jev_degraded: jev_degraded.clone(),
        regime: regime.as_ref(),
        variant: Variant::Strategy,
        replay: false,
    };
    let mut d = decide::decide(state, &brk, &inp);
    let ask: Vec<usize> = d.assets.iter().enumerate().filter(|(_, a)| a.needs_challenge).map(|(i, _)| i).collect();
    if !ask.is_empty() {
        for i in ask {
            let a = &d.assets[i];
            let titles: Vec<String> =
                a.drivers.iter().filter_map(|id| events.iter().find(|e| &e.id == id)).map(|e| e.title.clone()).collect();
            let asset = cfg.assets.iter().find(|x| x.symbol == a.symbol).expect("configured asset");
            inp.assets[i].challenge = Some(news::challenge(&ctx.jev, asset, &titles).await.map_err(|e| format!("{e:#}")));
            state.add_tokens(now, 1, 0, 0);
        }
        d = decide::decide(state, &brk, &inp);
    }
    status.posture = Some(d.posture.clone());
    status.review = d.assets.iter().filter_map(|a| a.review.clone().map(|r| (a.symbol.clone(), r))).collect();
    if let Some(r) = &d.account_review {
        status.review.insert("account".into(), r.clone());
    }
    status.jev_window = (wf, wt);
    status.jev_pending = state.pending.len();
    status.tokens_today = (state.tokens.calls, state.tokens.input, state.tokens.output);

    let summary: Vec<String> = d
        .assets
        .iter()
        .map(|a| {
            if a.review.is_some() {
                format!("{} REVIEW", a.symbol)
            } else {
                format!("{} {:+.2}", a.symbol, a.news.score)
            }
        })
        .collect();
    let gross: f64 = d.assets.iter().map(|a| a.weight).sum();
    stages.push(stage(
        "Signals",
        if d.assets.iter().all(|a| a.review.is_some()) { StageState::Degraded } else { StageState::Ok },
        summary.join(" · "),
        t0,
    ));
    let day_pnl = equity.map(|e| e / state.day_start_equity - 1.0);
    stages.push(stage(
        "Risk",
        match d.posture.kind {
            crate::status::PostureKind::EntriesAllowed => StageState::Ok,
            _ if brk.state == breaker::BreakerState::Halted => StageState::Error,
            _ => StageState::Degraded,
        },
        format!(
            "{} · gross {:.0}%/{:.0}% · day {}/−{:.0}%",
            d.posture.line(),
            gross * 100.0,
            d.ceiling * 100.0,
            day_pnl.map(|p| format!("{:+.1}%", p * 100.0)).unwrap_or_else(|| "?".into()),
            cfg.risk.daily_loss_limit_pct * 100.0
        )
        .replace('-', "−"),
        Instant::now(),
    ));

    // 6. Execute.
    let t0 = Instant::now();
    let (fills, exec_errors) = match (&ctx.live, account.as_mut()) {
        (Some(l), Some(acct)) => crate::live::execute(l, state, &mut d, cfg, now, acct).await,
        // Live but the account check failed: fail closed, no orders.
        (Some(_), None) => (Vec::new(), vec!["Hyperliquid account unavailable".to_string()]),
        (None, _) => (decide::execute(state, &mut d, cfg, now), Vec::new()),
    };
    let venue = match &ctx.live {
        Some(_) if cfg.exchange.is_testnet() => "TESTNET",
        Some(_) => "LIVE",
        None => "PAPER",
    };
    for f in &fills {
        info!("{venue} {} {:.6} {} @ {:.2} (${:.2}) — {}", f.side, f.qty, f.asset, f.price, f.notional, f.reason);
        append_jsonl(&ctx.paths.trades, f);
    }
    // A flatten is one-shot (it also pauses). A failed sell shows in the Execution
    // stage; press Flatten again rather than retrying every second.
    if flatten {
        state.flatten_handled = control.flatten_requested_at;
        info!("flatten complete");
    }
    let fill_word = if ctx.live.is_some() { "Hyperliquid fills" } else { "paper fills" };
    stages.push(stage(
        "Execution",
        if exec_errors.is_empty() { StageState::Ok } else { StageState::Degraded },
        if !exec_errors.is_empty() {
            format!("{} fills · {}", fills.len(), exec_errors.join(" · "))
        } else { match (fills.len(), control.paused, flatten) {
            (n, _, true) => format!("flattened · {n} fills"),
            (0, true, _) => "paused · no new buys".to_string(),
            (n, true, _) => format!("paused · {n} exits"),
            (0, false, _) => "no orders".to_string(),
            (n, false, _) => format!("{n} {fill_word}"),
        } },
        t0,
    ));

    let macro_event = macro_window(&ctx.paths.macro_calendar, now);
    for a in &d.assets {
        let Some(price) = a.mark else { continue };
        append_jsonl(
            &ctx.paths.ticks,
            &TickRecord {
                ts: now,
                asset: a.symbol.clone(),
                price,
                momentum: a.momentum,
                news: a.news,
                target: a.target,
                final_weight: a.weight,
                halted: brk.flatten,
                reason: a.reason.clone(),
                drivers: a.drivers.clone(),
                review: a.review.clone(),
                gates: a.gates.clone(),
                binding: a.binding,
                atr_pct: a.atr_pct,
                regime_score: regime.as_ref().and_then(|r| r.score),
                regime_zone: zone.map(str::to_string),
                funding: regime.as_ref().and_then(|r| r.funding.get(&a.symbol).copied()),
                open_interest: regime.as_ref().and_then(|r| r.open_interest.get(&a.symbol).copied()),
                macro_event: macro_event.clone(),
            },
        );
    }

    let eq_after = state.equity(&live);
    append_jsonl(&ctx.paths.equity, &EquityRecord { ts: now, equity: eq_after, cash: state.cash });

    // 7. Settlement (never blocks trading).
    if cfg.settle.enabled {
        let t0 = Instant::now();
        let mut recs: Vec<settle::Record> = events
            .iter()
            .filter(|e| firsts.contains_key(&e.id))
            .filter_map(|e| settle::headline_record(e, cfg, zone))
            .collect();
        recs.extend(settle::decision_records(state, &d.assets, cfg, now, zone));
        let queued = recs.len();
        settle::enqueue(&ctx.paths, recs);
        let s = settle::run(&ctx.http, cfg, &ctx.paths, now, zone).await;
        if s.settled > 0 {
            let all: Vec<settle::Settlement> = read_jsonl(&ctx.paths.settlements);
            state.reaction = settle::reaction_drift(&all, 4, 10);
        }
        stages.push(stage(
            "Settle",
            if s.errors.is_empty() { StageState::Ok } else { StageState::Degraded },
            format!(
                "+{queued} queued · {} settled · {} pending{}{}",
                s.settled,
                s.pending,
                if s.failed > 0 { format!(" · {} failed", s.failed) } else { String::new() },
                if s.errors.is_empty() { String::new() } else { format!(" · {}", truncate(&s.errors.join("; "), 80)) }
            ),
            t0,
        ));
    }
    info!(equity = format!("{:.2}", eq_after), cash = format!("{:.2}", state.cash), posture = %d.posture.line(), "tick done");
    Ok(stages)
}

/// Phase 7: operator-maintained macro calendar (`data/macro_calendar.toml`):
/// `[[event]] name = "FOMC", at = "2026-12-09T19:00:00Z"`. Returns the event
/// within ±1h of `now`, if any. Recorded only.
#[derive(Deserialize)]
struct MacroCalendar {
    #[serde(default)]
    event: Vec<MacroEvent>,
}

#[derive(Deserialize)]
struct MacroEvent {
    name: String,
    at: DateTime<Utc>,
}

fn macro_window(path: &Path, now: DateTime<Utc>) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let cal: MacroCalendar = toml::from_str(&text).ok()?;
    cal.event.into_iter().find(|e| (e.at - now).num_minutes().abs() <= 60).map(|e| e.name)
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

fn write_status(paths: &Paths, status: &Status) {
    if let Err(e) =
        serde_json::to_vec_pretty(status).map_err(anyhow::Error::from).and_then(|b| write_atomic(&paths.status, &b))
    {
        warn!("could not write status: {e:#}");
    }
}

pub async fn fetch_all_news(http: &reqwest::Client, cfg: &Config) -> Vec<NewsItem> {
    fetch_all_news_counted(http, cfg).await.0
}

async fn fetch_all_news_counted(http: &reqwest::Client, cfg: &Config) -> (Vec<NewsItem>, usize) {
    let results = futures::future::join_all(cfg.news.feeds.iter().map(|u| news::fetch_feed(http, u))).await;
    let mut items = Vec::new();
    let mut failed = 0;
    for (url, r) in cfg.news.feeds.iter().zip(results) {
        match r {
            Ok(v) => items.extend(v),
            Err(e) => {
                failed += 1;
                warn!("feed {url} failed: {e:#}");
            }
        }
    }
    items.sort_by_key(|i| std::cmp::Reverse(i.published));
    let mut seen = std::collections::HashSet::new();
    items.retain(|i| seen.insert(i.id.clone()));
    (items, failed)
}

pub fn append_jsonl<T: Serialize>(path: &Path, value: &T) {
    let res = (|| -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(f, "{}", serde_json::to_string(value)?)?;
        Ok(())
    })();
    if let Err(e) = res {
        warn!("could not write {}: {e:#}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_calendar_window() {
        let dir = std::env::temp_dir().join(format!("jev-macro-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("m.toml");
        std::fs::write(&p, "[[event]]\nname = \"CPI\"\nat = \"2026-11-12T13:30:00Z\"\n").unwrap();
        let t = |s: &str| s.parse::<DateTime<Utc>>().unwrap();
        assert_eq!(macro_window(&p, t("2026-11-12T13:00:00Z")).as_deref(), Some("CPI"));
        assert!(macro_window(&p, t("2026-11-12T16:00:00Z")).is_none());
        std::fs::remove_dir_all(dir).ok();
    }
}
