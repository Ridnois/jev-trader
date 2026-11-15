"use client";

import Link from "next/link";
import { useState } from "react";
import { ChevronRight } from "lucide-react";
import { useLive } from "@/components/live";
import { Empty, PageHead, Panel, Verdict, Tag } from "@/components/bits";
import { SequentialChart } from "@/components/charts";
import { StationIndex } from "@/components/pipeline";
import { PositionsPanel, StratTag } from "@/components/positions";
import { Gloss, GlossList } from "@/components/gloss";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { signed } from "@/lib/format";
import { cn } from "@/lib/utils";
import { engineState, stations } from "@/lib/v3/stations";
import type { StrategyView } from "@/lib/v3/types";

const n = (x: number) => Math.round(x).toLocaleString("en-US");
const years = (d: number) => (d / 365).toFixed(1);
const utcTime = (ts: string | number) => new Date(ts).toISOString().slice(11, 16);

/** The days the test expects to need, as a range, from the card. */
function horizon(s: StrategyView) {
  const lo = Math.min(s.card.days_if_none, s.card.days_if_real);
  const hi = Math.max(s.card.days_if_none, s.card.days_if_real);
  const same = Math.round(lo) === Math.round(hi);
  return { days: same ? n(lo) : `${n(lo)}–${n(hi)}`, yr: same ? years(lo) : `${years(lo)}–${years(hi)}` };
}

function drawdownWarn(s: StrategyView): boolean {
  return s.drawdownLimit > 0 && s.drawdown / s.drawdownLimit > 0.66;
}

/** Wide rows: strategy, test, days, drawdown, decision expected, and a call once there is one. */
const GRID = "@3xl:grid @3xl:grid-cols-[minmax(0,1.8fr)_repeat(4,minmax(0,1fr))_5.5rem] @3xl:items-center @3xl:gap-x-4";

function labLine(s: StrategyView): string {
  if (s.labVerdict === "REJECT") return `The Lab rejected this family${s.labStoppedAt ? ` at the ${s.labStoppedAt}` : ""}; it runs here as an unproven candidate.`;
  if (s.labVerdict === "PROMOTE_TO_PAPER") return "The Lab promoted this rule; the paper run decides whether the live market agrees.";
  return "The Lab has not promoted this rule: watched, not trusted.";
}

/** Everything about one run that the row does not say: the chart, the forecast, parity and settings. */
function RunDetail({ s }: { s: StrategyView }) {
  const [zoom, setZoom] = useState(false);
  const mu = s.card.mu_daily * 1e4;
  const sd = s.card.sd_daily * 1e4;
  return (
    <div className="border-t bg-muted/20">
      <div className="flex flex-col gap-3 px-5 pt-3 sm:flex-row sm:items-center sm:justify-between">
        <p className="text-[13px] leading-5 text-pretty text-muted-foreground">
          {labLine(s)}{" "}
          <Link href="/lab" className="text-foreground underline decoration-border underline-offset-2 hover:decoration-foreground">Read the gate</Link>
        </p>
        <Tabs value={zoom ? "sofar" : "whole"} onValueChange={(v) => setZoom(v === "sofar")}>
          <TabsList className="h-8">
            <TabsTrigger value="whole" className="text-xs">Whole test</TabsTrigger>
            <TabsTrigger value="sofar" className="text-xs">So far</TabsTrigger>
          </TabsList>
        </Tabs>
      </div>
      <SequentialChart s={s} zoom={zoom} />
      <dl className="grid gap-x-8 gap-y-3 border-t px-5 py-3 text-[13px] leading-5 sm:grid-cols-3">
        <div>
          <dt className="text-muted-foreground"><Gloss id="card">Card</Gloss> forecast</dt>
          <dd className="mt-0.5 tabular-nums">
            {signed(mu, 1)} bps a day <span className="text-muted-foreground">against a {sd.toFixed(0)} bps daily swing</span>
          </dd>
        </div>
        <div>
          <dt className="text-muted-foreground"><Gloss id="parity">Parity</Gloss></dt>
          <dd className="mt-0.5 text-pretty">{s.parity ?? "No parity line yet."}</dd>
        </div>
        <div className="min-w-0">
          <dt className="text-muted-foreground">Settings · {s.card.interval} bars · {s.hypothesis}</dt>
          <dd className="mt-0.5 font-mono text-xs break-all text-muted-foreground">
            {s.book?.spec ?? `${s.card.spec.family}:${Object.entries(s.card.spec.params).map(([k, v]) => `${k}=${v}`).join(",")}`}
          </dd>
        </div>
      </dl>
    </div>
  );
}

