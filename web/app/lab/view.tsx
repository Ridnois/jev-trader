"use client";

import Link from "next/link";
import { useState } from "react";
import { DataTable, Delta, Empty, Figures, Mark, PageHead, Panel, Verdict, Tag, Td, Th } from "@/components/bits";
import { CalibrationChart, GateLineMap, MapLegend, OosChart, stopIndex } from "@/components/charts";
import { Gloss, GlossList } from "@/components/gloss";
import { StationIndex } from "@/components/pipeline";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useLive } from "@/components/live";
import { familyName, sentence, signed } from "@/lib/format";
import { stations } from "@/lib/v3/stations";
import type { FamilyReport, Lab } from "@/lib/v3/types";

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
  const promoted = lab.families.filter((f) => f.verdict === "PROMOTE_TO_PAPER").length;
  const rejected = lab.families.filter((f) => f.verdict === "REJECT").length;
  const thin = lab.families.filter((f) => f.verdict === "INSUFFICIENT_EVIDENCE").length;
  const opened = lab.families.filter((f) => f.lockbox_opened).length;
  const lockDays = Math.max(...lab.families.map((f) => f.lockbox_days));
  const lockMin = Math.min(...lab.families.map((f) => f.lockbox_days));
  const revise = lab.families.filter((f) => f.verdict === "REVISE").length;
  const judged = lab.families.map((f) => f.judged).filter(Boolean).sort().at(-1);
  const pick = (family: string) => {
    setSelected(family);
    document.getElementById("family-detail")?.scrollIntoView({ behavior: "smooth", block: "nearest" });
  };

  return (
    <div className="flex flex-col gap-6">
      <PageHead
        question="Should any of this be trusted with money?"
        lede={
          <>
            The Lab searches a family of rules over old days, then judges the winner against what that search alone can
            produce by luck: the more rules tried, the better the best one has to be. A family must pass every check, then
            the sealed lockbox, the newest days no search may see. Passing is the only door to the paper run.{" "}
            <Link href="/paper" className="text-foreground underline decoration-border underline-offset-4 hover:decoration-foreground">Paper follows whatever comes through.</Link>
          </>
        }
        verdict={
          <Verdict
            rule={false}
            kind={promoted > 0 ? "good" : "calm"}
            label="Lab"
            meta={`${lab.families.length} families judged`}
            line={`${promoted} of ${lab.families.length} families passed`}
            detail={
              promoted === 0
                ? `${rejected} families were rejected and ${thin} did not have enough evidence${revise > 0 ? `, and ${revise} were sent back to revise` : ""}. A rejected family is not proof that no version could work; it is proof this search did not find one.`
                : `${lab.families.length - promoted} families did not pass. The promoted ones are running on paper against their cards.`
            }
            href="/start"
            link="What the gate is for"
          />
        }
      />

      <Figures
        items={[
          { label: "Families judged", value: `${lab.families.length}`, sub: `${rejected} families rejected, ${thin} with not enough evidence${revise > 0 ? `, ${revise} to revise` : ""}` },
          { label: "Trials on the ledger", value: lab.trials.toLocaleString("en-US"), sub: "Every rule run once on one data set, across every study; the Rules column covers these families only" },
          {
            label: "Lockbox",
            value: opened ? `Opened for ${opened} ${opened === 1 ? "family" : "families"}` : "Sealed",
            sub: `Withheld per family: ${lockMin} to ${lockDays} days`,
            tone: opened > 1 ? "warn" : undefined,
          },
          {
            label: "Last judged",
            value: judged ? new Date(judged).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" }) : "–",
            sub: "The date every verdict on this page was written",
          },
        ]}
      />

      <Panel flush title="Verdicts" description="The best rule of each family in sample, then what its walk-forward picks made out of sample. Rows are families, not a ranking: the best Sharpe can sit on a family that did not pass. Read this table first; the chart below shows how far each family got.">
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
                      <span className="h-[3px] w-3.5 rounded-full bg-foreground/30" />
                      {familyName(f.family)}
                    </button>
                    <div className="pl-[22px] text-xs whitespace-nowrap text-faint">{f.hypothesis}, {f.study.replace("-", " ")}</div>
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

      <Panel
        title="How far each family got"
        description={`Every family runs the same ${lab.families[0]?.checks.length ?? 0} checks in order, and the first fatal miss ends its run. ${promoted === 0 ? "None reached the lockbox." : `${promoted} reached the lockbox.`} Hover a check for its numbers; select a family to read them all.`}
      >
        <div className="border-t pt-4">
          <GateLineMap families={lab.families} selected={selected} onSelect={pick} />
        </div>
        <div className="mt-4 border-t pt-3">
          <MapLegend />
        </div>
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
                {lab.families.map((f) => (
                  <TabsTrigger key={f.family} value={f.family} className="text-xs">{familyName(f.family)}</TabsTrigger>
                ))}
              </TabsList>
            </div>
            {lab.families.map((f) => {
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
                          <Td className="text-muted-foreground">{sentence(c.need)}</Td>
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
        </Panel>
      )}

      <StationIndex stations={stations(live)} current="lab" />
    </div>
  );
}
