"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import type { Calibration, FamilyReport, ProbeQuestion, StrategyView } from "@/lib/v3/types";
import { familyName, plural, signed } from "@/lib/format";

export const FAMILY_COLOR = ["var(--chart-1)", "var(--chart-2)", "var(--chart-3)", "var(--chart-4)", "var(--chart-5)"];
const GRID = "#242424";
const AXIS = "#8f8f8f";
const INK = "#ededed";
const SURFACE = "#111111";

// Math.min(...xs) overflows the call stack on long arrays; these don't.
function minOf(xs: number[], start = Infinity): number { let m = start; for (const x of xs) if (x < m) m = x; return m; }
function maxOf(xs: number[], start = -Infinity): number { let m = start; for (const x of xs) if (x > m) m = x; return m; }

function useWidth<T extends HTMLElement>(): [React.RefObject<T | null>, number] {
  const ref = useRef<T>(null);
  const [w, setW] = useState(0);
  useEffect(() => {
    if (!ref.current) return;
    const ro = new ResizeObserver(([e]) => setW(Math.floor(e.contentRect.width)));
    ro.observe(ref.current);
    return () => ro.disconnect();
  }, []);
  return [ref, w];
}

function Tip({ x, y, width, children }: { x: number; y: number; width: number; children: React.ReactNode }) {
  const right = x > width / 2;
  return (
    <div
      role="status"
      className="pointer-events-none absolute z-10 w-max max-w-[260px] rounded-md border bg-popover px-2.5 py-1.5 text-xs leading-5 shadow-lg shadow-black/40"
      style={{ top: Math.max(0, y), ...(right ? { right: width - x + 12 } : { left: x + 12 }) }}
    >
      {children}
    </div>
  );
}

export function Sparkline({ values, width = 120, height = 28, up }: { values: number[]; width?: number; height?: number; up: boolean | null }) {
  if (values.length < 2) return <svg width={width} height={height} aria-hidden />;
  const [lo, hi] = [minOf(values), maxOf(values)];
  const span = hi - lo || 1;
  const x = (i: number) => 2 + (i / (values.length - 1)) * (width - 6);
  const y = (v: number) => 3 + (1 - (v - lo) / span) * (height - 6);
  const d = values.map((v, i) => `${i ? "L" : "M"}${x(i).toFixed(1)} ${y(v).toFixed(1)}`).join("");
  const color = up === null ? AXIS : up ? "var(--pos)" : "var(--neg)";
  return (
    <svg width={width} height={height} aria-hidden className="block">
      <path d={d} fill="none" stroke={color} strokeWidth={1.5} strokeLinejoin="round" strokeLinecap="round" />
      <circle cx={x(values.length - 1)} cy={y(values[values.length - 1])} r={2.5} fill={color} />
    </svg>
  );
}

/** Where the sequential test stands between its two fixed lines. */
export function TestGauge({ s }: { s: StrategyView }) {
  const span = s.graduate - s.kill || 1;
  const at = Math.min(100, Math.max(0, ((s.llr - s.kill) / span) * 100));
  const state = s.llr <= s.kill ? "bg-neg" : s.llr >= s.graduate ? "bg-pos" : "bg-foreground";
  return (
    <div className="min-w-0">
      <div className="relative h-1.5 rounded-full bg-secondary" aria-hidden>
        <span className="absolute top-0 left-0 h-full w-px bg-neg/70" />
        <span className="absolute top-0 right-0 h-full w-px bg-pos/70" />
        <span className={`absolute top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full ${state}`} style={{ left: `${at}%` }} />
      </div>
      <div className="mt-1.5 flex justify-between text-[11px] text-faint">
        <span>kill {signed(s.kill)}</span>
        <span className="text-muted-foreground">evidence {signed(s.llr)}</span>
        <span>graduate {signed(s.graduate)}</span>
      </div>
    </div>
  );
}

