import Link from "next/link";
import { DataTable, Empty, Figures, PageHead, Panel, Tag, Td, Th, Verdict } from "@/components/bits";
import { Gloss, GlossList } from "@/components/gloss";
import { dateShort, familyName, plural, sentence, signed } from "@/lib/format";
import type { CycleReport, GenNode, Generations, Pairing, ProposalRow } from "@/lib/v3/types";

const EVOLVE_DRY = "jev-trader lab evolve --dry-run";

function Code({ children }: { children: React.ReactNode }) {
  return <code className="font-mono text-[12px] text-foreground">{children}</code>;
}

/** A parent's readable name: `g0001` for a generated family, the family name for a hand-written one. */
function parentName(gens: Generations, parent: string | null): string {
  if (!parent) return "nothing (exploration)";
  return gens.names[parent] ?? familyName(parent);
}

function stamp(iso: string | null): string {
  if (!iso) return "time unknown";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "time unknown" : `${dateShort(iso)}, ${d.toISOString().slice(11, 16)} UTC`;
}

function bar(x: number): string {
  return Number.isFinite(x) ? x.toFixed(2) : "–";
}

function failedCount(note: string): number {
  return note ? note.split(",").filter((s) => s.trim()).length : 0;
}

// ---- the lineage tree ----

type Row = { node: GenNode; depth: number };

function flatten(nodes: GenNode[], depth: number, out: Row[]): Row[] {
  for (const node of nodes) {
    out.push({ node, depth });
    flatten(node.children, depth + 1, out);
  }
  return out;
}

function Lineage({ gens }: { gens: Generations }) {
  if (gens.roots.length === 0)
    return (
      <Panel title="Lineage">
        <Empty title="No family has been proposed for real yet">
          Only dry runs are on file. <Code>jev-trader lab evolve</Code> without <Code>--dry-run</Code> writes the families, charges
          their trials and judges them; each one is recorded in <Code>lab-data/lineage.jsonl</Code>.
        </Empty>
      </Panel>
    );
  return (
    <Panel
      flush
      title="Lineage"
      description="Each hand-written family with the families generated from it, and the newest state of each. A child is judged from scratch on the same gate; it inherits nothing from its parent's record."
    >
      <DataTable minWidth={820}>
        <thead>
          <tr>
            <Th>Family</Th>
            <Th className="text-right"><Gloss id="generation">Generation</Gloss></Th>
            <Th><Gloss id="operator">Operator</Gloss></Th>
            <Th>Study</Th>
            <Th className="text-right"><Gloss id="trial">Trials</Gloss></Th>
            <Th>State</Th>
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
              const n = failedCount(node.failed);
              return (
                <tr key={node.family}>
                  <Td>
                    <div className="flex items-baseline gap-2 whitespace-nowrap" style={{ paddingLeft: depth * 20 }}>
                      <span aria-hidden className="text-faint">└</span>
                      <span className="font-mono font-medium">{node.name}</span>
                      {node.name !== node.family && <span className="font-mono text-xs text-faint">{node.family}</span>}
                    </div>
                  </Td>
                  <Td className="text-right tabular-nums">{node.generation}</Td>
                  <Td>{node.operator || "–"}</Td>
                  <Td className="whitespace-nowrap text-muted-foreground">{node.study ? node.study.replace("-", " ") : "–"}</Td>
                  <Td className="text-right tabular-nums">{node.trials}</Td>
                  <Td>
                    <div className="flex items-center gap-2 whitespace-nowrap">
                      <Tag kind={node.state} />
                      {n > 0 && (
                        <span className="text-xs text-faint" title={node.failed}>
                          {plural(n, "check")} failed
                        </span>
                      )}
                    </div>
                  </Td>
                </tr>
              );
            })}
          </tbody>
        ))}
      </DataTable>
      <GlossList ids={["generation", "operator", "trial", "luck-bar", "family", "spec", "gate", "in-sample", "bps"]} intro="Hover an underlined term anywhere on the page for the same record." />
    </Panel>
  );
}

