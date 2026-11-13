"use client";

import Link from "next/link";
import { useState } from "react";
import { DataTable, Delta, Empty, Figures, Mark, PageHead, Panel, Verdict, Tag, Td, Th } from "@/components/bits";
import { CalibrationChart, GateLineMap, MapLegend, OosChart, stopIndex } from "@/components/charts";
import { Gloss, GlossList } from "@/components/gloss";
import { StationIndex } from "@/components/pipeline";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useLive } from "@/components/live";
import { familyName, plural, sentence, signed } from "@/lib/format";
import { stations } from "@/lib/v3/stations";
import type { FamilyReport, Lab, LoopCalibration } from "@/lib/v3/types";

/** The leading figure of a check's value: "+3.85 bps/day [..] over 600 days" → "+3.85 bps/day [..]". */
function lead(f: FamilyReport, name: string): string {
  const v = f.checks.find((c) => c.name === name)?.value ?? "";
  return v.replace(/\s*\(.*$/, "").replace(/\s+over .*$/, "").replace(/^p = /, "").replace(/-/g, "−") || "–";
}

/** The check names that have a glossary record, so the table can carry the definition. */
const CHECK_GLOSS: Record<string, Parameters<typeof Gloss>[0]["id"]> = {
  "walk-forward": "walk-forward",
  "reality check": "reality-check",
  "deflated Sharpe": "deflated-sharpe",
  "overfitting (PBO)": "pbo",
  "plateau": "plateau",
  "lockbox": "lockbox",
  "evidence": "evidence-days",
};

function CheckName({ name }: { name: string }) {
  const id = CHECK_GLOSS[name];
  return <>{id ? <Gloss id={id}>{sentence(name)}</Gloss> : sentence(name)}</>;
}

/** Checks a family failed; a family stopped for want of evidence before any check ran has none and no distance. */
function failed(f: FamilyReport) {
  return f.checks.filter((c) => !c.pass);
}

function scored(f: FamilyReport) {
  return f.checks.length > 0 && !(f.verdict === "INSUFFICIENT_EVIDENCE" && f.checks.every((c) => c.name === "evidence" || c.pass));
}

/** Fewest failed checks first; on a tie the family that got there first (hand-written, then the earlier generation). */
function byCloseness(a: FamilyReport, b: FamilyReport) {
  return failed(a).length - failed(b).length || (a.generation ?? 0) - (b.generation ?? 0) || (a.name ?? "").localeCompare(b.name ?? "");
}

const label = (f: FamilyReport) => f.name ?? familyName(f.family);

const dayTime = (iso: string) =>
  `${new Date(iso).toLocaleString("en-GB", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", timeZone: "UTC" })} UTC`;

/** The loop on synthetic panels, under the stop rules it runs with now (`--auto`). */
function LoopCalibrationRows({ rows }: { rows: LoopCalibration[] }) {
  const current = rows.filter((r) => r.auto);
  const shown = current.length ? current : rows;
  if (shown.length === 0) return null;
  const nulls = shown.filter((r) => r.edge === 0);
  const edges = shown.filter((r) => r.edge > 0);
  const noisePanels = nulls.reduce((n, r) => n + r.panels, 0);
  const noisePromoted = nulls.reduce((n, r) => n + r.anyPromoted, 0);
  return (
    <div className="mt-5 border-t pt-4">
      <p className="text-[13px] font-medium">With the loop searching on top</p>
      <p className="mt-1 max-w-[72ch] text-[13px] leading-5 text-pretty text-muted-foreground">
        The same synthetic markets, with the evolve loop breeding families for up to {shown[0].generations} generations under the
        hourly run&apos;s stop rules. Every family it generates counts as one more look at the lockbox, so searching longer cannot
        buy a pass.
      </p>
      <div className="mt-3 overflow-x-auto">
        <table className="w-full min-w-[560px] border-collapse text-[13px]">
          <caption className="sr-only">Evolve loop calibration: promotions on synthetic markets with and without a planted edge</caption>
          <thead>
            <tr className="text-left text-[11px] text-faint">
              <th scope="col" className="pb-2 font-normal">Synthetic market</th>
              <th scope="col" className="pb-2 font-normal">Seed family</th>
              <th scope="col" className="pb-2 text-right font-normal">Seed alone passed</th>
              <th scope="col" className="pb-2 text-right font-normal">Only the loop found it</th>
              <th scope="col" className="pb-2 text-right font-normal">Promoted</th>
            </tr>
          </thead>
          <tbody>
            {[...nulls, ...edges].map((r) => {
              const pct = Math.round((r.anyPromoted / Math.max(1, r.panels)) * 100);
              return (
                <tr key={r.file} className="border-t">
                  <td className="py-2.5">{r.edge === 0 ? "No edge" : `Edge ${r.edge} planted`}<span className="ml-2 text-[11px] text-faint">{r.panels} panels{r.file.includes("seed2") ? ", second set" : ""}</span></td>
                  <td className="py-2.5 text-muted-foreground">{familyName(r.seed)}, {r.interval}</td>
                  <td className="py-2.5 text-right tabular-nums text-muted-foreground">{r.seedPromoted}</td>
                  <td className="py-2.5 text-right tabular-nums text-muted-foreground">{r.loopOnly}</td>
                  <td className={`py-2.5 text-right font-mono tabular-nums ${r.edge === 0 ? (pct > 5 ? "text-neg" : "text-foreground") : pct > 0 ? "text-pos" : "text-foreground"}`}>
                    {r.anyPromoted} of {r.panels}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {noisePanels > 0 && (
        <p className="mt-2 text-xs leading-4 text-faint">
          {noisePromoted} of {noisePanels} no-edge markets promoted with the loop running.
          {edges.length > 0 && ` Where an edge was planted, the loop found ${plural(edges.reduce((n, r) => n + r.loopOnly, 0), "more market")} than the seed alone.`}
        </p>
      )}
    </div>
  );
}

export function LabView({ lab }: { lab: Lab }) {
  const { live } = useLive();
  const [selected, setSelected] = useState(lab.families.find((f) => f.family === "trend")?.family ?? lab.families[0]?.family ?? "");
  if (lab.families.length === 0)
    return (
      <Panel title="The lab has not judged anything yet">
        <Empty title="No reports on file">
          Run <code className="font-mono">jev-trader lab validate --family families/trend.toml</code> from the v3 worktree. Each verdict is written to <code className="font-mono">lab-data/reports/</code> and shows up here.
        </Empty>
      </Panel>
    );
  const all = [...lab.families, ...lab.generated];
  const promoted = all.filter((f) => f.verdict === "PROMOTE_TO_PAPER").length;
  const opened = all.filter((f) => f.lockbox_opened).length;
  const lockDays = Math.max(...lab.families.map((f) => f.lockbox_days));
  const lockMin = Math.min(...lab.families.map((f) => f.lockbox_days));
  const judged = all.map((f) => f.judged).filter(Boolean).sort().at(-1);
  const closest = all.filter(scored).sort(byCloseness)[0];
  const nearest = lab.generated.filter(scored).sort(byCloseness).slice(0, 3);
  const shown = [...lab.families, ...nearest];
  const handTrials = lab.trials - lab.loopTrials;
  const pick = (family: string) => {
    setSelected(family);
    document.getElementById("family-detail")?.scrollIntoView({ behavior: "smooth", block: "nearest" });
  };
  const closeLine = closest
    ? `Closest: ${label(closest)}${closest.generation !== undefined ? ` (generation ${closest.generation})` : ""}, which failed ${
        failed(closest).length <= 2 ? failed(closest).map((c) => c.name).join(" and ") : plural(failed(closest).length, "check")
      }.`
    : "";

  return (
    <div className="flex flex-col gap-6">
      <PageHead
        question="Has anything earned the paper run?"
        lede={
          <>
            The Lab is the gate. Every family, written by hand or generated by the hourly loop, is searched over old days and its
            winner judged against what that much searching produces by luck: the more rules tried, the better the best one has to
            be. Passing every check, then the sealed lockbox, is the only door to the{" "}
            <Link href="/paper" className="text-foreground underline decoration-border underline-offset-4 hover:decoration-foreground">paper run</Link>.
          </>
        }
        verdict={
          <Verdict
            rule={false}
            kind={promoted > 0 ? "good" : "calm"}
            label="Lab"
            meta={judged ? `Last verdict ${dayTime(judged)}` : "Never judged"}
            line={`${promoted} of ${plural(all.length, "family", "families")} passed`}
            detail={
              <>
                {lab.families.length} written by hand, {lab.generated.length} generated by the loop. {promoted === 0 ? closeLine : `${promoted} passed and run on paper; promoting stays your decision.`}
              </>
            }
            href="/generations"
            link="What the loop is trying"
          />
        }
      />

      <Figures
        items={[
          { label: "Families judged", value: all.length.toLocaleString("en-US"), sub: `${lab.families.length} by hand, ${lab.generated.length} by the loop` },
          { label: "Trials on the ledger", value: lab.trials.toLocaleString("en-US"), sub: `${handTrials.toLocaleString("en-US")} from hand-written searches, ${lab.loopTrials.toLocaleString("en-US")} from the loop. Every one raises the bar.` },
          {
            label: "Lockbox",
            value: opened ? `Opened for ${plural(opened, "family", "families")}` : "Sealed",
            sub: `The newest ${lockMin} to ${lockDays} days of each study, unseen by any search`,
            tone: opened > 1 ? "warn" : undefined,
          },
          {
            label: "Last verdict",
            value: judged ? new Date(judged).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" }) : "–",
            sub: judged ? `${dayTime(judged)}; the loop judges new families every hour` : "",
          },
        ]}
      />

      <Panel
        title="How far each family got"
        description={
          <>
            Every family meets the same checks in order, and the first fatal miss ends its run. The count lines say how many of each
            group were still in when a check came up; the rows are the {lab.families.length} hand-written families and the{" "}
            {plural(nearest.length, "generated family", "generated families")} that came closest. {promoted === 0 ? "None reached the lockbox." : `${promoted} reached the lockbox.`}{" "}
            A generated family is built after its design window was seen, so its walk-forward is not truly out of sample: the
            lockbox is its first clean test. Hover a check for its numbers; select a family to read them all.
          </>
        }
      >
        <div className="border-t pt-4">
          <GateLineMap
            families={shown}
            selected={selected}
            onSelect={pick}
            cohorts={[
              { label: `Hand-written, ${lab.families.length}`, families: lab.families },
              ...(lab.generated.length ? [{ label: `Generated, ${lab.generated.length}`, families: lab.generated }] : []),
            ]}
          />
        </div>
        <div className="mt-4 border-t pt-3">
          <MapLegend />
        </div>
      </Panel>

      <Panel flush title="Hand-written families" description="The best rule of each family in sample, then what its walk-forward picks made out of sample. Rows are families, not a ranking: the best Sharpe can sit on a family that did not pass. Generated families are on Generations.">
        <DataTable minWidth={820}>
          <thead>
            <tr>
              <Th>Family</Th><Th className="text-right"><Gloss id="spec">Rules</Gloss></Th><Th className="text-right">Best <Gloss id="sharpe">Sharpe</Gloss></Th>
              <Th className="text-right"><Gloss id="walk-forward">Walk-forward</Gloss></Th>
              <Th className="text-right"><Gloss id="reality-check">Reality check p</Gloss></Th>
              <Th className="text-right"><Gloss id="deflated-sharpe">Deflated Sharpe</Gloss></Th>
              <Th>Stopped at</Th><Th>Verdict</Th>
            </tr>
          </thead>
          <tbody>
            {lab.families.map((f) => {
              const stop = stopIndex(f);
              return (
                <tr key={f.family} onClick={() => pick(f.family)} className={`cursor-pointer ${selected === f.family ? "bg-muted/60" : ""}`}>
                  <Td>
                    <button type="button" className="flex items-center gap-2 font-medium whitespace-nowrap" onClick={() => pick(f.family)}>
                      {familyName(f.family)}
                    </button>
                    <div className="text-xs whitespace-nowrap text-faint">{f.hypothesis}, {f.study.replace("-", " ")}</div>
                  </Td>
                  <Td className="text-right">{f.specs}</Td>
                  <Td className="text-right"><span className="tabular-nums text-foreground/80">{f.leaders[0] ? signed(f.leaders[0].sharpe_annual) : "–"}</span></Td>
                  <Td className="text-right whitespace-nowrap">{f.oosCum.length ? lead(f, "walk-forward") : <span className="text-faint">Not run</span>}</Td>
                  <Td className="text-right">{lead(f, "reality check")}</Td>
                  <Td className="text-right">{lead(f, "deflated Sharpe")}</Td>
                  <Td className="text-foreground">
                    {stop < 0
                      ? f.verdict === "REVISE" ? "Ran the gate, short of the bar" : "Ran the whole gate"
                      : sentence(f.checks[stop]?.name ?? "")}
                  </Td>
                  <Td><Tag kind={f.verdict} /></Td>
                </tr>
              );
            })}
          </tbody>
        </DataTable>
        <p className="border-t px-5 py-3 text-[13px] leading-5 text-muted-foreground">
          A check passes when <span className="text-foreground">Measured</span> meets <span className="text-foreground">Needs</span>.
          The first fatal miss ends the line; the verdict is the result of every check, not of the number that looks best.
        </p>
        <GlossList
          ids={["family", "spec", "sharpe", "in-sample", "walk-forward", "reality-check", "deflated-sharpe", "trial", "lockbox"]}
          intro="Hover an underlined term anywhere on the page for the same record."
        />
      </Panel>

      <div id="family-detail" className="scroll-mt-28">
        <Tabs value={selected} onValueChange={setSelected}>
          <Panel
            flush
            title="Check by check"
            description="What the gate measured, and what it needed. The first fatal check is the one that ended the family. A family can stop at Evidence with good numbers: the gate needed more days, not a better result."
          >
            <div className="overflow-x-auto px-5 pb-3">
              <TabsList className="h-8">
                {shown.map((f, i) => (
                  <span key={f.family} className="flex items-center">
                    {f.name && !shown[i - 1]?.name && <span aria-hidden className="mx-1.5 h-4 w-px bg-border" />}
                    <TabsTrigger value={f.family} className={`text-xs ${f.name ? "font-mono" : ""}`}>{label(f)}</TabsTrigger>
                  </span>
                ))}
              </TabsList>
            </div>
            {shown.map((f) => {
              const stop = stopIndex(f);
              const insuff = f.verdict === "INSUFFICIENT_EVIDENCE";
              return (
                <TabsContent key={f.family} value={f.family} className="mt-0">
                  <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t px-5 py-3 text-[13px] text-muted-foreground">
                    <Tag kind={f.verdict} />
                    <span>{f.panel.replace(/ · /g, ", ").replace("→", "to")}</span>
                    <span>{f.design_days} design days, {f.lockbox_days} in the lockbox</span>
                    {f.judged && <span className="ml-auto text-faint">Judged {new Date(f.judged).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" })}</span>}
                  </div>
                  <DataTable minWidth={760}>
                    <thead>
                      <tr><Th className="w-10" /><Th>Check</Th><Th>Measured</Th><Th>Needs</Th></tr>
                    </thead>
                    <tbody>
                      {f.checks.map((c, i) => (
                        <tr key={c.name} className={i === stop ? (insuff ? "bg-warn/[0.04]" : "bg-neg/[0.04]") : undefined}>
                          <Td><Mark pass={c.pass} fatal={c.fatal} /></Td>
                          <Td className="font-medium whitespace-nowrap">
                            <CheckName name={c.name} />
                            {i === stop && <span className={`ml-2 text-xs font-normal ${insuff ? "text-warn" : "text-neg"}`}>{insuff ? "not enough evidence to judge" : "the line ends here"}</span>}
                          </Td>
                          <Td className="max-w-[520px] leading-5">{c.value.replace(/(^|[\s[(,])-(?=\d)/g, "$1−")}</Td>
                          <Td className="text-muted-foreground">{/^[a-z]{2}/i.test(c.need) ? sentence(c.need) : c.need}</Td>
                        </tr>
                      ))}
                    </tbody>
                  </DataTable>
                  {f.notes.map((n) => (
                    <p key={n} className="border-t px-5 py-3 text-[13px] leading-5 text-muted-foreground">{sentence(n)}.</p>
                  ))}
                  <div className="border-t px-5 pt-4 pb-2 text-sm font-medium">Best rules in sample</div>
                  <DataTable minWidth={720}>
                    <thead>
                      <tr><Th>Rule</Th><Th className="text-right">Sharpe</Th><Th className="text-right">Per day</Th><Th className="text-right">Trades</Th><Th className="text-right">Wins</Th><Th className="text-right">Per trade</Th></tr>
                    </thead>
                    <tbody>
                      {f.leaders.slice(0, 5).map((l) => (
                        <tr key={l.spec}>
                          <Td className="font-mono text-xs">{l.spec}</Td>
                          <Td className="text-right"><span className="tabular-nums text-foreground/80">{signed(l.sharpe_annual)}</span></Td>
                          <Td className="text-right"><Delta value={l.mean_bps_day} digits={1} unit=" bps" /></Td>
                          <Td className="text-right">{l.trades.toLocaleString("en-US")}</Td>
                          <Td className="text-right">{l.win_pct.toFixed(0)}%</Td>
                          <Td className="text-right"><Delta value={l.mean_trade_bps} digits={1} unit=" bps" /></Td>
                        </tr>
                      ))}
                    </tbody>
                  </DataTable>
                  <p className="border-t px-5 py-3 text-xs leading-4 text-faint">
                    Each rule is one point on the family&apos;s grid, named after the parameters that define it. These figures are in
                    sample, after costs, and the winner was chosen from the grid.
                  </p>
                </TabsContent>
              );
            })}
          </Panel>
        </Tabs>
      </div>

      <Panel
        title="Out of sample, walk-forward"
        description="Cumulative return of the rule each fold picked on 180 training days, traded on the next 60 it had not seen. Costs and funding included."
      >
        <OosChart families={lab.families} selected={selected} />
        <p className="mt-3 border-t pt-3 text-xs leading-4 text-faint">
          The line is a selection rule, not one fixed strategy: the pick can change per fold. A line that goes up left to
          right is what an edge would look like; the gate decides whether it is more than the search could produce by luck.
        </p>
      </Panel>

      {lab.calibrations.length > 0 && (
        <Panel
          title="Does the gate promote noise?"
          description="The whole gate run on synthetic markets where the answer is known. With no edge planted it should promote almost nothing; with a real edge it should find it more often the stronger the edge."
        >
          <div className="border-t pt-3">
            <CalibrationChart calibrations={lab.calibrations} />
          </div>
          <p className="mt-3 text-xs leading-4 text-faint">
            This is the gate&apos;s calibration, not a result. It is how a reader can tell a strict gate from a lucky one.
          </p>
          <LoopCalibrationRows rows={lab.loopCalibrations} />
        </Panel>
      )}

      <StationIndex stations={stations(live)} current="lab" />
    </div>
  );
}
