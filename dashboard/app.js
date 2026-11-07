"use strict";
// Jev trader dashboard: one departure board for both bots.
// Reads /api/snapshot (news bot) and /api/scalp (scalper). Writes only the news bot's pause/flatten controls.

const S = { snap: null, scalp: null, etag: null, view: "board", open: new Set(), tab: {}, filter: "all", confirmFlatten: false, err: {} };
const REDUCED = matchMedia("(prefers-reduced-motion: reduce)").matches;
const $ = (s, el = document) => el.querySelector(s);

// ---------- formatting ----------
const MINUS = "−";
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const num = (x, d = 2) => (x == null || !isFinite(x) ? "—" : Math.abs(x).toLocaleString("en-US", { minimumFractionDigits: d, maximumFractionDigits: d }));
const usd = (x, d = 2) => (x == null || !isFinite(x) ? "—" : (x < 0 ? MINUS : "") + "$" + num(x, d));
const susd = (x, d = 2) => (x == null || !isFinite(x) ? "—" : (x > 0 ? "+" : x < 0 ? MINUS : "") + "$" + num(x, d));
const pct = (x, d = 1) => (x == null || !isFinite(x) ? "—" : (x < 0 ? MINUS : "") + num(x * 100, d) + "%");
const spct = (x, d = 1) => (x == null || !isFinite(x) ? "—" : (x > 0 ? "+" : x < 0 ? MINUS : "") + num(x * 100, d) + "%");
const sgn = (x, d = 2) => (x == null || !isFinite(x) ? "—" : (x > 0 ? "+" : x < 0 ? MINUS : "") + num(x, d));
const cls = (x) => (x > 0.004 ? "pos" : x < -0.004 ? "neg" : "flat");
const px = (x) => (x == null ? "—" : x >= 1000 ? num(x, 1) : x >= 1 ? num(x, x >= 100 ? 2 : 4) : num(x, 6));
const t = (iso) => (iso ? new Date(iso) : null);
const hm24 = (d) => (d ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false }) : "");
const hm = (d) => (d ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false }) : "—");
const hms = (d) => (d ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false }) : "—");
const day = (d) => (d ? d.toLocaleDateString([], { month: "short", day: "numeric" }) : "");
function ago(d) {
  if (!d) return "never";
  const s = Math.max(0, (Date.now() - d.getTime()) / 1000);
  if (s < 5) return "just now";
  if (s < 90) return Math.round(s) + " s ago";
  if (s < 5400) return Math.round(s / 60) + " min ago";
  if (s < 172800) return Math.round(s / 3600) + " h ago";
  return Math.round(s / 86400) + " days ago";
}
function dur(secs) {
  if (secs == null) return "—";
  secs = Math.max(0, Math.round(secs));
  if (secs < 60) return secs + " s";
  if (secs < 3600) return Math.floor(secs / 60) + " min " + String(secs % 60).padStart(2, "0") + " s";
  return Math.floor(secs / 3600) + " h " + Math.floor((secs % 3600) / 60) + " min";
}
// bps per trade → dollars on a $1,000 trade (the scalper's usual size).
const per1k = (bps) => (bps == null ? null : bps / 10);

// ---------- icons (one stroke family) ----------
const ICON = {
  check: '<path d="M4 12.5l5 5L20 6.5"/>',
  x: '<path d="M6 6l12 12M18 6L6 18"/>',
  dash: '<path d="M6 12h12"/>',
  pause: '<path d="M9 5v14M15 5v14"/>',
  play: '<path d="M7 5l12 7-12 7z"/>',
  flat: '<path d="M4 12h16M12 3v5M9 5.5L12 8l3-2.5M12 21v-5M9 18.5l3-2.5 3 2.5"/>',
  chev: '<path d="M9 6l6 6-6 6"/>',
  warn: '<path d="M12 4l9 16H3z"/><path d="M12 10v4M12 17.5v.01"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.5v.01"/>',
};
const icon = (n) => `<svg class="i" viewBox="0 0 24 24" aria-hidden="true">${ICON[n]}</svg>`;

// ---------- the scalper's checks, in the order it runs them ----------
function scalpSettings() { return S.scalp?.status?.settings || {}; }
const MARKET_GATES = ["warm-up", "feed", "spread", "burst", "flow", "book", "push", "no chase", "cost", "news"];
const RISK_GATES = ["flat", "max positions", "cooldown", "trades/hour", "daily limit", "exchange", "cash", "jev review"];
function gateInfo(g) {
  const s = scalpSettings();
  const m = {
    "warm-up": ["Warm-up", `Needs ${Math.round((s.warmup_secs || 600) / 60)} minutes of trading history on the coin first.`, "WARMING UP"],
    feed: ["Fresh prices", `The live order book must be under ${((s.feed_stale_ms || 3000) / 1000).toFixed(0)} s old.`, "FRESH PRICES"],
    spread: ["A tight spread", `The gap between the best buy and sell price must be at most ${s.max_spread_bps ?? 5} bps (${((s.max_spread_bps ?? 5) / 100).toFixed(2)}%).`, "A TIGHT SPREAD"],
    burst: ["A rush of trading", `Trading must be at least ${s.min_activity_ratio ?? 1.5}× busier than its usual pace.`, "A RUSH OF TRADING"],
    flow: ["One-sided flow", `Over the last ${s.flow_window_secs ?? 30} s, trades must lean clearly to buyers (for a long) or sellers (for a short).`, "ONE-SIDED FLOW"],
    book: ["Book support", "Resting orders waiting in the book must lean the same way as the trades.", "BOOK SUPPORT"],
    push: ["Price moving", `Price must have moved at least ${s.min_push_bps ?? 3} bps that way in the last minute.`, "PRICE MOVING"],
    "no chase": ["A calmer price", `It won't jump in after a move of more than ${s.max_chase_bps ?? 60} bps in 5 minutes.`, "A CALMER PRICE"],
    cost: ["A big enough move", `The expected move must be at least ${s.cost_mult ?? 2}× the round-trip cost: fees, spread and funding.`, "A BIG ENOUGH MOVE"],
    news: ["Calmer news", `News tone moves the bar by up to ${Math.round((s.news_tilt ?? 0.1) * 100)}%. A hack alert blocks longs.`, "CALMER NEWS"],
    flat: ["Its open trade to end", "It holds at most one trade per coin.", "ITS TRADE TO END"],
    "max positions": ["A free slot", `At most ${s.max_positions ?? 2} trades open at once.`, "A FREE SLOT"],
    cooldown: ["Cooldown to end", `After a loss it leaves that coin alone for ${Math.round((s.cooldown_after_loss_secs ?? 900) / 60)} minutes.`, "COOLDOWN TO END"],
    "trades/hour": ["Room in the hourly limit", `At most ${s.max_trades_per_hour ?? 6} trades in any hour.`, "ROOM THIS HOUR"],
    "daily limit": ["Room in the daily loss limit", `Stops opening trades for the day after losing ${pct(s.daily_loss_limit_pct ?? 0.02, 0)} of capital.`, "TOMORROW"],
    exchange: ["Exchange matching the books", "Live only: the exchange's positions must match the ledger.", "BOOKS TO MATCH"],
    cash: ["Enough cash", `Needs at least $${s.min_trade_usd ?? 10} free.`, "MORE CASH"],
    "jev review": ["Jev's OK on the news", `Jev reads the last ${Math.round((s.jev_headline_minutes ?? 180) / 60)} h of headlines about the coin. Adverse news at ${s.jev_veto ?? 0.6} or more vetoes; news against the move halves the size.`, "JEV'S OK"],
  };
  return m[g] || [g, "", String(g || "").toUpperCase()];
}
const gateWait = (g) => gateInfo(g)[2];
// What it means when a check stops a trade.
const STOPPED = { "warm-up": "Still warming up", feed: "Prices were stale", spread: "Spread too wide", burst: "No rush of trading", flow: "Flow not one-sided", book: "Order book disagreed", push: "Price wasn't moving", "no chase": "Too late, already ran", cost: "Too small for fees", news: "News argued against it", flat: "Already in this coin", "max positions": "No free slot", cooldown: "Cooling down", "trades/hour": "Hourly limit full", "daily limit": "Daily loss limit hit", exchange: "Exchange mismatch", cash: "Not enough cash", "jev review": "Jev vetoed" };
const stoppedBy = (g) => STOPPED[g] || g;

// ---------- data ----------
async function getSnap() {
  try {
    const r = await fetch("/api/snapshot", { headers: S.etag ? { "If-None-Match": S.etag } : {} });
    if (r.status === 304) { S.err.snap = null; return false; }
    if (!r.ok) throw new Error("HTTP " + r.status);
    S.etag = r.headers.get("ETag");
    S.snap = await r.json();
    S.err.snap = null;
    return true;
  } catch (e) { S.err.snap = String(e.message || e); return true; }
}
async function getScalp() {
  try {
    const r = await fetch("/api/scalp");
    if (!r.ok) throw new Error("HTTP " + r.status);
    S.scalp = await r.json();
    S.err.scalp = null;
  } catch (e) { S.err.scalp = String(e.message || e); }
  return true;
}

