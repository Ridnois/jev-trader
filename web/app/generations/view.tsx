import Link from "next/link";
import { ChevronRight } from "lucide-react";
import { DataTable, Empty, PageHead, Panel, Tag, Td, Th, Verdict } from "@/components/bits";
import { Gloss, GlossList } from "@/components/gloss";
import { clock, dateShort, familyName, plural, signed } from "@/lib/format";
import type { Check, CycleReport, GenNode, Generations, Pairing, ProposalRow } from "@/lib/v3/types";
import { cn } from "@/lib/utils";

const EVOLVE = "jev-trader lab evolve";
const EVOLVE_DRY = "jev-trader lab evolve --dry-run";

function Code({ children }: { children: React.ReactNode }) {
  return <code className="font-mono text-[12px] text-foreground">{children}</code>;
}

/** A parent's readable name: `g0001` for a generated family, the family name for a hand-written one. */
function parentName(gens: Generations, parent: string | null): string {
  if (!parent) return "nothing (exploration)";
  return gens.names[parent] ?? familyName(parent);
}

function bar(x: number): string {
  return Number.isFinite(x) ? x.toFixed(2) : "–";
}

/** The `tested` row's note is a comma list of the checks that failed. */
function checks(note: string): string[] {
  return note ? note.split(",").map((s) => s.trim()).filter(Boolean) : [];
}

/**
 * What each gate check asks, in one clause. The names are the gate's own
 * (`src/lab/gate.rs`), so they stay lowercase and verbatim; the clause is the
 * only plain-words version of them on this page.
 */
const CHECK_ASKS: Record<string, string> = {
  evidence: "Enough design days and trades to judge at all.",
  "walk-forward": "Re-fitted on each stretch and traded on days it never saw, it earns with the 95% interval above zero.",
  "reality check": "Its best result beats what reshuffled price history produces by chance (p < 0.05).",
  "generation reality check": "The reality check again, counting every rule the generation tried (p < 0.05).",
  "deflated Sharpe": "The rule it picked beats the best of every trial on its ledger (probability ≥ 0.95).",
  "overfitting (PBO)": "The winner usually stays above the median when the data is split again (≤ 0.30).",
  plateau: "Its nearby settings work too, not just the peak.",
  "costs ×2": "Still positive at double the assumed costs.",
  "one bar late": "Still positive entering one bar later.",
  coins: "Profitable without leaning on a single coin.",
  quarters: "Positive in most quarters of the design window.",
  "best days removed": "Still positive without its best handful of days.",
  "timing baseline": "Beats time-shifted copies of the same trades.",
  "market-neutral part": "The gain survives after subtracting what the market did.",
  lockbox: "It also earns on the lockbox days, at a threshold divided by earlier looks.",
};

/** Check names that already have a glossary record, so the name is a trigger where one exists. */
const GLOSS_CHECK: Record<string, "walk-forward" | "reality-check" | "generation-reality-check" | "pbo" | "plateau" | "lockbox"> = {
  "walk-forward": "walk-forward",
  "reality check": "reality-check",
  "generation reality check": "generation-reality-check",
  "overfitting (PBO)": "pbo",
  plateau: "plateau",
  lockbox: "lockbox",
};

// ---- the lineage tree ----

type Row = { node: GenNode; depth: number };

function flatten(nodes: GenNode[], depth: number, out: Row[]): Row[] {
  for (const node of nodes) {
    out.push({ node, depth });
    flatten(node.children, depth + 1, out);
  }
  return out;
}

/** Every generated family on the page, judged or not. */
function allNodes(gens: Generations): GenNode[] {
  return gens.roots.flatMap((root) => flatten(root.children, 0, []).map((r) => r.node));
}

function judgedNodes(gens: Generations): GenNode[] {
  return allNodes(gens).filter((n) => n.state !== "proposed");
}

/** Judged and put through the checks. "Not enough evidence" with no failed checks never reached them, so it has no distance. */
function unscored(n: GenNode): boolean {
  return n.state === "INSUFFICIENT_EVIDENCE" && checks(n.failed).length === 0;
}

/** Scored in the ledger's current era: earlier verdicts faced the old bar and are not ranked with these. */
function scoredNodes(gens: Generations): GenNode[] {
  return judgedNodes(gens).filter((n) => !unscored(n) && !n.prior);
}

/** The gate's own check order (`src/lab/gate.rs`), so ties read in the order a family meets them. */
const CHECK_ORDER = [
  "evidence", "walk-forward", "reality check", "generation reality check", "deflated Sharpe", "overfitting (PBO)", "plateau",
  "costs ×2", "one bar late", "coins", "quarters", "best days removed", "timing baseline", "market-neutral part", "lockbox",
];

/** How many families each check stopped, and the family with the fewest failures. */
function checkTally(nodes: GenNode[]) {
  const counts = new Map<string, number>();
  for (const n of nodes) {
    for (const c of checks(n.failed)) counts.set(c, (counts.get(c) ?? 0) + 1);
  }
  const rank = (name: string) => (CHECK_ORDER.indexOf(name) < 0 ? 99 : CHECK_ORDER.indexOf(name));
  const rows = [...counts].map(([name, n]) => ({ name, n })).sort((a, b) => b.n - a.n || rank(a.name) - rank(b.name));
  let closest: { node: GenNode; n: number } | null = null;
  for (const node of nodes) {
    const n = checks(node.failed).length;
    if (n === 0) continue;
    // Ties go to the earlier generation, then the name: the Lab page's order, so both name the same family.
    if (!closest || n < closest.n || (n === closest.n && (node.generation - closest.node.generation || node.name.localeCompare(closest.node.name)) < 0)) closest = { node, n };
  }
  return { rows, closest };
}