function TrialsByStudy({ gens }: { gens: Generations }) {
  if (gens.byStudy.length === 0) return null;
  return (
    <Panel
      flush
      title="Trials per study"
      description="What the loop has charged to each study's ledger. Generated families share their study's count with the hand-written ones, so every trial here raises the bar for all of them."
    >
      <DataTable minWidth={480}>
        <thead>
          <tr>
            <Th>Study</Th>
            <Th className="text-right">Charged by the loop</Th>
            <Th className="text-right">On the ledger, all searches</Th>
          </tr>
        </thead>
        <tbody>
          {gens.byStudy.map((s) => (
            <tr key={s.study}>
              <Td className="font-mono text-xs">{s.study}</Td>
              <Td className="text-right tabular-nums">{plural(s.loop, "trial")}</Td>
              <Td className="text-right tabular-nums text-muted-foreground">{s.ledger ? plural(s.ledger, "trial") : "–"}</Td>
            </tr>
          ))}
        </tbody>
      </DataTable>
    </Panel>
  );
}

// ---- one cycle report ----

function Section({ title, children, note }: { title: string; children: React.ReactNode; note?: React.ReactNode }) {
  return (
    <div className="border-t">
      <div className="px-5 pt-4 pb-2 text-sm font-medium">{title}</div>
      {children}
      {note && <p className="border-t px-5 py-3 text-xs leading-4 text-faint">{note}</p>}
    </div>
  );
}

function Cost({ r }: { r: CycleReport }) {
  if (r.studies.length === 0)
    return (
      <Section title="Cost">
        <p className="px-5 pb-4 text-[13px] text-muted-foreground">No trials {r.dry_run ? "would be charged" : "were charged"}.</p>
      </Section>
    );
  const days = r.studies.find((c) => c.design_days)?.design_days ?? 0;
  return (
    <Section
      title="Cost"
      note={
        <>
          The luck bar is the annualised Sharpe the best of that many no-edge rules would show over the design window
          {days ? ` (${plural(days, "day")})` : ""}, counting every trial as independent. Every cycle raises it.
        </>
      }
    >
      <DataTable minWidth={620}>
        <thead>
          <tr>
            <Th>Study</Th>
            <Th className="text-right">Trials before</Th>
            <Th className="text-right">{r.dry_run ? "Would be after" : "After"}</Th>
            <Th className="text-right"><Gloss id="luck-bar">Luck bar</Gloss> before</Th>
            <Th className="text-right">{r.dry_run ? "Would be after" : "After"}</Th>
          </tr>
        </thead>
        <tbody>
          {r.studies.map((c) => (
            <tr key={c.study}>
              <Td className="font-mono text-xs">{c.study}</Td>
              <Td className="text-right tabular-nums">{c.trials_before.toLocaleString("en-US")}</Td>
              <Td className="text-right tabular-nums">
                {c.trials_after.toLocaleString("en-US")} <span className="text-faint">(+{c.trials_after - c.trials_before})</span>
              </Td>
              <Td className="text-right font-mono tabular-nums">{bar(c.luck_bar_before)}</Td>
              <Td className="text-right font-mono tabular-nums">{bar(c.luck_bar_after)}</Td>
            </tr>
          ))}
        </tbody>
      </DataTable>
      {r.lockbox_openings > 0 && (
        <p className="border-t px-5 py-3 text-[13px] leading-5 text-warn">
          The lockbox was opened {plural(r.lockbox_openings, "time")} this cycle. Each opening makes the next one harder to pass.
        </p>
      )}
    </Section>
  );
}

function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid gap-x-4 gap-y-0.5 sm:grid-cols-[112px_minmax(0,1fr)]">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="min-w-0 text-pretty break-words">{children}</dd>
    </div>
  );
}

