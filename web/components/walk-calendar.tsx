"use client";

import { useMemo, useState } from "react";
import { plural, signed } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { RuleFamily, RuleWalk, WalkCell } from "@/lib/v3/types";

const SHOWN = 12;

type Row = {
  id: string;
  /** Family key `study|family`. */
  key: string;
  label: React.ReactNode;
  meta: string;
  cells: (WalkCell | null)[];
  /** Lane rows show only their own rule; other windows are blank, not flat. */
  lane?: string;
  avg: number;
  onLabel?: () => void;
  labelPressed?: boolean;
};

const fmtDay = (d: string) => new Date(`${d}T00:00:00Z`).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });
/** The window's last day: `end` is the day after. */
const lastDay = (end: string) => new Date(Date.parse(`${end}T00:00:00Z`) - 86_400_000).toISOString().slice(0, 10);

/** Out-of-sample result as a muted fill: green up, red down, stronger with size. */
function fill(bps: number, scale: number): string {
  const p = Math.round(12 + 40 * Math.min(1, Math.abs(bps) / scale));
  return `color-mix(in oklch, var(${bps >= 0 ? "--pos" : "--neg"}) ${p}%, transparent)`;
}

/**
 * Every family's walk-forward on one calendar: a column per 60-day test window,
 * a cell per window holding the rule the fold picked and its out-of-sample
 * result. With one family in view, it opens into a lane per rule it picked.
 */
