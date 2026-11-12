"use client";

import { useState } from "react";
import { motion, useReducedMotion } from "motion/react";
import { EyeOff, History, MessageSquare, RotateCcw, Target } from "lucide-react";

const STEPS = [
  { label: "Hide", note: "Coin, date, price", icon: EyeOff },
  { label: "Ask", note: "One fixed question", icon: MessageSquare },
  { label: "Replay", note: "The same state, months back", icon: History },
  { label: "Score", note: "Chance and reshuffles", icon: Target },
];

/**
 * The probe pipeline as one authored sequence: the line draws left to right
 * (top to bottom on phones), each station lights as it passes, then the reader
 * can replay it. No loop, no decoration; reduced motion shows the final state.
 */
export function ProbeFlow() {
  const [run, setRun] = useState(0);
  const reduce = useReducedMotion();

  return (
    <section aria-labelledby="probe-flow" className="rounded-lg border bg-panel p-4 sm:p-5">
      <div className="flex items-center justify-between gap-3">
        <h2 id="probe-flow" className="text-sm font-medium">
          How a probe works
        </h2>
        <button
          type="button"
          onClick={() => setRun((n) => n + 1)}
          className="flex h-7 items-center gap-1 rounded-md px-2 text-[11px] text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <RotateCcw aria-hidden className="size-3" /> Play again
        </button>
      </div>

      <div key={run} className="relative mt-5">
        {/* The rule the pulse travels along: vertical on phones, horizontal from sm. */}
        <div aria-hidden className="absolute top-[18px] bottom-[18px] left-[17px] w-px bg-border sm:top-[17px] sm:right-0 sm:bottom-auto sm:left-0 sm:h-px" />
        {!reduce && (
          <>
            <motion.div
              aria-hidden
              className="absolute top-[18px] bottom-[18px] left-[17px] w-px origin-top bg-foreground sm:hidden"
              initial={{ scaleY: 0 }}
              animate={{ scaleY: 1 }}
              transition={{ duration: 2.15, ease: [0.16, 1, 0.3, 1] }}
            />
            <motion.div
              aria-hidden
              className="absolute top-[17px] right-0 left-0 hidden h-px origin-left bg-foreground sm:block"
              initial={{ scaleX: 0 }}
              animate={{ scaleX: 1 }}
              transition={{ duration: 2.15, ease: [0.16, 1, 0.3, 1] }}
            />
          </>
        )}

        <ol className="relative grid gap-4 sm:grid-cols-4">
          {STEPS.map((s, i) => (
            <motion.li
              key={s.label}
              className="flex items-center gap-3 sm:flex-col sm:text-center"
              initial={reduce ? false : { opacity: 0.35 }}
              animate={{ opacity: 1 }}
              transition={{ delay: 0.15 + i * 0.48, duration: 0.3 }}
            >
              <motion.span
                className="relative z-10 flex size-9 shrink-0 items-center justify-center rounded-full border bg-panel text-muted-foreground"
                initial={reduce ? false : { borderColor: "#242424", color: "#8f8f8f" }}
                animate={{ borderColor: "#ededed", color: "#ededed" }}
                transition={{ delay: 0.32 + i * 0.48, duration: 0.28 }}
              >
                <s.icon aria-hidden className="size-4" strokeWidth={1.75} />
              </motion.span>
              <span className="min-w-0">
                <span className="block text-[12px] font-medium">{s.label}</span>
                <span className="block text-[11px] leading-4 text-muted-foreground">{s.note}</span>
              </span>
            </motion.li>
          ))}
        </ol>
      </div>

      <p className="mt-4 text-[11px] leading-4 text-muted-foreground">
        A question counts only if it ranks above chance and beats 95% of reshuffles.
      </p>
    </section>
  );
}
