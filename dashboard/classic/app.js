"use strict";
(() => {
  const $ = (s) => document.querySelector(s);
  const VIEWS = {
    wire: "The wire",
    run: "Run list",
    pipeline: "Pipeline",
    trades: "Trades",
    score: "Scorecard",
    scalp: "Scalper",
  };

  // ---------- state ----------
  let snap = null;
  let scalp = null; // /api/scalp, polled only while the Scalper view is open
  let scalpLost = false;
  let skew = 0; // server clock - local clock (ms)
  let conn = "connecting";
  let view = "wire";
  let tab = "all";
  let assetFilter = "all";
  let shown = 60;
  let focus = null; // {ids:Set, label, key}
  let known = new Set();
  let firstWire = true;
  let flattenArmed = false;
  let armTimer = null;
  let busy = false;
  let actionError = null;
  let scrollToFocus = false;

  const store = {
    get(k, d) { try { const v = localStorage.getItem(k); return v === null ? d : JSON.parse(v); } catch { return d; } },
    set(k, v) { try { localStorage.setItem(k, JSON.stringify(v)); } catch { /* storage unavailable */ } },
  };
  let lastRead = store.get("jev.lastRead", null);
  let readIds = new Set(store.get("jev.readIds", []));

  // ---------- format ----------
  const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  const safeUrl = (u) => (/^https?:\/\//i.test(u || "") ? u : null);
  const usd = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" });
  const fmtUsd = (n) => (n == null || !isFinite(n) ? "—" : usd.format(n));
  const fmtPx = (n) => n == null ? "—" : n.toLocaleString("en-US", { minimumFractionDigits: n < 1 ? 4 : 2, maximumFractionDigits: n < 1 ? 4 : 2 });
  const fmtQty = (n) => n.toLocaleString("en-US", { maximumFractionDigits: n < 1 ? 6 : 4 });
  const sign = (n, d = 2) => (n > 0 ? "+" : n < 0 ? "−" : "") + Math.abs(n).toFixed(d);
  const pct = (x, d = 2) => (x == null || !isFinite(x) ? "—" : sign(x * 100, d) + "%");
  const cls = (x) => (x > 0 ? "pos" : x < 0 ? "neg" : "");
  const t = (iso) => new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const tsec = (iso) => new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  const now = () => Date.now() + skew;
  const ago = (iso) => {
    const s = Math.max(0, Math.round((now() - Date.parse(iso)) / 1000));
    if (s < 60) return `${s}s ago`;
    if (s < 3600) return `${Math.floor(s / 60)}m ago`;
    if (s < 86400) return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m ago`;
    return `${Math.floor(s / 86400)}d ago`;
  };
  const dur = (s) => { s = Math.max(0, Math.round(s)); const m = Math.floor(s / 60); return `${m}:${String(s % 60).padStart(2, "0")}`; };
  const shortSource = (s) => String(s || "").split(":")[0].replace(/\.com\b/i, "").replace(/\s+News$/i, "").trim();
  const host = (u) => { try { return new URL(u).hostname.replace(/^www\./, ""); } catch { return ""; } };

  // ---------- derived ----------
  function liveness() {
    if (!snap || !snap.status) return { s: "never" };
    const st = snap.status;
    const last = st.last_tick_at || st.started_at;
    const age = (now() - Date.parse(last)) / 1000;
    if (snap.bot_pid_running === false) return { s: "stopped", age, last };
    if (age > st.interval_secs * 2.5 + 30) return { s: "stale", age, last };
    return { s: "alive", age, last };
  }

  function fillsByEvent() {
    const m = new Map();
    for (const f of snap?.trades || []) for (const id of f.events || []) {
      if (!m.has(id)) m.set(id, []);
      m.get(id).push(f);
    }
    return m;
  }

  function classify(ev, fbe) {
    const fills = fbe.get(ev.id);
    if (fills && fills.length) {
      const f = fills[0];
      return { kind: f.side === "buy" ? "buy" : "sell", stamp: `Run · ${f.side}`, small: f.asset, fill: f, group: "ran" };
    }
    if (ev.event === "hack" && ev.event_conf > 0.8 && ev.new_info > 0.7 && ev.asset !== "other")
      return { kind: "alert", stamp: "Alert", small: `${ev.asset} hack`, group: "ran" };
    if (ev.event === "noise") return { kind: "spiked", stamp: "Spiked", small: "opinion / recap", group: "spiked" };
    if (ev.asset === "other") return { kind: "spiked", stamp: "Spiked", small: "off the desk", group: "spiked" };
    if (ev.new_info < 0.5) return { kind: "spiked", stamp: "Spiked", small: "no new fact", group: "spiked" };
    return { kind: "filed", stamp: "Filed", small: `${sign(ev.sentiment)} ${ev.asset === "market" ? "all" : ev.asset}`, group: "filed" };
  }

  // Same-story clusters (5.1): how many outlets carried each story.
  function outletCounts() {
    const m = new Map();
    for (const e of snap?.events || []) { const k = e.cluster_id || e.id; m.set(k, (m.get(k) || 0) + 1); }
    return m;
  }

  const isUnread = (ev) => lastRead && !readIds.has(ev.id) && ev.analyzed_at > lastRead;

  // ---------- self-explanation: guides and terms ----------
  let hiddenGuides = new Set(store.get("jev.guides.hidden", []));
  const term = (label, def) => `<span class="term" tabindex="0" title="${esc(def)}">${label}</span>`;
  const stampEx = (kind, label, small) => `<span class="stamp ex ${kind}">${label}<small>${small}</small></span>`;
  // A flow of steps joined by arrows; a step flagged `aside` is a side input drawn under the flow.
  const flow = (steps) => {
    const lastMain = steps.map((x) => !x[2]).lastIndexOf(true);
    return `<ol class="flowstrip">${steps.map(([n, d, aside], i) => `<li class="${aside ? "aside" : i === lastMain ? "end" : ""}"><b>${n}</b><span>${d}</span></li>`).join("")}</ol>`;
  };

  function renderSub(sub, deskSlug) {
    if (!sub.querySelector(".subtext")) sub.innerHTML = `<span class="deskslug"></span><span class="subtext"></span><button type="button" class="linkbtn guide-open" data-act="guide-show">How to read this page</button>`;
    sub.querySelector(".deskslug").textContent = deskSlug;
    sub.querySelector(".guide-open").hidden = !hiddenGuides.has(view);
  }

  const GUIDES = {
    wire: () => `<h2>How to read the wire</h2>
      <p>Every slip is one headline from CoinDesk, Cointelegraph or Decrypt. Jev reads it and answers four typed questions; the news desk then stamps a verdict in the margin.</p>
      <div class="anatomy"><p class="slugex">BTC-MACRO · COINDESK · 02:13 AM</p><p>the coin it is about, the kind of event · the outlet · when it was published. <b>MKT</b> means the whole market, <b>OFF</b> a coin this desk doesn't trade.</p>
      <p class="readex"><b>macro</b> 0.95 · sentiment <b>+0.48</b> (conf 0.80) · new fact <b>0.72</b></p><p>Jev's confidence in the event type · expected price impact from −1 to +1, with its confidence · the probability it reports a new, verifiable fact rather than opinion.</p></div>
      <div class="stamps">${[["buy", "Run · buy", "BTC", "caused a paper buy"], ["sell", "Run · sell", "ETH", "caused a sale"], ["alert", "Alert", "SOL hack", "a fresh hack on a traded coin: buying it is blocked"], ["filed", "Filed", "+0.48 BTC", "counted in that coin's news score"], ["spiked", "Spiked", "off the desk", "ignored: other coin, opinion, or no new fact"]].map(([k, l, sm, d]) => `<div class="stampdef">${stampEx(k, l, sm)}<p>${d}</p></div>`).join("")}</div>
      <p class="note">Yellow paper means you haven't looked at it yet. Click a coin in the run list to light up the copy behind its position.</p>`,
    run: () => `<h2>How each coin is decided</h2>
      <p>Once a minute, every coin gets a ${term("news score", "Recent headlines about the coin, each weighted by Jev's confidence and by how likely it is a new fact, fading by half every 2 hours. Squashed to −1..+1.")} from the wire and a ${term("momentum", "Last 1-hour close compared with its 20-hour average. Positive = price is above its recent average.")} reading from Binance prices. The gauge under each coin shows the score as a needle:</p>
      ${flow([["Sell zone", `score below ${snap ? sign(-snap.settings.news_exit) : "−0.30"}, or a hack alert`], ["Hold zone", "keep whatever is held"], ["Buy zone", `score above ${snap ? sign(snap.settings.news_entry) : "+0.30"} and momentum positive`]])}
      <p class="note">The <b>decision trace</b> lists every risk check a decision went through; a pink one blocked it. <b>Limited by</b> names the cap that set the final size.</p>`,
    pipeline: () => `<h2>What happens every tick</h2>
      ${flow([["Feeds", "pull headlines from 3 RSS sources"], ["Jev", "read each new headline: coin, event, sentiment, new fact"], ["Prices", "1-hour Binance candles per coin"], ["Signals", "news score + momentum → target"], ["Risk", "caps, stop loss, daily kill switch, circuit breakers"], ["Execution", "paper fills (simulated)"]])}
      <p class="note">Dots: <span class="sdot"></span> ok · <span class="sdot degraded"></span> degraded · <span class="sdot error"></span> error · <span class="sdot skipped"></span> skipped. The strip shows the last 60 ticks, oldest on the left; hover a cell for its detail.</p>`,
    trades: () => `<h2>What a fill is</h2><p>Every row is a simulated order on the news desk: no exchange is involved. Each pays a 0.1% fee and 0.05% slippage so the results aren't flattered. <b>Source copy</b> jumps to the headlines that caused it.</p>`,
    score: () => `<h2>How the news desk is graded</h2>
      <p>After the fact, every headline and decision is scored against what the price actually did 1, 4 and 24 hours later.</p>
      <div class="kv glossary"><div><span>${term("IC", "Rank correlation between Jev's reading and the later price move. 0 means no relation; 0.05 is already meaningful in markets.")}</span><b>does the reading line up with the move?</b></div>
      <div><span>${term("Null pct", "Where the result ranks against the same data shuffled many times. Below about 90 it is indistinguishable from luck.")}</span><b>is it better than luck?</b></div>
      <div><span>${term("Hit rate", "Share of times the direction was right.")}</span><b>how often was the direction right?</b></div>
      <div><span>Greyed rows</span><b>fewer than 30 samples: don't trust them yet</b></div></div>`,
    scalp: () => `<h2>How the scalp desk works</h2>
      <p>A separate process trading in seconds to minutes, <b>long and short</b>. The market's own order flow decides; news only informs. It holds ${scalp?.status ? fmtUsd(scalp.status.starting_cash) : "its"} cash plus a small neutral inventory of each coin (${scalp?.status?.venue === "binance-testnet" ? "fake money on the Binance testnet" : scalp?.status?.venue === "paper" ? "simulated" : "real money"}).</p>
      <p>A <b>long</b> buys extra coin and sells it back. A <b>short</b> sells part of the inventory and buys it back; spot can't borrow, so a short never sells more than the inventory. <b>Trade P&amp;L</b> is reported apart from <b>inventory drift</b> (the inventory moving with the market). Costs are counted at real Binance fees, so in quiet markets it mostly waits.</p>
      ${flow([["Market stream", "every trade and the best bid/ask, live from Binance"], ["Gates", "all must pass, mirrored per side: busy market, buyers (or sellers) in control, price rising (or falling), moves big enough to pay fees"], ["Risk limits", "size, positions, trades per hour, daily loss"], ["Jev review", "reads recent headlines; can veto or halve the trade, never starts one"], ["Order", "market order, then take profit, a stop that trails once in profit, or a 10-minute time stop"], ["News context", "background: good news lowers the bar for longs and raises it for shorts (and vice versa); a hack alert blocks longs only", true]])}
      <div class="kv glossary">
        <div><span>${term("Activity", "Trades per second over the last 30s, divided by the normal rate over 30 minutes. 2× = twice as busy as usual.")}</span><b>how busy vs normal</b></div>
        <div><span>${term("Flow", "Aggressive buy volume minus aggressive sell volume, over total, last 30s. +1 = only buyers hitting the ask.")}</span><b>who is pushing: buyers or sellers</b></div>
        <div><span>${term("Book", "Size waiting at the best bid vs the best ask, smoothed. Positive = more buyers queued.")}</span><b>which side is queued up</b></div>
        <div><span>${term("Push 60s", "Mid-price change over the last minute, in bps (1 bp = 0.01%).")}</span><b>is price already rising</b></div>
        <div><span>${term("Exp. move / hurdle", "Typical price move over the holding period (from recent volatility) vs the move needed to beat fees: cost × safety multiple.")}</span><b>can a trade even pay its fees</b></div>
        <div><span>${term("Markout", "What the price did 1, 5 and 10 minutes after a candidate, minus the round-trip cost. Recorded whether the trade was taken or not.")}</span><b>was the signal worth anything</b></div>
      </div>`,
  };

  function guideHtml() {
    if (hiddenGuides.has(view) || !GUIDES[view]) return "";
    return `<section class="guide" aria-label="How to read this page">${GUIDES[view]()}<button type="button" class="btn btn-quiet guide-close" data-act="guide-hide" aria-label="Hide this guide">Got it, hide</button></section>`;
  }

  const WHY = {
    "warm-up": "building a baseline of normal activity",
    feed: "market data is stale",
    spread: "spread too wide",
    burst: "market is quiet",
    flow: "buyers aren't in control",
    book: "order book doesn't lean to buyers",
    push: "price isn't rising",
    "no chase": "already ran up; not chasing",
    cost: "moves too small to pay fees",
    news: "hack alert",
    flat: "already holding it",
    "max positions": "position limit reached",
    cooldown: "pausing after a loss",
    "trades/hour": "hourly trade limit reached",
    "daily limit": "daily loss limit hit",
    exchange: "exchange balance check failed",
    cash: "not enough cash",
    inventory: "not enough coin inventory to sell",
    "jev review": "Jev vetoed on the news",
  };
  const why = (g) => (g ? WHY[g] || g : "—");

  // ---------- render: rail ----------
  function scalpLive() {
    if (!scalp) return { s: "connecting" };
    if (!scalp.available) return { s: "never" };
    const age = (now() - Date.parse(scalp.status.ts)) / 1000;
    if (scalp.running === false) return { s: "stopped", age };
    if (age > 15) return { s: "stale", age };
    return { s: "alive", age };
  }
  const VENUE = { paper: "Paper · simulated", "binance-testnet": "Testnet · fake money", binance: "LIVE · real money" };

  function renderBeat() {
    const l = liveness();
    const st = snap?.status;
    let title = "Connecting…";
    if (snap) {
      if (l.s === "never") title = "Not started";
      else if (l.s === "stopped") title = `Stopped · last tick ${ago(l.last)}`;
      else if (l.s === "stale") title = `Silent · last tick ${ago(l.last)}`;
      else {
        const next = st.next_tick_at ? (Date.parse(st.next_tick_at) - now()) / 1000 : null;
        title = `${st.paused ? "Paused · " : ""}tick ${st.tick.toLocaleString()} · ${next != null && next > 0 ? "next " + dur(next) : "working…"}`;
      }
    }
    if (conn === "lost") title = "Dashboard offline";
    const nv = st?.testnet ? "Testnet · fake money" : st?.live ? "LIVE · real money" : "Paper · simulated";
    const bn = $("#beat-news");
    bn.dataset.s = l.s;
    bn.querySelector(".line").textContent = title;
    bn.querySelector(".venue").textContent = nv;
    bn.querySelector(".venue").classList.toggle("live", !!st?.live && !st?.testnet);

    const sl = scalpLive(), S = scalp?.status;
    const bs = $("#beat-scalp");
    bs.dataset.s = sl.s === "connecting" ? "never" : sl.s;
    bs.querySelector(".line").textContent =
      sl.s === "connecting" ? "Connecting…" : sl.s === "never" ? "Not started" : sl.s === "stopped" ? `Stopped · ${ago(S.ts)}` : sl.s === "stale" ? `Silent · ${ago(S.ts)}` :
      S.assets?.some((a) => a.position) ? `In a trade · ${fmtUsd(S.equity)}` : `Watching · ${fmtUsd(S.equity)}`;
    bs.querySelector(".venue").textContent = S ? VENUE[S.venue] || S.venue : "";
    bs.querySelector(".venue").classList.toggle("live", S?.venue === "binance");
    // The desk headings in the nav carry the same dot.
    $("#desk-news").dataset.s = bn.dataset.s;
    $("#desk-scalp").dataset.s = bs.dataset.s;
    $("#desk-news .tag").textContent = st?.testnet ? "testnet" : st?.live ? "LIVE" : "paper";
    $("#desk-scalp .tag").textContent = S ? ({ paper: "paper", "binance-testnet": "testnet", binance: "LIVE" }[S.venue] || "") : "";
    $("#beat-synth").hidden = !snap?.synthetic;
  }

  // ---------- render: bar ----------
  function renderBar() {
    $("#view-title").textContent = VIEWS[view];
    const deskSlug = view === "scalp" ? "Scalp desk" : "News desk";
    document.title = `${VIEWS[view]} · Jev Wire Desk`;
    document.querySelectorAll(".rail ul a").forEach((a) => (a.dataset.view === view ? a.setAttribute("aria-current", "page") : a.removeAttribute("aria-current")));
    const sub = $("#view-sub");
    renderSub(sub, deskSlug);
    const subText = sub.querySelector(".subtext");
    if (!snap) { subText.textContent = ""; }
    else if (view === "wire") {
      const today = new Date().toDateString();
      const n = snap.events.length;
      const todayN = snap.events.filter((ev) => new Date(ev.analyzed_at).toDateString() === today).length;
      subText.textContent = n ? `${n} items on the wire from the last ${snap.settings.retain_hours}h · ${todayN} today` : "No copy on the wire yet";
    } else if (view === "run") subText.textContent = "Why each asset is held or flat, traced back to the copy";
    else if (view === "pipeline") subText.textContent = "Every process and stage, tick by tick";
    else if (view === "score") subText.textContent = snap.scorecard ? `${snap.scorecard.label} · n=${snap.scorecard.n_settled} settled records` : "Nothing settled yet";
    else if (view === "scalp") subText.textContent = "Order flow decides · Jev reviews entries against the news · separate process and ledger";
    else subText.textContent = `${snap.trades.length} ${snap.status?.live ? "Binance" : "paper"} fills on record`;

    // Pause/Flatten act on the news bot only; the scalper has no browser controls.
    document.querySelector(".controls").hidden = view === "scalp";
    const paused = !!snap?.control?.paused;
    const pb = $("#btn-pause");
    pb.innerHTML = paused ? '<svg><use href="#i-play"/></svg><span>Resume news desk</span>' : '<svg><use href="#i-pause"/></svg><span>Pause news desk</span>';
    pb.title = paused ? "Allow the news desk to buy again" : "Stop new buys on the news desk. Stops, exits and the kill switch keep working.";
    pb.disabled = busy || !snap;

    const fb = $("#btn-flatten");
    const npos = snap?.account?.positions?.length || 0;
    const pending = !!snap?.account?.flatten_pending;
    fb.disabled = busy || !snap || npos === 0 || pending;
    fb.title = npos === 0 ? "The news desk holds nothing to sell" : "Sell every news-desk position and pause it (asks you to confirm)";
    fb.classList.toggle("btn-danger", flattenArmed);
    fb.classList.toggle("btn-ink", !flattenArmed);
    fb.innerHTML = pending ? '<svg><use href="#i-flat"/></svg><span>Flatten queued…</span>'
      : flattenArmed ? `<svg><use href="#i-flat"/></svg><span>Confirm: sell ${npos} position${npos === 1 ? "" : "s"}</span>`
      : '<svg><use href="#i-flat"/></svg><span>Flatten news desk</span>';
    $("#btn-flatten-cancel").hidden = !flattenArmed;
  }

  function renderPosture() {
    const el = $("#posture");
    const p = view === "scalp" ? null : snap?.status?.posture;
    if (!p) { el.innerHTML = ""; return; }
    const head = p.kind === "ENTRIES_ALLOWED" ? `Entries allowed · ceiling ${(p.ceiling * 100).toFixed(0)}%` : p.kind === "EXITS_ONLY" ? "Exits only" : "Review";
    const until = p.until ? ` until ${new Date(p.until).toLocaleString([], { weekday: "short", hour: "2-digit", minute: "2-digit" })}` : "";
    el.innerHTML = `<div class="posture ${p.kind.toLowerCase()}"><b>${esc(head)}</b>${p.reason ? `<span>${esc(p.reason)}${esc(until)}</span>` : until ? `<span>${esc(until)}</span>` : ""}</div>`;
  }

  function renderNotices() {
    const out = [];
    const l = liveness();
    if (snap?.synthetic) out.push(["bad", "<b>Synthetic data.</b> This data directory is marked as a demo: headlines, Jev readings, prices and fills are not real."]);
    if (conn === "lost") out.push(["bad", `<b>Lost connection to the dashboard server.</b> Showing the snapshot from ${snap ? tsec(snap.now) : "—"}; retrying.`]);
    if (actionError) out.push(["bad", `<b>That didn't go through.</b> ${esc(actionError)} The bot was not changed.`, '<button class="btn" data-act="dismiss">Dismiss</button>']);
    if (snap && view !== "scalp" && (l.s === "stale" || l.s === "stopped"))
      out.push(["bad", `<b>The bot ${l.s === "stopped" ? "process has stopped" : "has gone quiet"}.</b> Last tick ${ago(l.last)} (${tsec(l.last)}). Check the terminal running <code>jev-trader run</code>.`]);
    if (snap?.account?.halted_on && snap.account.halted_on === new Date(now()).toISOString().slice(0, 10))
      out.push(["bad", `<b>Daily loss limit hit (−${(snap.settings.daily_loss_limit_pct * 100).toFixed(0)}%).</b> The bot stays flat until the next UTC day.`]);
    const jw = snap?.status?.jev_window;
    if (jw && jw[1] >= 4 && jw[0] / jw[1] > 0.5) out.push(["bad", `<b>Jev circuit breaker open.</b> ${jw[0]} of ${jw[1]} recent calls failed. New entries are blocked; exits still run.`]);
    if (snap?.account?.flatten_pending) out.push(["warn", "<b>Flatten queued.</b> The bot sells everything within a second or two of its next check."]);
    else if (snap?.control?.paused) out.push(["warn", "<b>Trading is paused.</b> No new buys. Stops, exits and the kill switch still act.", '<button class="btn" data-act="resume">Resume trading</button>']);
    $("#notices").innerHTML = out.map(([k, h, b]) => `<div class="notice ${k}" role="${k === "bad" ? "alert" : "status"}"><span class="grow">${h}</span>${b || ""}</div>`).join("");
  }

  // ---------- render: wire ----------
  function slipHtml(ev, c, arrive) {
    const url = safeUrl(ev.id);
    const unread = isUnread(ev);
    const cl = ["slip", unread ? "unread" : "", arrive ? "arrive" : "", focus && focus.ids.has(ev.id) ? "focus" : ""].join(" ");
    const slugAsset = ev.asset === "market" ? "MKT" : ev.asset === "other" ? "OFF" : ev.asset;
    const outlets = c.outlets > 1 ? ` · <b>${c.outlets} outlets</b>` : "";
    let outcome = "";
    if (c.fill) outcome = `<p class="outcome ${c.fill.side}">→ Paper ${c.fill.side} ${fmtQty(c.fill.qty)} ${esc(c.fill.asset)} @ ${fmtPx(c.fill.price)} · ${fmtUsd(c.fill.notional)} · ${esc(c.fill.reason)}</p>`;
    else if (c.kind === "alert") outcome = `<p class="outcome sell">→ Hack alert: ${esc(ev.asset)} is blocked from buying while this is fresh</p>`;
    return `<article class="${cl}" data-id="${esc(ev.id)}">
      <div class="slug">${unread ? '<span class="new">New</span>' : ""}<span>${esc(slugAsset)}-${esc(ev.event)} · ${esc(shortSource(ev.source))} · ${t(ev.published)}${outlets}</span></div>
      <h2>${url ? `<a href="${esc(url)}" target="_blank" rel="noopener noreferrer">${esc(ev.title)}</a>` : esc(ev.title)}</h2>
      <p class="reading"><b>${esc(ev.event)}</b> ${ev.event_conf.toFixed(2)} · sentiment <b>${sign(ev.sentiment)}</b> (conf ${ev.sentiment_conf.toFixed(2)}) · new fact <b>${ev.new_info.toFixed(2)}</b> · asset ${esc(ev.asset)} ${ev.asset_conf.toFixed(2)}</p>
      ${outcome}
      <div class="stamp ${c.kind}">${esc(c.stamp)}<small>${esc(c.small)}</small></div>
    </article>`;
  }

  function renderWire(el) {
    if (!snap.events.length) {
      el.innerHTML = snap.status ? `<div class="empty"><h3>Nothing on the wire yet</h3><p>The bot is running but hasn't analyzed a headline. New copy appears here within one tick of reaching the feeds.</p></div>` : neverRan();
      return;
    }
    const fbe = fillsByEvent();
    const oc = outletCounts();
    const rows = snap.events.map((ev) => ({ ev, c: { ...classify(ev, fbe), outlets: oc.get(ev.cluster_id || ev.id) || 1 } }));
    const counts = { all: rows.length, ran: 0, filed: 0, spiked: 0 };
    rows.forEach((r) => counts[r.c.group]++);
    const assets = snap.assets.map((a) => a.symbol);
    let list = rows.filter((r) => (tab === "all" || r.c.group === tab) && (assetFilter === "all" || r.ev.asset === assetFilter));
    const unreadN = rows.filter((r) => isUnread(r.ev)).length;
    const tabBtn = (k, label) => `<button type="button" data-tab="${k}" aria-pressed="${tab === k}">${label}<span class="n">${counts[k]}</span></button>`;
    const opts = ["all", ...assets, "market", "other"].map((a) => `<option value="${a}" ${assetFilter === a ? "selected" : ""}>${a === "all" ? "All assets" : a === "market" ? "Whole market" : a === "other" ? "Off the desk" : a}</option>`).join("");

    let html = `<div class="tools"><div class="tabs" role="group" aria-label="Filter by verdict">${tabBtn("all", "All")}${tabBtn("ran", "Ran")}${tabBtn("filed", "Filed")}${tabBtn("spiked", "Spiked")}</div>
      <label class="sr"><select id="asset-filter" aria-label="Filter by asset">${opts}</select></label>
      <span class="sp"></span>${unreadN ? `<span><span class="unread-n">${unreadN} unread</span><button type="button" class="linkbtn" data-act="readall">Mark all read</button></span>` : ""}</div>`;
    if (focus) html += `<div class="focusbar"><span class="grow">Showing the copy behind <b>${esc(focus.label)}</b> · ${focus.ids.size} item${focus.ids.size === 1 ? "" : "s"}</span><button type="button" data-act="clearfocus">Show everything</button></div>`;
    if (!list.length) html += `<div class="empty"><h3>No copy matches</h3><p>Nothing ${tab === "all" ? "" : `marked “${tab}” `}${assetFilter === "all" ? "" : `for ${esc(assetFilter)} `}in the last ${snap.events.length} items. <button type="button" class="linkbtn" data-act="clearfilters">Clear filters</button></p></div>`;
    else {
      html += `<div class="slips ${focus ? "focusing" : ""}">` + list.slice(0, shown).map((r) => slipHtml(r.ev, r.c, !firstWire && !known.has(r.ev.id))).join("") + "</div>";
      if (list.length > shown) html += `<button type="button" class="btn more" data-act="more">Show ${Math.min(60, list.length - shown)} more</button>`;
    }
    snap.events.forEach((e) => known.add(e.id));
    firstWire = false;
    el.innerHTML = html;
  }

  function neverRan() {
    return `<div class="empty"><h3>The bot hasn't run yet</h3><p>Start it in another terminal and this page fills in on its first tick:</p><p style="margin-top:10px"><code>TYPESAFE_API_KEY=… cargo run --release -- run</code></p></div>`;
  }

  // ---------- render: run list (view) ----------
  function scoreChart(a) {
    const h = a.history;
    if (h.length < 2) return `<p class="why" style="color:var(--mute)">The chart starts after two ticks.</p>`;
    const W = 600, H = 130, pad = 6;
    const t0 = Date.parse(h[0][0]), t1 = Date.parse(h[h.length - 1][0]) || t0 + 1;
    const x = (ts) => ((Date.parse(ts) - t0) / Math.max(1, t1 - t0)) * W;
    const y = (s) => pad + ((1 - s) / 2) * (H - 2 * pad);
    const maxW = snap.settings.max_asset_weight || 0.2;
    const line = h.map((p) => `${x(p[0]).toFixed(1)},${y(p[2]).toFixed(1)}`).join(" ");
    let area = `M0,${H} ` + h.map((p) => `L${x(p[0]).toFixed(1)},${(H - (p[3] / maxW) * (H * 0.45)).toFixed(1)}`).join(" ") + ` L${W},${H} Z`;
    const yE = y(snap.settings.news_entry), yX = y(-snap.settings.news_exit);
    return `<svg class="score" viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" role="img" aria-label="News score and position size over time for ${esc(a.symbol)}">
      <path d="${area}" fill="rgba(47,74,63,.12)"/>
      <line x1="0" x2="${W}" y1="${y(0)}" y2="${y(0)}" stroke="#D5D6CE"/>
      <line x1="0" x2="${W}" y1="${yE}" y2="${yE}" stroke="#2F6B4F" stroke-dasharray="4 4" vector-effect="non-scaling-stroke"/>
      <line x1="0" x2="${W}" y1="${yX}" y2="${yX}" stroke="#B3304C" stroke-dasharray="4 4" vector-effect="non-scaling-stroke"/>
      <polyline points="${line}" fill="none" stroke="#1C1E22" stroke-width="2" vector-effect="non-scaling-stroke" stroke-linejoin="round"/>
    </svg>
    <div class="legend"><span><i style="background:#1C1E22"></i>News score</span><span><i style="background:#2F6B4F"></i>Entry ${sign(snap.settings.news_entry)}</span><span><i style="background:#B3304C"></i>Exit ${sign(-snap.settings.news_exit)}</span><span><i style="background:rgba(47,74,63,.3);height:8px"></i>Position size</span><span style="margin-left:auto">${t(h[0][0])} – ${t(h[h.length - 1][0])}</span></div>`;
  }

  function assetState(a) {
    const p = snap.account?.positions.find((x) => x.asset === a.symbol);
    const L = a.latest;
    const review = snap.status?.review?.[a.symbol] || L?.review;
    if (p) return { text: `Long ${fmtQty(p.qty)} · ${(p.weight * 100).toFixed(0)}%${review ? " · REVIEW" : ""}`, pos: p, review };
    if (review) return { text: "REVIEW", review };
    if (a.cooldown_until) return { text: `Cooldown to ${t(a.cooldown_until)}` };
    if (L?.news.hack_alert) return { text: "Hack hold" };
    if (snap.control.paused) return { text: "Flat · paused" };
    return { text: "Flat" };
  }

  function driversFor(sym) {
    const a = snap.assets.find((x) => x.symbol === sym);
    const ids = new Set(a?.latest?.drivers || []);
    snap.trades.filter((f) => f.asset === sym).slice(0, 3).forEach((f) => (f.events || []).forEach((id) => ids.add(id)));
    return ids;
  }

  const BINDING = { signal: "signal", asset_cap: "per-asset cap", gross_cap: "gross cap", heat_cap: "heat cap", regime_ceiling: "regime ceiling", rebalance_band: "rebalance band", min_trade: "min trade", cash: "cash" };

  // Decision chain (0.6): every gate the last decision passed through.
  function gateTrace(L) {
    if (!L?.gates?.length) return "";
    return `<div class="trace"><h3>Decision trace</h3><ol>${L.gates.map((g) => `<li class="${g.passed ? "pass" : "fail"}" title="${esc(g.detail || "")}"><span>${esc(g.gate)}</span>${g.detail ? `<small>${esc(g.detail)}</small>` : ""}</li>`).join("")}<li class="cap"><span>caps</span><small>${esc(BINDING[L.binding] || "—")} → ${(L.final_weight * 100).toFixed(1)}%</small></li></ol></div>`;
  }

  function renderRun(el) {
    if (!snap.status) { el.innerHTML = neverRan(); return; }
    const evById = new Map(snap.events.map((e) => [e.id, e]));
    el.innerHTML = '<div style="height:18px"></div>' + snap.assets.map((a) => {
      const L = a.latest, st = assetState(a);
      const drivers = [...(L?.drivers || [])].map((id) => evById.get(id)).filter(Boolean);
      return `<section class="asset" aria-labelledby="as-${esc(a.symbol)}">
        <div class="head"><h2 id="as-${esc(a.symbol)}">${esc(a.symbol)}</h2><p class="px">${esc(a.name)} · ${L ? fmtPx(L.price) : "no price yet"}</p>
          <dl><dt>Position</dt><dd>${esc(st.text)}</dd>
          <dt>P&amp;L</dt><dd class="${cls(st.pos?.pnl_pct)}">${st.pos ? pct(st.pos.pnl_pct) : "—"}</dd>
          <dt>News score</dt><dd class="${cls(L?.news.score)}">${L ? sign(L.news.score) : "—"}</dd>
          <dt>Stance</dt><dd>${L && L.news.n_clusters != null ? `<span class="pos">+${L.news.pos}</span> / <span class="neg">−${L.news.neg}</span> / ~${L.news.neutral}` : "—"}</dd>
          <dt>Momentum</dt><dd class="${cls(L?.momentum)}">${L ? (L.momentum == null ? "REVIEW" : pct(L.momentum)) : "—"}</dd>
          <dt>Target</dt><dd>${L ? (L.final_weight * 100).toFixed(0) + "%" : "—"}</dd>
          <dt>Last decision</dt><dd>${esc(L?.reason || "—")}</dd>
          <dt>Limited by</dt><dd>${L?.binding ? esc(BINDING[L.binding] || L.binding) : "—"}</dd></dl>
          ${st.review ? `<p class="review-tag">REVIEW · ${esc(st.review)}</p>` : ""}</div>
        <div>${scoreChart(a)}</div>
        ${gateTrace(L)}
        <div class="drivers"><h3>Copy driving the score</h3>${drivers.length ? `<ul>${drivers.map((e) => `<li><span class="t">${t(e.published)}</span><span class="h" title="${esc(e.title)}">${esc(e.title)}</span><span class="${cls(e.sentiment)}">${sign(e.sentiment)}</span></li>`).join("")}</ul><button type="button" class="linkbtn" data-focus="${esc(a.symbol)}" style="margin-top:6px">Show on the wire</button>` : `<p style="color:var(--mute)">No recent copy moves ${esc(a.symbol)}.</p>`}</div>
      </section>`;
    }).join("");
  }

  // ---------- render: pipeline ----------
  function renderPipeline(el) {
    const st = snap.status, l = liveness();
    const stateWord = { alive: "Running", stale: "Silent", stopped: "Stopped", never: "Not started" }[l.s];
    const stateCls = { alive: "ok", stale: "error", stopped: "error", never: "skipped" }[l.s];
    const next = st?.next_tick_at ? (Date.parse(st.next_tick_at) - now()) / 1000 : null;
    let html = `<div style="height:18px"></div><div class="sheet"><h2>Processes</h2><p class="sub">The trading loop and this dashboard run separately; either can restart without the other.</p>
      <table><thead><tr><th>Process</th><th>State</th><th class="num">PID</th><th>Started</th><th>Last tick</th><th>Next tick</th><th class="num">Tick</th></tr></thead><tbody>
      <tr><td><b>Trading loop</b><br><span style="color:var(--mute)">jev-trader run</span></td><td><span class="state-tag ${stateCls}">${stateWord}</span>${st?.paused ? ' · <span class="state-tag degraded">paused</span>' : ""}</td>
        <td class="num">${st ? st.pid : "—"}</td><td>${st ? ago(st.started_at) : "—"}</td><td>${st?.last_tick_at ? `${tsec(st.last_tick_at)} · ${ago(st.last_tick_at)}` : "—"}</td>
        <td>${next == null ? "—" : next > 0 ? `in ${dur(next)}` : l.s === "alive" ? "running now" : `overdue ${dur(-next)}`}</td><td class="num">${st ? st.tick.toLocaleString() : "—"}</td></tr>
      <tr><td><b>Dashboard</b><br><span style="color:var(--mute)">jev-trader dashboard</span></td><td><span class="state-tag ${conn === "live" ? "ok" : "error"}">${conn === "live" ? "Connected" : "Disconnected"}</span></td>
        <td class="num">${snap.dashboard.pid}</td><td>${ago(snap.dashboard.started_at)}</td><td colspan="3" style="color:var(--mute)">Snapshot ${tsec(snap.now)}</td></tr>
      </tbody></table></div>`;

    const recs = snap.pipeline || [];
    const names = st?.stages?.length ? st.stages.map((s) => s.name) : ["Feeds", "Jev", "Prices", "Signals", "Risk", "Execution"];
    const N = 60;
    html += `<div class="sheet"><h2>Stages</h2><p class="sub">Latest tick${st?.last_tick_at ? " at " + tsec(st.last_tick_at) : ""}, with the last ${N} ticks oldest to newest.</p>
      <table class="stages"><thead><tr><th>Stage</th><th>State</th><th>Detail</th><th class="num">Time</th><th>Last ${N} ticks</th></tr></thead><tbody>` +
      names.map((n) => {
        const s = st?.stages?.find((x) => x.name === n);
        const cells = recs.slice(-N).map((r) => { const x = r.stages.find((y) => y.name === n); return `<i class="${x ? x.state : "none"}" title="Tick ${r.tick} · ${x ? esc(x.state + " · " + x.detail) : "not run"}"></i>`; });
        while (cells.length < N) cells.unshift('<i class="none"></i>');
        return `<tr><td><b>${esc(n)}</b></td><td>${s ? `<span class="state-tag ${s.state}">${esc(s.state)}</span>` : "—"}</td><td>${s ? esc(s.detail) : "—"}</td><td class="num">${s ? s.ms.toLocaleString() + " ms" : "—"}</td><td><span class="strip">${cells.join("")}</span></td></tr>`;
      }).join("") + `</tbody></table></div>`;

    const S = snap.settings;
    const kv = [["Tick interval", `${S.interval_secs}s`], ["Jev model", S.jev_model], ["Entry score", sign(S.news_entry)], ["Exit score", sign(-S.news_exit)], ["News half-life", `${S.half_life_hours}h`], ["Max per asset", `${S.max_asset_weight * 100}%`], ["Max invested", `${S.max_gross * 100}%`], ["Stop loss", `${S.stop_loss_pct * 100}%`], ["Daily loss limit", `${S.daily_loss_limit_pct * 100}%`], ["Cooldown after stop", `${S.cooldown_hours}h`], ["Sizing", S.sizing], ["Stop", S.stop === "atr" ? "ATR" : "fixed %"], ["Heat cap", `${(S.max_heat * 100).toFixed(1)}%`], ["Weekly halt", S.weekly_loss_limit_pct ? `${S.weekly_loss_limit_pct * 100}%` : "off"], ["Monthly halt", S.monthly_loss_limit_pct ? `${S.monthly_loss_limit_pct * 100}%` : "off"], ["Losing streak", S.losing_streak_n ? `${S.losing_streak_n} → ${S.streak_cooldown_hours}h` : "off"], ["Regime gate", S.regime_mode], ["Challenge", S.challenge ? "on" : "off"], ["Story dedupe", S.dedupe_clusters ? "on" : "off"], ["Min clusters", String(S.min_clusters)], ["Reaction gate", S.reaction_gate ? "on" : "off"], ["Jev attempts", String(S.jev_max_attempts)]];
    html += `<div class="sheet"><h2>Settings</h2><p class="sub">Read-only here. Change <code>config.toml</code> and restart the trading loop.</p><div class="kv">${kv.map(([k, v]) => `<div><span>${k}</span><b>${esc(v)}</b></div>`).join("")}</div>
      <p class="sub">Feeds: ${S.feeds.map((f) => esc(host(f))).join(" · ")}</p></div>`;
    el.innerHTML = html;
  }

  // ---------- render: trades ----------
  function renderTrades(el) {
    if (!snap.trades.length) {
      el.innerHTML = `<div style="height:18px"></div><div class="empty"><h3>No paper fills yet</h3><p>Trades appear here when news and momentum agree, or when a stop, hack alert or flatten sells a position.</p></div>`;
      return;
    }
    el.innerHTML = `<div style="height:18px"></div><div class="sheet"><table><thead><tr><th>Time</th><th>Side</th><th>Asset</th><th class="num">Qty</th><th class="num">Price</th><th class="num">Notional</th><th class="num">Fee</th><th>Reason</th><th>Source copy</th></tr></thead><tbody>` +
      snap.trades.map((f, i) => `<tr><td class="t">${t(f.ts)}<br><span style="font-size:12px">${new Date(f.ts).toLocaleDateString()}</span></td><td><span class="side ${f.side}">${esc(f.side)}</span></td><td><b>${esc(f.asset)}</b></td>
        <td class="num">${fmtQty(f.qty)}</td><td class="num">${fmtPx(f.price)}</td><td class="num">${fmtUsd(f.notional)}</td><td class="num">${fmtUsd(f.fee)}</td><td title="${esc((f.gates || []).filter((g) => !g.passed).map((g) => g.gate).join(", "))}">${esc(f.reason)}${f.realized_pnl != null ? `<br><span class="${cls(f.realized_pnl)}" style="font-size:12px">${f.realized_pnl >= 0 ? "+" : "−"}${fmtUsd(Math.abs(f.realized_pnl))} realized</span>` : ""}</td>
        <td>${(f.events || []).length ? `<button type="button" class="linkbtn" data-trade="${i}">${f.events.length} item${f.events.length === 1 ? "" : "s"}</button>` : '<span style="color:var(--mute)">none</span>'}</td></tr>`).join("") +
      `</tbody></table></div>`;
  }

  // ---------- render: scorecard ----------
  const num = (x, d = 3) => (x == null || !isFinite(x) ? "—" : sign(x, d));
  const pc0 = (x) => (x == null ? "—" : `${(x * 100).toFixed(0)}%`);
  const low = (b) => (b ? ' class="low"' : "");
  const noise = (p) => (p == null ? "no null test" : p > 0.1 ? "indistinguishable from noise" : `p ${p.toFixed(2)}`);

  function renderRegime() {
    const r = snap.regime;
    if (!r) return `<div class="sheet"><h2>Market regime</h2><p class="sub">Not computed yet. It refreshes hourly from keyless sources.</p></div>`;
    return `<div class="sheet"><h2>Market regime · ${esc(r.zone)} ${r.score == null ? "" : r.score.toFixed(0)}</h2><p class="sub">${snap.settings.regime_mode === "off" ? "Recorded and shown only: it has no effect on trading." : `Gate mode: ${esc(snap.settings.regime_mode)}.`} Updated ${ago(r.ts)}. Fewer than 4 of 6 components → UNKNOWN.</p>
      <table><thead><tr><th>Component</th><th class="num">Weight</th><th class="num">Score</th><th>Detail</th></tr></thead><tbody>${r.components.map((c) => `<tr${c.score == null ? ' class="low"' : ""}><td>${esc(c.name)}</td><td class="num">${c.weight}</td><td class="num">${c.score == null ? "—" : c.score.toFixed(0)}</td><td>${esc(c.detail)}</td></tr>`).join("")}</tbody></table></div>`;
  }

  function renderScore(el) {
    let html = '<div style="height:18px"></div>' + renderRegime();
    const sc = snap.scorecard;
    if (!sc) {
      el.innerHTML = html + `<div class="empty"><h3>Nothing settled yet</h3><p>Headlines and decisions are scored 24 hours after they happen, on closed 1h candles. Run <code>jev-trader report</code> for the full write-up.</p></div>`;
      return;
    }
    html += `<p class="label-line">${esc(sc.label)} · n=${sc.n_settled}${sc.n_failed ? ` · ${sc.n_failed} failed` : ""} · greyed: n &lt; 30</p>`;
    html += `<div class="sheet"><h2>Headlines → forward move</h2><p class="sub">IC: Spearman of sentiment × new-fact against the vol-normalised return, overlapping windows merged. Percentile within a block-permutation null.</p>
      <table><thead><tr><th>Horizon</th><th class="num">Clusters</th><th class="num">IC</th><th class="num">Null pct</th><th>Verdict</th><th class="num">Hit rate</th></tr></thead><tbody>${sc.headlines.map((r) => `<tr${low(r.low_n)}><td>${r.h}h</td><td class="num">${r.n_clusters} <small>/ ${r.n_raw}</small></td><td class="num">${num(r.ic)}</td><td class="num">${r.null_pct == null ? "—" : r.null_pct.toFixed(0)}</td><td>${noise(r.p)}</td><td class="num">${pc0(r.hit_rate)}</td></tr>`).join("")}</tbody></table></div>`;
    const d = sc.decisions;
    html += `<div class="sheet"><h2>Decisions</h2><p class="sub${d.low_n ? " low" : ""}">${d.n} decisions in ${d.n_clusters} clusters · hit rate ${pc0(d.hit_rate)} · expectancy ${d.expectancy == null ? "—" : pct(d.expectancy)} per decision (24h) · ${d.regime_mismatch} regime mismatches</p>
      <div class="kv">${Object.entries(d.tags).map(([k, v]) => `<div><span>${esc(k)}</span><b>${v}</b></div>`).join("") || '<div><span>No outcomes yet</span><b>—</b></div>'}</div></div>`;
    html += `<div class="sheet"><h2>Gate values</h2><p class="sub">Blocked entries settled as if taken (24h). Missed = would have worked.</p>${sc.gates.length ? `<table><thead><tr><th>Gate</th><th class="num">n</th><th class="num">Missed</th><th class="num">Correctly blocked</th><th class="num">Mean blocked return</th></tr></thead><tbody>${sc.gates.map((g) => `<tr${low(g.low_n)}><td>${esc(g.gate)}</td><td class="num">${g.n}</td><td class="num">${g.missed}</td><td class="num">${g.correctly_blocked}</td><td class="num ${cls(g.mean_ret)}">${pct(g.mean_ret)}</td></tr>`).join("")}</tbody></table>` : '<p class="sub">No blocked entries settled yet.</p>'}</div>`;
    const s = sc.stop;
    html += `<div class="sheet"><h2>Stop calibration</h2><p class="sub${s.low_n ? " low" : ""}">${s.n_winners} winning long entries · ${pc0(s.share_below_stop)} first went below −${(s.stop_pct * 100).toFixed(1)}% · MAE median ${s.mae_q50 == null ? "—" : pct(s.mae_q50)}, 90th pct ${s.mae_q90 == null ? "—" : pct(s.mae_q90)}</p></div>`;
    if (sc.cohorts.length) html += `<div class="sheet"><h2>Breakdown (4h)</h2><table><thead><tr><th>Cohort</th><th class="num">Clusters</th><th class="num">IC</th><th>Verdict</th><th class="num">Hit rate</th></tr></thead><tbody>${sc.cohorts.map((c) => `<tr${low(c.low_n)}><td>${esc(c.dim)} = ${esc(c.dim === "source" ? shortSource(c.key) : c.key)}</td><td class="num">${c.n_clusters}</td><td class="num">${num(c.ic)}</td><td>${noise(c.p)}</td><td class="num">${pc0(c.hit_rate)}</td></tr>`).join("")}</tbody></table></div>`;
    el.innerHTML = html;
  }

  // ---------- render: scalper ----------
  const bps = (x, d = 1) => (x == null || !isFinite(x) ? "—" : sign(x, d));
  const GROUPS = { all: "All candidates", long: "Long candidates", short: "Short candidates", taken: "Taken", jev_veto: "Blocked by Jev review", cost: "Blocked by cost gate", other: "Blocked by other gates" };
  const sideTag = (side) => `<span class="side ${side === "short" ? "sell" : "buy"}">${side === "short" ? "short" : "long"}</span>`;

  function renderScalp(el) {
    const pad = '<div style="height:18px"></div>';
    if (!scalp || scalp.lite) { el.innerHTML = pad + `<div class="empty"><h3>Loading the scalp desk…</h3><p>Reading its status file.</p></div>`; return; }
    if (!scalp.available) {
      el.innerHTML = pad + `<div class="empty"><h3>No scalper data</h3><p>Nothing in <code>${esc(scalp.dir)}</code> yet. Start it with <code>jev-trader --config config.scalp.toml --env-file .env.test scalp --live</code>, or point the dashboard at another folder with <code>--scalp-dir</code>.</p></div>`;
      return;
    }
    const S = scalp.status, C = S.settings || {}, sm = scalp.summary;
    const age = (now() - Date.parse(S.ts)) / 1000;
    const state = scalp.running === false ? "stopped" : age > 15 ? "silent" : "running";
    const notes = [];
    if (scalpLost) notes.push(["bad", "<b>Can't reach the dashboard server.</b> Showing the last scalper snapshot."]);
    if (state !== "running") notes.push(["bad", `<b>The scalper ${state === "stopped" ? "process has stopped" : "has gone quiet"}.</b> Last update ${ago(S.ts)}. Check <code>${esc(scalp.status.venue === "paper" ? "" : "data-scalp/")}scalp.log</code>.`]);
    if (S.live_hold) notes.push(["bad", `<b>Exchange hold.</b> ${esc(S.live_hold)}`]);
    if (S.halted_today) notes.push(["bad", `<b>Daily loss limit hit (−${((C.daily_loss_limit_pct || 0) * 100).toFixed(0)}%).</b> No entries until the next UTC day; exits still run.`]);
    if (!S.feed.connected) notes.push(["bad", `<b>Market stream disconnected.</b> ${esc(S.feed.last_error || "")} · ${S.feed.reconnects} reconnects. No entries without a fresh book.`]);
    const warm = (S.assets || []).find((a) => a.features && a.features.history_secs < C.warmup_secs);
    if (warm && state === "running") notes.push(["warn", `<b>Warming up.</b> Building a ${Math.round(C.warmup_secs / 60)}-minute activity baseline before trading (${Math.round(warm.features.history_secs / 60)} min so far).`]);
    let html = pad + notes.map(([k, h]) => `<div class="notice ${k}" role="${k === "bad" ? "alert" : "status"}"><span class="grow">${h}</span></div>`).join("");

    const venue = { paper: "Paper (simulated fills)", "binance-testnet": "Binance testnet · fake money", binance: "LIVE · Binance · real orders" }[S.venue] || S.venue;
    const stateCls = { running: "ok", silent: "error", stopped: "error" }[state];
    const kv = [
      ["State", `<span class="state-tag ${stateCls}">${state === "running" ? "Running" : state === "silent" ? "Silent" : "Stopped"}</span> · PID ${S.pid}`],
      ["Venue", esc(venue)],
      ["Market stream", S.feed.connected ? `<span class="state-tag ok">Connected</span> · ${S.feed.last_msg_ms_ago} ms` : '<span class="state-tag error">Down</span>'],
      ["Equity", `${fmtUsd(S.equity)} <small>of ${fmtUsd(S.capital ?? S.starting_cash)} allotted</small>`],
      ["Trade P&L today", `<span class="${cls(S.day_trade_pnl)}">${fmtUsd(S.day_trade_pnl ?? 0)}</span>`],
      ["Trade P&L total", `<span class="${cls(S.realized)}">${fmtUsd(S.realized)}</span> · ${S.wins}W / ${S.losses}L`],
      ["Inventory drift", `${term(`<span class="${cls(S.inventory_drift)}">${fmtUsd(S.inventory_drift ?? 0)}</span>`, "Price change of the neutral coin inventory since it was allotted. Not trading results: it moves with the market whether or not the desk trades.")}`],
      ["Candidates", `${S.candidates_session} this session · ${sm.candidates} logged`],
      ["Jev reviews", `${S.jev.reviews} · ${S.jev.vetoes} vetoed${S.jev.failures ? ` · <span class="neg">${S.jev.failures} failed</span>` : ""}`],
      ["News context", `${S.jev.events} headlines held · ${S.jev.news_ok} new this session${S.jev.news_failed ? ` · ${S.jev.news_failed} failed` : ""}`],
    ];
    html += `<div class="sheet"><h2>Process</h2><p class="sub">Runs on its own: <code>jev-trader scalp</code>. Stop it with Ctrl+C in its terminal; open positions stay in its ledger and are managed on restart.</p><div class="kv">${kv.map(([k, v]) => `<div><span>${k}</span><b>${v}</b></div>`).join("")}</div></div>`;

    // Gates, right now, against their thresholds.
    const cell = (pass, txt, title) => `<td class="num ${pass ? "pos" : ""}"${pass ? "" : ' style="color:var(--mute)"'} title="${esc(title || "")}">${txt}</td>`;
    const rows = (S.assets || []).map((a) => {
      const f = a.features, n = a.news || {}, p = a.position;
      if (!f) return `<tr><td><b>${esc(a.symbol)}</b></td><td colspan="11" style="color:var(--mute)">No market data yet</td></tr>`;
      // Every directional gate is judged for the side the flow leans to.
      const side = a.leaning || "long", s = side === "short" ? -1 : 1;
      const cost = 2 * C.fee_bps + 2 * C.slippage_bps + f.spread_bps, hurdle = C.cost_mult * cost;
      const flowT = Math.max(0.05, C.min_flow_imbalance - C.news_tilt * s * Math.max(-1, Math.min(1, n.score || 0)));
      const other = side === "short" ? a.long_block : a.short_block;
      const blocked = p ? `in a ${p.side} trade` : a.blocked_by ? `<b>${esc(a.blocked_by)}</b> · ${esc(why(a.blocked_by))}` : "all gates open";
      return `<tr><td><b>${esc(a.symbol)}</b><br><span style="color:var(--mute);font-size:12px">${fmtPx(f.mid)}</span></td>
        <td>${sideTag(side)}</td>
        ${cell(f.activity >= C.min_activity_ratio, f.activity.toFixed(2) + "×", `needs ≥ ${C.min_activity_ratio}×`)}
        ${cell(s * f.flow >= flowT, sign(f.flow), `${side}: needs ${s > 0 ? "≥ " + sign(flowT) : "≤ " + sign(-flowT)}`)}
        ${cell(s * f.obi >= C.min_book_imbalance, sign(f.obi), `${side}: needs ${s > 0 ? "≥ +" : "≤ −"}${C.min_book_imbalance}`)}
        ${cell(s * f.ret_60s_bps >= C.min_push_bps && s * f.micro_bps >= 0, bps(f.ret_60s_bps), `${side}: needs ${s > 0 ? "≥ +" : "≤ −"}${C.min_push_bps} bps`)}
        ${cell(s * f.ret_300s_bps <= C.max_chase_bps, bps(f.ret_300s_bps), `skip beyond ${s > 0 ? "+" : "−"}${C.max_chase_bps} bps`)}
        ${cell(f.spread_bps <= C.max_spread_bps, f.spread_bps.toFixed(2), `≤ ${C.max_spread_bps}`)}
        ${cell(f.exp_move_bps >= hurdle, `${f.exp_move_bps.toFixed(1)} <small>/ ${hurdle.toFixed(0)}</small>`, `typical move over ${C.hold_secs / 60} min vs ${C.cost_mult}× round-trip cost (${cost.toFixed(1)} bps)`)}
        ${cell(side === "short" || !n.hack_alert, n.hack_alert ? "HACK" : sign(n.score || 0), `${n.n_events || 0} headlines in context`)}
        <td title="${esc(C.shorts ? `${side === "short" ? "long" : "short"} side: ${other ? why(other) : "all gates open"}` : "")}">${blocked}</td></tr>`;
    }).join("");
    html += `<div class="sheet"><h2>Gates now</h2><p class="sub">Each row is judged for the side the flow leans to; green passes for that side. Values in bps unless marked. An entry needs every gate, then the risk limits and Jev's review. Hover a value for its threshold, or the last column for the other side.</p>
      <table><thead><tr><th>Asset</th><th>${term("Leaning", "The side the order flow points to right now; its gates are the ones judged in this row")}</th><th class="num">${term("Activity", "Trades/sec now vs normal")}</th><th class="num">${term("Flow", "Aggressive buyers vs sellers, −1..+1")}</th><th class="num">${term("Book", "Bid vs ask size at the top of the book, −1..+1")}</th><th class="num">${term("Push 60s", "Price change over the last minute, bps")}</th><th class="num">${term("5m move", "Price change over 5 minutes; above the limit means it already ran")}</th><th class="num">${term("Spread", "Ask minus bid, bps")}</th><th class="num">${term("Exp. move / hurdle", "Typical move over the hold vs what is needed to beat fees, bps")}</th><th class="num">${term("News", "Background news score; a hack alert blocks")}</th><th>Waiting because</th></tr></thead><tbody>${rows}</tbody></table></div>`;

    const open = (S.assets || []).filter((a) => a.position);
    if (open.length) {
      html += `<div class="sheet"><h2>Open positions</h2><table><thead><tr><th>Asset</th><th>Side</th><th class="num">Qty</th><th class="num">Entry</th><th class="num">Exit price now</th><th class="num">P&amp;L</th><th class="num">Take profit</th><th class="num">${term("Stop", "Moves to break-even after fees, then trails the best price, once the trade is 1.5× its cost in profit")}</th><th class="num">Time left</th><th>Jev</th></tr></thead><tbody>` +
        open.map((a) => {
          const p = a.position, s = p.side === "short" ? -1 : 1;
          const px = s > 0 ? a.features?.bid : a.features?.ask;
          const pnl = px ? s * (px / p.entry_px - 1) * 1e4 : null;
          const r = p.review;
          const trailed = s * (p.sl_px - p.init_sl_px) > 1e-9;
          return `<tr><td><b>${esc(a.symbol)}</b></td><td>${sideTag(p.side)}</td><td class="num">${fmtQty(p.qty)}</td><td class="num">${fmtPx(p.entry_px)}</td><td class="num">${fmtPx(px)}</td><td class="num ${cls(pnl)}">${bps(pnl)} bps</td><td class="num">${fmtPx(p.tp_px)}</td><td class="num">${fmtPx(p.sl_px)}${trailed ? '<br><small class="pos">trailing</small>' : ""}</td><td class="num">${dur((p.deadline_ms - now()) / 1000)}</td><td>${r ? `adverse ${r.adverse.toFixed(2)} · support ${r.support.toFixed(2)}` : "no relevant news"}${p.size_mult < 1 ? " · half size" : ""}</td></tr>`;
        }).join("") + `</tbody></table></div>`;
    }

    // Evidence: markouts net of cost, by what happened to the candidate.
    const hs = C.markouts || [60, 300, 600];
    const by = new Map(sm.markouts.map((m) => [`${m.group}:${m.h}`, m]));
    const groups = Object.keys(GROUPS).filter((g) => hs.some((h) => by.has(`${g}:${h}`)));
    html += `<p class="label-line">${esc(S.venue)} · markouts net of round-trip cost · greyed: n &lt; 30</p>`;
    html += `<div class="sheet"><h2>Evidence: what candidates were worth</h2><p class="sub">Every candidate is marked out at fixed horizons, taken or not. Net = mid-price move in the trade's direction (a fall counts as a gain for a short) minus the round-trip cost at entry. H-S01 needs positive net at 10 min with n ≥ 100; H-S02 needs Jev's vetoes to do worse than what it passed.</p>` +
      (groups.length
        ? `<table><thead><tr><th>Group</th>${hs.map((h) => `<th class="num">${h >= 60 ? h / 60 + " min" : h + "s"} net</th><th class="num">positive</th>`).join("")}<th class="num">n</th></tr></thead><tbody>` +
          groups.map((g) => {
            const n = Math.max(...hs.map((h) => by.get(`${g}:${h}`)?.n || 0));
            return `<tr${low(n < 30)}><td>${GROUPS[g]}</td>${hs.map((h) => { const m = by.get(`${g}:${h}`); return m ? `<td class="num ${cls(m.mean_net_bps)}">${bps(m.mean_net_bps)}</td><td class="num">${pc0(m.hit)}</td>` : '<td class="num">—</td><td class="num">—</td>'; }).join("")}<td class="num">${n}</td></tr>`;
          }).join("") + `</tbody></table>`
        : `<p class="sub">No markouts yet. The first ones arrive ${hs[0]}s after the first candidate.</p>`) + `</div>`;

    const bl = Object.entries(sm.blocked_by).sort((a, b) => b[1] - a[1]);
    if (bl.length) html += `<div class="sheet"><h2>What stopped candidates</h2><p class="sub">First failing gate for every logged candidate. "taken" means it traded.</p><div class="kv">${bl.map(([k, v]) => `<div><span>${esc(k === "taken" ? "taken" : why(k))}</span><b>${v}</b></div>`).join("")}</div></div>`;

    html += `<div class="sheet"><h2>Recent candidates</h2><p class="sub">Moments when activity, flow, book and push all agreed (at most one per asset per ${C.flow_window_secs}s).</p>` +
      (scalp.candidates.length
        ? `<table><thead><tr><th>Time</th><th>Asset</th><th>Side</th><th class="num">Activity</th><th class="num">Flow</th><th class="num">Exp. move / cost</th><th>Jev review</th><th>Outcome</th></tr></thead><tbody>` +
          scalp.candidates.map((c) => {
            const f = c.features, r = c.review;
            const jev = c.review_error ? `<span class="neg">failed</span>` : r ? `adverse ${r.adverse.toFixed(2)} · risk ${r.event_risk.toFixed(2)} · support ${r.support.toFixed(2)}` : `<span style="color:var(--mute)">not asked</span>`;
            return `<tr><td class="t">${tsec(c.ts)}</td><td><b>${esc(c.asset)}</b></td><td>${sideTag(c.side)}</td><td class="num">${f.activity.toFixed(2)}×</td><td class="num">${sign(f.flow)}</td><td class="num">${f.exp_move_bps.toFixed(1)} / ${c.eval.cost_bps.toFixed(1)}</td><td>${jev}</td><td>${c.taken ? '<b>taken</b>' : `blocked · ${esc(why(c.blocked_by))}`}</td></tr>`;
          }).join("") + `</tbody></table>`
        : `<p class="sub">None yet. In quiet markets order flow rarely lines up; that is expected.</p>`) + `</div>`;

    const T = sm.trades;
    html += `<div class="sheet"><h2>Round trips</h2><p class="sub">${T.n ? `${T.n} trades · ${T.wins} winners · ${fmtUsd(T.pnl_usd)} · mean ${bps(T.mean_bps)} bps after fees` : "No trades yet."}</p>` +
      (scalp.trades.length
        ? `<table><thead><tr><th>Opened</th><th>Asset</th><th>Side</th><th class="num">Hold</th><th class="num">Entry</th><th class="num">Exit</th><th class="num">P&amp;L</th><th class="num">bps</th><th class="num">Fees</th><th>Exit reason</th></tr></thead><tbody>` +
          scalp.trades.map((x) => `<tr><td class="t">${tsec(x.opened)}</td><td><b>${esc(x.asset)}</b></td><td>${sideTag(x.side)}</td><td class="num">${dur(x.hold_secs)}</td><td class="num">${fmtPx(x.entry_px)}</td><td class="num">${fmtPx(x.exit_px)}</td><td class="num ${cls(x.pnl_usd)}">${fmtUsd(x.pnl_usd)}</td><td class="num ${cls(x.pnl_bps)}">${bps(x.pnl_bps)}</td><td class="num">${fmtUsd(x.fees_usd)}</td><td>${esc(x.reason)}</td></tr>`).join("") + `</tbody></table>`
        : "") + `</div>`;
    el.innerHTML = html;
  }

  async function pollScalp(lite) {
    try {
      const r = await fetch(lite ? "/api/scalp?lite=1" : "/api/scalp", { cache: "no-store" });
      if (!r.ok) throw new Error(String(r.status));
      const d = await r.json();
      // A lite answer refreshes the status but keeps the last full lists.
      scalp = lite && scalp && !scalp.lite ? { ...scalp, status: d.status, running: d.running, available: d.available } : d;
      scalpLost = false;
    } catch { scalpLost = true; }
    if (view === "scalp") render(); else renderBeat();
  }

  // ---------- render: glance column ----------
  function renderEq() {
    const el = $("#eq");
    if (view === "scalp") { renderScalpEq(el); return; }
    const A = snap?.account;
    const head = (cash) => `<h3>News desk · equity <span>${cash}</span></h3>`;
    const where = `<p class="where">${snap?.status?.testnet ? "Binance testnet · fake money" : snap?.status?.live ? "Real money on Binance" : "Simulated money · never touches an exchange"}</p>`;
    if (!A) { el.innerHTML = head("") + `<p class="fig">—</p>${where}<p class="chg">Waiting for the first tick</p>`; return; }
    const total = A.equity / A.starting_cash - 1, day = A.equity / A.day_start_equity - 1;
    const pts = snap.equity;
    let chart = `<p class="chg" style="margin-top:12px">The chart starts after two ticks.</p>`;
    if (pts.length >= 2) {
      const W = 356, H = 54, p = 4;
      const t0 = Date.parse(pts[0].ts), t1 = Date.parse(pts[pts.length - 1].ts);
      const vals = pts.map((e) => e.equity).concat([A.starting_cash]);
      let lo = Math.min(...vals), hi = Math.max(...vals); if (hi - lo < 1) { hi += 1; lo -= 1; }
      const x = (ts) => ((Date.parse(ts) - t0) / Math.max(1, t1 - t0)) * W;
      const y = (v) => p + (1 - (v - lo) / (hi - lo)) * (H - 2 * p - 8);
      const line = pts.map((e) => `${x(e.ts).toFixed(1)},${y(e.equity).toFixed(1)}`).join(" ");
      const ticks = snap.trades.filter((f) => Date.parse(f.ts) >= t0).map((f) => `<line x1="${x(f.ts)}" x2="${x(f.ts)}" y1="${H - 7}" y2="${H}" stroke="${f.side === "buy" ? "#2F6B4F" : "#B3304C"}" stroke-width="2"/>`).join("");
      chart = `<svg class="chart" viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" role="img" aria-label="Paper equity over the last 24 hours">
        <line x1="0" x2="${W}" y1="${y(A.starting_cash)}" y2="${y(A.starting_cash)}" stroke="#C5C7BD" stroke-dasharray="3 4" vector-effect="non-scaling-stroke"/>
        <polyline points="0,${H - 8} ${line} ${W},${H - 8}" fill="rgba(47,74,63,.08)" stroke="none"/>
        <polyline points="${line}" fill="none" stroke="#1C1E22" stroke-width="2" vector-effect="non-scaling-stroke" stroke-linejoin="round"/>${ticks}</svg>
        <div class="axis"><span>${t(pts[0].ts)}</span><span>dashed: start ${fmtUsd(A.starting_cash)} · ticks: fills</span><span>${t(pts[pts.length - 1].ts)}</span></div>`;
    }
    el.innerHTML = `${head("cash " + fmtUsd(A.cash))}<p class="fig">${fmtUsd(A.equity)}</p>${where}
      <p class="chg"><span><b class="${cls(day)}">${pct(day)}</b> today</span><span><b class="${cls(total)}">${pct(total)}</b> since start</span></p>${chart}`;
  }

  function renderScalpEq(el) {
    const S = scalp?.status;
    if (!S) { el.innerHTML = `<h3>Scalp desk · equity</h3><p class="fig">—</p><p class="where">${scalp && !scalp.available ? "Not started" : "Loading…"}</p>`; return; }
    const cap = S.capital ?? S.starting_cash;
    el.innerHTML = `<h3>Scalp desk · equity <span>cash ${fmtUsd(S.cash)}</span></h3><p class="fig">${fmtUsd(S.equity)}</p>
      <p class="where">${S.venue === "binance-testnet" ? `${fmtUsd(S.starting_cash)} cash + coin inventory allotted from the Binance testnet account · fake money` : S.venue === "paper" ? "Simulated cash and coin inventory" : `${fmtUsd(cap)} allotted · real money`}</p>
      <p class="chg"><span>trades <b class="${cls(S.realized)}">${fmtUsd(S.realized)}</b></span><span>inventory drift <b class="${cls(S.inventory_drift)}">${fmtUsd(S.inventory_drift ?? 0)}</b></span><span>${S.wins}W / ${S.losses}L</span></p>`;
  }

  function renderScalpRunlist(el) {
    const S = scalp?.status;
    if (!S) { el.innerHTML = ""; return; }
    el.innerHTML = `<h3>Watching <span>why each coin waits</span></h3><div class="rl">` + (S.assets || []).map((a) => {
      const p = a.position, f = a.features;
      const s = p?.side === "short" ? -1 : 1;
      const pnl = p && f ? s * ((s > 0 ? f.bid : f.ask) / p.entry_px - 1) * 1e4 : null;
      return `<div class="rlrow"><span class="sym">${esc(a.symbol)}</span><span class="state">${p ? `In a ${p.side}` : "Flat"}${a.inventory?.qty ? ` <small style="color:var(--mute);font-weight:500">· ${fmtQty(a.held)} held</small>` : ""}</span><span class="pnl ${cls(pnl)}">${pnl == null ? "—" : sign(pnl, 1) + " bps"}</span>
        <span class="why">${p ? `target ${fmtPx(p.tp_px)} · stop ${fmtPx(p.sl_px)} · ${dur((p.deadline_ms - now()) / 1000)} left` : a.inventory_note && a.leaning === "short" ? esc(a.inventory_note) : a.blocked_by ? `${a.leaning === "short" ? "short" : "long"}: ${esc(why(a.blocked_by))}` : "all gates open"}</span></div>`;
    }).join("") + `</div>`;
  }

  function renderScalpHealth(el) {
    const S = scalp?.status;
    if (!S) { el.innerHTML = ""; return; }
    const rows = [
      [S.feed.connected ? "ok" : "error", "Market stream", S.feed.connected ? `live · ${S.feed.last_msg_ms_ago} ms ago` : esc(S.feed.last_error || "down")],
      [S.jev.failures ? "degraded" : "ok", "Jev review", `${S.jev.reviews} reviews · ${S.jev.vetoes} vetoed`],
      [S.jev.news_failed ? "degraded" : "ok", "News context", `${S.jev.events} headlines held`],
      [S.live_hold ? "error" : S.halted_today ? "degraded" : "ok", "Risk", S.live_hold ? "exchange hold" : S.halted_today ? "daily limit hit" : "entries allowed"],
    ];
    el.innerHTML = `<h3>Scalp desk health <span>${ago(S.ts)}</span></h3><div class="hl">` +
      rows.map(([st, n, d]) => `<div class="row"><span class="sdot ${st}" role="img" aria-label="${st}"></span><b>${n}</b><span>${d}</span></div>`).join("") + `</div>`;
  }

  function renderRunlist() {
    const el = $("#runlist");
    if (view === "scalp") { renderScalpRunlist(el); return; }
    if (!snap) { el.innerHTML = ""; return; }
    const S = snap.settings;
    const zIn = (1 + S.news_entry) / 2 * 100, zOut = (1 - S.news_exit) / 2 * 100;
    el.innerHTML = `<h3>News desk · run list <span>click to trace</span></h3><div class="rl">` + snap.assets.map((a) => {
      const st = assetState(a), L = a.latest;
      const sc = L ? L.news.score : 0;
      return `<button type="button" data-focus="${esc(a.symbol)}" aria-pressed="${focus?.key === a.symbol}">
        <span class="sym">${esc(a.symbol)}</span><span class="state">${esc(st.text)}</span><span class="pnl ${cls(st.pos?.pnl_pct)}">${st.pos ? pct(st.pos.pnl_pct) : "—"}</span>
        <span class="why">${L ? `news ${sign(sc)}${L.news.n_clusters ? ` (+${L.news.pos}/−${L.news.neg})` : ""} · mom ${L.momentum == null ? "—" : pct(L.momentum, 1)}${/^news /.test(L.reason) ? "" : " · " + esc(L.reason)}` : "waiting for the first tick"}</span>
        <span class="gauge" aria-hidden="true"><span class="track"></span><span class="zone out" style="left:0;width:${zOut}%"></span><span class="zone in" style="left:${zIn}%;right:0"></span><span class="mid"></span>${L ? `<span class="needle" style="left:${((1 + sc) / 2) * 100}%"></span>` : ""}</span>
      </button>`;
    }).join("") + `</div><p class="rl-legend"><span><i style="background:rgba(179,48,76,.25)"></i>exit below ${sign(-S.news_exit)}</span><span><i style="background:rgba(47,107,79,.25)"></i>buy above ${sign(S.news_entry)}</span><span>needle: news score</span></p>`;
  }

  function renderHealth() {
    const el = $("#health");
    if (view === "scalp") { renderScalpHealth(el); return; }
    const st = snap?.status;
    if (!st) { el.innerHTML = `<h3>Pipeline</h3><p style="color:var(--mute)">No ticks yet.</p>`; return; }
    const total = st.stages.reduce((s, x) => s + x.ms, 0);
    el.innerHTML = `<h3>News desk · pipeline <span>${st.last_tick_at ? ago(st.last_tick_at) : "starting"}</span></h3><div class="hl">` +
      st.stages.map((s) => `<div class="row"><span class="sdot ${s.state}" role="img" aria-label="${s.state}"></span><b>${esc(s.name)}</b><span title="${esc(s.detail)}">${esc(s.detail)}</span></div>`).join("") +
      `</div><p class="foot">Tick ${st.tick.toLocaleString()} · ${total.toLocaleString()} ms${st.jev_window ? ` · Jev ${st.jev_window[0]}/${st.jev_window[1]} failed` : ""}${st.jev_pending ? ` · ${st.jev_pending} retrying` : ""}${snap.spiked ? ` · ${snap.spiked} spiked` : ""}${st.tokens_today && st.tokens_today[0] ? ` · ${st.tokens_today[0]} calls today` : ""}</p>`;
  }

  // ---------- render all ----------
  function render() {
    renderBeat();
    renderBar();
    renderPosture();
    renderNotices();
    const el = $("#view");
    if (!snap) { el.innerHTML = `<div style="height:18px"></div><div class="empty"><h3>Connecting to the desk…</h3><p>Waiting for the dashboard server.</p></div>`; }
    else if (view === "wire") renderWire(el);
    else if (view === "run") renderRun(el);
    else if (view === "pipeline") renderPipeline(el);
    else if (view === "score") renderScore(el);
    else if (view === "scalp") renderScalp(el);
    else renderTrades(el);
    renderEq();
    renderRunlist();
    renderHealth();
    if (snap) el.insertAdjacentHTML("afterbegin", guideHtml());
    if (scrollToFocus && view === "wire") {
      scrollToFocus = false;
      const first = document.querySelector(".slip.focus");
      if (first) first.scrollIntoView({ block: "center", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
    }
  }

  function onSnapshot(s, receivedAt) {
    snap = s;
    skew = Date.parse(s.now) - receivedAt;
    if (!lastRead) {
      lastRead = s.events[0]?.analyzed_at || s.now;
      store.set("jev.lastRead", lastRead);
    }
    if (flattenArmed && !(s.account?.positions?.length)) disarm();
    render();
  }

  // ---------- actions ----------
  async function post(path, body) {
    busy = true; actionError = null; renderBar();
    try {
      const r = await fetch(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body || {}) });
      if (!r.ok) throw new Error((await r.text()) || `Server answered ${r.status}.`);
      const control = await r.json();
      if (snap) {
        snap.control = control;
        if (path.endsWith("flatten") && snap.account) snap.account.flatten_pending = true;
      }
    } catch (e) {
      actionError = e.message === "Failed to fetch" ? "The dashboard server isn't reachable." : e.message;
    } finally {
      busy = false; render(); poll();
    }
  }
  function disarm() { flattenArmed = false; clearTimeout(armTimer); }

  $("#btn-pause").addEventListener("click", () => post("/api/control/pause", { paused: !snap?.control?.paused }));
  $("#btn-flatten").addEventListener("click", () => {
    if (!flattenArmed) {
      flattenArmed = true; clearTimeout(armTimer);
      armTimer = setTimeout(() => { disarm(); renderBar(); }, 6000);
      renderBar();
      return;
    }
    disarm();
    post("/api/control/flatten");
  });
  $("#btn-flatten-cancel").addEventListener("click", () => { disarm(); renderBar(); });

  document.addEventListener("click", (e) => {
    const b = e.target.closest("[data-act],[data-tab],[data-focus],[data-trade]");
    if (b) {
      if (b.dataset.tab) { tab = b.dataset.tab; shown = 60; }
      else if (b.dataset.focus) {
        const sym = b.dataset.focus;
        if (focus?.key === sym && view === "wire") focus = null;
        else {
          const st = assetState(snap.assets.find((a) => a.symbol === sym));
          focus = { key: sym, label: `${sym} · ${st.text.toLowerCase()}`, ids: driversFor(sym) };
          tab = "all"; assetFilter = "all"; scrollToFocus = true;
          if (view !== "wire") { location.hash = "#wire"; return; }
        }
      } else if (b.dataset.trade) {
        const f = snap.trades[+b.dataset.trade];
        focus = { key: "trade", label: `${f.side} ${f.asset} at ${t(f.ts)}`, ids: new Set(f.events) };
        tab = "all"; assetFilter = "all"; scrollToFocus = true;
        location.hash = "#wire"; return;
      } else {
        const a = b.dataset.act;
        if (a === "more") shown += 60;
        if (a === "readall") { lastRead = new Date(now()).toISOString(); readIds = new Set(); store.set("jev.lastRead", lastRead); store.set("jev.readIds", []); }
        if (a === "clearfocus") focus = null;
        if (a === "guide-hide") { hiddenGuides.add(view); store.set("jev.guides.hidden", [...hiddenGuides]); }
        if (a === "guide-show") { hiddenGuides.delete(view); store.set("jev.guides.hidden", [...hiddenGuides]); }
        if (a === "clearfilters") { tab = "all"; assetFilter = "all"; }
        if (a === "resume") { post("/api/control/pause", { paused: false }); return; }
        if (a === "dismiss") actionError = null;
      }
      render();
      return;
    }
    const slip = e.target.closest(".slip.unread");
    if (slip && !e.target.closest("a")) {
      readIds.add(slip.dataset.id);
      store.set("jev.readIds", [...readIds].slice(-500));
      slip.classList.remove("unread");
      slip.querySelector(".new")?.remove();
    }
  });
  document.addEventListener("change", (e) => {
    if (e.target.id === "asset-filter") { assetFilter = e.target.value; shown = 60; render(); }
  });

  // ---------- routing + data ----------
  function route() {
    const v = (location.hash || "#wire").slice(1);
    view = VIEWS[v] ? v : "wire";
    if (view !== "wire") firstWire = true;
    if (view === "scalp") pollScalp();
    $("#work").scrollTop = 0;
    window.scrollTo(0, 0);
    render();
  }
  window.addEventListener("hashchange", route);

  let etag = null;
  async function poll() {
    try {
      const r = await fetch("/api/snapshot", { headers: etag ? { "If-None-Match": etag } : {}, cache: "no-store" });
      if (r.status === 304) { if (conn !== "live") { conn = "live"; render(); } return; }
      if (!r.ok) throw new Error(String(r.status));
      etag = r.headers.get("ETag");
      conn = "live";
      onSnapshot(await r.json(), Date.now());
    } catch {
      if (conn !== "lost") { conn = "lost"; renderNotices(); renderBeat(); }
    }
  }
  (async function loop() { await poll(); await pollScalp(view !== "scalp"); setTimeout(loop, document.hidden ? 10000 : 2000); })();
  document.addEventListener("visibilitychange", () => { if (!document.hidden) poll(); });

  setInterval(() => {
    if (!snap) return;
    renderBeat();
    if (view === "pipeline") renderView();
  }, 1000);
  function renderView() { const el = $("#view"); if (view === "pipeline") { renderPipeline(el); el.insertAdjacentHTML("afterbegin", guideHtml()); } }

  // Keep Pause/Flatten in view: at mid widths the glance block sits above the wire,
  // so the controls move into the sticky rail.
  const controls = document.querySelector(".controls");
  const midMq = matchMedia("(max-width:1240px) and (min-width:761px)");
  function placeControls() {
    if (midMq.matches) $("#beat").before(controls);
    else document.querySelector(".bar").append(controls);
  }
  midMq.addEventListener("change", placeControls);
  placeControls();
  // The focus bar sticks just under the header, whatever its height.
  new ResizeObserver(([e]) => document.documentElement.style.setProperty("--bar-h", Math.ceil(e.target.getBoundingClientRect().height) + "px")).observe(document.querySelector(".bar"));

  route();
})();
