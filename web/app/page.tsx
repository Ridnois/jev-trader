"use client";

import Link from "next/link";
import { useEffect, useSyncExternalStore } from "react";
import { ArrowRight } from "lucide-react";
import { useLive } from "@/components/live";
import { DataTable, Delta, Empty, Figures, Panel, Side, Tag, Td, Th } from "@/components/bits";
import { StationRail, type RailItem } from "@/components/pipeline";
import { Sparkline, TestGauge } from "@/components/charts";
import { Gloss } from "@/components/gloss";
import { ago, duration, price, signed, strategyLabel, usd } from "@/lib/format";
import { notice, stations, totals } from "@/lib/v3/stations";

const VISIT_KEY = "jev-v3-last-visit";
const SESSION_KEY = "jev-v3-visit-since";

/** localStorage is an external store; read it the way React asks, not in an effect. */
const subscribeVisit = () => () => {};
function readVisit(): number | null {
  const seen = window.sessionStorage.getItem(SESSION_KEY);
  if (seen !== null) return seen ? Number(seen) : null;
  const prev = Number(window.localStorage.getItem(VISIT_KEY) || 0);
  return prev > 0 ? prev : null;
}
function useLastVisit() {
  const last = useSyncExternalStore(subscribeVisit, readVisit, () => undefined);
  useEffect(() => {
    if (window.sessionStorage.getItem(SESSION_KEY) !== null) return;
    const prev = window.localStorage.getItem(VISIT_KEY) || "";
    window.sessionStorage.setItem(SESSION_KEY, prev);
    window.localStorage.setItem(VISIT_KEY, String(Date.now()));
  }, []);
  return last;
}

/** Where the test stands between its two fixed lines, in one glance. */

function SinceLastVisit() {
  const { live } = useLive();
  const last = useLastVisit();

  if (last === undefined)
    return (
      <div className="grid grid-cols-3 gap-3" aria-hidden>
        {[0, 1, 2].map((i) => (
          <div key={i}>
            <div className="h-3 w-16 rounded bg-muted" />
            <div className="mt-2 h-5 w-10 rounded bg-muted" />
          </div>
        ))}
      </div>
    );

  if (last === null)
    return (
      <div className="flex flex-col gap-2">
        <p className="text-[13px] leading-5 text-muted-foreground">First visit in this browser.</p>
        <Link href="/start" className="flex items-center gap-1 text-[12px] text-foreground underline decoration-border underline-offset-4 hover:decoration-foreground">
          How the numbers fit <ArrowRight className="size-3" />
        </Link>
      </div>
    );

  const candidates = live.candidates.filter((c) => Date.parse(c.ts) > last);
  const triggered = candidates.filter((c) => c.triggered);
  const trades = live.trades.filter((t) => t.closed_ms > last);
  const more = candidates.length >= 200;

  return (
    <div>
      <dl className="grid grid-cols-3 gap-3">
        <div>
          <dt className="text-[11px] text-muted-foreground">Headlines</dt>
          <dd className="font-mono text-[18px] tabular-nums">{more ? "200+" : candidates.length}</dd>
        </div>
        <div>
          <dt className="text-[11px] text-muted-foreground">Triggered</dt>
          <dd className="font-mono text-[18px] tabular-nums">{triggered.length}</dd>
        </div>
        <div>
          <dt className="text-[11px] text-muted-foreground">Trades</dt>
          <dd className="font-mono text-[18px] tabular-nums">{trades.length}</dd>
        </div>
      </dl>
      <p className="mt-3 text-[11px] leading-4 text-faint">Since {duration((live.now - last) / 1000)} ago · newest 200 headlines, 50 trades</p>
    </div>
  );
}