// ---------- who's alive, whose money ----------
function scalpLife() {
  const sc = S.scalp;
  if (S.err.scalp) return { k: "down", line: "Dashboard can't read it" };
  if (!sc) return { k: "", line: "Connecting…" };
  if (!sc.available) return { k: "down", line: "Not set up · no scalp_status.json" };
  const ts = t(sc.status?.ts);
  const age = ts ? (Date.now() - ts) / 1000 : 1e9;
  if (sc.running === false) return { k: "down", line: "Stopped · last seen " + ago(ts) };
  if (age > 15) return { k: "stale", line: "No update for " + dur(age) };
  const n = sc.status?.assets?.length || 0;
  const feed = sc.status?.feed;
  if (feed && !feed.connected) return { k: "stale", line: "Feed reconnecting" };
  return { k: "ok", line: `Watching ${n} coins · ${ago(ts)}` };
}
function newsLife() {
  const sn = S.snap;
  if (S.err.snap) return { k: "down", line: "Dashboard can't read it" };
  if (!sn) return { k: "", line: "Connecting…" };
  const st = sn.status;
  if (!st) return { k: "down", line: "Never started here" };
  if (sn.bot_pid_running === false) return { k: "down", line: "Stopped · last tick " + ago(t(st.last_tick_at)) };
  const next = t(st.next_tick_at);
  const late = next ? (Date.now() - next) / 1000 : 0;
  if (late > (st.interval_secs || 60) * 2) return { k: "stale", line: "Tick overdue by " + dur(late) };
  if (sn.control?.paused) return { k: "stale", line: "Paused from this page" };
  const left = next ? Math.max(0, (next - Date.now()) / 1000) : null;
  return { k: "ok", line: left != null ? `Next look in ${Math.round(left)} s` : "Running" };
}
function scalpMoney() {
  const st = S.scalp?.status;
  if (!st) return null;
  const live = !!st.live, testnet = st.network === "testnet";
  const kind = !live ? "paper" : testnet ? "testnet" : "live";
  const capital = st.capital ?? st.starting_cash;
  const say = kind === "paper" ? `${usd(capital, 0)} simulated · real Hyperliquid prices, fees and funding. No real money.`
    : kind === "testnet" ? `Real orders on Hyperliquid testnet, with fake money. ${usd(capital, 0)} managed.`
    : `Real money on Hyperliquid mainnet, capped at ${usd(capital, 0)}.` + (st.withdrawable != null ? ` ${usd(st.withdrawable, 0)} withdrawable on the account.` : "");
  return { kind, label: kind === "paper" ? "Paper" : kind === "testnet" ? "Testnet" : "Live", say, capital };
}
function newsMoney() {
  const sn = S.snap;
  if (!sn) return null;
  const st = sn.status || {};
  const kind = st.live ? (st.testnet ? "testnet" : "live") : "paper";
  const start = sn.account?.starting_cash;
  const say = kind === "paper" ? `${usd(start, 0)} simulated · Hyperliquid prices and fees. No real money.`
    : kind === "testnet" ? "Real orders on Hyperliquid testnet, with fake money."
    : "Real money on Hyperliquid mainnet.";
  return { kind, label: kind === "paper" ? "Paper" : kind === "testnet" ? "Testnet" : "Live", say, start };
}
function renderRail() {
  for (const [id, f] of [["beat-scalp", scalpLife], ["beat-news", newsLife]]) {
    const el = document.getElementById(id), l = f();
    el.className = "beat " + l.k;
    $("small", el).textContent = l.line;
  }
  const a = scalpMoney(), b = newsMoney();
  const row = (name, m) => m ? `<div class="w"><b>${name}<span class="venue ${m.kind}">${m.label}</span></b><small>${esc(m.say)}</small></div>` : `<div class="w"><b>${name}</b><small>No data yet.</small></div>`;
  const anyLive = a?.kind === "live" || b?.kind === "live";
  $("#whose").innerHTML = `<h2>Whose money</h2>${row("Scalper", a)}${row("News bot", b)}<div class="w"><small>${anyLive ? "Real money is armed. Stops run inside the bot: if the process dies, open positions wait until it restarts." : "Nothing is trading real money. Going live needs mode = \"live\" in the config and --live on the command."}</small></div>`;
  for (const a of document.querySelectorAll(".nav a")) {
    if (a.dataset.view === S.view) a.setAttribute("aria-current", "page"); else a.removeAttribute("aria-current");
  }
}

// ---------- notices ----------
function renderNotices() {
  const out = [];
  const sn = S.snap, st = S.scalp?.status;
  if (sn?.synthetic) out.push(["bad", "warn", "<b>Demo data.</b> The news bot's folder holds synthetic data: headlines are real, readings, prices and fills are invented."]);
  if (st?.live_hold) out.push(["bad", "warn", `<b>Scalper on hold:</b> ${esc(st.live_hold)}. It won't open trades until you check Hyperliquid and run <code>jev-trader hl clear-hold</code>.`]);
  if (st?.halted_today) out.push(["warn", "warn", "<b>Scalper hit its daily loss limit.</b> No new trades until tomorrow (UTC). Open trades still close normally."]);
  if (sn?.account?.halted_on) out.push(["warn", "warn", "<b>News bot hit its daily loss limit.</b> Exits only until tomorrow."]);
  if (sn?.account?.flatten_pending) out.push(["warn", "info", "<b>Flatten requested.</b> The news bot sells everything on its next look, then stays paused."]);
  if (S.err.snap) out.push(["bad", "warn", `<b>Can't read the news bot's files:</b> ${esc(S.err.snap)}. Showing the last data received.`]);
  if (S.err.scalp) out.push(["bad", "warn", `<b>Can't read the scalper's files:</b> ${esc(S.err.scalp)}.`]);
  $("#notices").innerHTML = out.map(([k, i, h]) => `<div class="notice ${k}">${icon(i)}<p>${h}</p></div>`).join("");
}

// ---------- flaps ----------
const cellHTML = (c) => { const v = c === " " ? "&nbsp;" : esc(c); return `<span class="c" aria-hidden="true"><span class="h t">${v}</span><span class="h b">${v}</span></span>`; };
function flapHTML(text, n, tone = "") {
  const s = String(text ?? "").toUpperCase().padEnd(n).slice(0, n);
  return `<span class="fl ${tone}" data-n="${n}" data-v="${esc(s)}" role="text" aria-label="${esc(String(text ?? "").trim())}">${[...s].map(cellHTML).join("")}</span>`;
}
const FLAPS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+-$.";
// One flap turn: the top leaf (old character) folds down over the hairline, then the bottom leaf (new character) lands.
function fold(cell, to) {
  const [top, bot] = cell.children;
  const from = top.textContent;
  top.textContent = to;
  const lt = document.createElement("span"); lt.className = "h t leaf lt"; lt.textContent = from;
  const lb = document.createElement("span"); lb.className = "h b leaf lb"; lb.textContent = to;
  lt.addEventListener("animationend", () => lt.remove(), { once: true });
  lb.addEventListener("animationend", () => { bot.textContent = to; lb.remove(); }, { once: true });
  cell.append(lt, lb);
}
function setFlap(el, text, tone = "") {
  const n = +el.dataset.n;
  const s = String(text ?? "").toUpperCase().padEnd(n).slice(0, n);
  el.className = "fl " + tone;
  el.setAttribute("aria-label", String(text ?? "").trim());
  if (el.dataset.v === s) return;
  const old = el.dataset.v || "";
  el.dataset.v = s;
  const cells = el.children;
  [...s].forEach((ch, i) => {
    const cell = cells[i];
    if (!cell || old[i] === ch) return;
    const final = ch === " " ? "\u00a0" : ch;
    if (REDUCED) { cell.children[0].textContent = final; cell.children[1].textContent = final; return; }
    let k = 0;
    const steps = 1 + (i % 3);
    setTimeout(function step() {
      for (const l of cell.querySelectorAll(".leaf")) l.remove();
      fold(cell, k < steps ? FLAPS[(Math.random() * FLAPS.length) | 0] : final);
      if (k++ < steps) setTimeout(step, 120);
    }, i * 24);
  });
}