function Lineage({ gens }: { gens: Generations }) {
  if (gens.roots.length === 0) return null;
  const n = allNodes(gens).length;
  return (
    <details className="group rounded-lg border bg-panel">
      <summary className="flex cursor-pointer list-none flex-wrap items-center gap-x-3 gap-y-1 px-5 py-3">
        <ChevronRight aria-hidden className="size-3.5 shrink-0 text-muted-foreground transition-transform group-open:rotate-90" />
        <span className="text-sm font-medium">Family tree</span>
        <span className="text-[13px] text-muted-foreground">
          {plural(n, "family", "families")} under the {plural(gens.roots.length, "hand-written family", "hand-written families")} they came from
        </span>
      </summary>
      <DataTable minWidth={760} label="Generated families by parent, with state, generation, operator, study and trials charged">
        <thead>
          <tr>
            <Th>Family</Th>
            <Th>State</Th>
            <Th className="text-right"><Gloss id="generation">Generation</Gloss></Th>
            <Th><Gloss id="operator">Operator</Gloss></Th>
            <Th>Study</Th>
            <Th className="text-right"><Gloss id="trial">Trials</Gloss></Th>
          </tr>
        </thead>
        {gens.roots.map((root) => (
          <tbody key={root.family || "(none)"}>
            <tr className="bg-muted/30">
              <Td colSpan={6} className="font-medium">
                {root.family ? familyName(root.family) : "No parent"}
                <span className="ml-2 text-xs font-normal text-faint">
                  {root.family ? <span className="font-mono">{root.family}</span> : "exploration"}
                  {root.family && " · hand-written"}
                </span>
              </Td>
            </tr>
            {flatten(root.children, 0, []).map(({ node, depth }) => {
              const k = checks(node.failed).length;
              return (
                <tr key={node.family}>
                  <Td>
                    <div className="flex items-baseline gap-2 whitespace-nowrap" style={{ paddingLeft: depth * 20 }}>
                      <span aria-hidden className="text-faint">└</span>
                      <span className="font-mono font-medium">{node.name}</span>
                      {node.name !== node.family && <span className="hidden font-mono text-xs text-faint sm:inline">{node.family}</span>}
                    </div>
                  </Td>
                  <Td>
                    <div className="flex items-center gap-2 whitespace-nowrap">
                      <Tag kind={node.state} />
                      {k > 0 && <span className="text-xs text-faint" title={node.failed}>{plural(k, "check")} failed</span>}
                    </div>
                  </Td>
                  <Td className="text-right tabular-nums">{node.generation}</Td>
                  <Td>{node.operator || "–"}</Td>
                  <Td className="whitespace-nowrap font-mono text-xs text-muted-foreground">{node.study || "–"}</Td>
                  <Td className="text-right tabular-nums">{node.trials}</Td>
                </tr>
              );
            })}
          </tbody>
        ))}
      </DataTable>
    </details>
  );
}

/** The gate in the loop's own numbers: which check stopped how many of the families it judged. */
function Stops({ gens }: { gens: Generations }) {
  const nodes = scoredNodes(gens);
  if (nodes.length === 0) return null;
  const { rows } = checkTally(nodes);
  const all = rows.filter((r) => r.n === nodes.length);
  const latestGen = nodes.reduce((m, n) => Math.max(m, n.generation), 0);
  const latest = nodes.filter((n) => n.generation === latestGen);
  const inLatest = (name: string) => latest.filter((n) => checks(n.failed).includes(name)).length;
  return (
    <Panel
      flush
      title={<h3 className="text-[15px] font-medium tracking-tight">What stands in the way</h3>}
      description={
        <>
          {all.length > 0 ? (
            <>
              Every one failed <span className="text-foreground">{all.map((r) => r.name).join(" and ")}</span>. A family passes
              only by failing none of them.{" "}
            </>
          ) : (
            "No check stopped all of them. "
          )}
          The last column shows whether generation {latestGen} got past a check the earlier ones kept failing.
        </>
      }
    >
      <DataTable minWidth={720} label="Gate checks, what each asks, and how many generated families failed it, overall and in the latest generation">
        <thead>
          <tr>
            <Th>Check</Th>
            <Th>What it asks</Th>
            <Th className="text-right">All generations</Th>
            <Th className="text-right">Generation {latestGen}</Th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => {
            const id = GLOSS_CHECK[r.name];
            return (
              <tr key={r.name}>
                <Td className="whitespace-nowrap">{id ? <Gloss id={id}>{r.name}</Gloss> : r.name}</Td>
                <Td className="text-muted-foreground">{CHECK_ASKS[r.name] ?? "–"}</Td>
                <Td className="text-right tabular-nums">
                  {r.n} of {nodes.length}
                </Td>
                <Td className={cn("text-right tabular-nums", inLatest(r.name) === 0 ? "text-pos" : "text-muted-foreground")}>
                  {inLatest(r.name)} of {latest.length}
                </Td>
              </tr>
            );
          })}
        </tbody>
      </DataTable>
    </Panel>
  );
}

