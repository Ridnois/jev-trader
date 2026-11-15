/**
 * A strategy's entry rule in words, with its spec's settings filled in.
 * Mirrors `src/lab/compose.rs`: the same features, the same mirror for shorts,
 * and the same composed form of the hand-written `trend` family (`template`).
 */

type Num = number | string;
export type Cond = { feature: string; n: Num; op: string; level: Num };
export type ComposedRule = {
  long: Cond[];
  short?: string;
  filter?: Cond[];
  exit?: Cond[];
  exit_opposite?: boolean;
  every?: Num | null;
};

const UNSIGNED = new Set(["vol_ratio", "rvol", "mkt_rvol_pct"]);
const OP: Record<string, string> = { ">=": "≥", "<=": "≤", ">": ">", "<": "<" };
const FLIP: Record<string, string> = { ">=": "<=", "<=": ">=", ">": "<", "<": ">" };

/** `$n`, `$n*$out_frac`, `$z*-1` or a number, against the spec's params. */
function value(p: Num, params: Record<string, number>): number {
  if (typeof p === "number") return p;
  return p.split("*").reduce((acc, f) => {
    const t = f.trim();
    const x = t.startsWith("$") ? params[t.slice(1)] : Number(t);
    return acc * (Number.isFinite(x) ? x : NaN);
  }, 1);
}

/** Bars as time: 12 → 12h, 96 → 4d (1h bars); 15m bars as hours or days. */
function bars(n: number, interval: string): string {
  const mins = interval === "15m" ? 15 : interval === "1h" ? 60 : interval === "4h" ? 240 : interval === "1d" ? 1440 : 0;
  if (!mins || !Number.isFinite(n)) return `${n}-bar`;
  const h = (n * mins) / 60;
  if (h >= 24 && h % 24 === 0) return `${h / 24}d`;
  if (h >= 48) return `${Math.round(h / 24)}d`;
  return `${Math.round(h * 10) / 10}h`;
}

const num = (x: number) => (Number.isInteger(x) ? String(x) : String(Number(x.toFixed(3))));
const pct = (x: number) => `${num(x * 100)}%`;
const minus = (s: string) => s.replace(/^-/, "−");

/** A cross-sectional rank in [−1, 1] as a share of the coins. */
function rankWords(op: string, level: number): string {
  const top = Math.round(((1 - level) / 2) * 100);
  const bottom = Math.round(((1 + level) / 2) * 100);
  if (op === ">=" || op === ">") return `top ${top}% of coins`;
  return `bottom ${bottom}% of coins`;
}

function condWords(c: Cond, params: Record<string, number>, interval: string): string {
  const n = bars(value(c.n, params), interval);
  const lv = value(c.level, params);
  const op = OP[c.op] ?? c.op;
  switch (c.feature) {
    case "channel":
      if ((c.op === ">" || c.op === ">=") && lv >= 1) return `it closes above the ${n} high`;
      if ((c.op === "<" || c.op === "<=") && lv <= -1) return `it closes below the ${n} low`;
      return `${n} channel position ${op} ${minus(num(lv))}`;
    case "vol_ratio":
      return `volume ${op} ${num(lv)}× its ${n} average`;
    case "xs_ret":
      return `${n} return in the ${rankWords(c.op, lv)}`;
    case "xs_funding":
      return `${n} funding in the ${rankWords(c.op, lv)}`;
    case "ret":
      return `${n} return ${op} ${minus(pct(lv))}`;
    case "mkt_ret":
      return `market ${n} return ${op} ${minus(pct(lv))}`;
    case "z":
      return `price ${op} ${minus(num(lv))}σ from its ${n} mean`;
    case "resid_z":
      return `price vs the basket ${op} ${minus(num(lv))}σ from its ${n} mean`;
    case "rvol":
      return `${n} volatility ${op} ${minus(num(lv))}`;
    case "mkt_rvol_pct":
      return `market ${n} volatility in its ${c.op.startsWith(">") ? "top" : "bottom"} ${c.op.startsWith(">") ? num((1 - lv) * 100) : num(lv * 100)}% of 30 days`;
    case "funding":
      return `${n} funding ${op} ${minus(num(lv))}`;
    default:
      return `${c.feature}(${n}) ${op} ${minus(num(lv))}`;
  }
}

/** The condition seen from the short side (`Cond::inverted`). */
function inverted(c: Cond): Cond {
  if (UNSIGNED.has(c.feature)) return c;
  const level = typeof c.level === "number" ? -c.level : c.level.endsWith("*-1") ? c.level.slice(0, -3) : `${c.level}*-1`;
  return { ...c, op: FLIP[c.op] ?? c.op, level };
}

function joinAll(cs: Cond[], params: Record<string, number>, interval: string, sep = " and "): string {
  const seen = new Set<string>();
  const words: string[] = [];
  for (const c of cs) {
    const w = condWords(c, params, interval);
    if (!seen.has(w)) words.push(w);
    seen.add(w);
  }
  return words.join(sep);
}

/** The composed form of a hand-written family, where it is exact (only `trend`). */
function template(family: string, params: Record<string, number>): ComposedRule | null {
  if (family !== "trend") return null;
  return {
    long: [{ feature: "channel", n: "$n", op: ">", level: 1 }],
    short: params.shorts ? "mirror" : "none",
    exit: [{ feature: "channel", n: "$n*$out_frac", op: "<", level: -1 }],
  };
}

/**
 * One sentence: when it buys, when it sells short, when it leaves, and the stop.
 * Null when the family has no composed form to read from.
 */
export function ruleSentence(
  rule: ComposedRule | null | undefined,
  card: { spec: { family: string; params?: Record<string, number> }; interval: string },
): string | null {
  const params = card.spec.params ?? {};
  const r = rule ?? template(card.spec.family, params);
  if (!r || !r.long?.length) return null;
  const iv = card.interval;
  const mirror = r.short === "mirror" || r.short === "Mirror";
  const parts: string[] = [];
  parts.push(`Long when ${joinAll(r.long, params, iv)}`);
  if (mirror) parts.push(`short when ${joinAll(r.long.map(inverted), params, iv)}`);
  else parts.push("long only");
  if (r.filter?.length) parts.push(`only when ${joinAll(r.filter, params, iv)}`);
  if (r.exit?.length) {
    const long = joinAll(r.exit, params, iv, " or ");
    parts.push(mirror ? `exit a long when ${long}, a short when ${joinAll(r.exit.map(inverted), params, iv, " or ")}` : `exit when ${long}`);
  }
  if (r.exit_opposite) parts.push("exit on the opposite signal");
  if (r.every != null) parts.push(`entries every ${bars(value(r.every, params), iv)}`);
  if (typeof params.stop === "number" && params.stop > 0) parts.push(`${num(params.stop)} bps stop`);
  return `${parts.join("; ")}.`;
}

/** The entry clause on each side, for one order: what made it buy or sell short. */
export function entryClauses(
  rule: ComposedRule | null | undefined,
  card: { spec: { family: string; params?: Record<string, number> }; interval: string },
): { long: string; short: string | null } | null {
  const params = card.spec.params ?? {};
  const r = rule ?? template(card.spec.family, params);
  if (!r || !r.long?.length) return null;
  const mirror = r.short === "mirror" || r.short === "Mirror";
  const only = r.filter?.length ? `, only when ${joinAll(r.filter, params, card.interval)}` : "";
  return {
    long: `Long when ${joinAll(r.long, params, card.interval)}${only}`,
    short: mirror ? `Short when ${joinAll(r.long.map(inverted), params, card.interval)}${only}` : null,
  };
}
