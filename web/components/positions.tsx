"use client";

import { Fragment } from "react";
import { ChevronRight } from "lucide-react";
import { useLive } from "@/components/live";
import { DataTable, Delta, Panel, Side, Td, Th } from "@/components/bits";
import { Gloss, GlossList } from "@/components/gloss";
import { plural, price, signedUsd, span, tagText, usd, utcStamp, type StrategyTag } from "@/lib/format";
import { cn } from "@/lib/utils";
import { NEAR_STOP_PCT, STALE_QUOTE_SECS, openBook, type OpenRow } from "@/lib/v3/stations";
import type { StrategyView } from "@/lib/v3/types";

const COLS = 8;

/** Signed dollars coloured by sign, like Delta; zero and missing stay neutral. */
function Pnl({ value, className }: { value: number | null; className?: string }) {
  if (value === null) return <span className={cn("text-faint", className)}>–</span>;
  const r = Number(value.toFixed(2));
  return <span className={cn(r > 0 ? "text-pos" : r < 0 ? "text-neg" : "text-muted-foreground", className)}>{signedUsd(value)}</span>;
}

/** `G0/trend·4d`: generation, family, variant. */
export function StratTag({ tag, fallback, className }: { tag: StrategyTag | null; fallback?: string; className?: string }) {
  if (!tag) return <span className={cn("font-mono text-xs", className)}>{fallback}</span>;
  return (
    <span className={cn("font-mono text-xs whitespace-nowrap", className)} aria-label={tagText(tag)}>
      <span className="text-muted-foreground">G{tag.generation}</span>
      <span className="text-faint">/</span>
      <span className="text-foreground">{tag.family}</span>
      {tag.variant && <span className="text-muted-foreground">·{tag.variant}</span>}
    </span>
  );
}

/** Stale price, no stop, or within NEAR_STOP_PCT of the stop: the rows that need a look. */
function trouble(r: OpenRow): boolean {
  return r.mark === null || !r.stop_px || (r.toStop !== null && r.toStop < NEAR_STOP_PCT);
}

/** Closest to trouble first: no price, then no stop, then distance to stop. */
function byRisk(a: OpenRow, b: OpenRow): number {
  const rank = (r: OpenRow) => (r.mark === null ? 0 : !r.stop_px ? 1 : 2);
  return rank(a) - rank(b) || (a.toStop ?? Infinity) - (b.toStop ?? Infinity) || a.coin.localeCompare(b.coin);
}

function ToStop({ row }: { row: OpenRow }) {
  if (!row.stop_px) return <span className="text-warn">No stop</span>;
  if (row.toStop === null) return <span className="text-faint">–</span>;
  return <span className={row.toStop < NEAR_STOP_PCT ? "text-warn" : undefined}>{row.toStop.toFixed(1)}%</span>;
}

function Mark({ row }: { row: OpenRow }) {
  if (row.mark === null) return <span className="text-warn">No price in {STALE_QUOTE_SECS / 60} min</span>;
  return <>{price(row.mark)}</>;
}

function sides(longs: number, shorts: number): string {
  if (!longs) return plural(shorts, "short");
  if (!shorts) return plural(longs, "long");
  return `${plural(longs, "long")} and ${plural(shorts, "short")}`;
}

type Group = { key: string; tag: StrategyTag | null; label: string; rule: string | null; hypothesis: string | null; clauses: string[]; rows: OpenRow[] };

/** One group per strategy, in the order of their most urgent row. */
function groupRows(rows: OpenRow[], strategies: StrategyView[]): Group[] {
  const byName = new Map(strategies.map((s) => [s.name, s]));
  const groups = new Map<string, Group>();
  for (const r of [...rows].sort(byRisk)) {
    let g = groups.get(r.strategy);
    if (!g) {
      g = { key: r.strategy, tag: r.tag, label: r.family, rule: r.rule, hypothesis: r.hypothesis, clauses: [], rows: [] };
      groups.set(r.strategy, g);
    }
    g.rows.push(r);
  }
  for (const g of groups.values()) {
    const entry = byName.get(g.key)?.entry;
    if (g.rows.some((r) => r.side > 0) && entry?.long) g.clauses.push(entry.long);
    if (g.rows.some((r) => r.side < 0) && entry?.short) g.clauses.push(entry.short);
  }
  return [...groups.values()];
}

