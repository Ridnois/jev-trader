"use client";

import { GLOSSARY, type GlossId } from "@/lib/glossary";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { ChevronRight } from "lucide-react";

/**
 * A term with its record one hover or focus away. The dotted underline is the
 * only decoration; the same record is listed in <GlossList> on the page.
 */
export function Gloss({ id, children, className }: { id: GlossId; children?: React.ReactNode; className?: string }) {
  const r = GLOSSARY[id];
  if (!r) return <>{children}</>;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          className={`inline cursor-help text-left underline decoration-faint/70 decoration-dotted underline-offset-[3px] hover:decoration-foreground ${className ?? ""}`}
          aria-label={`${r.term}: ${r.meaning}`}
        >
          {children ?? r.term}
        </button>
      </TooltipTrigger>
      <TooltipContent side="top" className="max-w-[300px] leading-5">
        <span className="block font-medium">{r.term}</span>
        <span className="mt-1 block">{r.meaning}</span>
        {r.good && <span className="mt-1 block text-foreground/80">{r.good}</span>}
        <span className="mt-1 block text-muted-foreground">{r.misreading}</span>
      </TooltipContent>
    </Tooltip>
  );
}

/** The records a page uses, in the order given, behind one disclosure. */
export function GlossList({ ids, intro }: { ids: GlossId[]; intro?: string }) {
  const seen = new Set<GlossId>();
  const rows = ids.filter((id) => (seen.has(id) ? false : (seen.add(id), true)));
  return (
    <details className="group border-t px-5 py-3 text-[13px] leading-5">
      <summary className="flex cursor-pointer list-none items-center gap-2 font-medium">
        <ChevronRight aria-hidden className="size-3.5 text-muted-foreground transition-transform group-open:rotate-90" />
        What these words mean
      </summary>
      {intro && <p className="mt-2 max-w-[62ch] text-muted-foreground">{intro}</p>}
      <dl className="mt-3 grid gap-x-8 gap-y-3 sm:grid-cols-2">
        {rows.map((id) => {
          const r = GLOSSARY[id];
          return (
            <div key={id}>
              <dt className="font-medium text-foreground">{r.term}</dt>
              <dd className="text-muted-foreground">
                {r.meaning}
                {r.good && <span className="block text-muted-foreground">Good: {r.good}</span>}
                <span className="block text-faint">{r.misreading}</span>
              </dd>
            </div>
          );
        })}
      </dl>
    </details>
  );
}
