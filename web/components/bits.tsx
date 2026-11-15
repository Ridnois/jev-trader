import Link from "next/link";
import { ArrowDownRight, ArrowRight, ArrowUpRight, Check, Minus, X } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { cn } from "@/lib/utils";
import { signed } from "@/lib/format";

const DOT: Record<string, string> = {
  running: "bg-pos text-pos",
  degraded: "bg-warn text-warn",
  stopped: "bg-neg text-neg",
  never: "bg-faint text-faint",
};

export function Dot({ level }: { level: "running" | "degraded" | "stopped" | "never" }) {
  return <span aria-hidden className={cn("inline-block size-2 shrink-0 rounded-full", DOT[level])} />;
}

/** The page question, the state readout, then the lede. On phones the state comes before the explanation. */
export function PageHead({ question, lede, verdict }: { question: React.ReactNode; lede: React.ReactNode; verdict: React.ReactNode }) {
  return (
    <div className="grid gap-x-12 gap-y-3 border-b pb-6 lg:grid-cols-[minmax(0,1.1fr)_minmax(340px,0.9fr)] lg:grid-rows-[auto_1fr]">
      <h2 className="max-w-[28ch] text-[24px] leading-8 font-medium tracking-tight text-pretty lg:col-start-1 lg:row-start-1">{question}</h2>
      <div className="min-w-0 lg:col-start-2 lg:row-span-2 lg:row-start-1">{verdict}</div>
      <p className="max-w-[52ch] text-[13px] leading-5 text-pretty text-muted-foreground lg:col-start-1 lg:row-start-2">{lede}</p>
    </div>
  );
}

export function Panel({
  title, description, action, children, className, flush,
}: {
  title?: React.ReactNode;
  description?: React.ReactNode;
  action?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
  /** Content runs edge to edge (tables). */
  flush?: boolean;
}) {
  return (
    <Card className={cn("gap-0 rounded-lg py-0 shadow-none ring-border", className)}>
      {title && (
        <CardHeader className="flex flex-col gap-3 px-5 pt-4 pb-3 sm:flex-row sm:items-start sm:justify-between">
          <div className="flex min-w-0 flex-col gap-0.5">
            <CardTitle className="text-sm font-medium">{title}</CardTitle>
            {description && <CardDescription className="max-w-[36rem] text-[13px] leading-5 text-pretty">{description}</CardDescription>}
          </div>
          {action && <div className="max-w-full shrink-0">{action}</div>}
        </CardHeader>
      )}
      <CardContent className={cn("min-w-0 flex-1", flush ? "px-0" : "px-5 pb-5", !title && !flush && "pt-5")}>{children}</CardContent>
    </Card>
  );
}

/** A signed figure coloured by its sign; zero stays neutral. */
export function Delta({ value, digits = 2, unit = "", className }: { value: number | null | undefined; digits?: number; unit?: string; className?: string }) {
  if (value === null || value === undefined || Number.isNaN(value)) return <span className={cn("text-faint", className)}>–</span>;
  const r = Number(value.toFixed(digits));
  return <span className={cn(r > 0 ? "text-pos" : r < 0 ? "text-neg" : "text-muted-foreground", className)}>{signed(value, digits, unit)}</span>;
}

