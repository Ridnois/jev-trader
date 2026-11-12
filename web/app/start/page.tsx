"use client";

import Link from "next/link";
import { ArrowRight, Lock } from "lucide-react";
import { useLive } from "@/components/live";
import { Panel, Tag } from "@/components/bits";
import { PipelineSpine } from "@/components/pipeline";
import { GlossList } from "@/components/gloss";
import { stations } from "@/lib/v3/stations";

const STATIONS = [
  {
    name: "Lab",
    question: "Is the rule real?",
    proves: "That this rule, found by this search, survives a gate built to cancel out the search, then a sealed lockbox of days no search saw.",
    cannot: "That another version of the idea would work, or that a rejected family is worthless. It judges the search, not the market.",
    href: "/lab",
  },
  {
    name: "Paper run",
    question: "Does the live market agree?",
    proves: "That the paper engine and the simulator entered the same trades, and whether the edge is large enough to show up in the days collected.",
    cannot: "That real fills would match paper, or that a graduated rule is safe to arm. It is the last filter, not a licence.",
    href: "/paper",
  },
  {
    name: "News recorder",
    question: "Is there money in the news?",
    proves: "Whether Jev's read of a headline predicts the price 5 minutes to 6 hours later, net of the round trip, on headlines collected forward.",
    cannot: "Replay the past: hindsight would leak into the score. It also cannot trade; it only measures.",
    href: "/catalysts",
  },
  {
    name: "Jev tests",
    question: "Can Jev read a market?",
    proves: "Whether the model ranks outcomes above chance on anonymised market states, and whether that ranking beats shuffled history.",
    cannot: "Prove a strategy. A read worth building on still has to pass the Lab like any other idea.",
    href: "/probes",
  },
];

const STATES = [
  ["Engine running", "The process wrote its status in the last 90 seconds and the price feed is connected. Figures are live."],
  ["Engine silent", "The process exists but has not written a status for over 90 seconds. Treat every figure as old."],
  ["Engine stopped", "The process is gone. The last written figures stay on screen; nothing new is arriving."],
  ["Feed down", "The engine runs but Hyperliquid's stream is disconnected. Prices are stale, so decisions pause."],
];

