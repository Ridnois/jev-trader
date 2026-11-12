"use client";

import Link from "next/link";
import { useState } from "react";
import { ChevronRight, Lock } from "lucide-react";
import { useLive } from "@/components/live";
import { DataTable, Delta, Empty, PageHead, Panel, Side, Verdict, Tag, Td, Th } from "@/components/bits";
import { SequentialChart } from "@/components/charts";
import { StationIndex } from "@/components/pipeline";
import { Gloss, GlossList } from "@/components/gloss";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ago, plural, price, signed, specSentence, strategyLabel, usd } from "@/lib/format";
import { cn } from "@/lib/utils";
import { engineState, stations, totals } from "@/lib/v3/stations";
import type { StrategyView } from "@/lib/v3/types";

const n = (x: number) => Math.round(x).toLocaleString("en-US");
const years = (d: number) => (d / 365).toFixed(1);
const utcTime = (ts: string | number) => new Date(ts).toISOString().slice(11, 16);

/** The days the test expects to need, as a range, from the card. */
function horizon(s: StrategyView) {
  const lo = Math.min(s.card.days_if_none, s.card.days_if_real);
  const hi = Math.max(s.card.days_if_none, s.card.days_if_real);
  const same = Math.round(lo) === Math.round(hi);
  return {
    days: same ? `about ${n(lo)}` : `${n(lo)} to ${n(hi)}`,
    short: same ? n(lo) : `${n(lo)}–${n(hi)}`,
    years: same ? `about ${years(lo)}` : `${years(lo)} to ${years(hi)}`,
    yr: same ? years(lo) : `${years(lo)}–${years(hi)}`,
  };
}

/** Settings beyond the lookback, which the strategy's name already carries. */
function settings(s: StrategyView): string {
  return specSentence(s.card).split(", ").filter((b) => !b.endsWith("lookback")).join(" · ");
}

function drawdownWarn(s: StrategyView): boolean {
  return s.drawdownLimit > 0 && s.drawdown / s.drawdownLimit > 0.66;
}

/** One row per strategy: the numbers that decide the run, side by side. */
function Standing({ strategies, stale }: { strategies: StrategyView[]; stale: boolean }) {
  return (
    <Panel
      flush
      title="Where each run stands"
      description="The test value against its fixed lines decides the run. Drawdown past its band kills it early."
    >
      <DataTable minWidth={720} className={cn(stale && "text-muted-foreground")}>
        <thead>
          <tr>
            <Th>Strategy</Th>
            <Th className="text-right"><Gloss id="sequential-test">Test now</Gloss></Th>
            <Th className="text-right">Kill · graduate</Th>
            <Th className="text-right"><Gloss id="paper-day">Full paper days</Gloss></Th>
            <Th className="text-right"><Gloss id="drawdown">Drawdown</Gloss> of band</Th>
            <Th className="text-right">Decision expected</Th>
          </tr>
        </thead>
        <tbody>
          {strategies.map((s) => {
            const h = horizon(s);
            return (
              <tr key={s.name}>
                <Td>
                  <a href={`#${s.name}`} className="font-medium text-foreground underline decoration-border underline-offset-4 hover:decoration-foreground">
                    {strategyLabel(s.name, s.card)}
                  </a>
                </Td>
                <Td className="text-right font-medium tabular-nums">{signed(s.llr)}</Td>
                <Td className="text-right text-muted-foreground tabular-nums">{signed(s.kill)} · {signed(s.graduate)}</Td>
                <Td className="text-right tabular-nums">
                  {n(s.fullDays)}
                  {s.partialDays > 0 && <span className="text-muted-foreground"> + {n(s.partialDays)} partial</span>}
                </Td>
                <Td className={cn("text-right tabular-nums", drawdownWarn(s) && "text-warn")}>
                  {(s.drawdown * 100).toFixed(2)}% <span className="text-muted-foreground">of {(s.drawdownLimit * 100).toFixed(1)}%</span>
                </Td>
                <Td className="text-right tabular-nums">
                  {h.short} days <span className="text-muted-foreground">· {h.yr} yr</span>
                </Td>
              </tr>
            );
          })}
        </tbody>
      </DataTable>
    </Panel>
  );
}

