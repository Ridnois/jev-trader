"use client";

import Link from "next/link";
import { useState, useTransition } from "react";
import { ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { DataTable, Delta, Empty, Figures, Mark, PageHead, Panel, Verdict, Tag, Td, Th } from "@/components/bits";
import { CalibrationChart, OosAll, ReturnByGeneration, stopIndex } from "@/components/charts";
import { Gloss } from "@/components/gloss";
import { StationIndex } from "@/components/pipeline";
import { useLive } from "@/components/live";
import { familyName, plural, sentence, signed } from "@/lib/format";
import { stations } from "@/lib/v3/stations";
import type { FamilyReport, Lab, LoopCalibration, OnPaper } from "@/lib/v3/types";
import { promoteFamily, type PromoteResult } from "./actions";

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

/**
 * On paper by the operator's decision although the gate did not pass it (`lab promote --candidate`).
 * Matched by name first: two generated families can carry the same rule id (g0034 and g0043), and only the
 * one promoted was approved. A strategy with no report under its own name (hand-written, or a rule whose
 * report is filed under its twin) matches by rule id.
 */
const approvedBy = (paper: OnPaper[], reports: FamilyReport[]) => {
  const names = new Set(reports.map((r) => r.name).filter(Boolean));
  const ok = new Set<string>();
  for (const p of paper.filter((p) => p.status !== "validated")) {
    for (const r of reports) {
      if (names.has(p.name) ? r.name === p.name : r.family === p.family) ok.add(`${r.study}|${r.family}|${r.name ?? ""}`);
    }
  }
  return (f: FamilyReport) => f.verdict !== "PROMOTE_TO_PAPER" && ok.has(`${f.study}|${f.family}|${f.name ?? ""}`);
};

/** The gate's verdict, then the operator's approval when there is one: two separate calls, side by side. */
function Verdicts({ f, approved }: { f: FamilyReport; approved: boolean }) {
  return (
    <span className="inline-flex flex-wrap items-center gap-1.5">
      <Tag kind={f.verdict} />
      {approved && <span title="You put it on paper as an unproven candidate. The gate's verdict is unchanged."><Tag kind="approved" /></span>}
    </span>
  );
}

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

/**
 * Hand the family to the paper trader. A family the gate passed goes in one click, as validated; any other
 * asks first and goes in labelled as an unproven candidate. Either way the trader restarts to load it.
 */
function PromoteControl({ f, paper }: { f: FamilyReport; paper: OnPaper[] }) {
  const [confirming, setConfirming] = useState(false);
  const [result, setResult] = useState<PromoteResult | null>(null);
  const [pending, start] = useTransition();
  const on = paper.filter((p) => p.family === f.family);
  const validated = f.verdict === "PROMOTE_TO_PAPER";
  const go = () =>
    start(async () => {
      setResult(await promoteFamily(f.family, f.study));
      setConfirming(false);
    });
  let body: React.ReactNode;
  if (on.length > 0) {
    body = (
      <span className="text-muted-foreground">
        On paper as {on.map((p) => <span key={p.name}><span className="font-mono text-foreground">{p.name}</span> ({p.status === "validated" ? "validated" : "unproven candidate"})</span>)}.{" "}
        <Link href="/paper" className="text-foreground underline decoration-border underline-offset-4 hover:decoration-foreground">Paper run</Link>
      </span>
    );
  } else if (f.leaders.length === 0) {
    body = <span className="text-faint">Nothing to promote: the gate picked no rule for this family.</span>;
  } else if (pending) {
    body = <span className="text-muted-foreground">Promoting {label(f)} and restarting the trader…</span>;
  } else if (confirming) {
    body = (
      <>
        <span className="max-w-[64ch] text-pretty">
          {label(f)} did not pass the gate ({failed(f).map((c) => c.name).join(", ") || f.verdict.toLowerCase().replaceAll("_", " ")}). It will
          paper-trade labelled as an unproven candidate, with paper money only, and the trader restarts.
        </span>
        <span className="ml-auto flex gap-2">
          <Button size="sm" variant="ghost" onClick={() => setConfirming(false)}>Cancel</Button>
          <Button size="sm" onClick={go}>Paper-trade it</Button>
        </span>
      </>
    );
  } else {
    body = (
      <>
        <span className="text-muted-foreground">
          {validated ? "It passed every check and the lockbox." : "Not validated: it would run as an unproven candidate."} Rule {f.leaders[0].spec}.
        </span>
        <Button size="sm" variant={validated ? "default" : "outline"} className="ml-auto" onClick={validated ? go : () => setConfirming(true)}>
          {validated ? "Promote to paper" : "Promote as candidate"}
        </Button>
      </>
    );
  }
  return (
    <div className="border-t px-5 py-3 text-[13px]">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">{body}</div>
      {result && (
        <p role="status" className={`mt-2 text-xs leading-4 ${result.ok ? "text-pos" : "text-neg"}`}>{result.message}</p>
      )}
    </div>
  );
}

/** The selected family, read check by check: what the gate measured, what it needed, and the best rules it searched. */
function FamilyDetail({ f, place, total, paper, approved }: { f: FamilyReport; place: number; total: number; paper: OnPaper[]; approved: boolean }) {
  const stop = stopIndex(f);
  const insuff = f.verdict === "INSUFFICIENT_EVIDENCE";
  const short = failed(f);
  return (
    <Panel
      flush
      title={
        <span className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
          <span className={`text-[15px] font-medium tracking-tight ${f.name ? "font-mono" : ""}`}>{label(f)}</span>
          <span className="text-[13px] font-normal text-muted-foreground">
            {f.generation !== undefined ? `generation ${f.generation}` : "hand-written"} · {f.hypothesis} · {f.study.replace("-", " ")}
          </span>
        </span>
      }
      description={
        f.prior
          ? `Judged before the ledger restarted, against ${plural(f.trials_ledger, "trial")} on the old ledger. Kept as a record: its checks faced a different bar from this era's families, so it is not ranked against them.`
          : scored(f)
          ? `${place} of ${total} by distance to the goal. ${short.length === 0 ? "It failed no check." : `It failed ${short.map((c) => c.name).join(", ")}.`} A check passes when Measured meets Needs; the first fatal miss ends the run.`
          : "The gate stopped it at evidence: too few days or trades to run the checks, so it has no distance yet."
      }
      action={
        <span className="flex items-center gap-2">
          {f.prior && <Tag kind="era1">Earlier era</Tag>}
          <Verdicts f={f} approved={approved} />
        </span>
      }
    >
      <div className="flex flex-wrap gap-x-4 gap-y-1 border-t px-5 py-3 text-[13px] text-muted-foreground">
        <span>{f.panel.replace(/ · /g, ", ").replace("→", "to")}</span>
        <span>{f.design_days} design days, {f.lockbox_days} in the lockbox</span>
        <span>{plural(f.specs, "rule")} searched</span>
        {f.judged && <span className="ml-auto text-faint">Judged {dayTime(f.judged)}</span>}
      </div>
      <PromoteControl key={`${f.study}|${f.family}`} f={f} paper={paper} />
      <DataTable minWidth={760} label={`Gate checks for ${label(f)}`}>
        <thead>
          <tr><Th className="w-10" /><Th>Check</Th><Th>Measured</Th><Th>Needs</Th></tr>
        </thead>
        <tbody>
          {f.checks.map((c, i) => (
            <tr key={c.name} className={i === stop ? (insuff ? "bg-warn/[0.04]" : "bg-neg/[0.04]") : undefined}>
              <Td><Mark pass={c.pass} fatal={c.fatal} /></Td>
              <Td className="font-medium whitespace-nowrap">
                <CheckName name={c.name} />
                {i === stop && <span className={`ml-2 text-xs font-normal ${insuff ? "text-warn" : "text-neg"}`}>{insuff ? "not enough evidence to judge" : "the run ends here"}</span>}
              </Td>
              <Td className="max-w-[520px] leading-5">{c.value.replace(/(^|[\s[(,])-(?=\d)/g, "$1−")}</Td>
              <Td className="text-muted-foreground">{/^[a-z]{2}/i.test(c.need) ? sentence(c.need) : c.need}</Td>
            </tr>
          ))}
        </tbody>
      </DataTable>
      {f.notes.map((n) => (
        <p key={n} className="border-t px-5 py-3 text-[13px] leading-5 text-muted-foreground">{sentence(n).replace(/\.*$/, ".")}</p>
      ))}
      {f.leaders.length > 0 && (
        <details className="group border-t">
          <summary className="flex cursor-pointer list-none items-center gap-2 px-5 py-3 text-[13px]">
            <ChevronRight aria-hidden className="size-3.5 text-muted-foreground transition-transform group-open:rotate-90" />
            <span className="font-medium">Best rules in sample</span>
            <span className="text-muted-foreground">
              {f.leaders[0].spec} · Sharpe {signed(f.leaders[0].sharpe_annual)} · {signed(f.leaders[0].mean_bps_day, 1)} bps a day, before the gate
            </span>
          </summary>
          <DataTable minWidth={720} label={`Best rules in sample for ${label(f)}`}>
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
            In sample, after costs, and chosen from the grid: these are the numbers the gate exists to discount.
          </p>
        </details>
      )}
    </Panel>
  );
}

const EARLIER_SHOWN = 12;

/** Verdicts from before the ledger restarted: a record, in the order they came closest, never mixed into this era's ranking. */
function EarlierVerdicts({ families, era, selected, onSelect, approved }: { families: FamilyReport[]; era: string | null; selected: string; onSelect: (family: string) => void; approved: (f: FamilyReport) => boolean }) {
  const [all, setAll] = useState(false);
  const shown = all ? families : families.slice(0, EARLIER_SHOWN);
  return (
    <Panel
      flush
      title="Earlier verdicts"
      description={`Judged before the ledger restarted${era ? ` ${era}` : ""}, each against the trials on the old ledger at the time. The bar was higher than this era's, so "1 check short" here and in this era do not mean the same thing. Kept as a record; re-judging would charge every rule again.`}
    >
      <DataTable minWidth={640} label="Verdicts from before the ledger restarted">
        <thead>
          <tr><Th>Family</Th><Th>Verdict</Th><Th className="text-right">Checks short</Th><Th>First check short</Th><Th className="text-right">Ledger then</Th></tr>
        </thead>
        <tbody>
          {shown.map((f) => {
            const short = failed(f);
            const on = f.family === selected;
            return (
              <tr key={`${f.study}|${f.family}`} onClick={() => onSelect(f.family)} className={`cursor-pointer ${on ? "bg-muted/40" : ""}`} aria-selected={on}>
                <Td>
                  <button type="button" onClick={(e) => { e.stopPropagation(); onSelect(f.family); }} className="flex items-baseline gap-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring">
                    <span className={f.name ? "font-mono" : "font-medium"}>{label(f)}</span>
                    <span className="text-[11px] text-faint">{f.generation !== undefined ? `gen ${f.generation}` : "hand-written"} · {f.study.replace("binance-", "")}</span>
                  </button>
                </Td>
                <Td><Verdicts f={f} approved={approved(f)} /></Td>
                <Td className="text-right tabular-nums">{scored(f) ? short.length : <span className="text-faint">–</span>}</Td>
                <Td className="text-muted-foreground">{scored(f) ? (short[0]?.name ?? "none") : "stopped at evidence"}</Td>
                <Td className="text-right tabular-nums text-muted-foreground">{f.trials_ledger.toLocaleString("en-US")} trials</Td>
              </tr>
            );
          })}
        </tbody>
      </DataTable>
      {families.length > EARLIER_SHOWN && (
        <div className="flex items-center justify-between border-t px-5 py-3 text-[12px] text-muted-foreground">
          <span>Showing {shown.length} of {families.length}</span>
          <button type="button" onClick={() => setAll((v) => !v)} className="rounded-md border px-3 py-1.5 text-foreground transition-colors hover:bg-muted">
            {all ? `Show the closest ${EARLIER_SHOWN}` : `Show all ${families.length}`}
          </button>
        </div>
      )}
    </Panel>
  );
}

export function LabView({ lab }: { lab: Lab }) {
  const { live } = useLive();
  const every = [...lab.families, ...lab.generated];
  // Only this era's verdicts are ranked: earlier ones faced the old ledger's bar and are kept as a record below.
  const all = every.filter((f) => !f.prior);
  const earlier = [...every.filter((f) => f.prior && scored(f)).sort(byCloseness), ...every.filter((f) => f.prior && !scored(f))];
  // One ranking for every family: fewest checks failed first; families the gate could not judge last.
  const ranked = [...all.filter(scored).sort(byCloseness), ...all.filter((f) => !scored(f))];
  const [selected, setSelected] = useState(ranked[0]?.family ?? earlier[0]?.family ?? "");
  if (every.length === 0)
    return (
      <Panel title="The lab has not judged anything yet">
        <Empty title="No reports on file">
          Run <code className="font-mono">jev-trader lab validate --family families/trend.toml</code> from the v3 worktree. Each verdict is written to <code className="font-mono">lab-data/reports/</code> and shows up here.
        </Empty>
      </Panel>
    );
  const promoted = all.filter((f) => f.verdict === "PROMOTE_TO_PAPER").length;
  const isApproved = approvedBy(lab.onPaper, every);
  const approved = every.filter(isApproved);
  const approvedNote = approved.length
    ? ` You approved ${promoted > 0 ? plural(approved.length, "other family", "other families") : plural(approved.length, "family", "families")} for paper anyway (${approved.map(label).join(", ")}); they run as unproven candidates.`
    : "";
  const opened = all.filter((f) => f.lockbox_opened).length;
  const lockDays = Math.max(...every.map((f) => f.lockbox_days));
  const judged = all.map((f) => f.judged).filter(Boolean).sort().at(-1);
  const closest = ranked.find(scored);
  const unjudged = all.length - all.filter(scored).length;
  const sel = every.find((f) => f.family === selected) ?? ranked[0] ?? earlier[0];
  const eraWhen = lab.era ? dayTime(lab.era) : null;
  const dist = (f: FamilyReport) => failed(f).length;
  const within = (k: number) => all.filter((f) => scored(f) && dist(f) <= k).length;
  const pick = (family: string) => {
    setSelected(family);
    document.getElementById("family-detail")?.scrollIntoView({ behavior: "smooth", block: "nearest" });
  };

  return (
    <div className="flex flex-col gap-6">
      <PageHead
        question="How close is any family to the paper run?"
        lede={
          <>
            A family is one idea about the market, written by hand or bred by the hourly loop; the gate treats both the same. It
            searches the family over old days, then judges the winner against what that much searching produces by luck. The goal
            is a family that fails none of the checks and then holds up on the sealed lockbox: only that one reaches the{" "}
            <Link href="/paper" className="text-foreground underline decoration-border underline-offset-4 hover:decoration-foreground">paper run</Link>.
          </>
        }
        verdict={
          <Verdict
            rule={false}
            kind={promoted > 0 ? "good" : "calm"}
            label="Lab"
            meta={judged ? `Last verdict ${dayTime(judged)}` : eraWhen ? `Ledger restarted ${eraWhen}` : "Never judged"}
            line={
              promoted > 0
                ? `${promoted} of ${plural(all.length, "family", "families")} passed`
                : closest
                  ? `${plural(dist(closest), "check")} short`
                  : all.length === 0 && eraWhen
                    ? "No verdict this era"
                    : `0 of ${plural(all.length, "family", "families")} judged`
            }
            detail={
              promoted > 0
                ? `${promoted} passed the gate and the lockbox. Promoting stays your decision.${approvedNote}`
                : closest
                  ? `The closest of ${plural(all.length, "family", "families")} judged this era is ${label(closest)}, which failed only ${failed(closest).map((c) => c.name).join(" and ")}. None has passed.${approvedNote}`
                  : all.length === 0 && eraWhen
                    ? `The ledger restarted ${eraWhen}. The ${earlier.length} families judged before it faced the old ledger's bar; their verdicts are kept below as a record, not ranked. The next family the loop judges starts this era's ranking.`
                    : "No family has been judged yet."
            }
            href="/generations"
            link="How the loop breeds them"
          />
        }
      />

      <Figures
        items={[
          {
            label: lab.era ? "Families judged this era" : "Families judged",
            value: all.length.toLocaleString("en-US"),
            sub: [
              all.length ? `${all.filter((f) => f.generation === undefined).length} written by hand` : "",
              unjudged ? `${unjudged} stopped before the checks` : "",
              earlier.length ? `${earlier.length} earlier, kept as a record` : "",
              approved.length ? `${approved.length} approved by you for paper` : "",
            ].filter(Boolean).join(", ") || "None yet",
          },
          { label: "Within 1 check", value: all.length ? within(1).toLocaleString("en-US") : "–", sub: all.length ? `${within(3)} within 3 checks of passing` : "Counted from this era's verdicts" },
          { label: "Trials on the ledger", value: lab.trials.toLocaleString("en-US"), sub: "Every rule searched since the ledger last restarted. Each one raises the bar the next family must clear" },
          {
            label: "Lockbox",
            value: opened ? `Opened for ${plural(opened, "family", "families")}` : "Sealed",
            sub: `The newest ${lockDays} days, unseen by any search until a family passes every check`,
            tone: opened > 1 ? "warn" : undefined,
          },
        ]}
      />

      <div id="family-detail" className="scroll-mt-28">
        {sel && <FamilyDetail f={sel} place={ranked.indexOf(sel) + 1} total={ranked.length} paper={lab.onPaper} approved={isApproved(sel)} />}
      </div>

      {earlier.length > 0 && <EarlierVerdicts families={earlier} era={eraWhen} selected={sel?.family ?? ""} onSelect={pick} approved={isApproved} />}

      <Panel
        title="Out of sample, walk-forward"
        description="Cumulative return of the rule each fold picked on 180 training days, traded on the next 60 it had not seen, costs and funding included. Every family on one chart, coloured by final return: greenest highest, reddest lowest, the median dashed. Hover a line to name it; click it to read its checks."
      >
        <OosAll hand={[]} generated={every} selected={sel.family} onSelect={pick} />
        <p className="mt-4 border-t pt-3 text-xs leading-4 text-faint">
          Each line is a selection rule, not one fixed strategy: the pick can change per fold. A line that climbs is what an edge would
          look like; the gate decides whether it is more than the search could produce by luck. A family bred by the loop was built after
          the design window was seen, so its folds are not truly out of sample. The lines stop where the last full 60-day fold ends;
          the shaded days after that are the lockbox, held back from every search until a family passes everything else.
        </p>
      </Panel>

      <Panel
        title="Return per generation"
        description="Each dot is one family's final walk-forward return, placed on the generation that bred it; hand-written families come first. The solid line joins the best of each generation, the dashed line its median. Click a dot to read its checks."
      >
        <ReturnByGeneration families={every} selected={sel.family} onSelect={pick} />
        <p className="mt-4 border-t pt-3 text-xs leading-4 text-faint">
          A best line that climbs generation after generation is what learning would look like, but it is also what searching longer
          produces by luck. Bred families were built after the design window was seen, so these returns are not clean out-of-sample
          evidence; only the lockbox and paper days are.
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
