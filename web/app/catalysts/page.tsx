"use client";

import Link from "next/link";
import { useState } from "react";
import { useLive } from "@/components/live";
import { DataTable, Delta, Empty, Figures, Mark, PageHead, Panel, Side, Verdict, Tag, Td, Th } from "@/components/bits";
import { Gloss, GlossList } from "@/components/gloss";
import { StationIndex } from "@/components/pipeline";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { clock, horizon, plural, signed } from "@/lib/format";
import { stations } from "@/lib/v3/stations";
import type { Candidate } from "@/lib/v3/types";

const PAGE = 50;

const COUNT = (n: number) => <span aria-hidden className="ml-1 text-faint tabular-nums">{n}</span>;

function Headline({ id, title, source, meta }: { id: string; title: string; source: string; meta?: React.ReactNode }) {
  const isLink = id.startsWith("http");
  return (
    <div className="max-w-[460px] min-w-[220px]">
      {meta && <div className="mb-0.5 flex items-center gap-2 text-xs sm:hidden">{meta}</div>}
      {isLink ? (
        <a href={id} target="_blank" rel="noreferrer" title={title} className="line-clamp-2 leading-5 hover:underline">{title}</a>
      ) : (
        <span className="line-clamp-2 leading-5" title={title}>{title}</span>
      )}
      <div className="text-xs text-faint">{source}</div>
    </div>
  );
}

function Gates({ c }: { c: Candidate }) {
  return (
    <div className="flex gap-1">
      {c.gates.map((g) => (
        <Tooltip key={g.gate}>
          <TooltipTrigger asChild>
            <button type="button" className="rounded-full" aria-label={`${g.gate}: ${g.passed ? "passed" : "not met"}, ${g.detail}`}>
              <Mark pass={g.passed} />
            </button>
          </TooltipTrigger>
          <TooltipContent>
            <span className="font-medium capitalize">{g.gate}</span>: {g.detail}
          </TooltipContent>
        </Tooltip>
      ))}
    </div>
  );
}

/** A markout still to come, or one lost when the engine stopped first. */
function Markout({ c, secs, now, enough }: { c: Candidate; secs: number; now: number; enough: boolean }) {
  const m = c.markouts[String(secs)];
  if (m)
    return enough ? (
      <Delta value={m.net_bps} digits={0} unit=" bps" />
    ) : (
      <span className="tabular-nums text-muted-foreground">{signed(m.net_bps, 0, " bps")}</span>
    );
  const due = Date.parse(c.ts) + secs * 1000;
  if (due > now) return <span className="text-faint" title={`Due in ${Math.max(1, Math.ceil((due - now) / 60_000))} min`}>–</span>;
  return <span className="text-faint" title="The engine was down when this markout was due, so no price was recorded">no data</span>;
}