/** The strategy, the clause that opened its positions, and the whole rule one tap away. */
function RuleLine({ g }: { g: Group }) {
  return (
    <details className="group/rule">
      <summary className="flex cursor-pointer list-none flex-wrap items-baseline gap-x-2 gap-y-0.5 rounded-sm text-[13px] leading-5 [&::-webkit-details-marker]:hidden">
        <ChevronRight aria-hidden className="size-3.5 shrink-0 translate-y-0.5 self-start text-muted-foreground transition-transform group-open/rule:rotate-90" />
        <StratTag tag={g.tag} fallback={g.label} />
        <span className="text-muted-foreground">{g.clauses.join("; ") || "rule not in the strategy file"}</span>
      </summary>
      <p className="mt-1.5 max-w-[68ch] pl-5.5 text-[13px] leading-5 text-pretty text-muted-foreground">
        {g.rule ?? "The strategy file carries no rule to read."}
        {g.hypothesis && <span className="text-faint"> Hypothesis {g.hypothesis}.</span>}
      </p>
    </details>
  );
}

function PositionRow({ p, now }: { p: OpenRow; now: number }) {
  return (
    <tr className={cn(trouble(p) && "bg-warn/[0.07] hover:bg-warn/10")}>
      <Td className="font-medium">
        <span className="inline-flex items-center gap-1.5">
          <Side side={p.side} bare />
          {p.coin}
        </span>
      </Td>
      <Td className="text-right font-medium"><Pnl value={p.pnl} /></Td>
      <Td className="text-right"><Delta value={p.move} digits={2} unit="%" /></Td>
      <Td className="text-right"><ToStop row={p} /></Td>
      <Td className="text-right">{usd(p.notional)}</Td>
      <Td className="text-right text-muted-foreground" title={`Time limit ${utcStamp(p.deadline_ms)}`}>
        {span((now - p.opened_ms) / 1000)} <span className="text-faint">of {span((p.deadline_ms - p.opened_ms) / 1000)}</span>
      </Td>
      <Td className="text-right font-mono text-xs text-muted-foreground">
        {price(p.entry_px)} <span className="text-faint">→</span> <span className="text-foreground"><Mark row={p} /></span>
      </Td>
      <Td className="text-right font-mono text-xs text-muted-foreground">{p.stop_px ? price(p.stop_px) : "–"}</Td>
    </tr>
  );
}

/** Below the table breakpoint: two lines a position, the result on the first. */
function PositionItem({ p, now }: { p: OpenRow; now: number }) {
  return (
    <li className={cn("px-5 py-2.5", trouble(p) && "bg-warn/[0.07]")}>
      <div className="flex items-baseline justify-between gap-3">
        <span className="inline-flex items-center gap-1.5 font-medium">
          <Side side={p.side} bare />
          {p.coin}
          <span className="font-normal text-muted-foreground">{usd(p.notional)}</span>
        </span>
        <Pnl value={p.pnl} className="font-medium" />
      </div>
      <div className="mt-0.5 flex items-baseline justify-between gap-3 text-xs text-muted-foreground">
        <span>
          <Delta value={p.move} digits={2} unit="%" /> move <span className="text-faint">·</span> <ToStop row={p} />
          {p.stop_px && p.toStop !== null && " to stop"}
          {p.mark === null && <span className="text-warn"> · no price</span>}
        </span>
        <span className="whitespace-nowrap">
          {span((now - p.opened_ms) / 1000)} <span className="text-faint">of {span((p.deadline_ms - p.opened_ms) / 1000)}</span>
        </span>
      </div>
    </li>
  );
}