// ---------- board rows ----------
function scalpRows() {
  const st = S.scalp?.status;
  if (!st?.assets) return [];
  const trades = S.scalp.trades || [];
  return st.assets.map((a) => {
    const sym = a.symbol || a.coin;
    const last = trades.find((x) => x.asset === sym);
    const f = a.features || {};
    let side = a.position ? a.position.side : a.resting ? a.resting.side : a.leaning || "";
    let status = "WATCHING", tone = "", wait = "", wtone = "dim", res = "", rtone = "dim";
    if (a.position) {
      const p = a.position;
      const dir = p.side === "short" ? -1 : 1;
      const mid = f.mid;
      const u = mid ? dir * (mid - p.entry_px) * p.qty - (p.entry_fee || 0) + (p.funding_usd || 0) : null;
      status = "IN TRADE"; tone = "green"; wait = "TARGET OR STOP"; wtone = "";
      res = u == null ? "OPEN" : `${susd(u)} OPEN`; rtone = u == null ? "" : u >= 0 ? "green" : "red";
    } else if (a.resting) {
      status = "ORDER WAITING"; tone = "amber"; wait = "A SELLER TO FILL".replace("SELLER", a.resting.side === "short" ? "BUYER" : "SELLER"); wtone = "";
    } else if (a.exchange_review) {
      status = "REVIEW"; tone = "amber"; wait = "BOOKS TO MATCH"; wtone = "amber";
    } else if (st.live_hold) {
      status = "ON HOLD"; tone = "red"; wait = "YOU TO CHECK"; wtone = "red";
    } else if (a.blocked_by === "warm-up") {
      status = "WARMING UP"; tone = "amber"; wait = `${Math.max(0, Math.ceil(((st.settings?.warmup_secs || 600) - (f.history_secs || 0)) / 60))} MIN MORE`;
    } else {
      wait = a.blocked_by ? gateWait(a.blocked_by) : "READY";
      if (["cooldown", "trades/hour", "daily limit", "max positions"].includes(a.blocked_by)) { status = "HELD"; tone = "amber"; wtone = "amber"; }
    }
    if (!res && last) {
      res = `${susd(last.pnl_usd)} AT ${hm24(t(last.closed))}`;
      rtone = last.pnl_usd > 0 ? "green" : last.pnl_usd < 0 ? "red" : "dim";
    }
    if (!res) res = "NO TRADES YET";
    const s = st.settings || {};
    const sd = side === "short" ? -1 : 1;
    const cost = f.spread_bps == null ? null : (s.entry === "taker" ? 2 * s.fee_bps + 2 * s.slippage_bps + f.spread_bps : (s.maker_fee_bps ?? 1.5) + (s.fee_bps ?? 4.5) + (s.slippage_bps ?? 1) + 0.5 * f.spread_bps) + Math.max(0, sd * (f.funding_bps_1h || 0)) * (s.hold_secs || 600) / 3600;
    const need = cost == null ? null : cost * (s.cost_mult || 2);
    const costTxt = cost == null ? "—" : `${num(cost, 1)}/${num(f.exp_move_bps, 0)}`;
    const ctone = cost == null || f.exp_move_bps == null ? "dim" : f.exp_move_bps >= need ? "" : "dim";
    return { key: "s:" + sym, coin: sym, side: side ? side.toUpperCase() : "—", status, tone, wait, wtone, cost: costTxt, ctone, res, rtone, a };
  });
}
function newsRows() {
  const sn = S.snap;
  if (!sn?.assets) return [];
  const pos = new Map((sn.account?.positions || []).map((p) => [p.asset, p]));
  const set = sn.settings || {};
  const review = sn.status?.review || {};
  return sn.assets.map((a) => {
    const L = a.latest || {}, ns = L.news || {};
    const p = pos.get(a.symbol);
    let status = "WATCHING", tone = "", wait = "", wtone = "dim", res = "", rtone = "dim";
    const failed = (L.gates || []).find((g) => !g.passed);
    if (p) { status = "HOLDING"; tone = "green"; wait = `NEWS BELOW ${MINUS}${num(set.news_exit, 2)}`; res = `${spct(p.pnl_pct)} OPEN`; rtone = p.pnl_pct >= 0 ? "green" : "red"; }
    else if (review[a.symbol]) { status = "REVIEW"; tone = "amber"; wait = "TRUSTED DATA"; wtone = "amber"; }
    else if (sn.control?.paused) { status = "PAUSED"; tone = "amber"; wait = "YOU TO RESUME"; wtone = "amber"; }
    else if (a.cooldown_until) { status = "HELD"; tone = "amber"; wait = "COOLDOWN"; wtone = "amber"; }
    else if (failed) { status = "HELD"; tone = "amber"; wait = NEWS_WAIT[failed.gate] || failed.gate.toUpperCase(); wtone = "amber"; }
    else if ((ns.score ?? 0) <= set.news_entry) wait = `NEWS ABOVE +${num(set.news_entry, 2)}`;
    else if ((L.momentum ?? 0) <= 0) wait = "AN UPTREND";
    else wait = "NEXT LOOK";
    if (!res) {
      const last = (sn.trades || []).find((f) => f.asset === a.symbol && (f.round_trip_pnl != null || f.realized_pnl != null));
      if (last) { const v = last.round_trip_pnl ?? last.realized_pnl; res = `${susd(v)} ${day(t(last.ts))}`; rtone = v > 0 ? "green" : v < 0 ? "red" : "dim"; }
      else res = "NO TRADES YET";
    }
    const rc = set.round_trip_cost_bps;
    const costTxt = rc == null ? "—" : `${num(rc, 0)}/${L.atr_pct != null ? num(L.atr_pct * 1e4, 0) : "—"}`;
    return { key: "n:" + a.symbol, coin: a.symbol, side: "LONG", status, tone, wait, wtone, cost: costTxt, ctone: "dim", res, rtone, a };
  });
}
const COLS = [6, 5, 13, 18, 7, 15];
function rowHTML(r) {
  const open = S.open.has(r.key);
  return `<button class="brow" data-key="${r.key}" data-lamp="${r.tone}" aria-expanded="${open}"><span class="lamp" aria-hidden="true"></span>${flapHTML(r.coin, COLS[0])}${flapHTML(r.side, COLS[1], r.side === "LONG" || r.side === "SHORT" ? "" : "dim")}${flapHTML(r.status, COLS[2], r.tone)}${flapHTML(r.wait, COLS[3], r.wtone)}${flapHTML(r.cost, COLS[4], r.ctone)}${flapHTML(r.res, COLS[5], r.rtone)}</button><div class="trace" data-trace="${r.key}" ${open ? "" : "hidden"}></div>`;
}
function updateRow(r) {
  const b = document.querySelector(`.brow[data-key="${r.key}"]`);
  if (!b) return false;
  const f = b.querySelectorAll(":scope > .fl");
  setFlap(f[0], r.coin); setFlap(f[1], r.side, r.side === "LONG" || r.side === "SHORT" ? "" : "dim");
  setFlap(f[2], r.status, r.tone); setFlap(f[3], r.wait, r.wtone); setFlap(f[4], r.cost, r.ctone); setFlap(f[5], r.res, r.rtone);
  b.dataset.lamp = r.tone;
  const tr = document.querySelector(`[data-trace="${r.key}"]`);
  if (tr && !tr.hidden) tr.innerHTML = r.key[0] === "s" ? scalpCoinWhy(r.a) : newsCoinWhy(r.a);
  return true;
}

