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
  plus: '<path d="M12 5v14M5 12h14"/>',
  search: '<circle cx="11" cy="11" r="6.5"/><path d="M16 16l4.5 4.5"/>',
};
const icon = (n) => `<svg class="i" viewBox="0 0 24 24" aria-hidden="true">${ICON[n]}</svg>`;

// ---------- the scalper's checks, in the order it runs them ----------
function scalpSettings() { return S.scalp?.status?.settings || {}; }
const MARKET_GATES = ["warm-up", "feed", "spread", "burst", "flow", "book", "push", "no chase", "cost", "news"];
const RISK_GATES = ["flat", "max positions", "cooldown", "trades/hour", "daily limit", "macro", "exchange", "cash", "jev review"];
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
    macro: ["The macro release to pass", `No new trades from ${s.macro_before_mins ?? 30} minutes before a CPI, jobs or Fed release to ${Math.round((s.macro_after_mins ?? 120) / 60)} h after it.`, "MACRO TO PASS"],
    exchange: ["Exchange matching the books", "Live only: the exchange's positions must match the ledger.", "BOOKS TO MATCH"],
    cash: ["Enough cash", `Needs at least $${s.min_trade_usd ?? 10} free.`, "MORE CASH"],
    "jev review": ["Jev's OK on the news", `Jev reads the last ${Math.round((s.jev_headline_minutes ?? 180) / 60)} h of headlines about the coin. Adverse news at ${s.jev_veto ?? 0.6} or more vetoes; news against the move halves the size.`, "JEV'S OK"],
  };
  return m[g] || [g, "", String(g || "").toUpperCase()];
}
const gateWait = (g) => gateInfo(g)[2];
// What it means when a check stops a trade.
const STOPPED = { "warm-up": "Still warming up", feed: "Prices were stale", spread: "Spread too wide", burst: "No rush of trading", flow: "Flow not one-sided", book: "Order book disagreed", push: "Price wasn't moving", "no chase": "Too late, already ran", cost: "Too small for fees", news: "News argued against it", flat: "Already in this coin", "max positions": "No free slot", cooldown: "Cooling down", "trades/hour": "Hourly limit full", "daily limit": "Daily loss limit hit", macro: "Macro release window", exchange: "Exchange mismatch", cash: "Not enough cash", "jev review": "Jev vetoed" };
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
      if (["cooldown", "trades/hour", "daily limit", "macro", "max positions"].includes(a.blocked_by)) { status = "HELD"; tone = "amber"; wtone = "amber"; }
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
    head = ["cooldown", "trades/hour", "daily limit", "macro", "max positions", "flat"].includes(block) ? `${sym}: held, waiting for ${name.toLowerCase()}.` : `${sym}: watching, waiting for ${name.toLowerCase()}.`;
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
function scalperViewLegacy() {
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
    const F = { all: () => true, taken: (c) => c.taken, veto: (c) => c.blocked_by === "jev review", cost: (c) => c.blocked_by === "cost", limits: (c) => ["cooldown", "trades/hour", "max positions", "daily limit", "macro", "flat"].includes(c.blocked_by) };
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

// ---------- backtests ----------
// Reads /api/backtests (the runs) and /api/backtests/<id> (hourly equity per variant + the strategy's fills).
// Drag across the chart to measure any stretch; the readout, bars and trade list follow the stretch.
const HOUR = 3600e3, DAY = 24 * HOUR;
const BT = { list: null, id: null, run: null, ser: null, fills: [], err: null, sel: null, zoom: null, pick: null, drag: null, show: { momentum_only: true, news_only: false, hold: true } };
const SERIES = [["strategy", "Strategy"], ["momentum_only", "Momentum only"], ["news_only", "News only"], ["hold", "Hold every coin"]];
// The first comparison series a run has (momentum-only for news runs, holding every coin for scalper runs).
const btCmp = () => SERIES.slice(1).find(([k]) => BT.ser?.[k]?.t.length) || [null, ""];
const VERDICT = {
  RESIDUAL_EDGE: ["ok", "Edge beyond momentum", "Jev's news reading added returns that momentum alone doesn't explain, and it held across the run."],
  RESIDUAL_FRAGILE: ["wait", "Fragile edge", "A hint of returns beyond momentum, but it doesn't hold across the run or against simply holding BTC."],
  BASELINE_EXPLAINED: ["no", "Momentum explains it", "Momentum alone explains the returns. The news reading added nothing measurable."],
  INSUFFICIENT_EVIDENCE: ["", "Too little evidence", "Too few hours, decisions or trades to tell either way."],
  PAYS_AFTER_FEES: ["ok", "Pays after fees", "The average trade made money after fees, by more than chance would explain (t above 2). Trades overlap in time, so this overstates the certainty a little."],
  LOSES_AFTER_FEES: ["no", "Loses after fees", "The average trade lost money after fees, by more than chance would explain."],
  UNCLEAR: ["wait", "Unclear", "The average trade isn't clearly above or below zero after fees. It needs more trades to tell."],
};
const lb = (a, x) => { let lo = 0, hi = a.length; while (lo < hi) { const m = (lo + hi) >> 1; a[m] < x ? (lo = m + 1) : (hi = m); } return lo; };
const when = (ms) => { const d = new Date(ms); return `${day(d)}, ${hm(d)}`; };
function span(ms) { const h = Math.round(ms / HOUR); return h < 48 ? `${h} h` : `${Math.floor(h / 24)} days${h % 24 ? ` ${h % 24} h` : ""}`; }

async function btLoad(id) {
  try {
    if (!BT.list) { const r = await fetch("/api/backtests"); if (!r.ok) throw new Error("HTTP " + r.status); BT.list = await r.json(); }
    id = id || BT.id || BT.list.runs[0]?.id;
    if (id && BT.run?.id !== id) {
      const r = await fetch("/api/backtests/" + encodeURIComponent(id));
      if (!r.ok) throw new Error("HTTP " + r.status);
      const run = await r.json();
      BT.ser = {};
      for (const [k] of SERIES) { const e = run.equity[k] || []; BT.ser[k] = { t: e.map((x) => Date.parse(x[0])), v: e.map((x) => x[1]) }; }
      const s = BT.ser.strategy;
      BT.fills = (run.trades || []).map((f) => { const tms = Date.parse(f.ts); let i = Math.min(lb(s.t, tms), s.t.length - 1); if (i > 0 && tms - s.t[i - 1] < s.t[i] - tms) i--; return { ...f, tms, i }; });
      BT.run = run; BT.id = id; BT.sel = BT.zoom = BT.pick = null;
    }
    BT.err = null;
  } catch (e) { BT.err = String(e.message || e); }
  if (S.view === "backtest") { $("#view").dataset.sig = ""; backtestView(); }
}

// Equity at or before t (first value if t is before the run).
const valAt = (ser, t) => ser.v[Math.max(0, lb(ser.t, t + 1) - 1)];
function btWindow() {
  const s = BT.ser.strategy, full = [s.t[0], s.t[s.t.length - 1]];
  return BT.sel || BT.zoom || full;
}
function btStats([a, b]) {
  const s = BT.ser.strategy;
  const i0 = Math.min(lb(s.t, a), s.t.length - 1), i1 = Math.max(i0, lb(s.t, b + 1) - 1);
  let peak = s.v[i0], mdd = 0;
  for (let i = i0; i <= i1; i++) { peak = Math.max(peak, s.v[i]); mdd = Math.max(mdd, 1 - s.v[i] / peak); }
  const fills = BT.fills.filter((f) => f.i >= i0 && f.i <= i1);
  const m = BT.ser[btCmp()[0]];
  const mom = m?.t.length ? valAt(m, s.t[i1]) - valAt(m, s.t[i0]) : null;
  return { a: s.t[i0], b: s.t[i1], start: s.v[i0], end: s.v[i1], pnl: s.v[i1] - s.v[i0], ret: s.v[i1] / s.v[i0] - 1, mdd, fills, mom,
    realized: fills.reduce((x, f) => x + (f.realized_pnl || 0), 0), fees: fills.reduce((x, f) => x + (f.fee || 0), 0) };
}

function backtestView() {
  const v = $("#view");
  const job = BT.list?.job;
  const sig = `bt|${BT.id}|${BT.list ? BT.list.runs.length : "-"}|${BT.err || ""}|${BT.compose}|${job?.id}:${job?.state}`;
  if (v.dataset.view === "backtest" && v.dataset.sig === sig) return;
  v.dataset.view = "backtest"; v.dataset.sig = sig;
  const busy = job?.state === "running";
  const head = `<header class="head"><div class="title"><h1>Backtests</h1></div>${BT.list?.runs?.length ? `<div class="controls"><button class="btn" data-bt="new" aria-expanded="${!!BT.compose}"${busy ? " disabled" : ""}>${icon("plus")}New backtest</button></div>` : ""}<p class="lede">A backtest runs a strategy's own rules over past prices and makes every trade it would have made. The scalper replays on Hyperliquid's 15-minute candles, with no news needed. The news bot replays its cached Jev answers, so it only trades on days the bot was running. Costs are modelled, not measured, so a good run is a reason to keep testing, not proof.</p></header>`;
  if (BT.err) { v.innerHTML = head + `<div class="notice bad">${icon("warn")}<p><b>Can't read the backtests:</b> ${esc(BT.err)}.</p></div>`; return; }
  if (!BT.list || (BT.list.runs.length && !BT.run)) { v.innerHTML = head + `<div class="sheet empty">Loading…</div>`; if (!BT.list) btLoad(); return; }
  if (!BT.form) BT.form = btDefaultForm();
  const composing = (BT.compose || !BT.list.runs.length) && !busy;
  const top = `<div id="bt-job">${btJobHTML()}</div>${composing ? btComposeHTML() : ""}`;
  if (!BT.list.runs.length) {
    v.innerHTML = head + top;
    btFormPaint(); btObserveStrip();
    return;
  }
  const sm = BT.run.summary || {}, vd = VERDICT[sm.verdict] || ["", sm.verdict || "No verdict", "This run has no summary file."];
  const st = sm.strategy || {};
  const synthetic = sm.synthetic || BT.list.synthetic;
  const runs = BT.list.runs.map((r) => {
    const s = r.summary || {};
    const label = (r.kind === "scalp" ? "Scalper · " : "News · ") + (r.config ? r.config + " · " : "") + (s.from ? `${day(t(s.from))} – ${day(t(s.to))}` : r.id);
    return `<button class="chip" data-bt="run:${esc(r.id)}" aria-pressed="${r.id === BT.id}" title="${esc(r.id)}">${esc(label)}</button>`;
  }).join("");
  const ols = (r) => r?.ols ? `${esc(r.baseline)}: extra return per hour ${sgn(r.ols.alpha * 1e4, 2)} bps (t ${sgn(r.ols.t_alpha, 1)}), held in ${pct(r.stability, 0)} of the run's quarters.` : "";
  v.innerHTML = head + top + `
  ${synthetic && !S.snap?.synthetic ? `<div class="notice bad">${icon("warn")}<p><b>Demo run.</b> Every price, fill and figure in this backtest is invented.</p></div>` : ""}
  ${BT.list.runs.length > 1 ? `<div class="filters" role="group" aria-label="Backtest runs"><span class="lbl">Run</span>${runs}</div>` : ""}
  <section class="sheet btsheet">
    <div class="bttop"><h2>${esc(st.hours ? `${span(st.hours * HOUR)} replayed` : "Replay")}<span class="sm" style="margin-left:6px">${esc(sm.from ? `${when(Date.parse(sm.from))} → ${when(Date.parse(sm.to))}` : BT.id)}${BT.run.source?.config ? ` · ${esc(BT.run.source.config)}` : ""}</span></h2>
      <div class="filters" role="group" aria-label="Compare with"><span class="lbl">Compare</span>${SERIES.slice(1).map(([k, n]) => BT.ser[k].t.length ? `<button class="chip key-${k}" data-bt="show:${k}" aria-pressed="${BT.show[k]}"><i></i>${n}</button>` : "").join("")}</div></div>
    <div class="btread" id="bt-read" aria-live="polite"></div>
    <div class="btbar"><div class="filters" role="group" aria-label="Stretch"><span class="lbl">Stretch</span><button class="chip" data-bt="last:1">Last 24 h</button><button class="chip" data-bt="last:7">Last 7 days</button><button class="chip" data-bt="all">Whole run</button></div><button class="btn quiet" data-bt="zoom" id="bt-zoom"></button></div>
    <div class="btwrap" id="bt-wrap"><svg id="bt-svg" role="img"></svg><div class="bttip" id="bt-tip" hidden></div></div>
    <p class="bthint">Drag across the chart to measure any stretch. Double-click to see the whole run again. <span class="mk buy"></span> bought, <span class="mk sell"></span> sold; select a marker to find its trade below.</p>
  </section>
  <div class="tablewrap" id="bt-trades"></div>
  <section class="sheet"><h2>What the run says <span class="tag ${vd[0]}">${esc(vd[1])}</span></h2><p class="say">${esc(vd[2])}</p>
    ${sm.kind === "scalp" ? `<div class="readings" style="margin-top:12px"><div class="reading"><b class="${cls(st.total_return * 100)}">${spct(st.total_return, 2)}</b><small>return over the whole run</small></div><div class="reading"><b class="${cls(sm.hold_return * 100)}">${spct(sm.hold_return, 2)}</b><small>holding every coin instead</small></div><div class="reading"><b>${pct(st.max_drawdown, 2)}</b><small>worst drop from a high</small></div><div class="reading"><b>${st.trades ?? 0}</b><small>trades, ${st.trades ? pct(st.wins / st.trades, 0) : "—"} won</small></div><div class="reading"><b class="${cls(per1k(st.mean_bps))}">${susd(per1k(st.mean_bps))}</b><small>per $1,000 trade after fees (${sgn(st.mean_bps, 1)} bps, t ${sgn(st.t_stat, 2)})</small></div><div class="reading"><b>${usd(st.fees)}</b><small>fees paid</small></div></div>
    <ul class="assume">${(sm.assumptions || []).map((a) => `<li>${esc(a)}</li>`).join("")}</ul></section>` : `<div class="readings" style="margin-top:12px"><div class="reading"><b class="${cls(st.total_return * 100)}">${spct(st.total_return, 2)}</b><small>return over the whole run</small></div><div class="reading"><b>${pct(st.max_drawdown, 2)}</b><small>worst drop from a high</small></div><div class="reading"><b>${st.fills ?? "—"}</b><small>fills</small></div><div class="reading"><b>${usd(st.fees)}</b><small>fees paid</small></div><div class="reading"><b>${st.decision_hit_rate != null && st.decision_clusters ? pct(st.decision_hit_rate, 0) : "—"}</b><small>decisions right a day later (${st.decision_clusters ?? 0} separate ones)</small></div></div>
    ${sm.primary ? `<p class="say" style="margin-top:12px">${ols(sm.primary)} ${ols(sm.alternate)} A t above 2 is the bar for "not luck".</p>` : ""}</section>`}`;
  btPaint(); btFormPaint(); btObserveStrip();
  if (!btPaint.ro) { btPaint.ro = new ResizeObserver(() => { if (S.view === "backtest" && $("#bt-svg")) btChart(); }); }
  btPaint.ro.disconnect(); btPaint.ro.observe($("#bt-wrap"));
}

function btPaint() {
  const w = btWindow(), x = btStats(w);
  const z = $("#bt-zoom");
  z.innerHTML = BT.zoom ? "Zoom out" : "Zoom to stretch"; z.disabled = !BT.zoom && !BT.sel;
  $("#bt-read").innerHTML = `
    <div class="reading main"><span class="lbl">${BT.sel ? "This stretch" : "Whole view"}</span><b class="${cls(x.pnl)}">${susd(x.pnl)}</b><small><span class="${cls(x.ret * 100)}">${spct(x.ret, 2)}</span> · ${when(x.a)} → ${when(x.b)} · ${span(x.b - x.a)}</small></div>
    <div class="reading"><span class="lbl">${btCmp()[1] || "Compared with"}</span><b class="${cls(x.mom)}">${x.mom != null ? susd(x.mom) : "—"}</b><small>${btCmp()[0] === "hold" ? "same stretch, bought and held" : "same stretch, no news"}</small></div>
    <div class="reading"><span class="lbl">Worst drop</span><b>${pct(x.mdd, 2)}</b><small>from a high inside it</small></div>
    <div class="reading"><span class="lbl">Fills</span><b>${x.fills.length}</b><small>${susd(x.realized)} locked in · ${usd(x.fees)} fees</small></div>`;
  btChart();
  const rows = x.fills.slice(0, 400).map((f) => {
    const r = f.round_trip_pnl ?? f.realized_pnl;
    return `<tr class="click${BT.pick === f ? " open" : ""}" tabindex="0" data-fill="${BT.fills.indexOf(f)}"><td>${hm(t(f.ts))}<span class="sm">${day(t(f.ts))}</span></td><td><b>${esc(f.asset)}</b></td><td><span class="side ${f.side === "buy" ? "long" : "short"}">${esc(f.side)}</span></td><td class="n">${px(f.price)}</td><td class="n">${usd(f.notional, 0)}</td><td>${esc(f.reason)}</td><td class="n ${cls(r)}">${r != null ? susd(r) : "—"}</td><td class="n">${usd(BT.ser.strategy.v[f.i], 0)}</td></tr>`;
  }).join("");
  $("#bt-trades").innerHTML = `<table><caption>Trades in ${BT.sel ? "this stretch" : "view"}${x.fills.length > 400 ? `: first 400 of ${x.fills.length}` : ""}</caption><thead><tr><th>Time</th><th>Coin</th><th>Side</th><th class="n">Price</th><th class="n">Amount</th><th>Why</th><th class="n">Result after fees</th><th class="n">Account after</th></tr></thead><tbody>${rows || `<tr><td colspan="8" class="empty">No trades in this stretch.</td></tr>`}</tbody></table>`;
}

// One SVG: equity on top, P&L per bucket below, shared time axis. Drawn at the wrap's pixel width.
function btGeom() {
  const W = Math.max(280, $("#bt-wrap").clientWidth), narrow = W < 560;
  const s = BT.ser.strategy, d = BT.zoom || [s.t[0], s.t[s.t.length - 1]];
  const L = narrow ? 50 : 64, R = 12, top = 8, eqH = narrow ? 220 : 300, gap = 30, barH = narrow ? 64 : 84;
  const g = { W, L, R, d, eqT: top, eqB: top + eqH, barT: top + eqH + gap, barB: top + eqH + gap + barH };
  g.H = g.barB + 26;
  g.X = (tm) => L + ((tm - d[0]) / Math.max(1, d[1] - d[0])) * (W - L - R);
  g.T = (px) => d[0] + ((px - L) / (W - L - R)) * (d[1] - d[0]);
  return g;
}
function niceStep(range, n) { const raw = range / n, mag = 10 ** Math.floor(Math.log10(raw || 1)); return [1, 2, 2.5, 5, 10].map((k) => k * mag).find((k) => k >= raw); }
function btChart() {
  const svg = $("#bt-svg"); if (!svg) return;
  const g = btGeom(), { W, L, R, d, X } = g, s = BT.ser.strategy;
  const vis = (ser) => { const a = Math.max(0, lb(ser.t, d[0]) - 1), b = Math.min(ser.t.length, lb(ser.t, d[1] + 1) + 1); return [a, b]; };
  const shown = SERIES.filter(([k]) => k === "strategy" || (BT.show[k] && BT.ser[k].t.length));
  let lo = Infinity, hi = -Infinity;
  for (const [k] of shown) { const ser = BT.ser[k], [a, b] = vis(ser); for (let i = a; i < b; i++) { lo = Math.min(lo, ser.v[i]); hi = Math.max(hi, ser.v[i]); } }
  const pad = (hi - lo) * 0.08 || hi * 0.002 || 1; lo -= pad; hi += pad;
  const Y = (v) => g.eqB - ((v - lo) / (hi - lo)) * (g.eqB - g.eqT);
  const line = (ser) => { const [a, b] = vis(ser); let p = ""; for (let i = a; i < b; i++) p += (p ? "L" : "M") + X(ser.t[i]).toFixed(1) + "," + Y(ser.v[i]).toFixed(1); return p; };
  const out = [];
  // y grid
  const ys = niceStep(hi - lo, g.eqB - g.eqT < 250 ? 4 : 5);
  for (let v = Math.ceil(lo / ys) * ys; v <= hi; v += ys) out.push(`<line class="grid" x1="${L}" x2="${W - R}" y1="${Y(v)}" y2="${Y(v)}"/><text class="ax" x="${L - 8}" y="${Y(v) + 4}" text-anchor="end">${usd(v, ys < 1 ? 2 : 0)}</text>`);
  // x ticks
  const steps = [HOUR, 3 * HOUR, 6 * HOUR, 12 * HOUR, DAY, 2 * DAY, 7 * DAY, 14 * DAY, 30 * DAY];
  const xs = steps.find((k) => (d[1] - d[0]) / k <= (W - L - R) / 84) || 30 * DAY;
  const ticks = [];
  if (xs >= DAY) { const dt = new Date(d[0]); dt.setHours(0, 0, 0, 0); while (dt.getTime() <= d[1]) { ticks.push(dt.getTime()); dt.setDate(dt.getDate() + xs / DAY); } }
  else { const off = -new Date(d[0]).getTimezoneOffset() * 60e3; for (let tm = Math.floor((d[0] + off) / xs) * xs - off; tm <= d[1]; tm += xs) ticks.push(tm); }
  for (const tm of ticks) {
    if (tm < d[0]) continue;
    const xd = X(tm), dt = new Date(tm), lab = xs >= DAY || (dt.getHours() === 0 && dt.getMinutes() === 0) ? day(dt) : hm(dt);
    out.push(`<line class="grid" x1="${xd}" x2="${xd}" y1="${g.eqT}" y2="${g.barB}"/><text class="ax" x="${xd}" y="${g.H - 8}" text-anchor="middle">${lab}</text>`);
  }
  // the stretch: start-level reference and profit/loss shading between it and the curve
  const w = btWindow(), st = btStats(w), ry = Y(st.start), xa = X(st.a), xb = X(st.b);
  const i0 = lb(s.t, st.a), i1 = lb(s.t, st.b + 1) - 1;
  if (i1 > i0) {
    let p = `M${xa.toFixed(1)},${ry.toFixed(1)}`;
    for (let i = i0; i <= i1; i++) if (s.t[i] >= d[0] - HOUR && s.t[i] <= d[1] + HOUR) p += `L${X(s.t[i]).toFixed(1)},${Y(s.v[i]).toFixed(1)}`;
    p += `L${xb.toFixed(1)},${ry.toFixed(1)}Z`;
    out.push(`<clipPath id="bt-up"><rect x="0" y="0" width="${W}" height="${Math.max(0, ry)}"/></clipPath><clipPath id="bt-dn"><rect x="0" y="${ry}" width="${W}" height="${Math.max(0, g.eqB - ry)}"/></clipPath>`);
    out.push(`<path class="gain" d="${p}" clip-path="url(#bt-up)"/><path class="loss" d="${p}" clip-path="url(#bt-dn)"/>`);
    out.push(`<line class="ref" x1="${Math.max(L, xa)}" x2="${Math.min(W - R, xb)}" y1="${ry}" y2="${ry}"/>`);
  }
  for (const [k] of shown.slice().reverse()) out.push(`<path class="ln ${k}" d="${line(BT.ser[k])}"/>`);
  // P&L per bucket
  const span_ = d[1] - d[0], bk = span_ <= 4 * DAY ? HOUR : span_ <= 90 * DAY ? DAY : 7 * DAY;
  const bstart = (tm) => { const dt = new Date(tm); if (bk === HOUR) { dt.setMinutes(0, 0, 0); return dt.getTime(); } dt.setHours(0, 0, 0, 0); if (bk > DAY) dt.setDate(dt.getDate() - ((dt.getDay() + 6) % 7)); return dt.getTime(); };
  const bucketEnd = (b0) => { if (bk === HOUR) return b0 + HOUR; const dt = new Date(b0); dt.setDate(dt.getDate() + bk / DAY); return dt.getTime(); };
  const buckets = []; const [va, vb] = vis(s);
  let prev = va > 0 ? s.v[va - 1] : s.v[0], cur = null;
  for (let i = va; i < vb; i++) {
    const b0 = bstart(s.t[i] - 1); // an hour closing at midnight belongs to the day before
    if (!cur || cur.b0 !== b0) { if (cur) { prev = cur.last; } cur = { b0, base: prev, last: s.v[i] }; buckets.push(cur); }
    cur.last = s.v[i];
  }
  const maxAbs = Math.max(1e-9, ...buckets.map((b) => Math.abs(b.last - b.base)));
  const zy = (g.barT + g.barB) / 2, bh = (g.barB - g.barT) / 2;
  out.push(`<text class="ax lab" x="${L}" y="${g.barT - 10}">${bk === HOUR ? "Profit or loss each hour" : bk === DAY ? "Profit or loss each day" : "Profit or loss each week"}</text>`);
  if (maxAbs < 0.005) out.push(`<text class="ax" x="${(L + W - R) / 2}" y="${(g.barT + g.barB) / 2 - 6}" text-anchor="middle">No profit or loss in this view</text>`);
  else out.push(`<text class="ax" x="${L - 8}" y="${g.barT + 8}" text-anchor="end">${susd(maxAbs, 0)}</text><text class="ax" x="${L - 8}" y="${g.barB}" text-anchor="end">${susd(-maxAbs, 0)}</text>`);
  out.push(`<line class="zero" x1="${L}" x2="${W - R}" y1="${zy}" y2="${zy}"/>`);
  for (const b of buckets) {
    const pnl = b.last - b.base, x0 = Math.max(L, X(b.b0)), x1 = Math.min(W - R, X(bucketEnd(b.b0)));
    const bw = Math.max(1, x1 - x0 - (x1 - x0 > 4 ? 1.5 : 0)), hgt = Math.max(pnl ? 1 : 0, (Math.abs(pnl) / maxAbs) * bh);
    out.push(`<rect class="bar ${pnl >= 0 ? "pos" : "neg"}" x="${x0.toFixed(1)}" y="${(pnl >= 0 ? zy - hgt : zy).toFixed(1)}" width="${bw.toFixed(1)}" height="${hgt.toFixed(1)}" data-b0="${b.b0}" data-pnl="${pnl}"/>`);
  }
  // fills on the curve: bought below the line, sold above it
  for (const f of BT.fills) {
    if (f.tms < d[0] - HOUR || f.tms > d[1] + HOUR) continue;
    const fx = X(s.t[f.i]), fy = Y(s.v[f.i]), buy = f.side === "buy", picked = BT.pick === f;
    const r = picked ? 7 : 5, cy = buy ? fy + 10 + r : fy - 10 - r;
    const pts = buy ? `${fx},${cy - r} ${fx - r},${cy + r * 0.8} ${fx + r},${cy + r * 0.8}` : `${fx},${cy + r} ${fx - r},${cy - r * 0.8} ${fx + r},${cy - r * 0.8}`;
    out.push(`<polygon class="mk ${buy ? "buy" : "sell"}${picked ? " picked" : ""}" points="${pts}"/>`);
  }
  // dim everything outside the stretch
  if (BT.sel) {
    const a = Math.max(L, xa), b = Math.min(W - R, xb);
    if (a > L) out.push(`<rect class="dim" x="${L}" y="${g.eqT}" width="${a - L}" height="${g.barB - g.eqT}"/>`);
    if (b < W - R) out.push(`<rect class="dim" x="${b}" y="${g.eqT}" width="${W - R - b}" height="${g.barB - g.eqT}"/>`);
    out.push(`<line class="edge" x1="${a}" x2="${a}" y1="${g.eqT}" y2="${g.barB}"/><line class="edge" x1="${b}" x2="${b}" y1="${g.eqT}" y2="${g.barB}"/>`);
  }
  out.push(`<rect class="brush" id="bt-brush" y="${g.eqT}" height="${g.barB - g.eqT}" x="0" width="0" hidden/><line class="cross" id="bt-cross" y1="${g.eqT}" y2="${g.barB}" hidden/><circle class="dot" id="bt-dot" r="4" hidden/>`);
  svg.setAttribute("viewBox", `0 0 ${W} ${g.H}`); svg.setAttribute("width", W); svg.setAttribute("height", g.H);
  svg.setAttribute("aria-label", `Account value over the run, ${usd(s.v[0], 0)} to ${usd(s.v[s.v.length - 1], 0)}, with ${BT.fills.length} fills marked. The selected stretch made ${susd(st.pnl)}.`);
  svg.innerHTML = out.join("");
  btChart.g = { ...g, Y, buckets, bk, bstart };
}

function btNearFill(px, py) {
  const g = btChart.g, s = BT.ser.strategy; let best = null, bd = 10;
  for (const f of BT.fills) { const fx = g.X(s.t[f.i]); if (Math.abs(fx - px) > bd) continue; const fy = g.Y(s.v[f.i]) + (f.side === "buy" ? 15 : -15); const dd = Math.hypot(fx - px, fy - py); if (dd < bd) { bd = dd; best = f; } }
  return best;
}
function btPoint(e) { const r = $("#bt-svg").getBoundingClientRect(); return [(e.clientX - r.left) * (btChart.g.W / r.width), (e.clientY - r.top) * (btChart.g.H / r.height)]; }
function btHover(e) {
  const g = btChart.g; if (!g) return;
  const [mx, my] = btPoint(e), s = BT.ser.strategy, tip = $("#bt-tip"), cross = $("#bt-cross"), dot = $("#bt-dot");
  if (mx < g.L || mx > g.W - g.R) { tip.hidden = cross.hidden = dot.hidden = true; return; }
  if (BT.drag) {
    const a = Math.max(g.L, Math.min(BT.drag.x0, mx)), b = Math.min(g.W - g.R, Math.max(BT.drag.x0, mx)), br = $("#bt-brush");
    br.hidden = false; br.setAttribute("x", a); br.setAttribute("width", b - a);
    tip.hidden = false; tip.innerHTML = `<b>${span(Math.abs(g.T(b) - g.T(a)))}</b><span>Let go to measure this stretch.</span>`;
    return;
  }
  let i = Math.min(lb(s.t, g.T(mx)), s.t.length - 1); if (i > 0 && Math.abs(s.t[i - 1] - g.T(mx)) < Math.abs(s.t[i] - g.T(mx))) i--;
  const x = g.X(s.t[i]), y = g.Y(s.v[i]);
  cross.hidden = dot.hidden = false; cross.setAttribute("x1", x); cross.setAttribute("x2", x); dot.setAttribute("cx", x); dot.setAttribute("cy", y);
  const f = my < g.eqB ? btNearFill(mx, my) : null, w = btWindow();
  const inside = s.t[i] >= w[0] && s.t[i] <= w[1], from = inside ? w[0] : BT.zoom ? BT.zoom[0] : s.t[0];
  const base = s.v[Math.min(lb(s.t, from), s.t.length - 1)];
  let html;
  if (f) {
    const r = f.round_trip_pnl ?? f.realized_pnl;
    html = `<b>${f.side === "buy" ? "Bought" : "Sold"} ${esc(f.asset)} · ${when(f.tms)}</b><span>${usd(f.notional, 0)} at ${px(f.price)}</span><span>${esc(f.reason)}</span>${r != null ? `<span class="${r >= 0 ? "up" : "dn"}">${susd(r)} after fees</span>` : ""}`;
    $("#bt-svg").style.cursor = "pointer";
  } else if (my > g.barT - 4) {
    const b0 = g.bstart(s.t[i] - 1), b = g.buckets.find((x) => x.b0 === b0);
    const pnl = b ? b.last - b.base : null;
    html = `<b>${b ? (g.bk === HOUR ? when(b.b0) : g.bk === DAY ? new Date(b.b0).toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" }) : "Week of " + day(new Date(b.b0))) : ""}</b><span class="${pnl >= 0 ? "up" : "dn"}">${susd(pnl)}</span>`;
    $("#bt-svg").style.cursor = "";
  } else {
    const since = s.v[i] - base;
    html = `<b>${when(s.t[i])}</b><span>Account ${usd(s.v[i])}</span><span class="${since >= 0 ? "up" : "dn"}">${susd(since)} since ${inside && BT.sel ? when(from) : "the start of the view"}</span>`;
    $("#bt-svg").style.cursor = "";
  }
  tip.hidden = false; tip.innerHTML = html;
  const wrap = $("#bt-wrap").getBoundingClientRect(), tw = tip.offsetWidth, sx = (e.clientX - wrap.left);
  tip.style.left = Math.max(0, Math.min(wrap.width - tw, sx + (sx + tw + 16 > wrap.width ? -tw - 14 : 14))) + "px";
  tip.style.top = Math.max(0, (e.clientY - wrap.top) - 20) + "px";
}
document.addEventListener("pointerdown", (e) => {
  if (!e.target.closest("#bt-svg") || e.button !== 0) return;
  const [px] = btPoint(e); BT.drag = { x0: px, py: btPoint(e)[1] };
  $("#bt-svg").setPointerCapture(e.pointerId);
});
document.addEventListener("pointermove", (e) => { if (e.target.closest?.("#bt-svg") || BT.drag) btHover(e); });
document.addEventListener("pointerup", (e) => {
  if (!BT.drag) return;
  const g = btChart.g, [mx, my] = btPoint(e), x0 = BT.drag.x0; BT.drag = null;
  if (Math.abs(mx - x0) > 6) {
    const s = BT.ser.strategy, clamp = (tm) => Math.max(s.t[0], Math.min(s.t[s.t.length - 1], tm));
    const a = clamp(g.T(Math.max(g.L, Math.min(x0, mx)))), b = clamp(g.T(Math.min(g.W - g.R, Math.max(x0, mx))));
    if (b - a >= HOUR) { BT.sel = [a, b]; BT.pick = null; btPaint(); }
    else btChart();
    return;
  }
  const f = btNearFill(mx, my);
  if (f) { BT.pick = BT.pick === f ? null : f; btPaint(); document.querySelector(`#bt-trades tr[data-fill="${BT.fills.indexOf(f)}"]`)?.scrollIntoView({ block: "nearest", behavior: REDUCED ? "auto" : "smooth" }); }
});
document.addEventListener("pointerout", (e) => { if (e.target.id === "bt-svg" && !BT.drag && !e.relatedTarget?.closest?.("#bt-svg")) { for (const id of ["bt-tip", "bt-cross", "bt-dot"]) { const el = document.getElementById(id); if (el) el.hidden = true; } } });
document.addEventListener("dblclick", (e) => { if (e.target.closest("#bt-svg")) { BT.sel = BT.zoom = null; btPaint(); } });
function btAction(arg) {
  const [k, val] = arg.split(/:(.*)/s);
  const s = BT.ser?.strategy;
  if (k === "run") { btLoad(val); return; }
  if (k === "new" || k === "retry") { BT.compose = k === "retry" ? true : !BT.compose; BT.postErr = null; $("#view").dataset.sig = ""; backtestView(); if (BT.compose) $(".compose")?.scrollIntoView({ block: "nearest", behavior: REDUCED ? "auto" : "smooth" }); return; }
  if (k === "cancel") { BT.compose = false; $("#view").dataset.sig = ""; backtestView(); return; }
  if (k === "submit") { btSubmit(); return; }
  if (k === "preset") {
    const x = btFormStats(), end = x.max;
    BT.form.to = end;
    BT.form.from = val === "max" ? x.min : val === "first" ? (x.first < x.min ? x.min : x.first) : ymd(todayMs() - (Number(val) - 1) * DAY);
    if (BT.form.from >= BT.form.to) BT.form.from = ymd(dms(end) - DAY);
    BT.postErr = null; btFormPaint(); return;
  }
  if (k === "show") { BT.show[val] = !BT.show[val]; document.querySelector(`[data-bt="show:${val}"]`).setAttribute("aria-pressed", BT.show[val]); btChart(); return; }
  if (k === "all") { BT.sel = BT.zoom = null; }
  if (k === "last") { const end = s.t[s.t.length - 1]; BT.sel = [Math.max(s.t[0], end - Number(val) * DAY), end]; if (BT.zoom && (BT.sel[0] < BT.zoom[0])) BT.zoom = null; }
  if (k === "zoom") { if (BT.zoom) BT.zoom = null; else if (BT.sel) { BT.zoom = BT.sel; BT.sel = null; } }
  BT.pick = null; btPaint();
}

// ---------- new backtest: pick settings, a window on the headline strip, a name; run it here ----------
const MAX_BACK_DAYS = 205; // Hyperliquid serves ~5000 hourly candles
const ymd = (ms) => new Date(ms).toISOString().slice(0, 10);
const dms = (s) => Date.parse(s + "T00:00:00Z");
const todayMs = () => dms(ymd(Date.now()));
const dayLabel = (s) => new Date(dms(s)).toLocaleDateString([], { month: "short", day: "numeric", timeZone: "UTC" });
const BT_STEPS = {
  news: ["Fetch hourly prices and replay the strategy", "Replay momentum alone, with no news", "Replay the news alone, with no momentum", "Score it against momentum and holding BTC"],
  scalp: ["Fetch 15-minute candles from Hyperliquid", "Replay the scalper's rules bar by bar", "Score it against holding every coin"],
};
const btScalpPick = (srcs) => srcs.find((s) => s.file === "config.scalp.toml") || srcs.find((s) => /scalp/.test(s.file) && !/live/.test(s.file)) || srcs[0];

function btSource() { return (BT.list?.sources || []).find((s) => s.file === BT.form?.config); }
function btDefaultForm(strategy = "scalp") {
  const srcs = BT.list?.sources || [];
  const src = strategy === "scalp" ? btScalpPick(srcs) : srcs.slice().sort((a, b) => b.events - a.events)[0];
  const end = todayMs();
  return { strategy, config: src?.file || "", from: ymd(end - 29 * DAY), to: ymd(end), name: "" };
}
function btFormStats() {
  const f = BT.form, src = btSource(), a = dms(f.from), b = dms(f.to);
  const days = Object.entries(src?.days || {});
  const inside = days.filter(([d]) => d >= f.from && d <= f.to).reduce((x, [, n]) => x + n, 0);
  const first = days.length ? days[0][0] : null, last = days.length ? days[days.length - 1][0] : null;
  const nDays = Math.round((b - a) / DAY) + 1;
  const endMs = Math.min(b + DAY, Date.now() - HOUR);
  const hours = Math.max(0, Math.floor((endMs - a) / HOUR));
  const scalp = f.strategy === "scalp";
  const min = scalp && src?.scalp?.earliest ? ymd(Date.parse(src.scalp.earliest) + DAY) : ymd(todayMs() - MAX_BACK_DAYS * DAY), max = ymd(todayMs());
  let err = null;
  if (!f.config) err = "Pick the settings to test.";
  else if (!f.from || !f.to || isNaN(a) || isNaN(b)) err = "Pick a start and an end date.";
  else if (f.from < min) err = `Hyperliquid's ${scalp ? (src?.scalp?.interval || "15m") : "hourly"} candles only reach back to ${dayLabel(min)}${scalp ? " after the warm-up" : ""}. Start on or after that.`;
  else if (f.to > max) err = "The window can't end in the future.";
  else if (b < a) err = "The end date is before the start date.";
  else if (hours < 48) err = "The window needs at least two days of closed hours.";
  else if (f.name && !/^[A-Za-z0-9_-]{1,80}$/.test(f.name)) err = "Names use letters, digits, - and _ only.";
  const id = f.name || `${scalp ? "scalp_" : ""}${f.from.replaceAll("-", "")}_${(f.to >= max ? max : ymd(b + DAY)).replaceAll("-", "")}`;
  return { scalp, src, inside, first, last, nDays, hours, min, max, err, id, exists: (BT.list?.runs || []).some((r) => r.id === id) };
}

function btComposeHTML() {
  const f = BT.form, srcs = BT.list?.sources || [];
  const first = !BT.list?.runs?.length;
  const scalp = f.strategy === "scalp";
  const opt = (s) => {
    if (scalp) {
      const p = s.scalp || {};
      const what = `Fades ${p.interval} closes past ${p.z}σ of the last ${Math.round((p.bars * (p.interval === "15m" ? 15 : 60)) / 1440)} days' mean · ${p.hold_secs >= 3600 ? Math.round(p.hold_secs / 3600) + " h" : Math.round(p.hold_secs / 60) + " min"} hold, ${p.stop_bps} bps stop · ${pct(p.position_pct, 0)} per trade${p.shorts ? " · longs and shorts" : ""}`;
      return `<label class="opt"><input type="radio" name="bt-config" value="${esc(s.file)}" data-btf="config"${s.file === f.config ? " checked" : ""}><span><b>${esc(s.file)}</b><small>${esc(what)} · ${esc(s.assets.join(", "))}</small></span></label>`;
    }
    const ds = Object.keys(s.days || {});
    const cover = s.events ? `${s.events} cached headlines, ${dayLabel(ds[0])}${ds.length > 1 ? ` to ${dayLabel(ds[ds.length - 1])}` : ""}` : "No cached headlines yet";
    return `<label class="opt"><input type="radio" name="bt-config" value="${esc(s.file)}" data-btf="config"${s.file === f.config ? " checked" : ""}><span><b>${esc(s.file)}</b><small>${esc(cover)} · ${esc(s.assets.join(", "))} · reads <code>${esc(s.data_dir)}/</code></small></span></label>`;
  };
  // Settings with no cached headlines can only replay momentum: keep them one click away.
  const withNews = scalp ? srcs.filter((s) => /scalp/.test(s.file) || s.file === f.config) : srcs.filter((s) => s.events || s.file === f.config), without = srcs.filter((s) => !withNews.includes(s));
  const opts = srcs.length ? withNews.map(opt).join("") + (without.length ? `<details class="optmore"><summary>${icon("chev")}${without.length} more ${scalp ? "settings files" : "with no cached headlines"}</summary><div class="opts">${without.map(opt).join("")}</div></details>` : "") : `<p class="say">No settings files found next to the dashboard.</p>`;
  return `<section class="sheet compose" aria-labelledby="bt-new-h">
    <div class="compose-top"><h2 id="bt-new-h">${first ? "Run your first backtest" : "New backtest"}</h2>${first ? "" : `<button class="btn quiet" data-bt="cancel">Cancel</button>`}</div>
    <p class="say">Pick a strategy, its settings and a stretch of past days. It replays them on this machine: no orders, no Jev calls. Prices come from Hyperliquid's public API.</p>
    <div class="field"><span class="lbl" id="bt-l-kind">Strategy</span><div class="opts opts-2" role="radiogroup" aria-labelledby="bt-l-kind">
      <label class="opt"><input type="radio" name="bt-kind" value="scalp" data-btf="strategy"${scalp ? " checked" : ""}><span><b>Scalper</b><small>Its live entry and exit rules on 15-minute candles. Trades on any stretch of days; news plays no part.</small></span></label>
      <label class="opt"><input type="radio" name="bt-kind" value="news" data-btf="strategy"${scalp ? "" : " checked"}><span><b>News bot</b><small>Cached Jev answers plus hourly momentum. Only trades on days the bot was running and saved headlines.</small></span></label>
    </div></div>
    <div class="field"><span class="lbl" id="bt-l-cfg">Settings to test</span><div class="opts" role="radiogroup" aria-labelledby="bt-l-cfg">${opts}</div></div>
    <div class="field"><span class="lbl">Window</span>
      <div class="filters" role="group" aria-label="Window presets">${scalp ? "" : `<button class="chip" data-bt="preset:first">Since the first headline</button>`}<button class="chip" data-bt="preset:7">Last 7 days</button><button class="chip" data-bt="preset:30">Last 30 days</button>${scalp ? "" : `<button class="chip" data-bt="preset:90">Last 90 days</button>`}<button class="chip" data-bt="preset:max">As far back as ${scalp ? "candles" : "prices"} go</button></div>
      <div class="strip" id="bt-strip-wrap"><svg id="bt-strip" role="img"></svg></div>
      <div class="dates"><label><span class="lbl">From</span><input type="date" data-btf="from" value="${f.from}"></label><label><span class="lbl">Through</span><input type="date" data-btf="to" value="${f.to}"></label></div>
      <p class="say" id="bt-plan" aria-live="polite"></p>
    </div>
    <div class="field"><label class="lbl" for="bt-name">Name <span class="opt-note">optional</span></label><input id="bt-name" class="text" data-btf="name" value="${esc(f.name)}" autocomplete="off" spellcheck="false"><p class="say" id="bt-name-note"></p></div>
    <div class="compose-foot"><button class="btn ink" data-bt="submit" id="bt-go">${icon("play")}Run backtest</button><p class="err" id="bt-err" role="alert"></p></div>
  </section>`;
}

function btFormPaint() {
  if (!$("#bt-strip")) return;
  const f = BT.form, x = btFormStats();
  for (const el of document.querySelectorAll("[data-btf=from],[data-btf=to]")) { el.min = x.min; el.max = x.max; if (document.activeElement !== el && el.value !== f[el.dataset.btf]) el.value = f[el.dataset.btf]; }
  $("#bt-name").placeholder = x.id;
  const preset = { first: x.first && f.from === (x.first < x.min ? x.min : x.first) && f.to === x.max, max: f.from === x.min && f.to === x.max };
  for (const n of [7, 30, 90]) preset[n] = f.to === x.max && f.from === ymd(todayMs() - (n - 1) * DAY);
  for (const b of document.querySelectorAll("[data-bt^='preset:']")) { const k = b.dataset.bt.slice(7); b.setAttribute("aria-pressed", !!preset[k]); if (k === "first") b.disabled = !x.first; }
  // what this run will be, in sentences
  const notes = [];
  if (!x.err && x.scalp) {
    const p = x.src?.scalp || {};
    notes.push(`<b>${x.nDays} days</b> of ${p.interval} candles for ${x.src?.assets.length} coins (${esc(x.src?.assets.join(", "))}), after a ${Math.round((p.bars * 15) / 1440)}-day warm-up before the start. Every trade the rules would take is made, compared with holding every coin.`);
    notes.push(`Costs: taker fee ${p.fee_bps} bps and ${p.slippage_bps} bps slippage each way, plus an assumed 2 bps spread. No funding, no Jev review.`);
  } else if (!x.err) {
    notes.push(`<b>${x.nDays} days</b>, about ${x.hours.toLocaleString("en-US")} hours, replayed three ways: the strategy, momentum alone and news alone.`);
    if (!x.inside) notes.push(`<span class="warnline">${icon("warn")}No cached headlines fall in this window, so the strategy can only follow momentum. Pick days on the strip where headlines exist.</span>`);
    else {
      notes.push(`${x.inside} cached headline${x.inside === 1 ? "" : "s"} fall inside.`);
      if (x.first > f.from) notes.push(`Before ${dayLabel(x.first)} there are none, so on those days it trades on momentum alone.`);
    }
    if (x.inside && (x.nDays < 7 || x.inside < 20)) notes.push(`<span class="warnline">${icon("info")}The verdict needs at least a week and 20 separate decisions. Expect "Too little evidence".</span>`);
  }
  $("#bt-plan").innerHTML = notes.join(" ");
  $("#bt-name-note").innerHTML = x.exists ? `<span class="warnline">${icon("warn")}A run named <b>${esc(x.id)}</b> exists. Running replaces it.</span>` : f.name ? "" : "Leave it empty to name the run after its dates.";
  const running = BT.list?.job?.state === "running";
  $("#bt-go").disabled = !!x.err || running || BT.posting;
  $("#bt-err").textContent = BT.postErr || (running ? "Another backtest is running. Wait for it to finish." : x.err || "");
  btStrip(x);
}

// The strip: every day prices exist for, a bar per day with cached headlines, the chosen window on top. Drag to choose.
function btStrip(x) {
  const svg = $("#bt-strip"), W = Math.max(280, $("#bt-strip-wrap").clientWidth), H = 72, L = 0, R = 0, top = 6, base = 46;
  // Span: back to the earlier of the window start and the first headline, plus a margin; at least 30 days, at most what prices allow.
  const end = dms(x.max) + DAY, early = x.scalp ? dms(x.min) : Math.min(dms(BT.form.from) || end, x.first ? dms(x.first) : end);
  // While dragging, the scale holds still under the pointer.
  const d0 = BT.sdrag && btStrip.X ? btStrip.X.d0 : Math.max(dms(x.min), Math.min(end - 30 * DAY, early - Math.max(7 * DAY, (end - early) * 0.25))), d1 = end;
  const X = (ms) => L + ((ms - d0) / (d1 - d0)) * (W - L - R);
  const days = x.scalp ? [] : Object.entries(x.src?.days || {}).filter(([d]) => dms(d) >= d0);
  const maxN = Math.max(1, ...days.map(([, n]) => n)), bw = Math.max(2, (W - L - R) / ((d1 - d0) / DAY) - 1);
  const out = [`<rect class="track" x="${L}" y="${base}" width="${W - L - R}" height="1"/>`];
  const spanDays = (d1 - d0) / DAY, step = spanDays > 100 ? 0 : spanDays > 45 ? 14 : 7;
  const ticks = [];
  if (!step) { const m = new Date(d0); m.setUTCDate(1); m.setUTCMonth(m.getUTCMonth() + 1); for (; m.getTime() < d1; m.setUTCMonth(m.getUTCMonth() + 1)) ticks.push([m.getTime(), m.toLocaleDateString([], { month: "short", timeZone: "UTC" })]); }
  else for (let tm = d1 - DAY; tm > d0; tm -= step * DAY) ticks.push([tm, dayLabel(ymd(tm))]);
  for (const [tm, lab] of ticks) { const xm = X(tm); if (xm > W - 40) continue; out.push(`<line class="tick" x1="${xm}" x2="${xm}" y1="${base}" y2="${base + 5}"/><text class="ax" x="${xm + 3}" y="${H - 6}">${lab}</text>`); }
  out.push(`<text class="ax" x="${W - 2}" y="${H - 6}" text-anchor="end">Today</text>`);
  if (!x.err || dms(BT.form.to) >= dms(BT.form.from)) {
    const a = Math.max(L, X(dms(BT.form.from))), b = Math.min(W - R, X(dms(BT.form.to) + DAY));
    if (b > a) out.push(`<rect class="win" x="${a}" y="${top - 4}" width="${b - a}" height="${base - top + 8}" rx="3"/>`);
  }
  for (const [d, n] of days) { const h = Math.max(3, Math.sqrt(n / maxN) * (base - top - 4)); out.push(`<rect class="hd" x="${X(dms(d)).toFixed(1)}" y="${(base - h).toFixed(1)}" width="${bw.toFixed(1)}" height="${h.toFixed(1)}"><title>${dayLabel(d)}: ${n} headlines</title></rect>`); }
  if (x.scalp) out.push(`<text class="ax" x="${W / 2}" y="${base - 14}" text-anchor="middle">Candles reach back to ${dayLabel(x.min)}. Drag to pick days.</text>`);
  else if (!days.length) out.push(`<text class="ax" x="${W / 2}" y="${base - 14}" text-anchor="middle">No cached headlines for these settings yet</text>`);
  svg.setAttribute("viewBox", `0 0 ${W} ${H}`); svg.setAttribute("width", W); svg.setAttribute("height", H);
  svg.setAttribute("aria-label", `Days with cached headlines since ${dayLabel(ymd(d0))}; the chosen window is ${dayLabel(BT.form.from)} to ${dayLabel(BT.form.to)}. Drag to choose a window, or use the date fields.`);
  svg.innerHTML = out.join("");
  btStrip.X = { d0, d1, W, L, R };
}
function btStripDay(e) { const g = btStrip.X, r = $("#bt-strip").getBoundingClientRect(); const px_ = (e.clientX - r.left) * (g.W / r.width); const ms = g.d0 + ((px_ - g.L) / (g.W - g.L - g.R)) * (g.d1 - g.d0); return ymd(Math.max(g.d0, Math.min(g.d1 - DAY, Math.floor(ms / DAY) * DAY))); }
document.addEventListener("pointerdown", (e) => { if (!e.target.closest("#bt-strip") || e.button !== 0) return; BT.sdrag = btStripDay(e); BT.form.from = BT.form.to = BT.sdrag; $("#bt-strip").setPointerCapture(e.pointerId); btFormPaint(); });
document.addEventListener("pointermove", (e) => { if (!BT.sdrag) return; const d = btStripDay(e); [BT.form.from, BT.form.to] = d < BT.sdrag ? [d, BT.sdrag] : [BT.sdrag, d]; btFormPaint(); });
document.addEventListener("pointerup", () => { if (BT.sdrag) { BT.sdrag = null; BT.postErr = null; btFormPaint(); } });
document.addEventListener("input", (e) => { const k = e.target.dataset?.btf; if (!k || !BT.form) return;
  if (k === "strategy") { const keep = { from: BT.form.from, to: BT.form.to, name: BT.form.name }; BT.form = { ...btDefaultForm(e.target.value), ...keep }; BT.postErr = null; $("#view").dataset.sig = ""; backtestView(); $(".compose")?.scrollIntoView({ block: "nearest" }); return; } BT.form[k] = k === "name" ? e.target.value.trim() : e.target.value; BT.postErr = null; btFormPaint(); });

async function btSubmit() {
  const x = btFormStats(); if (x.err || BT.posting) return;
  const f = BT.form, body = { strategy: f.strategy, config: f.config, from: f.from, name: f.name || null, to: f.to >= x.max ? null : ymd(dms(f.to) + DAY) };
  BT.posting = true; btFormPaint();
  try {
    const r = await fetch("/api/backtests", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
    if (!r.ok) throw new Error((await r.text()) || "HTTP " + r.status);
    BT.list.job = { ...(await r.json()), stage: 0 }; BT.compose = false; BT.postErr = null;
  } catch (e) { BT.postErr = "Couldn't start it: " + (e.message || e); }
  BT.posting = false;
  $("#view").dataset.sig = ""; backtestView();
}

function btJobHTML() {
  const j = BT.list?.job; if (!j) return "";
  const el = ((j.finished_at ? Date.parse(j.finished_at) : Date.now()) - Date.parse(j.started_at)) / 1000;
  const win = `${dayLabel(j.from)} to ${j.to ? dayLabel(ymd(dms(j.to) - DAY)) : "now"} with ${esc(j.config)}`;
  if (j.state === "failed") return `<div class="notice bad">${icon("warn")}<p><b>Backtest ${esc(j.id)} failed</b> after ${dur(el)}: ${esc(j.error)}. <button class="btn quiet" data-bt="retry">Edit and try again</button></p></div>`;
  if (j.state === "done") {
    if (BT.id === j.id) return "";
    const vd = VERDICT[j.verdict] || ["", j.verdict || ""];
    return `<div class="notice">${icon("check")}<p><b>Backtest ${esc(j.id)} finished</b> in ${dur(el)} · <span class="tag ${vd[0]}">${esc(vd[1])}</span> <button class="btn quiet" data-bt="run:${esc(j.id)}">Open it</button></p></div>`;
  }
  const steps = (BT_STEPS[j.kind] || BT_STEPS.news).map((s, i) => { const st = i < j.stage ? "ok" : i === j.stage ? "now" : "skip"; return `<li class="${st}"><span class="mk">${icon(st === "ok" ? "check" : st === "now" ? "dash" : "dash")}</span><span>${s}</span><span class="raw">${st === "now" ? "working" : st === "ok" ? "done" : ""}</span></li>`; }).join("");
  return `<section class="sheet jobsheet" aria-live="polite"><div class="compose-top"><h2>Running ${esc(j.id)}</h2><span class="elapsed" id="bt-elapsed">${dur(el)}</span></div><p class="say">${win}. You can leave this page; it keeps running in the dashboard process.</p><ul class="checks">${steps}</ul></section>`;
}
async function btPoll() {
  const was = BT.list?.job;
  try { const r = await fetch("/api/backtests"); if (!r.ok) return; const l = await r.json(); BT.list = l; } catch { return; }
  const j = BT.list.job;
  if (was?.state === "running" && j?.state === "done") { BT.run = null; await btLoad(j.id); return; }
  const box = $("#bt-job");
  if (box && S.view === "backtest") { if (was?.state !== j?.state) { $("#view").dataset.sig = ""; backtestView(); } else box.innerHTML = btJobHTML(); }
}
setInterval(() => { const e = $("#bt-elapsed"), j = BT.list?.job; if (e && j?.state === "running") e.textContent = dur((Date.now() - Date.parse(j.started_at)) / 1000); }, 1000);

function btObserveStrip() {
  const w = $("#bt-strip-wrap"); if (!w) return;
  if (!btObserveStrip.ro) btObserveStrip.ro = new ResizeObserver(() => { if ($("#bt-strip")) btFormPaint(); });
  btObserveStrip.ro.disconnect(); btObserveStrip.ro.observe(w);
}
// ---------- scalper page: React + htm from pinned CDN builds (SRI-checked), styled by app.css in the board's own tokens ----------
// (Tailwind's CDN sends no CORS headers, so its integrity can't be checked; it isn't loaded.)
// Loaded only when the Scalper page opens. If a script can't load, the page falls back to the plain view below.
const SC_LIBS = [
  ["https://unpkg.com/react@18.3.1/umd/react.production.min.js", "sha384-DGyLxAyjq0f9SPpVevD6IgztCFlnMF6oW/XQGmfe+IsZ8TqEiDrcHkMLKI6fiB/Z"],
  ["https://unpkg.com/react-dom@18.3.1/umd/react-dom.production.min.js", "sha384-gTGxhz21lVGYNMcdJOyq01Edg0jhn/c22nsx0kyqP0TxaV5WVdsSH1fSDUf5YJj1"],
  ["https://unpkg.com/htm@3.1.1/dist/htm.umd.js", "sha384-toVdrLSMaw7Y55MowcKqkmFL/Ek6Sky62NOk0b5sDDZBu2wcoPyyQUt9unDVjXhL"],
];
const SC = { state: "idle", root: null, App: null, err: null };
function scLoad() {
  if (SC.state !== "idle") return;
  SC.state = "loading";
  SC_LIBS.reduce((p, [src, sri]) => p.then(() => new Promise((ok, no) => {
    const s = document.createElement("script");
    s.src = src; s.integrity = sri; s.crossOrigin = "anonymous"; s.onload = ok; s.onerror = () => no(new Error("couldn't load " + new URL(src).host + new URL(src).pathname));
    document.head.appendChild(s);
  })), Promise.resolve())
    .then(() => {
      SC.App = scDefine(); SC.state = "ready";
    })
    .catch((e) => { SC.state = "failed"; SC.err = String(e.message || e); })
    .finally(() => { if (S.view === "scalper") render(); });
}

function scalperView() {
  const v = $("#view");
  if (SC.state === "failed") {
    scalperViewLegacy();
    v.insertAdjacentHTML("afterbegin", `<div class="notice warn">${icon("warn")}<p><b>Showing the simple view.</b> The page's components ${esc(SC.err)}; it needs internet access to unpkg.com.</p></div>`);
    return;
  }
  if (SC.state !== "ready") {
    scLoad();
    if (v.dataset.view !== "scalper-loading") { v.dataset.view = "scalper-loading"; v.innerHTML = `<header class="head"><h1>Scalper</h1></header><div class="sheet empty">Loading…</div>`; }
    return;
  }
  if (!SC.root || !document.getElementById("sc-root")) {
    v.dataset.view = "scalper-r"; v.dataset.sig = "";
    v.innerHTML = `<div id="sc-root"></div>`;
    SC.root = ReactDOM.createRoot(document.getElementById("sc-root"));
  }
  SC.root.render(React.createElement(SC.App, { sc: S.scalp, snapAt: Date.now() }));
}

// Open trade value right now: price move on the size, less the entry fee, plus funding.
function scOpenPnl(a) {
  const p = a?.position, mid = a?.features?.mid;
  if (!p || !mid) return null;
  return (p.side === "short" ? -1 : 1) * (mid - p.entry_px) * p.qty - (p.entry_fee || 0) + (p.funding_usd || 0);
}
function scHowHTML(sc) {
  const sum = sc.summary || {}, s = sc.status.settings || {}, blocked = sum.blocked_by || {};
  let left = sum.candidates || 0;
  const order = [...MARKET_GATES.filter((g) => g !== "warm-up"), ...RISK_GATES];
  const items = order.map((g, i) => {
    const n = blocked[g] || 0, [name, say] = gateInfo(g);
    const before = left; left -= n;
    return `<li><span class="num">${i + 1}</span><div><h4>${esc(name)}</h4><p>${esc(say)}</p></div><div class="cnt"><b class="${n ? "neg" : ""}">${n ? MINUS + n : "0"}</b><span>${before} reached it</span></div></li>`;
  }).join("");
  return `<section class="sheet"><h2>How it decides</h2><p class="say" style="max-width:74ch">A chance starts when the market checks below pass for one side. Since ${hm(t(sum.session_start))} it found <b>${sum.candidates ?? 0}</b> chances that got past warm-up. Each check stopped some; <b>${blocked.taken ?? 0}</b> made it through every one and were placed as orders. Then it exits at the target, the stop (which trails to break-even once profit covers ${s.trail_after_cost_mult ?? 1.5}× the cost), when the flow reverses, on a hack alert, or after ${Math.round((s.hold_secs || 600) / 60)} minutes.</p><ol class="sieve" style="margin-top:12px">${items}</ol></section>`;
}

function scDefine() {
  const { useState, useMemo, useEffect, useRef } = React;
  const html = htm.bind(React.createElement);
  const Icon = ({ n, cls = "" }) => html`<svg class=${"i " + cls} viewBox="0 0 24 24" aria-hidden="true" dangerouslySetInnerHTML=${{ __html: ICON[n] }} />`;
  const Money = ({ x, d = 2, plain }) => html`<span class=${plain ? "" : cls(x)}>${plain ? usd(x, d) : susd(x, d)}</span>`;
  const TONE = { green: "ok", amber: "wait", red: "no" };
  const TIME = [["all", "All"], ["1h", "Last hour"], ["6h", "Last 6 h"], ["today", "Today"]];
  const since = (k) => { if (k === "1h") return Date.now() - HOUR; if (k === "6h") return Date.now() - 6 * HOUR; if (k === "today") { const d = new Date(); d.setHours(0, 0, 0, 0); return d.getTime(); } return 0; };
  const OUTCOME = (c) => (c.taken ? "Taken" : c.blocked_by === "jev review" ? "Jev vetoed" : c.blocked_by === "cost" ? "Too small for fees" : ["cooldown", "trades/hour", "max positions", "daily limit", "macro", "flat"].includes(c.blocked_by) ? "Held by limits" : "Market said no");

  // Sortable column head (shadcn data-table pattern).
  function Th({ k, sort, setSort, n, children, title }) {
    const on = sort.k === k;
    return html`<th class=${n ? "n" : ""} aria-sort=${on ? (sort.dir > 0 ? "ascending" : "descending") : "none"} title=${title}>
      <button class="sc-sort" onClick=${() => setSort({ k, dir: on ? -sort.dir : n ? -1 : 1 })}>${children}<${Icon} n="chev" cls=${"sc-caret " + (on ? (sort.dir > 0 ? "up" : "down") : "idle")} /></button></th>`;
  }
  function useSorted(rows, sort, get) {
    return useMemo(() => { const r = rows.slice(); r.sort((a, b) => { const x = get(a, sort.k), y = get(b, sort.k); return (x < y ? -1 : x > y ? 1 : 0) * sort.dir; }); return r; }, [rows, sort.k, sort.dir]);
  }

  // Faceted filter: dashed button, popover of options with counts (shadcn data-table faceted filter).
  function Facet({ title, options, value, onChange }) {
    const [open, setOpen] = useState(false), ref = useRef(null);
    useEffect(() => {
      if (!open) return;
      const close = (e) => { if (!ref.current?.contains(e.target)) setOpen(false); };
      const esc_ = (e) => { if (e.key === "Escape") { setOpen(false); ref.current?.querySelector("button")?.focus(); } };
      document.addEventListener("pointerdown", close); document.addEventListener("keydown", esc_);
      return () => { document.removeEventListener("pointerdown", close); document.removeEventListener("keydown", esc_); };
    }, [open]);
    const sel = new Set(value);
    const toggle = (v) => { const s = new Set(sel); s.has(v) ? s.delete(v) : s.add(v); onChange([...s]); };
    const chosen = options.filter((o) => sel.has(o.value));
    return html`<div class="sc-facet-wrap" ref=${ref}>
      <button class=${"sc-facet" + (chosen.length ? " on" : "")} aria-haspopup="true" aria-expanded=${open} onClick=${() => setOpen(!open)}>
        <${Icon} n="plus" />${title}
        ${chosen.length ? html`<span class="sc-sep"></span>${chosen.length > 2 ? html`<span class="sc-pill">${chosen.length} selected</span>` : chosen.map((o) => html`<span class="sc-pill" key=${o.value}>${o.label}</span>`)}` : null}
      </button>
      ${open ? html`<div class="sc-pop" role="menu" aria-label=${title}>
        ${options.map((o) => html`<button key=${o.value} role="menuitemcheckbox" aria-checked=${sel.has(o.value)} class="sc-opt" onClick=${() => toggle(o.value)}>
          <span class="sc-box">${sel.has(o.value) ? html`<${Icon} n="check" />` : null}</span><span class="sc-opt-l">${o.label}</span><span class="sc-count">${o.count}</span></button>`)}
        ${chosen.length ? html`<button class="sc-opt sc-clear" onClick=${() => onChange([])}>Clear ${title.toLowerCase()}</button>` : null}
      </div>` : null}
    </div>`;
  }
  const count = (rows, f) => { const m = new Map(); for (const r of rows) { const k = f(r); m.set(k, (m.get(k) || 0) + 1); } return m; };
  const opts = (rows, f, label = (x) => x) => [...count(rows, f)].sort((a, b) => b[1] - a[1]).map(([v, n]) => ({ value: v, label: label(v), count: n }));

  function Account({ sc, coinRows }) {
    const st = sc.status, m = scalpMoney(), openPnl = coinRows.reduce((x, r) => x + (r.open || 0), 0);
    const nOpen = coinRows.filter((r) => r.a?.position).length, start = st.starting_cash ?? st.capital;
    const s = st.settings || {};
    return html`<section class="sheet sc-acct">
      <div class="sc-acct-top"><h2>Account ${m ? html`<span class=${"venue " + m.kind}>${m.label}</span>` : null}</h2><p class="say">${m?.say}</p></div>
      <div class="sc-read">
        <div><span class="lbl">Balance</span><b class="sc-fig">${usd(st.equity)}</b><small>Started with ${usd(start, 0)}: <${Money} x=${st.equity - start} /> (${spct(st.equity / start - 1, 2)})</small></div>
        <div><span class="lbl">Closed trades</span><b><${Money} x=${st.realized} /></b><small>${(st.wins || 0) + (st.losses || 0)} trades, ${st.wins || 0} won (${pct((st.wins || 0) / Math.max(1, (st.wins || 0) + (st.losses || 0)), 0)})</small></div>
        <div><span class="lbl">Open now</span><b><${Money} x=${openPnl} /></b><small>${nOpen} ${nOpen === 1 ? "trade" : "trades"}, ${usd(st.open_notional, 0)} at work</small></div>
        <div><span class="lbl">Today</span><b><${Money} x=${st.day_trade_pnl} /></b><small>Stops for the day at ${MINUS}${pct(s.daily_loss_limit_pct ?? 0.02, 0)} (${spct(st.day_pnl_pct / 100, 2)} now)</small></div>
      </div></section>`;
  }

  function Coins({ coinRows, picked, setPicked }) {
    const [sort, setSort] = useState({ k: "net", dir: -1 });
    const get = (r, k) => (k === "coin" ? r.coin : k === "now" ? r.statusRank : k === "net" ? r.net : k === "closed" ? r.closed : k === "open" ? r.open ?? 0 : k === "n" ? r.n : k === "won" ? (r.n ? r.wins / r.n : -1) : k === "avg" ? (r.n ? r.bps / r.n : -1e9) : k === "fees" ? r.fees : r.last || "");
    const rows = useSorted(coinRows, sort, get);
    const maxAbs = Math.max(1, ...coinRows.map((r) => Math.abs(r.net)));
    const toggle = (c) => setPicked(picked.includes(c) ? picked.filter((x) => x !== c) : [...picked, c]);
    return html`<section class="sc-block">
      <div class="sc-head"><h2>Coins</h2><p class="say">What each coin has made or lost, closed and open. Select coins to filter the trades and decisions below.</p>
        ${picked.length ? html`<button class="btn quiet" onClick=${() => setPicked([])}>Show all coins</button>` : null}</div>
      <div class="tablewrap"><table class="sc-coins">
        <thead><tr>
          <${Th} k="coin" sort=${sort} setSort=${setSort}>Coin<//><${Th} k="now" sort=${sort} setSort=${setSort}>Now<//>
          <${Th} k="net" sort=${sort} setSort=${setSort} n title="Closed results plus the open trade's value now, after fees">Balance<//>
          <${Th} k="closed" sort=${sort} setSort=${setSort} n>Closed<//><${Th} k="open" sort=${sort} setSort=${setSort} n>Open<//>
          <${Th} k="n" sort=${sort} setSort=${setSort} n>Trades<//><${Th} k="won" sort=${sort} setSort=${setSort} n>Won<//>
          <${Th} k="avg" sort=${sort} setSort=${setSort} n title="Average result per trade, scaled to a $1,000 trade">Per $1k trade<//>
          <${Th} k="fees" sort=${sort} setSort=${setSort} n>Fees<//><${Th} k="last" sort=${sort} setSort=${setSort}>Last closed<//>
        </tr></thead>
        <tbody>${rows.map((r) => {
          const on = picked.includes(r.coin), w = (Math.abs(r.net) / maxAbs) * 50;
          return html`<tr key=${r.coin} class=${"sc-row" + (on ? " picked" : "")} onClick=${() => toggle(r.coin)}>
            <td><button class="sc-coin" aria-pressed=${on} onClick=${(e) => { e.stopPropagation(); toggle(r.coin); }}><span class="sc-check">${on ? html`<${Icon} n="check" />` : null}</span><b>${r.coin}</b></button></td>
            <td><span class=${"tag " + (TONE[r.tone] || "")}>${r.status.toLowerCase()}</span>${r.a?.position ? html`<span class=${"side " + r.a.position.side} style=${{ marginLeft: "6px" }}>${r.a.position.side}</span>` : null}</td>
            <td class="n"><div class="sc-bal"><span class="sc-track"><i class=${r.net >= 0 ? "up" : "dn"} style=${{ width: w + "%", [r.net >= 0 ? "left" : "right"]: "50%" }}></i></span><b class=${cls(r.net)}>${susd(r.net)}</b></div></td>
            <td class="n"><${Money} x=${r.closed} /></td>
            <td class="n">${r.open == null ? html`<span class="low">—</span>` : html`<${Money} x=${r.open} />`}</td>
            <td class="n">${r.n || html`<span class="low">0</span>`}${r.n ? html`<span class="sm">${r.long_n || 0} long · ${r.short_n || 0} short</span>` : null}</td>
            <td class=${"n " + (r.n && r.n < 10 ? "low" : "")}>${r.n ? pct(r.wins / r.n, 0) : "—"}</td>
            <td class="n">${r.n ? html`<${Money} x=${per1k(r.bps / r.n)} />` : html`<span class="low">—</span>`}</td>
            <td class="n">${r.n ? usd(r.fees) : html`<span class="low">—</span>`}</td>
            <td>${r.last ? html`${hm(t(r.last))}<span class="sm">${day(t(r.last))}</span>` : html`<span class="low">never</span>`}</td>
          </tr>`;
        })}</tbody>
      </table></div>
      <p class="say sc-foot">Grey win rates rest on fewer than 10 trades. Balance counts fees and funding; the open part changes with the price.</p>
    </section>`;
  }

  function OpenNow({ sc }) {
    const st = sc.status, open = st.assets.filter((a) => a.position), resting = st.assets.filter((a) => a.resting);
    return html`<section class="sc-block">
      <div class="sc-head"><h2>Open now</h2><p class="say">${open.length ? `${open.length} open ${open.length === 1 ? "trade" : "trades"}${resting.length ? `, ${resting.length} waiting ${resting.length === 1 ? "order" : "orders"}` : ""}. Each closes at its target, its stop, or the time limit.` : resting.length ? "" : "Nothing open. It's watching for a rush of trading."}</p></div>
      ${open.length || resting.length ? html`<div class="sheets">${open.map((a) => {
        const p = a.position, mid = a.features?.mid, u = scOpenPnl(a), left = (p.deadline_ms - Date.now()) / 1000;
        const lo = Math.min(p.sl_px, p.tp_px, mid ?? p.entry_px), hi = Math.max(p.sl_px, p.tp_px, mid ?? p.entry_px), X = (x) => ((x - lo) / Math.max(1e-12, hi - lo)) * 100;
        return html`<section class="sheet sc-pos" key=${a.symbol}>
          <div class="sc-pos-top"><h3>${a.symbol} <span class=${"side " + p.side}>${p.side}</span></h3><span class="say">${dur(left)} left at most</span></div>
          <p class=${"sc-fig " + cls(u)}>${susd(u)}<small> so far, after the entry fee</small></p>
          <div class="ruler" style=${{ "--e": X(p.entry_px) + "%" }}><span class="bar"></span><span class="tick stop" style=${{ left: X(p.sl_px) + "%" }}></span><span class="tick" style=${{ left: X(p.entry_px) + "%" }}></span><span class="tick tp" style=${{ left: X(p.tp_px) + "%" }}></span>${mid ? html`<span class="now" style=${{ left: X(mid) + "%" }}></span>` : null}</div>
          <div class="ruler-l"><span>stop <b>${px(p.sl_px)}</b></span><span>entry <b>${px(p.entry_px)}</b> · now <b>${px(mid)}</b></span><span>target <b>${px(p.tp_px)}</b></span></div>
          <p class="say" style=${{ marginTop: "10px" }}>${usd(Math.abs(p.entry_cash ?? p.qty * p.entry_px), 0)} ${p.side === "short" ? "sold short" : "bought"} at ${hm(new Date(p.opened_ms))}${p.funding_usd ? `, funding ${susd(p.funding_usd)}` : ""}.</p>
        </section>`;
      })}${resting.map((a) => html`<section class="sheet sc-pos" key=${"r" + a.symbol}><div class="sc-pos-top"><h3>${a.symbol} <span class=${"side " + a.resting.side}>${a.resting.side}</span></h3><span class="tag wait">order waiting</span></div><p class="say">Resting at ${px(a.resting.px)} for the cheaper fee, waiting for a ${a.resting.side === "short" ? "buyer" : "seller"} to fill it.</p></section>`)}</div>` : null}
    </section>`;
  }

  function Activity({ sc, picked, setPicked }) {
    const [tab, setTab] = useState("trades");
    const [q, setQ] = useState(""), [time, setTime] = useState("all");
    const [side, setSide] = useState([]), [result, setResult] = useState([]), [reason, setReason] = useState([]), [outcome, setOutcome] = useState([]);
    const [open, setOpen] = useState(null);
    const [tsort, setTsort] = useState({ k: "opened", dir: -1 }), [dsort, setDsort] = useState({ k: "ts", dir: -1 });
    const trades = sc.trades || [], cands = sc.candidates || [];
    const ql = q.trim().toLowerCase(), t0 = since(time);
    const base = (r, ts, text) => (!picked.length || picked.includes(r.asset)) && (!side.length || side.includes(r.side)) && (!t0 || Date.parse(ts) >= t0) && (!ql || text.toLowerCase().includes(ql));
    const tRows = trades.filter((x) => base(x, x.opened, `${x.asset} ${x.side} ${x.reason}`) && (!result.length || result.includes(x.pnl_usd > 0 ? "Won" : "Lost")) && (!reason.length || reason.includes(x.reason)));
    const dRows = cands.filter((c) => base(c, c.ts, `${c.asset} ${c.side} ${OUTCOME(c)} ${c.blocked_by || ""}`) && (!outcome.length || outcome.includes(OUTCOME(c))));
    const tSorted = useSorted(tRows, tsort, (x, k) => (k === "opened" ? x.opened : k === "coin" ? x.asset : k === "held" ? x.hold_secs : k === "fees" ? x.fees_usd : x.pnl_usd));
    const dSorted = useSorted(dRows, dsort, (c, k) => (k === "ts" ? c.ts : k === "coin" ? c.asset : k === "what" ? OUTCOME(c) : c.after?.[k]?.net_bps ?? -1e9));
    const active = picked.length + side.length + result.length + reason.length + outcome.length + (time !== "all") + !!ql;
    const reset = () => { setPicked([]); setSide([]); setResult([]); setReason([]); setOutcome([]); setTime("all"); setQ(""); };
    const src = tab === "trades" ? trades : cands;
    const coinOpts = opts(src, (r) => r.asset);
    for (const c of picked) if (!coinOpts.some((o) => o.value === c)) coinOpts.push({ value: c, label: c, count: 0 });
    const tot = tRows.reduce((x, r) => x + r.pnl_usd, 0), won = tRows.filter((r) => r.pnl_usd > 0).length;
    const after = (c, h) => { const a = c.after?.[h]; if (!a) return html`<td class="n low">—</td>`; const v = per1k(a.net_bps); return html`<td class=${"n " + cls(v)}>${susd(v)}</td>`; };
    return html`<section class="sc-block">
      <div class="tabs" role="tablist">${[["trades", "Trades", trades.length], ["decisions", "Decisions", sc.summary?.candidates], ["how", "How it decides"]].map(([id, l, n]) => html`<button key=${id} class="tab" role="tab" aria-selected=${tab === id} onClick=${() => { setTab(id); setOpen(null); }}>${l}${n != null ? html`<small>${n}</small>` : null}</button>`)}</div>
      ${tab === "how" ? html`<div dangerouslySetInnerHTML=${{ __html: scHowHTML(sc) }} />` : html`
      <div class="sc-tools" role="toolbar" aria-label="Filters">
        <label class="sc-search"><${Icon} n="search" /><input type="search" placeholder=${tab === "trades" ? "Search coin or exit reason" : "Search coin or check"} value=${q} onInput=${(e) => setQ(e.target.value)} aria-label="Search" /></label>
        <${Facet} title="Coin" options=${coinOpts} value=${picked} onChange=${setPicked} />
        <${Facet} title="Side" options=${opts(src, (r) => r.side)} value=${side} onChange=${setSide} />
        ${tab === "trades" ? html`<${Facet} title="Result" options=${opts(trades, (x) => (x.pnl_usd > 0 ? "Won" : "Lost"))} value=${result} onChange=${setResult} /><${Facet} title="Exit" options=${opts(trades, (x) => x.reason)} value=${reason} onChange=${setReason} />` : html`<${Facet} title="Outcome" options=${opts(cands, OUTCOME)} value=${outcome} onChange=${setOutcome} />`}
        <div class="sc-seg" role="group" aria-label="Time">${TIME.map(([k, l]) => html`<button key=${k} aria-pressed=${time === k} onClick=${() => setTime(k)}>${l}</button>`)}</div>
        ${active ? html`<button class="btn quiet sc-reset" onClick=${reset}><${Icon} n="x" />Reset${active > 1 ? ` (${active})` : ""}</button>` : null}
      </div>
      <p class="sc-sum" aria-live="polite">${tab === "trades"
        ? html`<b>${tRows.length}</b> of ${trades.length} trades${tRows.length ? html` · <${Money} x=${tot} /> after fees · ${won} won (${pct(won / tRows.length, 0)})` : null}`
        : html`<b>${dRows.length}</b> of the latest ${cands.length} decisions${dRows.length ? ` · ${dRows.filter((c) => c.taken).length} taken` : ""}`}</p>
      ${tab === "trades" ? html`<div class="tablewrap"><table>
        <thead><tr><${Th} k="opened" sort=${tsort} setSort=${setTsort}>Opened<//><${Th} k="coin" sort=${tsort} setSort=${setTsort}>Coin<//><th>Side</th><${Th} k="held" sort=${tsort} setSort=${setTsort} n>Held<//><th>Why it closed</th><${Th} k="fees" sort=${tsort} setSort=${setTsort} n>Fees<//><${Th} k="pnl" sort=${tsort} setSort=${setTsort} n>Result after fees<//></tr></thead>
        <tbody>${tSorted.length ? tSorted.map((x) => {
          const id = x.candidate_id + x.closed, isOpen = open === id;
          const story = `${x.side === "short" ? "Sold short" : "Bought"} ${usd(x.notional, 0)} of ${x.asset} at ${px(x.entry_px)} (${/maker/.test(x.entry_order || "") ? "a resting order, the cheaper fee" : "a market order"}). Stop ${px(x.init_sl_px)}, target ${px(x.tp_px)}. After ${dur(x.hold_secs)} it closed at ${px(x.exit_px)}: ${x.reason}. Fees ${usd(x.fees_usd)}.`;
          return html`<${React.Fragment} key=${id}><tr class=${"sc-row" + (isOpen ? " open" : "")} tabindex="0" aria-expanded=${isOpen} onClick=${() => setOpen(isOpen ? null : id)} onKeyDown=${(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); setOpen(isOpen ? null : id); } }}>
            <td>${hm(t(x.opened))}<span class="sm">${day(t(x.opened))}</span></td><td><b>${x.asset}</b></td><td><span class=${"side " + x.side}>${x.side}</span></td><td class="n">${dur(x.hold_secs)}</td><td>${x.reason}</td><td class="n">${usd(x.fees_usd)}</td><td class=${"n " + cls(x.pnl_usd)}><b>${susd(x.pnl_usd)}</b><span class="sm">${sgn(x.pnl_bps, 0)} bps</span></td></tr>
            ${isOpen ? html`<tr class="detail"><td colSpan="7"><div class="why"><h3>${story}</h3>${x.candidate ? html`<div dangerouslySetInnerHTML=${{ __html: candidateTrace(x.candidate) }} />` : null}</div></td></tr>` : null}<//>`;
        }) : html`<tr><td colSpan="7" class="empty">${trades.length ? "No trades match these filters." : "No closed trades yet."}</td></tr>`}</tbody></table></div>`
      : html`<div class="tablewrap"><table>
        <thead><tr><${Th} k="ts" sort=${dsort} setSort=${setDsort}>Time<//><${Th} k="coin" sort=${dsort} setSort=${setDsort}>Coin<//><th>Side</th><${Th} k="what" sort=${dsort} setSort=${setDsort}>What happened<//><${Th} k="60" sort=${dsort} setSort=${setDsort} n>1 min later<//><${Th} k="300" sort=${dsort} setSort=${setDsort} n>5 min later<//><${Th} k="600" sort=${dsort} setSort=${setDsort} n>10 min later<//></tr></thead>
        <tbody>${dSorted.length ? dSorted.map((c) => {
          const [k, word] = outcomeOf(c), isOpen = open === c.id;
          return html`<${React.Fragment} key=${c.id}><tr class=${"sc-row" + (isOpen ? " open" : "")} tabindex="0" aria-expanded=${isOpen} onClick=${() => setOpen(isOpen ? null : c.id)} onKeyDown=${(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); setOpen(isOpen ? null : c.id); } }}>
            <td>${hms(t(c.ts))}</td><td><b>${c.asset}</b></td><td><span class=${"side " + c.side}>${c.side}</span></td><td><span class=${"tag " + k}>${word}</span></td>${after(c, 60)}${after(c, 300)}${after(c, 600)}</tr>
            ${isOpen ? html`<tr class="detail"><td colSpan="7"><div dangerouslySetInnerHTML=${{ __html: candidateTrace(c) }} /></td></tr>` : null}<//>`;
        }) : html`<tr><td colSpan="7" class="empty">${cands.length ? "No decisions match these filters." : "No decisions yet."}</td></tr>`}</tbody></table></div>
        <p class="say sc-foot">"Later" columns: what a $1,000 trade in that direction would have made after fees, whether or not it was taken.</p>`}`}
    </section>`;
  }

  return function ScalperApp({ sc }) {
    const [picked, setPicked] = useState([]);
    if (!sc?.available) return html`<div><header class="head"><h1>Scalper</h1></header><div class="sheet empty" dangerouslySetInnerHTML=${{ __html: sc ? "No scalper data yet. Start it with <code>jev-trader --config config.scalp.toml scalp</code>." : "Loading…" }} /></div>`;
    const st = sc.status, s = st.settings || {}, by = sc.summary?.by_coin || {};
    const rank = { "IN TRADE": 0, "ORDER WAITING": 1, REVIEW: 2, "ON HOLD": 3, HELD: 4, "WARMING UP": 5, WATCHING: 6 };
    const live = scalpRows(), syms = [...new Set([...live.map((r) => r.coin), ...Object.keys(by)])];
    const coinRows = syms.map((c) => {
      const r = live.find((x) => x.coin === c) || { coin: c, status: "Not watched", tone: "" }, b = by[c] || {}, open = scOpenPnl(r.a);
      return { ...r, coin: c, statusRank: rank[r.status] ?? 9, n: b.n || 0, wins: b.wins || 0, closed: b.pnl_usd || 0, fees: b.fees_usd || 0, bps: b.bps_sum || 0, long_n: b.long_n, short_n: b.short_n, last: b.last_closed, open, net: (b.pnl_usd || 0) + (open || 0) };
    });
    return html`<div class="sc">
      <header class="head"><div class="title"><h1>Scalper</h1></div><p class="lede">It watches ${st.assets.length} Hyperliquid perps every ${((s.eval_ms || 500) / 1000).toFixed(1)} s and joins when trading suddenly rushes one way and the move looks big enough to pay its fees: long when buyers rush in, short when sellers do. Jev reads recent headlines before each entry and can veto it.</p></header>
      <${Account} sc=${sc} coinRows=${coinRows} />
      <${Coins} coinRows=${coinRows} picked=${picked} setPicked=${setPicked} />
      <${OpenNow} sc=${sc} />
      <${Activity} sc=${sc} picked=${picked} setPicked=${setPicked} />
    </div>`;
  };
}

// ---------- render loop ----------
function render() {
  renderRail(); renderNotices();
  const titles = { board: "Departures", scalper: "Scalper", news: "News bot", evidence: "Is it working?", backtest: "Backtests" };
  document.title = (titles[S.view] || "Departures") + " · Jev trader";
  const keep = window.scrollY;
  if (S.view === "board") boardView();
  else if (S.view === "scalper") scalperView();
  else if (S.view === "news") newsView();
  else if (S.view === "backtest") backtestView();
  else evidenceView();
  if (S.view !== "board") window.scrollTo(0, keep);
}
function route() {
  if (SC.root) { SC.root.unmount(); SC.root = null; }
  const v = (location.hash || "#board").slice(1);
  S.view = ["board", "scalper", "news", "evidence", "backtest"].includes(v) ? v : "board";
  $("#view").dataset.view = "";
  render();
  window.scrollTo(0, 0);
}
addEventListener("hashchange", route);

document.addEventListener("click", async (e) => {
  const b = e.target.closest("button, tr.click");
  if (!b) return;
  if (b.dataset.bt) { btAction(b.dataset.bt); return; }
  if (b.dataset.fill) { const f = BT.fills[+b.dataset.fill]; BT.pick = BT.pick === f ? null : f; btPaint(); return; }
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
  try { await Promise.all([getSnap(), getScalp(), S.view === "backtest" && BT.list?.job?.state === "running" ? btPoll() : null]); render(); } finally { ticking = false; }
}
route();
tick();
setInterval(tick, 2500);
setInterval(renderRail, 1000);