/** The sequential test as a path between two lines fixed before the run started. */
export function SequentialChart({ s, zoom }: { s: StrategyView; zoom: boolean }) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const [hover, setHover] = useState<number | null>(null);
  const H = 300;
  const m = { l: 44, r: 16, t: 14, b: 30 };
  const full = s.days.filter((d) => d.full);
  const horizon = Math.max(s.card.days_if_real, s.card.days_if_none);
  const xMax = zoom ? Math.max(30, full.length + 5) : Math.max(Math.ceil((horizon * 1.08) / 100) * 100, full.length + 10);
  const pad = (s.graduate - s.kill) * 0.22;
  const [yMin, yMax] = [s.kill - pad, s.graduate + pad];
  const iw = Math.max(10, width - m.l - m.r);
  const ih = H - m.t - m.b;
  const x = (d: number) => m.l + (d / xMax) * iw;
  const y = (v: number) => m.t + (1 - (Math.min(yMax, Math.max(yMin, v)) - yMin) / (yMax - yMin)) * ih;
  const pts = [{ d: 0, llr: 0 }, ...full.map((r, i) => ({ d: i + 1, llr: r.llr }))];
  const path = pts.map((p, i) => `${i ? "L" : "M"}${x(p.d).toFixed(1)} ${y(p.llr).toFixed(1)}`).join("");
  const last = pts[pts.length - 1];
  const step = [10, 50, 100, 200, 500, 1000].find((k) => xMax / k <= 10) ?? 2000;
  const ticks = Array.from({ length: Math.floor(xMax / step) + 1 }, (_, i) => i * step);
  const marks = zoom ? [] : [
    { d: s.card.days_if_none, label: "if there is no edge" },
    { d: s.card.days_if_real, label: "if the edge is real" },
  ];
  const hp = hover !== null ? pts[hover] : null;

  return (
    <div ref={ref} className="relative" style={{ height: H }}>
      {width > 0 && (
        <svg
          width={width}
          height={H}
          role="img"
          aria-label={`Sequential test for ${s.name}: ${signed(s.llr)} after ${s.fullDays} full days; kill at ${signed(s.kill)}, graduate at ${signed(s.graduate)}.`}
          onMouseLeave={() => setHover(null)}
          onMouseMove={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            const d = Math.round(((e.clientX - r.left - m.l) / iw) * xMax);
            setHover(pts.length > 1 ? Math.min(pts.length - 1, Math.max(0, d)) : null);
          }}
        >
          <rect x={m.l} y={m.t} width={iw} height={y(s.graduate) - m.t} fill="var(--pos)" opacity={0.07} />
          <rect x={m.l} y={y(s.kill)} width={iw} height={m.t + ih - y(s.kill)} fill="var(--neg)" opacity={0.07} />
          {ticks.map((t) => (
            <text key={t} x={x(t)} y={H - 10} fontSize={11} fill={AXIS} textAnchor="middle">
              {t}
            </text>
          ))}
          <text x={m.l + iw} y={H - 10} fontSize={11} fill={AXIS} textAnchor="end" dx={0} dy={-16}>
            full paper days
          </text>
          <line x1={m.l} x2={m.l + iw} y1={y(0)} y2={y(0)} stroke={GRID} />
          <text x={m.l - 8} y={y(0) + 4} fontSize={11} fill={AXIS} textAnchor="end">0</text>
          <line x1={m.l} x2={m.l + iw} y1={y(s.graduate)} y2={y(s.graduate)} stroke="var(--pos)" strokeWidth={1.5} />
          <text x={m.l - 8} y={y(s.graduate) + 4} fontSize={11} fill={AXIS} textAnchor="end">{signed(s.graduate)}</text>
          <text x={m.l + 10} y={y(s.graduate) - 8} fontSize={12} fill={INK}>Graduate above this line</text>
          <line x1={m.l} x2={m.l + iw} y1={y(s.kill)} y2={y(s.kill)} stroke="var(--neg)" strokeWidth={1.5} />
          <text x={m.l - 8} y={y(s.kill) + 4} fontSize={11} fill={AXIS} textAnchor="end">{signed(s.kill)}</text>
          <text x={m.l + 10} y={y(s.kill) + 18} fontSize={12} fill={INK}>Kill below this line</text>
          {marks.map((k, i) => (
            <g key={k.label}>
              <line x1={x(k.d)} x2={x(k.d)} y1={y(s.graduate)} y2={y(s.kill)} stroke="#3a3a3a" />
              <text y={i === 0 ? y(0) + 20 : y(s.graduate) + 22} fontSize={11} fill="#a1a1a1" textAnchor="end">
                <tspan x={x(k.d) - 8}>Decision near day {Math.round(k.d)}</tspan>
                <tspan x={x(k.d) - 8} dy={14}>{k.label}</tspan>
              </text>
            </g>
          ))}
          <path d={path} fill="none" stroke={INK} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />
          <circle cx={x(last.d)} cy={y(last.llr)} r={5} fill={INK} stroke={SURFACE} strokeWidth={2} />
          <text x={x(last.d)} y={y(last.llr) - 10} fontSize={11} fill={AXIS} textAnchor="middle">now</text>
          {hp && (
            <g>
              <line x1={x(hp.d)} x2={x(hp.d)} y1={m.t} y2={m.t + ih} stroke="#4a4a4a" />
              <circle cx={x(hp.d)} cy={y(hp.llr)} r={5} fill={INK} stroke={SURFACE} strokeWidth={2} />
            </g>
          )}
        </svg>
      )}
      {hp && hover !== null && (
        <Tip x={x(hp.d)} y={y(hp.llr) - 44} width={width}>
          <div className="text-muted-foreground">{hover === 0 ? "Start" : `Day ${hp.d}, ${full[hover - 1].day}`}</div>
          <div>
            Test at <span className="font-medium">{signed(hp.llr)}</span>
            {hover > 0 && <span className="text-muted-foreground"> after a {signed(full[hover - 1].ret * 1e4, 1, " bps")} day</span>}
          </div>
        </Tip>
      )}
    </div>
  );
}

// ---- The gate as a line map ----

const SHORT: Record<string, string> = {
  "evidence": "Evidence",
  "walk-forward": "Walk-forward",
  "reality check": "Reality check",
  "deflated Sharpe": "Deflated Sharpe",
  "overfitting (PBO)": "Overfitting",
  "plateau": "Plateau",
  "costs ×2": "Costs ×2",
  "one bar late": "One bar late",
  "coins": "Coins",
  "quarters": "Quarters",
  "best days removed": "Best days removed",
  "timing baseline": "Timing baseline",
  "market-neutral part": "Market-neutral",
};

/** The check that ended a family's run, or -1 when it ran the whole gate. */
export function stopIndex(f: FamilyReport): number {
  if (f.checks.length === 0) return -1;
  if (f.verdict === "INSUFFICIENT_EVIDENCE") return Math.max(0, f.checks.findIndex((c) => !c.pass));
  return f.checks.findIndex((c) => c.fatal && !c.pass);
}