function Proposal({ p, r, gens }: { p: ProposalRow; r: CycleReport; gens: Generations }) {
  const promotable = r.promotable.includes(p.name);
  return (
    <li className="border-t px-5 py-4">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span className="font-mono text-sm font-medium">{p.name || p.family}</span>
        <span className="text-[13px] text-muted-foreground">
          {p.operator || "–"} of {parentName(gens, p.parent)}
        </span>
        <span className="ml-auto flex items-center gap-2">
          {promotable && <Tag kind="accepted">Promotable</Tag>}
          {p.verdict ? <Tag kind={p.verdict} /> : <Tag kind="proposed">{r.dry_run ? "Not simulated" : "Not judged"}</Tag>}
        </span>
      </div>
      <dl className="mt-3 grid gap-2 text-[13px] leading-5">
        <Fact label="Family"><span className="font-mono text-xs">{p.family}</span></Fact>
        <Fact label="Rule"><span className="font-mono text-xs">{p.rule || "–"}</span></Fact>
        <Fact label="Why">{p.reason || "–"}</Fact>
        {p.lessons.length > 0 && <Fact label="Lessons"><span className="font-mono text-xs">{p.lessons.join(", ")}</span></Fact>}
        <Fact label="Size">
          {plural(p.specs, "rule")}, {plural(p.new_trials, "new trial")}{r.dry_run ? " if run" : ""}
          {p.approximate && (
            <span className="text-warn" title="The parent is hand-written and its block form is an approximation, so this child is not a strict variant of it.">
              {" "}· approximate parent
            </span>
          )}
        </Fact>
        {p.failed.length > 0 && <Fact label="Failed checks">{p.failed.map(sentence).join(", ")}</Fact>}
        {p.pick && <Fact label="Pick"><span className="font-mono text-xs">{p.pick}</span></Fact>}
      </dl>
    </li>
  );
}

function bps(x: number): string {
  return signed(x, 1);
}

function PairTable({ rows, child, parent, duplicates }: { rows: Pairing[]; child: string; parent: string; duplicates: boolean }) {
  return (
    <DataTable minWidth={duplicates ? 860 : 720}>
      <thead>
        <tr>
          <Th>{child}</Th>
          <Th>{parent}</Th>
          <Th className="text-right">Days</Th>
          <Th className="text-right">{child}</Th>
          <Th className="text-right">{parent}</Th>
          <Th className="text-right">Difference [95%]</Th>
          {duplicates && <Th className="text-right">Correlation</Th>}
          {duplicates && <Th />}
        </tr>
      </thead>
      <tbody>
        {rows.map((p, i) => (
          <tr key={`${p.challenger}-${p.incumbent}-${i}`}>
            <Td className="font-mono text-xs">{p.challenger}</Td>
            <Td className="whitespace-nowrap">
              {p.incumbent}
              {p.incumbent_spec && <div className="max-w-[260px] truncate font-mono text-[11px] text-faint" title={p.incumbent_spec}>{p.incumbent_spec}</div>}
            </Td>
            <Td className="text-right tabular-nums">{p.days}</Td>
            <Td className="text-right tabular-nums">{bps(p.challenger_bps)}</Td>
            <Td className="text-right tabular-nums">{bps(p.incumbent_bps)}</Td>
            <Td className="text-right whitespace-nowrap tabular-nums">
              {bps(p.diff_bps)} <span className="text-faint">[{bps(p.lo_bps)}, {bps(p.hi_bps)}]</span>
            </Td>
            {duplicates && <Td className="text-right tabular-nums">{signed(p.corr, 2)}</Td>}
            {duplicates && <Td>{p.duplicate && <Tag kind="REVISE">Duplicate</Tag>}</Td>}
          </tr>
        ))}
      </tbody>
    </DataTable>
  );
}