// ---------- per-coin "why" ----------
function scalpCoinWhy(a) {
  const st = S.scalp.status, f = a.features || {}, sym = a.symbol || a.coin, s = st.settings || {};
  const side = a.leaning || "long";
  const block = a.position || a.resting ? null : (side === "short" ? a.short_block : a.long_block) || a.blocked_by;
  const idx = block ? [...MARKET_GATES, ...RISK_GATES].indexOf(block) : -1;
  const list = [...MARKET_GATES, ...RISK_GATES].map((g, i) => {
    const [name, say] = gateInfo(g);
    const state = idx < 0 ? (a.position || a.resting ? "ok" : "skip") : i < idx ? "ok" : i === idx ? "no" : "skip";
    return `<li class="${state}"><span class="mk">${icon(state === "ok" ? "check" : state === "no" ? "x" : "dash")}</span><span><b>${esc(name)}</b><br><small class="low">${esc(say)}</small></span><span class="raw"></span></li>`;
  });
  let head, sub;
  if (a.position) {
    const p = a.position, dir = p.side === "short" ? "short" : "long";
    head = `${sym}: in a ${dir} trade since ${hm(new Date(p.opened_ms))}.`;
    sub = `Entered at ${px(p.entry_px)}. It sells at ${px(p.tp_px)} (target) or ${px(p.sl_px)} (stop, trails once in profit), or after ${Math.round((s.hold_secs || 600) / 60)} minutes at the latest.`;
  } else if (a.resting) {
    head = `${sym}: a ${a.resting.side} order is waiting at ${px(a.resting.px)}.`;
    sub = `It only counts as filled when someone trades through that price. Cancelled after ${s.maker_timeout_secs ?? 20} s if nobody does.`;
  } else if (block === "warm-up") {
    head = `${sym}: still learning what normal trading looks like.`;
    sub = `It has ${dur(f.history_secs)} of history and needs ${dur(s.warmup_secs)}.`;
  } else if (block) {
    const [name] = gateInfo(block);
    head = ["cooldown", "trades/hour", "daily limit", "max positions", "flat"].includes(block) ? `${sym}: held, waiting for ${name.toLowerCase()}.` : `${sym}: watching, waiting for ${name.toLowerCase()}.`;
    sub = `Leaning ${side}. Every check above it passed for a ${side}; the first one that didn't is marked.`;
  } else { head = `${sym}: every check passes.`; sub = "An entry is about to be placed or was just placed."; }
  const dir = side === "short" ? -1 : 1;
  const readings = [
    [`${num(f.activity, 2)}×`, `trading pace vs normal (needs ${s.min_activity_ratio ?? 1.5}×)`],
    [sgn(f.flow, 2), `trade flow, −1 all sellers … +1 all buyers`],
    [sgn(f.obi, 2), `order book lean, same scale`],
    [`${sgn(f.ret_60s_bps, 1)} bps`, "price move in the last minute"],
    [`${sgn(f.ret_300s_bps, 1)} bps`, "price move in 5 minutes"],
    [`${num(f.exp_move_bps, 1)} bps`, "typical move over a hold"],
    [`${num(f.spread_bps, 2)} bps`, "spread now"],
    [`${sgn(f.funding_bps_1h, 3)} bps/h`, dir > 0 ? "funding (longs pay when +)" : "funding (shorts pay when −)"],
    [sgn(a.news?.score, 2), `news tone, ${a.news?.n_events ?? 0} headlines${a.news?.hack_alert ? " · HACK ALERT" : ""}`],
  ];
  return `<div class="why"><h3>${esc(head)}</h3><p class="sub">${esc(sub)} 100 bps = 1%.</p>
  <div class="cols"><div><span class="lbl">Checks, in order</span><ul class="checks">${list.join("")}</ul></div>
  <div><span class="lbl">Readings now · mid ${px(f.mid)}</span><div class="readings">${readings.map(([v, l]) => `<div class="reading"><b>${esc(v)}</b><small>${esc(l)}</small></div>`).join("")}</div>
  <p class="sub" style="margin-top:14px"><a href="#scalper">Every decision and trade for ${esc(sym)} →</a></p></div></div></div>`;
}
function newsHeadlines(ids) {
  const ev = new Map((S.snap.events || []).map((e) => [e.id, e]));
  return (ids || []).map((id) => ev.get(id)).filter(Boolean);
}
function newsCoinWhy(a) {
  const sn = S.snap, set = sn.settings || {}, L = a.latest || {}, ns = L.news || {};
  const p = (sn.account?.positions || []).find((x) => x.asset === a.symbol);
  let head;
  if (!a.latest) head = `${a.symbol}: no reading yet.`;
  else if (p) head = `${a.symbol}: holding ${pct(p.weight, 0)} of the account since its last buy.`;
  else if ((ns.score ?? 0) <= set.news_entry) head = `${a.symbol}: news isn't positive enough to buy.`;
  else if ((L.momentum ?? 0) <= 0) head = `${a.symbol}: the news is good but price isn't rising yet.`;
  else head = `${a.symbol}: ready, ${L.reason || ""}.`;
  const gates = (L.gates || []).map((g) => `<li class="${g.passed ? "ok" : "no"}"><span class="mk">${icon(g.passed ? "check" : "x")}</span><span>${esc(newsGateName(g.gate))}</span><span class="raw">${esc(g.detail || "")}</span></li>`).join("");
  const heads = newsHeadlines(L.drivers).slice(0, 5).map((e) => `<li><a href="${esc(e.id)}" target="_blank" rel="noopener">${esc(e.title)}</a><small>${esc(e.source)} · ${ago(t(e.published))} · Jev: ${sentWord(e.sentiment)} (${sgn(e.sentiment, 2)}), new information ${pct(e.new_info, 0)}</small></li>`).join("");
  return `<div class="why"><h3>${esc(head)}</h3>
  <p class="sub">It buys when the news score is above +${num(set.news_entry, 2)} <i>and</i> the hourly price trend is up. It sells when the score falls below ${MINUS}${num(set.news_exit, 2)}, on a hack alert, or at the ${pct(set.stop_loss_pct, 0)} stop. Each headline's weight halves every ${set.half_life_hours} h.</p>
  <div class="cols"><div><span class="lbl">News score now</span>${gauge(ns.score, set.news_entry, set.news_exit)}
  <div class="readings"><div class="reading"><b>${sgn(ns.score, 2)}</b><small>score from ${ns.n_events ?? 0} headlines (${ns.pos ?? 0} positive, ${ns.neg ?? 0} negative, ${ns.neutral ?? 0} neutral)</small></div>
  <div class="reading"><b>${spct(L.momentum, 2)}</b><small>hourly trend vs its 20-hour average</small></div>
  <div class="reading"><b>${px(L.price)}</b><small>last price</small></div></div>
  <span class="lbl" style="display:block;margin-top:14px">Headlines driving it</span><ul class="heads">${heads || '<li class="low">None in the window.</li>'}</ul></div>
  <div><span class="lbl">Safety checks</span><ul class="checks">${gates || '<li class="skip"><span class="mk"></span><span>No checks recorded yet.</span><span></span></li>'}</ul></div></div></div>`;
}
function newsGateName(g) {
  return ({ flatten: "No flatten requested", "kill switch": "Daily loss limit not hit", "stop loss": "Stop loss not hit", "asset cooldown": "Coin not cooling down", inputs: "Prices and candles can be trusted", account: "Account value can be trusted", breaker: "No weekly/monthly halt or losing streak", "jev health": "Jev is answering", regime: "Market regime allows it", "news reaction": "Price reacted to the news", pause: "Not paused", challenge: "Survives the counter-argument" })[g] || g;
}
const NEWS_WAIT = { flatten: "FLATTEN TO FINISH", "kill switch": "TOMORROW", "stop loss": "STOP TO CLEAR", "asset cooldown": "COOLDOWN TO END", inputs: "TRUSTED PRICES", account: "TRUSTED ACCOUNT", breaker: "THE HALT TO END", "jev health": "JEV TO RECOVER", regime: "A BETTER MARKET", "news reaction": "PRICE TO REACT", pause: "YOU TO RESUME", challenge: "A STRONGER CASE" };
const sentWord = (x) => (x == null ? "unclear" : x > 0.25 ? "positive" : x < -0.25 ? "negative" : "neutral");
function gauge(score, entry, exit) {
  const X = (v) => ((Math.max(-1, Math.min(1, v)) + 1) / 2) * 100;
  return `<div class="gauge" role="img" aria-label="news score ${sgn(score, 2)}, buy above ${num(entry, 2)}, sell below ${MINUS}${num(exit, 2)}"><span class="track"></span><span class="zone sell" style="left:0;width:${X(-exit)}%"></span><span class="zone buy" style="left:${X(entry)}%;right:0"></span><span class="mid"></span><span class="needle" style="left:${X(score ?? 0)}%"></span></div>`;
}

