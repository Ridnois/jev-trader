import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { familyName, strategyTag } from "@/lib/format";
import { entryClauses, ruleSentence, type ComposedRule } from "@/lib/v3/rule";
import type {
  AutoRun, Calibration, RuleFamily, RuleRow, Rules, Candidate, Card, CycleReport, DayRow, EngineStatus, EvolveCfg, EvolveSchedule, LoopCalibration, FamilyReport, GenNode, GenRoot, Generations,
  HorizonSummary, Hypothesis, Lab, LabSummary, LineageEvent, Live, NearMiss, NewsEvent, Pairing, ProbeReport, ProposalRow, Quote, StrategyView,
  OnPaper, StudyCost, Trade, WalkCell, WalkFamily,
} from "./types";

/** The v3 worktree: the folder that holds data-v3/, lab-data/, strategies/ and lab.toml. */
export const ROOT = process.env.JEV_V3_ROOT ? path.resolve(process.env.JEV_V3_ROOT) : path.resolve(process.cwd(), "..");
const DATA = path.join(ROOT, "data-v3");
const LAB = path.join(ROOT, "lab-data");

function readText(p: string): string | null {
  try {
    return fs.readFileSync(p, "utf8");
  } catch {
    return null;
  }
}

function readJson<T>(p: string): T | null {
  const t = readText(p);
  if (!t) return null;
  try {
    return JSON.parse(t) as T;
  } catch {
    return null; // caught mid-write; the next poll reads it whole
  }
}

function readJsonl<T>(p: string): T[] {
  const t = readText(p);
  if (!t) return [];
  const out: T[] = [];
  for (const line of t.split("\n")) {
    if (!line) continue;
    try {
      out.push(JSON.parse(line) as T);
    } catch {
      /* a half-written last line */
    }
  }
  return out;
}

function pidAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    return (e as NodeJS.ErrnoException).code === "EPERM";
  }
}

/** Start of the ledger's current era (`era_start` in lab.toml), in ms; trials and loop spend before it no longer count. */
function eraStart(): number | null {
  const m = (readText(path.join(ROOT, "lab.toml")) ?? "").match(/^era_start\s*=\s*"([^"]+)"/m);
  const t = m ? Date.parse(m[1]) : NaN;
  return Number.isFinite(t) ? t : null;
}

/** Whether a lineage row's time falls in the current era. */
function inEra(at: unknown, era: number | null): boolean {
  if (era === null) return true;
  const t = typeof at === "string" ? Date.parse(at) : NaN;
  return Number.isFinite(t) && t >= era;
}

function tomlNumber(text: string, key: string, fallback: number): number {
  const m = text.match(new RegExp(`^${key}\\s*=\\s*([-\\d.eE]+)`, "m"));
  return m ? Number(m[1]) : fallback;
}

function catalystCfg() {
  const t = readText(path.join(ROOT, "lab.toml")) ?? "";
  const list = t.match(/^markouts_secs\s*=\s*\[([^\]]*)\]/m);
  return {
    min_impact: tomlNumber(t, "min_impact", 0.45),
    min_new_info: tomlNumber(t, "min_new_info", 0.6),
    max_lag_min: tomlNumber(t, "max_lag_min", 20),
    markouts_secs: list ? list[1].split(",").map((s) => Number(s.trim())).filter(Boolean) : [300, 1800, 7200, 21600],
  };
}

// ---- 1-second recorder: read only the bytes appended since the last poll ----

type Tape = { file: string; offset: number; carry: string; minutes: Map<number, number>; open: number; low: number; high: number; lastSec: number; mid: number };
const tapes = new Map<string, Tape>();
const SPARK_MINUTES = 180;

function latestTickFile(coin: string): string | null {
  const dir = path.join(DATA, "ticks", coin);
  const today = path.join(dir, `${new Date().toISOString().slice(0, 10)}.csv`);
  if (fs.existsSync(today)) return today;
  try {
    const files = fs.readdirSync(dir).filter((f) => f.endsWith(".csv")).sort();
    return files.length ? path.join(dir, files[files.length - 1]) : null;
  } catch {
    return null;
  }
}

function readTape(coin: string): Quote | null {
  const file = latestTickFile(coin);
  if (!file) return null;
  let tape = tapes.get(coin);
  if (!tape || tape.file !== file) {
    tape = { file, offset: 0, carry: "", minutes: new Map(), open: 0, low: Infinity, high: -Infinity, lastSec: 0, mid: 0 };
    tapes.set(coin, tape);
  }
  try {
    const size = fs.statSync(file).size;
    if (size < tape.offset) tape.offset = 0;
    if (size > tape.offset) {
      const fd = fs.openSync(file, "r");
      const buf = Buffer.alloc(size - tape.offset);
      fs.readSync(fd, buf, 0, buf.length, tape.offset);
      fs.closeSync(fd);
      tape.offset = size;
      const lines = (tape.carry + buf.toString("utf8")).split("\n");
      tape.carry = lines.pop() ?? "";
      for (const line of lines) {
        const a = line.indexOf(",");
        const b = line.indexOf(",", a + 1);
        if (a < 0 || b < 0) continue;
        const sec = Number(line.slice(0, a));
        const mid = Number(line.slice(a + 1, b));
        if (!(mid > 0) || !sec) continue;
        if (!tape.open) tape.open = mid;
        if (mid < tape.low) tape.low = mid;
        if (mid > tape.high) tape.high = mid;
        tape.lastSec = sec;
        tape.mid = mid;
        tape.minutes.set(Math.floor(sec / 60), mid);
      }
      const cutoff = Math.floor(tape.lastSec / 60) - SPARK_MINUTES;
      for (const k of tape.minutes.keys()) {
        if (k < cutoff) tape.minutes.delete(k);
        else break;
      }
    }
  } catch {
    return null;
  }
  if (!tape.mid) return null;
  const lastMin = Math.floor(tape.lastSec / 60);
  // The mid an hour ago: that minute, or the nearest sampled minute up to 5 earlier (the recorder can skip minutes).
  let hourAgo: number | undefined;
  for (let k = lastMin - 60; k >= lastMin - 65 && hourAgo === undefined; k--) hourAgo = tape.minutes.get(k);
  return {
    coin,
    mid: tape.mid,
    lastSec: tape.lastSec,
    open: tape.open,
    low: tape.low,
    high: tape.high,
    chg1h: hourAgo ? (tape.mid / hourAgo - 1) * 100 : null,
    chgDay: tape.open ? (tape.mid / tape.open - 1) * 100 : null,
    spark: [...tape.minutes.values()],
  };
}