/** Why a check ended a run: "p = 0.217" and "needs p < 0.05". */
function why(c: FamilyReport["checks"][number]): [string, string] {
  const v = c.value.replace(/\s*\(.*$/, "").replace(/\s+over .*$/, "").replace(/\s*\[.*?\]/g, "").replace(/(^|[\s[(,])-(?=\d)/g, "$1−");
  return [v, `needs ${c.need.replace(/\s+with .*$/, "")}`];
}

/** Check names across families, in gate order: a check only some families ran is slotted after its predecessor. */
function checkColumns(families: FamilyReport[]): string[] {
  const names: string[] = [];
  for (const f of families) {
    let after = -1;
    for (const c of f.checks) {
      const at = names.indexOf(c.name);
      if (at >= 0) after = at;
      else names.splice(++after, 0, c.name);
    }
  }
  return names;
}

/** A family's run on the shared columns: its check per column (or none), and the column it stopped at (-1 = ran it all). */
function aligned(f: FamilyReport, names: string[]) {
  const by = new Map(f.checks.map((c) => [c.name, c]));
  const at = names.map((n) => by.get(n));
  const s = stopIndex(f);
  const stopName = s >= 0 ? f.checks[s]?.name : undefined;
  const stop = stopName ? names.indexOf(stopName) : f.checks.length === 0 ? 0 : -1;
  const ran = at.reduce((last, c, i) => (c ? i : last), -1);
  return { at, stop, end: stop < 0 ? ran : stop, reach: stop < 0 ? names.length : stop };
}

export type Cohort = { label: string; families: FamilyReport[] };

/**
 * How far each family got through the gate. One neutral bar per family, sorted by distance, so the rows form the
 * funnel; the end cap says how the run ended and the label says why. Above the rows, one count line per cohort says
 * how many of its families were still in the run when each check came up.
 */
export function GateLineMap({ families, selected, onSelect, cohorts, keepOrder = false }: { families: FamilyReport[]; selected: string; onSelect: (family: string) => void; cohorts: Cohort[]; keepOrder?: boolean }) {
  const names = useMemo(() => checkColumns([...cohorts.flatMap((c) => c.families), ...families]), [cohorts, families]);
  const N = names.length;
  const rows = useMemo(
    () =>
      families
        .map((f, order) => ({ f, order, ...aligned(f, names) }))
        .sort((a, b) => (keepOrder ? a.order - b.order : b.reach - a.reach || a.order - b.order)),
    [families, names, keepOrder],
  );
  const counts = useMemo(
    () =>
      cohorts.map((c) => {
        const runs = c.families.map((f) => aligned(f, names));
        return {
          label: c.label,
          // null where no family in the cohort has that check at all: "0 still in" would read as a result.
          still: names.map((_, i) => (runs.some((r) => r.at[i]) ? runs.filter((r) => r.reach >= i).length : null)),
          passed: c.families.filter((f) => f.verdict === "PROMOTE_TO_PAPER").length,
        };
      }),
    [cohorts, names],
  );
  const sealed = !families.some((f) => f.lockbox_opened);
  const wrap = useRef<HTMLDivElement>(null);
  const [tip, setTip] = useState<{ r: number; c: number; x: number; y: number; below: boolean } | null>(null);
  const cols = `var(--label) repeat(${N}, minmax(56px, 1fr)) 72px`;
  // Column centres as a share of the track (the N check columns), for the bar geometry.
  const at = (i: number) => `calc(${(i + 0.5) * 100}% / ${N})`;

  const hover = (e: React.MouseEvent<HTMLDivElement>, r: number, c: number) => {
    const box = wrap.current?.getBoundingClientRect();
    const cell = e.currentTarget.getBoundingClientRect();
    if (!box) return;
    setTip({ r, c, x: cell.left - box.left + cell.width / 2, y: r < 2 ? cell.bottom - box.top : cell.top - box.top, below: r < 2 });
  };
  const tr = tip ? rows[tip.r] : null;
  const tc = tr ? tr.at[tip!.c] : null;
  const label = (f: FamilyReport) => f.name ?? familyName(f.family);

  return (
    <div className="overflow-x-auto">
      <div ref={wrap} className="relative [--label:200px] sm:[--label:256px]" style={{ minWidth: `calc(var(--label) + 72px + ${N} * 56px)` }} onMouseLeave={() => setTip(null)}>
        {/* Check names, then per cohort how many families were still in the run when each check came up. */}
        <div className="grid items-end text-[11px] leading-[14px] text-muted-foreground" style={{ gridTemplateColumns: cols }}>
          <span className="sticky left-0 z-[2] self-stretch bg-card" />
          {names.map((n, i) => (
            <span key={n} className={`px-0.5 text-center text-balance ${tip?.c === i ? "text-foreground" : ""}`}>{SHORT[n] ?? n}</span>
          ))}
          <span className="px-1 text-center text-info">Lockbox, {sealed ? "sealed" : "opened"}</span>
        </div>
        <div className="mt-2 border-b pb-2">
          {counts.map((c) => (
            <div key={c.label} className="grid items-center py-0.5 font-mono text-[13px] tabular-nums" style={{ gridTemplateColumns: cols }}>
              <span className="sticky left-0 z-[2] bg-card pr-3 font-sans text-xs text-faint">{c.label}</span>
              {/* Counts stop at the first zero: after that every column would only repeat it. */}
              {c.still.map((k, i) => {
                const prev = c.still.slice(0, i).filter((x): x is number => x !== null).at(-1);
                if (k === null) return <span key={names[i]} className="text-center text-faint" title="No family in this group runs this check">–</span>;
                return (
                  <span key={names[i]} className={`text-center ${k === 0 ? "text-faint" : k < (prev ?? k) ? "text-foreground" : "text-muted-foreground"}`}>{k === 0 && prev === 0 ? "" : k}</span>
                );
              })}
              <span className={`text-center ${c.passed ? "text-pos" : "text-faint"}`}>{c.passed}</span>
            </div>
          ))}
        </div>

        <div className="relative pt-1">
          {/* The finish: one line every run would have to reach. */}
          <span aria-hidden className="pointer-events-none absolute top-1 bottom-0 right-[36px] w-px bg-info/60" />
          {rows.map(({ f, at: checksAt, stop, end, reach }, r) => {
            const on = selected === f.family;
            const insuff = f.verdict === "INSUFFICIENT_EVIDENCE";
            const ended = stop >= 0 ? checksAt[stop] : undefined;
            return (
              <div
                key={`${f.study}-${f.family}`}
                onClick={() => onSelect(f.family)}
                className={`grid min-h-12 cursor-pointer items-center rounded-md transition-colors ${on ? "bg-popover" : "bg-card hover:bg-[#141414]"}`}
                style={{ gridTemplateColumns: cols }}
              >
                <button type="button" aria-pressed={on} onClick={(e) => { e.stopPropagation(); onSelect(f.family); }} className="sticky left-0 z-[2] flex flex-col self-stretch justify-center rounded-md bg-inherit py-2 pr-4 pl-3 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset">
                  <span className="flex items-baseline gap-2 text-[13px] font-medium text-foreground">
                    <span className={f.name ? "font-mono" : ""}>{label(f)}</span>
                    <span className="text-[11px] font-normal text-faint">{f.generation !== undefined ? `generation ${f.generation}` : "hand-written"}</span>
                  </span>
                  <span className="text-[11px] leading-4 text-muted-foreground">
                    {!ended ? (() => {
                      const short = f.checks.filter((c) => !c.pass);
                      if (short.length === 0) return "Ran the whole gate";
                      const c = short[0];
                      return (
                        <>
                          Ran the whole gate, short on <span className="text-warn">{SHORT[c.name] ?? c.name}</span> {why(c)[0]} · <span className="whitespace-nowrap">{why(c)[1]}</span>
                          {short.length > 1 && ` and ${short.length - 1} more`}
                        </>
                      );
                    })() : (
                      <>
                        <span className={insuff ? "text-warn" : "text-neg"}>{SHORT[ended.name] ?? ended.name}</span>
                        {" "}{why(ended)[0]} · <span className="whitespace-nowrap">{why(ended)[1]}</span>
                      </>
                    )}
                  </span>
                </button>
                <div className="relative h-12" style={{ gridColumn: `2 / span ${N}` }}>
                  {/* The road not taken: faint stations after the run ended, or checks this family never had. */}
                  {names.map((n, i) => (i > end || !checksAt[i]) && (
                    <span key={n} aria-hidden className="absolute top-1/2 size-1 -translate-x-1/2 -translate-y-1/2 rounded-full bg-[#3a3a3a]" style={{ left: at(i) }} />
                  ))}
                  {/* Distance covered. */}
                  <span aria-hidden className={`absolute top-1/2 h-1.5 -translate-y-1/2 rounded-full ${on ? "bg-foreground" : "bg-foreground/70"}`} style={{ left: 0, width: reach === 0 || end < 0 ? 0 : at(end) }} />
                  {checksAt.map((c, i) => c && i < end && !c.pass && (
                    <span key={c.name} aria-hidden className="absolute top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-foreground bg-card" style={{ left: at(i) }} />
                  ))}
                  {stop >= 0 && (
                    <span aria-hidden className={`absolute top-1/2 h-5 w-1.5 -translate-x-1/2 -translate-y-1/2 rounded-[2px] ${insuff ? "bg-warn" : "bg-neg"}`} style={{ left: at(stop) }} />
                  )}
                  {names.map((n, i) => (
                    <div key={n} className="absolute inset-y-0 -translate-x-1/2" style={{ left: at(i), width: `calc(100% / ${N})` }} onMouseEnter={(e) => hover(e, r, i)} />
                  ))}
                </div>
                {(() => {
                  // Distance to the lockbox: checks failed, or no distance when the gate never ran its checks.
                  const short = f.checks.filter((c) => !c.pass).length;
                  const unjudged = f.checks.length === 0 || (insuff && f.checks.every((c) => c.name === "evidence" || c.pass));
                  return (
                    <span className={`text-center font-mono text-[12px] tabular-nums ${f.verdict === "PROMOTE_TO_PAPER" ? "text-pos" : unjudged ? "text-faint" : short <= 1 ? "text-foreground" : "text-muted-foreground"}`}>
                      {f.verdict === "PROMOTE_TO_PAPER" ? "passed" : unjudged ? "–" : `${short} short`}
                    </span>
                  );
                })()}
              </div>
            );
          })}
        </div>

        {tip && tr && tc && (
          <div
            role="status"
            className="pointer-events-none absolute z-10 w-max max-w-[280px] rounded-md border bg-popover px-2.5 py-1.5 text-xs leading-5 shadow-lg shadow-black/40"
            style={{ left: tip.x, top: tip.y, transform: `translate(-50%, ${tip.below ? "6px" : "calc(-100% - 6px)"})` }}
          >
            <div className="font-medium">{label(tr.f)}: {SHORT[tc.name] ?? tc.name}</div>
            <div>{tc.value}</div>
            <div className="text-muted-foreground">Needs {tc.need}</div>
            <div className={tr.stop >= 0 && tip.c > tr.stop ? "text-muted-foreground" : tc.pass ? "text-pos" : tip.c === tr.stop ? (tr.f.verdict === "INSUFFICIENT_EVIDENCE" ? "text-warn" : "text-neg") : tc.fatal ? "text-neg" : "text-warn"}>
              {tr.stop >= 0 && tip.c > tr.stop ? "Computed after the run ended: " : ""}
              {tc.pass ? "passed" : tip.c === tr.stop && tr.f.verdict === "INSUFFICIENT_EVIDENCE" ? "not enough evidence to judge" : tc.fatal ? "fatal miss" : "short of the bar, not fatal"}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

export function MapLegend() {
  const item = "flex items-center gap-2";
  return (
    <div className="flex flex-wrap gap-x-5 gap-y-2 text-xs text-muted-foreground">
      <span className={item}><span aria-hidden className="h-1.5 w-6 rounded-full bg-foreground/70" />Checks cleared</span>
      <span className={item}><span aria-hidden className="size-2.5 rounded-full border-2 border-foreground" />Missed, not fatal</span>
      <span className={item}><span aria-hidden className="h-4 w-1.5 rounded-[2px] bg-neg" />Fatal miss, the run ends</span>
      <span className={item}><span aria-hidden className="h-4 w-1.5 rounded-[2px] bg-warn" />Not enough evidence to judge</span>
    </div>
  );
}

/** What Jev said against what happened, by fifths of its answer. On the diagonal is calibrated. */
export function Reliability({ q }: { q: ProbeQuestion }) {
  const [hover, setHover] = useState<number | null>(null);
  const S = 220, m = { l: 36, r: 10, t: 10, b: 30 };
  const [lo, hi] = [0.1, 0.7];
  const iw = S - m.l - m.r, ih = S - m.t - m.b;
  const x = (v: number) => m.l + ((v - lo) / (hi - lo)) * iw;
  const y = (v: number) => m.t + (1 - (v - lo) / (hi - lo)) * ih;
  const ticks = [0.1, 0.3, 0.5, 0.7];
  const pts = q.buckets;
  return (
    <div className="relative" style={{ width: S, height: S }}>
      <svg width={S} height={S} role="img" aria-label={`Reliability of ${q.id}: Jev's answer against the observed rate in five groups.`}>
        {ticks.map((t) => (
          <g key={t}>
            <line x1={m.l} x2={m.l + iw} y1={y(t)} y2={y(t)} stroke={GRID} />
            <line x1={x(t)} x2={x(t)} y1={m.t} y2={m.t + ih} stroke={GRID} />
            <text x={m.l - 6} y={y(t) + 4} fontSize={11} fill={AXIS} textAnchor="end">{Math.round(t * 100)}%</text>
            <text x={x(t)} y={S - 12} fontSize={11} fill={AXIS} textAnchor="middle">{Math.round(t * 100)}%</text>
          </g>
        ))}
        <line x1={x(lo)} y1={y(lo)} x2={x(hi)} y2={y(hi)} stroke="#5a5a5a" />
        <line x1={m.l} x2={m.l + iw} y1={y(q.base_rate)} y2={y(q.base_rate)} stroke="var(--info)" opacity={0.7} />
        <path d={pts.map((p, i) => `${i ? "L" : "M"}${x(p[0]).toFixed(1)} ${y(p[1]).toFixed(1)}`).join("")} fill="none" stroke={INK} strokeWidth={2} strokeLinejoin="round" />
        <text x={m.l + iw / 2} y={S - 2} fontSize={10} fill={AXIS} textAnchor="middle">Jev said (%)</text>
        <text transform={`translate(10 ${m.t + ih / 2}) rotate(-90)`} fontSize={10} fill={AXIS} textAnchor="middle">Happened (%)</text>
        {pts.map((p, i) => (
          <g key={i} onMouseEnter={() => setHover(i)} onMouseLeave={() => setHover(null)}>
            <circle cx={x(p[0])} cy={y(p[1])} r={12} fill="transparent" />
            <circle cx={x(p[0])} cy={y(p[1])} r={hover === i ? 6 : 4.5} fill={INK} stroke={SURFACE} strokeWidth={2} />
          </g>
        ))}
      </svg>
      {hover !== null && (
        <Tip x={x(pts[hover][0])} y={y(pts[hover][1]) + 10} width={S}>
          <div>Jev said {(pts[hover][0] * 100).toFixed(0)}%</div>
          <div>It happened {(pts[hover][1] * 100).toFixed(0)}% of the time</div>
          <div className="text-muted-foreground">{pts[hover][3]} entries, {signed(pts[hover][2], 0, " bps")} mean</div>
        </Tip>
      )}
    </div>
  );
}

// ---- The gate's calibration: what it does when the answer is known ----

const OUTCOME = [
  { key: "PROMOTE_TO_PAPER", label: "Promoted", bar: "bg-pos", text: "text-[#0a0a0a]" },
  { key: "REVISE", label: "Revise", bar: "bg-warn/45", text: "text-foreground" },
  { key: "REJECT", label: "Rejected", bar: "bg-neg/35", text: "text-foreground" },
] as const;

/** The false-positive ceiling the gate is held to on markets with no edge. */
const FALSE_POSITIVE_CEILING = 5;

/**
 * One 100% bar per synthetic market, from no edge to the strongest planted edge. The promoted share is the reading:
 * near zero with no edge (a false positive), rising with the edge (the gate's power).
 */
export function CalibrationChart({ calibrations }: { calibrations: Calibration[] }) {
  const rows = calibrations
    .map((c) => {
      const edge = Number(c.title.match(/edge\s+([\d.]+)/)?.[1] ?? NaN);
      const total = c.rows.reduce((a, r) => a + r.panels, 0) || 1;
      const share = (k: string) => ((c.rows.find((r) => r.outcome === k)?.panels ?? 0) / total) * 100;
      return { c, edge, total, family: c.title.split(" ")[0], parts: OUTCOME.map((o) => ({ ...o, pct: share(o.key) })) };
    })
    .sort((a, b) => (a.edge || 0) - (b.edge || 0));

  return (
    <div className="flex flex-col">
      <div className="grid grid-cols-[minmax(0,1fr)_72px] items-end gap-x-4 pb-2 text-[11px] text-faint sm:grid-cols-[200px_minmax(0,1fr)_88px]">
        <span className="hidden sm:block">Synthetic market</span>
        <span>Share of panels by verdict</span>
        <span className="text-right">Promoted</span>
      </div>
      {rows.map(({ c, edge, total, family, parts }) => {
        const none = edge === 0;
        const promoted = parts[0].pct;
        const over = none && promoted > FALSE_POSITIVE_CEILING;
        return (
          <div key={c.title} className="grid grid-cols-[minmax(0,1fr)_72px] items-center gap-x-4 gap-y-1.5 border-t py-3.5 sm:grid-cols-[200px_minmax(0,1fr)_88px]">
            <div className="col-span-2 sm:col-span-1">
              <div className="text-[13px] font-medium text-foreground">{none ? "No edge" : `Edge ${edge} planted`}</div>
              <div className="text-[11px] text-muted-foreground">{total} panels, {familyName(family).toLowerCase()} rules</div>
            </div>
            <div className="relative">
              <div className="flex h-7 overflow-hidden rounded-[6px]" role="img" aria-label={parts.map((p) => `${p.label} ${Math.round(p.pct)}%`).join(", ")}>
                {parts.map((p) => p.pct > 0 && (
                  <div key={p.key} className={`flex items-center overflow-hidden px-2 text-[11px] font-medium tabular-nums whitespace-nowrap ${p.bar} ${p.text}`} style={{ width: `${p.pct}%` }}>
                    <span className="hidden sm:inline">{p.pct >= 7 ? `${Math.round(p.pct)}%` : ""}</span>
                  </div>
                ))}
              </div>
              {none && (
                <div aria-hidden className="pointer-events-none absolute -top-1.5 -bottom-1.5 w-px bg-foreground/80" style={{ left: `${FALSE_POSITIVE_CEILING}%` }}>
                  <span className="absolute top-full left-1.5 mt-0.5 text-[11px] whitespace-nowrap text-muted-foreground">Ceiling {FALSE_POSITIVE_CEILING}%</span>
                </div>
              )}
            </div>
            <div className="text-right">
              <div className={`font-mono text-[19px] leading-6 tabular-nums ${over ? "text-neg" : !none && promoted > 0 ? "text-pos" : "text-foreground"}`}>{Math.round(promoted)}%</div>
              <div className="text-[11px] text-faint">{none ? (over ? "above the ceiling" : "false positives") : "edge found"}</div>
            </div>
          </div>
        );
      })}
      <div className="flex flex-wrap gap-x-5 gap-y-2 border-t pt-3 text-xs text-muted-foreground">
        {OUTCOME.map((o) => (
          <span key={o.key} className="flex items-center gap-2"><span aria-hidden className={`size-2.5 rounded-[3px] ${o.bar}`} />{o.key === "REVISE" ? "Sent back to revise" : o.label}</span>
        ))}
        <span className="flex items-center gap-2"><span aria-hidden className="h-3.5 w-px bg-foreground/80" />False-positive ceiling</span>
      </div>
    </div>
  );
}

/** The walk-forward check's own reading: mean bps/day and its 95% interval, and whether the interval clears zero. */
function walkRead(f: FamilyReport): { text: string; pass: boolean } | null {
  const c = f.checks.find((k) => k.name === "walk-forward");
  if (!c) return null;
  const text = c.value.replace(/\s+over .*$/, "").replace(/(^|[\s[(,])-(?=\d)/g, "$1−");
  return { text, pass: c.pass };
}

const DAY = 86_400_000;

/** Rank colour for generated families: 0 = best (green) … 1 = worst (red), mixed in OKLCH so the middle stays bright. */
function rankColor(t: number): string {
  const p = Math.round((1 - t) * 100);
  return `color-mix(in oklch, var(--pos) ${p}%, var(--neg))`;
}

const monthFmt = new Intl.DateTimeFormat("en-GB", { month: "short", timeZone: "UTC" });
const dateFmt = new Intl.DateTimeFormat("en-GB", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });

/**
 * Walk-forward out of sample, every family on one chart against calendar dates. Hand-written
 * families keep their colours and labels; generated families run from green (the best final
 * return) to red (the worst) by rank, with their median dashed. Hovering names the nearest line;
 * clicking selects that family.
 */
export function OosAll({ hand, generated, selected, onSelect }: { hand: FamilyReport[]; generated: FamilyReport[]; selected: string; onSelect: (family: string) => void }) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const [hover, setHover] = useState<{ t: number; key: string } | null>(null);
  const series = useMemo(() => {
    const ok = (f: FamilyReport) => f.oosCum.length > 1 && !!f.oosStart;
    const at = (f: FamilyReport) => Date.parse(`${f.oosStart}T00:00:00Z`);
    // One colour rule for every family: ranked by final return, best green, worst red.
    const pool = [...hand.filter(ok).map((f) => ({ f, gen: false })), ...generated.filter(ok).map((f) => ({ f, gen: true }))].sort(
      (a, b) => b.f.oosCum[b.f.oosCum.length - 1] - a.f.oosCum[a.f.oosCum.length - 1],
    );
    const out = pool.map(({ f, gen }, k) => {
      const rank = pool.length > 1 ? k / (pool.length - 1) : 0;
      return { f, key: `${f.study}-${f.family}`, t0: at(f), gen, rank, place: k + 1, color: rankColor(rank) };
    });
    // Worst first so the best sit on top; hand-written after every generated line.
    return [...out.filter((x) => x.gen).reverse(), ...out.filter((x) => !x.gen).reverse()];
  }, [hand, generated]);
  const gens = series.filter((s) => s.gen);
  const tMin = series.length ? minOf(series.map((s) => s.t0)) : 0;
  const oosEnd = series.length ? maxOf(series.map((s) => s.t0 + (s.f.oosCum.length - 1) * DAY)) : DAY;
  const dayOf = (d: string | null) => (d ? Date.parse(`${d}T00:00:00Z`) : NaN);
  const lockStarts = series.map((s) => dayOf(s.f.lockboxStart)).filter(Number.isFinite);
  const panelEnds = series.map((s) => dayOf(s.f.panelEnd)).filter(Number.isFinite);
  const lockStart = lockStarts.length ? minOf(lockStarts) : null;
  const tMax = maxOf(panelEnds, oosEnd);
  const valueAt = (s: (typeof series)[number], t: number) => {
    const i = Math.round((t - s.t0) / DAY);
    return i < 0 || i >= s.f.oosCum.length ? null : s.f.oosCum[i];
  };
  // Median of the generated lines on each calendar day they cover.
  const median = useMemo(() => {
    if (gens.length < 3) return [] as { t: number; v: number }[];
    const out: { t: number; v: number }[] = [];
    for (let t = tMin; t <= tMax; t += DAY) {
      const v = gens.map((s) => valueAt(s, t)).filter((x): x is number => x !== null).sort((a, b) => a - b);
      if (v.length >= gens.length / 2) out.push({ t, v: v[Math.floor(v.length / 2)] });
    }
    return out;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gens, tMin, tMax]);
  if (!series.length) return <p className="py-8 text-center text-[13px] text-muted-foreground">No walk-forward results on file. Run <code className="font-mono">lab validate</code>.</p>;

  const H = 360;
  const m = { l: 48, r: 16, t: 12, b: 28 };
  const all = series.flatMap((s) => s.f.oosCum);
  const lo = minOf(all, 0), hi = maxOf(all, 0);
  const stepY = [10, 20, 25, 50, 100].find((s) => (hi - lo) / s <= 8) ?? 100;
  const [yMin, yMax] = [Math.floor(lo / stepY) * stepY, Math.ceil(hi / stepY) * stepY];
  const iw = Math.max(10, width - m.l - m.r), ih = H - m.t - m.b;
  const x = (t: number) => m.l + ((t - tMin) / (tMax - tMin || 1)) * iw;
  const y = (v: number) => m.t + (1 - (v - yMin) / (yMax - yMin || 1)) * ih;
  const path = (s: (typeof series)[number]) => s.f.oosCum.map((p, i) => `${i ? "L" : "M"}${x(s.t0 + i * DAY).toFixed(1)} ${y(p).toFixed(1)}`).join("");
  const yTicks: number[] = [];
  for (let v = yMin; v <= yMax + 1e-9; v += stepY) yTicks.push(v);
  // Month ticks, thinned to fit; January carries the year.
  const ticks: { t: number; label: string }[] = [];
  {
    const d = new Date(tMin);
    const cur = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + 1, 1));
    const months: number[] = [];
    while (cur.getTime() <= tMax) {
      months.push(cur.getTime());
      cur.setUTCMonth(cur.getUTCMonth() + 1);
    }
    const every = Math.max(1, Math.ceil(months.length / Math.max(2, Math.floor(iw / 70))));
    let year = -1;
    months.forEach((t, k) => {
      if (k % every) return;
      const dt = new Date(t);
      // The first tick of each calendar year carries the year, whichever month it lands on.
      const y = dt.getUTCFullYear();
      ticks.push({ t, label: y !== year ? `${monthFmt.format(dt)} ${y}` : monthFmt.format(dt) });
      year = y;
    });
  }
  const last = (s: { f: FamilyReport }) => s.f.oosCum[s.f.oosCum.length - 1];
  const endT = (s: (typeof series)[number]) => s.t0 + (s.f.oosCum.length - 1) * DAY;
  const byPlace = [...series].sort((a, b) => a.place - b.place);
  const best = byPlace[0] ?? null;
  const worst = byPlace.length > 1 ? byPlace[byPlace.length - 1] : null;
  const picked = series.find((s) => s.f.family === selected && s.gen);
  const active = series.find((s) => s.key === hover?.key);
  const name = (f: FamilyReport) => f.name ?? familyName(f.family);

  const tag = (s: (typeof series)[number]) => (s === best ? ", best" : s === worst ? ", worst" : "");
  const labels = [
    ...series.filter((s) => !s.gen).map((s) => ({ key: s.key, text: `${name(s.f)}${tag(s)}`, color: s.color, ink: INK, v: last(s), t: endT(s) })),
    ...(median.length ? [{ key: "median", text: "Median", color: AXIS, ink: AXIS, v: median[median.length - 1].v, t: median[median.length - 1].t }] : []),
    ...[best, worst]
      .filter((s): s is (typeof series)[number] => !!s && s.gen)
      .map((s) => ({ key: s.key, text: `${name(s.f)}${tag(s)}`, color: s.color, ink: s.color, v: last(s), t: endT(s) })),
    ...(picked && picked !== best && picked !== worst ? [{ key: picked.key, text: name(picked.f), color: picked.color, ink: INK, v: last(picked), t: endT(picked) }] : []),
  ]
    .map((l) => ({ ...l, y: y(l.v) }))
    .sort((a, b) => a.y - b.y);
  for (let i = 1; i < labels.length; i++) if (labels[i].y - labels[i - 1].y < 15) labels[i].y = labels[i - 1].y + 15;

  const move = (e: React.MouseEvent<SVGSVGElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const px = e.clientX - r.left, py = e.clientY - r.top;
    const t = tMin + Math.round(((Math.min(Math.max(px, m.l), m.l + iw) - m.l) / iw) * ((tMax - tMin) / DAY)) * DAY;
    let best: (typeof series)[number] | null = null;
    let dist = Infinity;
    for (const s of series) {
      const v = valueAt(s, t);
      if (v === null) continue;
      const d = Math.abs(y(v) - py);
      if (d < dist) [best, dist] = [s, d];
    }
    setHover({ t, key: best && dist < 24 ? best.key : "" });
  };
  const style = (s: (typeof series)[number]) => {
    const on = s.key === hover?.key || s === picked || (!s.gen && s.f.family === selected);
    if (s.gen) {
      const edge = s === best || s === worst;
      return { opacity: on || edge ? 1 : 0.45, w: on ? 2.25 : edge ? 2 : 1 };
    }
    return { opacity: active && !on ? 0.5 : 1, w: on ? 3 : 2.25 };
  };
  const drawn = [...series.filter((s) => s.gen && s !== active && s !== picked), ...series.filter((s) => s.gen && (s === active || s === picked))];

  return (
    <div className="flex flex-col gap-3">
      <div ref={ref} className="relative" style={{ height: H }}>
        {width > 0 && (
          <svg
            width={width}
            height={H}
            role="img"
            aria-label={`Cumulative out-of-sample return of ${series.length} families' walk-forward picks, ${dateFmt.format(tMin)} to ${dateFmt.format(tMax)}.`}
            onMouseMove={move}
            onMouseLeave={() => setHover(null)}
            onClick={() => active && onSelect(active.f.family)}
            className={active ? "cursor-pointer" : undefined}
          >
            {yTicks.map((v) => (
              <g key={v}>
                <line x1={m.l} x2={m.l + iw} y1={y(v)} y2={y(v)} stroke={v === 0 ? "#4a4a4a" : GRID} />
                <text x={m.l - 8} y={y(v) + 4} fontSize={11} fill={AXIS} textAnchor="end">{signed(v, 0, "%")}</text>
              </g>
            ))}
            {lockStart !== null && lockStart < tMax && (
              <g>
                <rect x={x(lockStart)} y={m.t} width={Math.max(0, x(tMax) - x(lockStart))} height={ih} fill="var(--info)" opacity={0.06} />
                <line x1={x(lockStart)} x2={x(lockStart)} y1={m.t} y2={m.t + ih} stroke="var(--info)" strokeOpacity={0.5} strokeDasharray="3 3" />
                <text x={x(lockStart) + 8} y={m.t + 14} fontSize={11} fill="var(--info)">Lockbox, sealed</text>
                <text x={x(lockStart) + 8} y={m.t + 28} fontSize={11} fill={AXIS}>from {dateFmt.format(lockStart)}</text>
              </g>
            )}
            {ticks.map((k) => (
              <g key={k.t}>
                <line x1={x(k.t)} x2={x(k.t)} y1={m.t + ih} y2={m.t + ih + 4} stroke={GRID} />
                <text x={x(k.t)} y={H - 8} fontSize={11} fill={AXIS} textAnchor="middle">{k.label}</text>
              </g>
            ))}
            {drawn.map((s) => {
              const st = style(s);
              return <path key={s.key} d={path(s)} fill="none" stroke={s.color} strokeOpacity={st.opacity} strokeWidth={st.w} strokeLinejoin="round" />;
            })}
            {median.length > 0 && (
              <path d={median.map((p, i) => `${i ? "L" : "M"}${x(p.t).toFixed(1)} ${y(p.v).toFixed(1)}`).join("")} fill="none" stroke={INK} strokeOpacity={0.75} strokeWidth={1.5} strokeDasharray="5 4" />
            )}
            {series.filter((s) => !s.gen).map((s) => {
              const st = style(s);
              return <path key={s.key} d={path(s)} fill="none" stroke={s.color} strokeOpacity={st.opacity} strokeWidth={st.w} strokeLinejoin="round" />;
            })}
            {labels.map((l) => (
              <g key={l.key}>
                <circle cx={x(l.t)} cy={y(l.v)} r={3} fill={l.color} stroke={SURFACE} strokeWidth={1.5} />
              </g>
            ))}
            {hover && <line x1={x(hover.t)} x2={x(hover.t)} y1={m.t} y2={m.t + ih} stroke="#3a3a3a" />}
          </svg>
        )}
        {hover && (
          <Tip x={x(hover.t)} y={m.t + 4} width={width}>
            <div className="text-muted-foreground">{dateFmt.format(hover.t)}</div>
            {active ? (
              <>
                <div className="flex items-center gap-2">
                  <span className="h-[3px] w-3 rounded-full" style={{ background: active.color }} />
                  <span className={active.f.name ? "font-mono" : ""}>{name(active.f)}</span>
                  <span className="ml-auto pl-3 font-medium">{signed(valueAt(active, hover.t) ?? last(active), 1, "%")}</span>
                </div>
                <div className="text-muted-foreground">
                  {active.f.generation !== undefined ? `Generation ${active.f.generation}` : "Hand-written"}, {active.place} of {series.length} by final return
                </div>
                {walkRead(active.f) && <div className="text-muted-foreground">Walk-forward {walkRead(active.f)!.text}</div>}
                <div className="text-faint">Click to read its checks</div>
              </>
            ) : (
              (() => {
                const md = median.find((p) => p.t === hover.t);
                return md ? <div>Median {signed(md.v, 1, "%")}</div> : null;
              })()
            )}
          </Tip>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-x-5 gap-y-2 text-xs text-muted-foreground">
        <span className="flex items-center gap-2">
          Final return: highest
          <span aria-hidden className="h-[3px] w-16 rounded-full" style={{ background: "linear-gradient(to right in oklch, var(--pos), var(--neg))" }} />
          lowest
        </span>
        {series.length > gens.length && <span className="flex items-center gap-2"><span aria-hidden className="h-[3px] w-4 rounded-full bg-foreground" />Hand-written, {series.length - gens.length}</span>}
        <span className="flex items-center gap-2"><span aria-hidden className="h-px w-4 bg-foreground/50" />{series.length > gens.length ? `Generated, ${gens.length}` : `${gens.length} families`}</span>
        {median.length > 0 && <span className="flex items-center gap-2"><span aria-hidden className="w-4 border-t border-dashed border-foreground/75" />Median</span>}
        {lockStart !== null && <span className="flex items-center gap-2"><span aria-hidden className="size-2.5 rounded-[2px] bg-info/20" />Lockbox, not tested yet</span>}
      </div>
    </div>
  );
}

/**
 * Return per generation: every family's final walk-forward return as one dot, placed on its
 * generation (hand-written families first), with the best and the median of each generation joined.
 * Hover a dot to name it; click it to read its checks.
 */
export function ReturnByGeneration({ families, selected, onSelect }: { families: FamilyReport[]; selected: string; onSelect: (family: string) => void }) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const [hover, setHover] = useState<string | null>(null);
  const { dots, gens, skipped } = useMemo(() => {
    const ok = families.filter((f) => f.oosCum.length > 1);
    const byGen = new Map<number, FamilyReport[]>();
    for (const f of ok) {
      const g = f.generation ?? 0;
      byGen.set(g, [...(byGen.get(g) ?? []), f]);
    }
    const gens = [...byGen.keys()].sort((a, b) => a - b).map((g) => {
      const fs = byGen.get(g)!.map((f) => ({ f, v: f.oosCum[f.oosCum.length - 1] })).sort((a, b) => b.v - a.v);
      const vs = fs.map((d) => d.v).sort((a, b) => a - b);
      const mid = vs.length % 2 ? vs[(vs.length - 1) / 2] : (vs[vs.length / 2 - 1] + vs[vs.length / 2]) / 2;
      return { g, fs, best: fs[0].v, median: mid };
    });
    const dots = gens.flatMap(({ g, fs }) => fs.map((d, k) => ({ ...d, g, k, n: fs.length, key: `${d.f.study}-${d.f.family}` })));
    return { dots, gens, skipped: families.length - ok.length };
  }, [families]);
  if (!gens.length) return <p className="py-8 text-center text-[13px] text-muted-foreground">No walk-forward results on file yet.</p>;

  const H = 300;
  const m = { l: 48, r: 16, t: 12, b: 28 };
  const iw = Math.max(10, width - m.l - m.r), ih = H - m.t - m.b;
  const gMin = gens[0].g, gMax = gens[gens.length - 1].g;
  const band = iw / (gMax - gMin + 1);
  const cx = (g: number) => m.l + (g - gMin + 0.5) * band;
  // Families of one generation side by side inside its band, best on the left.
  const spread = Math.min(band * 0.6, 40);
  const dx = (d: (typeof dots)[number]) => cx(d.g) + (d.n > 1 ? (d.k / (d.n - 1) - 0.5) * spread : 0);
  const vals = dots.map((d) => d.v);
  const lo = minOf(vals, 0), hi = maxOf(vals, 0);
  const stepY = [5, 10, 20, 25, 50, 100].find((s) => (hi - lo) / s <= 6) ?? 100;
  const [yMin, yMax] = [Math.floor(lo / stepY) * stepY, Math.ceil(hi / stepY) * stepY];
  const y = (v: number) => m.t + (1 - (v - yMin) / (yMax - yMin || 1)) * ih;
  const yTicks: number[] = [];
  for (let v = yMin; v <= yMax + 1e-9; v += stepY) yTicks.push(v);
  const every = Math.max(1, Math.ceil(gens.length / Math.max(2, Math.floor(iw / 36))));
  const xTicks = gens.filter((_, i) => i % every === 0 || gens[i].g === 0);
  // The best line runs through each generation's best dot (left in its band); the median sits at the band's centre.
  const line = (pick: (x: (typeof gens)[number]) => number, atBest = false) =>
    gens.filter((x) => x.g > 0).map((x, i) => `${i ? "L" : "M"}${(atBest && x.fs.length > 1 ? cx(x.g) - spread / 2 : cx(x.g)).toFixed(1)} ${y(pick(x)).toFixed(1)}`).join("");
  const active = dots.find((d) => d.key === hover) ?? null;
  const name = (f: FamilyReport) => f.name ?? familyName(f.family);
  const failedN = (f: FamilyReport) => f.checks.filter((c) => !c.pass).length;
  const generated = gens.filter((x) => x.g > 0);
  const top = generated.length ? generated.reduce((a, b) => (b.best > a.best ? b : a)) : null;

  const move = (e: React.MouseEvent<SVGSVGElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const px = e.clientX - r.left, py = e.clientY - r.top;
    let near: string | null = null;
    let dist = 14;
    for (const d of dots) {
      const dd = Math.hypot(dx(d) - px, y(d.v) - py);
      if (dd < dist) [near, dist] = [d.key, dd];
    }
    setHover(near);
  };

  return (
    <div className="flex flex-col gap-3">
      <div ref={ref} className="relative" style={{ height: H }}>
        {width > 0 && (
          <svg
            width={width}
            height={H}
            role="img"
            aria-label={`Final walk-forward return of ${dots.length} families by generation, ${gens.length} generations. ${top ? `Best generated: ${signed(top.best, 1, "%")} in generation ${top.g}.` : ""}`}
            onMouseMove={move}
            onMouseLeave={() => setHover(null)}
            onClick={() => active && onSelect(active.f.family)}
            className={active ? "cursor-pointer" : undefined}
          >
            {yTicks.map((v) => (
              <g key={v}>
                <line x1={m.l} x2={m.l + iw} y1={y(v)} y2={y(v)} stroke={v === 0 ? "#4a4a4a" : GRID} />
                <text x={m.l - 8} y={y(v) + 4} fontSize={11} fill={AXIS} textAnchor="end">{signed(v, 0, "%")}</text>
              </g>
            ))}
            {gMin === 0 && gMax > 0 && <line x1={m.l + band} x2={m.l + band} y1={m.t} y2={m.t + ih} stroke={GRID} strokeDasharray="3 3" />}
            {xTicks.map((x) => (
              <text key={x.g} x={cx(x.g)} y={H - 8} fontSize={11} fill={AXIS} textAnchor="middle">{x.g === 0 ? "Hand" : x.g}</text>
            ))}
            {generated.length > 1 && (
              <>
                <path d={line((x) => x.median)} fill="none" stroke={INK} strokeOpacity={0.75} strokeWidth={1.5} strokeDasharray="5 4" />
                <path d={line((x) => x.best, true)} fill="none" stroke={INK} strokeWidth={2} strokeLinejoin="round" />
              </>
            )}
            {dots.map((d) => {
              const on = d.key === hover || d.f.family === selected;
              return (
                <circle
                  key={d.key}
                  cx={dx(d)}
                  cy={y(d.v)}
                  r={on ? 5.5 : 4}
                  fill={d.v >= 0 ? "var(--pos)" : "var(--neg)"}
                  fillOpacity={on ? 1 : 0.7}
                  stroke={on ? INK : SURFACE}
                  strokeWidth={on ? 1.5 : 2}
                />
              );
            })}
          </svg>
        )}
        {active && (
          <Tip x={dx(active)} y={Math.max(0, y(active.v) - 40)} width={width}>
            <div className="flex items-center gap-2">
              <span className="size-2 rounded-full" style={{ background: active.v >= 0 ? "var(--pos)" : "var(--neg)" }} />
              <span className={active.f.name ? "font-mono" : ""}>{name(active.f)}</span>
              <span className="ml-auto pl-3 font-medium">{signed(active.v, 1, "%")}</span>
            </div>
            <div className="text-muted-foreground">
              {active.g === 0 ? "Hand-written" : `Generation ${active.g}`}, {active.k + 1} of {active.n} in it
            </div>
            <div className="text-muted-foreground">{active.f.checks.length ? `${failedN(active.f)} of ${active.f.checks.length} checks failed` : "Stopped before the checks"}</div>
            <div className="text-faint">Click to read its checks</div>
          </Tip>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-x-5 gap-y-2 text-xs text-muted-foreground">
        <span className="flex items-center gap-2"><span aria-hidden className="size-2 rounded-full bg-pos" /><span aria-hidden className="-ml-1 size-2 rounded-full bg-neg" />One family, ended up or down</span>
        {generated.length > 1 && <span className="flex items-center gap-2"><span aria-hidden className="h-[2px] w-4 rounded-full bg-foreground" />Best of each generation</span>}
        {generated.length > 1 && <span className="flex items-center gap-2"><span aria-hidden className="w-4 border-t border-dashed border-foreground/75" />Median of each generation</span>}
        {skipped > 0 && <span className="text-faint">{plural(skipped, "family", "families")} without a walk-forward line left out</span>}
      </div>
    </div>
  );
}