function ForOperator({ r }: { r: CycleReport }) {
  let line: React.ReactNode;
  if (r.dry_run)
    line = (
      <>
        Dry run. The would-be family files are in <Code>{`lab-data/reports/evolve/dry-g${String(r.generation).padStart(3, "0")}/`}</Code>.{" "}
        <Code>jev-trader lab evolve</Code> without <Code>--dry-run</Code> writes them, charges their trials and judges them.
      </>
    );
  else if (r.promotable.length === 0) line = "Nothing to promote. That is a result.";
  else
    line = (
      <>
        Passed the gate and not a duplicate: <span className="font-mono">{r.promotable.join(", ")}</span>. Promoting is your decision,
        in the terminal: <Code>jev-trader lab promote --family families/gen/&lt;name&gt;.toml</Code>.
      </>
    );
  return (
    <Section title="For the operator">
      <p className="px-5 pb-4 text-[13px] leading-5">{line}</p>
    </Section>
  );
}

function Report({ r, gens }: { r: CycleReport; gens: Generations }) {
  const duplicates = r.pairings.filter((p) => p.duplicate).length;
  return (
    <section id={r.file.replace(/\.json$/, "")} className="scroll-mt-20">
      <Panel
        flush
        title={
          <span className="flex flex-wrap items-center gap-2">
            Generation {r.generation}
            {r.dry_run && <Tag kind="dry" className="border-info/30 bg-info/10 text-info">Dry run</Tag>}
          </span>
        }
        description={
          <>
            {stamp(r.at)}. {r.lessons_usable} of {plural(r.lessons_total, "lesson")} usable; the rest are anecdotes or for the operator.
            {r.dry_run && " Nothing was simulated, nothing was charged, and no family file was written where the lab reads families."}
          </>
        }
        action={<span className="font-mono text-xs text-faint">{r.file}</span>}
      >
        <Cost r={r} />
        <Section
          title={`${r.dry_run ? "Would propose" : "Proposals"}: ${r.proposals.length}`}
          note={!r.dry_run && r.proposals.length > 0 ? "A generated family was built after the design window had been looked at, so its walk-forward folds are not truly out of sample. The lockbox and paper days are the only clean evidence for it." : undefined}
        >
          {r.proposals.length === 0 ? (
            <p className="px-5 pb-4 text-[13px] text-muted-foreground">None. The proposer&apos;s notes below say why.</p>
          ) : (
            <ul>
              {r.proposals.map((p) => <Proposal key={p.family || p.name} p={p} r={r} gens={gens} />)}
            </ul>
          )}
        </Section>
        {r.pairings.length > 0 && (
          <Section
            title={`Against the incumbents, design window${duplicates ? `: ${plural(duplicates, "duplicate")}` : ""}`}
            note="Mean return in bps per day on common days. In sample for both sides: a difference here is a reason to look, not evidence. A correlation of 0.9 or more marks a duplicate, which is never promotable."
          >
            <PairTable rows={r.pairings} child="Challenger" parent="Incumbent" duplicates />
          </Section>
        )}
        {r.forward.length > 0 && (
          <Section title="On paper: child against parent" note="Mean paper return in bps per day over the days both ran. Paper is not live money, and a few days decide nothing.">
            <PairTable rows={r.forward} child="Child" parent="Parent" duplicates={false} />
          </Section>
        )}
        {r.messages.length > 0 && (
          <Section title="Notes from the proposer">
            <ul className="list-disc space-y-1 pr-5 pb-4 pl-10 text-[13px] leading-5 text-muted-foreground">
              {r.messages.map((m, i) => <li key={i}>{m}</li>)}
            </ul>
          </Section>
        )}
        <ForOperator r={r} />
      </Panel>
    </section>
  );
}

// ---- the page ----

