import Link from "next/link";
import { ArrowRight } from "lucide-react";
import type { Station, Tone } from "@/lib/v3/stations";
import { cn } from "@/lib/utils";

const DOT: Record<Tone, string> = {
  neutral: "bg-faint",
  active: "bg-info",
  good: "bg-pos",
  warn: "bg-warn",
  bad: "bg-neg",
  info: "bg-info",
};

export function StationDot({ tone, className }: { tone: Tone; className?: string }) {
  return <span aria-hidden className={cn("inline-block size-2 shrink-0 rounded-full", DOT[tone], className)} />;
}

/**
 * The four stations in the order the work flows: two evidence sources, the
 * gate, then the paper run. In guide mode each cell says what the station is
 * for instead of its live count.
 */
export function PipelineSpine({ stations, guide }: { stations: Station[]; guide?: boolean }) {
  const byId = new Map(stations.map((s) => [s.id, s]));
  const ordered = (["news", "jev", "lab", "paper"] as const).map((id) => byId.get(id)).filter((s): s is Station => !!s);
  return (
    <section aria-labelledby="pipeline-title" className="rounded-lg border bg-panel px-5 py-5">
      <div className="flex flex-wrap items-baseline justify-between gap-x-6 gap-y-1">
        <h2 id="pipeline-title" className="text-sm font-medium">Where the work is</h2>
        <p className="text-[13px] text-muted-foreground">News and Jev tests feed the Lab; the Lab feeds the paper run.</p>
      </div>
      <ol className="mt-4 grid gap-px overflow-hidden rounded-lg border bg-border sm:grid-cols-2 lg:grid-cols-4">
        {ordered.map((s) => (
          <li key={s.id} className="flex min-w-0 flex-col bg-panel p-4">
            <div className="flex flex-wrap items-baseline gap-x-2">
              <StationDot tone={s.tone} className="translate-y-[-1px]" />
              <Link href={s.href} className="text-[13px] font-medium text-foreground underline-offset-4 hover:underline">
                {s.name}
              </Link>
              <span className="text-[11px] text-faint">{s.question}</span>
            </div>
            {guide ? (
              <p className="mt-2 max-w-[38ch] text-[13px] leading-5 text-pretty text-muted-foreground">{s.role}</p>
            ) : (
              <>
                <p className="mt-2 text-[15px] leading-6 font-medium tracking-tight text-pretty">{s.state}</p>
                <p className="mt-1 max-w-[38ch] text-[13px] leading-5 text-pretty text-muted-foreground">{s.detail}</p>
                <p className="mt-auto pt-3 text-[11px] leading-4 text-faint">{s.meta}</p>
              </>
            )}
          </li>
        ))}
      </ol>
      <p className="mt-3 text-[13px] leading-5 text-muted-foreground">
        Going live is a terminal decision made outside this site. Nothing here can place an order.
      </p>
    </section>
  );
}

/**
 * The four stations as one instrument rail: a value, its unit, a bar where a
 * denominator exists, and one note. No paragraphs; each cell is a link.
 */
export type RailItem = {
  id: Station["id"];
  name: string;
  href: string;
  tone: Station["tone"];
  value: string;
  unit: string;
  note?: string;
  ratio?: { value: number; max: number; met: boolean };
};

export function StationRail({ items }: { items: RailItem[] }) {
  return (
    <ol aria-label="The four stations" className="grid grid-cols-2 gap-px overflow-hidden rounded-lg border bg-border lg:grid-cols-4">
      {items.map((s) => (
        <li key={s.id} className="min-w-0 bg-panel">
          <Link
            href={s.href}
            className="group flex h-full flex-col p-4 hover:bg-muted/30 focus-visible:ring-1 focus-visible:ring-foreground/40 focus-visible:ring-inset focus-visible:outline-none"
          >
            <div className="flex items-center gap-2">
              <StationDot tone={s.tone} />
              <span className="text-[13px] font-medium">{s.name}</span>
              <ArrowRight aria-hidden className="ml-auto size-3.5 text-faint transition-transform group-hover:translate-x-0.5" />
            </div>
            <div className="mt-3 flex items-baseline gap-2">
              <span className="font-mono text-[26px] leading-none tracking-tight tabular-nums">{s.value}</span>
              <span className="text-[11px] leading-4 text-muted-foreground">{s.unit}</span>
            </div>
            {s.ratio && (
              <div className="mt-3 h-[3px] w-full bg-foreground/15" aria-hidden>
                <div
                  className={s.ratio.met ? "h-full bg-pos" : "h-full bg-foreground"}
                  style={{ width: `${Math.min(100, Math.max(0, (s.ratio.value / Math.max(1, s.ratio.max)) * 100))}%` }}
                />
              </div>
            )}
            {s.note && <p className="mt-auto pt-3 text-[11px] leading-4 text-faint">{s.note}</p>}
          </Link>
        </li>
      ))}
    </ol>
  );
}

/**
 * The four stations as a footer index: name, state and the way in. This is the
 * compact replacement for a second copy of the sidebar, and it works on a phone.
 */
export function StationIndex({ stations, current }: { stations: Station[]; current?: Station["id"] }) {
  return (
    <nav aria-label="All four stations" className="border-t pt-4">
      <h2 className="text-[13px] font-medium">All four stations</h2>
      <ul className="mt-3 grid gap-x-8 gap-y-1 sm:grid-cols-2">
        {stations.map((s) => (
          <li key={s.id} className="min-w-0">
            <Link
              href={s.href}
              aria-current={current === s.id ? "page" : undefined}
              className={cn(
                "flex min-w-0 items-baseline gap-2 rounded-md py-1.5 text-[13px] underline-offset-4 hover:underline",
                current === s.id ? "text-foreground" : "text-muted-foreground",
              )}
            >
              <StationDot tone={s.tone} className="translate-y-[-1px]" />
              <span className={cn("font-medium", current === s.id ? "text-foreground" : "text-foreground/80")}>{s.name}</span>
              <span className="min-w-0 truncate">{s.state}</span>
              <ArrowRight aria-hidden className="ml-auto size-3.5 shrink-0 text-faint" />
            </Link>
          </li>
        ))}
      </ul>
    </nav>
  );
}