export function WalkCalendar({
  walk, families, keys, single, selected, ruleLabel, onFamily, onRule,
}: {
  walk: RuleWalk;
  families: Record<string, RuleFamily>;
  /** Family keys in the current view. */
  keys: Set<string>;
  /** The one family in view, when the page is filtered to it. */
  single: string | null;
  /** The opened rule, `study|spec`. */
  selected: string | null;
  ruleLabel: (spec: string) => string;
  onFamily: (key: string) => void;
  onRule: (key: string, rule: string) => void;
}) {
  const [all, setAll] = useState(false);
  const [active, setActive] = useState<{ r: number; c: number } | null>(null);
  const [focused, setFocused] = useState(false);
  const N = walk.windows.length;

  const inView = useMemo(() => walk.families.filter((f) => keys.has(f.key)), [walk.families, keys]);
  const noWalk = walk.none.filter((k) => keys.has(k));

  const rows: Row[] = useMemo(() => {
    const famLabel = (key: string) => {
      const f = families[key];
      return { name: f?.name ?? key, gen: f?.generation ?? null, study: f?.study.replace("binance-", "") ?? "" };
    };
    if (single) {
      const fam = inView.find((f) => f.key === single);
      if (!fam) return [];
      const l = famLabel(fam.key);
      const order: string[] = [];
      for (const c of fam.cells) if (c && !order.includes(c.spec)) order.push(c.spec);
      // Lanes name only the settings that differ between them; the rest are shared and said once.
      const kv = (spec: string) => Object.fromEntries((spec.split(":")[1] ?? "").split(",").map((x) => x.split("=")));
      const parsed = order.map(kv);
      const varying = Object.keys(parsed[0] ?? {}).filter((k) => parsed.some((p) => p[k] !== parsed[0][k]));
      const short = (spec: string) => {
        if (varying.length === 0) return ruleLabel(spec) || "no settings";
        const p = kv(spec);
        return ruleLabel(`x:${varying.map((k) => `${k}=${p[k]}`).join(",")}`);
      };
      const lanes = order.map((spec) => {
        const mine = fam.cells.filter((c) => c?.spec === spec) as WalkCell[];
        return {
          id: `${fam.key}|${spec}`,
          key: fam.key,
          label: <span className="truncate font-mono text-[12px] text-foreground" title={ruleLabel(spec)}>{short(spec)}</span>,
          meta: plural(mine.length, "window"),
          cells: fam.cells.map((c) => (c?.spec === spec ? c : null)),
          lane: spec,
          avg: mine.reduce((a, c) => a + c.bps, 0) / mine.length,
        };
      });
      return [
        {
          id: fam.key,
          key: fam.key,
          label: <span className={cn("truncate text-[13px] font-medium", l.gen !== null && "font-mono")}>{l.name}</span>,
          meta: `${plural(order.length, "rule")} picked · ${l.study}${varying.length && order.length > 1 ? ` · lanes differ in ${varying.map((k) => ruleLabel(`x:${k}=0`).replace(/ 0$/, "")).join(", ")}` : ""}`,
          cells: fam.cells,
          avg: fam.mean,
        },
        ...lanes,
      ];
    }
    return (all ? inView : inView.slice(0, SHOWN)).map((f) => {
      const l = famLabel(f.key);
      return {
        id: f.key,
        key: f.key,
        label: <span className={cn("truncate text-[13px]", l.gen !== null ? "font-mono" : "font-medium")}>{l.name}</span>,
        meta: `${l.gen !== null ? `gen ${l.gen}` : "hand-written"} · ${l.study}`,
        cells: f.cells,
        avg: f.mean,
        onLabel: () => onFamily(f.key),
      };
    });
  }, [single, inView, all, families, ruleLabel, onFamily]);

  // One colour scale for the view: the 90th percentile of |bps|, so one wild window does not wash out the rest.
  const scale = useMemo(() => {
    const xs = rows.flatMap((r) => r.cells.filter((c): c is WalkCell => c !== null).map((c) => Math.abs(c.bps))).sort((a, b) => a - b);
    return Math.max(1, xs[Math.floor(xs.length * 0.9)] ?? 1);
  }, [rows]);

  // Across the families in view (all of them, not just the rows shown): how many made money in each window.
  const breadth = useMemo(
    () => walk.windows.map((_, c) => {
      const picked = inView.map((f) => f.cells[c]).filter((x): x is WalkCell => x !== null);
      return { up: picked.filter((x) => x.bps > 0).length, n: picked.length };
    }),
    [walk.windows, inView],
  );

  const selSpec = selected && keys.size ? selected : null;
  const selHits = selSpec ? rows.some((r) => r.cells.some((c) => c && `${families[r.key]?.study}|${c.spec}` === selSpec)) : false;

  if (N === 0) return null;
  if (inView.length === 0)
    return (
      <p className="px-5 pb-4 text-[13px] text-muted-foreground">
        {noWalk.length ? `${plural(noWalk.length, "family", "families")} in this view never reached walk-forward: the gate stopped them before the checks.` : "No family in this view has a walk-forward report."}
      </p>
    );

  const cols = `var(--label) repeat(${N}, minmax(18px, 1fr)) 64px 64px`;
  const years = walk.windows.map((w, i) => (i === 0 || w.start.slice(0, 4) !== walk.windows[i - 1].start.slice(0, 4) ? w.start.slice(0, 4) : null));

  // r = -1 is the breadth row.
  const move = (dr: number, dc: number) => {
    const cur = active ?? { r: 0, c: 0 };
    setActive({ r: Math.max(single ? 0 : -1, Math.min(rows.length - 1, cur.r + dr)), c: Math.max(0, Math.min(N - 1, cur.c + dc)) });
  };
  const choose = (r: number, c: number) => {
    const row = rows[r];
    const cell = row?.cells[c];
    if (!row || !cell) return;
    const rule = `${families[row.key]?.study}|${cell.spec}`;
    onRule(row.key, rule);
  };
  const onKey = (e: React.KeyboardEvent) => {
    const k: Record<string, [number, number]> = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] };
    if (k[e.key]) {
      e.preventDefault();
      move(...k[e.key]);
    } else if (e.key === "Home" || e.key === "End") {
      e.preventDefault();
      setActive({ r: active?.r ?? 0, c: e.key === "Home" ? 0 : N - 1 });
    } else if ((e.key === "Enter" || e.key === " ") && active && active.r >= 0) {
      e.preventDefault();
      choose(active.r, active.c);
    }
  };

  const readout = (() => {
    if (!active) {
      if (selSpec && !selHits) return <span>The opened rule was never picked by a walk-forward window in this view.</span>;
      return <span className="text-faint">Point at a window, or focus the grid and use the arrow keys, for the rule it picked and how that rule did. Enter opens the rule below.</span>;
    }
    const w = walk.windows[active.c];
    const when = `${fmtDay(w.start)} to ${fmtDay(lastDay(w.end))}`;
    if (active.r < 0) {
      const b = breadth[active.c];
      return <span>{when} · <span className="text-foreground">{b.up} of {plural(b.n, "family", "families")}</span> made money out of sample{b.n < inView.length ? `, ${inView.length - b.n} picked nothing` : ""}</span>;
    }
    const row = rows[active.r];
    const cell = row?.cells[active.c];
    const name = families[row.key]?.name ?? row.key;
    if (!cell) return <span>{name} · {when} · {row.lane ? "another rule held this window" : "picked nothing, flat"}</span>;
    return (
      <span>
        {name} · {when} · <span className="font-mono text-[12px] text-foreground">{ruleLabel(cell.spec) || "no settings"}</span> ·{" "}
        <span className="font-mono tabular-nums text-foreground">{signed(cell.bps, 1)}</span> bps a day, Sharpe <span className="font-mono tabular-nums text-foreground">{signed(cell.sharpe, 2)}</span>, out of sample
      </span>
    );
  })();

  const lockLabel = walk.lockbox ? `Lockbox, sealed from ${fmtDay(walk.lockbox)}` : "Lockbox, sealed";

  return (
    <div>
      <div className="overflow-x-auto px-5">
        <div className="relative [--label:176px] sm:[--label:232px]" style={{ minWidth: `calc(var(--label) + 128px + ${N} * 18px)` }} onMouseLeave={() => !focused && setActive(null)}>
          {/* Years along the calendar, then the breadth row. */}
          <div className="grid items-end text-[11px] leading-4 text-faint" style={{ gridTemplateColumns: cols }}>
            <span className="sticky left-0 z-[2] self-stretch bg-card" />
            {years.map((y, i) => (
              <span key={walk.windows[i].start} className="relative h-4">
                {y && <span className="absolute left-0 font-mono whitespace-nowrap">{y}</span>}
              </span>
            ))}
            <span className="text-right">Avg</span>
            <span className="text-right text-info" title={lockLabel}>Lockbox</span>
          </div>

          <div
            role="grid"
            aria-label="Walk-forward out-of-sample result per family and 60-day window"
            tabIndex={0}
            onKeyDown={onKey}
            onFocus={() => {
              setFocused(true);
              if (!active) setActive({ r: 0, c: 0 });
            }}
            onBlur={() => setFocused(false)}
            className="mt-1 rounded-md outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-4 focus-visible:ring-offset-card"
          >
            {!single && <div role="row" className="relative grid items-end gap-x-px border-b pb-1.5" style={{ gridTemplateColumns: cols }}>
              <span role="rowheader" className="sticky left-0 z-[2] bg-card pr-3 text-xs text-muted-foreground">Families that made money</span>
              {/* Half the families: bars above it are windows where most families made money. */}
              <span aria-hidden className="pointer-events-none absolute right-[128px] bottom-[18px] left-[var(--label)] z-[1] border-t border-dashed border-foreground/30" />
              {breadth.map((b, c) => (
                <span
                  key={walk.windows[c].start}
                  role="gridcell"
                  aria-label={`${b.up} of ${b.n}`}
                  onMouseEnter={() => setActive({ r: -1, c })}
                  className={cn("relative flex h-6 items-end", active?.r === -1 && active.c === c && "bg-muted")}
                >
                  <span className={cn("w-full", b.up * 2 >= b.n ? "bg-foreground/60" : "bg-foreground/30")} style={{ height: `${b.n ? Math.max(1, (b.up / b.n) * 100) : 0}%` }} />
                </span>
              ))}
              <span />
              <span className="self-stretch bg-info/[0.06]" />
            </div>}

            <div className="pt-1">
              {rows.map((row, r) => (
                <div key={row.id} role="row" className={cn("grid items-center gap-x-px", row.lane ? "min-h-7" : "min-h-8", single && r === 0 && "border-b pb-1 mb-1")} style={{ gridTemplateColumns: cols }}>
                  <div role="rowheader" className="sticky left-0 z-[2] flex min-w-0 items-center self-stretch bg-card pr-3">
                    {row.onLabel ? (
                      <button type="button" onClick={row.onLabel} className="flex min-w-0 flex-col text-left outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring" title="Show this family's rules, one lane each">
                        {row.label}
                        <span className="text-[11px] leading-4 text-faint">{row.meta}</span>
                      </button>
                    ) : (
                      <div className={cn("flex min-w-0 flex-col", row.lane && "pl-3")}>
                        {row.label}
                        <span className="text-[11px] leading-4 text-faint">{row.meta}</span>
                      </div>
                    )}
                  </div>
                  {row.cells.map((cell, c) => {
                    const prev = c > 0 ? row.cells[c - 1] : null;
                    const switched = !row.lane && cell && prev && prev.spec !== cell.spec;
                    const rule = cell ? `${families[row.key]?.study}|${cell.spec}` : null;
                    const hit = selHits && rule === selSpec;
                    const dim = selHits && !hit;
                    const isActive = active?.r === r && active.c === c;
                    return (
                      <span
                        key={walk.windows[c].start}
                        role="gridcell"
                        aria-label={cell ? `${signed(cell.bps, 1)} bps a day` : row.lane ? "other rule" : "flat"}
                        aria-selected={hit || undefined}
                        onMouseEnter={() => setActive({ r, c })}
                        onClick={() => cell && choose(r, c)}
                        className={cn(
                          "relative h-5 rounded-[2px] transition-opacity",
                          cell ? "cursor-pointer" : row.lane ? "" : "bg-foreground/[0.05]",
                          dim && "opacity-25",
                          hit && "ring-1 ring-foreground ring-inset",
                          isActive && "outline-2 outline-offset-1 outline-foreground",
                        )}
                        style={cell ? { background: fill(cell.bps, scale) } : undefined}
                      >
                        {switched && <span aria-hidden className="absolute -top-1 -bottom-1 -left-px z-[1] w-0.5 rounded-full bg-foreground/85" />}
                      </span>
                    );
                  })}
                  <span className="text-right font-mono text-[12px] tabular-nums text-muted-foreground">{signed(row.avg, 1)}</span>
                  <span className="self-stretch bg-info/[0.06]" />
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>

      <p aria-live="polite" className="mt-3 min-h-10 border-t px-5 pt-3 text-[13px] leading-5 text-pretty text-muted-foreground">{readout}</p>

      <div className="flex flex-wrap items-center gap-x-5 gap-y-2 border-t px-5 py-3 text-[12px] text-muted-foreground">
        <span className="flex items-center gap-1.5">
          <span className="font-mono tabular-nums">{signed(-scale, 0)}</span>
          {[-1, -0.5, -0.15, 0.15, 0.5, 1].map((t) => (
            <span key={t} aria-hidden className="h-3 w-4 rounded-[2px]" style={{ background: fill(t * scale, scale) }} />
          ))}
          <span className="font-mono tabular-nums">{signed(scale, 0)}</span>
          <span>bps a day</span>
        </span>
        {!single && (
          <span className="flex items-center gap-1.5">
            <span aria-hidden className="h-3 w-4 rounded-[2px] bg-foreground/[0.05]" /> picked nothing, flat
          </span>
        )}
        {!single && (
          <span className="flex items-center gap-1.5">
            <span aria-hidden className="relative h-3 w-4 rounded-[2px] bg-foreground/10"><span className="absolute -inset-y-0.5 left-1.5 w-0.5 rounded-full bg-foreground/85" /></span> switched rule
          </span>
        )}
        <span className="text-info">{lockLabel}, not tested</span>
        {!single && inView.length > SHOWN && (
          <button type="button" onClick={() => setAll((v) => !v)} className="ml-auto rounded-md border px-3 py-1.5 text-foreground transition-colors hover:bg-muted">
            {all ? `Show the top ${SHOWN}` : `Show all ${inView.length} families`}
          </button>
        )}
      </div>
      {!single && noWalk.length > 0 && (
        <p className="border-t px-5 py-3 text-[12px] text-faint">
          {plural(noWalk.length, "more family", "more families")} in this view never reached walk-forward: the gate stopped them before the checks.
        </p>
      )}
    </div>
  );
}
