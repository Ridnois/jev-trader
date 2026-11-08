"use strict";
// jev-trader v2 dashboard: polls /api/state every 2s and redraws. Read-only except
// Pause entries / Flatten all (written to control.json, picked up by the engine).

const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const usd = (x, sign = false) => x == null || !isFinite(x) ? "–" : (sign && x > 0 ? "+" : x < 0 ? "−" : "") + "$" + Math.abs(x).toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const num = (x, d = 1, sign = false) => x == null || !isFinite(x) ? "–" : (sign && x > 0 ? "+" : "") + Number(x).toFixed(d).replace("-", "−");
const cls = (x) => (x > 0 ? "pos" : x < 0 ? "neg" : "");
const ago = (iso) => {
  if (!iso) return "–";
  const s = (Date.now() - new Date(iso).getTime()) / 1000;
  return s < 90 ? `${Math.round(s)}s` : s < 5400 ? `${Math.round(s / 60)}m` : s < 172800 ? `${Math.round(s / 3600)}h` : `${Math.round(s / 86400)}d`;
};
const sideTag = (s) => `<span class="tag ${esc(s)}">${esc(s)}</span>`;
const px = (x) => (x == null ? "–" : x >= 1000 ? x.toFixed(1) : x >= 1 ? x.toFixed(4) : x.toPrecision(5));
function bar(v) {
  const x = Math.max(-1, Math.min(1, v || 0));
  const w = Math.abs(x) * 50;
  const left = x >= 0 ? 50 : 50 - w;
  return `<span class="bar" title="${num(x, 2, true)}"><i style="left:${left}%;width:${w}%;background:${x >= 0 ? "var(--green)" : "var(--red)"}"></i></span>`;
}
function table(head, rows, empty) {
  if (!rows.length) return `<p class="empty">${empty}</p>`;
  return `<table><thead><tr>${head.map((h) => `<th class="${h.startsWith("#") ? "num" : ""}">${esc(h.replace(/^#/, ""))}</th>`).join("")}</tr></thead><tbody>${rows.join("")}</tbody></table>`;
}

let state = null;
let flattenArmed = 0;

async function load() {
  try {
    const r = await fetch("/api/state", { cache: "no-store" });
    state = await r.json();
    render();
  } catch (e) {
    $("beat").className = "beat bad";
    $("beat").querySelector("span").textContent = "Dashboard server unreachable";
  }
}