export default function Start() {
  const { live } = useLive();
  const pipe = stations(live);
  return (
    <div className="flex flex-col gap-8">
      <header className="flex flex-col gap-3">
        <h2 className="max-w-[26ch] text-[24px] leading-8 font-medium tracking-tight text-pretty">How this works.</h2>
        <p className="max-w-[70ch] text-[13px] leading-5 text-pretty text-muted-foreground">
          A four-minute read, no video. This page is the whole model; every other page assumes it.
        </p>
        <p className="max-w-[70ch] text-[15px] leading-6 text-pretty text-muted-foreground">
          This site is a window onto a research process, not a trading terminal. jev-trader tests ideas about crypto markets,
          refuses to trust the ones that only look good in hindsight, and runs the survivors on paper. Every number belongs
          to one of four stations, and every station answers one question.
        </p>
        <p className="flex items-start gap-2 text-[13px] leading-5 text-muted-foreground">
          <Lock aria-hidden className="mt-0.5 size-4 shrink-0" />
          <span><span className="text-foreground">Nothing here can trade.</span> The site is read-only: no pause, no arm, no order. Arming a strategy is a separate human decision, made in the terminal.</span>
        </p>
      </header>

      <section aria-labelledby="lifecycle" className="flex flex-col gap-3">
        <h3 id="lifecycle" className="text-[15px] font-medium tracking-tight">The lifecycle</h3>
        <p className="max-w-[70ch] text-[13px] leading-5 text-muted-foreground">
          Evidence from the two recorders feeds the Lab. The Lab judges what is worth running; a family it rejects can still
          run on paper as an unproven candidate, but nothing reaches live money without it. The paper run ends at a decision
          this site cannot make: yours.
        </p>
        <PipelineSpine stations={pipe} guide />
      </section>

      <section aria-labelledby="stations" className="flex flex-col gap-3">
        <h3 id="stations" className="text-[15px] font-medium tracking-tight">What each station proves, and what it cannot</h3>
        <div className="overflow-x-auto rounded-lg border">
          <table className="w-full min-w-[760px] border-collapse text-[13px]">
            <thead>
              <tr className="border-b text-left text-xs text-muted-foreground">
                <th className="h-9 px-4 font-medium">Station</th>
                <th className="h-9 px-4 font-medium">Question</th>
                <th className="h-9 px-4 font-medium">What it can prove</th>
                <th className="h-9 px-4 font-medium">What it cannot prove</th>
              </tr>
            </thead>
            <tbody>
              {STATIONS.map((s) => (
                <tr key={s.name} className="border-b align-top last:border-b-0">
                  <td className="px-4 py-3 whitespace-nowrap">
                    <Link href={s.href} className="font-medium text-foreground underline-offset-4 hover:underline">{s.name}</Link>
                  </td>
                  <td className="px-4 py-3 text-muted-foreground">{s.question}</td>
                  <td className="max-w-[34ch] px-4 py-3 leading-5">{s.proves}</td>
                  <td className="max-w-[34ch] px-4 py-3 leading-5 text-muted-foreground">{s.cannot}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>

      <section aria-labelledby="words" className="flex flex-col gap-3">
        <h3 id="words" className="text-[15px] font-medium tracking-tight">The words, once</h3>
        <p className="max-w-[70ch] text-[13px] leading-5 text-muted-foreground">
          These are the load-bearing terms. On every page the same wording appears in a hover or a disclosure, so the site
          never defines a term twice in two ways.
        </p>
        <div className="rounded-lg border bg-panel">
          <GlossList
            ids={["family", "grid", "spec", "trial", "gate", "lockbox", "strategy", "card", "paper-day", "sequential-test", "markout", "probe", "jev", "bps"]}
            intro="A family is an idea. A rule is one version of it. A trial is one simulation. A strategy is a promoted rule on paper. Those four are not interchangeable."
          />
        </div>
      </section>

      <section aria-labelledby="states" className="flex flex-col gap-3">
        <h3 id="states" className="text-[15px] font-medium tracking-tight">What the states mean</h3>
        <p className="max-w-[70ch] text-[13px] leading-5 text-muted-foreground">
          State is shown as a word and a dot, never colour alone. These are the engine states, then each station&apos;s state right now.
        </p>
        <dl className="grid gap-x-8 gap-y-3 rounded-lg border bg-panel px-5 py-4 sm:grid-cols-2">
          {STATES.map(([term, meaning]) => (
            <div key={term}>
              <dt className="text-[13px] font-medium">{term}</dt>
              <dd className="text-[13px] leading-5 text-muted-foreground">{meaning}</dd>
            </div>
          ))}
          {pipe.map((s) => (
            <div key={s.id}>
              <dt className="text-[13px] font-medium">
                {s.name}: <span className="font-mono text-xs font-normal text-foreground">{s.state}</span>
              </dt>
              <dd className="text-[13px] leading-5 text-muted-foreground">{s.detail}</dd>
            </div>
          ))}
        </dl>
      </section>

      <section aria-labelledby="do" className="flex flex-col gap-3">
        <h3 id="do" className="text-[15px] font-medium tracking-tight">What you do here</h3>
        <div className="grid gap-4 sm:grid-cols-2">
          <Panel title="Read, and decide what to look at">
            <p className="text-[13px] leading-5 text-muted-foreground">
              The site never asks you to act. Start on <Link href="/" className="text-foreground underline decoration-border underline-offset-4">Now</Link>,
              which says whether anything needs you. Then follow the station that is waiting: the Lab for a verdict, the
              paper run for evidence, the recorders for what to test next.
            </p>
          </Panel>
          <Panel title="The decisions that live elsewhere">
            <ul className="flex flex-col gap-2 text-[13px] leading-5 text-muted-foreground">
              <li><span className="text-foreground">Start or stop the engine:</span> <code className="font-mono text-xs">scripts/v3-services.sh start|stop</code></li>
              <li><span className="text-foreground">Promote a rule:</span> <code className="font-mono text-xs">lab promote</code>, after a Lab verdict.</li>
              <li><span className="text-foreground">Arm real money:</span> not implemented in v3. A graduate is a precondition, not an order.</li>
            </ul>
          </Panel>
        </div>
      </section>

      <footer className="flex flex-wrap items-center gap-3 border-t pt-4 text-[13px] text-muted-foreground">
        <span>Read next:</span>
        <Link href="/" className="flex items-center gap-1 text-foreground underline decoration-border underline-offset-4 hover:decoration-foreground">Now <ArrowRight className="size-3.5" /></Link>
        <span className="text-faint">the current state</span>
        <Link href="/lab" className="flex items-center gap-1 text-foreground underline decoration-border underline-offset-4 hover:decoration-foreground">Lab <ArrowRight className="size-3.5" /></Link>
        <span className="text-faint">the gate</span>
        <Tag kind="continue">No verdict yet</Tag>
        <span className="text-faint">is the normal state of a young paper run</span>
      </footer>
    </div>
  );
}