export function GenerationsView({ gens }: { gens: Generations }) {
  const head = (verdict: React.ReactNode) => (
    <PageHead
      question="What has the lab tried on its own?"
      lede={
        <>
          <Code>lab evolve</Code> reads every verdict, proposes a few new families with a written reason, charges their trials and
          judges them with the same gate as the hand-written ones. It runs only when you ask, and it promotes nothing. Every trial
          it charges makes the next family harder to pass.{" "}
          <Link href="/lab" className="text-foreground underline decoration-border underline-offset-4 hover:decoration-foreground">The gate is on the Lab page.</Link>
        </>
      }
      verdict={verdict}
    />
  );

  if (gens.roots.length === 0 && gens.reports.length === 0)
    return (
      <div className="flex flex-col gap-6">
        {head(<Verdict rule={false} kind="calm" label="Generations" meta="Never run" line="No generations yet" detail="The loop has not proposed anything, so nothing has been charged to the ledger by it." />)}
        <Panel title="No generations yet">
          <Empty title="Nothing on file">
            <Code>{EVOLVE_DRY}</Code> shows what the first generation would propose and what it would cost, without simulating or
            charging anything. Lineage is written to <Code>lab-data/lineage.jsonl</Code> and reports to{" "}
            <Code>lab-data/reports/evolve/</Code>; both show up here.
          </Empty>
        </Panel>
      </div>
    );

  const judged = Object.values(gens.verdicts).reduce((a, b) => a + b, 0);
  const passed = gens.verdicts.PROMOTE_TO_PAPER ?? 0;
  const rejected = gens.verdicts.REJECT ?? 0;
  const latest = gens.reports.find((r) => !r.dry_run) ?? null;
  const dryRuns = gens.reports.filter((r) => r.dry_run).length;
  const promotable = latest?.promotable.length ?? 0;
  const rest = judged - passed - rejected;

  return (
    <div className="flex flex-col gap-6">
      {head(
        <Verdict
          rule={false}
          kind={passed > 0 ? "good" : "calm"}
          label="Generations"
          meta={`${plural(gens.generations, "generation")}, ${plural(gens.trials, "trial")} charged`}
          line={judged > 0 ? `${passed} of ${plural(judged, "generated family", "generated families")} passed` : "0 generated families judged"}
          detail={
            judged === 0
              ? "Only dry runs so far: nothing simulated, nothing charged."
              : passed === 0
                ? `${plural(rejected, "family", "families")} rejected${rest > 0 ? `, ${rest} short of the bar or without enough evidence` : ""}. A family the loop could not make pass is a result, not a missing one.`
                : `${passed} passed the gate. Passing is the door to the paper run, not evidence of an edge, and promoting is your decision.`
          }
          href="/lab"
          link="The gate they face"
        />,
      )}

      <Figures
        items={[
          { label: "Generations", value: `${gens.generations}`, sub: `${plural(gens.proposed, "family", "families")} proposed${dryRuns ? `, ${plural(dryRuns, "dry run")} on file` : ""}` },
          { label: "Trials charged by the loop", value: gens.trials.toLocaleString("en-US"), sub: "Each one raises the luck bar for every family on its study" },
          { label: "Promotable, last generation", value: plural(promotable, "family", "families"), sub: latest ? `From generation ${latest.generation}. Promoting is done in the terminal` : "No real generation yet" },
          { label: "Last cycle", value: gens.reports[0]?.at ? dateShort(gens.reports[0].at) : "–", sub: gens.reports[0] ? `Generation ${gens.reports[0].generation}${gens.reports[0].dry_run ? ", dry run" : ""}` : "No report on file" },
        ]}
      />

      <Lineage gens={gens} />
      <TrialsByStudy gens={gens} />

      {gens.reports.length > 0 && (
        <div className="flex flex-col gap-2">
          <h3 className="text-[15px] font-medium tracking-tight">Cycle reports</h3>
          <p className="max-w-[72ch] text-[13px] leading-5 text-muted-foreground">
            One per run of <Code>lab evolve</Code>, newest first, as written to <Code>lab-data/reports/evolve/</Code>.
          </p>
        </div>
      )}
      {gens.reports.map((r) => <Report key={r.file} r={r} gens={gens} />)}
    </div>
  );
}