// ---------- money sheets ----------
function meter(used, cap) {
  const r = cap > 0 ? Math.min(1, Math.max(0, used / cap)) : 0;
  return `<div class="meter ${r >= 1 ? "full" : r >= 0.75 ? "warn" : ""}"><i style="width:${(r * 100).toFixed(1)}%"></i></div>`;
}
const budget = (txt, used, cap) => `<div class="budget"><span>${txt}</span>${meter(used, cap)}</div>`;
function sheetFrame(name, m, big, sub, risk, budgets, link) {
  return `<section class="sheet bot"><div class="sh-top"><h2><a href="${link}">${name}</a>${m ? `<span class="venue ${m.kind}">${m.label}</span>` : ""}</h2><p class="num ${big[1]}">${big[0]}</p></div>
  <p class="sh-sub">${sub}</p><p class="atrisk"><span class="lbl">At risk</span> ${risk}</p><div class="budgets">${budgets}</div></section>`;
}
function scalpSheet() {
  const sc = S.scalp, st = sc?.status;
  if (!st) return `<section class="sheet bot"><h2>Scalper</h2><p class="sh-sub">${sc && !sc.available ? "No data in " + esc(sc.dir || "data-hl") + ". Start it with <code>jev-trader --config config.scalp.toml scalp</code>." : "Loading…"}</p></section>`;
  const s = st.settings || {}, m = scalpMoney();
  const open = st.assets.filter((a) => a.position);
  const risk = open.length ? open.map((a) => { const p = a.position; const loss = Math.abs(p.entry_px - p.sl_px) * p.qty + (p.entry_fee || 0) * 2; return `<b>${esc(a.symbol)} ${p.side}</b>, about ${usd(loss)} if its stop hits`; }).join("; ") + "." : `nothing open, ${usd(st.cash, 0)} in cash.`;
  const hourN = (sc.trades || []).filter((x) => Date.now() - t(x.opened) < 3600e3).length;
  const lossCap = (s.daily_loss_limit_pct || 0.02) * (st.capital || st.starting_cash || 0);
  const dayLoss = Math.max(0, -(st.day_trade_pnl || 0));
  return sheetFrame("Scalper", m, [susd(st.realized), cls(st.realized)], `after fees · ${st.wins ?? 0} won, ${st.losses ?? 0} lost`, risk,
    budget(`<b>${hourN}</b>/${s.max_trades_per_hour ?? 6} trades this hour`, hourN, s.max_trades_per_hour ?? 6) + budget(`<b>${usd(dayLoss)}</b>/${usd(lossCap, 0)} day loss`, dayLoss, lossCap) + budget(`<b>${open.length}</b>/${s.max_positions ?? 2} open`, open.length, s.max_positions ?? 2), "#scalper");
}
function newsSheet() {
  const sn = S.snap;
  if (!sn) return `<section class="sheet bot"><h2>News bot</h2><p class="sh-sub">Loading…</p></section>`;
  const acc = sn.account, set = sn.settings || {}, m = newsMoney();
  if (!acc) return `<section class="sheet bot"><h2>News bot</h2><p class="sh-sub">No account yet. Start it with <code>jev-trader run</code>.</p></section>`;
  const pnl = acc.equity - acc.starting_cash;
  const gross = acc.positions.reduce((x, p) => x + p.weight, 0);
  const dayLoss = Math.max(0, acc.day_start_equity - acc.equity), lossCap = set.daily_loss_limit_pct * acc.day_start_equity;
  const risk = acc.positions.length ? acc.positions.map((p) => `<b>${esc(p.asset)}</b> ${usd(p.value, 0)}, about ${usd(p.value * set.stop_loss_pct)} at the ${pct(set.stop_loss_pct, 0)} stop`).join("; ") + "." : `nothing open, ${usd(acc.cash, 0)} in cash.`;
  return sheetFrame("News bot", m, [susd(pnl), cls(pnl)], `since start, after fees · ${(sn.trades || []).length} fills`, risk,
    budget(`<b>${pct(gross, 0)}</b>/${pct(set.max_gross, 0)} invested`, gross, set.max_gross) + budget(`<b>${usd(dayLoss)}</b>/${usd(lossCap, 0)} day loss`, dayLoss, lossCap) + budget(`<b>${acc.positions.length}</b>/${sn.assets.length} coins held`, acc.positions.length, sn.assets.length), "#news");
}
function markout(group, h) { return (S.scalp?.summary?.markouts || []).find((m) => m.group === group && m.h === h); }
function workingSheet() {
  const sc = S.scalp, sn = S.snap;
  const tk = markout("taken", 600) || markout("taken", 300), vt = markout("jev_veto", 600);
  const need = 150, lines = [];
  if (sc?.summary) {
    const n = tk?.n || 0;
    lines.push(`<p><span class="lbl">Scalper</span> ${n < need ? `<b>Too early to tell:</b> ${n} of ${need} trades.` : tk.mean_net_bps > 0 ? "<b>Positive so far.</b>" : "<b>Not beating its fees.</b>"}${tk ? ` A $1,000 trade made <b class="${cls(per1k(tk.mean_net_bps))}">${susd(per1k(tk.mean_net_bps))}</b> after fees.` : ""}</p>`);
    if (vt && tk) lines.push(`<p class="${vt.n < 30 ? "lown" : ""}"><span class="lbl">Jev vetoes</span> blocked trades would have made ${susd(per1k(vt.mean_net_bps))} (n=${vt.n}), ${vt.mean_net_bps < tk.mean_net_bps ? "worse than allowed ones." : "not worse than allowed ones."}</p>`);
  }
  if (sn) { const d = sn.scorecard?.decisions; lines.push(`<p class="${!d || d.low_n ? "lown" : ""}"><span class="lbl">News bot</span> ${d ? `${d.n} decisions scored, ${pct(d.hit_rate, 0)} right${d.low_n ? ": too few to judge." : "."}` : "nothing scored yet."}</p>`); }
  return `<section class="sheet bot verdict"><div class="sh-top"><h2><a href="#evidence">Is it working?</a></h2></div>${lines.join("") || '<p class="sh-sub">Loading…</p>'}</section>`;
}

// ---------- views ----------
function controlsHTML() {
  const sn = S.snap;
  if (!sn?.status) return "";
  const paused = !!sn.control?.paused;
  const fl = S.confirmFlatten
    ? `<button class="btn danger" data-act="flatten-yes">${icon("flat")}Yes, sell everything</button><button class="btn quiet" data-act="flatten-no">Cancel</button>`
    : `<button class="btn red" data-act="flatten">${icon("flat")}Flatten news bot</button>`;
  return `<div class="controls"><div class="ctlrow"><button class="btn" data-act="pause">${icon(paused ? "play" : "pause")}${paused ? "Resume news bot" : "Pause news bot"}</button>${fl}</div><span class="ctlnote">News bot only. The scalper has no browser controls: Ctrl+C stops it.</span></div>`;
}
function postureLine() {
  const st = S.scalp?.status, sn = S.snap;
  const a = st ? (st.live_hold ? "scalper on hold" : st.halted_today ? "scalper done for today" : `scalper ${st.assets.filter((x) => x.position).length} open`) : "";
  const pk = sn?.status?.posture;
  const b = pk ? ({ ENTRIES_ALLOWED: "news bot taking entries", EXITS_ONLY: "news bot exits only", REVIEW: "news bot in review" })[pk.kind] + (pk.reason ? ` (${pk.reason})` : "") : "";
  return [a, b].filter(Boolean).join(" · ");
}
function boardView() {
  const v = $("#view");
  const rowsS = scalpRows(), rowsN = newsRows();
  const sig = rowsS.map((r) => r.key).join() + "|" + rowsN.map((r) => r.key).join();
  if (v.dataset.view !== "board" || v.dataset.sig !== sig) {
    v.dataset.view = "board"; v.dataset.sig = sig;
    const st = S.scalp?.status;
    v.innerHTML = `<header class="head"><div class="title"><h1>Departures</h1><span class="posture" id="posture"></span><p class="moneymini" id="moneymini"></p></div><div id="ctl"></div></header>
    <div class="sheets" id="sheets"></div>
    <section class="board" aria-label="Every coin both bots watch">
      <div class="board-top"><h2>${flapHTML("Departures", 10)}</h2><p>One row per coin. Select a row to see why.</p></div>
      <div class="bhead"><span></span><span>Coin</span><span>Side</span><span>Status</span><span>Waiting for</span><span title="Round-trip cost in bps / typical move in bps (scalper: over a hold; news bot: one hour)">Cost/move</span><span>Last result</span></div>
      <div class="bsec">Scalper <small>${rowsS.length} coins · checks every ${((st?.settings?.eval_ms || 500) / 1000).toFixed(1)} s · trades last minutes</small></div>
      ${rowsS.map(rowHTML).join("") || '<p class="board-foot">No scalper data.</p>'}
      <div class="bsec">News bot <small>${rowsN.length} coins · looks every ${S.snap?.settings?.interval_secs ?? 60} s · trades last hours to days</small></div>
      ${rowsN.map(rowHTML).join("") || '<p class="board-foot">No news bot data.</p>'}
      <div class="board-foot"><span class="key white"><i></i><span>watching</span></span><span class="key amber"><i></i><span>waiting, held by a limit, or needs review</span></span><span class="key green"><i></i><span>in a trade or a profit</span></span><span class="key red"><i></i><span>stopped or a loss</span></span><span>Side: which way it leans now. Cost/move: round-trip cost vs typical move, in bps (100 bps = 1%); the scalper needs the move to be ${scalpSettings().cost_mult ?? 2}× the cost.</span></div>
    </section>`;
    for (const r of [...rowsS, ...rowsN]) { const tr = v.querySelector(`[data-trace="${r.key}"]`); if (tr && !tr.hidden) tr.innerHTML = r.key[0] === "s" ? scalpCoinWhy(r.a) : newsCoinWhy(r.a); }
  } else {
    for (const r of [...rowsS, ...rowsN]) updateRow(r);
  }
  $("#posture").textContent = postureLine();
  const ma = scalpMoney(), mb = newsMoney();
  $("#moneymini").textContent = [ma && `Scalper: ${ma.label.toLowerCase()}, ${ma.kind === "paper" ? usd(ma.capital, 0) + " simulated" : ma.kind === "testnet" ? "fake money" : "REAL money, cap " + usd(ma.capital, 0)}`, mb && `News bot: ${mb.label.toLowerCase()}, ${mb.kind === "paper" ? usd(mb.start, 0) + " simulated" : mb.kind === "testnet" ? "fake money" : "REAL money"}`].filter(Boolean).join(" · ") + (ma?.kind !== "live" && mb?.kind !== "live" ? " · no real money" : "");
  $("#ctl").innerHTML = controlsHTML();
  $("#sheets").innerHTML = scalpSheet() + newsSheet() + workingSheet();
}