function RunRow({ s, sharedLines, markUnproven }: { s: StrategyView; sharedLines: boolean; markUnproven: boolean }) {
  const h = horizon(s);
  const decided = s.call !== "continue";
  const dd = (
    <span className={cn(drawdownWarn(s) && "text-warn")}>
      {(s.drawdown * 100).toFixed(2)}%<span className="text-muted-foreground"> of {(s.drawdownLimit * 100).toFixed(1)}%</span>
    </span>
  );
  const days = (
    <>
      {n(s.fullDays)}
      {s.partialDays > 0 && <span className="text-muted-foreground"> +{n(s.partialDays)} partial</span>}
    </>
  );
  const test = (
    <>
      <span className="font-medium">{signed(s.llr)}</span>
      {!sharedLines && <span className="text-muted-foreground"> ({signed(s.kill)} / {signed(s.graduate)})</span>}
    </>
  );
  return (
    <details id={s.name} className="group/run scroll-mt-24 border-t first:border-t-0">
      <summary className={cn("cursor-pointer list-none px-5 py-2.5 text-[13px] tabular-nums hover:bg-muted/40 [&::-webkit-details-marker]:hidden", GRID)}>
        <span className="flex min-w-0 items-center justify-between gap-2 @3xl:justify-start">
          <span className="flex min-w-0 items-center gap-2">
            <ChevronRight aria-hidden className="size-3.5 shrink-0 text-muted-foreground transition-transform group-open/run:rotate-90" />
            <StratTag tag={s.tag} className="text-[13px]" />
            {markUnproven && s.status !== "validated" && <span className="hidden text-xs text-warn @lg:inline">unproven</span>}
          </span>
          {decided && <Tag kind={s.call} className="@3xl:hidden" />}
        </span>
        {/* Narrow: one muted line under the name. */}
        <span className="mt-1 block pl-5.5 text-xs text-muted-foreground @3xl:hidden">
          test {test} · {days} {s.fullDays === 1 && !s.partialDays ? "day" : "days"} · drawdown {dd} · decides in {h.days} days
        </span>
        <span className="hidden text-right @3xl:block">{test}</span>
        <span className="hidden text-right @3xl:block">{days}</span>
        <span className="hidden text-right @3xl:block">{dd}</span>
        <span className="hidden text-right whitespace-nowrap @3xl:block" title={`${h.yr} years of full paper days`}>
          {h.days} <span className="text-muted-foreground">d</span>
        </span>
        <span className="hidden justify-end @3xl:flex">{decided && <Tag kind={s.call} />}</span>
      </summary>
      <RunDetail s={s} />
    </details>
  );
}

/** One row per run, the numbers that decide it; open a row for its chart, card and settings. */
function Runs({ strategies, stale }: { strategies: StrategyView[]; stale: boolean }) {
  const first = strategies[0];
  const allUnproven = strategies.every((s) => s.status !== "validated");
  const sharedLines = strategies.every((s) => s.kill.toFixed(2) === first.kill.toFixed(2) && s.graduate.toFixed(2) === first.graduate.toFixed(2));
  return (
    <Panel
      flush
      title="Runs"
      description={
        <>
          {sharedLines
            ? <>Each run is killed when its <Gloss id="sequential-test">test</Gloss> falls below {signed(first.kill)} or its drawdown passes its band, and graduates above {signed(first.graduate)}.</>
            : <>Each run is killed when its <Gloss id="sequential-test">test</Gloss> falls below its kill line or its drawdown passes its band, and graduates above its graduate line.</>}{" "}
          {allUnproven && <span className="text-warn">{strategies.length === 1 ? "It is an unproven candidate" : "All are unproven candidates"}: the Lab passed none of them. </span>}{strategies.every((s) => s.call === "continue") && "No run has a call yet. "}Open a run for its chart.
        </>
      }
    >
      <div className={cn("@container border-t", stale && "text-muted-foreground")}>
        <div className={cn("hidden border-b px-5 py-2 text-xs font-medium text-muted-foreground", GRID)} aria-hidden>
          <span className="pl-5.5"><Gloss id="tag">Strategy</Gloss></span>
          <span className="text-right">Test now</span>
          <span className="text-right"><Gloss id="paper-day">Full paper days</Gloss></span>
          <span className="text-right"><Gloss id="drawdown">Drawdown</Gloss></span>
          <span className="text-right">Decision expected</span>
          <span />
        </div>
        {strategies.map((s) => (
          <RunRow key={s.name} s={s} sharedLines={sharedLines} markUnproven={!allUnproven} />
        ))}
      </div>
    </Panel>
  );
}