function StrategyCard({ s }: { s: StrategyView }) {
  const [zoom, setZoom] = useState(false);
  const days = s.fullDays;
  const h = horizon(s);
  const rest = settings(s);
  const verdict = s.labVerdict === "REJECT"
    ? `The Lab rejected this family${s.labStoppedAt ? ` at the ${s.labStoppedAt}` : ""}. It runs on paper anyway, as an unproven candidate: watched, not trusted.`
    : s.labVerdict === "PROMOTE_TO_PAPER"
      ? "The Lab promoted this rule. The paper run decides whether the live market agrees."
      : "The Lab has not promoted this rule: watched, not trusted.";
  const mu = s.card.mu_daily * 1e4;
  const sd = s.card.sd_daily * 1e4;

  return (
    <Panel
      flush
      title={
        <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
          {strategyLabel(s.name, s.card)}
          {rest && <span className="font-normal text-muted-foreground">· {rest}</span>}
          <Tag kind={s.status} />
          <Tag kind={s.call} />
        </span>
      }
      description={
        <>
          {verdict}{" "}
          <Link href="/lab" className="text-foreground underline decoration-border underline-offset-2 hover:decoration-foreground">Read the gate</Link>
        </>
      }
      action={
        <Tabs value={zoom ? "sofar" : "whole"} onValueChange={(v) => setZoom(v === "sofar")}>
          <TabsList className="h-8">
            <TabsTrigger value="whole" className="text-xs">Whole test</TabsTrigger>
            <TabsTrigger value="sofar" className="text-xs">So far</TabsTrigger>
          </TabsList>
        </Tabs>
      }
    >
      <div className="flex flex-col gap-1.5 border-t px-5 py-4 text-[13px] leading-5">
        <p>
          <span className="font-medium">
            {days === 0 ? `Test at ${signed(s.llr)}, by construction.` : `After ${plural(days, "full paper day")} the test stands at ${signed(s.llr)}.`}
          </span>{" "}
          <span className="text-muted-foreground">
            {days === 0
              ? `Kill below ${signed(s.kill)}, graduate above ${signed(s.graduate)}, both fixed before the run.${s.partialDays > 0 ? ` ${plural(s.partialDays, "partial day")} left out.` : ""}`
              : s.callWhy}
          </span>
        </p>
        <p>
          <span className="font-medium">
            At this edge size the test expects to need {h.days} full paper days, {h.years} years.
          </span>{" "}
          <span className="text-muted-foreground">Earlier if there is no edge, later if the edge is real, and longer for every day the engine is down.</span>
        </p>
      </div>

      <SequentialChart s={s} zoom={zoom} />
      <p className="px-5 pt-1 pb-3 text-xs leading-4 text-muted-foreground">
        Vertical: the test value (<Gloss id="llr">log-likelihood ratio</Gloss>). Horizontal: full paper days. At day 0 nothing
        has been counted; later, 0 means the counted days balance.
      </p>

      <dl className="grid gap-x-8 gap-y-3 border-t px-5 py-4 text-[13px] leading-5 sm:grid-cols-2">
        <div>
          <dt className="text-muted-foreground">What the <Gloss id="card">card</Gloss> forecast</dt>
          <dd className="mt-0.5 text-pretty">
            {signed(mu, 1)} bps a day against a {sd.toFixed(0)} bps typical daily swing,{" "}
            {sd > 0 ? `${(Math.abs(mu) / sd).toFixed(2)} of one swing.` : "with no swing on file."}{" "}
            <span className="text-muted-foreground">Fixed on entry to the paper book. A forecast, not a result.</span>
          </dd>
        </div>
        <div>
          <dt className="text-muted-foreground"><Gloss id="parity">Parity with the simulator</Gloss></dt>
          <dd className="mt-0.5 text-pretty">{s.parity ?? "The monitor has not written a parity line yet."}</dd>
        </div>
      </dl>

      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 border-t px-5 py-3 text-xs text-muted-foreground">
        <span>Settings</span>
        <span className="font-mono">{s.book?.spec ?? `${s.card.spec.family}:${Object.entries(s.card.spec.params).map(([k, v]) => `${k}=${v}`).join(",")}`}</span>
        <span>{s.card.interval} bars</span>
        <span className="ml-auto">Hypothesis {s.hypothesis}</span>
      </div>
    </Panel>
  );
}