function render() {
  const S = state, st = S.status;
  const fresh = st && Date.now() - new Date(st.ts).getTime() < 10000;
  const beat = $("beat");
  if (!st) {
    beat.className = "beat bad";
    beat.querySelector("span").textContent = `No engine status in ${S.data_dir}. Start it: jev-trader run`;
  } else if (!S.alive || !fresh) {
    beat.className = "beat bad";
    beat.querySelector("span").textContent = `Engine stopped · last seen ${ago(st.ts)} ago`;
  } else {
    const feedOk = st.feed.connected && st.feed.last_msg_ms_ago < 10000;
    beat.className = "beat " + (feedOk ? "ok" : "warn");
    beat.querySelector("span").textContent = `Engine running since ${ago(st.session.started)} · market feed ${feedOk ? `live (${st.feed.coins} coins)` : "reconnecting"} · news checked ${ago(st.intel.last_cycle)} ago`;
  }
  const v = $("venue");
  v.textContent = !st ? "…" : st.live ? (st.network === "testnet" ? "TESTNET · FAKE MONEY" : "LIVE · REAL MONEY") : "PAPER · REAL PRICES";
  v.className = "venue " + (st?.live ? st.network : "");
  const paused = S.control?.paused;
  $("pause").textContent = paused ? "Resume entries" : "Pause entries";
  $("pause").classList.toggle("on", !!paused);
  if (!st) return;

  // Notices.
  const a = st.account, notes = [];
  if (a.live_hold) notes.push([`Hold: ${a.live_hold}`, true]);
  if (a.halted_today) notes.push(["Daily loss limit hit: no new entries until 00:00 UTC.", true]);
  if (paused) notes.push(["Entries paused from the dashboard. Exits, stops and take-profits keep running."]);
  for (const p of st.positions) if (!p.protected) notes.push([`${p.coin}: no exchange-side backstop stop; protected only while the engine runs.`, true]);
  if (st.jev.today.failures > 0) notes.push([`Jev: ${st.jev.today.failures} failed calls today.`]);
  $("notices").innerHTML = notes.map(([t, bad]) => `<div class="notice ${bad ? "bad" : ""}">${esc(t)}</div>`).join("");

  // Money.
  const unreal = st.positions.reduce((s, p) => s + p.unrealized, 0);
  const risk = st.positions.reduce((s, p) => s + p.risk_usd, 0);
  const result = a.equity - a.capital;
  const k = (label, value, sub, c = "") => `<div class="kpi"><span>${label}</span><b class="${c}">${value}</b><small>${sub}</small></div>`;
  $("kpis").innerHTML = [
    k("Equity", usd(a.equity), `${st.live ? "managing" : "paper"} capital ${usd(a.capital)}`),
    k("Result after fees", usd(result, true), `${num((result / a.capital) * 100, 2, true)}% · fees ${usd(a.fees)} · funding ${usd(a.funding, true)}`, cls(result)),
    k("Today", usd(a.day_trade_pnl, true), `limit −${num(st.settings.risk.daily_loss_pct * 100, 1)}% of capital`, cls(a.day_trade_pnl)),
    k("Open", `${st.positions.length} / ${st.settings.risk.max_positions}`, `unrealized ${usd(unreal, true)} · at risk ${usd(risk)}`),
    k("Won / lost", `${a.wins} / ${a.losses}`, `${a.entries_last_hour}/${st.settings.risk.max_entries_per_hour} entries this hour`),
    k("Jev today", `${(st.jev.today.input + st.jev.today.output).toLocaleString()} tok`, `${st.jev.today.calls} calls · ${st.jev.tokens_per_headline ? num(st.jev.tokens_per_headline, 0) : "–"} per headline (v1: 751)`),
  ].join("");

  // Verdict.
  $("summary").innerHTML = (S.score?.summary || []).map((l) => `<li>${esc(l)}</li>`).join("") || "<li>No data yet.</li>";

  // Open trades, watches, resting orders.
  const openRows = st.positions.map((p) => {
    const q = p.pos;
    const age = (Date.now() - q.opened_ms) / 60000;
    return `<tr><td><b>${esc(p.coin)}</b><br><span class="tag">${esc(q.kind)}</span></td><td>${sideTag(q.side)}</td>
      <td class="num">${px(q.entry_px)}<br><span class="dim">now ${px(p.mark)}</span></td>
      <td class="num">${px(q.sl_px)}<br><span class="dim">target ${px(q.tp_px)}</span></td>
      <td class="num ${cls(p.unrealized)}">${usd(p.unrealized, true)}<br><span class="dim">${Math.round(age)}m</span></td>
      <td class="why">${esc(q.why)}${q.closing ? `<br><b class="neg">closing: ${esc(q.closing)}</b>` : ""}</td></tr>`;
  });
  const watchRows = st.watches.map((w) => `<tr><td><b>${esc(w.coin)}</b><br><span class="tag">watch</span></td><td>${sideTag(w.side)}</td>
      <td class="num">${bar(w.impact)}<br><span class="dim">new ${num(w.new_info, 2)}</span></td><td class="num">${w.secs_left}s left</td>
      <td>waiting for <b>${esc(w.waiting_for)}</b></td><td class="why">${esc(w.title)} <span class="dim">[${esc(w.source)}]</span></td></tr>`);
  const restRows = st.resting.map((r) => `<tr><td><b>${esc(r.coin)}</b><br><span class="tag">maker order</span></td><td>${sideTag(r.side)}</td><td class="num">${px(r.px)}</td><td class="num">${usd(r.usd)}</td><td colspan="2">resting ${Math.round(r.age_ms / 1000)}s</td></tr>`);
  $("open").innerHTML = openRows.length || watchRows.length || restRows.length
    ? `${openRows.length ? table(["Coin", "Side", "#Entry", "#Stop", "#P&L", "Why"], openRows, "") : ""}${watchRows.length || restRows.length ? table(["Coin", "Side", "#Impact", "#", "Status", "Headline"], [...watchRows, ...restRows], "") : ""}`
    : `<p class="empty">Nothing open. A trade needs a strong, fresh headline confirmed by order flow (catalyst), or an order-flow burst the news agrees with (flow).</p>`;

  // Coins.
  $("coins").innerHTML = table(["Coin", "#Price", "News bias", "#Flow", "#Funding bp/h", "Waiting for"], st.coins.map((c) => `<tr>
      <td><b>${esc(c.coin)}</b>${c.hack ? ' <span class="tag short">hack</span>' : ""}</td><td class="num">${px(c.mid)}</td>
      <td>${bar(c.bias)} <span class="dim">${num(c.bias, 2, true)} · ${c.bias_n}</span></td>
      <td class="num ${cls(c.flow)}">${num(c.flow, 2, true)}</td><td class="num">${num(c.funding_bps_1h, 3, true)}</td>
      <td>${c.exchange_review ? `<b class="neg">${esc(c.exchange_review)}</b>` : esc(c.block || (c.warm === false ? "warm-up" : "–"))}</td></tr>`), "Waiting for the market feed.");

  // News funnel + feed.
  const f = st.intel.funnel;
  $("funnel").innerHTML = `<span><b>${f.fresh}</b> fresh</span><span>dropped free: <b>${f.noise}</b> noise · <b>${f.recap}</b> price recaps · <b>${f.untradable}</b> untradable · <b>${f.duplicate}</b> duplicates</span><span>→ <b>${f.analyzed}</b> scored by Jev</span><span><b>${st.intel.queue}</b> queued</span><span><b>${st.jev.calls_last_hour}</b>/${st.jev.max_calls_per_hour} calls this hour</span>`;
  $("news").innerHTML = table(["Seen", "Source", "Coin", "Impact", "#New fact", "#Lag", "Headline"], st.news.slice(0, 25).map((e) => `<tr>
      <td class="dim">${ago(e.first_seen)}</td><td>${esc(e.source)}${e.kind !== "news" ? ` <span class="tag listing">${esc(e.kind)}</span>` : ""}</td>
      <td>${esc((e.coins && e.coins[0]) || "market")}</td><td>${bar(e.impact)} ${num(e.impact, 2, true)}</td>
      <td class="num">${num(e.new_info, 2)}</td><td class="num">${num(e.lag_min, 0)}m</td><td>${esc(e.title)}${e.hack != null && e.hack > 0.7 ? ' <span class="tag short">hack</span>' : ""}</td></tr>`), "No headlines scored yet.");

  // Trades.
  $("trades").innerHTML = table(["Closed", "Coin", "Kind", "Side", "#Net", "#bps", "Exit", "Why"], S.trades.slice().reverse().slice(0, 30).map((t) => `<tr>
      <td class="dim">${ago(t.closed)}</td><td><b>${esc(t.coin)}</b></td><td>${esc(t.kind)}</td><td>${sideTag(t.side)}</td>
      <td class="num ${cls(t.pnl_usd)}">${usd(t.pnl_usd, true)}</td><td class="num ${cls(t.pnl_bps)}">${num(t.pnl_bps, 1, true)}</td>
      <td>${esc(t.reason)}</td><td class="why">${esc(t.why)}</td></tr>`), "No closed trades yet.");

  // Catalyst decisions.
  $("cands").innerHTML = table(["When", "Coin", "Side", "Outcome", "Headline"], S.candidates.filter((c) => c.kind === "catalyst").slice(0, 25).map((c) => `<tr>
      <td class="dim">${ago(c.ts)}</td><td><b>${esc(c.coin)}</b></td><td>${sideTag(c.side)}</td>
      <td>${esc(c.outcome)}</td><td class="why">${esc(c.title)}</td></tr>`), "No catalyst-grade headline yet (|impact| ≥ " + st.settings.catalyst.min_impact + ", new fact ≥ " + st.settings.catalyst.min_new_info + ", seen within " + st.settings.catalyst.max_lag_min + " min).");

  // Evidence.
  const sc = S.score || {};
  const er = (r) => {
    const e = r.est, thin = e.clusters < 20;
    const m = e.mean == null ? "–" : num(e.mean, 1, true);
    const ci = e.lo == null ? "–" : `${num(e.lo, 1, true)} … ${num(e.hi, 1, true)}`;
    const verdict = thin ? "too little data" : e.lo > 0 ? "pays" : e.hi < 0 ? "loses" : "unclear";
    return `<tr class="${thin ? "dim" : ""}"><td>${esc(r.label)}</td><td class="num ${thin ? "" : cls(e.mean)}">${m}</td><td class="num">${ci}</td><td class="num">${e.n}</td><td class="num">${e.clusters}</td><td>${verdict}</td></tr>`;
  };
  const head = ["", "#Mean bps", "#95% range", "#n", "#Hours", "Verdict"];
  $("evidence").innerHTML = `<h3>Trades</h3>${table(head, (sc.trades || []).map(er), "")}
    <h3>Strong headlines, in Jev's direction, taken or not</h3>${table(head, (sc.events || []).map(er), "")}
    <h3>Flow candidates: with vs without news support</h3>${table(head, (sc.flow || []).map(er), "No flow candidates yet.")}`;

  // Sources.
  const src = Object.entries(st.intel.sources).sort((a, b) => a[0].localeCompare(b[0]));
  $("sources").innerHTML = `<div class="sources">${src.map(([n, h]) => `<div class="${h.ok ? "" : "bad"}" title="${esc(h.error)}"><b>${esc(n)}</b> · ${h.ok ? `${h.items} items` : "failing"}${h.median_lag_min != null ? ` · lag ${num(h.median_lag_min, 0)}m` : ""}</div>`).join("")}</div>`;
}

$("pause").addEventListener("click", async () => {
  const paused = !state?.control?.paused;
  await fetch("/api/pause", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ paused }) });
  load();
});
$("flatten").addEventListener("click", async () => {
  const b = $("flatten");
  if (Date.now() - flattenArmed > 5000) {
    flattenArmed = Date.now();
    b.textContent = "Click again to close everything";
    b.classList.add("armed");
    setTimeout(() => { b.textContent = "Flatten all"; b.classList.remove("armed"); }, 5000);
    return;
  }
  flattenArmed = 0;
  b.textContent = "Flatten all";
  b.classList.remove("armed");
  await fetch("/api/flatten", { method: "POST" });
  load();
});

load();
setInterval(load, 2000);