const eraDate = (iso: string) => new Date(iso).toLocaleString("en-GB", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", timeZone: "UTC" }) + " UTC";

/** Loop trials, said against the ledger's era when it restarted. */
function trialsLine(gens: Generations): string {
  return gens.era ? `${plural(gens.trials, "trial")} since the ledger restarted ${eraDate(gens.era)}` : `${plural(gens.trials, "trial")}`;
}

/** The luck bar on a study: the earliest cycle's "before" and the newest cycle's "after". */
function luckSpan(reports: CycleReport[], study: string, era: string | null): { before: number; after: number } | null {
  const real = reports.filter((r) => !r.dry_run && r.studies.some((s) => s.study === study) && (!era || (r.at !== null && Date.parse(r.at) >= Date.parse(era))));
  if (real.length === 0) return null;
  const before = real[real.length - 1].studies.find((s) => s.study === study);
  const after = real[0].studies.find((s) => s.study === study);
  if (!before || !after) return null;
  return { before: before.luck_bar_before, after: after.luck_bar_after };
}

// ---- one cycle report ----

function Section({ title, children }: { title: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="border-t">
      <h4 className="px-5 pt-4 pb-2 text-sm font-medium">{title}</h4>
      {children}
    </div>
  );
}

function Proposal({ p, r, gens }: { p: ProposalRow; r: CycleReport; gens: Generations }) {
  const promotable = r.promotable.includes(p.name);
  return (
    <li className="border-t px-5 py-4">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <span className="font-mono text-sm font-medium">{p.name || p.family}</span>
        <span className="text-[13px] text-muted-foreground">
          {p.operator || "–"} of {parentName(gens, p.parent)}
        </span>
        <span className="ml-auto flex items-center gap-2">
          {promotable && <Tag kind="accepted">Promotable</Tag>}
          {p.verdict ? <Tag kind={p.verdict} /> : <Tag kind="proposed">{r.dry_run ? "Not simulated" : "Not judged"}</Tag>}
        </span>
      </div>
      <p className="mt-2 max-w-[72ch] text-[13px] leading-5 text-pretty">{reasonText(p)}</p>
      {p.failed.length > 0 && (
        <p className="mt-1 max-w-[72ch] text-[13px] leading-5 text-muted-foreground">
          <span className="text-foreground">Failed {p.failed.length}:</span> {p.failed.join(", ")}
        </p>
      )}
      <p className="mt-1.5 max-w-[72ch] font-mono text-[12px] leading-5 break-words text-muted-foreground">{p.rule || "–"}</p>
      <p className="mt-1 max-w-[72ch] text-[12px] leading-4 break-words text-faint">
        {plural(p.specs, "rule")}, {plural(p.new_trials, "new trial")}
        {r.dry_run ? " if run" : ""}
        {p.pick && <span className="font-mono"> · pick {p.pick}</span>}
      </p>
    </li>
  );
}

function bps(x: number): string {
  return signed(x, 1);
}

/** The proposer repeats the approximate-parent caveat inside every reason; the section lead says it once. */
function reasonText(p: ProposalRow): string {
  if (!p.approximate) return p.reason || "–";
  const cut = p.reason.indexOf("The parent is hand-written");
  return cut > 0 ? p.reason.slice(0, cut).trim() : p.reason || "–";
}

function PairTable({ rows, child, parent }: { rows: Pairing[]; child: string; parent: string }) {
  return (
    <DataTable
      minWidth={740}
      label={`${child} against ${parent}: mean bps per day on common days, with the difference and its 95% interval`}
    >
      <thead>
        <tr>
          <Th>{child}</Th>
          <Th>{parent}</Th>
          <Th className="text-right">Common days</Th>
          <Th className="text-right">{child} bps/day</Th>
          <Th className="text-right">{parent} bps/day</Th>
          <Th className="text-right">Difference bps/day [95%]</Th>
          <Th className="text-right">Correlation</Th>
        </tr>
      </thead>
      <tbody>
        {rows.map((p, i) => (
          <tr key={`${p.challenger}-${p.incumbent}-${i}`}>
            <Td className="font-mono text-xs whitespace-nowrap">
              {p.challenger} {p.duplicate && <Tag kind="duplicate" />}
            </Td>
            <Td className="whitespace-nowrap">
              {p.incumbent}
              {p.incumbent_spec && (
                <div className="max-w-[260px] truncate font-mono text-[12px] text-faint" title={p.incumbent_spec}>
                  {p.incumbent_spec}
                </div>
              )}
            </Td>
            <Td className="text-right tabular-nums">{p.days}</Td>
            <Td className="text-right tabular-nums">{bps(p.challenger_bps)}</Td>
            <Td className="text-right tabular-nums">{bps(p.incumbent_bps)}</Td>
            <Td className="text-right whitespace-nowrap tabular-nums">
              {bps(p.diff_bps)} <span className="text-faint">[{bps(p.lo_bps)}, {bps(p.hi_bps)}]</span>
            </Td>
            <Td className="text-right tabular-nums">{signed(p.corr, 2)}</Td>
          </tr>
        ))}
      </tbody>
    </DataTable>
  );
}

/** The one-line answer from a comparison table: how many intervals clear zero, how many are duplicates. */
function pairStats(rows: Pairing[]) {
  return {
    total: rows.length,
    above: rows.filter((p) => p.lo_bps > 0).length,
    duplicates: rows.filter((p) => p.duplicate).length,
  };
}

function Comparison({ r, kind }: { r: CycleReport; kind: "design" | "paper" }) {
  const rows = kind === "design" ? r.pairings : r.forward;
  if (rows.length === 0) return null;
  const s = pairStats(rows);
  const sorted = kind === "design" ? [...rows].sort((a, b) => Number(b.duplicate) - Number(a.duplicate) || b.diff_bps - a.diff_bps) : rows;
  return (
    <Section title={kind === "design" ? "Against the incumbents, design window" : "On paper: child against parent"}>
      <details className="group/table">
        <summary className="flex cursor-pointer list-none flex-wrap items-center gap-x-2 gap-y-1 px-5 pb-3 text-[13px] leading-5">
          <ChevronRight
            aria-hidden
            className="size-3.5 shrink-0 text-muted-foreground transition-transform group-open/table:rotate-90"
          />
          <span className="text-foreground">
            {plural(s.total, "comparison")} · {s.above} where the 95% interval sits above 0 · {plural(s.duplicates, "duplicate")}
          </span>
          <span className="text-muted-foreground">
            {kind === "design"
              ? "In sample for both sides: a difference here is a reason to look, not evidence. Sorted by difference, duplicates first."
              : "Mean paper return over the days both ran. A few paper days decide nothing."}
          </span>
        </summary>
        <PairTable
          rows={sorted}
          child={kind === "design" ? "Challenger" : "Child"}
          parent={kind === "design" ? "Incumbent" : "Parent"}
        />
        {kind === "design" && (
          <p className="border-t px-5 py-3 text-xs leading-4 text-faint">
            A correlation of 0.9 or more marks a duplicate, which is never promotable.
          </p>
        )}
      </details>
    </Section>
  );
}

function OnPaper({ r }: { r: CycleReport }) {
  if (r.forward.length === 0) return null;
  return <Comparison r={r} kind="paper" />;
}

/** The conclusion of a cycle, read first: what the operator has to do about it. */
function outcome(r: CycleReport): React.ReactNode {
  const lock =
    r.lockbox_openings > 0 ? (
      <>
        {" "}The <Gloss id="lockbox">lockbox</Gloss> was opened {plural(r.lockbox_openings, "time")} in this generation, which makes
        the next opening harder to pass.
      </>
    ) : null;
  if (r.dry_run)
    return (
      <>
        Dry run: nothing was simulated, nothing was charged, and no family file was written. The would-be files are in{" "}
        <Code>{`lab-data/reports/evolve/dry-g${String(r.generation).padStart(3, "0")}/`}</Code>.
      </>
    );
  if (r.promotable.length === 0)
    return (
      <>
        <span className="text-foreground">Nothing to promote.</span>
        {lock}
      </>
    );
  return (
    <>
      Passed the gate and not a duplicate: <span className="font-mono">{r.promotable.join(", ")}</span>. Promoting is your
      decision, in the terminal: <Code>jev-trader lab promote --family families/gen/&lt;name&gt;.toml</Code>.{lock}
    </>
  );
}

/** Trials a cycle charged: the sum of what it added to each study's ledger. */
function trialsCharged(r: CycleReport): number {
  return r.studies.reduce((n, c) => n + Math.max(0, c.trials_after - c.trials_before), 0);
}

/** Proposer notes are mostly one reason repeated; when they are, say it once and list what it applies to. */
function notesLine(messages: string[]): string {
  const parts = messages.map((m) => {
    const i = m.indexOf(": ");
    return i < 0 ? null : ([m.slice(0, i), m.slice(i + 2)] as const);
  });
  const first = parts[0];
  if (first && parts.every((p) => p && p[1] === first[1])) {
    return `${plural(messages.length, "more proposal")} skipped: ${parts.map((p) => p![0]).join(", ")} — ${first[1]}, so they cost nothing.`;
  }
  return messages.join(" · ");
}

function summaryLine(r: CycleReport): string {
  const date = r.at ? dateShort(r.at) : "time unknown";
  if (r.dry_run)
    return [
      `Generation ${r.generation}`,
      `${plural(r.proposals.length, "family", "families")} would be proposed`,
      `${plural(trialsCharged(r), "trial")} would be charged`,
      date,
    ].join(" · ");
  const words: Record<string, string> = { PROMOTE_TO_PAPER: "passed", REVISE: "to revise", REJECT: "rejected", INSUFFICIENT_EVIDENCE: "not enough evidence" };
  const tally = new Map<string, number>(Object.values(words).map((w) => [w, 0]));
  for (const p of r.proposals) {
    const w = p.verdict ? (words[p.verdict] ?? p.verdict.toLowerCase()) : "not judged";
    tally.set(w, (tally.get(w) ?? 0) + 1);
  }
  return [
    `${plural(r.proposals.length, "family", "families")} proposed`,
    ...[...tally].filter(([, n]) => n > 0).map(([w, n]) => `${n} ${w}`),
    `${plural(trialsCharged(r), "trial")} charged`,
    date,
  ].join(" · ");
}

function Report({ r, gens, open }: { r: CycleReport; gens: Generations; open: boolean }) {
  return (
    <details id={r.file.replace(/\.json$/, "")} open={open} className="group scroll-mt-20 rounded-lg border bg-panel">
      <summary className="flex cursor-pointer list-none flex-wrap items-center gap-x-3 gap-y-1 px-5 py-3">
        <ChevronRight aria-hidden className="size-3.5 shrink-0 text-muted-foreground transition-transform group-open:rotate-90" />
        <span className="text-sm font-medium">{r.dry_run ? "Dry run" : `Generation ${r.generation}`}</span>
        <span className="text-[13px] text-muted-foreground">{summaryLine(r)}</span>
        <span className="ml-auto font-mono text-[12px] text-faint">{r.file}</span>
      </summary>
      <div className="border-t">
        <div className="px-5 py-3">
          <p className="max-w-[72ch] text-[13px] leading-5 text-pretty text-muted-foreground">{outcome(r)}</p>
        </div>
        <Section title={r.dry_run ? "Would propose" : "Proposals"}>
          {!r.dry_run && r.proposals.length > 0 && (
            <p className="max-w-[72ch] px-5 pb-3 text-[13px] leading-5 text-pretty text-muted-foreground">
              Built after the design window had been looked at, so a generated family&apos;s walk-forward folds are not truly out
              of sample: the lockbox and paper days are its only clean evidence. The L-numbers are the lessons it was built from.
              A child of a hand-written family rebuilds it approximately, so it is not an exact variant.
            </p>
          )}
          {r.proposals.length === 0 ? (
            <p className="px-5 pb-4 text-[13px] text-muted-foreground">None. The proposer&apos;s notes below say why.</p>
          ) : (
            <ul>
              {r.proposals.map((p) => (
                <Proposal key={p.family || p.name} p={p} r={r} gens={gens} />
              ))}
            </ul>
          )}
        </Section>
        <Comparison r={r} kind="design" />
        <OnPaper r={r} />
        {r.messages.length > 0 && (
          <Section title="Notes from the proposer">
            <p className="max-w-[72ch] px-5 pb-4 text-[13px] leading-5 text-pretty text-muted-foreground">{notesLine(r.messages)}</p>
          </Section>
        )}
      </div>
    </details>
  );
}


// ---- runs, the schedule and the budget ----

const UTC_DAY_TIME: Intl.DateTimeFormatOptions = { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", timeZone: "UTC" };

function when(iso: string | null): string {
  return iso ? `${new Date(iso).toLocaleString("en-GB", UTC_DAY_TIME)} UTC` : "time unknown";
}

function stopWords(stop: string, gens: Generations): string {
  switch (stop) {
    case "passed":
      return "a family passed the gate";
    case "nothing_new":
      return "nothing new was left to propose or the trial budget was spent";
    case "max_generations":
      return `it reached the ${gens.cfg.auto_max_generations}-generation cap`;
    case "stalled":
      return `${plural(gens.cfg.auto_stall, "generation")} in a row produced nothing better than rejected`;
    default:
      return stop || "unknown";
  }
}

/** Which run each generation belongs to (index into `gens.runs`), or -1 when it was run by hand. */
function runOf(gens: Generations): Map<number, number> {
  const m = new Map<number, number>();
  gens.runs.forEach((r, i) => r.generations.forEach((g) => m.set(g.generation, i)));
  return m;
}

/** Trials per study left in the loop's lifetime budget, and the recent pace per run. */
function runway(gens: Generations) {
  return gens.byStudy.map((s) => {
    const budget = gens.cfg.auto_trial_budgets[s.study] ?? gens.cfg.auto_trial_budget;
    const spends: number[] = [];
    let prev = 0;
    // Runs before the ledger's era counted the old trials; pace comes from this era's runs only.
    for (const r of gens.runs.filter((r) => !gens.era || (r.started !== null && Date.parse(r.started) >= Date.parse(gens.era)))) {
      const n = r.loop_trials.find((x) => x.study === s.study)?.n ?? prev;
      if (n > prev) spends.push(n - prev);
      prev = Math.max(prev, n);
    }
    const pace = spends.length ? spends.reduce((a, b) => a + b, 0) / spends.length : 0;
    const left = Math.max(0, budget - s.loop);
    return { study: s.study, used: s.loop, left, budget, runsLeft: pace > 0 ? Math.floor(left / pace) : null, luck: luckSpan(gens.reports, s.study, gens.era) };
  });
}

function HowItRuns({ gens }: { gens: Generations }) {
  const { schedule, cfg, runs } = gens;
  const last = schedule.ticks[schedule.ticks.length - 1];
  const lastRun = runs[runs.length - 1];
  const cells = [
    {
      name: "Every hour",
      body:
        schedule.minute !== null ? (
          <>
            Your crontab starts a run at :{String(schedule.minute).padStart(2, "0")} past each hour, after fetching the newest Binance
            candles.{schedule.next && ` Next start ${clock(schedule.next)} UTC.`}
          </>
        ) : schedule.cron ? (
          <>
            Your crontab starts a run on <Code>{schedule.cron}</Code>.
          </>
        ) : (
          <>Not scheduled: no crontab line runs it, so it starts only when you run <Code>{EVOLVE} --auto</Code>.</>
        ),
      meta: last ? `Last start ${when(last.at)}${last.skipped ? ", skipped: the one before was still running" : last.exit ? `, exited ${last.exit}` : ""}` : "No start logged yet",
    },
    {
      name: "Each run",
      body: (
        <>
          Repeats generations until a stop rule: a family passes, nothing new is left, the trial budget is spent, or{" "}
          {plural(cfg.auto_stall, "generation")} in a row come out no better than rejected.
        </>
      ),
      meta: lastRun ? `${plural(runs.length, "run")} so far. The last stopped because ${stopWords(lastRun.stop, gens)}.` : "No run yet",
    },
    {
      name: "Each generation",
      body: (
        <>
          Turns what the earlier families failed into up to {plural(cfg.proposals, "new family", "new families")}, charges their
          trials, and has the gate judge them together. A child inherits nothing from its parent&apos;s record.
        </>
      ),
      meta: `${cfg.specs_per_proposal} rules per family, at most ${cfg.trials_per_cycle} trials per generation`,
    },
    {
      name: "The goal",
      body: (
        <>
          One family that fails none of the gate&apos;s checks. That only opens the{" "}
          <Link href="/paper" className="text-foreground underline decoration-border underline-offset-4 hover:decoration-foreground">
            paper run
          </Link>
          ; the loop promotes nothing, and promoting stays your decision.
        </>
      ),
      meta: "",
    },
  ];
  const way = runway(gens);
  return (
    <details className="group overflow-hidden rounded-lg border bg-panel">
      <summary className="flex cursor-pointer list-none flex-wrap items-center gap-x-3 gap-y-1 px-5 py-3">
        <ChevronRight aria-hidden className="size-3.5 shrink-0 text-muted-foreground transition-transform group-open:rotate-90" />
        <span className="text-sm font-medium">How the loop runs</span>
        <span className="text-[13px] text-muted-foreground">
          {[
            schedule.minute !== null ? `Hourly at :${String(schedule.minute).padStart(2, "0")}` : schedule.cron ? "Scheduled" : "Not scheduled",
            `${plural(runs.length, "run")} so far`,
            lastRun ? `last stopped because ${stopWords(lastRun.stop, gens)}` : null,
            `${plural(gens.trials, "trial")} this era`,
          ].filter(Boolean).join(" · ")}
        </span>
      </summary>
      <div className="border-t">
        <ol className="grid gap-px bg-border sm:grid-cols-2 lg:grid-cols-4">
          {cells.map((c) => (
            <li key={c.name} className="flex min-w-0 flex-col bg-panel p-4">
              <p className="text-[13px] font-medium">{c.name}</p>
              <p className="mt-1.5 text-[13px] leading-5 text-pretty text-muted-foreground">{c.body}</p>
              {c.meta && <p className="mt-auto pt-3 text-[12px] leading-4 text-faint">{c.meta}</p>}
            </li>
          ))}
        </ol>
        {way.length > 0 && (
          <div className="border-t px-5 py-4">
            <p className="text-[13px] font-medium">
              <Gloss id="trial-budget">Trial budget</Gloss>
            </p>
            <p className="mt-1 max-w-[72ch] text-[13px] leading-5 text-pretty text-muted-foreground">
              Every <Gloss id="trial">trial</Gloss> raises the <Gloss id="luck-bar">luck bar</Gloss> every family on that study must
              clear.{" "}
              {gens.cfg.auto_trials_per_run
                ? `Each hourly run may charge up to ${gens.cfg.auto_trials_per_run} new trials per study.`
                : "The loop gets a fixed number for its whole life; when it is spent, the hourly runs stop at once."}
              {gens.era && ` Counts start ${eraDate(gens.era)}, when the ledger restarted; earlier trials are archived and no longer raise the bar.`}
            </p>
            <ul className="mt-3 grid gap-x-10 gap-y-3 sm:grid-cols-2">
              {way.map((w) => (
                <li key={w.study} className="min-w-0">
                  <div className="flex items-baseline justify-between gap-3 text-[13px]">
                    <span className="font-mono text-xs">{w.study}</span>
                    <span className="tabular-nums">
                      {gens.cfg.auto_trials_per_run ? `${plural(w.used, "trial")}${gens.era ? " this era" : ""}` : `${w.used} of ${w.budget} used`}
                    </span>
                  </div>
                  {!gens.cfg.auto_trials_per_run && (
                    <div aria-hidden className="mt-1.5 h-[3px] w-full bg-foreground/15">
                      <div className="h-full bg-foreground" style={{ width: `${Math.min(100, (w.used / Math.max(1, w.budget)) * 100)}%` }} />
                    </div>
                  )}
                  <p className="mt-1.5 text-[12px] leading-4 text-faint">
                    {gens.cfg.auto_trials_per_run
                      ? `Up to ${gens.cfg.auto_trials_per_run} per run`
                      : w.left === 0
                      ? "Spent: runs no longer search this study."
                      : w.runsLeft === null
                        ? `${w.left} left`
                        : w.runsLeft === 0
                          ? `${w.left} left, less than one run's worth`
                          : `${w.left} left, about ${plural(w.runsLeft, "run")} at the recent pace`}
                    {w.luck && <span className="font-mono"> · luck bar {bar(w.luck.before)} → {bar(w.luck.after)}</span>}
                  </p>
                </li>
              ))}
            </ul>
          </div>
        )}
        <GlossList
          ids={[
            "run", "generation", "family", "operator", "checks-failed", "trial", "trial-budget", "luck-bar", "gate",
            "generation-reality-check", "walk-forward", "reality-check", "deflated-sharpe", "lockbox", "design-window", "duplicate", "dry-run",
          ]}
          intro="Every underlined word opens the same record."
        />
      </div>
    </details>
  );
}

// ---- progress: distance to the goal, generation by generation ----

type Col = { generation: number; points: { node: GenNode; n: number }[]; record: number | null };

/** This era's scored families by generation, with the closest any family had come by then (it can only fall). */
function columns(nodes: GenNode[]): Col[] {
  if (nodes.length === 0) return [];
  const lo = Math.min(...nodes.map((n) => n.generation));
  const hi = Math.max(...nodes.map((n) => n.generation));
  const out: Col[] = [];
  let record: number | null = null;
  for (let g = lo; g <= hi; g++) {
    const points = nodes.filter((n) => n.generation === g).map((node) => ({ node, n: checks(node.failed).length })).sort((a, b) => a.n - b.n);
    if (points.length && (record === null || points[0].n < record)) record = points[0].n;
    out.push({ generation: g, points, record });
  }
  return out;
}

/** One sentence: where the record was set, and whether anything since has matched or beaten it. */
function trendLine(cols: Col[]): string {
  const last = cols[cols.length - 1];
  if (!last || last.record === null) return "";
  const best = last.record;
  const setAt = cols.find((c) => c.record === best)!;
  const who = setAt.points[0].node.name;
  if (best === 0) return `${who} reached the goal in generation ${setAt.generation}.`;
  const after = cols.filter((c) => c.generation > setAt.generation && c.points.length);
  const matched = after.filter((c) => c.points[0].n === best).length;
  const since = after.length === 0 ? "" : matched === 0 ? ` None of the ${plural(after.length, "generation")} since has come as close.` : ` In the ${plural(after.length, "generation")} since, ${matched} matched it and none beat it.`;
  return `Closest so far: ${who}, ${plural(best, "check")} short, in generation ${setAt.generation}.${since}`;
}

/** Why a check gets harder over time, when it does. */
const CHECK_MOVES: Record<string, string> = {
  "deflated Sharpe": "its bar rises with every trial on the ledger",
  lockbox: "its threshold tightens with every opening",
  "generation reality check": "it counts every rule the generation tried",
};

/** The measured part of a check value, without the working in brackets. */
function measured(c: Check): string {
  const p = c.value.match(/p = [\d.]+/);
  if (c.name === "lockbox" && p) return p[0];
  return c.value.replace(/\s*\(.*$/, "").split(/;| · | over | on |, (?=Sharpe)/)[0].trim();
}

function needed(c: Check): string {
  const p = c.need.match(/p < [\d.]+/);
  return c.name === "lockbox" && p ? p[0] : c.need;
}

function NearMisses({ gens }: { gens: Generations }) {
  const rows = gens.near;
  if (rows.length === 0) return null;
  const one = rows.filter((r) => r.failed.length === 1);
  const missed = new Map<string, number>();
  for (const r of one) missed.set(r.failed[0].name, (missed.get(r.failed[0].name) ?? 0) + 1);
  const named = [...missed].sort((a, b) => b[1] - a[1]);
  const moving = named.filter(([name]) => CHECK_MOVES[name]);
  return (
    <div className="-mx-5 mt-6 -mb-5 border-t">
      <div className="px-5 pt-4 pb-3">
        <h4 className="text-sm font-medium">How short is short</h4>
        <p className="mt-1 max-w-[72ch] text-[13px] leading-5 text-pretty text-muted-foreground">
          {one.length > 0 ? (
            <>
              The {plural(one.length, "family", "families")} one check short missed{" "}
              <span className="text-foreground">{named.map(([name, n]) => `${name} (${n})`).join(", ")}</span>.{" "}
              {moving.length > 0 && <>That bar moves: {moving.map(([name]) => `for ${name}, ${CHECK_MOVES[name]}`).join("; ")}. </>}
            </>
          ) : (
            "No family this era is one check short. "
          )}
          Each row is what the gate measured against what it needed.
        </p>
      </div>
      <DataTable minWidth={720} label="Families one or two checks short: each missed check, its measured value and what it needed">
        <thead>
          <tr>
            <Th>Family</Th>
            <Th>Missed</Th>
            <Th className="text-right">Measured</Th>
            <Th className="text-right">Needed</Th>
            <Th className="text-right"><Gloss id="trial">Trials</Gloss> on ledger</Th>
            <Th className="text-right">Judged</Th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.family} className="align-top">
              <Td className="whitespace-nowrap">
                <span className="font-mono font-medium">{r.name}</span>
                <span className="ml-2 text-xs text-faint">{r.generation ? `generation ${r.generation}` : "hand-written"}</span>
              </Td>
              <Td className="whitespace-nowrap">
                {r.failed.map((c) => (
                  <div key={c.name}>{GLOSS_CHECK[c.name] ? <Gloss id={GLOSS_CHECK[c.name]}>{c.name}</Gloss> : c.name}</div>
                ))}
              </Td>
              <Td className="text-right font-mono text-[12px] whitespace-nowrap">
                {r.failed.map((c) => (
                  <div key={c.name} title={c.value}>{measured(c)}</div>
                ))}
              </Td>
              <Td className="text-right font-mono text-[12px] whitespace-nowrap text-muted-foreground">
                {r.failed.map((c) => (
                  <div key={c.name} title={c.need}>{needed(c)}</div>
                ))}
              </Td>
              <Td className="text-right tabular-nums">{r.trials_ledger}</Td>
              <Td className="text-right whitespace-nowrap text-muted-foreground">{when(r.judged)}</Td>
            </tr>
          ))}
        </tbody>
      </DataTable>
    </div>
  );
}