export default function Paper() {
  const { live, offline } = useLive();
  const paper = stations(live)[1];
  const engine = engineState(live);
  const ts = live.engine.status?.ts;
  const stale = offline || engine.level === "stopped" || engine.level === "degraded";
  const asOf = ts ? `${utcTime(ts)} UTC` : null;

  return (
    <div className="flex flex-col gap-6">
      <PageHead
        question="Does the live market agree with the backtest?"
        lede={
          <>
            Each strategy trades a paper book on Hyperliquid&apos;s live prices. A test fixed before the run decides whether it continues, is killed or graduates.{" "}
            <Link href="/start" className="text-foreground underline decoration-border underline-offset-4 hover:decoration-foreground">Where this fits</Link>
          </>
        }
        verdict={
          <Verdict
            rule={false}
            kind={stale ? "warn" : paper.tone === "bad" ? "bad" : paper.tone === "good" ? "good" : paper.tone === "active" ? "active" : "calm"}
            label="Paper run"
            meta={stale && asOf ? `As of ${asOf}` : paper.meta}
            line={paper.state}
            detail={paper.detail}
            href="/lab"
            link="How a rule gets here"
          />
        }
      />

      {stale && (
        <p role="status" className="-mt-2 text-[13px] leading-5 text-warn">
          {offline ? "Dashboard server unreachable." : `${engine.label}.`}{" "}
          <span className="text-muted-foreground">
            Every figure on this page is from {asOf ? `the last write at ${asOf}` : "the last write"} and may be out of date.
          </span>
        </p>
      )}

      {live.strategies.length === 0 ? (
        <Panel>
          <Empty title="Nothing on paper">
            The trader runs whatever is in <code className="font-mono">strategies/</code>. Add a rule with{" "}
            <code className="font-mono">lab promote</code>, then restart the engine.
          </Empty>
        </Panel>
      ) : (
        <Runs strategies={live.strategies} stale={stale} />
      )}

      <PositionsPanel
        empty={
          <Empty title="No positions and no closed trades">
            {live.strategies.length
              ? `Strategies re-plan at each bar close (${[...new Set(live.strategies.map((s) => s.card.interval))].join(", ")}) and enter at the next open.`
              : "Nothing is trading."}
          </Empty>
        }
      />

      <Panel flush title="What this page cannot tell you">
        <ul className="flex flex-col gap-2 border-t px-5 py-4 text-[13px] leading-5 text-muted-foreground">
          <li><span className="text-foreground">Paper is not live.</span> Fills, slippage and funding are modelled.</li>
          <li><span className="text-foreground">The card is a forecast.</span> It was fixed on entry, before any paper day.</li>
          <li><span className="text-foreground">The test is slow by design.</span> Edges this small take hundreds to thousands of days to tell from luck.</li>
          <li><span className="text-foreground">A graduate is not an order.</span> Going live stays a human decision, made in the terminal.</li>
        </ul>
        <GlossList ids={["sequential-test", "card", "paper-day", "drawdown", "parity", "tag", "bps"]} />
      </Panel>

      <StationIndex stations={stations(live)} current="paper" />
    </div>
  );
}

