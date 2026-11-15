const MINUS = "−";

/** A signed number with a real minus sign; an exact zero carries no sign. */
export function signed(x: number, digits = 2, unit = ""): string {
  const r = Number(x.toFixed(digits));
  const body = Math.abs(r).toLocaleString("en-US", { minimumFractionDigits: digits, maximumFractionDigits: digits });
  return `${r > 0 ? "+" : r < 0 ? MINUS : ""}${body}${unit}`;
}

export function usd(x: number, digits = 0): string {
  return `${x < 0 ? MINUS : ""}$${Math.abs(x).toLocaleString("en-US", { minimumFractionDigits: digits, maximumFractionDigits: digits })}`;
}

/** Prices keep about five significant figures whatever the coin costs. */
export function price(x: number): string {
  const d = x >= 10_000 ? 1 : x >= 100 ? 2 : x >= 1 ? 4 : x >= 0.01 ? 5 : 7;
  return x.toLocaleString("en-US", { minimumFractionDigits: d, maximumFractionDigits: d });
}

export function duration(secs: number): string {
  if (secs < 60) return `${Math.floor(secs)}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  if (secs < 86_400) return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`;
  return `${Math.floor(secs / 86_400)}d ${Math.floor((secs % 86_400) / 3600)}h`;
}

export function ago(iso: string | number, now: number): string {
  const t = typeof iso === "number" ? iso : Date.parse(iso);
  return `${duration(Math.max(0, (now - t) / 1000))} ago`;
}

export function clock(iso: string | number): string {
  return new Date(iso).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit", timeZone: "UTC" });
}

export function horizon(secs: number): string {
  return secs < 3600 ? `${secs / 60} min` : `${secs / 3600} h`;
}

const FAMILY_NAMES: Record<string, string> = {
  meanrev: "Mean reversion",
  trend: "Trend",
  xs_reversal: "Cross-section",
  funding_carry: "Funding carry",
  jev_gate: "Jev-gated fade",
  jev_direction: "Jev direction",
  hvt: "Volume-weighted",
  ensemble: "Ensemble",
};

export function familyName(id: string): string {
  return FAMILY_NAMES[id] ?? id;
}

/**
 * A strategy's screen name. The file name and the spec id can differ, and two
 * strategies can share a family, so the lookback is what tells them apart.
 */
export function strategyLabel(name: string, card: { spec: { family: string; params?: Record<string, number> }; interval: string }): string {
  const family = familyName(card.spec.family);
  const n = card.spec.params?.n;
  const barsPerDay = card.interval === "15m" ? 96 : card.interval === "1h" ? 24 : 0;
  if (n && barsPerDay) return `${family}, ${Math.round(n / barsPerDay)}-day lookback`;
  return `${family}, ${name.replace(/_/g, " ")}`;
}

/**
 * A strategy's tag: `G<generation>/<family>[·<variant>]`. Hand-written families are
 * generation 0 and carry their lookback as the variant (`G0/trend·4d`), since two
 * strategies can share one; a generated family is one rule (`G12/g0045`).
 */
export type StrategyTag = { generation: number; family: string; variant: string | null };

export function strategyTag(family: string, generation: number, card: { spec: { params?: Record<string, number> }; interval: string } | null): StrategyTag {
  const n = card?.spec.params?.n;
  const barsPerDay = card?.interval === "15m" ? 96 : card?.interval === "1h" ? 24 : 0;
  const variant = n && barsPerDay ? `${Math.round(n / barsPerDay)}d` : null;
  return { generation, family, variant };
}

export function tagText(t: StrategyTag): string {
  return `G${t.generation}/${t.family}${t.variant ? `·${t.variant}` : ""}`;
}

/** The settings that matter, in words, for a strategy's card. */
export function specSentence(card: { spec: { params?: Record<string, number> }; interval: string }): string {
  const p = card.spec.params ?? {};
  const barsPerDay = card.interval === "15m" ? 96 : card.interval === "1h" ? 24 : 0;
  const bits: string[] = [];
  if (typeof p.n === "number" && barsPerDay) bits.push(`${Math.round(p.n / barsPerDay)}-day lookback`);
  if (typeof p.stop === "number" && p.stop > 0) bits.push(`${p.stop} bps stop`);
  if (typeof p.shorts === "number") bits.push(p.shorts ? "long and short" : "long only");
  if (typeof p.hold === "number" && barsPerDay) bits.push(`${Math.round(p.hold / barsPerDay)}-day time limit`);
  return bits.join(", ");
}

/** A short calendar date in the reader's time zone, no time. */
export function dateShort(iso: string | number | null | undefined): string {
  if (!iso) return "unknown";
  return new Date(iso).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" });
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n.toLocaleString("en-US")} ${n === 1 ? one : many}`;
}

export function sentence(s: string): string {
  return s ? s[0].toUpperCase() + s.slice(1) : s;
}

/** A signed dollar amount with a real minus sign: +$2.41, −$0.80. */
export function signedUsd(x: number, digits = 2): string {
  const r = Number(x.toFixed(digits));
  return `${r > 0 ? "+" : r < 0 ? MINUS : ""}${usd(Math.abs(r), digits)}`;
}

/** One unit only, for tight columns: 42m, 19h, 30d. */
export function span(secs: number): string {
  if (secs < 3600) return `${Math.max(0, Math.floor(secs / 60))}m`;
  if (secs < 172_800) return `${Math.floor(secs / 3600)}h`;
  return `${Math.round(secs / 86_400)}d`;
}

/** A UTC timestamp with its year, so no reader has to guess day or month first. */
export function utcStamp(ms: number): string {
  return `${new Date(ms).toISOString().slice(0, 16).replace("T", " ")} UTC`;
}