function Progress({ gens }: { gens: Generations }) {
  const judged = judgedNodes(gens);
  const now = judged.filter((n) => !n.prior);
  const scored = now.filter((n) => !unscored(n));
  const cols = columns(scored);
  if (cols.length === 0) return null;
  const prior = judged.length - now.length;
  const thin = now.length - scored.length;
  const worst = Math.max(...scored.map((n) => checks(n.failed).length));
  const top = Math.max(4, Math.ceil(worst / 2) * 2);
  const ticks = Array.from({ length: top / 2 + 1 }, (_, i) => i * 2);
  const y = (n: number) => (n / top) * 100;
  const x = (i: number) => ((i + 0.5) / cols.length) * 100;
  // The record as a step line: flat until a family comes closer, then down.
  const steps: string[] = [];
  cols.forEach((c, i) => {
    if (c.record === null) return;
    const prev = steps.length ? cols[i - 1]?.record : null;
    if (prev !== null && prev !== undefined && prev !== c.record) steps.push(`${x(i)},${y(prev)}`);
    steps.push(`${x(i)},${y(c.record)}`);
  });
  const lastRecord = cols[cols.length - 1].record;
  if (lastRecord !== null) steps.push(`100,${y(lastRecord)}`);
  const setters = new Set(cols.filter((c, i) => c.points.length && c.points[0].n === c.record && (i === 0 || cols[i - 1].record !== c.record)).map((c) => c.generation));
  const xTicks = cols.filter((c, i) => i === 0 || i === cols.length - 1 || c.generation % 10 === 0);
  return (
    <Panel
      title={<h3 className="text-[15px] font-medium tracking-tight">Distance to the goal</h3>}
      description={
        <>
          Each dot is a family, placed by its <Gloss id="checks-failed">checks failed</Gloss>; the goal is 0, at the top. The line is the
          closest any family has come so far. {trendLine(cols)}
        </>
      }
    >
      <div className="flex gap-3">
        <div aria-hidden className="relative h-[200px] w-6 shrink-0 font-mono text-[11px] text-faint tabular-nums">
          {ticks.map((t) => (
            <span key={t} className="absolute right-0 -translate-y-1/2" style={{ top: `${y(t)}%` }}>
              {t}
            </span>
          ))}
        </div>
        <div className="min-w-0 flex-1">
          <div
            role="img"
            aria-label={`Checks failed by generation, ${scored.length} families. ${trendLine(cols)}`}
            className="relative h-[200px]"
          >
            {ticks.map((t) => (
              <div
                key={t}
                aria-hidden
                className={cn("absolute inset-x-0 border-t", t === 0 ? "border-dashed border-pos/60" : "border-border/60")}
                style={{ top: `${y(t)}%` }}
              />
            ))}
            <span aria-hidden className="absolute right-0 -top-1 -translate-y-full text-[11px] text-pos">Goal: passes the gate</span>
            {cols.map((c, i) =>
              c.points.map((p, k) => {
                const same = c.points.filter((q) => q.n === p.n);
                const dx = (same.indexOf(p) - (same.length - 1) / 2) * 3;
                const setter = k === 0 && setters.has(c.generation);
                return (
                  <span
                    key={p.node.family}
                    title={`${p.node.name}, generation ${c.generation}: ${plural(p.n, "check")} failed${p.node.failed ? ` (${p.node.failed})` : ""}`}
                    className={cn(
                      "absolute -translate-x-1/2 -translate-y-1/2 rounded-full",
                      p.n === 0 ? "size-2 bg-pos" : setter ? "size-2 bg-foreground" : "size-1.5 bg-foreground/30",
                    )}
                    style={{ top: `${y(p.n)}%`, left: `calc(${x(i)}% + ${dx}px)` }}
                  />
                );
              }),
            )}
            <svg aria-hidden viewBox="0 0 100 100" preserveAspectRatio="none" className="pointer-events-none absolute inset-0 size-full overflow-visible">
              <polyline points={steps.join(" ")} fill="none" stroke="var(--foreground)" strokeWidth={1.5} vectorEffect="non-scaling-stroke" />
            </svg>
            {cols.map((c, i) =>
              setters.has(c.generation) ? (
                <span
                  key={c.generation}
                  aria-hidden
                  className={cn("absolute -translate-y-1/2 font-mono text-[11px] text-foreground tabular-nums", i > 0 && "-translate-x-full")}
                  style={{ top: `calc(${y(c.points[0].n)}% + 9px)`, left: i > 0 ? `calc(${x(i)}% - 6px)` : `calc(${x(i)}% + 6px)` }}
                >
                  {c.points[0].node.name}
                </span>
              ) : null,
            )}
          </div>
          <div aria-hidden className="relative mt-2 h-4 font-mono text-[11px] text-faint tabular-nums">
            {xTicks.map((c) => (
              <span key={c.generation} className="absolute -translate-x-1/2" style={{ left: `${x(cols.indexOf(c))}%` }}>
                {c.generation}
              </span>
            ))}
          </div>
          <p className="mt-1 text-center text-[11px] text-faint">Generation</p>
        </div>
      </div>
      <p className="mt-4 max-w-[90ch] text-[12px] leading-4 text-faint">
        Drawn: the {plural(scored.length, "family", "families")} the loop bred and the gate scored
        {gens.era ? ` since the ledger restarted ${eraDate(gens.era)}` : ""}. Not drawn: {plural(thin, "family", "families")} with too
        few trades to reach the checks{prior > 0 && `, ${prior} judged against the old ledger's higher bar`}, and the{" "}
        {plural(gens.handWritten, "hand-written family", "hand-written families")}, which are on the{" "}
        <Link href="/lab" className="text-muted-foreground underline decoration-border underline-offset-4 hover:decoration-foreground">Lab</Link>. Checks
        are not equal: a count is a distance, not a score.
      </p>
      <NearMisses gens={gens} />
    </Panel>
  );
}