function GroupHead({ g, as }: { g: Group; as: "tr" | "li" }) {
  const pnl = g.rows.some((r) => r.pnl !== null) ? g.rows.reduce((a, r) => a + (r.pnl ?? 0), 0) : null;
  const body = (
    <span className="flex items-baseline justify-between gap-3">
      <span>
        <StratTag tag={g.tag} fallback={g.label} /> <span className="text-muted-foreground">· {plural(g.rows.length, "position")}</span>
      </span>
      <Pnl value={pnl} />
    </span>
  );
  if (as === "li") return <li className="bg-muted/30 px-5 py-1.5 text-xs">{body}</li>;
  return (
    <tr className="bg-muted/30 hover:bg-muted/30">
      <td colSpan={COLS} className="px-5 py-1.5 text-xs">{body}</td>
    </tr>
  );
}

/**
 * The paper book's open positions, valued at the latest mids, and its closed
 * round trips. One component for Now and Paper run, so the two never drift.
 */
export function PositionsPanel({ tradeLimit, empty }: { tradeLimit?: number; empty?: React.ReactNode }) {
  const { live } = useLive();
  const book = openBook(live);
  const trades = tradeLimit ? live.trades.slice(0, tradeLimit) : live.trades;
  const open = book.rows.length > 0;
  const byName = new Map(live.strategies.map((s) => [s.name, s]));

  if (!open && trades.length === 0) {
    if (!empty) return null;
    return (
      <Panel flush title="Positions and closed trades" description="The paper book. Nothing here reaches a real venue.">
        {empty}
      </Panel>
    );
  }

  const title = open && trades.length ? "Open positions and closed trades" : open ? "Open positions" : "Closed trades";
  const groups = groupRows(book.rows, live.strategies);
  const grouped = groups.length > 1;
  const nearest = book.nearest;

  return (
    <Panel
      flush
      title={title}
      description={
        open ? (
          <>
            {sides(book.longs, book.shorts)}, paper, valued at the latest mid
            {book.unmarked > 0 && <span className="text-warn"> · {plural(book.unmarked, "position")} without a fresh price</span>}
          </>
        ) : (
          "No open positions. The latest round trips, after costs."
        )
      }
    >
      {open && (
        <div className="@container">
          <div className="flex flex-col gap-3 px-5 pb-4">
            <div>
              <p className="flex flex-wrap items-baseline gap-x-5 font-mono text-[22px] leading-7 tabular-nums">
                <span className="whitespace-nowrap"><Pnl value={book.pnl} /> <span className="text-[15px] text-muted-foreground">open</span></span>
                {nearest && (
                  <span className="whitespace-nowrap">
                    <span className="text-[15px] text-muted-foreground">nearest stop </span>
                    <span className={nearest.toStop! < NEAR_STOP_PCT ? "text-warn" : undefined}>{nearest.toStop!.toFixed(1)}%</span>
                    <span className="text-[15px] text-muted-foreground"> {nearest.coin}</span>
                  </span>
                )}
              </p>
              <p className="mt-0.5 text-[13px] leading-5 text-muted-foreground tabular-nums">
                {usd(book.gross)} gross
                {book.atRisk > 0 && (
                  <>
                    {" · "}
                    <span className="text-foreground">−{usd(book.atRisk)}</span> if every stop fills
                  </>
                )}
                {book.noStop > 0 && <span className="text-warn"> · {plural(book.noStop, "position")} with no stop</span>}
              </p>
            </div>
            <div className="flex flex-col gap-1.5 border-t pt-3">
              {groups.map((g) => (
                <RuleLine key={g.key} g={g} />
              ))}
            </div>
          </div>

          <ul aria-label="Open positions, paper, closest to its stop first" className="divide-y border-t tabular-nums @4xl:hidden">
            {groups.map((g) => (
              <Fragment key={g.key}>
                {grouped && <GroupHead g={g} as="li" />}
                {g.rows.map((p) => (
                  <PositionItem key={p.strategy + p.coin} p={p} now={live.now} />
                ))}
              </Fragment>
            ))}
            <li className="flex items-baseline justify-between gap-3 px-5 py-2.5 font-medium">
              <span>
                Total <span className="font-normal text-muted-foreground">{usd(book.gross)}</span>
              </span>
              <Pnl value={book.pnl} />
            </li>
          </ul>

          <div className="hidden @4xl:block">
            <DataTable minWidth={860} label="Open positions, paper, closest to its stop first" className="tabular-nums">
              <thead>
                <tr>
                  <Th>Coin</Th>
                  <Th className="text-right"><Gloss id="open_pnl">Open P&amp;L</Gloss></Th>
                  <Th className="text-right"><Gloss id="move">Move</Gloss></Th>
                  <Th className="text-right"><Gloss id="to_stop">To stop</Gloss></Th>
                  <Th className="text-right">Size</Th>
                  <Th className="text-right"><Gloss id="held">Held</Gloss></Th>
                  <Th className="text-right">Entry → mark</Th>
                  <Th className="text-right">Stop</Th>
                </tr>
              </thead>
              <tbody>
                {groups.map((g) => (
                  <Fragment key={g.key}>
                    {grouped && <GroupHead g={g} as="tr" />}
                    {g.rows.map((p) => (
                      <PositionRow key={p.strategy + p.coin} p={p} now={live.now} />
                    ))}
                  </Fragment>
                ))}
              </tbody>
              <tfoot>
                <tr className="border-t font-medium">
                  <Td>Total</Td>
                  <Td className="text-right"><Pnl value={book.pnl} /></Td>
                  <Td colSpan={2} />
                  <Td className="text-right">{usd(book.gross)}</Td>
                  <Td colSpan={3} />
                </tr>
              </tfoot>
            </DataTable>
          </div>

          <p className="border-t px-5 py-3 text-xs leading-5 text-muted-foreground">
            Paper money. Closest to its stop first; amber rows need a look.
          </p>
          <GlossList ids={["open_pnl", "move", "to_stop", "held", "tag"]} />
        </div>
      )}
      {trades.length > 0 && (
        <div className={cn(open && "border-t")}>
          {open && <div className="px-5 pt-4 pb-2 text-sm font-medium">Closed round trips</div>}
          <DataTable minWidth={640} label="Closed round trips, paper" className="tabular-nums">
            <thead>
              <tr><Th>Coin</Th><Th className="text-right">Result</Th><Th className="text-right">After costs</Th><Th>Exit</Th><Th>Closed</Th><Th><Gloss id="tag">Strategy</Gloss></Th></tr>
            </thead>
            <tbody>
              {trades.map((tr) => {
                const s = byName.get(tr.strategy);
                return (
                  <tr key={tr.strategy + tr.coin + tr.closed_ms}>
                    <Td className="font-medium">
                      <span className="inline-flex items-center gap-1.5"><Side side={tr.side} bare />{tr.coin}</span>
                    </Td>
                    <Td className="text-right"><Delta value={tr.pnl_bps} digits={1} unit=" bps" /></Td>
                    <Td className="text-right"><Pnl value={tr.pnl} /></Td>
                    <Td className="text-muted-foreground">{tr.reason}</Td>
                    <Td className="text-muted-foreground">{utcStamp(tr.closed_ms)}</Td>
                    <Td title={s?.rule ?? undefined}><StratTag tag={s?.tag ?? null} fallback={tr.strategy.replace(/_/g, " ")} /></Td>
                  </tr>
                );
              })}
            </tbody>
          </DataTable>
        </div>
      )}
    </Panel>
  );
}