function tabsHTML(page, tabs) {
  const cur = S.tab[page] || tabs[0][0];
  return `<div class="tabs" role="tablist">${tabs.map(([id, label, n]) => `<button class="tab" role="tab" data-tab="${page}:${id}" aria-selected="${cur === id}">${label}${n != null ? `<small>${n}</small>` : ""}</button>`).join("")}</div>`;
}
function outcomeOf(c) {
  if (c.taken) return ["ok", "Taken"];
  const g = c.blocked_by;
  return [g === "jev review" ? "no" : "wait", stoppedBy(g)];
}
function afterCells(c) {
  return [60, 300, 600].map((h) => {
    const a = c.after?.[h];
    if (!a) return `<td class="n low">—</td>`;
    const v = per1k(a.net_bps);
    return `<td class="n ${cls(v)}">${susd(v)}</td>`;
  }).join("");
}
function candidateTrace(c) {
  const g = [...(c.eval?.gates || []), ...(c.risk || [])];
  const stopIdx = g.findIndex((x) => !x.passed);
  const li = g.map((x, i) => {
    const state = x.passed ? "ok" : i === stopIdx ? "no" : "skip";
    return `<li class="${state}"><span class="mk">${icon(state === "ok" ? "check" : state === "no" ? "x" : "dash")}</span><span>${esc(gateInfo(x.gate)[0])}</span><span class="raw">${esc(x.detail || "")}</span></li>`;
  }).join("");
  const r = c.review;
  const rv = r ? `<div class="readings"><div class="reading"><b>${num(r.adverse, 2)}</b><small>adverse news (vetoes at ${scalpSettings().jev_veto ?? 0.6})</small></div><div class="reading"><b>${num(r.support, 2)}</b><small>news supports the move</small></div><div class="reading"><b>${num(r.event_risk, 2)}</b><small>chance of a big scheduled event</small></div><div class="reading"><b>${r.headlines}</b><small>headlines read</small></div></div>` : c.review_error ? `<p class="sub">Jev review failed: ${esc(c.review_error)}</p>` : `<p class="sub">Jev wasn't asked: an earlier check stopped it.</p>`;
  const [, word] = outcomeOf(c);
  const sl = c.eval?.sl_bps, tp = c.eval?.tp_bps;
  return `<div class="why"><h3>${esc(c.asset)} ${esc(c.side)} at ${hms(t(c.ts))}: ${esc(word.toLowerCase())}.</h3>
  <p class="sub">${c.taken ? `Planned stop ${num(sl, 1)} bps away, target ${num(tp, 1)} bps away; size ×${c.size_mult ?? 1}.` : "Nothing was bought. The columns on the right show what would have happened."} Round-trip cost was ${num(c.eval?.cost_bps, 1)} bps (${usd((c.eval?.cost_bps || 0) / 10)} on $1,000).</p>
  <div class="cols"><div><span class="lbl">Checks</span><ul class="checks">${li}</ul></div><div><span class="lbl">Jev's review</span>${rv}</div></div></div>`;
}
function scalperView() {
  const v = $("#view"), sc = S.scalp;
  v.dataset.view = "scalper"; v.dataset.sig = "";
  if (!sc?.available) { v.innerHTML = `<header class="head"><h1>Scalper</h1></header><div class="sheet empty">${sc ? "No scalper data yet. Start it with <code>jev-trader --config config.scalp.toml scalp</code>." : "Loading…"}</div>`; return; }
  const st = sc.status, s = st.settings || {}, sum = sc.summary || {};
  const cands = sc.candidates || [], trades = sc.trades || [];
  const tab = S.tab.scalper || "decisions";
  const open = st.assets.filter((a) => a.position);
  const openHTML = open.map((a) => {
    const p = a.position, mid = a.features?.mid, dir = p.side === "short" ? -1 : 1;
    const lo = Math.min(p.sl_px, p.tp_px, mid ?? p.entry_px), hi = Math.max(p.sl_px, p.tp_px, mid ?? p.entry_px);
    const X = (x) => ((x - lo) / Math.max(1e-12, hi - lo)) * 100;
    const u = mid ? dir * (mid - p.entry_px) * p.qty - (p.entry_fee || 0) + (p.funding_usd || 0) : null;
    const left = (p.deadline_ms - Date.now()) / 1000;
    return `<section class="sheet"><h2>${esc(a.symbol)} <span class="side ${p.side}">${p.side}</span> <span class="tag ok">open</span></h2>
    <p class="big ${cls(u)}">${susd(u)}<small>so far, after the entry fee · ${dur(left)} left at most</small></p>
    <div class="ruler" style="--e:${X(p.entry_px)}%"><span class="bar"></span><span class="tick stop" style="left:${X(p.sl_px)}%"></span><span class="tick" style="left:${X(p.entry_px)}%"></span><span class="tick tp" style="left:${X(p.tp_px)}%"></span>${mid ? `<span class="now" style="left:${X(mid)}%"></span>` : ""}</div>
    <div class="ruler-l"><span>stop <b>${px(p.sl_px)}</b></span><span>entry <b>${px(p.entry_px)}</b> · now <b>${px(mid)}</b></span><span>target <b>${px(p.tp_px)}</b></span></div>
    <p class="say" style="margin-top:10px">${usd(Math.abs(p.entry_cash ?? p.qty * p.entry_px), 0)} ${p.side === "short" ? "sold short" : "bought"} at ${hm(new Date(p.opened_ms))}. Jev: adverse ${num(p.review?.adverse, 2)}, support ${num(p.review?.support, 2)}.</p></section>`;
  }).join("");
  let body = "";
  if (tab === "decisions") {
    const F = { all: () => true, taken: (c) => c.taken, veto: (c) => c.blocked_by === "jev review", cost: (c) => c.blocked_by === "cost", limits: (c) => ["cooldown", "trades/hour", "max positions", "daily limit", "flat"].includes(c.blocked_by) };
    const list = cands.filter(F[S.filter] || F.all);
    const chips = [["all", "All"], ["taken", "Taken"], ["veto", "Jev vetoed"], ["cost", "Too small for fees"], ["limits", "Held by limits"]].map(([k, l]) => `<button class="chip" data-filter="${k}" aria-pressed="${S.filter === k}">${l}</button>`).join("");
    const rows = list.map((c) => {
      const [k, word] = outcomeOf(c), open = S.open.has("c:" + c.id);
      return `<tr class="click ${open ? "open" : ""}" data-row="c:${esc(c.id)}" tabindex="0" aria-expanded="${open}"><td>${hms(t(c.ts))}</td><td><b>${esc(c.asset)}</b></td><td><span class="side ${c.side}">${esc(c.side)}</span></td><td><span class="tag ${k}">${esc(word)}</span></td>${afterCells(c)}</tr>${open ? `<tr class="detail"><td colspan="7">${candidateTrace(c)}</td></tr>` : ""}`;
    }).join("");
    body = `<div class="filters">${chips}</div><div class="tablewrap"><table><thead><tr><th>Time</th><th>Coin</th><th>Side</th><th>What happened</th><th class="n">1 min later</th><th class="n">5 min later</th><th class="n">10 min later</th></tr></thead><tbody>${rows || `<tr><td colspan="7" class="empty">No decisions match this filter yet.</td></tr>`}</tbody></table></div>
    <p class="say low" style="font-size:13px;color:var(--steel-ink)">"Later" columns: what a $1,000 trade in that direction would have made after fees, whether or not it was taken. ${cands.length < (sum.candidates || 0) ? `Showing the latest ${cands.length} of ${sum.candidates}.` : ""}</p>`;
  } else if (tab === "trades") {
    const rows = trades.map((x) => {
      const id = "t:" + x.candidate_id + x.closed, open = S.open.has(id);
      const story = `${x.side === "short" ? "Sold short" : "Bought"} ${usd(x.notional, 0)} of ${esc(x.asset)} at ${px(x.entry_px)} (${x.entry_order === "paper-maker" || /maker/.test(x.entry_order || "") ? "a resting order, the cheaper fee" : "a market order"}). Stop ${px(x.init_sl_px)}, target ${px(x.tp_px)}. After ${dur(x.hold_secs)} it closed at ${px(x.exit_px)}: ${esc(x.reason)}. Fees ${usd(x.fees_usd)}.`;
      return `<tr class="click ${open ? "open" : ""}" data-row="${esc(id)}" tabindex="0" aria-expanded="${open}"><td>${hm(t(x.opened))}<span class="sm">${day(t(x.opened))}</span></td><td><b>${esc(x.asset)}</b></td><td><span class="side ${x.side}">${esc(x.side)}</span></td><td>${dur(x.hold_secs)}</td><td>${esc(x.reason)}</td><td class="n">${usd(x.fees_usd)}</td><td class="n ${cls(x.pnl_usd)}"><b>${susd(x.pnl_usd)}</b></td></tr>${open ? `<tr class="detail"><td colspan="7"><div class="why"><h3>${story}</h3>${x.candidate ? candidateTrace(x.candidate) : ""}</div></td></tr>` : ""}`;
    }).join("");
    body = `<div class="tablewrap"><table><thead><tr><th>Opened</th><th>Coin</th><th>Side</th><th>Held</th><th>Why it closed</th><th class="n">Fees</th><th class="n">Result after fees</th></tr></thead><tbody>${rows || `<tr><td colspan="7" class="empty">No closed trades yet.</td></tr>`}</tbody></table></div>`;
  } else {
    const blocked = sum.blocked_by || {};
    let left = sum.candidates || 0;
    const order = [...MARKET_GATES.filter((g) => g !== "warm-up"), ...RISK_GATES];
    const items = order.map((g, i) => {
      const n = blocked[g] || 0, [name, say] = gateInfo(g);
      const before = left; left -= n;
      return `<li><span class="num">${i + 1}</span><div><h4>${esc(name)}</h4><p>${esc(say)}</p></div><div class="cnt"><b class="${n ? "neg" : ""}">${n ? MINUS + n : "0"}</b><span>${before} reached it</span></div></li>`;
    }).join("");
    body = `<section class="sheet"><h2>How it decides</h2><p class="say" style="max-width:74ch">A chance starts when the market checks below pass for one side. Since ${hm(t(sum.session_start))} it found <b>${sum.candidates ?? 0}</b> chances that got past warm-up. Each check stopped some; <b>${blocked.taken ?? 0}</b> made it through every one and were placed as orders. Then it exits at the target, the stop (which trails to break-even once profit covers ${s.trail_after_cost_mult ?? 1.5}× the cost), when the flow reverses, on a hack alert, or after ${Math.round((s.hold_secs || 600) / 60)} minutes.</p>
    <ol class="sieve" style="margin-top:12px">${items}</ol></section>`;
  }
  v.innerHTML = `<header class="head"><h1>Scalper</h1><p class="lede">It watches ${st.assets.length} Hyperliquid perps (${st.assets.map((a) => a.symbol).join(", ")}) every ${((s.eval_ms || 500) / 1000).toFixed(1)} s. When trading suddenly rushes one way and the move looks big enough to pay its fees, it joins: long when buyers rush in, short when sellers do. Before each entry Jev reads recent headlines and can veto it. Orders rest at the best price for the lower fee (${s.maker_fee_bps ?? 1.5} bps instead of ${s.fee_bps ?? 4.5}).</p></header>
  ${open.length ? `<div class="sheets">${openHTML}</div>` : ""}
  ${tabsHTML("scalper", [["decisions", "Decisions", sum.candidates], ["trades", "Trades", sum.trades?.n], ["how", "How it decides"]])}
  ${body}`;
}