// ---- the page ----

export function GenerationsView({ gens }: { gens: Generations }) {
  const head = (verdict: React.ReactNode) => (
    <PageHead
      question="Is the loop getting closer?"
      lede="Every hour the lab turns what its rejected families failed into new ones, and the gate judges each. The goal is one family that fails none of the gate's checks."
      verdict={verdict}
    />
  );
  const meta =
    gens.schedule.next ? `Every hour · next ${clock(gens.schedule.next)} UTC` : gens.schedule.cron ? "Scheduled" : "Not scheduled";

  if (gens.roots.length === 0 && gens.reports.length === 0)
    return (
      <div className="flex flex-col gap-6">
        {head(
          <Verdict rule={false} kind="calm" label="Generations" meta={meta} line="No generations yet" detail="The loop has not proposed anything, so it has charged nothing to the ledger." />,
        )}
        <HowItRuns gens={gens} />
        <Panel title={<h3 className="text-[15px] font-medium tracking-tight">Nothing on file</h3>}>
          <Empty title="No generation has run">
            <Code>{EVOLVE_DRY}</Code> shows what the first generation would propose and cost without charging anything. The lineage
            goes to <Code>lab-data/lineage.jsonl</Code> and reports to <Code>lab-data/reports/evolve/</Code>; both show up here.
          </Empty>
        </Panel>
      </div>
    );

  const judged = Object.values(gens.verdicts).reduce((a, b) => a + b, 0);
  const passed = gens.verdicts.PROMOTE_TO_PAPER ?? 0;
  const closest = checkTally(scoredNodes(gens)).closest;
  const judgedNow = judgedNodes(gens).filter((n) => !n.prior).length;
  const owner = runOf(gens);
  const real = gens.reports.filter((r) => !r.dry_run);
  const dry = gens.reports.filter((r) => r.dry_run);
  // Newest run first; generations nobody's run owns are grouped as "by hand".
  const groups: { key: string; title: string; detail: string; reports: CycleReport[] }[] = [];
  for (let i = gens.runs.length - 1; i >= 0; i--) {
    const run = gens.runs[i];
    const reports = real.filter((r) => owner.get(r.generation) === i);
    if (reports.length === 0) continue;
    const trials = run.generations.reduce((n, g) => n + g.trials, 0);
    groups.push({
      key: `run-${i}`,
      title: `Run ${i + 1} · ${when(run.started)}`,
      detail: `${plural(run.generations.length, "generation")}, ${plural(trials, "trial")}. Stopped because ${stopWords(run.stop, gens)}.`,
      reports,
    });
  }
  const loose = real.filter((r) => !owner.has(r.generation));
  if (loose.length) groups.push({ key: "hand", title: "Run by hand", detail: `${plural(loose.length, "generation")} started with ${EVOLVE}.`, reports: loose });
  if (dry.length) groups.push({ key: "dry", title: "Dry runs", detail: "Proposed only: nothing simulated, nothing charged.", reports: dry });

  return (
    <div className="flex flex-col gap-8">
      {head(
        <Verdict
          rule={false}
          kind={passed > 0 ? "good" : "calm"}
          label="Generations"
          meta={meta}
          line={
            gens.era
              ? `${passed} of ${plural(judgedNow, "generated family", "generated families")} passed this era`
              : judged > 0 ? `${passed} of ${plural(judged, "family", "families")} passed` : "0 families judged"
          }
          detail={
            judged === 0
              ? "Only dry runs so far: nothing simulated, nothing charged."
              : passed > 0
                ? `${passed} passed the gate. That opens the paper run, it is not evidence of an edge, and promoting is your decision.`
                : closest
                  ? `Closest: ${closest.node.name}, ${plural(closest.n, "check")} short, from generation ${closest.node.generation}. ${plural(gens.generations, "generation")} so far; ${trialsLine(gens)}.`
                  : gens.era && judgedNow === 0
                    ? `No family judged since the ledger restarted ${eraDate(gens.era)}. The ${plural(judged - judgedNow, "earlier verdict")} faced the old ledger's bar and stay as a record, drawn hollow below. ${plural(gens.generations, "generation")} so far.`
                    : `${plural(gens.generations, "generation")} so far; ${trialsLine(gens)}.`
          }
          href="/lab"
          link="The gate they face"
        />,
      )}

      <Progress gens={gens} />
      <Stops gens={gens} />

      <section aria-labelledby="gens-title" className="flex flex-col gap-3">
        <h3 id="gens-title" className="text-[15px] font-medium tracking-tight">Every generation</h3>
        {groups.map((g) => (
          <div key={g.key} className="flex flex-col gap-2">
            <p className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5 pt-2 text-[13px]">
              <span className="font-medium">{g.title}</span>
              <span className="text-muted-foreground">{g.detail}</span>
            </p>
            {g.reports.map((r) => (
              <Report key={r.file} r={r} gens={gens} open={false} />
            ))}
          </div>
        ))}
      </section>

      <div className="flex flex-col gap-3">
        <HowItRuns gens={gens} />
        <Lineage gens={gens} />
      </div>
    </div>
  );
}