export default function Now() {
  const { live } = useLive();
  const n = notice(live);
  const pipe = stations(live);
  const t = totals(live);
  const headlines = live.candidates.slice(0, 4);
  const markets = live.quotes.slice(0, 6);
  const days = live.strategies.reduce((a, s) => a + s.fullDays, 0);
  const byId = new Map(pipe.map((s) => [s.id, s]));
  const lab = live.lab;
  const q = lab.questions;
  const rail: RailItem[] = [
    {
      id: "lab",
      name: "Lab",
      href: "/lab",
      tone: byId.get("lab")?.tone ?? "neutral",
      value: lab.families ? `${lab.promoted}/${lab.families}` : "–",
      unit: "families passed",
      note: lab.lastJudged ? `Judged ${new Date(lab.lastJudged).toLocaleDateString("en-GB", { day: "numeric", month: "short" })}` : "Never judged",
      ratio: lab.families ? { value: lab.promoted, max: lab.families, met: lab.promoted > 0 } : undefined,
    },
    {
      id: "paper",
      name: "Paper run",
      href: "/paper",
      tone: byId.get("paper")?.tone ?? "neutral",
      value: `${live.strategies.length}`,
      unit: "strategies on paper",
      note: days > 0 ? `${days} full paper ${days === 1 ? "day" : "days"}` : "No full paper day yet",
    },
    {
      id: "news",
      name: "News",
      href: "/catalysts",
      tone: byId.get("news")?.tone ?? "neutral",
      value: `${live.markoutDays}/20`,
      unit: "evidence days",
      note: `${live.triggeredCount} triggered headlines`,
      ratio: { value: live.markoutDays, max: 20, met: live.markoutDays >= 20 },
    },
    {
      id: "jev",
      name: "Jev tests",
      href: "/probes",
      tone: byId.get("jev")?.tone ?? "neutral",
      value: q ? `${lab.acceptedQuestions}/${q}` : "–",
      unit: "questions show skill",
      note: q ? (lab.acceptedQuestions > 0 ? `${lab.acceptedQuestions} show skill` : "No skill yet") : "Not tested yet",
      ratio: q ? { value: lab.acceptedQuestions, max: q, met: lab.acceptedQuestions > 0 } : undefined,
    },
  ];
  return (
    <div className="flex flex-col gap-6">
      <header className="flex flex-col gap-1 border-b pb-5">
        <h2 className="text-[20px] leading-7 font-medium tracking-tight">{n.kind === "calm" ? "Nothing needs you." : n.title}</h2>
        {n.kind !== "calm" && <p className="max-w-[70ch] text-[13px] leading-5 text-muted-foreground">{n.detail}</p>}
      </header>

      <StationRail items={rail} />

      <section className="grid grid-cols-1 gap-4 lg:grid-cols-3">
        <Panel
          className="lg:col-span-2"
          flush
          title="Paper book"
          action={
            <Link href="/paper" className="flex items-center gap-1 text-[13px] text-muted-foreground hover:text-foreground">
              Paper run <ArrowRight className="size-3.5" />
            </Link>
          }
        >
          {live.strategies.length === 0 ? (
            <p className="border-t px-5 py-4 text-[13px] leading-5 text-muted-foreground">
              Nothing on paper. <Link href="/lab" className="text-foreground underline decoration-border underline-offset-4">See the Lab verdicts</Link>.
            </p>
          ) : (
            <ul className="border-t">
              {live.strategies.map((s) => (
                <li key={s.name} className="grid gap-3 border-b px-5 py-3 last:border-b-0 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <Link href="/paper" className="text-[13px] font-medium text-foreground underline-offset-4 hover:underline">
                        {strategyLabel(s.name, s.card)}
                      </Link>
                      <Tag kind={s.status} />
                      <Tag kind={s.call} />
                    </div>
                    <div className="mt-2">
                      <TestGauge s={s} />
                    </div>
                  </div>
                  <div className="flex items-center gap-4 text-[11px] leading-4 text-muted-foreground sm:flex-col sm:items-end sm:gap-0">
                    <div>
                      <span className="font-mono text-[15px] tabular-nums text-foreground">{s.fullDays}</span> full {s.fullDays === 1 ? "day" : "days"}
                    </div>
                    {s.partialDays > 0 && <div>{s.partialDays} partial</div>}
                  </div>
                </li>
              ))}
            </ul>
          )}
        </Panel>

        <Panel title="Since you last look">
          <SinceLastVisit />
        </Panel>
      </section>

      <Figures
        items={[
          {
            label: "Paper equity",
            value: t.capital ? usd(t.capital + t.change, 2) : "–",
            sub: !t.capital ? "No paper book yet" : (
              <>
                <span className={t.change > 0.005 ? "text-pos" : t.change < -0.005 ? "text-neg" : undefined}>
                  {t.change > 0.005 ? "+" : ""}{usd(t.change, 2)}
                </span>{" "}
                since start
              </>
            ),
          },
          { label: "Open positions", value: t.positions.length, sub: t.positions.length ? `${usd(t.atRisk, 0)} at risk before stops` : "Nothing exposed" },
          t.positions.length > 0
            ? { label: "At risk", value: usd(t.atRisk, 0), sub: "Notional before stops, paper money" }
            : { label: "Headlines scored", value: live.candidateCount.toLocaleString("en-US"), sub: "Read by Jev; none traded" },
        ]}
      />

      <section className="grid grid-cols-1 gap-4 lg:grid-cols-3">
        <Panel
          flush
          className="lg:col-span-2"
          title="Latest headlines"
          action={
            <Link href="/catalysts" className="flex items-center gap-1 text-[13px] text-muted-foreground hover:text-foreground">
              All {live.candidateCount} headlines <ArrowRight className="size-3.5" />
            </Link>
          }
        >
          {headlines.length === 0 ? (
            <Empty title="No headlines scored yet">
              Jev reads every headline about a tracked coin as it arrives. The first ones appear within a few minutes of the engine starting.
            </Empty>
          ) : (
            <ul className="border-t">
              {headlines.map((c) => (
                <li key={c.id + c.ts} className="border-b px-5 py-3 last:border-b-0">
                  <div className="line-clamp-2 text-[13px] leading-5">{c.title}</div>
                  <div className="mt-1 flex flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
                    <span className="font-medium text-foreground">{c.coin}</span>
                    <Side side={c.side} />
                    <span><Gloss id="impact">impact</Gloss> <span className="tabular-nums text-muted-foreground">{signed(c.impact)}</span></span>
                    {c.triggered && <Tag kind="triggered">Triggered</Tag>}
                    <span className="ml-auto text-faint">{ago(c.ts, live.now)}</span>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </Panel>

        <Panel
          flush
          title="Prices"
          action={
            live.quotes.length > 0 ? (
              <Link href="/markets" className="flex items-center gap-1 text-[13px] text-muted-foreground hover:text-foreground">
                All {live.quotes.length} coins <ArrowRight className="size-3.5" />
              </Link>
            ) : undefined
          }
        >
          {markets.length === 0 ? (
            <Empty title="No quotes recorded">The recorder writes one row per coin per second while the engine runs.</Empty>
          ) : (
            <table className="w-full text-[13px]">
              <thead>
                <tr className="text-[11px] text-faint">
                  <th className="py-1.5 pl-5 text-left font-normal">Coin</th>
                  <th className="text-center font-normal">3 h</th>
                  <th className="text-right font-normal">Price</th>
                  <th className="w-[76px] py-1.5 pr-5 text-right font-normal">1 h</th>
                </tr>
              </thead>
              <tbody>
                {markets.map((q) => (
                  <tr key={q.coin} className="border-t">
                    <td className="py-2 pl-5 font-medium">{q.coin}</td>
                    <td className="px-2 py-2"><Sparkline values={q.spark} width={84} height={24} up={null} /></td>
                    <td className="py-2 text-right">{price(q.mid)}</td>
                    <td className="w-[76px] py-2 pr-5 text-right tabular-nums text-muted-foreground">{q.chg1h === null ? "–" : signed(q.chg1h, 2, "%")}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Panel>
      </section>

      {(t.positions.length > 0 || live.trades.length > 0) && (
        <Panel
          flush
          title="Open positions and closed trades"
          description={t.positions.length ? `${t.positions.length} open, ${usd(t.atRisk)} notional at risk before stops.` : undefined}
        >
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
                    <Td className="text-right">{usd(p.notional)}</Td>
                    <Td className="text-right">{price(p.entry_px)}</Td>
                    <Td className="text-right">{p.stop_px ? price(p.stop_px) : "None"}</Td>
                    <Td className="text-muted-foreground">{ago(p.opened_ms, live.now)}</Td>
                    <Td className="text-muted-foreground">{new Date(p.deadline_ms).toISOString().slice(5, 16).replace("T", " ")} UTC</Td>
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
                  <tr><Th>Coin</Th><Th>Side</Th><Th className="text-right">Result</Th><Th className="text-right">After costs</Th><Th>Exit</Th><Th>Closed</Th></tr>
                </thead>
                <tbody>
                  {live.trades.slice(0, 8).map((tr) => (
                    <tr key={tr.coin + tr.closed_ms}>
                      <Td className="font-medium">{tr.coin}</Td>
                      <Td><Side side={tr.side} /></Td>
                      <Td className="text-right"><Delta value={tr.pnl_bps} digits={1} unit=" bps" /></Td>
                      <Td className="text-right">{usd(tr.pnl, 2)}</Td>
                      <Td className="text-muted-foreground">{tr.reason}</Td>
                      <Td className="text-muted-foreground">{new Date(tr.closed_ms).toISOString().slice(5, 16).replace("T", " ")} UTC</Td>
                    </tr>
                  ))}
                </tbody>
              </DataTable>
            </div>
          )}
        </Panel>
      )}
    </div>
  );
}