export default function Paper() {
  const { live, offline } = useLive();
  const paper = stations(live)[1];
  const t = totals(live);
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
            A rule from the Lab runs here against Hyperliquid&apos;s live prices with paper money, one book each. A family the
            Lab rejected can run here too, as an unproven candidate: the run tests that verdict, it does not overrule it. A
            test fixed before the run decides continue, kill or graduate. This page is read-only.{" "}
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
            No rule has been promoted or sent forward from the Lab. The trader runs whatever is in{" "}
            <code className="font-mono">strategies/</code>; a new rule is added with <code className="font-mono">lab promote</code>,
            then the engine is restarted. A rejected family can also run here as an unproven candidate.
          </Empty>
        </Panel>
      ) : (
        <>
          <Standing strategies={live.strategies} stale={stale} />
          {live.strategies.map((s) => (
            <div key={s.name} id={s.name} className={cn("scroll-mt-24", stale && "opacity-80")}>
              <StrategyCard s={s} />
            </div>
          ))}
        </>
      )}

      <Panel
        flush
        title="Positions and closed trades"
        description={t.positions.length ? `${t.positions.length} open, ${usd(t.atRisk)} notional at risk before stops.` : "The paper book. Nothing here reaches a real venue."}
      >
        {t.positions.length === 0 && live.trades.length === 0 ? (
          <Empty title="No positions and no closed trades">
            {live.strategies.length
              ? `${new Set(live.strategies.map((s) => s.card.interval)).size === 1
                  ? `${live.strategies.length === 1 ? "The strategy re-plans" : "Every strategy re-plans"} at each ${live.strategies[0].card.interval} bar close`
                  : `Each strategy re-plans at its own bar close (${live.strategies.map((s) => `${s.card.interval} for ${strategyLabel(s.name, s.card)}`).join("; ")})`} and enters at the next open. Entries appear here with their stop and time limit.`
              : "Nothing is trading, so there is nothing to show."}
          </Empty>
        ) : (
          <>
            {t.positions.length > 0 && (
              <DataTable minWidth={620}>
                <thead>
                  <tr><Th>Coin</Th><Th>Side</Th><Th className="text-right">Size</Th><Th className="text-right">Entry</Th><Th className="text-right">Stop</Th><Th>Opened</Th><Th>Closes by</Th></tr>
                </thead>
                <tbody>
                  {t.positions.map((p) => (
                    <tr key={p.strategy + p.coin}>
                      <Td className="font-medium">{p.coin}</Td>
                      <Td><Side side={p.side} /></Td>
                      <Td className="text-right tabular-nums">{usd(p.notional)}</Td>
                      <Td className="text-right tabular-nums">{price(p.entry_px)}</Td>
                      <Td className="text-right tabular-nums">{p.stop_px ? price(p.stop_px) : "None"}</Td>
                      <Td className="text-muted-foreground">{ago(p.opened_ms, live.now)}</Td>
                      <Td className="text-muted-foreground tabular-nums">{new Date(p.deadline_ms).toISOString().slice(5, 16).replace("T", " ")} UTC</Td>
                    </tr>
                  ))}
                </tbody>
              </DataTable>
            )}
            {live.trades.length > 0 && (
              <div className="border-t">
                <div className="px-5 pt-4 pb-2 text-sm font-medium">Closed round trips</div>
                <DataTable minWidth={560}>
                  <thead>
                    <tr><Th>Coin</Th><Th>Side</Th><Th className="text-right">Result</Th><Th className="text-right">After costs, USD</Th><Th>Exit</Th><Th>Closed</Th></tr>
                  </thead>
                  <tbody>
                    {live.trades.map((tr) => (
                      <tr key={tr.coin + tr.closed_ms}>
                        <Td className="font-medium">{tr.coin}</Td>
                        <Td><Side side={tr.side} /></Td>
                        <Td className="text-right tabular-nums"><Delta value={tr.pnl_bps} digits={1} unit=" bps" /></Td>
                        <Td className="text-right tabular-nums">{usd(tr.pnl, 2)}</Td>
                        <Td className="text-muted-foreground">{tr.reason}</Td>
                        <Td className="text-muted-foreground tabular-nums">{new Date(tr.closed_ms).toISOString().slice(5, 16).replace("T", " ")} UTC</Td>
                      </tr>
                    ))}
                  </tbody>
                </DataTable>
              </div>
            )}
          </>
        )}
      </Panel>

      <Panel
        flush
        title="What this page cannot tell you"
        description="The limits are fixed and worth knowing before reading the line."
      >
        <ul className="flex flex-col gap-3 border-t px-5 py-4 text-[13px] leading-5 text-muted-foreground">
          <li><span className="text-foreground">Paper is not live.</span> Fills, slippage and funding are modelled. A paper result is evidence, not proof.</li>
          <li><span className="text-foreground">The card is a forecast, not a result.</span> It was fixed on entry to the paper book, including the days the test expects to need.</li>
          <li><span className="text-foreground">The test is slow by design.</span> Edges this small take hundreds to thousands of full paper days to tell from luck. Each strategy states its own count above.</li>
          <li><span className="text-foreground">A graduate is not an instruction.</span> It is the precondition for arming a real book, and arming stays a human decision.</li>
        </ul>
        <details className="group border-t">
          <summary className="flex cursor-pointer list-none items-center gap-2 px-5 py-3 text-[13px] leading-5 font-medium [&::-webkit-details-marker]:hidden">
            <ChevronRight aria-hidden className="size-3.5 text-muted-foreground transition-transform group-open:rotate-90" />
            The three outcomes the test can give
          </summary>
          <dl className="grid border-t sm:grid-cols-3 sm:divide-x">
            <div className="px-5 py-4">
              <dt><Tag kind="continue" /></dt>
              <dd className="mt-2 text-[13px] leading-5 text-pretty text-muted-foreground">Neither line crossed. The run has not produced enough evidence, or the evidence is still ambiguous.</dd>
            </div>
            <div className="border-t px-5 py-4 sm:border-t-0">
              <dt><Tag kind="kill" /></dt>
              <dd className="mt-2 text-[13px] leading-5 text-pretty text-muted-foreground">The test now favours no edge, or drawdown passed the band the backtest allows. Stop watching this rule.</dd>
            </div>
            <div className="border-t px-5 py-4 sm:border-t-0">
              <dt><Tag kind="graduate" /></dt>
              <dd className="mt-2 text-[13px] leading-5 text-pretty text-muted-foreground">The paper run supports the predicted edge. Going live is still a human decision, made in the terminal.</dd>
            </div>
          </dl>
          <p className="border-t px-5 py-3 text-xs text-muted-foreground">None of them places an order.</p>
        </details>
        <GlossList
          ids={["card", "paper-day", "sequential-test", "drawdown", "parity", "strategy", "bps"]}
          intro="The terms this page uses. Hover any underlined word for the same record."
        />
      </Panel>

      <p className="text-[13px] text-muted-foreground">
        <Lock aria-hidden className="mr-1 inline size-3.5 align-[-2px]" />
        Read-only. v3 places no real orders.
      </p>

      <StationIndex stations={stations(live)} current="paper" />
    </div>
  );
}