function newsView() {
  const v = $("#view"), sn = S.snap;
  v.dataset.view = "news"; v.dataset.sig = "";
  if (!sn) { v.innerHTML = `<header class="head"><h1>News bot</h1></header><div class="sheet empty">Loading…</div>`; return; }
  const set = sn.settings || {}, tab = S.tab.news || "coins";
  let body = "";
  if (tab === "coins") {
    body = `<section class="sheet">${sn.assets.map((a) => {
      const r = newsRows().find((x) => x.coin === a.symbol);
      return `<div class="coinrow"><div><h3>${esc(a.symbol)}</h3><p class="st"><span class="tag ${r.tone === "green" ? "ok" : r.tone === "amber" ? "wait" : ""}">${esc(r.status.toLowerCase())}</span></p><p class="say">${esc(a.name)}</p></div><div style="grid-column: 2 / -1">${newsCoinWhy(a)}</div></div>`;
    }).join("")}</section>`;
  } else if (tab === "headlines") {
    const ev = (sn.events || []).slice(0, 120);
    const used = new Set(sn.assets.flatMap((a) => a.latest?.drivers || []));
    body = `<div class="tablewrap"><table><thead><tr><th>Published</th><th>Headline</th><th>Coin</th><th>Jev says</th><th class="n">New info</th></tr></thead><tbody>${ev.map((e) => `<tr><td>${hm(t(e.published))}<span class="sm">${esc(e.source)}</span></td><td><a href="${esc(e.id)}" target="_blank" rel="noopener">${esc(e.title)}</a>${used.has(e.id) ? '<span class="sm">Driving a score now</span>' : ""}</td><td>${esc(e.asset)}<span class="sm">${pct(e.asset_conf, 0)} sure</span></td><td><span class="${e.sentiment > 0.25 ? "pos" : e.sentiment < -0.25 ? "neg" : ""}">${sentWord(e.sentiment)} ${sgn(e.sentiment, 2)}</span><span class="sm">${esc(e.event)}</span></td><td class="n">${pct(e.new_info, 0)}</td></tr>`).join("") || `<tr><td colspan="5" class="empty">No headlines analysed yet.</td></tr>`}</tbody></table></div>
    <p class="say" style="font-size:13px;color:var(--steel-ink)">Jev answers four questions per headline: which coin, what kind of event, how positive or negative (−1 to +1), and how likely it's genuinely new. ${sn.spiked ? `${sn.spiked} headlines were given up on after Jev failed repeatedly.` : ""}</p>`;
  } else if (tab === "trades") {
    body = `<div class="tablewrap"><table><thead><tr><th>Time</th><th>Coin</th><th>Side</th><th class="n">Price</th><th class="n">Amount</th><th>Why</th><th class="n">Result after fees</th></tr></thead><tbody>${(sn.trades || []).map((f) => { const v2 = f.round_trip_pnl ?? f.realized_pnl; return `<tr><td>${hm(t(f.ts))}<span class="sm">${day(t(f.ts))}</span></td><td><b>${esc(f.asset)}</b></td><td><span class="side ${f.side === "buy" ? "long" : "short"}">${esc(f.side)}</span></td><td class="n">${px(f.price)}</td><td class="n">${usd(f.notional, 0)}</td><td>${esc(f.reason)}${newsHeadlines(f.events).slice(0, 2).map((e) => `<span class="sm">${esc(e.title)}</span>`).join("")}</td><td class="n ${cls(v2)}">${v2 != null ? susd(v2) : "—"}</td></tr>`; }).join("") || `<tr><td colspan="7" class="empty">No trades yet. It buys only when the news score passes +${num(set.news_entry, 2)} and price is rising.</td></tr>`}</tbody></table></div>`;
  } else {
    const st = sn.status;
    body = `<section class="sheet"><h2>Each look, step by step</h2><p class="say">Last look ${ago(t(st?.last_tick_at))}, look #${st?.tick ?? "—"}. Jev used ${st?.tokens_today ? `${st.tokens_today[0]} calls today` : "no calls yet"}.</p></section>
    <div class="tablewrap"><div class="stages">${(st?.stages || []).map((g) => `<div class="stage ${g.state}"><i></i><b>${esc(g.name)}</b><span class="low">${g.ms != null ? g.ms + " ms" : ""}</span><small>${esc(g.detail)}</small></div>`).join("") || '<p class="empty">No looks recorded yet.</p>'}</div></div>`;
  }
  v.innerHTML = `<header class="head"><div class="title"><h1>News bot</h1></div><div id="ctl">${controlsHTML()}</div><p class="lede">Every ${set.interval_secs} s it reads headlines from ${set.feeds?.length ?? 3} crypto news sites and asks Jev what each one means for ${sn.assets.map((a) => a.symbol).join(", ")}. It adds them into a news score per coin, with older headlines counting less. It buys when that score is clearly positive and the hourly price trend is up, and sells when the news turns bad, on a hack alert, or at a ${pct(set.stop_loss_pct, 0)} stop. Long only, and no coin above ${pct(set.max_asset_weight, 0)} of the account.</p></header>
  ${tabsHTML("news", [["coins", "Coins"], ["headlines", "Headlines", sn.events?.length], ["trades", "Trades", sn.trades?.length], ["steps", "Each look"]])}
  ${body}`;
}