export default function Catalysts() {
  const { live } = useLive();
  const [tab, setTab] = useState("coin");
  const [shown, setShown] = useState(PAGE);
  const cfg = live.engine.catalystCfg;
  const rows = tab === "triggered" ? live.candidates.filter((c) => c.triggered) : live.candidates;
  const visible = rows.slice(0, shown);
  const waiting = live.engine.status?.catalyst.candidates_waiting_for_markouts ?? 0;
  const anyMarkouts = live.horizons.some((h) => h.allN > 0);
  const triggers = live.candidates.filter((c) => c.triggered).length;
  const enough = live.markoutDays >= 20 && live.triggeredCount >= 30;
  const loaded = live.candidates.length;

  return (
    <div className="flex flex-col gap-6">
      <PageHead
        question="Is there money in the news?"
        lede={
          <>
            A catalyst is a headline with enough new fact in it to move a price. <Gloss id="jev">Jev</Gloss> scores every one about a tracked coin for{" "}
            <Gloss id="impact">impact</Gloss> and <Gloss id="new-info">new information</Gloss>, and the recorder measures what
            the price did 5 min, 30 min, 2 h and 6 h later (a <Gloss id="markout">markout</Gloss>), net of the round trip and in{" "}
            <Gloss id="bps">bps</Gloss>. Nothing is traded and nothing is replayed from history, because hindsight would leak
            in.{" "}
            <Link href="/lab" className="text-foreground underline decoration-border underline-offset-4 hover:decoration-foreground">
              A pattern that clears the bar becomes a family for the Lab.
            </Link>
          </>
        }
        verdict={
          <Verdict
            rule={false}
            kind="calm"
            label="News recorder"
            meta={live.lastScoredAt ? `Last headline ${clock(live.lastScoredAt)} UTC` : "No headline yet"}
            line={
              enough
                ? `${plural(live.markoutDays, "evidence day")} · ${plural(live.triggeredCount, "triggered headline")}`
                : `${live.markoutDays} of 20 evidence days · ${live.triggeredCount} of 30 triggered headlines`
            }
            detail={
              enough
                ? `${live.candidateCount.toLocaleString("en-US")} headlines scored across ${plural(live.newsDays, "day")}. Compare the triggered column with every headline: if they are equal, the checks are not sorting anything.`
                : `${plural(live.markoutDays, "evidence day")} with a markout and ${live.triggeredCount} triggered headlines. A first read needs 20 evidence days and 30 triggered headlines; until both are met the averages are an anecdote and the table marks them provisional.`
            }
            href="/start"
            link="Where this fits"
          />
        }
      />

      <Figures
        items={[
          { label: "Headlines scored", value: live.candidateCount.toLocaleString("en-US"), sub: `Scored across ${plural(live.newsDays, "calendar day")}` },
          { label: "Triggered", value: live.triggeredCount, sub: "Passed impact, new info and freshness", ratio: { value: live.triggeredCount, max: 30, met: live.triggeredCount >= 30 } },
          { label: "Headlines awaiting last markout", value: waiting, sub: "Scored headlines whose 6 h price reading has not landed" },
          { label: "Evidence days", value: `${live.markoutDays}`, sub: "Days with at least one price reading; 20 makes the averages readable", tone: enough ? "pos" : undefined, ratio: { value: live.markoutDays, max: 20, met: live.markoutDays >= 20 } },
        ]}
      />

      <Panel
        flush
        title={
          <span className="flex flex-wrap items-center gap-2">
            Would following Jev&apos;s read have paid?
            {!enough && <Tag kind="proposed">Provisional</Tag>}
          </span>
        }
        description="Mean return after costs, for triggered headlines and for every headline with a markout at that horizon. A good result is positive and clearly better than the all-headlines column; a bad one is negative, or no different."
      >
        {!anyMarkouts ? (
          <Empty title="No markouts yet">
            Each scored headline is marked out {live.horizons.map((h) => horizon(h.secs)).join(", ")} after it arrives. {waiting} are waiting.
          </Empty>
        ) : (
          <>
            {!enough && (
              <p className="border-t px-5 py-3 text-[13px] leading-5 text-muted-foreground">
                <span className="text-foreground">Provisional.</span> With {plural(live.markoutDays, "evidence day")} and {live.triggeredCount} triggered headlines,
                a mean can rest on a handful of headlines. The averages and every markout are shown without colour until 20 evidence days and 30 triggered headlines, so none of them reads as a finding.
              </p>
            )}
            <DataTable minWidth={560}>
              <thead>
                <tr>
                  <Th>After</Th>
                  <Th className="text-right">Triggered with a markout, net bps</Th>
                  <Th className="text-right">n markouts</Th>
                  <Th className="text-right">All headlines with a markout, net bps</Th>
                  <Th className="text-right">n markouts</Th>
                </tr>
              </thead>
              <tbody>
                {live.horizons.map((h) => (
                  <tr key={h.secs}>
                    <Td className="font-medium">{horizon(h.secs)}</Td>
                    <Td className={`text-right ${h.triggeredN < 10 ? "text-faint" : ""}`}>
                      {h.triggeredN === 0 || h.triggeredMean === null ? (
                        <span className="text-faint">None yet</span>
                      ) : enough ? (
                        <Delta value={h.triggeredMean} digits={1} unit=" bps" />
                      ) : (
                        <span className="tabular-nums text-muted-foreground">{signed(h.triggeredMean, 1, " bps")}</span>
                      )}
                    </Td>
                    <Td className="text-right text-muted-foreground">{h.triggeredN}</Td>
                    <Td className={`text-right ${h.allN < 10 ? "text-faint" : ""}`}>
                      {h.allN === 0 || h.allMean === null ? (
                        <span className="text-faint">Waiting</span>
                      ) : enough ? (
                        <Delta value={h.allMean} digits={1} unit=" bps" />
                      ) : (
                        <span className="tabular-nums text-muted-foreground">{signed(h.allMean, 1, " bps")}</span>
                      )}
                    </Td>
                    <Td className="text-right text-muted-foreground">{h.allN}</Td>
                  </tr>
                ))}
              </tbody>
            </DataTable>
            <p className="max-w-[62ch] border-t px-5 py-3 text-[13px] leading-5 text-muted-foreground">
              {plural(live.markoutDays, "day")} of evidence, {waiting} headlines still waiting. Below 30 triggered headlines
              across 20 days this is an anecdote, not a result.
            </p>
          </>
        )}
      </Panel>

      <Panel
        flush
        title="Scored headlines"
        description={
          tab === "all"
            ? "Every headline Jev read, including market-wide news that names no coin. Newest first."
            : tab === "triggered"
              ? "Headlines that passed all three checks. Strong, new and recent enough at once is rare: expect a few a week."
              : "Headlines that name a tracked coin, newest first, with what Jev read and what the price did after."
        }
        action={
          <Tabs value={tab} onValueChange={(v) => { setTab(v); setShown(PAGE); }}>
            <TabsList className="h-8">
              <TabsTrigger value="coin" className="text-xs" aria-label={`About a coin, ${loaded} in view`}>About a coin{COUNT(loaded)}</TabsTrigger>
              <TabsTrigger value="triggered" className="text-xs" aria-label={`Triggered, ${triggers} in view`}>Triggered{COUNT(triggers)}</TabsTrigger>
              <TabsTrigger value="all" className="text-xs" aria-label={`All headlines, ${live.news.length} in view`}>All headlines{COUNT(live.news.length)}</TabsTrigger>
            </TabsList>
          </Tabs>
        }
      >
        {tab === "all" ? (
          live.news.length === 0 ? (
            <Empty title="No headlines read yet">The engine polls 18 feeds and 4 exchange announcement pages. Headlines appear here once Jev has scored them.</Empty>
          ) : (
            <DataTable minWidth={520}>
              <thead>
                <tr><Th className="hidden sm:table-cell">Seen, UTC</Th><Th>Headline</Th><Th className="hidden sm:table-cell">About</Th><Th className="text-right">Impact</Th><Th className="text-right">New info</Th><Th className="text-right">Late by</Th></tr>
              </thead>
              <tbody>
                {live.news.slice(0, shown).map((n) => (
                  <tr key={n.id + n.first_seen}>
                    <Td className="hidden sm:table-cell whitespace-nowrap text-muted-foreground">{clock(n.first_seen)}</Td>
                    <Td>
                      <Headline id={n.id} title={n.title} source={n.source} meta={<><span className="font-medium">{n.coins.length ? n.coins.join(", ") : "Market"}</span><span className="text-faint">{clock(n.first_seen)} UTC</span></>} />
                    </Td>
                    <Td className="hidden sm:table-cell whitespace-nowrap">{n.coins.length ? n.coins.join(", ") : <span className="text-muted-foreground">Market</span>}</Td>
                    <Td className="text-right tabular-nums text-muted-foreground">{n.impact === null ? "–" : signed(n.impact)}</Td>
                    <Td className="text-right">{n.new_info === null ? "–" : n.new_info.toFixed(2)}</Td>
                    <Td className="text-right whitespace-nowrap text-muted-foreground">{n.lag_min.toFixed(0)} min</Td>
                  </tr>
                ))}
              </tbody>
            </DataTable>
          )
        ) : rows.length === 0 ? (
          <Empty title={tab === "triggered" ? "Nothing has triggered yet" : "No headlines scored yet"}>
            {tab === "triggered"
              ? "No headline has cleared all three checks yet."
              : "Jev scores each headline about a tracked coin as it arrives."}
          </Empty>
        ) : (
          <DataTable minWidth={820}>
            <thead>
              <tr>
                <Th rowSpan={2} className="hidden sm:table-cell">Seen, UTC</Th>
                <Th rowSpan={2} className="hidden sm:table-cell">Coin, side</Th>
                <Th rowSpan={2}>Headline</Th>
                <Th rowSpan={2} className="text-right">Impact</Th>
                <Th rowSpan={2} className="text-right">New info</Th>
                <Th rowSpan={2}>Checks</Th>
                <Th colSpan={live.horizons.length} className="border-l">Return if followed, net bps</Th>
              </tr>
              <tr>
                {live.horizons.map((h) => <Th key={h.secs} className="text-right">{horizon(h.secs).replace(" min", "m").replace(" h", "h")}</Th>)}
              </tr>
            </thead>
            <tbody>
              {visible.map((c) => (
                <tr key={c.id + c.ts}>
                  <Td className="hidden sm:table-cell whitespace-nowrap text-muted-foreground">{clock(c.ts)}</Td>
                  <Td className="hidden sm:table-cell whitespace-nowrap">
                    <span className="font-medium">{c.coin}</span> <Side side={c.side} />
                  </Td>
                  <Td>
                    <Headline id={c.id} title={c.title} source={c.source} meta={<><span className="font-medium">{c.coin}</span><Side side={c.side} /><span className="text-faint">{clock(c.ts)} UTC</span></>} />
                    {c.triggered && <Tag kind="triggered" className="mt-1">Triggered</Tag>}
                  </Td>
                  <Td className="text-right tabular-nums text-muted-foreground">{signed(c.impact)}</Td>
                  <Td className="text-right">{c.new_info.toFixed(2)}</Td>
                  <Td><Gates c={c} /></Td>
                  {live.horizons.map((h) => (
                    <Td key={h.secs} className="text-right whitespace-nowrap">
                      <Markout c={c} secs={h.secs} now={live.now} enough={enough} />
                    </Td>
                  ))}
                </tr>
              ))}
            </tbody>
          </DataTable>
        )}
        {tab === "all" && live.news.length > 0 && (
          <p className="max-w-[62ch] border-t px-5 py-3 text-[13px] leading-5 text-muted-foreground">
            Impact runs -1 to +1 and carries the direction; new info runs 0 to 1. Late by is the gap between publication and
            the moment Jev read it. Only headlines that name a coin get the three checks and a markout. Showing the newest{" "}
            {Math.min(shown, live.news.length)} of {live.newsCount}.
          </p>
        )}
        {tab !== "all" && rows.length > 0 && (
          <p className="max-w-[62ch] border-t px-5 py-3 text-[13px] leading-5 text-muted-foreground">
            Checks, in order: <span className="text-foreground">impact</span> at least ±{cfg.min_impact.toFixed(2)},{" "}
            <span className="text-foreground">new information</span> at least {cfg.min_new_info.toFixed(2)},{" "}
            <span className="text-foreground">late by</span> at most {cfg.max_lag_min.toFixed(0)} min after publication. A dash
            means a markout is still to come; “no data” means the engine was down when it was due. Each column counts only
            headlines with a price reading at that delay, so the four counts can differ; “Headlines awaiting last markout”
            above counts scored headlines whose 6 h price reading has not landed. Triggered counts every headline that passed
            the checks; Awaiting counts only those still short of a 6 h reading, so the two can differ. {triggers} of{" "}
            {live.triggeredCount} triggered headlines are in view; showing {Math.min(shown, rows.length)} of {rows.length}, newest first.
            The table holds the newest {loaded} of {live.candidateCount.toLocaleString("en-US")} scored headlines.
          </p>
        )}
        {(tab === "all" ? live.news.length > shown : rows.length > shown) && (
          <div className="border-t px-5 py-3">
            <button
              type="button"
              onClick={() => setShown((n) => n + PAGE)}
              className="rounded-md border px-3 py-1.5 text-[13px] text-foreground hover:bg-muted"
            >
              Show {PAGE} more
              <span className="ml-2 text-muted-foreground">
                {(tab === "all" ? live.news.length - shown : rows.length - shown).toLocaleString("en-US")}{" "}
                {tab === "all" ? "headlines" : "headlines about a coin"} left
              </span>
            </button>
          </div>
        )}
        <GlossList
          ids={["impact", "new-info", "trigger", "markout", "lag", "evidence-days"]}
          intro="The terms this table uses. Hover an underlined word for the same record."
        />
      </Panel>

      <StationIndex stations={stations(live)} current="news" />
    </div>
  );
}