const VERDICTS: Record<string, { label: string; cls: string }> = {
  PROMOTE_TO_PAPER: { label: "Promoted to paper", cls: "border-pos/30 bg-pos/10 text-pos" },
  REVISE: { label: "Revise", cls: "border-warn/30 bg-warn/10 text-warn" },
  REJECT: { label: "Rejected", cls: "border-neg/30 bg-neg/10 text-neg" },
  INSUFFICIENT_EVIDENCE: { label: "Not enough evidence", cls: "border-warn/30 bg-warn/10 text-warn" },
  /** The operator's call, not the gate's: put on paper although the gate did not pass it. Neutral, never green. */
  approved: { label: "Approved by you", cls: "border-foreground/25 bg-foreground/[0.06] text-foreground" },
  candidate: { label: "Unproven candidate", cls: "border-warn/30 bg-warn/10 text-warn" },
  validated: { label: "Validated", cls: "border-pos/30 bg-pos/10 text-pos" },
  rejected: { label: "Rejected", cls: "border-neg/30 bg-neg/10 text-neg" },
  accepted: { label: "Accepted", cls: "border-pos/30 bg-pos/10 text-pos" },
  triggered: { label: "Triggered", cls: "border-border bg-muted text-muted-foreground" },
  proposed: { label: "Not tested yet", cls: "border-border bg-muted text-muted-foreground" },
  continue: { label: "No verdict yet", cls: "border-border bg-muted text-muted-foreground" },
  /** Not a state and not a near miss: the same bet under another name, so it is neutral, never amber. */
  duplicate: { label: "Duplicate", cls: "border-border bg-muted text-muted-foreground" },
  /** A verdict from before the ledger restarted: a record, neutral. */
  era1: { label: "Earlier era", cls: "border-border bg-muted text-muted-foreground" },
  PROMOTED: { label: "On paper", cls: "border-warn/30 bg-warn/10 text-warn" },
  KILLED: { label: "Killed on paper", cls: "border-neg/30 bg-neg/10 text-neg" },
  GRADUATED: { label: "Graduated on paper", cls: "border-pos/30 bg-pos/10 text-pos" },
  kill: { label: "Kill", cls: "border-neg/30 bg-neg/10 text-neg" },
  graduate: { label: "Graduate", cls: "border-pos/30 bg-pos/10 text-pos" },
};

export function Tag({ kind, children, className }: { kind: string; children?: React.ReactNode; className?: string }) {
  const v = VERDICTS[kind] ?? { label: kind, cls: "border-border bg-muted text-muted-foreground" };
  return <span className={cn("inline-flex h-[22px] items-center rounded-full border px-2 text-xs font-medium whitespace-nowrap", v.cls, className)}>{children ?? v.label}</span>;
}

/** Pass, fail or fatal, said with a shape as well as a colour. */
export function Mark({ pass, fatal, className }: { pass: boolean; fatal?: boolean; className?: string }) {
  const cls = "inline-flex size-[18px] shrink-0 items-center justify-center rounded-full";
  if (pass)
    return (
      <span className={cn(cls, "bg-pos/15 text-pos", className)}>
        <Check className="size-3" strokeWidth={3} aria-label="Passed" />
      </span>
    );
  if (fatal)
    return (
      <span className={cn(cls, "bg-neg/15 text-neg", className)}>
        <X className="size-3" strokeWidth={3} aria-label="Fatal" />
      </span>
    );
  return (
    <span className={cn(cls, "bg-warn/15 text-warn", className)}>
      <Minus className="size-3" strokeWidth={3} aria-label="Short of the bar" />
    </span>
  );
}

export function Empty({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="px-5 py-10 text-center">
      <div className="text-sm font-medium">{title}</div>
      <p className="mx-auto mt-1 max-w-[52ch] text-[13px] leading-5 text-pretty text-muted-foreground">{children}</p>
    </div>
  );
}

const VERDICT_TONE: Record<"calm" | "active" | "warn" | "bad" | "good", string> = {
  calm: "text-foreground",
  active: "text-foreground",
  warn: "text-warn",
  bad: "text-neg",
  good: "text-pos",
};

const VERDICT_DOT: Record<"calm" | "active" | "warn" | "bad" | "good", string> = {
  calm: "bg-faint",
  /** Running and undecided: the same blue the station index uses, not green, because nothing has passed. */
  active: "bg-info",
  warn: "bg-warn",
  bad: "bg-neg",
  good: "bg-pos",
};

/**
 * The state of a station, set as an instrument readout: a fixed label, a mono
 * line with units and a noun, then one sentence saying what it means. No card.
 */
