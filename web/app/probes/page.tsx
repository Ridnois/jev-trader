import type { Metadata } from "next";
import Link from "next/link";
import { getLab } from "@/lib/v3/server";
import { DataTable, Empty, PageHead, Panel, Tag, Td, Th, Verdict } from "@/components/bits";
import { Gloss, GlossList } from "@/components/gloss";
import { Reliability } from "@/components/charts";
import { ProbeFlow } from "@/components/probe-flow";
import { showsSkill } from "@/lib/v3/stations";
import { dateShort, sentence, signed } from "@/lib/format";
import type { ProbeReport } from "@/lib/v3/types";

export const metadata: Metadata = { title: "Jev tests" };
export const dynamic = "force-dynamic";

const QUESTIONS: Record<string, string> = {
  p_revert: "Will this fade revert?",
  p_run: "Will price run 2% against the trade first?",
  p_up: "Will the next 6 hours be up?",
};

const VENUE_SHORT: Record<string, string> = { hyperliquid: "Hyperliquid", binance: "Binance" };

function Row({ label, children }: { label: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-4 border-b border-border/60 py-1 last:border-0">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="tabular-nums">{children}</dd>
    </div>
  );
}

function entries(p: ProbeReport) {
  return p.questions.reduce((n, q) => n + q.n, 0);
}

function Question({ q }: { q: ProbeReport["questions"][number] }) {
  const skill = showsSkill(q);
  return (
    <div className="border-b p-4 last:border-b-0 sm:border-r sm:border-b-0 sm:last:border-r-0">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-[13px] leading-5 font-medium">{QUESTIONS[q.id] ?? q.id}</h3>
        <Tag kind={skill ? "accepted" : "proposed"}>
          {skill ? "Shows skill" : q.auc > 0.55 ? "Ranks, fails shuffle" : "No skill"}
        </Tag>
      </div>

      <div className="mt-3 flex flex-col gap-3 sm:flex-row sm:items-start">
        <div className="shrink-0">
          <Reliability q={q} />
          <p className="mt-1 max-w-[220px] text-[11px] leading-4 text-muted-foreground">
            Jev said (across) against what happened (up). Diagonal is perfect; blue line is the base rate.
          </p>
        </div>
        <dl className="min-w-0 flex-1 text-[13px]">
          <Row label={<Gloss id="auc">AUC</Gloss>}><span className="font-mono">{q.auc.toFixed(2)}</span></Row>
          <Row label={<Gloss id="shuffle-test">Reshuffles beaten</Gloss>}><span className="font-mono">{q.ic_null_pct.toFixed(0)}%</span></Row>
          <Row label={<Gloss id="brier">Brier skill</Gloss>}><span className="font-mono">{signed(q.brier_skill, 3)}</span></Row>
        </dl>
      </div>

      <p className="mt-2 text-[11px] leading-4 text-faint">
        Jev said {(q.mean_answer * 100).toFixed(0)}% · happened {(q.base_rate * 100).toFixed(0)}% · {q.n} entries over {q.days} days · rank corr{" "}
        <span className="tabular-nums">{signed(q.ic, 2)}</span>
      </p>
    </div>
  );
}

function ProbePanel({ p }: { p: ProbeReport }) {
  const accepted = p.questions.filter(showsSkill).length;
  const total = p.questions.length;
  return (
    <div id={`${p.probe}-${p.venue}`} className="scroll-mt-24">
      <Panel
        flush
        title={`${sentence(p.probe.replace("_", " "))} · ${VENUE_SHORT[p.venue] ?? p.venue}`}
        description={`${p.calls.toLocaleString("en-US")} calls · ${entries(p).toLocaleString("en-US")} entries · ${p.models.join(", ")}${p.scored ? ` · scored ${dateShort(p.scored)}` : ""}`}
        action={
          <Tag kind={accepted > 0 ? "accepted" : "proposed"}>
            {accepted} of {total} show skill
          </Tag>
        }
      >
        <div className="grid grid-cols-1 border-t sm:grid-cols-2">
          {p.questions.map((q) => (
            <Question key={q.id} q={q} />
          ))}
        </div>

        {p.choices.length > 0 && (
          <details className="border-t">
            <summary className="cursor-pointer px-5 py-3 text-[13px] font-medium">
              What Jev called the setup
              <span className="ml-2 font-normal text-faint">{p.choices.length} labels</span>
            </summary>
            <DataTable minWidth={420}>
              <thead>
                <tr><Th>Jev&apos;s label</Th><Th className="text-right">Entries</Th><Th className="text-right">Mean result of the fade</Th></tr>
              </thead>
              <tbody>
                {p.choices.map(([, label, n, bps]) => (
                  <tr key={label}>
                    <Td className="font-medium">{sentence(label)}</Td>
                    <Td className="text-right">{n}</Td>
                    <Td className="text-right"><span className="tabular-nums text-foreground/80">{signed(bps, 0, " bps")}</span></Td>
                  </tr>
                ))}
              </tbody>
            </DataTable>
          </details>
        )}
      </Panel>
    </div>
  );
}

