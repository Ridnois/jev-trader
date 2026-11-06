"use strict";
// Jev trader dashboard. Read-only except Pause / Flatten on the news bot.
// Every number is turned into a plain sentence first; the raw readings sit beside it in mono.
(() => {
  const $ = (s) => document.querySelector(s);
  const VIEWS = { today: "Today", trades: "Trades", decisions: "Decisions", news: "News bot", score: "Scorecard" };

  // ---------- state ----------
  let snap = null, scalp = null, skew = 0, conn = "connecting";
  let view = "today";
  let tradeFilter = "all", decFilter = "all", decShown = 50, headShown = 20;
  const openTrades = new Set(), openTraces = new Set();
  let flattenArmed = false, armTimer = null, busy = false, actionError = null;

  // ---------- format ----------
  const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  const safeUrl = (u) => (/^https?:\/\//i.test(u || "") ? u : null);
  const usdF = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" });
  const usd = (n) => (n == null || !isFinite(n) ? "—" : usdF.format(n));
  const susd = (n) => (n == null || !isFinite(n) ? "—" : (n > 0 ? "+" : n < 0 ? "−" : "") + usdF.format(Math.abs(n)));
  const px = (n) => n == null ? "—" : n.toLocaleString("en-US", { minimumFractionDigits: n < 1 ? 4 : 2, maximumFractionDigits: n < 1 ? 4 : n < 10 ? 3 : 2 });
  const qtyF = (n) => n.toLocaleString("en-US", { maximumFractionDigits: n < 1 ? 5 : 3 });
  // Basis points are an insider unit; everything a reader sees is a percent.
  const pc = (bps, d = 2) => (bps == null || !isFinite(bps) ? "—" : (Math.abs(bps) / 100).toFixed(d) + "%");
  const spc = (bps, d = 2) => (bps == null || !isFinite(bps) ? "—" : (bps > 0 ? "+" : bps < 0 ? "−" : "") + pc(bps, d));
  const cls = (x) => (x > 0 ? "pos" : x < 0 ? "neg" : "");
  const hm = (iso) => new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const now = () => Date.now() + skew;
  const ago = (iso) => {
    const s = Math.max(0, Math.round((now() - Date.parse(iso)) / 1000));
    if (s < 60) return `${s}s ago`;
    if (s < 3600) return `${Math.floor(s / 60)} min ago`;
    return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m ago`;
  };
  const dur = (s) => { s = Math.max(0, Math.round(s)); const m = Math.floor(s / 60); return m ? `${m} min ${String(s % 60).padStart(2, "0")} s` : `${s} s`; };
  const clock = (s) => { s = Math.max(0, Math.round(s)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`; };
  const plural = (n, w, p = w + "s") => `${n.toLocaleString()} ${n === 1 ? w : p}`;
  const clamp = (x, a, b) => Math.max(a, Math.min(b, x));

  // ---------- vocabulary: one plain phrase per machine concept ----------
  const S = () => scalp?.status?.settings || {};
  const feeBps = () => S().fee_bps ?? 10;
  const isPractice = () => scalp?.status?.venue !== "binance";
  // Testnet fills come back with zero commission; real Binance spot charges fee_bps each way.
  const realFees = (t) => (t.fees_usd > 0 || !isPractice() ? 0 : t.notional * 2 * feeBps() / 1e4);
  const realPnl = (t) => t.pnl_usd - realFees(t);
  const betWords = (side) => (side === "short" ? "Bet it would fall" : "Bet it would rise");
  const pushers = (side) => (side === "short" ? "sellers" : "buyers");

  const ENDS = {
    "take profit": ["Hit the target", "Price reached the profit target it set when it entered."],
    "trailing stop": ["Locked in a gain", "Price moved its way, so it raised the stop to protect the gain; when price pulled back to it, it got out."],
    "flow reversed": ["Got out as the rush faded", "The traders who started the move stopped pushing, so it left early instead of waiting."],
    "time stop": ["Ran out of time", "Neither the target nor the stop was hit within its 10-minute limit, so it closed."],
    stop: ["Hit the stop loss", "Price went far enough against the bet to reach the stop it set at entry."],
    "hack alert": ["Got out on a hack alert", "A fresh hack headline about this coin closed the bet immediately."],
  };
  const endOf = (r) => ENDS[r] || [r ? r[0].toUpperCase() + r.slice(1) : "Closed", ""];

  const GATE_ORDER = ["warm-up", "feed", "spread", "burst", "flow", "book", "push", "no chase", "cost", "news", "flat", "max positions", "cooldown", "trades/hour", "daily limit", "exchange", "cash", "inventory", "jev review"];
  const WAIT = {
    "warm-up": "Still learning what normal trading looks like for this coin.",
    feed: "Price data is lagging, so it won't act on it.",
    spread: "The gap between buy and sell prices is too wide right now.",
    burst: "Trading is quiet. There's no rush to ride.",
    flow: "Neither buyers nor sellers are clearly in control.",
    book: "The orders waiting in line don't back up the move.",
    push: "Someone is pushing, but the price hasn't started moving yet.",
    "no chase": "The move already happened. It doesn't chase.",
    cost: "Price swings are too small right now to pay the fees.",
    news: "A hack alert blocks buying this coin.",
    flat: "Already in a trade on this coin.",
    "max positions": "Already in as many trades as it allows at once.",
    cooldown: "Taking a short break on this coin after a loss.",
    "trades/hour": "Reached its limit of trades for this hour.",
    "daily limit": "Stopped for the day after hitting its loss limit.",
    exchange: "The exchange balance check failed, so it holds off.",
    cash: "Not enough cash to buy.",
    inventory: "Not enough of this coin on hand to sell short.",
    "jev review": "Jev flagged news working against the bet.",
  };

  // News-bot gates and caps (0.6), in plain words.
  const NGATE = {
    flatten: "Sell-everything request", "flatten (dashboard)": "Sell-everything request", "flatten requested": "Sell-everything request",
    "kill switch": "Daily loss limit", "stop loss": "Stop loss", "asset cooldown": "Rest after a stop on this coin",
    inputs: "Prices and news were complete and sane", account: "Account records were consistent", breaker: "Weekly, monthly and losing-streak limits",
    "jev health": "Jev was answering reliably", regime: "Whole-market mood", "regime unknown": "Whole-market mood", "regime ceiling": "Whole-market mood",
    "news reaction": "Price has been following good news", challenge: "Jev's case against buying", pause: "Not paused by you", paused: "Not paused by you",
  };
  const BIND = {
    signal: "nothing: the news signal alone set the size", asset_cap: "the per-coin limit", gross_cap: "the limit on total money invested",
    heat_cap: "the cap on total risk at the stop losses", regime_ceiling: "the whole-market mood ceiling", rebalance_band: "the change being too small to bother trading",
    min_trade: "the minimum trade size", cash: "the cash on hand",
  };
  const posture = () => snap?.status?.posture || null;
  const untilWords = (iso) => (iso ? ` until ${new Date(iso).toLocaleString([], { weekday: "short", hour: "2-digit", minute: "2-digit" })}` : "");
  function postureWords(p) {
    if (p.kind === "ENTRIES_ALLOWED") return [`Can open new trades`, `It may put up to ${Math.round(p.ceiling * 100)}% of its money into coins.`];
    if (p.kind === "EXITS_ONLY") return [`Selling only`, `It won't buy anything${untilWords(p.until)}${p.reason ? `: ${p.reason}` : ""}. Stop losses and sells keep working.`];
    return [`On hold for review`, `Something it relies on looks wrong${p.reason ? ` (${p.reason})` : ""}, so it won't buy until that clears. Stop losses and sells keep working.`];
  }
  const zoneWords = { RISK_ON: "broadly rising", NEUTRAL: "mixed", RISK_OFF: "broadly falling", UNKNOWN: "unclear (too few readings)" };

  // ---------- liveness ----------
  function scalpLive() {
    if (!scalp) return "connecting";
    if (!scalp.available) return "never";
    if (scalp.running === false) return "stopped";
    return (now() - Date.parse(scalp.status.ts)) / 1000 > 15 ? "stale" : "alive";
  }
  function newsLive() {
    if (!snap) return "connecting";
    if (!snap.status) return "never";
    if (snap.bot_pid_running === false) return "stopped";
    const last = snap.status.last_tick_at;
    const lim = Math.max(180, 3 * (snap.status.interval_secs || 60));
    return last && (now() - Date.parse(last)) / 1000 > lim ? "stale" : "alive";
  }

  // ---------- rail ----------
  function renderRail() {
    document.querySelectorAll(".nav a").forEach((a) => (a.dataset.view === view ? a.setAttribute("aria-current", "page") : a.removeAttribute("aria-current")));
    const sl = scalpLive(), st = scalp?.status;
    const ps = $("#pulse-scalp");
    ps.dataset.s = sl;
    const inTrade = st?.assets?.filter((a) => a.position).length || 0;
    ps.querySelector("span").textContent =
      sl === "connecting" ? "Connecting…" : sl === "never" ? "Not started" : sl === "stopped" ? `Stopped ${ago(st.ts)}` : sl === "stale" ? `No update since ${ago(st.ts)}` :
      inTrade ? `In ${plural(inTrade, "trade")} right now` : `Watching ${plural(st.assets?.length || 0, "coin")}`;
    const nl = newsLive(), ns = snap?.status;
    const pn = $("#pulse-news");
    pn.dataset.s = nl;
    let line = "Connecting…";
    if (nl === "never") line = "Not started";
    else if (nl === "stopped") line = `Stopped ${ago(ns.last_tick_at)}`;
    else if (nl === "stale") line = `No update since ${ago(ns.last_tick_at)}`;
    else if (ns) {
      const next = ns.next_tick_at ? (Date.parse(ns.next_tick_at) - now()) / 1000 : null;
      const p = posture();
      line = ns.paused ? "Paused by you" : p && p.kind !== "ENTRIES_ALLOWED" ? postureWords(p)[0] : next > 0 ? `Next read in ${clock(next)}` : "Reading the news…";
    }
    pn.querySelector("span").textContent = line;
    const m = $("#money-note");
    const v = st?.venue;
    m.classList.toggle("real", v === "binance" || !!ns?.live && !ns?.testnet);
    m.innerHTML = v === "binance" ? "<b>Real money.</b> The scalp bot trades on Binance with your funds."
      : `<b>Practice money only.</b> The scalp bot uses Binance's testnet; the news bot is ${ns?.testnet ? "on testnet too" : "simulated"}.`;
  }

  function renderNotices() {
    const out = [];
    if (conn === "lost") out.push(["", "<b>The dashboard server isn't answering.</b> What you see may be out of date; it keeps retrying."]);
    if (actionError) out.push(["", `<b>That didn't go through.</b> ${esc(actionError)} Nothing was changed. <button class="btn quiet" data-act="dismiss">Dismiss</button>`]);
    const sl = scalpLive();
    if (sl === "stopped" || sl === "stale") out.push(["", `<b>The scalp bot ${sl === "stopped" ? "has stopped" : "has gone quiet"}.</b> Its last update was ${ago(scalp.status.ts)}. Numbers below are from then.`]);
    if (scalp?.status?.halted_today) out.push(["warn", "<b>The scalp bot hit its daily loss limit.</b> It won't open trades again until tomorrow (UTC)."]);
    if (scalp?.status?.feed && !scalp.status.feed.connected) out.push(["warn", "<b>The scalp bot lost its live price feed.</b> It won't trade until the feed reconnects."]);
    const nl = newsLive();
    if ((view === "news" || view === "score") && (nl === "stopped" || nl === "stale")) out.push(["", `<b>The news bot ${nl === "stopped" ? "has stopped" : "has gone quiet"}.</b> Last read ${ago(snap.status.last_tick_at)}.`]);
    const p = posture();
    if (p && p.kind !== "ENTRIES_ALLOWED" && nl === "alive" && !snap.status.paused) { const [h, t] = postureWords(p); out.push(["warn", `<b>News bot: ${esc(h.toLowerCase())}.</b> ${esc(t)}`]); }
    if (snap?.synthetic) out.push(["", "<b>Demo data.</b> The news bot's headlines, prices and fills here are synthetic, not real."]);
    $("#notices").innerHTML = out.map(([k, h]) => `<div class="notice ${k}" role="status">${h}</div>`).join("");
  }

  // ---------- shared drawings ----------
  // One bet, drawn on a line measured in the bet's favour: stop on the left, target on the right.
  function ruler(t, { labels = true, h = 46 } = {}) {
    const dir = t.side === "short" ? -1 : 1;
    const move = ((t.exit_px - t.entry_px) / t.entry_px) * 1e4 * dir;
    const tp = Math.abs(t.tp_px - t.entry_px) / t.entry_px * 1e4;
    const sl = ((t.init_sl_px ?? t.sl_px) - t.entry_px) / t.entry_px * 1e4 * dir;
    const fee = isPractice() && !(t.fees_usd > 0) ? 2 * feeBps() : 0;
    const lo = Math.min(sl, move) * 1.08, hi = Math.max(tp, move, fee) * 1.08;
    const W = 300, y = labels ? 20 : 12;
    const x = (b) => ((b - lo) / (hi - lo)) * W;
    const col = t.pnl_usd >= 0 ? "var(--gain)" : "var(--loss)";
    const ex = x(move);
    const anchor = ex < 40 ? "start" : ex > W - 40 ? "end" : "middle";
    return `<svg class="ruler" viewBox="0 0 ${W} ${h}" preserveAspectRatio="none" role="img" aria-label="Stop ${spc(sl)}, target ${spc(tp)}, got out at ${spc(move)}">
      <defs><pattern id="hatch" width="4" height="4" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><rect width="1.6" height="4" fill="#B9C0B9"/></pattern></defs>
      <rect x="${x(sl)}" y="${y - 3}" width="${x(0) - x(sl)}" height="6" fill="var(--loss-wash)"/>
      <rect x="${x(0)}" y="${y - 3}" width="${x(tp) - x(0)}" height="6" fill="var(--gain-wash)"/>
      ${fee ? `<rect x="${x(0)}" y="${y - 3}" width="${x(fee) - x(0)}" height="6" fill="url(#hatch)"/>` : ""}
      <line x1="${x(sl)}" x2="${x(sl)}" y1="${y - 7}" y2="${y + 7}" stroke="var(--loss)" stroke-width="2" vector-effect="non-scaling-stroke"/>
      <line x1="${x(tp)}" x2="${x(tp)}" y1="${y - 7}" y2="${y + 7}" stroke="var(--gain)" stroke-width="2" vector-effect="non-scaling-stroke"/>
      <line x1="${x(0)}" x2="${x(0)}" y1="${y - 7}" y2="${y + 7}" stroke="var(--ink)" stroke-width="1.5" vector-effect="non-scaling-stroke"/>
      <line x1="${x(0)}" x2="${ex}" y1="${y}" y2="${y}" stroke="${col}" stroke-width="2.5" vector-effect="non-scaling-stroke"/>
      <circle cx="${ex}" cy="${y}" r="5" fill="${col}" stroke="#fff" stroke-width="1.5"/>
      ${labels ? `<text x="${ex}" y="${y - 10}" text-anchor="${anchor}" style="fill:${col};font-weight:700">out ${spc(move)}</text>
      <text x="${x(sl)}" y="${y + 20}" text-anchor="start">stop</text>
      <text x="${x(tp)}" y="${y + 20}" text-anchor="end">target</text>` : ""}
    </svg>`;
  }

  function pnlChart(trades) {
    const T = [...trades].sort((a, b) => Date.parse(a.closed) - Date.parse(b.closed));
    if (T.length < 2) return `<p class="small">The chart starts after the second trade.</p>`;
    let g = 0, r = 0;
    const pts = [{ ts: Date.parse(T[0].opened), g: 0, r: 0 }, ...T.map((t) => ({ ts: Date.parse(t.closed), g: (g += t.pnl_usd), r: (r += realPnl(t)) }))];
    const W = 520, H = 210, padL = 52, padR = 70, padT = 10, padB = 24;
    const t0 = pts[0].ts, t1 = pts[pts.length - 1].ts;
    const vals = pts.flatMap((p) => [p.g, p.r, 0]);
    let lo = Math.min(...vals), hi = Math.max(...vals);
    const span = Math.max(1, hi - lo); lo -= span * 0.08; hi += span * 0.08;
    const x = (ts) => padL + ((ts - t0) / Math.max(1, t1 - t0)) * (W - padL - padR);
    const y = (v) => padT + (1 - (v - lo) / (hi - lo)) * (H - padT - padB);
    const step = (k) => pts.map((p, i) => (i ? `H${x(p.ts).toFixed(1)}V${y(p[k]).toFixed(1)}` : `M${x(p.ts).toFixed(1)},${y(p[k]).toFixed(1)}`)).join("");
    const last = pts[pts.length - 1];
    const showReal = isPractice();
    let ly1 = y(last.g), ly2 = y(last.r);
    if (Math.abs(ly1 - ly2) < 16) { const mid = (ly1 + ly2) / 2; ly1 = mid - (last.g >= last.r ? 9 : -9); ly2 = mid + (last.g >= last.r ? 9 : -9); }
    return `<svg class="pnl" viewBox="0 0 ${W} ${H}" role="img" aria-label="Running total of trade results: ${susd(last.g)} on testnet${showReal ? `, ${susd(last.r)} with real fees` : ""}">
      <line x1="${padL}" x2="${W - padR}" y1="${y(0)}" y2="${y(0)}" stroke="var(--rule)" />
      <text x="${padL - 8}" y="${y(0) + 4}" text-anchor="end" font-size="12" fill="var(--mute)">$0</text>
      <text x="${padL - 8}" y="${y(hi) + 12}" text-anchor="end" font-size="12" fill="var(--mute)">${susd(hi)}</text>
      <text x="${padL - 8}" y="${y(lo)}" text-anchor="end" font-size="12" fill="var(--mute)">${susd(lo)}</text>
      ${showReal ? `<path d="${step("r")}" fill="none" stroke="var(--loss)" stroke-width="2" stroke-dasharray="5 4"/>` : ""}
      <path d="${step("g")}" fill="none" stroke="var(--ink)" stroke-width="2.4"/>
      <text x="${W - padR + 8}" y="${ly1 + 4}" font-size="13" font-weight="700" fill="var(--ink)">${susd(last.g)}</text>
      ${showReal ? `<text x="${W - padR + 8}" y="${ly2 + 4}" font-size="13" font-weight="700" fill="var(--loss)">${susd(last.r)}</text>` : ""}
      <text x="${padL}" y="${H - 4}" font-size="12" fill="var(--mute)">${hm(new Date(t0).toISOString())}</text>
      <text x="${W - padR}" y="${H - 4}" text-anchor="end" font-size="12" fill="var(--mute)">${hm(new Date(t1).toISOString())}</text>
    </svg>
    <div class="legend"><span><i style="background:var(--ink)"></i>What it made on the testnet (no fees charged)</span>${showReal ? `<span><i style="background:repeating-linear-gradient(90deg,var(--loss) 0 5px,transparent 5px 8px)"></i>The same trades if real Binance fees applied</span>` : ""}</div>`;
  }

  // ---------- today ----------
  const SIEVE = [
    { title: "Spotted a rush", text: "Trading suddenly got busier than normal, one side (buyers or sellers) took control, and the price started moving their way." },
    { title: "Could read the market", text: "The price feed was fresh and buy/sell prices were close together.", drops: ["feed", "spread"], why: "price feed lagging or prices too far apart" },
    { title: "Wasn't too late", text: "The price hadn't already run far in that direction. Late entries tend to catch the end of a move.", drops: ["no chase"], why: "the move had already happened" },
    { title: "Could pay the fees", text: "Typical price swings were big enough to beat fees and spread with room to spare.", drops: ["cost"], why: "swings too small to pay the fees" },
    { title: "Within its own limits", text: "Not already in that coin, under its caps on open trades and trades per hour, no recent loss on it.", drops: ["flat", "max positions", "cooldown", "trades/hour", "daily limit", "exchange", "cash", "inventory"], why: null },
    { title: "Jev saw no news against it", text: "Jev, the AI that reads headlines, checked recent news on the coin. It can block a trade or halve it, never start one.", drops: ["jev review"], why: "Jev saw news working against the bet" },
  ];
  const LIMIT_WORDS = { flat: "already in that coin", "max positions": "too many trades open", cooldown: "resting after a loss", "trades/hour": "hourly trade limit", "daily limit": "daily loss limit", exchange: "exchange check", cash: "not enough cash", inventory: "not enough coin to sell" };

  function sieveHtml(sum) {
    const by = { ...(sum.blocked_by || {}) };
    const total = sum.candidates || 0;
    const known = new Set(SIEVE.flatMap((s) => s.drops || []).concat(["taken"]));
    // Anything unexpected is counted with the limits so the arithmetic always adds up.
    const extra = Object.keys(by).filter((k) => !known.has(k));
    let left = total;
    const rows = SIEVE.map((s, i) => {
      if (!s.drops) return { ...s, left, dropped: 0, parts: [] };
      const keys = i === 4 ? [...s.drops, ...extra] : s.drops;
      const parts = keys.map((k) => [k, by[k] || 0]).filter(([, n]) => n > 0);
      const dropped = parts.reduce((a, [, n]) => a + n, 0);
      const before = left; left -= dropped;
      return { ...s, before, left, dropped, parts };
    });
    const pct = (n) => (total ? (n / total) * 100 : 0);
    const T = scalp.trades || [];
    const won = T.filter((t) => t.pnl_usd > 0).length;
    return `<ol class="sieve">${rows.map((r, i) => {
      const note = r.dropped ? `<p class="dropnote"><b>−${r.dropped.toLocaleString()}</b> ${r.why ? esc(r.why) : r.parts.sort((a, b) => b[1] - a[1]).map(([k, n]) => `${n} ${esc(LIMIT_WORDS[k] || k)}`).join(", ")}</p>` : "";
      return `<li><span class="n">${i + 1}</span><div class="what"><h3>${r.title}</h3><p>${r.text}</p></div><span class="count">${r.left.toLocaleString()}</span>
        <div class="bar" aria-hidden="true"><span class="keep" style="width:${pct(r.left)}%"></span><span class="drop" style="width:${pct(r.dropped)}%"></span></div>${note}</li>`;
    }).join("")}
      <li class="final"><span class="n">${rows.length + 1}</span><div class="what"><h3>Traded</h3><p>Bought or sold right away, with a profit target, a stop loss and a 10-minute time limit.</p></div><span class="count">${(by.taken || 0).toLocaleString()}</span>
        <div class="bar" aria-hidden="true"><span class="keep" style="width:${Math.max(0.6, pct(by.taken || 0))}%"></span></div></li>
    </ol>
    ${T.length ? `<div class="outcome"><span class="w">${won} made money</span><span class="l">${T.length - won} lost money</span></div>` : ""}`;
  }

  function coinCard(a) {
    const f = a.features || {}, p = a.position;
    const name = a.symbol;
    let line;
    if (p) {
      const dir = p.side === "short" ? -1 : 1;
      const mv = f.mid ? ((f.mid - p.entry_px) / p.entry_px) * 1e4 * dir : null;
      const left = p.deadline_ms ? (p.deadline_ms - now()) / 1000 : null;
      line = `<p><mark>In a trade.</mark> ${betWords(p.side)} from ${px(p.entry_px)}. ${mv != null ? `Now <b class="${cls(mv)}">${spc(mv)}</b> in its favour.` : ""} ${left > 0 ? `Closes within ${clock(left)} at most.` : ""}</p>`;
    } else if (a.inventory_note && !a.long_block) {
      line = `<p>${esc(a.inventory_note)}</p>`;
    } else {
      const lb = a.long_block, sb = a.short_block;
      const rank = (g) => (g == null ? 99 : GATE_ORDER.indexOf(g));
      const g = rank(lb) >= rank(sb) ? lb : sb;
      const side = rank(lb) >= rank(sb) ? "long" : "short";
      line = !lb || !sb ? `<p><mark>Ready.</mark> Every check passes for a bet that it will ${(!lb ? "rise" : "fall")}; a trade can open any second.</p>`
        : `<p>${esc(WAIT[g] || g)}${rank(g) >= GATE_ORDER.indexOf("no chase") ? ` <span class="small">(closest to a ${side === "long" ? "rise" : "fall"} bet)</span>` : ""}</p>`;
    }
    const act = f.activity ?? 0, need = S().min_activity_ratio ?? 1.5;
    const buyShare = f.flow != null ? (1 + f.flow) / 2 : 0.5;
    return `<article class="coin${p ? " intrade" : ""}">
      <header><b>${esc(name)}</b><span>${px(f.mid)}</span></header>
      ${line}
      <div class="meters">
        <div class="meter"><small>Busy: ${act.toFixed(1)}× normal</small><div class="track"><span class="fill" style="left:0;width:${clamp(act / 4, 0, 1) * 100}%"></span><span class="tick" style="left:${(need / 4) * 100}%"></span></div></div>
        <div class="meter"><small>Buyers ${Math.round(buyShare * 100)}% · sellers ${Math.round((1 - buyShare) * 100)}%</small><div class="track"><span class="fill" style="${buyShare >= 0.5 ? `left:50%;width:${(buyShare - 0.5) * 100}%` : `left:${buyShare * 100}%;width:${(0.5 - buyShare) * 100}%`}"></span><span class="tick" style="left:50%"></span></div></div>
      </div>
    </article>`;
  }

  function renderToday(el) {
    if (!scalp) { el.innerHTML = `<p class="small">Loading…</p>`; return; }
    if (!scalp.available) {
      el.innerHTML = `<h1 class="hero">The scalp bot hasn't run yet.</h1><p class="lede">Start it with <code>jev-trader --config config.scalp.toml scalp</code>. This page fills in as soon as it writes its first status.</p>`;
      return;
    }
    const st = scalp.status, sum = scalp.summary, T = scalp.trades || [];
    const coins = st.assets.map((a) => a.symbol);
    const gross = T.reduce((a, t) => a + t.pnl_usd, 0);
    const real = T.reduce((a, t) => a + realPnl(t), 0);
    const feesEst = T.reduce((a, t) => a + realFees(t), 0);
    const won = T.filter((t) => t.pnl_usd > 0).length;
    const since = sum.session_start ? hm(sum.session_start) : null;
    const venue = st.venue === "binance" ? `<span class="venue-tag live">real money</span>` : `<span class="venue-tag">practice money</span>`;
    const endings = Object.entries(T.reduce((m, t) => ((m[t.reason] = (m[t.reason] || 0) + 1), m), {})).sort((a, b) => b[1] - a[1]);
    const showReal = isPractice() && feesEst > 0;

    el.innerHTML = `
      <p class="kicker">Scalp bot ${venue}</p>
      <h1 class="hero">It waits for sudden rushes of buying or selling, rides them for a few minutes, then gets out.</h1>
      <p class="lede">It watches ${coins.length} coins on Binance (${esc(coins.join(", "))}) second by second. It can bet a price will rise (buy, then sell) or fall (sell coins it keeps on hand, then buy them back). Before every trade it runs the same checks, in order, and most chances fail one of them.</p>

      <p class="day">${since ? `Since ${since}` : "So far"} it spotted <b>${sum.candidates.toLocaleString()}</b> possible trades and took <b>${T.length}</b>${T.length ? `; <b>${won}</b> of them made money` : ""}.
      ${T.length ? `In practice money that's <b class="${cls(gross)}">${susd(gross)}</b>${showReal ? `, but the testnet charges no fees. With Binance's real fees (${(feeBps() / 100).toFixed(1)}% each way, about ${usd(feesEst)} in total) the same trades would be at <span class="money-real ${cls(real)}">${susd(real)}</span>.` : "."}` : "No trade has passed every check yet."}</p>

      <div class="grid2">
        <section class="panel" aria-labelledby="h-sieve">
          <h2 id="h-sieve">How it decides</h2>
          <p class="sub">Every chance it spotted, run through its checks in order. The number on the right is how many were still in the running after that check.</p>
          ${sieveHtml(sum)}
        </section>
        <section class="panel" aria-labelledby="h-money">
          <h2 id="h-money">Is it making money?</h2>
          <p class="sub">${T.length ? (showReal ? (real < 0 && gross > 0 ? "On the testnet, yes. With real fees, not yet: most wins are smaller than the fee on the round trip." : real >= 0 ? "Yes, even after counting real fees." : "No, not even before fees.") : gross >= 0 ? "Yes, so far." : "Not so far.") : "Nothing to judge yet: no trades have closed."}</p>
          ${T.length ? pnlChart(T) : ""}
          ${T.length ? `<dl class="figures">
            <dt>Trades won / lost</dt><dd>${won} / ${T.length - won}</dd>
            <dt>Average result per trade</dt><dd class="${cls(gross)}">${susd(gross / T.length)}</dd>
            <dt>Result on the testnet</dt><dd class="${cls(gross)}">${susd(gross)}</dd>
            ${showReal ? `<dt>Real fees it didn't pay</dt><dd class="neg">−${usd(feesEst)}</dd><dt class="total">With real fees</dt><dd class="total ${cls(real)}">${susd(real)}</dd>` : ""}
          </dl>
          <ul class="endings" aria-label="How trades ended">${endings.map(([r, n]) => `<li><b>${n}</b><span>${esc(endOf(r)[0])}</span></li>`).join("")}</ul>` : ""}
          ${st.inventory_drift != null && Math.abs(st.inventory_drift) > 0.5 ? `<p class="aside">Separate from its trades: to bet on falls it holds some of each coin. Market moves changed that stock's value by <b class="${cls(st.inventory_drift)}">${susd(st.inventory_drift)}</b>. That comes from the market, not from any decision it made.</p>` : ""}
        </section>
      </div>

      <section class="block" aria-labelledby="h-now">
        <h2 id="h-now">What it's doing right now</h2>
        <p class="sub">Each coin with the reason it isn't trading, or the trade it's in. "Busy" compares trading speed with the last 30 minutes; it needs ${(S().min_activity_ratio ?? 1.5).toFixed(1)}× (the black tick) before it looks closer.</p>
        <div class="coins">${st.assets.map(coinCard).join("")}</div>
      </section>`;
  }

  // ---------- trades ----------
  function gateDetail(c, name) { return [...(c?.eval?.gates || []), ...(c?.risk || [])].find((g) => g.gate === name)?.detail || ""; }

  function storyHtml(t) {
    const c = t.candidate, f = c?.features || {}, e = c?.eval || {}, rv = t.review || c?.review;
    const coin = esc(t.asset), side = t.side;
    const who = pushers(side);
    const share = f.flow != null ? Math.round(((1 + Math.abs(f.flow)) / 2) * 100) : null;
    const dir = side === "short" ? -1 : 1;
    const r60 = f.ret_60s_bps, r300 = f.ret_300s_bps;
    const steps = [];
    if (c) {
      steps.push(["Spotted a rush", `${who[0].toUpperCase() + who.slice(1)} took over ${coin}. Trading was <b>${(f.activity ?? 0).toFixed(1)}×</b> busier than usual${share ? ` and <b>${share}%</b> of the volume came from ${who}` : ""}. The price had ${r60 * dir >= 0 ? (side === "short" ? "dropped" : "risen") : "barely moved"} ${pc(Math.abs(r60))} in the last minute.`,
        ["burst", "flow", "book", "push"].map((g) => `${g}: <b>${esc(gateDetail(c, g))}</b>`).join("<br>")]);
      steps.push(["Checked it wasn't late", (r300 * dir > 0 ? `Over the last 5 minutes the price had already moved ${pc(r300)} in the bet's direction, under the ${pc(S().max_chase_bps ?? 60)} it allows before calling a move finished.` : `Over the last 5 minutes the price had moved ${pc(r300)} the other way, so the move was just starting.`), `no chase: <b>${esc(gateDetail(c, "no chase"))}</b>`]);
      steps.push(["Checked it could pay the fees", `Typical swings for ${coin} over the next 10 minutes were about <b>${pc(f.exp_move_bps)}</b>. A round trip costs about <b>${pc(e.cost_bps)}</b> in fees and spread. It wants at least ${S().cost_mult ?? 2}× the cost, so this passed.`, `cost: <b>${esc(gateDetail(c, "cost"))}</b><br>spread: <b>${esc(gateDetail(c, "spread"))}</b>`]);
      steps.push(["Asked Jev about the news", rv
        ? `Jev read ${plural(rv.headlines, "recent headline")} about ${coin}. It put the chance that the news works against this bet at <b>${Math.round(rv.adverse * 100)}%</b> and the chance of a market-moving event at <b>${Math.round(rv.event_risk * 100)}%</b>. ${t.size_mult < 1 ? "That was enough to <b>cut the size in half</b>." : "No reason to object, so full size."}`
        : c.review_error ? `Jev couldn't be reached (${esc(c.review_error)}), so the trade went ahead on market data alone.` : `There were no recent headlines about ${coin}, so Jev had nothing to object to.`,
        `news mood: <b>${(c.news?.score ?? 0).toFixed(2)}</b> from ${c.news?.n_events ?? 0} headlines${rv ? `<br>jev review: <b>${esc(gateDetail(c, "jev review"))}</b>` : ""}`]);
      steps.push(["Checked its own limits", `${gateDetail(c, "max positions") ? `Trades open: ${esc(gateDetail(c, "max positions"))}. ` : ""}${gateDetail(c, "trades/hour") ? `Trades this hour: ${esc(gateDetail(c, "trades/hour"))}. ` : ""}No recent loss on ${coin} and the daily loss limit wasn't hit.`, (c.risk || []).map((g) => `${esc(g.gate)}: <b>${g.passed ? "ok" : "blocked"}</b>`).join(" · ")]);
    } else {
      steps.push(["Why it entered", "The decision record for this trade is older than the last 20,000 decisions kept, so only the trade itself is shown.", ""]);
    }
    const tpPct = Math.abs(t.tp_px - t.entry_px) / t.entry_px * 1e4, slPct = Math.abs((t.init_sl_px ?? t.sl_px) - t.entry_px) / t.entry_px * 1e4;
    steps.push(["Placed the trade", `${side === "short" ? "Sold" : "Bought"} <b>${qtyF(t.qty)} ${coin}</b> at <b>${px(t.entry_px)}</b> (${usd(t.notional)}) at ${hm(t.opened)}. Target ${px(t.tp_px)} (${pc(tpPct)} away), stop ${px(t.init_sl_px ?? t.sl_px)} (${pc(slPct)} away).`, `order ${esc(t.entry_order)}`, true]);
    const [endT, endX] = endOf(t.reason);
    steps.push([endT, `${endX} ${side === "short" ? "Bought back" : "Sold"} at <b>${px(t.exit_px)}</b> after ${dur(t.hold_secs)}.`, `exit: <b>${esc(t.reason)}</b><br>order ${esc(t.exit_order)}`, true]);
    const showReal = realFees(t) > 0;
    const res = `<div class="big"><div><small>On the testnet</small><b class="${cls(t.pnl_usd)}">${susd(t.pnl_usd)}</b></div>${showReal ? `<div><small>With real fees (${usd(realFees(t))})</small><b class="${cls(realPnl(t))}">${susd(realPnl(t))}</b></div>` : ""}</div>`;
    return `<div class="story"><ol>${steps.map(([h, p, raw, did]) => `<li class="${did ? "did" : ""}"><h4>${h}</h4><p>${p}</p>${raw ? `<div class="raw">${raw}</div>` : ""}</li>`).join("")}
      <li class="did"><h4>Result</h4>${res}</li></ol></div>`;
  }

  function renderTrades(el) {
    if (!scalp?.available) { el.innerHTML = `<p class="small">${scalp ? "The scalp bot hasn't run yet." : "Loading…"}</p>`; return; }
    const T = scalp.trades || [];
    const won = T.filter((t) => t.pnl_usd > 0), lost = T.filter((t) => t.pnl_usd <= 0);
    const list = tradeFilter === "won" ? won : tradeFilter === "lost" ? lost : T;
    const fee = isPractice();
    el.innerHTML = `
      <p class="kicker">Scalp bot</p>
      <h1 class="hero">Every trade, from the moment it spotted the chance to the money it made.</h1>
      <p class="lede">Open a trade to read its story. The line on each row is the bet: <b class="neg">stop loss</b> on the left, <b class="pos">profit target</b> on the right, the dot is where it got out.${fee ? ` The hatched part is what real fees would take, so a dot inside it only wins on the testnet.` : ""}</p>
      <div class="toolbar" role="group" aria-label="Filter trades" style="margin-top:28px">
        ${[["all", "All", T.length], ["won", "Made money", won.length], ["lost", "Lost money", lost.length]].map(([k, l, n]) => `<button class="chip" data-tf="${k}" aria-pressed="${tradeFilter === k}">${l}<span class="c">${n}</span></button>`).join("")}
      </div>
      ${list.length ? `<ul class="ledger">${list.map((t) => {
        const id = t.candidate_id || t.opened;
        return `<li><details class="trade" data-id="${esc(id)}"${openTrades.has(id) ? " open" : ""}>
          <summary>
            <span class="when"><b>${hm(t.opened)}</b>${clock(t.hold_secs)} held</span>
            <span class="bet"><b>${esc(t.asset)}: ${betWords(t.side).toLowerCase()}</b><span>${esc(endOf(t.reason)[0])}</span></span>
            <span class="rulerwrap">${ruler(t)}</span>
            <span class="res"><b class="${cls(t.pnl_usd)}">${susd(t.pnl_usd)}</b>${realFees(t) > 0 ? `<span>${susd(realPnl(t))} with real fees</span>` : ""}</span>
            <span class="chev" aria-hidden="true"></span>
          </summary>
          ${openTrades.has(id) ? storyHtml(t) : ""}
        </details></li>`;
      }).join("")}</ul>` : `<div class="empty">${T.length ? "No trades match this filter." : "<b>No trades yet.</b> The bot only trades when a chance passes every check; the Today page shows where they're failing."}</div>`}`;
  }

  // ---------- decisions ----------
  function reasonSentence(c) {
    const f = c.features || {}, e = c.eval || {}, g = c.blocked_by;
    const dir = c.side === "short" ? -1 : 1;
    if (c.taken) return `Took it: ${pushers(c.side)} pushing, ${(f.activity ?? 0).toFixed(1)}× normal activity, likely swing ${pc(f.exp_move_bps)} vs ${pc(e.cost_bps)} cost.${c.size_mult < 1 ? " Jev halved the size." : ""}`;
    switch (g) {
      case "cost": return `Likely swing of ${pc(f.exp_move_bps)} wasn't ${S().cost_mult ?? 2}× the ${pc(e.cost_bps)} cost of a round trip.`;
      case "no chase": return `Too late: price had already moved ${pc(Math.abs((f.ret_300s_bps ?? 0) * dir))} its way in 5 minutes.`;
      case "trades/hour": return `Already made ${esc(gateDetail(c, "trades/hour"))} trades this hour.`;
      case "max positions": return `Already in ${esc(gateDetail(c, "max positions"))} trades, its maximum.`;
      case "jev review": return c.review ? `Jev vetoed it: ${Math.round(c.review.adverse * 100)}% chance the news works against this bet (${plural(c.review.headlines, "headline")}).` : "Jev vetoed it on the news.";
      default: return WAIT[g] || g || "Skipped.";
    }
  }
  function afterSentence(c, h) {
    const a = c.after?.[String(h)];
    if (!a) return (now() - Date.parse(c.ts)) / 1000 < h + 30 ? `Too recent to check yet.` : "";
    const m = h / 60;
    if (c.taken) return `${m} min later the price was ${spc(a.ret_bps)} in its favour.`;
    return a.net_bps > 0 ? `${m} min later: ${spc(a.ret_bps)} its way. That would have beaten the costs; a missed chance.`
      : `${m} min later: ${spc(a.ret_bps)} its way. Not enough to beat costs, so skipping saved money.`;
  }

  function renderDecisions(el) {
    if (!scalp?.available) { el.innerHTML = `<p class="small">${scalp ? "The scalp bot hasn't run yet." : "Loading…"}</p>`; return; }
    const sum = scalp.summary, C = scalp.candidates || [];
    const hs = S().markouts || [60, 300, 600];
    const H = hs.includes(300) ? 300 : hs[Math.floor(hs.length / 2)];
    const mk = (g) => (sum.markouts || []).find((m) => m.group === g && m.h === H);
    const groups = [["taken", "Trades it took", "were ahead by more than the cost"], ["cost", "Skipped as too small", "would have beaten the costs"], ["jev_veto", "Vetoed by Jev", "would have beaten the costs"], ["other", "Skipped for timing or limits", "would have beaten the costs"]];
    const list = C.filter((c) => (decFilter === "took" ? c.taken : decFilter === "skipped" ? !c.taken : true));
    el.innerHTML = `
      <p class="kicker">Scalp bot</p>
      <h1 class="hero">Every chance it considered, why it acted or passed, and what the price did next.</h1>
      <p class="lede">It writes down every possible trade it spots, whether it takes it or not, then checks the price ${H / 60} minutes later. That's how you can tell if its rules are skipping the right ones.</p>
      <h2 style="margin-top:36px">Were its calls right?</h2>
      <p class="sub">Share of each group that would have made money ${H / 60} minutes after the decision, counting real fees. If skipped chances score as well as taken ones, the rule doing the skipping isn't helping.</p>
      <div class="verdicts">${groups.map(([g, t, w]) => {
        const m = mk(g);
        return `<div class="verdict"><h3>${t}</h3>${m ? `<p class="score">${Math.round(m.hit * 100)}%</p><p>${w}, out of ${m.n.toLocaleString()}. Average move ${spc(m.mean_ret_bps)}.</p>` : `<p class="small">No data yet.</p>`}</div>`;
      }).join("")}</div>
      <h2>Latest decisions</h2>
      <div class="toolbar" role="group" aria-label="Filter decisions">
        ${[["all", "All"], ["took", "Took"], ["skipped", "Skipped"]].map(([k, l]) => `<button class="chip" data-df="${k}" aria-pressed="${decFilter === k}">${l}</button>`).join("")}
      </div>
      ${list.length ? `<ul class="dlist">${list.slice(0, decShown).map((c) => `<li class="${c.taken ? "took" : ""}">
        <span class="tm">${hm(c.ts)}</span>
        <span class="who"><b>${esc(c.asset)}</b><span>${c.side === "short" ? "might fall" : "might rise"}</span></span>
        <span><span class="tag${c.taken ? " took" : ""}">${c.taken ? "Took" : "Skipped"}</span>${reasonSentence(c)}</span>
        <span class="after">${afterSentence(c, H)}</span></li>`).join("")}</ul>
        ${list.length > decShown ? `<button class="btn more" data-act="more-dec">Show more decisions</button>` : ""}` : `<div class="empty">Nothing here yet.</div>`}`;
  }

  // ---------- news bot ----------
  function moodWord(s) { return s > 0.5 ? "clearly good" : s > 0.15 ? "mildly good" : s >= -0.15 ? "neutral" : s >= -0.5 ? "mildly bad" : "clearly bad"; }
  function scaleSvg(score, entry, exit) {
    const W = 600, y = 18, x = (s) => ((clamp(s, -1, 1) + 1) / 2) * W;
    return `<svg viewBox="0 0 ${W} 46" preserveAspectRatio="none" role="img" aria-label="News score ${score.toFixed(2)} on a scale from −1 to +1; buys above +${entry}, sells below −${exit}">
      <rect x="0" y="${y - 4}" width="${x(-exit)}" height="8" fill="var(--loss-wash)"/>
      <rect x="${x(entry)}" y="${y - 4}" width="${W - x(entry)}" height="8" fill="var(--gain-wash)"/>
      <rect x="${x(-exit)}" y="${y - 1}" width="${x(entry) - x(-exit)}" height="2" fill="var(--rule)"/>
      <line x1="${x(score)}" x2="${x(score)}" y1="${y - 12}" y2="${y + 12}" stroke="var(--ink)" stroke-width="3" vector-effect="non-scaling-stroke"/>
      <text x="0" y="44">−1 very bad</text><text x="${x(-exit)}" y="44" text-anchor="middle">sell below −${exit}</text>
      <text x="${x(entry)}" y="44" text-anchor="middle">buy above +${entry}</text><text x="${W}" y="44" text-anchor="end">+1 very good</text>
    </svg>`;
  }

  function traceHtml(L, sym) {
    if (!L?.gates?.length) return "";
    const failed = L.gates.filter((g) => !g.passed);
    const sum = failed.length ? `Why: ${failed.map((g) => esc(NGATE[g.gate] || g.gate).toLowerCase()).join(", ")} stopped it` : `Why this decision: every check passed`;
    return `<details class="trace" data-sym="${esc(sym)}"${openTraces.has(sym) ? " open" : ""}><summary>${sum}</summary><ol>${L.gates.map((g) => `<li class="${g.passed ? "ok" : "no"}"><span>${esc(NGATE[g.gate] || g.gate)}</span>${g.detail ? `<code>${esc(g.detail)}</code>` : ""}</li>`).join("")}</ol>
      <p class="small">Size was limited by <b>${esc(BIND[L.binding] || L.binding || "—")}</b>; it ended at ${(L.final_weight * 100).toFixed(1)}% of its money.</p></details>`;
  }
  function stanceWords(n) {
    if (!n || n.n_clusters == null || !n.n_clusters) return "";
    return ` ${plural(n.n_clusters, "separate story", "separate stories")}: <b class="pos">${n.pos} good</b>, <b class="neg">${n.neg} bad</b>, ${n.neutral} neutral.`;
  }
  function regimeHtml() {
    const r = snap.regime;
    if (!r) return `<p class="sub">Not measured yet. It refreshes every hour from free public data.</p>`;
    const mode = snap.settings.regime_mode;
    return `<p class="sub">Right now the crypto market looks <b>${esc(zoneWords[r.zone] || r.zone)}</b>${r.score == null ? "" : `, a score of <b>${r.score.toFixed(0)}</b> out of 100 (below 40 is falling, 80 and up is rising)`}. Updated ${ago(r.ts)}. ${mode === "off" ? "For now it's only written down next to each decision; it doesn't change what the bot does." : `It ${mode === "block" ? "blocks new buys when the market is falling" : "lowers how much it may invest when the market is weak"}.`}</p>
      <ul class="regime">${r.components.map((c) => `<li class="${c.score == null ? "off" : ""}"><span>${esc(c.name)}</span><span class="bar" aria-hidden="true"><i style="width:${c.score == null ? 0 : clamp(c.score, 0, 100)}%"></i></span><b>${c.score == null ? "—" : c.score.toFixed(0)}</b><code>${esc(c.detail)}</code></li>`).join("")}</ul>
      <p class="small">Readings that are missing don't count; with fewer than 4 of the 6, it calls the market unclear.</p>`;
  }
  function jevHealth() {
    const st = snap.status; if (!st) return "";
    const [failed, total] = st.jev_window || [0, 0];
    const bits = [];
    bits.push(total ? `Jev answered ${total - failed} of its last ${total} questions.` : "Jev hasn't been asked anything recently.");
    if (st.jev_pending) bits.push(`${plural(st.jev_pending, "headline")} waiting for another try.`);
    if (snap.spiked) bits.push(`${plural(snap.spiked, "headline")} given up on after ${snap.settings.jev_max_attempts} failed tries.`);
    if (st.tokens_today?.[0]) bits.push(`${plural(st.tokens_today[0], "question")} today (${st.tokens_today[1].toLocaleString()} tokens in, ${st.tokens_today[2].toLocaleString()} out).`);
    return `<p class="small">${bits.join(" ")}</p>`;
  }

  function renderNews(el) {
    if (!snap) { el.innerHTML = `<p class="small">Loading…</p>`; return; }
    const set = snap.settings, acc = snap.account, ev = snap.events || [];
    const syms = new Set(snap.assets.map((a) => a.symbol));
    const counts = (e) => syms.has(e.asset) || e.asset === "market";
    const relevant = ev.filter(counts).length;
    const best = snap.assets.reduce((m, a) => Math.max(m, a.latest?.news?.score ?? -1), -1);
    const venue = snap.status?.live && !snap.status?.testnet ? "real money" : snap.status?.testnet ? "testnet" : "simulated money";
    const positions = acc?.positions || [];
    const paused = !!snap.control?.paused;
    el.innerHTML = `
      <p class="kicker">News bot <span class="venue-tag${venue === "real money" ? " live" : ""}">${venue}</span></p>
      <h1 class="hero">It reads crypto headlines and buys only when the news is clearly good and the price agrees.</h1>
      <p class="lede">A separate, slower bot. It has read <b>${ev.length}</b> headlines in the last ${set.retain_hours} hours, <b>${relevant}</b> of them about ${esc([...syms].join(", "))} or the whole market. ${snap.trades.length ? `It has made <b>${snap.trades.length}</b> trades.` : `It hasn't traded yet: no coin's news has been good enough. The best score right now is <b>${best.toFixed(2)}</b>; it needs <b>+${set.news_entry}</b>.`}</p>
      ${(() => { const p = posture(); if (!p) return ""; const [h, t] = postureWords(p); return `<div class="posture ${p.kind.toLowerCase()}" role="status"><b>${esc(h)}</b><span>${esc(t)}</span></div>`; })()}
      <ol class="rules">
        <li><b>Reads the news</b>Every ${set.interval_secs / 60 === 1 ? "minute" : `${set.interval_secs / 60} minutes`}, from ${set.feeds.length} crypto news sites.</li>
        <li><b>Jev reads each headline</b>Which coin is it about, what kind of event, good or bad for the price, and is it a new fact or opinion?</li>
        <li><b>Scores each coin</b>Recent headlines add up to a score from −1 to +1. Each one counts half as much after ${set.half_life_hours} hours.</li>
        <li><b>Decides</b>Buys when the score is above +${set.news_entry} and the price is above its 20-hour average. Sells below −${set.news_exit}, on a hack, or at a ${(set.stop_loss_pct * 100).toFixed(0)}% loss.</li>
      </ol>

      <section class="block" aria-labelledby="h-scores">
        <h2 id="h-scores">Where each coin stands</h2>
        <p class="sub">The black mark is the coin's news score right now. It only buys once the mark is inside the green zone, the price trend agrees, and every safety check passes. Open "Why" under a coin to see each check.</p>
        <div class="scales">${snap.assets.map((a) => {
          const L = a.latest; if (!L) return "";
          const s = L.news.score, mom = L.momentum;
          const held = positions.find((p) => p.asset === a.symbol);
          const review = snap.status?.review?.[a.symbol] || L.review;
          const reviewTxt = review ? `<span class="review">On hold for review: ${esc(review)}.</span> It won't buy ${esc(a.symbol)} until that clears. ` : "";
          const state = held ? `<mark>Holding ${qtyF(held.qty)} ${esc(a.symbol)}.</mark>` : review ? "" : L.news.hack_alert ? "Blocked: fresh hack alert." : s > set.news_entry && mom > 0 ? "<mark>Buy signal.</mark>" : s > set.news_entry ? "News is good enough, but the price is below its recent average, so it waits." : `News is ${moodWord(s)} (${s.toFixed(2)}), not strong enough to buy.`;
          const momTxt = mom == null ? "There isn't enough recent price history to judge the trend." : `Price is ${pc(Math.abs(mom) * 1e4, 1)} ${mom >= 0 ? "above" : "below"} its 20-hour average.`;
          return `<div class="scale"><div class="nm"><b>${esc(a.symbol)}</b><span>${px(L.price)}</span></div>${scaleSvg(s, set.news_entry, set.news_exit)}
            <p>${reviewTxt}${state} ${momTxt} ${plural(L.news.n_events, "headline")} counted.${stanceWords(L.news)}</p>${traceHtml(L, a.symbol)}</div>`;
        }).join("")}</div>
      </section>

      <section class="block" aria-labelledby="h-regime">
        <h2 id="h-regime">The market as a whole</h2>
        ${regimeHtml()}
      </section>

      <section class="block" aria-labelledby="h-acct">
        <h2 id="h-acct">Its money</h2>
        <p class="sub">${usd(acc.equity)} in total, ${usd(acc.cash)} of it cash, from ${usd(acc.starting_cash)} at the start (${susd(acc.equity - acc.starting_cash)}). ${positions.length ? `Holding ${positions.map((p) => `${qtyF(p.qty)} ${esc(p.asset)}`).join(", ")}.` : "Holding no coins."}</p>
        <div class="controls">
          <button class="btn" data-act="pause" ${busy ? "disabled" : ""}>${paused ? "Resume the news bot" : "Pause the news bot"}</button>
          <button class="btn ${flattenArmed ? "danger" : "ink"}" data-act="flatten" ${busy || !positions.length || acc.flatten_pending ? "disabled" : ""} title="${positions.length ? "Sell everything the news bot holds and pause it" : "The news bot holds nothing to sell"}">${acc.flatten_pending ? "Selling everything…" : flattenArmed ? `Confirm: sell ${plural(positions.length, "position")}` : "Sell everything and pause"}</button>
          ${flattenArmed ? `<button class="btn quiet" data-act="disarm">Cancel</button>` : ""}
        </div>
        <p class="small" style="margin-top:8px">${paused ? "Paused: it won't buy. Stop losses and selling still work." : "Pausing stops new buys; stop losses and selling keep working. These buttons only affect the news bot."}</p>
        ${snap.status?.stages ? `<ul class="health" aria-label="Last run, step by step">${snap.status.stages.map((s) => `<li class="${esc(s.state)}" title="${esc(s.detail)}">${esc(s.name)}</li>`).join("")}</ul>` : ""}
        ${jevHealth()}
      </section>

      <section class="block" aria-labelledby="h-heads">
        <h2 id="h-heads">Latest headlines and what Jev made of them</h2>
        <p class="sub">Greyed headlines are about other coins or topics, so they don't affect any score.</p>
        <ul class="heads">${(() => { const oc = new Map(); for (const e of ev) { const k = e.cluster_id || e.id; oc.set(k, (oc.get(k) || 0) + 1); } return ev.slice(0, headShown).map((e) => { const outlets = oc.get(e.cluster_id || e.id) || 1;
          const u = safeUrl(e.id);
          const about = e.asset === "market" ? "the whole market" : e.asset === "other" ? "something it doesn't trade" : e.asset;
          return `<li class="${counts(e) ? "counts" : "off"}"><span class="tm">${hm(e.published || e.analyzed_at)}</span>
            ${u ? `<a href="${esc(u)}" target="_blank" rel="noopener noreferrer">${esc(e.title)}</a>` : `<span>${esc(e.title)}</span>`}
            <p>${esc(e.source)}${outlets > 1 ? ` <span class="outlets">same story at ${outlets} outlets${snap.settings.dedupe_clusters ? ", counted once" : ""}</span>` : ""}. About <b>${esc(about)}</b>; Jev reads it as ${moodWord(e.sentiment)} ${esc(e.event)} news${counts(e) ? ` (${e.sentiment >= 0 ? "+" : ""}${e.sentiment.toFixed(2)}), ${Math.round(e.new_info * 100)}% likely a new fact` : ""}.</p></li>`;
        }).join(""); })()}</ul>
        ${ev.length > headShown ? `<button class="btn more" data-act="more-heads">Show more headlines</button>` : ""}
      </section>`;
  }

  // ---------- scorecard ----------
  const pct0 = (x) => (x == null || !isFinite(x) ? "—" : `${Math.round(x * 100)}%`);
  const spct = (x, d = 1) => (x == null || !isFinite(x) ? "—" : `${x > 0 ? "+" : x < 0 ? "−" : ""}${(Math.abs(x) * 100).toFixed(d)}%`);
  const sig = (x, d = 2) => (x == null || !isFinite(x) ? "—" : `${x > 0 ? "+" : x < 0 ? "−" : ""}${Math.abs(x).toFixed(d)}`);
  // The null test is the honest part: say "could be luck" whenever it is.
  function luck(p, lowN) {
    if (lowN) return `<span class="verdict-tag thin">too few to tell</span>`;
    if (p == null) return `<span class="verdict-tag thin">not tested</span>`;
    return p > 0.1 ? `<span class="verdict-tag luck">could be luck</span>` : `<span class="verdict-tag real">unlikely to be luck</span>`;
  }
  const TAGS = { WORKED: "Worked", FAILED_STOP: "Hit the stop", FAILED_FADE: "Rose, then gave it back", CHOPPY: "Went nowhere, choppily", NEUTRAL: "Barely moved" };
  const DIMS = { event: "Kind of news", source: "News site", asset: "Coin", regime: "Market mood" };

  function renderScore(el) {
    if (!snap) { el.innerHTML = `<p class="small">Loading…</p>`; return; }
    const sc = snap.scorecard;
    const head = `<p class="kicker">News bot</p>
      <h1 class="hero">Is reading the news actually helping?</h1>
      <p class="lede">Every headline, every decision and every trade a safety check blocked is checked again 1, 4 and 24 hours later, against what the price really did. Stories that overlap in time are merged first, so one busy morning can't count as twenty proofs. Each result is then compared with shuffled data: if shuffling does about as well, it says <b>could be luck</b>.</p>`;
    if (!sc) {
      el.innerHTML = head + `<div class="empty"><b>Nothing checked yet.</b> The first results appear about a day after the news bot starts reading headlines${snap.status ? ` (it's on read ${snap.status.tick.toLocaleString()})` : ""}. For the long write-up run <code>jev-trader report</code>.</div>`;
      return;
    }
    const d = sc.decisions, st = sc.stop;
    const best = sc.headlines.filter((r) => !r.low_n && r.p != null && r.p <= 0.1).sort((a, b) => (b.ic ?? 0) - (a.ic ?? 0))[0];
    const verdict = best ? `So far, the news scores line up with the price ${best.h} hours later better than chance would explain.`
      : sc.headlines.some((r) => !r.low_n) ? "So far, nothing here beats chance. The news scores don't yet predict the price better than shuffled data."
      : "There aren't enough separate stories yet to judge anything. Treat every number below as a first glimpse.";
    const total = d.tags ? Object.values(d.tags).reduce((a, b) => a + b, 0) : 0;
    el.innerHTML = head + `
      <p class="day"><span class="venue-tag">${esc(sc.label)}</span> ${plural(sc.n_settled, "record")} checked${sc.n_failed ? `, ${sc.n_failed} couldn't be checked` : ""}. ${verdict}</p>

      <section class="block" aria-labelledby="h-sc-heads">
        <h2 id="h-sc-heads">Do good headlines come before rising prices?</h2>
        <p class="sub">"Agreement" runs from −1 to +1: 0 means Jev's reading of the news says nothing about where the price goes next, +1 would mean it ranks every move perfectly. In markets, even +0.05 that holds up is worth having. Greyed rows have fewer than 30 separate stories.</p>
        <table class="sc"><thead><tr><th>After</th><th class="num">Separate stories</th><th class="num">Agreement</th><th class="num">Right direction</th><th>Verdict</th></tr></thead><tbody>
        ${sc.headlines.map((r) => `<tr class="${r.low_n ? "low" : ""}"><td>${r.h} hour${r.h === 1 ? "" : "s"}</td><td class="num">${r.n_clusters.toLocaleString()} <small>of ${r.n_raw.toLocaleString()} headlines</small></td><td class="num">${sig(r.ic, 3)}</td><td class="num">${pct0(r.hit_rate)}</td><td>${luck(r.p, r.low_n)}${r.null_pct != null ? ` <small>beats ${r.null_pct.toFixed(0)}% of shuffles</small>` : ""}</td></tr>`).join("")}
        </tbody></table>
      </section>

      <div class="grid2">
        <section class="panel" aria-labelledby="h-sc-dec">
          <h2 id="h-sc-dec">How its decisions turned out</h2>
          <p class="sub">${d.n ? `${plural(d.n, "decision")} to buy or sell, in ${plural(d.n_clusters, "separate episode")}. ${d.hit_rate != null ? `<b>${pct0(d.hit_rate)}</b> went the way it bet` : "No hit rate yet"}${d.expectancy != null ? `, an average of <b class="${cls(d.expectancy)}">${spct(d.expectancy, 2)}</b> per decision after 24 hours` : ""}.${d.low_n ? " Still too few to trust." : ""}` : "No decision has been checked yet."}</p>
          ${total ? `<ul class="tags">${Object.entries(d.tags).sort((a, b) => b[1] - a[1]).map(([k, v]) => `<li><span class="bar" aria-hidden="true"><i style="width:${(v / total) * 100}%"></i></span><b>${v}</b><span>${esc(TAGS[k] || k)}</span></li>`).join("")}</ul>` : ""}
          ${d.regime_mismatch ? `<p class="small">${plural(d.regime_mismatch, "failure")} happened after the whole market's mood changed between buying and selling.</p>` : ""}
        </section>
        <section class="panel" aria-labelledby="h-sc-stop">
          <h2 id="h-sc-stop">Is the stop loss in the right place?</h2>
          ${st.n_winners ? `<p class="sub">Of <b>${st.n_winners}</b> buys that were up after 24 hours, <b>${pct0(st.share_below_stop)}</b> first dipped past the ${(st.stop_pct * 100).toFixed(1)}% stop on the way. ${st.share_below_stop > 0.2 ? "That's a lot of good trades the stop would have cut short; it may be too tight." : "Few good trades dipped that far, so the stop isn't cutting many winners short."}${st.low_n ? " Too few to act on yet." : ""}</p>
          <dl class="figures"><dt>Typical dip before winning</dt><dd>${spct(st.mae_q50, 2)}</dd><dt>Deep dip (1 in 10 winners)</dt><dd>${spct(st.mae_q90, 2)}</dd><dt>Stop loss today</dt><dd>−${(st.stop_pct * 100).toFixed(1)}%</dd></dl>` : `<p class="sub">No winning buys have been checked yet.</p>`}
        </section>
      </div>

      <section class="block" aria-labelledby="h-sc-gates">
        <h2 id="h-sc-gates">What each safety check cost or saved</h2>
        <p class="sub">When a check stopped a buy, the buy is followed anyway as if it had happened. "Missed" means it would have made money; "saved" means it would have lost.</p>
        ${sc.gates.length ? `<table class="sc"><thead><tr><th>Check</th><th class="num">Buys stopped</th><th class="num">Missed</th><th class="num">Saved</th><th class="num">Average move of stopped buys</th></tr></thead><tbody>
          ${sc.gates.map((g) => `<tr class="${g.low_n ? "low" : ""}"><td>${esc(NGATE[g.gate] || g.gate)}</td><td class="num">${g.n}</td><td class="num">${g.missed}</td><td class="num">${g.correctly_blocked}</td><td class="num ${cls(g.mean_ret)}">${spct(g.mean_ret, 2)}</td></tr>`).join("")}
        </tbody></table>` : `<div class="empty">No stopped buys have been checked yet.</div>`}
      </section>

      ${sc.cohorts.length ? `<section class="block" aria-labelledby="h-sc-coh">
        <h2 id="h-sc-coh">Which kinds of news work</h2>
        <p class="sub">The same 4-hour check, split up. Groups with fewer than 5 separate stories are hidden; fewer than 30 are greyed.</p>
        <table class="sc"><thead><tr><th>Group</th><th class="num">Separate stories</th><th class="num">Agreement</th><th class="num">Right direction</th><th>Verdict</th></tr></thead><tbody>
          ${sc.cohorts.map((c) => `<tr class="${c.low_n ? "low" : ""}"><td>${esc(DIMS[c.dim] || c.dim)}: <b>${esc(c.key)}</b></td><td class="num">${c.n_clusters}</td><td class="num">${sig(c.ic, 3)}</td><td class="num">${pct0(c.hit_rate)}</td><td>${luck(c.p, c.low_n)}</td></tr>`).join("")}
        </tbody></table></section>` : ""}

      <p class="small" style="margin-top:28px">These are suggestions, never automatic changes. Only you edit the config, and <code>jev-trader backtest --compare</code> checks a change before you make it.</p>`;
  }

  // ---------- render ----------
  function render() {
    renderRail();
    renderNotices();
    document.title = `${VIEWS[view]} · Jev trader`;
    const el = $("#view");
    ({ today: renderToday, trades: renderTrades, decisions: renderDecisions, news: renderNews, score: renderScore })[view](el);
  }

  // ---------- actions ----------
  async function post(path, body) {
    busy = true; actionError = null; render();
    try {
      const r = await fetch(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body || {}) });
      if (!r.ok) throw new Error((await r.text()) || `The server answered ${r.status}.`);
      const control = await r.json();
      if (snap) { snap.control = control; if (path.endsWith("flatten") && snap.account) snap.account.flatten_pending = true; }
    } catch (e) {
      actionError = e.message === "Failed to fetch" ? "The dashboard server isn't reachable." : e.message;
    } finally { busy = false; render(); pollSnap(); }
  }
  const disarm = () => { flattenArmed = false; clearTimeout(armTimer); };

  document.addEventListener("click", (e) => {
    const b = e.target.closest("[data-act],[data-tf],[data-df]");
    if (!b) return;
    if (b.dataset.tf) tradeFilter = b.dataset.tf;
    else if (b.dataset.df) { decFilter = b.dataset.df; decShown = 50; }
    else {
      const a = b.dataset.act;
      if (a === "more-dec") decShown += 50;
      if (a === "more-heads") headShown += 20;
      if (a === "dismiss") actionError = null;
      if (a === "disarm") disarm();
      if (a === "pause") return post("/api/control/pause", { paused: !snap?.control?.paused });
      if (a === "flatten") {
        if (!flattenArmed) { flattenArmed = true; armTimer = setTimeout(() => { disarm(); render(); }, 6000); }
        else { disarm(); return post("/api/control/flatten"); }
      }
    }
    render();
  });
  // Story bodies are built on demand and the open set survives every refresh.
  document.addEventListener("toggle", (e) => {
    const d = e.target;
    if (d.classList?.contains("trace")) { d.open ? openTraces.add(d.dataset.sym) : openTraces.delete(d.dataset.sym); return; }
    if (!d.classList?.contains("trade")) return;
    const id = d.dataset.id;
    if (d.open) {
      openTrades.add(id);
      if (!d.querySelector(".story")) {
        const t = (scalp?.trades || []).find((x) => (x.candidate_id || x.opened) === id);
        if (t) d.insertAdjacentHTML("beforeend", storyHtml(t));
      }
    } else openTrades.delete(id);
  }, true);

  // ---------- routing + data ----------
  function route() {
    const v = (location.hash || "#today").slice(1);
    view = VIEWS[v] ? v : "today";
    window.scrollTo(0, 0);
    render();
  }
  window.addEventListener("hashchange", route);

  let etag = null;
  async function pollSnap() {
    try {
      const r = await fetch("/api/snapshot", { headers: etag ? { "If-None-Match": etag } : {}, cache: "no-store" });
      if (r.status === 304) { conn = "live"; return false; }
      if (!r.ok) throw new Error(String(r.status));
      etag = r.headers.get("ETag");
      const s = await r.json();
      snap = s; skew = Date.parse(s.now) - Date.now(); conn = "live";
      if (flattenArmed && !s.account?.positions?.length) disarm();
      return true;
    } catch { conn = "lost"; return true; }
  }
  let scalpKey = "";
  async function pollScalp() {
    try {
      const r = await fetch("/api/scalp", { cache: "no-store" });
      if (!r.ok) throw new Error(String(r.status));
      scalp = await r.json();
      // Re-render only when something a reader would notice has changed.
      const st = scalp.status || {};
      const key = [scalp.summary?.candidates, scalp.trades?.length, st.assets?.map((a) => `${a.symbol}${a.long_block}${a.short_block}${!!a.position}${(a.features?.activity ?? 0).toFixed(1)}${(a.features?.flow ?? 0).toFixed(1)}`).join(), Math.floor(Date.parse(st.ts || 0) / 10000)].join("|");
      const changed = key !== scalpKey; scalpKey = key;
      return changed;
    } catch { return false; }
  }
  (async function loop() {
    const [a, b] = await Promise.all([pollSnap(), pollScalp()]);
    // Don't redraw under the reader's pointer while they're selecting text.
    const newsy = view === "news" || view === "score";
    if ((a && newsy) || (b && !newsy) || conn === "lost") { if (!String(getSelection())) render(); }
    else renderRail();
    setTimeout(loop, document.hidden ? 15000 : 4000);
  })();
  setInterval(() => { if (snap || scalp) renderRail(); }, 1000);
  route();
})();
