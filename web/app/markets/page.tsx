"use client";

import { useState } from "react";
import { ArrowDown, ArrowUp } from "lucide-react";
import Link from "next/link";
import { useLive } from "@/components/live";
import { DataTable, Empty, PageHead, Panel, Td, Th } from "@/components/bits";
import { Sparkline } from "@/components/charts";
import { price, signed } from "@/lib/format";
import { StationDot } from "@/components/pipeline";
import type { Quote } from "@/lib/v3/types";

type Key = "coin" | "chg1h" | "chgDay";

function RangeBar({ q }: { q: Quote }) {
  const span = q.high - q.low;
  const at = span > 0 ? ((q.mid - q.low) / span) * 100 : 50;
  return (
    <div className="flex items-center gap-2 text-xs text-muted-foreground" title={`Low ${price(q.low)} · now ${price(q.mid)} (${at.toFixed(0)}% of the day's range) · high ${price(q.high)}`}>
      <span className="w-[68px] text-right">{price(q.low)}</span>
      <span className="relative h-1 w-24 rounded-full bg-secondary" aria-hidden>
        <span className="absolute top-1/2 size-2 -translate-x-1/2 -translate-y-1/2 rounded-full bg-foreground" style={{ left: `${at}%` }} />
      </span>
      <span className="w-[68px]">{price(q.high)}</span>
    </div>
  );
}

export default function Markets() {
  const { live } = useLive();
  const [sort, setSort] = useState<{ key: Key; dir: 1 | -1 } | null>(null);
  const nowSec = live.now / 1000;
  const rows = [...live.quotes];
  if (sort) rows.sort((a, b) => (sort.key === "coin" ? a.coin.localeCompare(b.coin) : (a[sort.key] ?? -Infinity) - (b[sort.key] ?? -Infinity)) * sort.dir);
  const stale = rows.filter((q) => nowSec - q.lastSec > 30).length;
  const open = (live.engine.status?.books ?? []).reduce((n, b) => n + b.positions.length, 0);
  const maxAge = rows.length ? Math.round(Math.max(...rows.map((q) => nowSec - q.lastSec))) : 0;
  const slowest = rows.length ? rows.reduce((a, q) => (nowSec - q.lastSec > nowSec - a.lastSec ? q : a), rows[0]) : null;

  const head = (key: Key, label: string, right = true) => {
    const active = sort?.key === key;
    return (
      <Th className={right ? "text-right" : undefined}>
        <button
          type="button"
          onClick={() => setSort(active && sort.dir === -1 ? { key, dir: 1 } : active ? null : { key, dir: -1 })}
          className="inline-flex items-center gap-1 rounded-sm hover:text-foreground"
          aria-label={`Sort by ${label}`}
        >
          {label}
          {active && (sort.dir === -1 ? <ArrowDown className="size-3" /> : <ArrowUp className="size-3" />)}
        </button>
      </Th>
    );
  };

  return (
    <div className="flex flex-col gap-6">
      <PageHead
        question="Are the markets alive while the paper run waits?"
        lede={
          <>
            These are one mid per coin, sampled about once a second while the engine runs, 20 Hyperliquid perpetuals. The tick
            age column says how old the last quote is; the feed can be slower than the sampler. They are reference data, not
            the paper book&apos;s fills: the paper run re-plans at each bar close on Hyperliquid&apos;s own candles and fills at
            the next open. A quote here is not a trade.{" "}
            <Link href="/paper" className="text-foreground underline decoration-border underline-offset-4 hover:decoration-foreground">Read the paper run.</Link>
          </>
        }
        verdict={
          <div>
            <div className="flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
              <StationDot tone={rows.length === 0 ? "neutral" : stale > 0 ? "warn" : "good"} />
              <span className="font-medium text-foreground">Recorder</span>
              <span className="text-faint">·</span>
              <span>{rows.length} coins</span>
            </div>
            <p className="mt-2 font-mono text-[21px] leading-8 tracking-tight tabular-nums sm:text-[24px]">
              {rows.length === 0 ? "No quotes yet" : stale > 0 ? `${stale} of ${rows.length} coins stale` : `All ${rows.length} coins live`}
            </p>
            <p className="mt-2 max-w-[46ch] text-[13px] leading-5 text-muted-foreground">
              {rows.length > 0 && slowest ? `Slowest tick ${maxAge}s, on ${slowest.coin}. ` : ""}The paper book holds {open} {open === 1 ? "position" : "positions"}.
            </p>
          </div>
        }
      />

      <Panel
        flush
        title={rows.length ? `${rows.length} Hyperliquid perps` : "Hyperliquid perps"}
        description="Mid prices from the engine's 1-second recorder. The day starts at 00:00 UTC, or when the recorder did."
      >
        {rows.length === 0 ? (
          <Empty title="No quotes recorded">The recorder writes one row per coin per second while the engine runs. Start the engine to fill this table.</Empty>
        ) : (
          <DataTable minWidth={860}>
            <thead>
              <tr>
                {head("coin", "Coin", false)}
                <Th className="text-right">Price</Th>
                {head("chg1h", "1h %")}
                {head("chgDay", "Today % (UTC)")}
                <Th>Last 3 hours</Th>
                <Th className="text-center">Today&apos;s range, dot = now</Th>
                <Th className="text-right">Tick age</Th>
              </tr>
            </thead>
            <tbody>
              {rows.map((q) => {
                const age = nowSec - q.lastSec;
                return (
                  <tr key={q.coin}>
                    <Td className="font-medium">{q.coin}</Td>
                    <Td className="text-right font-mono text-[13px]">{price(q.mid)}</Td>
                    <Td className="text-right tabular-nums text-muted-foreground" title={q.chg1h === null ? "No price recorded an hour ago: the recorder was not running then" : undefined}>{q.chg1h === null ? "–" : signed(q.chg1h, 2, "%")}</Td>
                    <Td className="text-right tabular-nums text-muted-foreground">{q.chgDay === null ? "–" : signed(q.chgDay, 2, "%")}</Td>
                    <Td className="py-1.5"><Sparkline values={q.spark} width={150} height={30} up={null} /></Td>
                    <Td><div className="flex justify-center"><RangeBar q={q} /></div></Td>
                    <Td className={`text-right font-mono text-[11px] whitespace-nowrap ${age > 30 ? "text-warn" : "text-muted-foreground"}`} title={age > 30 ? "Older than 30 seconds; the price may have moved" : "Seconds since the last recorded tick"}>
                      {age < 1 ? "now" : `${Math.round(age)}s`}
                    </Td>
                  </tr>
                );
              })}
            </tbody>
          </DataTable>
        )}
        <p className="max-w-[62ch] border-t px-5 py-3 text-[13px] leading-5 text-muted-foreground">
          The dot on the range bar is the current mid; the bar spans the recorder&apos;s low to high since 00:00 UTC or since
          the recorder started. Prices use each coin&apos;s own tick precision, which is why the decimals differ by row. The
          sparkline has no axis and no colour: read its shape, not its level. A coin older than 30 seconds is flagged because
          its price may have moved since the last recorded tick.
        </p>
      </Panel>

      <Panel title="Why this page exists">
        <p className="max-w-[70ch] text-[13px] leading-5 text-muted-foreground">
          The recorder answers a simple question: is the engine awake, and is the market it trades alive? It is also the seed
          for a future check of fills between bars, which the research history cannot show. It does not decide anything, and
          no number here is a result.
        </p>
      </Panel>
    </div>
  );
}