export default function Probes() {
  const lab = getLab();
  const jev = lab.hypotheses.filter((h) => h.id.startsWith("H-J"));
  const questions = lab.probes.flatMap((p) => p.questions);
  const accepted = questions.filter(showsSkill).length;
  const partial = questions.filter((q) => q.auc > 0.55 && !showsSkill(q)).length;

  return (
    <div className="flex flex-col gap-5">
      <PageHead
        question="Can Jev read a market at all?"
        lede={
          <>
            A probe hides the coin, date and price, asks <Gloss id="jev">Jev</Gloss> one fixed question, then replays the
            same states over months of history. Skill means ranking above chance and beating the shuffle test.
          </>
        }
        verdict={
          lab.probes.length > 0 ? (
            <Verdict
              rule={false}
              kind={accepted > 0 ? "good" : "calm"}
              label="Jev tests"
              meta={`${lab.probes.length} probes · ${questions.length} questions`}
              line={`${accepted} of ${questions.length} questions show skill`}
              detail={
                accepted === 0
                  ? `${partial > 0 ? `${partial} of ${questions.length} rank above chance but fail the shuffle test. ` : ""}Nothing is built on Jev's read.`
                  : `${accepted} of ${questions.length} rank outcomes above chance and beat the shuffle test. The next step is a rule, judged by the same gate as any family.`
              }
              href="/start"
              link="Where this fits"
            />
          ) : (
            <p className="text-[13px] text-muted-foreground">No probe has been scored yet.</p>
          )
        }
      />

      <ProbeFlow />

      {lab.probes.length === 0 ? (
        <Panel title="No probe has been scored">
          <Empty title="No scores on file">
            Replay a probe with <code className="font-mono">lab jev replay --probe probes/fade_judge.toml</code>, then score it with <code className="font-mono">lab jev score</code>.
          </Empty>
        </Panel>
      ) : (
        <nav aria-label="Probes on file" className="grid gap-px overflow-hidden rounded-lg border bg-border sm:grid-cols-2">
          {lab.probes.map((p) => {
            const ok = p.questions.filter(showsSkill).length;
            return (
              <Link
                key={p.probe + p.venue}
                href={`#${p.probe}-${p.venue}`}
                className="group flex flex-col bg-panel p-4 hover:bg-muted/30 focus-visible:ring-1 focus-visible:ring-foreground/40 focus-visible:ring-inset focus-visible:outline-none"
              >
                <div className="flex items-baseline justify-between gap-3">
                  <span className="text-[13px] font-medium">{sentence(p.probe.replace("_", " "))}</span>
                  <span className="text-[11px] text-faint">{VENUE_SHORT[p.venue] ?? p.venue}</span>
                </div>
                <div className="mt-1 flex items-baseline justify-between text-[11px] text-muted-foreground">
                  <span>{p.questions.length} questions</span>
                  <span>{ok} show skill</span>
                </div>
              </Link>
            );
          })}
        </nav>
      )}

      {lab.probes.map((p) => (
        <ProbePanel key={p.probe + p.venue} p={p} />
      ))}

      {jev.length > 0 && (
        <details className="rounded-lg border bg-panel">
          <summary className="flex cursor-pointer items-baseline justify-between gap-3 px-5 py-3 text-sm font-medium">
            Hypotheses about Jev
            <span className="text-[11px] font-normal text-faint">{jev.length} registered before the test</span>
          </summary>
          <ul className="border-t">
            {jev.map((h) => (
              <li key={h.id} className="border-b px-5 py-3 last:border-b-0">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-mono text-xs text-muted-foreground">{h.id}</span>
                  <Tag kind={h.status} />
                </div>
                <p className="mt-1 max-w-[62ch] text-[13px] leading-5 text-pretty">{h.statement}.</p>
                {h.result && <p className="mt-1 max-w-[62ch] text-[13px] leading-5 text-pretty text-muted-foreground">{h.result}</p>}
              </li>
            ))}
          </ul>
        </details>
      )}

      <Panel title="What a probe cannot tell you">
        <p className="border-t px-5 py-3 text-[13px] leading-5 text-muted-foreground">
          A probe measures a read, not a profit, and one stretch is not a market. A family built on it still has to pass the
          Lab like any other idea.
        </p>
        <GlossList ids={["probe", "auc", "shuffle-test", "brier"]} />
      </Panel>
    </div>
  );
}
