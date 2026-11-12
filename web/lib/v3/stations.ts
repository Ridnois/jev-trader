import type { Live, ProbeQuestion, StrategyView } from "./types";
import { strategyLabel } from "@/lib/format";

export const START_CMD = "./target/lab/jev-trader --env-file ../trading/.env.local trade run";

export type EngineLevel = "running" | "degraded" | "stopped" | "never";
export type EngineState = { level: EngineLevel; label: string; detail: string };

/** Alive, stale or gone, said once, with the consequence for the figures on screen. */
export function engineState(live: Live): EngineState {
  const { status, alive, stale, ageSec } = live.engine;
  if (!status) return { level: "never", label: "No engine data", detail: "data-v3/status.json does not exist yet. Nothing has run." };
  if (!alive) return { level: "stopped", label: "Engine stopped", detail: `Process ${status.pid} is gone. Every figure below is from its last write.` };
  if (stale) return { level: "degraded", label: "Engine silent", detail: `Running, but no status written for ${Math.round(ageSec ?? 0)} s. Treat the figures as old.` };
  if (!status.feed.connected) return { level: "degraded", label: "Feed down", detail: status.feed.last_error ?? "The Hyperliquid stream is disconnected. Prices are stale." };
  return { level: "running", label: "Engine running", detail: `${status.quotes} of ${status.coins} coins quoting.` };
}

/** AUC and the shuffle test, the one place this pair is defined. */
export function showsSkill(q: Pick<ProbeQuestion, "auc" | "ic_null_pct">): boolean {
  return q.auc > 0.55 && q.ic_null_pct > 95;
}

export type Tone = "neutral" | "active" | "good" | "warn" | "bad" | "info";

export type Station = {
  id: "lab" | "paper" | "news" | "jev";
  /** Full name on the Now page and the sidebar. */
  name: string;
  /** Three or four letters for the stage bar. */
  short: string;
  /** The question this station answers. */
  question: string;
  /** What it does, in one line. */
  role: string;
  href: string;
  /** The state, four words at most. */
  state: string;
  /** One sentence with the numbers behind the state. */
  detail: string;
  tone: Tone;
  /** Freshness or coverage line: when it last ran, what it covers. */
  meta: string;
};

function fullDays(live: Live): number {
  return live.strategies.reduce((n, s) => n + s.fullDays, 0);
}