function evidenceView() {
  const v = $("#view"), sc = S.scalp, sn = S.snap;
  v.dataset.view = "evidence"; v.dataset.sig = "";
  const groups = [["taken", "Taken"], ["jev_veto", "Jev vetoed"], ["cost", "Too small for fees"], ["other", "Other skips"], ["long", "All longs"], ["short", "All shorts"], ["all", "Everything"]];
  const mrow = ([g, label]) => `<tr><td><b>${label}</b></td>${[60, 300, 600].map((h) => { const m = markout(g, h); if (!m) return `<td class="n low">—</td>`; const lo = m.n < 30; return `<td class="n ${lo ? "low" : cls(per1k(m.mean_net_bps))}"><b>${susd(per1k(m.mean_net_bps))}</b><span class="sm">${pct(m.hit, 0)} won · n=${m.n}</span></td>`; }).join("")}</tr>`;
  const hyps = sn?.hypotheses || [];
  // The scalper's measurement loop (src/scalp/report.rs): each H-C test applied to the data.
  const ss = sc?.scorecard;
  const prog = Object.fromEntries((ss?.verdicts || []).map((x) => [x.id, [x.progress, x.need, x.detail, x.verdict]]));
  const oneHyp = (h) => {
    const p = prog[h.id];
    const tag = { testing: "wait", accepted: "ok", rejected: "no", proposed: "" }[h.status] ?? "";
    return `<div class="hyp"><span class="id">${esc(h.id)}</span><div><h4>${esc(h.statement)}</h4><p><b>Passes if</b> ${esc(h.pass_if)}</p><p><b>Fails if</b> ${esc(h.invalidate_if)}</p>${h.result ? `<p><b>Result</b> ${esc(h.result)}</p>` : ""}</div><div class="prog"><span class="tag ${tag}">${esc(h.status)}</span>${p ? `<span>${esc(p[2])}</span>${meter(p[0], p[1])}${p[3] && p[3] !== "testing" ? `<span class="tag ${p[3] === "pass" ? "ok" : "no"}">data says ${esc(p[3])}</span>` : ""}` : ""}</div></div>`;
  };
  const rank = { testing: 0, accepted: 1, rejected: 2, proposed: 3 };
  const group = (title, list) => {
    if (!list.length) return "";
    list = list.slice().sort((a, b) => (rank[a.status] ?? 9) - (rank[b.status] ?? 9) || (b.id.startsWith("H-C") - a.id.startsWith("H-C")));
    const live = list.filter((h) => h.status !== "proposed"), later = list.filter((h) => h.status === "proposed");
    return `<div class="hypgroup"><h3>${title}</h3>${live.map(oneHyp).join("")}${later.length ? `<details class="hypmore"><summary>${icon("chev")}${later.length} proposed, not yet being tested</summary>${later.map(oneHyp).join("")}</details>` : ""}</div>`;
  };
  const isScalp = (h) => /^H-(C|S)/.test(h.id);
  const hypHTML = group("Scalper", hyps.filter(isScalp)) + group("News bot", hyps.filter((h) => !isScalp(h)));
  const eg = [["passed every market check", "Passed every check"], ["placed as an order", "Placed as an order"], ["filled", "Filled orders"]];
  const ecell = (g, h) => {
    const e = (ss?.edge || []).find((x) => x.group === g && x.h === h);
    if (!e) return `<td class="n low">—</td>`;
    return `<td class="n ${e.net.low_n ? "low" : cls(per1k(e.net.mean))}"><b>${susd(per1k(e.net.mean))}</b><span class="sm">${e.net.lo != null ? `${susd(per1k(e.net.lo))} to ${susd(per1k(e.net.hi))}` : "no range yet"} · ${esc(e.verdict)}</span></td>`;
  };
  const erow = ([g, label]) => `<tr><td><b>${label}</b></td>${[60, 300, 600].map((h) => ecell(g, h)).join("")}</tr>`;
  const sevTag = { high: "no", medium: "wait" };
  const backlogHTML = (ss?.backlog || []).map((b) => `<div class="line"><span class="lbl"><span class="tag ${sevTag[b.severity] || ""}">${esc(b.severity)}</span></span><span><b>${esc(b.pattern)}</b> (n=${b.n}): ${esc(b.effect)}. ${esc(b.suggestion)}.</span></div>`).join("");
  const scd = sn?.scorecard;
  const headRows = (scd?.headlines || []).map((r) => `<tr class="${r.low_n ? "low" : ""}"><td>${r.h} h after</td><td class="n">${r.n_clusters}</td><td class="n">${sgn(r.ic, 2)}</td><td class="n">${pct(r.hit_rate, 0)}</td><td>${r.low_n ? "too few to judge" : r.null_pct != null ? `beats ${num(r.null_pct, 0)}% of random shuffles` : ""}</td></tr>`).join("");
  const rg = sn?.regime;
  v.innerHTML = `<header class="head"><h1>Is it working?</h1><p class="lede">Neither bot has proven an edge yet. Every decision is scored afterwards against what the price actually did, net of fees, and each bot has written tests (hypotheses) that say how much evidence is needed before trusting it. Grey figures rest on too few cases to mean anything.</p></header>
  <section class="sheet"><h2>Scalper: what each kind of decision would have made</h2><p class="say">Dollars per $1,000 trade after fees, 1, 5 and 10 minutes after the decision, whether or not it was taken. If the scalper is any good, "Taken" should beat the skipped groups and stay above $0.</p></section>
  <div class="tablewrap"><table><thead><tr><th>Decision</th><th class="n">1 min later</th><th class="n">5 min later</th><th class="n">10 min later</th></tr></thead><tbody>${sc?.summary ? groups.map(mrow).join("") : `<tr><td colspan="4" class="empty">No scalper data.</td></tr>`}</tbody></table></div>
  <section class="sheet"><h2>Scalper: does the signal pay?</h2><p class="say">The same question with its uncertainty: dollars per $1,000 after fees, and a 95% range that counts each hour as one piece of evidence, because moves inside the same hour aren't independent. A range entirely below $0 means it loses money after fees. ${ss ? `${ss.candidates} chances over ${ss.hours} hours.` : ""} Fills, exits, each check and which inputs predict anything: <code>jev-trader --config config.scalp.toml scalp-report</code>.</p></section>
  <div class="tablewrap"><table><thead><tr><th>Decision</th><th class="n">1 min later</th><th class="n">5 min later</th><th class="n">10 min later</th></tr></thead><tbody>${ss ? eg.map(erow).join("") : `<tr><td colspan="4" class="empty">No scalper data.</td></tr>`}</tbody></table></div>
  <section class="sheet"><h2>Scalper: what the log says</h2><p class="say">Patterns with enough evidence (20 hours or 20 cases), as suggestions. Nothing here changes a setting.</p>${backlogHTML ? `<div class="verdict" style="margin-top:10px">${backlogHTML}</div>` : `<p class="empty">Nothing has enough evidence yet${ss ? ` (${ss.hours} hours of data so far)` : ""}.</p>`}</section>
  <section class="sheet"><h2>The tests</h2><p class="say">From <code>hypotheses.toml</code>. A strategy change is written here before it is tried.</p>${hypHTML || '<p class="empty">No hypotheses file found next to the dashboard.</p>'}</section>
  <section class="sheet"><h2>News bot: do headlines predict the price?</h2><p class="say">Each headline is scored a few hours later. Hit rate is how often the price moved the way Jev's reading pointed; IC is the correlation between reading and move (−1 to +1). Overlapping stories count once. ${scd ? `${scd.n_settled} scored so far.` : ""}</p>
  ${headRows ? `<div class="tablewrap" style="box-shadow:none;margin-top:10px"><table><thead><tr><th>Horizon</th><th class="n">Stories</th><th class="n">IC</th><th class="n">Hit rate</th><th></th></tr></thead><tbody>${headRows}</tbody></table></div>` : '<p class="empty">Nothing scored yet.</p>'}
  ${rg ? `<p class="say" style="margin-top:12px">Market regime: <b>${esc(rg.zone)}</b> (${num(rg.score, 0)} of 100). ${set_regime_mode()}</p>` : ""}</section>`;
}
function set_regime_mode() { const m = S.snap?.settings?.regime_mode; return m === "record" || m === "recordonly" || m === "record_only" ? "Recorded only; it doesn't change decisions yet." : m ? `Mode: ${esc(m)}.` : ""; }

// ---------- render loop ----------
function render() {
  renderRail(); renderNotices();
  const titles = { board: "Departures", scalper: "Scalper", news: "News bot", evidence: "Is it working?" };
  document.title = (titles[S.view] || "Departures") + " · Jev trader";
  const keep = window.scrollY;
  if (S.view === "board") boardView();
  else if (S.view === "scalper") scalperView();
  else if (S.view === "news") newsView();
  else evidenceView();
  if (S.view !== "board") window.scrollTo(0, keep);
}
function route() {
  const v = (location.hash || "#board").slice(1);
  S.view = ["board", "scalper", "news", "evidence"].includes(v) ? v : "board";
  $("#view").dataset.view = "";
  render();
  window.scrollTo(0, 0);
}
addEventListener("hashchange", route);

document.addEventListener("click", async (e) => {
  const b = e.target.closest("button, tr.click");
  if (!b) return;
  if (b.classList.contains("brow")) {
    const k = b.dataset.key, open = !S.open.has(k);
    open ? S.open.add(k) : S.open.delete(k);
    b.setAttribute("aria-expanded", open);
    const tr = document.querySelector(`[data-trace="${k}"]`);
    tr.hidden = !open;
    if (open) { const r = [...scalpRows(), ...newsRows()].find((x) => x.key === k); tr.innerHTML = k[0] === "s" ? scalpCoinWhy(r.a) : newsCoinWhy(r.a); }
    return;
  }
  if (b.dataset.row) { const k = b.dataset.row; S.open.has(k) ? S.open.delete(k) : S.open.add(k); render(); return; }
  if (b.dataset.tab) { const [p, id] = b.dataset.tab.split(":"); S.tab[p] = id; render(); return; }
  if (b.dataset.filter) { S.filter = b.dataset.filter; render(); return; }
  const act = b.dataset.act;
  if (act === "pause") { await control("/api/control/pause", { paused: !S.snap?.control?.paused }); }
  else if (act === "flatten") { S.confirmFlatten = true; render(); }
  else if (act === "flatten-no") { S.confirmFlatten = false; render(); }
  else if (act === "flatten-yes") { S.confirmFlatten = false; await control("/api/control/flatten", null); }
});
document.addEventListener("keydown", (e) => {
  if ((e.key === "Enter" || e.key === " ") && e.target.matches("tr.click")) { e.preventDefault(); e.target.click(); }
});
async function control(url, body) {
  try {
    const r = await fetch(url, { method: "POST", headers: { "Content-Type": "application/json" }, body: body ? JSON.stringify(body) : "{}" });
    if (!r.ok) throw new Error(await r.text());
    S.etag = null; await getSnap(); render();
  } catch (err) { S.err.snap = "control failed: " + (err.message || err); renderNotices(); }
}

let ticking = false;
async function tick() {
  if (ticking) return; ticking = true;
  try { await Promise.all([getSnap(), getScalp()]); render(); } finally { ticking = false; }
}
route();
tick();
setInterval(tick, 2500);
setInterval(renderRail, 1000);