export function Verdict({
  kind, label, line, detail, meta, href, link, children, rule = true,
}: {
  kind: "calm" | "active" | "warn" | "bad" | "good";
  label: string;
  line: React.ReactNode;
  detail: React.ReactNode;
  meta?: React.ReactNode;
  href?: string;
  link?: string;
  children?: React.ReactNode;
  /** The hairline above the readout; off when a PageHead already draws one. */
  rule?: boolean;
}) {
  return (
    <section aria-label={`${label}: ${typeof line === "string" ? line : ""}`} className={cn(rule && "border-t pt-4")}>
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
        <span aria-hidden className={cn("size-1.5 rounded-full", VERDICT_DOT[kind])} />
        <span className="font-medium text-foreground">{label}</span>
        {meta && (
          <>
            <span className="text-faint">·</span>
            <span>{meta}</span>
          </>
        )}
        {href && link && (
          <Link href={href} className="ml-auto flex items-center gap-1 text-xs text-foreground underline decoration-border underline-offset-4 hover:decoration-foreground">
            {link} <ArrowRight aria-hidden className="size-3" />
          </Link>
        )}
      </div>
      <p className={cn("mt-2 font-mono text-[22px] leading-8 tracking-tight text-balance tabular-nums sm:text-[26px] sm:leading-9", VERDICT_TONE[kind])}>{line}</p>
      <p className="mt-2 max-w-[72ch] text-[13px] leading-5 text-pretty text-muted-foreground">{detail}</p>
      {children && <div className="mt-4">{children}</div>}
    </section>
  );
}

/** A row of figures in one band, separated by hairlines rather than four cards. */
export function Figures({ items }: { items: { label: string; value: React.ReactNode; sub?: React.ReactNode; tone?: "pos" | "neg" | "warn"; ratio?: { value: number; max: number; met: boolean } }[] }) {
  return (
    <dl className={cn("grid gap-px overflow-hidden rounded-lg border bg-border", items.length === 4 ? "grid-cols-2 lg:grid-cols-4" : "grid-cols-1 sm:grid-cols-3")}>
      {items.map((it) => (
        <div key={it.label} className="min-w-0 bg-panel px-4 py-3">
          <dt className="text-xs text-muted-foreground">{it.label}</dt>
          <dd className={cn("mt-0.5 truncate text-[17px] leading-6 font-semibold tracking-tight tabular-nums sm:text-[19px] sm:leading-7", it.tone === "pos" && "text-pos", it.tone === "neg" && "text-neg", it.tone === "warn" && "text-warn")}>{it.value}</dd>
          {it.ratio && (
            <div className="mt-1.5 h-[3px] w-full bg-foreground/15" aria-hidden>
              <div
                className={cn("h-full", it.ratio.met ? "bg-pos" : "bg-foreground")}
                style={{ width: `${Math.min(100, Math.max(0, (it.ratio.value / Math.max(1, it.ratio.max)) * 100))}%` }}
              />
            </div>
          )}
          {it.sub && <div className="mt-0.5 text-xs leading-4 text-muted-foreground">{it.sub}</div>}
        </div>
      ))}
    </dl>
  );
}

/**
 * A position's side: an arrow in the side's colour (long up, short down), with the
 * word unless `bare`. The colour is direction, never result; result is pos/neg.
 */
export function Side({ side, bare, className }: { side: number; bare?: boolean; className?: string }) {
  const long = side > 0;
  const Icon = long ? ArrowUpRight : ArrowDownRight;
  return (
    <span className={cn("inline-flex items-center gap-1 whitespace-nowrap", long ? "text-long" : "text-short", className)}>
      <Icon aria-hidden className="size-3.5 shrink-0" strokeWidth={2.25} />
      {bare ? <span className="sr-only">{long ? "Long" : "Short"}</span> : long ? "Long" : "Short"}
    </span>
  );
}

export function Th({ className, ...rest }: React.ComponentProps<"th">) {
  return (
    <th scope="col" className={cn("h-9 px-3 text-left text-xs font-medium whitespace-nowrap text-muted-foreground first:pl-5 last:pr-5", className)} {...rest} />
  );
}

export function Td({ children, className, ...rest }: React.ComponentProps<"td">) {
  return (
    <td className={cn("px-3 py-2.5 align-middle first:pl-5 last:pr-5", className)} {...rest}>
      {children}
    </td>
  );
}

export function DataTable({ children, className, minWidth, label }: { children: React.ReactNode; className?: string; minWidth?: number; label?: string }) {
  return (
    <div className="overflow-x-auto focus-visible:outline-2 focus-visible:-outline-offset-2" tabIndex={0} role="region" aria-label={label}>
      <table className={cn("w-full border-collapse text-[13px] [&_tbody_tr]:border-t [&_tbody_tr:hover]:bg-muted/50 [&_thead_tr]:border-t", className)} style={{ minWidth }}>
        {label && <caption className="sr-only">{label}</caption>}
        {children}
      </table>
    </div>
  );
}