function labStation(live: Live): Station {
  const { families: n, promoted, lastJudged: last, lockboxSealed, lockboxDays } = live.lab;
  if (n === 0)
    return {
      id: "lab", name: "Lab", short: "Lab", question: "Is the rule real?", href: "/lab", tone: "neutral",
      role: "Judges a family of rules on old days before any money is involved.",
      state: "Not run", detail: "No verdicts on file, so nothing has been judged yet.",
      meta: "Never judged",
    };
  return {
    id: "lab", name: "Lab", short: "Lab", question: "Is the rule real?", href: "/lab", tone: promoted > 0 ? "good" : "neutral",
    role: "Judges a family of rules on old days before any money is involved.",
    state: `${promoted} of ${n} families passed`,
    detail: promoted === 0
      ? `All ${n} families were rejected or not enough evidence was collected. That is the result, not a failure to report.`
      : `${promoted} of ${n} families passed the gate; the rest did not.`,
    meta: `${last ? `Last judged ${new Date(last).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" })}` : "Never judged"} · ${lockboxSealed ? `Lockbox: ${lockboxDays} days held back` : "Lockbox: opened"}`,
  };
}

function paperStation(live: Live): Station {
  const ss = live.strategies;
  const days = fullDays(live);
  const kill = ss.filter((s) => s.call === "kill");
  const graduate = ss.filter((s) => s.call === "graduate");
  const base = {
    id: "paper" as const, name: "Paper run", short: "Paper", question: "Does the live market agree?", href: "/paper",
    role: "Runs a rule against live prices with paper money, promoted or unproven and labelled, and decides continue, kill or graduate.",
  };
  if (ss.length === 0)
    return {
      ...base, tone: "neutral", state: "Nothing on paper",
      detail: "No strategy has been promoted, so the paper book is empty. The Lab is the step before this one.",
      meta: live.engine.started ? `Book started ${new Date(live.engine.started).toLocaleDateString("en-GB", { day: "numeric", month: "short" })}` : "Book never started",
    };
  if (kill.length)
    return {
      ...base, tone: "bad", state: `Kill: ${strategyLabel(kill[0].name, kill[0].card)}`,
      detail: kill[0].callWhy, meta: `${days} full paper ${days === 1 ? "day" : "days"} so far`,
    };
  if (graduate.length)
    return {
      ...base, tone: "good", state: "Passed the paper test",
      detail: `${strategyLabel(graduate[0].name, graduate[0].card)} reached the graduate line. Going live is still a human decision, made in the terminal.`,
      meta: `${days} full paper ${days === 1 ? "day" : "days"} so far`,
    };
  const first = ss[0];
  return {
    ...base, tone: "active", state: `${ss.length} ${ss.length === 1 ? "strategy" : "strategies"} on paper, ${days} full ${days === 1 ? "paper day" : "paper days"}`,
    detail: days === 0
      ? "No full paper day yet. A full paper day is a UTC day with the engine up at least 90% of it; the first point lands after one."
      : `The test stands at ${first.llr.toFixed(2)} log-likelihood ratio; it kills below ${first.kill.toFixed(2)} and graduates above ${first.graduate.toFixed(2)}.`,
    meta: live.engine.started ? `Running since ${new Date(live.engine.started).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" })}` : "Not started",
  };
}

function newsStation(live: Live): Station {
  const waiting = live.engine.status?.catalyst.candidates_waiting_for_markouts ?? 0;
  const days = live.markoutDays;
  const base = {
    id: "news" as const, name: "News recorder", short: "News", question: "Is there money in the news?", href: "/catalysts",
    role: "Scores headlines with Jev and measures what the price did afterwards. It never trades.",
  };
  if (live.candidateCount === 0)
    return {
      ...base, tone: "neutral", state: "No headlines scored yet",
      detail: "No headline has been read yet. The recorder starts when the engine starts and the feeds answer.",
      meta: "No evidence yet",
    };
  const evidence = days >= 20 && live.triggeredCount >= 30
    ? `${days} evidence days, ${live.triggeredCount} triggered headlines`
    : `${days} of 20 evidence days, ${live.triggeredCount} of 30 triggered headlines`;
  return {
    ...base, tone: "active", state: evidence,
    detail: `${live.candidateCount.toLocaleString("en-US")} headlines scored, ${live.triggeredCount} triggered, ${waiting} headlines awaiting their 6 h price reading. A first read needs 20 evidence days and 30 triggered headlines.`,
    meta: live.lastScoredAt ? `Last headline ${new Date(live.lastScoredAt).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit", timeZone: "UTC" })} UTC` : "No headline yet",
  };
}

function jevStation(live: Live): Station {
  const { questions, acceptedQuestions: accepted, lastScored: last } = live.lab;
  const base = {
    id: "jev" as const, name: "Jev tests", short: "Jev", question: "Can Jev read a market?", href: "/probes",
    role: "Asks Jev fixed questions about hidden market states and scores the answers against what happened next.",
  };
  if (questions === 0)
    return {
      ...base, tone: "neutral", state: "No questions tested yet",
      detail: "No probe has been scored, so nothing is known about Jev's read.",
      meta: "No scores on file",
    };
  return {
    ...base, tone: accepted > 0 ? "good" : "neutral", state: `${accepted} of ${questions} questions show skill`,
    detail: accepted === 0
      ? `None of ${questions} questions beats chance by the bar. Nothing is built on Jev's read yet.`
      : `${accepted} of ${questions} questions clear the bar. The next step would be a rule that trades on it, judged by the same gate.`,
    meta: last ? `Last scored ${new Date(last).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" })}` : "No scores on file",
  };
}

export function stations(live: Live): Station[] {
  return [labStation(live), paperStation(live), newsStation(live), jevStation(live)];
}

export type Notice = { kind: "calm" | "warn" | "bad" | "good"; title: string; detail: string; href?: string; link?: string };

/** The single thing the reader should know, in the order that matters. */
export function notice(live: Live): Notice {
  const e = engineState(live);
  if (e.level === "never")
    return {
      kind: "warn", title: "Nothing has run yet.", href: "/start", link: "Read how this works",
      detail: `The dashboard is a window onto files the engine writes. Start the paper engine from the v3 worktree: ${START_CMD}`,
    };
  if (e.level === "stopped")
    return {
      kind: "bad", title: "The engine is stopped.", href: "/start", link: "See the lifecycle",
      detail: `${e.detail} Restart it from the v3 worktree: ${START_CMD}`,
    };
  if (e.level === "degraded")
    return { kind: "warn", title: e.label + ".", detail: e.detail, href: "/markets", link: "Check the quotes" };

  const kill = live.strategies.find((s) => s.call === "kill");
  if (kill)
    return {
      kind: "bad", title: "A paper strategy hit its kill line.",
      detail: `${strategyLabel(kill.name, kill.card)}: ${kill.callWhy} The paper run is the last line of defence; nothing real was at risk.`,
      href: "/paper", link: "Read the paper run",
    };
  const graduate = live.strategies.find((s) => s.call === "graduate");
  if (graduate)
    return {
      kind: "good", title: "A paper strategy passed its test.",
      detail: `${strategyLabel(graduate.name, graduate.card)} reached the graduate line. Arming it is your decision, made in the terminal, and the gate's evidence is the place to start.`,
      href: "/paper", link: "Read the paper run",
    };

  const days = fullDays(live);
  const shortDays = Math.max(0, 20 - live.markoutDays);
  const shortTriggers = Math.max(0, 30 - live.triggeredCount);
  const newsNeed = shortDays === 0 && shortTriggers === 0
    ? "The news recorder has reached its first read"
    : `The news recorder needs ${shortDays} more evidence ${shortDays === 1 ? "day" : "days"} and ${shortTriggers} more triggered ${shortTriggers === 1 ? "headline" : "headlines"}`;
  const jevNeed = live.lab.questions ? `${live.lab.acceptedQuestions} of ${live.lab.questions} Jev questions show skill` : "Jev has not been tested";
  return {
    kind: "calm", title: "Engine running. Nothing needs you.",
    detail: live.strategies.length === 0
      ? `The Lab is the next step: a family has to pass the gate before it can run on paper. ${newsNeed}; ${jevNeed}.`
      : days === 0
        ? `The paper run has no full day yet, so it cannot decide anything. The next UTC day with the engine up 90% of it produces the first point. ${newsNeed}; ${jevNeed}.`
        : `The paper run is at ${days} full ${days === 1 ? "day" : "days"}. ${newsNeed}; ${jevNeed}.`,
    href: "/start", link: "How this works",
  };
}

/** The two-line readout at the top of Now: the verdict, then what it means. */
export function summary(live: Live): { line: string; note: string; kind: Notice["kind"] } {
  const [lab, paper] = stations(live);
  const n = notice(live);
  return { line: `${lab.state} · ${paper.state}`, note: `${n.title} ${n.detail}`, kind: n.kind };
}

/** The one-line state of the whole system, for the top of Now. */
export function headline(live: Live): string {
  const e = engineState(live);
  if (e.level !== "running") return `${e.label}.`;
  const days = fullDays(live);
  const n = live.strategies.length;
  if (n === 0) return "Running, with nothing on paper.";
  return `Running: ${n} ${n === 1 ? "rule" : "rules"} on paper, ${days} full ${days === 1 ? "day" : "days"}.`;
}

export function totals(live: Live) {
  const books = live.engine.status?.books ?? [];
  return {
    capital: books.reduce((a, b) => a + b.capital, 0),
    change: books.reduce((a, b) => a + b.equity_change, 0),
    atRisk: books.reduce((a, b) => a + b.at_risk, 0),
    positions: books.flatMap((b) => b.positions.map((p) => ({ ...p, strategy: b.strategy }))),
  };
}

export function strategyStatus(s: StrategyView): { label: string; tone: Tone } {
  if (s.status === "validated") return { label: "Validated by the lab", tone: "good" };
  return { label: "Unproven candidate", tone: "warn" };
}