function quotes(): Quote[] {
  let coins: string[] = [];
  try {
    coins = fs.readdirSync(path.join(DATA, "ticks")).sort();
  } catch {
    /* recorder never ran */
  }
  const majors = ["BTC", "ETH", "SOL", "HYPE", "XRP", "BNB", "DOGE"];
  coins.sort((a, b) => {
    const [ia, ib] = [majors.indexOf(a), majors.indexOf(b)];
    return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib) || a.localeCompare(b);
  });
  return coins.map(readTape).filter((q): q is Quote => q !== null);
}

// ---- strategies and the sequential test (the same arithmetic as `trade monitor`) ----

const MIN_UPTIME = 0.9;

function strategies(status: EngineStatus | null): StrategyView[] {
  const dir = path.join(ROOT, "strategies");
  let files: string[] = [];
  try {
    files = fs.readdirSync(dir).filter((f) => f.endsWith(".json")).sort();
  } catch {
    /* none promoted */
  }
  const daily = readJsonl<{ strategy: string; day: string; ret: number; up_secs: number }>(path.join(DATA, "daily.jsonl"));
  const monitor = readText(path.join(DATA, "monitor.md")) ?? "";
  const parity = new Map<string, string>();
  for (const block of monitor.split(/^## /m).slice(1)) {
    const name = block.split(/[\s(]/)[0];
    const line = block.match(/^- Parity: (.+)$/m);
    if (name && line) parity.set(name, line[1].trim());
  }
  // Generation of each generated family, from the lineage; hand-written families are generation 0.
  const generation = new Map<string, number>();
  for (const r of readJsonl<Record<string, unknown>>(path.join(LAB, "lineage.jsonl"))) {
    if (r && typeof r === "object" && r.event === "proposed" && typeof r.family === "string") generation.set(r.family, num(r.generation));
  }
  const out: StrategyView[] = [];
  for (const f of files) {
    const p = readJson<{ status: string; hypothesis: string; card: Card; rule?: ComposedRule; label?: string }>(path.join(dir, f));
    if (!p?.card) continue;
    const name = f.replace(/\.json$/, "");
    const { mu_daily: mu, sd_daily: sd, alpha, beta } = p.card;
    const kill = Math.log(beta / (1 - alpha));
    const graduate = Math.log((1 - beta) / alpha);
    let llr = 0, cum = 0, peak = 0, dd = 0, fullDays = 0;
    const days: DayRow[] = [];
    for (const r of daily.filter((d) => d.strategy === name)) {
      const full = r.up_secs >= MIN_UPTIME * 86_400;
      if (full && sd > 0) {
        llr += (mu * r.ret) / (sd * sd) - (mu * mu) / (2 * sd * sd);
        cum += r.ret;
        peak = Math.max(peak, cum);
        dd = Math.max(dd, peak - cum);
        fullDays += 1;
      }
      days.push({ day: r.day, ret: r.ret, up_secs: r.up_secs, full, llr });
    }
    const limit = fullDays <= 30 ? p.card.dd99[0] : fullDays <= 60 ? p.card.dd99[1] : p.card.dd99[2];
    let call: StrategyView["call"] = "continue";
    let callWhy = "Not enough paper days to decide either way.";
    if (limit > 0 && dd > limit) {
      call = "kill";
      callWhy = `Drawdown ${(dd * 100).toFixed(2)}% is past the ${(limit * 100).toFixed(2)}% the backtest allows.`;
    } else if (mu <= 0) {
      call = "kill";
      callWhy = "The card predicts no positive return.";
    } else if (llr <= kill) {
      call = "kill";
      callWhy = "The sequential test now favours no edge.";
    } else if (llr >= graduate) {
      call = "graduate";
      callWhy = "The paper run supports the predicted edge. Going live is still your decision.";
    }
    const report = readJson<{ verdict: string; checks: { name: string; pass: boolean; fatal: boolean }[] }>(path.join(LAB, "reports", p.card.study, `${p.card.spec.family}.json`));
    const stopped = report?.checks.find((c) => (report.verdict === "INSUFFICIENT_EVIDENCE" ? !c.pass : c.fatal && !c.pass));
    out.push({
      labVerdict: report?.verdict ?? null, labStoppedAt: stopped?.name ?? null,
      name, status: p.status, hypothesis: p.hypothesis, card: p.card,
      family: p.label ?? familyName(p.card.spec.family), tag: strategyTag(p.label ?? p.card.spec.family, generation.get(p.card.spec.family) ?? 0, p.label ? null : p.card), rule: ruleSentence(p.rule, p.card), entry: entryClauses(p.rule, p.card),
      book: status?.books.find((b) => b.strategy === name) ?? null,
      days, llr, kill, graduate, fullDays, partialDays: days.length - fullDays, drawdown: dd, drawdownLimit: limit, call, callWhy,
      parity: parity.get(name) ?? null,
    });
  }
  return out;
}

// ---- the live snapshot ----

let cached: { at: number; live: Live } | null = null;

export function getLive(): Live {
  const now = Date.now();
  if (cached && now - cached.at < 900) return cached.live;
  const lab = getLab();
  // Verdicts from before the ledger's era faced another bar: they are kept as a record, not ranked.
  const judgedAll = [...lab.families, ...lab.generated].filter((f) => !f.prior);
  // Fewest failed checks; a family stopped for want of evidence before any check ran has no distance.
  const fails = (f: FamilyReport) => f.checks.filter((c) => !c.pass);
  const near = judgedAll
    .filter((f) => f.checks.length > 0 && !(f.verdict === "INSUFFICIENT_EVIDENCE" && f.checks.every((c) => c.name === "evidence" || c.pass)) && f.verdict !== "PROMOTE_TO_PAPER")
    .sort((a, b) => fails(a).length - fails(b).length || (a.generation ?? 0) - (b.generation ?? 0) || (a.name ?? "").localeCompare(b.name ?? ""))[0];
  const nearCheck = near ? fails(near)[0] : undefined;
  const labSummary: LabSummary = {
    families: judgedAll.length,
    generated: lab.generated.filter((f) => !f.prior).length,
    prior: [...lab.families, ...lab.generated].filter((f) => f.prior).length,
    era: lab.era,
    closest: near && nearCheck ? { name: near.name ?? near.family, generation: near.generation ?? null, failed: fails(near).length, check: nearCheck.name, value: nearCheck.value.replace(/\s*\(.*$/, ""), need: nearCheck.need } : null,
    promoted: judgedAll.filter((f) => f.verdict === "PROMOTE_TO_PAPER").length,
    lastJudged: judgedAll.map((f) => f.judged).filter(Boolean).sort().at(-1) ?? null,
    lockboxSealed: judgedAll.every((f) => !f.lockbox_opened),
    lockboxDays: lab.families.length ? Math.max(...lab.families.map((f) => f.lockbox_days)) : 0,
    questions: lab.probes.flatMap((p) => p.questions).length,
    acceptedQuestions: lab.probes.flatMap((p) => p.questions).filter((q) => q.auc > 0.55 && q.ic_null_pct > 95).length,
    lastScored: lab.probes.map((p) => p.scored).filter(Boolean).sort().at(-1) ?? null,
  };
  const status = readJson<EngineStatus>(path.join(DATA, "status.json"));
  const ledger = readJson<{ started?: string }>(path.join(DATA, "ledger.json"));
  const ageSec = status ? Math.max(0, (now - Date.parse(status.ts)) / 1000) : null;
  const alive = !!status && pidAlive(status.pid);
  const cfg = catalystCfg();

  const marks = readJsonl<{ id: string; ts: string; h_secs: number; ret_bps: number; net_bps: number; triggered: boolean }>(path.join(DATA, "catalyst_markouts.jsonl"));
  const byId = new Map<string, Candidate["markouts"]>();
  for (const m of marks) {
    const o = byId.get(m.id) ?? {};
    o[String(m.h_secs)] = { ret_bps: m.ret_bps, net_bps: m.net_bps };
    byId.set(m.id, o);
  }
  const allCandidates = readJsonl<Omit<Candidate, "markouts">>(path.join(DATA, "catalyst_candidates.jsonl"));
  const candidates: Candidate[] = allCandidates.slice(-200).reverse().map((c) => ({ ...c, markouts: byId.get(c.id) ?? {} }));
  const trades = readJsonl<Trade>(path.join(DATA, "trades.jsonl")).slice(-50).reverse();
  const horizons: HorizonSummary[] = cfg.markouts_secs.map((secs) => {
    const at = marks.filter((m) => m.h_secs === secs);
    const trig = at.filter((m) => m.triggered);
    const mean = (xs: typeof at) => (xs.length ? xs.reduce((s, m) => s + m.net_bps, 0) / xs.length : null);
    return { secs, triggeredN: trig.length, triggeredMean: mean(trig), allN: at.length, allMean: mean(at) };
  });
  const allNews = readJsonl<NewsEvent>(path.join(DATA, "news_events.jsonl"));

  const live: Live = {
    now,
    lab: labSummary,
    engine: { status, alive, stale: ageSec === null || ageSec > 90, ageSec, started: ledger?.started ?? null, lastStatusTs: status?.ts ?? null, catalystCfg: cfg },
    strategies: strategies(status),
    trades,
    candidates,
    candidateCount: allCandidates.length,
    triggeredCount: allCandidates.filter((c) => c.triggered).length,
    news: allNews.slice(-120).reverse(),
    newsCount: allNews.length,
    horizons,
    markoutDays: new Set(marks.map((m) => m.ts.slice(0, 10))).size,
    newsDays: new Set(allCandidates.map((c) => c.ts.slice(0, 10))).size,
    firstScoredAt: allCandidates[0]?.ts ?? null,
    lastScoredAt: allCandidates[allCandidates.length - 1]?.ts ?? null,
    lastTradeAt: trades[0]?.closed_ms ?? null,
    quotes: quotes(),
  };
  cached = { at: now, live };
  return live;
}

// ---- the lab: reports written by `lab validate`, `lab jev score`, `lab calibrate` ----

const FAMILY_ORDER = ["meanrev", "trend", "xs_reversal", "funding_carry", "jev_gate"];

function hypotheses(): Hypothesis[] {
  const t = readText(path.join(ROOT, "hypotheses.toml")) ?? "";
  return t.split("[[hypothesis]]").slice(1).map((block) => {
    const get = (k: string) => {
      const m = block.match(new RegExp(`^${k}\\s*=\\s*"((?:[^"\\\\]|\\\\.)*)"`, "m"));
      return m ? m[1].replace(/\\"/g, '"').replace(/\\\\/g, "\\") : "";
    };
    return { id: get("id"), statement: get("statement"), status: get("status"), result: get("result"), pass_if: get("pass_if"), invalidate_if: get("invalidate_if") };
  });
}

function calibrations(): Calibration[] {
  const dir = path.join(LAB, "reports");
  let files: string[] = [];
  try {
    files = fs.readdirSync(dir).filter((f) => f.startsWith("calibration-") && !f.startsWith("calibration-evolve-") && f.endsWith(".md")).sort();
  } catch {
    /* never calibrated */
  }
  const order = (f: string) => (f.includes("null") ? 0 : 1);
  return files.sort((a, b) => order(a) - order(b) || a.localeCompare(b)).map((f) => {
    const lines = (readText(path.join(dir, f)) ?? "").split("\n");
    const rows: Calibration["rows"] = [];
    let inTable = false;
    for (const l of lines) {
      if (l.startsWith("| Outcome")) inTable = true;
      else if (inTable && l.startsWith("|") && !l.startsWith("|---")) {
        const c = l.split("|").map((s) => s.trim());
        rows.push({ outcome: c[1], panels: Number(c[2]), share: c[3] });
      } else if (inTable && !l.startsWith("|")) break;
    }
    return { title: (lines[0] ?? f).replace(/^#\s*Gate calibration:\s*/, ""), rows };
  });
}

/** `calibration-evolve-*.md`: the loop on synthetic panels, read from its title and its promotion table. */
function loopCalibrations(): LoopCalibration[] {
  const dir = path.join(LAB, "reports");
  let files: string[] = [];
  try {
    files = fs.readdirSync(dir).filter((f) => f.startsWith("calibration-evolve-") && f.endsWith(".md")).sort();
  } catch {
    return [];
  }
  const out: LoopCalibration[] = [];
  for (const file of files) {
    const t = readText(path.join(dir, file)) ?? "";
    const h = t.match(/seed `([^`]+)` \(([^)]+)\), (\d+) synthetic panels.*?, (\d+) generations, edge ([\d.]+)/);
    if (!h) continue;
    const row = (label: string) => Number(t.match(new RegExp(`^\\| ${label}[^|]*\\| (\\d+) \\|`, "m"))?.[1] ?? 0);
    out.push({
      file, seed: h[1], interval: h[2], panels: Number(h[3]), generations: Number(h[4]), edge: Number(h[5]), auto: file.includes("-auto-"),
      anyPromoted: row("Any promotion"), seedPromoted: row("Seed family promoted"), loopOnly: row("Promoted only by the loop"),
    });
  }
  return out;
}

const COMPOSED = /^c[0-9a-f]{12}$/;

let labCached: { at: number; lab: Lab } | null = null;

/** Every file in `strategies/`: the name the trader runs it under and the family its spec comes from. */
export function onPaper(): OnPaper[] {
  const dir = path.join(ROOT, "strategies");
  let files: string[] = [];
  try {
    files = fs.readdirSync(dir).filter((f) => f.endsWith(".json")).sort();
  } catch {
    return [];
  }
  const out: OnPaper[] = [];
  for (const f of files) {
    const p = readJson<{ status?: string; card?: { spec?: { family?: string } } }>(path.join(dir, f));
    const family = p?.card?.spec?.family;
    if (family) out.push({ family, name: f.replace(/\.json$/, ""), status: p?.status ?? "" });
  }
  return out;
}

export function getLab(): Lab {
  const now = Date.now();
  if (labCached && now - labCached.at < 5000) return labCached.lab;
  const reports = path.join(LAB, "reports");
  const families: FamilyReport[] = [];
  const generated: FamilyReport[] = [];
  const labToml = readText(path.join(ROOT, "lab.toml")) ?? "";
  const walkStart = tomlNumber(labToml, "train_days", 180) + tomlNumber(labToml, "purge_days", 5);
  // Generated families are named and numbered in the lineage; read it directly (getGenerations reads this function).
  const born = new Map<string, { name: string; generation: number }>();
  let loopTrials = 0;
  const era = eraStart();
  for (const r of readJsonl<Record<string, unknown>>(path.join(LAB, "lineage.jsonl"))) {
    if (!r || typeof r !== "object") continue;
    if (r.event === "proposed" && typeof r.family === "string") born.set(r.family, { name: str(r.name, r.family), generation: num(r.generation) });
    if (inEra(r.at, era)) loopTrials += num(r.trials);
  }
  // Hand-written rule families (families/*.toml with a [rule]) are named by their file's `name`, matched on hypothesis.
  const handNamed = new Map<string, string>();
  try {
    for (const f of fs.readdirSync(path.join(ROOT, "families")).filter((x) => x.endsWith(".toml"))) {
      const t = readText(path.join(ROOT, "families", f)) ?? "";
      const name = t.match(/^name\s*=\s*"([^"]+)"/m)?.[1];
      const hyp = t.match(/^hypothesis\s*=\s*"([^"]+)"/m)?.[1];
      if (name && hyp && /^\[rule\]/m.test(t)) handNamed.set(hyp, name);
    }
  } catch {
    /* no families dir */
  }
  const probes: ProbeReport[] = [];
  let studies: string[] = [];
  try {
    studies = fs.readdirSync(reports).filter((d) => fs.statSync(path.join(reports, d)).isDirectory());
  } catch {
    /* lab never ran */
  }
  for (const study of studies) {
    for (const f of fs.readdirSync(path.join(reports, study)).filter((x) => x.endsWith(".json"))) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const d = readJson<any>(path.join(reports, study, f));
      if (!d) continue;
      let judged = "";
      try {
        judged = fs.statSync(path.join(reports, study, f)).mtime.toISOString();
      } catch {
        /* file vanished between read and stat */
      }
      if (Array.isArray(d.checks)) {
        let cum = 0;
        const oosCum = ((d.walk?.oos ?? []) as number[]).map((r) => (cum += r * 100));
        const hand = COMPOSED.test(String(d.family ?? "")) && !born.has(d.family) ? handNamed.get(String(d.hypothesis ?? "")) : undefined;
        const gen = COMPOSED.test(String(d.family ?? "")) && !hand ? born.get(d.family) ?? { name: d.family, generation: 0 } : null;
        // Walk-forward days count from the panel's first day. The out-of-sample line starts at the first possible
        // test fold (train + purge days), even when that fold picked nothing: such folds are flat, and only folds
        // with a pick are listed, so the first listed fold is not the start.
        const panelStart = String(d.panel ?? "").match(/(\d{4}-\d{2}-\d{2})\s*→/)?.[1];
        const testStart = walkStart;
        const plus = (days: number) => new Date(Date.parse(`${panelStart}T00:00:00Z`) + days * 86_400_000).toISOString().slice(0, 10);
        const oosStart = panelStart && typeof testStart === "number" ? plus(testStart) : null;
        const lockboxStart = panelStart && typeof d.design_days === "number" ? plus(d.design_days) : null;
        const panelEnd = String(d.panel ?? "").match(/→\s*(\d{4}-\d{2}-\d{2})/)?.[1] ?? null;
        (gen ? generated : families).push({
          ...(gen ? { name: gen.name, generation: gen.generation } : hand ? { name: hand } : {}),
          study: d.study, family: d.family, hypothesis: d.hypothesis, panel: d.panel, specs: d.specs,
          trials_ledger: d.trials_ledger, trials_effective: d.trials_effective, design_days: d.design_days,
          lockbox_days: d.lockbox_days, lockbox_opened: d.lockbox_opened, leaders: d.leaders ?? [], checks: d.checks,
          oosCum, oosStart, lockboxStart, panelEnd, folds: d.walk?.folds ?? [], verdict: d.verdict, notes: d.notes ?? [], judged,
          prior: era !== null && !!judged && Date.parse(judged) < era,
        });
      } else if (Array.isArray(d.questions)) {
        probes.push({ probe: d.probe, venue: d.venue, models: d.models, calls: d.calls, tokens_per_call: d.tokens_per_call, median_ms: d.median_ms, questions: d.questions, choices: d.choices ?? [], scored: judged });
      }
    }
  }
  const rank = (f: string) => (FAMILY_ORDER.indexOf(f) < 0 ? 99 : FAMILY_ORDER.indexOf(f));
  families.sort((a, b) => rank(a.family) - rank(b.family));
  probes.sort((a, b) => b.calls - a.calls);
  const byStudy = new Map<string, number>();
  let trials = 0;
  for (const line of (readText(path.join(LAB, "trials.jsonl")) ?? "").split("\n")) {
    const m = line.match(/"study":"([^"]+)"/);
    if (!m) continue;
    trials += 1;
    byStudy.set(m[1], (byStudy.get(m[1]) ?? 0) + 1);
  }
  generated.sort((a, b) => (a.name ?? "").localeCompare(b.name ?? ""));
  const lab: Lab = { families, generated, loopTrials, loopCalibrations: loopCalibrations(), probes, hypotheses: hypotheses(), calibrations: calibrations(), trials, trialsByStudy: [...byStudy].map(([study, n]) => ({ study, n })), onPaper: onPaper(), era: era === null ? null : new Date(era).toISOString() };
  labCached = { at: now, lab };
  return lab;
}

// ---- the evolve loop: `lab evolve` writes lineage.jsonl and reports/evolve/(dry-)gNNN.json ----

const str = (x: unknown, d = ""): string => (typeof x === "string" ? x : d);
const num = (x: unknown, d = 0): number => (typeof x === "number" && Number.isFinite(x) ? x : d);
const strs = (x: unknown): string[] => (Array.isArray(x) ? x.filter((v): v is string => typeof v === "string") : []);
const arr = (x: unknown): Record<string, unknown>[] => (Array.isArray(x) ? x.filter((v) => v && typeof v === "object") : []);
const optStr = (x: unknown): string | null => (typeof x === "string" ? x : null);

function lineageEvent(r: Record<string, unknown>): LineageEvent | null {
  if (typeof r.event !== "string" || typeof r.family !== "string") return null;
  return {
    at: optStr(r.at), event: r.event, family: r.family, name: str(r.name), parent: optStr(r.parent), generation: num(r.generation),
    operator: str(r.operator), lessons: strs(r.lessons), trials: num(r.trials), verdict: optStr(r.verdict), study: str(r.study),
    key: str(r.key), note: str(r.note),
  };
}

function pairing(r: Record<string, unknown>): Pairing {
  return {
    challenger: str(r.challenger), incumbent: str(r.incumbent), incumbent_spec: str(r.incumbent_spec), days: num(r.days),
    challenger_bps: num(r.challenger_bps), incumbent_bps: num(r.incumbent_bps), diff_bps: num(r.diff_bps), lo_bps: num(r.lo_bps),
    hi_bps: num(r.hi_bps), corr: num(r.corr), duplicate: r.duplicate === true,
  };
}

function cycleReport(file: string, r: Record<string, unknown>): CycleReport | null {
  if (typeof r.generation !== "number") return null;
  const studies: StudyCost[] = arr(r.studies).map((c) => ({
    study: str(c.study), trials_before: num(c.trials_before), trials_after: num(c.trials_after), design_days: num(c.design_days),
    luck_bar_before: num(c.luck_bar_before, NaN), luck_bar_after: num(c.luck_bar_after, NaN),
  }));
  const proposals: ProposalRow[] = arr(r.proposals).map((p) => ({
    name: str(p.name), family: str(p.family), operator: str(p.operator), parent: optStr(p.parent), lessons: strs(p.lessons),
    reason: str(p.reason), rule: str(p.rule), specs: num(p.specs), new_trials: num(p.new_trials), approximate: p.approximate === true,
    verdict: optStr(p.verdict), failed: strs(p.failed), pick: optStr(p.pick),
  }));
  return {
    file, generation: r.generation, dry_run: r.dry_run === true || file.startsWith("dry-"), at: optStr(r.at),
    lessons_file: str(r.lessons_file), lessons_total: num(r.lessons_total), lessons_usable: num(r.lessons_usable), studies, proposals,
    pairings: arr(r.pairings).map(pairing), forward: arr(r.forward).map(pairing), messages: strs(r.messages),
    promotable: strs(r.promotable), lockbox_openings: num(r.lockbox_openings),
  };
}

function cycleReports(): CycleReport[] {
  const dir = path.join(LAB, "reports", "evolve");
  let files: string[] = [];
  try {
    // The dry-gNNN/ folders beside them hold would-be family files, not reports.
    files = fs.readdirSync(dir).filter((f) => /^(dry-)?g\d+\.json$/.test(f));
  } catch {
    /* the loop never ran */
  }
  const out: CycleReport[] = [];
  for (const f of files) {
    const d = readJson<Record<string, unknown>>(path.join(dir, f));
    const r = d && typeof d === "object" ? cycleReport(f, d) : null;
    if (r) out.push(r);
  }
  const t = (r: CycleReport) => (r.at ? Date.parse(r.at) || 0 : 0);
  return out.sort((a, b) => t(b) - t(a) || b.generation - a.generation || Number(a.dry_run) - Number(b.dry_run));
}

/** A `[name]` table of `key = number` lines. */
function tomlTable(t: string, name: string): Record<string, number> {
  const out: Record<string, number> = {};
  const at = t.indexOf(`[${name}]`);
  if (at < 0) return out;
  for (const line of t.slice(at).split("\n").slice(1)) {
    if (line.trim().startsWith("[")) break;
    const m = line.match(/^\s*"?([\w.-]+)"?\s*=\s*([\d.]+)/);
    if (m) out[m[1]] = Number(m[2]);
  }
  return out;
}

/** The `[evolve]` caps in `lab.toml`: what one cycle may try and when `--auto` stops. */
function evolveCfg(): EvolveCfg {
  const t = readText(path.join(ROOT, "lab.toml")) ?? "";
  return {
    proposals: tomlNumber(t, "proposals", 4),
    specs_per_proposal: tomlNumber(t, "specs_per_proposal", 12),
    trials_per_cycle: tomlNumber(t, "trials_per_cycle", 40),
    auto_trial_budget: tomlNumber(t, "auto_trial_budget", 200),
    auto_trial_budgets: tomlTable(t, "evolve.auto_trial_budgets"),
    auto_trials_per_run: /^auto_trials_per_run\s*=/m.test(t) ? tomlNumber(t, "auto_trials_per_run", 0) : null,
    auto_stall: tomlNumber(t, "auto_stall", 3),
    auto_max_generations: tomlNumber(t, "auto_max_generations", 20),
  };
}

/** Every `lab-data/reports/evolve/auto-<time>.json`, oldest first. */
function autoRuns(): AutoRun[] {
  const dir = path.join(LAB, "reports", "evolve");
  let files: string[] = [];
  try {
    files = fs.readdirSync(dir).filter((f) => /^auto-.*\.json$/.test(f)).sort();
  } catch {
    return [];
  }
  const out: AutoRun[] = [];
  for (const f of files) {
    const d = readJson<Record<string, unknown>>(path.join(dir, f));
    if (!d || typeof d !== "object") continue;
    const loop = d.loop_trials && typeof d.loop_trials === "object" ? (d.loop_trials as Record<string, unknown>) : {};
    out.push({
      started: optStr(d.started),
      finished: optStr(d.finished),
      stop: str(d.stop),
      generations: arr(d.generations).map((g) => ({
        generation: num(g.generation),
        proposals: arr(g.proposals).length,
        verdicts: arr(g.proposals).map((p) => str(p.verdict, "untested")),
        trials: arr(g.studies).reduce((n, c) => n + Math.max(0, num(c.trials_after) - num(c.trials_before)), 0),
      })),
      loop_trials: Object.entries(loop).map(([study, n]) => ({ study, n: num(n) })).sort((a, b) => a.study.localeCompare(b.study)),
      budget: num(d.budget, 200),
    });
  }
  return out;
}

const EVOLVE_SCRIPT = "v3-evolve-hourly.sh";

/** The crontab line that runs the hourly script, and the ticks it wrote to `data-v3/evolve.log`. */
function evolveSchedule(): EvolveSchedule {
  let cron: string | null = null;
  try {
    const tab = execFileSync("crontab", ["-l"], { encoding: "utf8", timeout: 2000, stdio: ["ignore", "pipe", "ignore"] });
    const line = tab.split("\n").find((l) => !l.trim().startsWith("#") && l.includes(EVOLVE_SCRIPT));
    if (line) cron = line.trim().split(/\s+/).slice(0, 5).join(" ");
  } catch {
    /* no crontab, or no cron on this machine */
  }
  const m = cron?.match(/^(\d{1,2}) \* \* \* \*$/);
  const ticks: EvolveSchedule["ticks"] = [];
  for (const l of (readText(path.join(DATA, "evolve.log")) ?? "").split("\n")) {
    const start = l.match(/^=== (\S+Z) evolve tick/);
    if (start) ticks.push({ at: start[1], done: null, exit: null, skipped: false });
    const done = l.match(/^=== (\S+Z) done \(exit (\d+)\)/);
    if (done && ticks.length) Object.assign(ticks[ticks.length - 1], { done: done[1], exit: Number(done[2]) });
    const skip = l.match(/^(\S+Z) previous tick still running/);
    if (skip) ticks.push({ at: skip[1], done: skip[1], exit: 0, skipped: true });
  }
  let next: string | null = null;
  if (m) {
    const d = new Date();
    d.setUTCSeconds(0, 0);
    d.setUTCMinutes(Number(m[1]));
    if (d.getTime() <= Date.now()) d.setUTCHours(d.getUTCHours() + 1);
    next = d.toISOString();
  }
  return { cron, everyMinutes: m ? 60 : null, minute: m ? Number(m[1]) : null, next, ticks };
}

let gensCached: { at: number; gens: Generations } | null = null;

/** The lineage as a tree, the same reading as `lab lineage`, plus every cycle report. */
export function getGenerations(): Generations {
  const now = Date.now();
  if (gensCached && now - gensCached.at < 5000) return gensCached.gens;
  const events = readJsonl<Record<string, unknown>>(path.join(LAB, "lineage.jsonl"))
    .map((r) => (r && typeof r === "object" ? lineageEvent(r) : null))
    .filter((e): e is LineageEvent => e !== null);
  const last = new Map<string, LineageEvent>();
  const failed = new Map<string, string>();
  const tested = new Map<string, string>();
  const testedAt = new Map<string, string | null>();
  const charged = new Map<string, number>();
  const loopByStudy = new Map<string, number>();
  const era = eraStart();
  const studiesSeen = new Set<string>();
  let eraTrials = 0;
  for (const e of events) {
    last.set(e.family, e);
    // `retested`: judged again on a longer archive (`lab rejudge`); its verdict replaces the first.
    if (e.event === "tested" || e.event === "retested") {
      failed.set(e.family, e.note);
      if (e.verdict) tested.set(e.family, e.verdict);
      testedAt.set(e.family, e.at);
    }
    charged.set(e.family, (charged.get(e.family) ?? 0) + e.trials);
    if (e.study) studiesSeen.add(e.study);
    // Spend counts against the budget and the luck bar only in the ledger's current era.
    if (e.trials > 0 && inEra(e.at, era)) {
      loopByStudy.set(e.study, (loopByStudy.get(e.study) ?? 0) + e.trials);
      eraTrials += e.trials;
    }
  }
  // One proposal per rule id. The loop has proposed the same rule twice under two names (g0034 and g0043);
  // kept twice, the rule and every descendant appear twice in the tree and in every count. The last name
  // wins, as in getLab, so both pages call it the same thing.
  const lastProposal = new Map<string, LineageEvent>();
  for (const e of events) if (e.event === "proposed") lastProposal.set(e.family, e);
  const proposed = [...lastProposal.values()];
  const names: Record<string, string> = {};
  for (const p of proposed) if (p.name) names[p.family] = p.name;
  const known = new Set(proposed.map((p) => p.family));
  const kids = new Map<string, LineageEvent[]>();
  for (const p of proposed) {
    const k = p.parent ?? "";
    kids.set(k, [...(kids.get(k) ?? []), p]);
  }
  const state = (family: string): string => {
    const e = last.get(family);
    if (!e || e.event === "proposed") return "proposed";
    if (e.event === "tested" || e.event === "retested") return e.verdict ?? "proposed";
    return e.event.toUpperCase();
  };
  const build = (parent: string, depth: number, seen: Set<string>): GenNode[] =>
    (kids.get(parent) ?? []).filter((p) => !seen.has(p.family)).map((p) => {
      const s = state(p.family);
      return {
        family: p.family, name: p.name || p.family, generation: p.generation, operator: p.operator, study: p.study, lessons: p.lessons,
        state: s, failed: failed.get(p.family) ?? "", trials: charged.get(p.family) ?? 0,
        prior: testedAt.has(p.family) && !inEra(testedAt.get(p.family), era),
        children: depth < 32 ? build(p.family, depth + 1, new Set([...seen, p.family])) : [],
      };
    });
  const roots: GenRoot[] = [...kids.keys()].filter((k) => !known.has(k)).sort().map((family) => ({ family, children: build(family, 1, new Set()) }));
  const verdicts: Record<string, number> = {};
  for (const v of tested.values()) verdicts[v] = (verdicts[v] ?? 0) + 1;
  const lab = getLab();
  const ledger = new Map(lab.trialsByStudy.map((s) => [s.study, s.n]));
  const studies = new Set([...studiesSeen, ...loopByStudy.keys()]);
  const runs = autoRuns();
  const near: NearMiss[] = [...lab.families, ...lab.generated]
    .filter((f) => !f.prior)
    .map((f) => ({ f, failed: f.checks.filter((c) => !c.pass) }))
    .filter(({ failed }) => failed.length >= 1 && failed.length <= 2)
    .map(({ f, failed }) => ({ name: f.name ?? familyName(f.family), family: f.family, generation: f.generation ?? 0, study: f.study, judged: f.judged, trials_ledger: f.trials_ledger, failed }))
    .sort((a, b) => a.failed.length - b.failed.length || Date.parse(b.judged) - Date.parse(a.judged));
  const gens: Generations = {
    roots,
    near,
    handWritten: lab.families.length,
    proposed: proposed.length,
    verdicts,
    generations: events.reduce((m, e) => Math.max(m, e.generation), 0),
    trials: eraTrials,
    era: era === null ? null : new Date(era).toISOString(),
    byStudy: [...studies].sort().map((study) => ({ study, loop: loopByStudy.get(study) ?? 0, ledger: ledger.get(study) ?? 0 })),
    names,
    reports: cycleReports(),
    auto: runs[runs.length - 1] ?? null,
    runs,
    schedule: evolveSchedule(),
    cfg: evolveCfg(),
  };
  gensCached = { at: now, gens };
  return gens;
}

// ---- the rules page ----

let rulesCached: { at: number; rules: Rules } | null = null;

/** Every rule on the trial ledger, joined to its family's name, origin, verdict and distance to the goal. */
export function getRules(): Rules {
  const now = Date.now();
  if (rulesCached && now - rulesCached.at < 5000) return rulesCached.rules;
  const lab = getLab();
  const statements = new Map(lab.hypotheses.map((h) => [h.id, h.statement]));
  const families: Record<string, RuleFamily> = {};
  const picks = new Set<string>();
  const foldCount = new Map<string, number>();
  const walkRaw = new Map<string, Map<string, WalkCell>>();
  const windowEnds = new Map<string, string>();
  const walkNone = new Set<string>();
  let lockbox: string | null = null;
  for (const f of [...lab.families, ...lab.generated]) {
    const key = `${f.study}|${f.family}`;
    const unjudged = f.checks.length === 0 || (f.verdict === "INSUFFICIENT_EVIDENCE" && f.checks.every((c) => c.name === "evidence" || c.pass));
    families[key] = {
      key, family: f.family, study: f.study, name: f.name ?? familyName(f.family), generation: f.generation ?? null, verdict: f.verdict,
      short: unjudged ? null : f.checks.filter((c) => !c.pass).length, statement: statements.get(f.hypothesis) ?? "", specs: f.specs,
    };
    if (f.leaders[0]) picks.add(`${f.study}|${f.leaders[0].spec}`);
  }
  // Walk-forward picks per fold live only in the raw reports.
  for (const study of lab.trialsByStudy.map((s) => s.study)) {
    let files: string[] = [];
    try {
      files = fs.readdirSync(path.join(LAB, "reports", study)).filter((x) => x.endsWith(".json"));
    } catch {
      continue;
    }
    for (const file of files) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const d = readJson<any>(path.join(LAB, "reports", study, file));
      const picked = strs(d?.fold_picks);
      for (const spec of picked) foldCount.set(`${study}|${spec}`, (foldCount.get(`${study}|${spec}`) ?? 0) + 1);
      if (!d || !Array.isArray(d.checks) || typeof d.family !== "string") continue;
      const key = `${study}|${d.family}`;
      const folds: { test?: { start?: number; end?: number }; test_mean?: number; test_sharpe?: number }[] = Array.isArray(d.walk?.folds) ? d.walk.folds : [];
      const panelStart = String(d.panel ?? "").match(/(\d{4}-\d{2}-\d{2})\s*→/)?.[1];
      if (!panelStart || folds.length === 0 || folds.length !== picked.length) {
        walkNone.add(key);
        continue;
      }
      const day = (n: number) => new Date(Date.parse(`${panelStart}T00:00:00Z`) + n * 86_400_000).toISOString().slice(0, 10);
      const byStart = new Map<string, WalkCell>();
      folds.forEach((f, i) => {
        if (typeof f.test?.start !== "number" || typeof f.test?.end !== "number") return;
        const start = day(f.test.start);
        windowEnds.set(start, day(f.test.end));
        byStart.set(start, { spec: picked[i], bps: num(f.test_mean) * 1e4, sharpe: num(f.test_sharpe) * Math.sqrt(365) });
      });
      walkRaw.set(key, byStart);
      if (typeof d.design_days === "number") {
        const lb = day(d.design_days);
        if (!lockbox || lb < lockbox) lockbox = lb;
      }
    }
  }
  // Every family on one calendar: windows line up by date, not by fold number, and a window a fold
  // left out (it picked nothing) is flat for that family.
  const windows = [...windowEnds.entries()].sort(([a], [b]) => a.localeCompare(b)).map(([start, end]) => ({ start, end }));
  const walkFamilies: WalkFamily[] = [...walkRaw.entries()].map(([key, byStart]) => {
    const cells = windows.map((w) => byStart.get(w.start) ?? null);
    return { key, cells, mean: cells.reduce((a, c) => a + (c?.bps ?? 0), 0) / Math.max(1, windows.length) };
  });
  walkFamilies.sort((a, b) => b.mean - a.mean);
  const rows: RuleRow[] = [];
  for (const r of readJsonl<Record<string, unknown>>(path.join(LAB, "trials.jsonl"))) {
    if (!r || typeof r !== "object") continue;
    const id = str(r.spec);
    const study = str(r.study);
    const family = str(r.family) || id.split(":")[0];
    const params: Record<string, number> = {};
    for (const kv of (id.split(":")[1] ?? "").split(",")) {
      const [k, v] = kv.split("=");
      if (k && v !== undefined && Number.isFinite(Number(v))) params[k] = Number(v);
    }
    const key = `${study}|${family}`;
    if (!families[key]) {
      families[key] = { key, family, study, name: familyName(family), generation: null, verdict: null, short: null, statement: "", specs: 0 };
    }
    rows.push({
      id, study, family, params, trades: num(r.trades), days: num(r.days), sharpe: num(r.sharpe_daily) * Math.sqrt(365), bps: num(r.mean_bps_day),
      at: str(r.at), pick: picks.has(`${study}|${id}`), folds: foldCount.get(`${study}|${id}`) ?? 0,
    });
  }
  const rules: Rules = { rows, families, walk: { windows, lockbox, families: walkFamilies.filter((w) => families[w.key]), none: [...walkNone].filter((k) => families[k]) } };
  rulesCached = { at: now, rules };
  return rules;
}

/** Drop the 5-second caches, so the next read sees a file just written (after a promotion). */
export function forgetCaches() {
  labCached = null;
  gensCached = null;
  rulesCached = null;
}
