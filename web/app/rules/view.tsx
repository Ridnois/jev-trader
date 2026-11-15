"use client";

import Link from "next/link";
import { Fragment, useMemo, useState } from "react";
import { ChevronDown, ChevronRight, Search } from "lucide-react";
import { DataTable, Empty, PageHead, Panel, Tag, Td, Th, Verdict } from "@/components/bits";
import { Gloss } from "@/components/gloss";
import { WalkCalendar } from "@/components/walk-calendar";
import { plural, signed } from "@/lib/format";
import type { RuleFamily, RuleRow, Rules } from "@/lib/v3/types";

const PAGE = 50;

type Sort = "sharpe" | "bps" | "trades" | "distance";
type Origin = "all" | "hand" | "gen";
type Sign = "all" | "pos" | "neg";

/** `b_b_look` → "ingredient 3 · look": each `b_` is one recombine deeper. */
function settingLabel(k: string): string {
  let depth = 0;
  let rest = k;
  while (rest.startsWith("b_")) {
    depth += 1;
    rest = rest.slice(2);
  }
  return depth === 0 ? rest : `ingredient ${depth + 1} · ${rest}`;
}

/** How many recombines deep a setting sits: 0 for the family's own settings. */
const depthOf = (k: string) => (k.match(/^(b_)*/)?.[0].length ?? 0) / 2;

function settingsText(p: Record<string, number>): string {
  return Object.entries(p)
    .sort(([a], [b]) => depthOf(a) - depthOf(b) || a.localeCompare(b))
    .map(([k, v]) => `${settingLabel(k)} ${v}`)
    .join(", ");
}

/** Settings from a spec id, `family:k=v,...`. */
function paramsOf(id: string): Record<string, number> {
  const out: Record<string, number> = {};
  for (const kv of (id.split(":")[1] ?? "").split(",")) {
    const [k, v] = kv.split("=");
    if (k && v !== undefined && Number.isFinite(Number(v))) out[k] = Number(v);
  }
  return out;
}

const ruleLabel = (spec: string) => settingsText(paramsOf(spec));

const fmtNum = (v: number) => (Number.isInteger(v) ? String(v) : String(Number(v.toFixed(4))));

function Segmented<T extends string>({ value, options, onChange, label }: { value: T; options: { value: T; label: string }[]; onChange: (v: T) => void; label: string }) {
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex rounded-md border bg-card p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          onClick={() => onChange(o.value)}
          className={`h-7 rounded-[5px] px-2.5 text-xs transition-colors ${value === o.value ? "bg-muted text-foreground" : "text-muted-foreground hover:text-foreground"}`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** One row of the settings comparison: each value tried, its mean bps a day as a bar around zero, and how many rules tried it. */
function SettingRow({ name, cells, scale }: { name: string; cells: { v: number; mean: number; n: number }[]; scale: number }) {
  return (
    <div className="grid grid-cols-1 gap-x-4 gap-y-1.5 border-t py-2.5 sm:grid-cols-[180px_minmax(0,1fr)]">
      <span className="text-[13px] text-muted-foreground">{name}</span>
      <div className="flex flex-wrap gap-x-4 gap-y-2">
        {cells.map((c) => {
          const w = Math.min(50, (Math.abs(c.mean) / scale) * 50);
          return (
            <div key={c.v} className="w-[132px]" title={`${name} = ${fmtNum(c.v)}: ${signed(c.mean, 1)} bps a day on average over ${plural(c.n, "rule")}`}>
              <div className="flex items-baseline justify-between gap-2 text-[12px]">
                <span className="font-mono text-foreground">{fmtNum(c.v)}</span>
                <span className="font-mono tabular-nums text-muted-foreground">{signed(c.mean, 1)}</span>
              </div>
              <div aria-hidden className="relative mt-1 h-1.5 rounded-full bg-foreground/10">
                <span className="absolute inset-y-0 left-1/2 w-px bg-foreground/40" />
                <span
                  className={`absolute inset-y-0 rounded-full ${c.mean >= 0 ? "bg-foreground/80" : "bg-foreground/35"}`}
                  style={c.mean >= 0 ? { left: "50%", width: `${w}%` } : { right: "50%", width: `${w}%` }}
                />
              </div>
              <div className="mt-0.5 text-[11px] text-faint">{plural(c.n, "rule")}</div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function SortTh({ k, sort, onSort, children, className }: { k: Sort; sort: Sort; onSort: (k: Sort) => void; children: React.ReactNode; className?: string }) {
  return (
    <Th className={className} aria-sort={sort === k ? "descending" : "none"}>
      <button type="button" onClick={() => onSort(k)} className={`inline-flex items-center gap-1 ${sort === k ? "text-foreground" : "hover:text-foreground"}`}>
        {children}
        {sort === k && <ChevronDown aria-hidden className="size-3" />}
      </button>
    </Th>
  );
}

function mean(xs: number[]) {
  return xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : 0;
}

/** What the settings say for one family; across several families, how each family's rules did. */
function SettingsStrip({ rows, families, onFamily }: { rows: RuleRow[]; families: Record<string, RuleFamily>; onFamily: (key: string) => void }) {
  const keys = [...new Set(rows.map((r) => `${r.study}|${r.family}`))];
  if (rows.length === 0) return null;
  if (keys.length > 1) {
    const groups = keys
      .map((k) => {
        const rs = rows.filter((r) => `${r.study}|${r.family}` === k);
        return { k, fam: families[k], n: rs.length, mean: mean(rs.map((r) => r.bps)), best: Math.max(...rs.map((r) => r.bps)) };
      })
      .sort((a, b) => b.mean - a.mean);
    const shown = groups.slice(0, 10);
    const scale = Math.max(1, ...shown.map((g) => Math.abs(g.mean)));
    return (
      <Panel
        title="What the families say"
        description={`The ${keys.length} families in this view, best average first: mean bps a day across every rule each one tried. Select a family to compare its settings value by value.`}
      >
        <div className="flex flex-col">
          {shown.map((g) => {
            const w = Math.min(50, (Math.abs(g.mean) / scale) * 50);
            return (
              <button
                key={g.k}
                type="button"
                onClick={() => onFamily(g.k)}
                className="grid grid-cols-[minmax(0,1fr)_120px] items-center gap-x-4 border-t py-2 text-left transition-colors hover:bg-muted/40 sm:grid-cols-[220px_minmax(0,1fr)_200px]"
              >
                <span className="flex items-baseline gap-2 truncate text-[13px]">
                  <span className={g.fam?.generation !== null ? "font-mono" : "font-medium"}>{g.fam?.name ?? g.k}</span>
                  <span className="text-[11px] text-faint">{g.fam?.study.replace("-", " ")}</span>
                </span>
                <span aria-hidden className="relative hidden h-1.5 rounded-full bg-foreground/10 sm:block">
                  <span className="absolute inset-y-0 left-1/2 w-px bg-foreground/40" />
                  <span className={`absolute inset-y-0 rounded-full ${g.mean >= 0 ? "bg-foreground/80" : "bg-foreground/35"}`} style={g.mean >= 0 ? { left: "50%", width: `${w}%` } : { right: "50%", width: `${w}%` }} />
                </span>
                <span className="text-right font-mono text-[12px] tabular-nums text-muted-foreground">
                  {signed(g.mean, 1)} avg · {plural(g.n, "rule")}
                </span>
              </button>
            );
          })}
        </div>
        {groups.length > shown.length && <p className="border-t pt-2 text-[12px] text-faint">{groups.length - shown.length} more families below the top 10; narrow the filters to see them.</p>}
      </Panel>
    );
  }
  const fam = families[keys[0]];
  const params = [...new Set(rows.flatMap((r) => Object.keys(r.params)))];
  const settings = params
    .map((k) => {
      const values = [...new Set(rows.map((r) => r.params[k]).filter((v) => v !== undefined))].sort((a, b) => a - b);
      const cells = values.map((v) => {
        const rs = rows.filter((r) => r.params[k] === v);
        return { v, mean: mean(rs.map((r) => r.bps)), n: rs.length };
      });
      const spread = cells.length > 1 ? Math.max(...cells.map((c) => c.mean)) - Math.min(...cells.map((c) => c.mean)) : 0;
      return { k, cells, spread };
    })
    .filter((s) => s.cells.length > 1)
    .sort((a, b) => b.spread - a.spread || depthOf(a.k) - depthOf(b.k));
  const fixed = params.filter((k) => !settings.some((s) => s.k === k));
  const scale = Math.max(1, ...settings.flatMap((s) => s.cells.map((c) => Math.abs(c.mean))));
  return (
    <Panel
      title={`What ${fam?.name ?? "this family"}'s settings say`}
      description="For each setting it varied, the average bps a day of every rule that used each value, the setting that mattered most first. A value that wins across the board is a pattern; a single good rule is not."
    >
      {settings.length === 0 ? (
        <p className="border-t pt-3 text-[13px] text-muted-foreground">Every rule in this view shares the same settings, so there is nothing to compare.</p>
      ) : (
        settings.map((s) => <SettingRow key={s.k} name={settingLabel(s.k)} cells={s.cells} scale={scale} />)
      )}
      {fixed.length > 0 && (
        <p className="border-t pt-2.5 text-[12px] leading-4 text-faint">
          Held fixed: {fixed.map((k) => `${settingLabel(k)} ${fmtNum(rows[0].params[k])}`).join(", ")}.
        </p>
      )}
    </Panel>
  );
}

function Detail({ r, fam, rankInFamily, familySize }: { r: RuleRow; fam: RuleFamily | undefined; rankInFamily: number; familySize: number }) {
  return (
    <div className="grid gap-x-8 gap-y-3 bg-muted/30 px-5 py-4 text-[13px] leading-5 lg:grid-cols-[minmax(0,1.4fr)_minmax(0,1fr)]">
      <div className="min-w-0">
        <p className="text-muted-foreground">{fam?.statement || "No hypothesis statement on file for this family."}</p>
        <p className="mt-2 font-mono text-[12px] break-all text-faint">{r.id}</p>
      </div>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1">
        <dt className="text-faint">Family</dt>
        <dd>
          {fam?.name ?? r.family} · {fam?.generation !== null && fam?.generation !== undefined ? `generation ${fam.generation}` : "hand-written"}
        </dd>
        <dt className="text-faint">Verdict</dt>
        <dd className="flex flex-wrap items-center gap-2">
          {fam?.verdict ? <Tag kind={fam.verdict} /> : <span className="text-muted-foreground">No report on file</span>}
          {fam?.short !== null && fam?.short !== undefined && <span className="text-muted-foreground">{plural(fam.short, "check")} short of the lockbox</span>}
        </dd>
        <dt className="text-faint">In its family</dt>
        <dd>
          {rankInFamily} of {familySize} by Sharpe{r.pick ? ", and its in-sample pick" : ""}
        </dd>
        <dt className="text-faint">Walk-forward</dt>
        <dd>{r.folds > 0 ? `Picked in ${plural(r.folds, "fold")}` : "Never picked by a fold"}</dd>
        <dt className="text-faint">Simulated</dt>
        <dd>
          {r.days} days, {r.trades.toLocaleString("en-US")} trades, {new Date(r.at).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" })}
        </dd>
        <dt className="text-faint">Read more</dt>
        <dd className="flex gap-3">
          <Link href="/lab" className="underline decoration-border underline-offset-4 hover:decoration-foreground">Lab</Link>
          {fam?.generation !== null && fam?.generation !== undefined && (
            <Link href={`/generations#g${String(fam.generation).padStart(3, "0")}`} className="underline decoration-border underline-offset-4 hover:decoration-foreground">Generation {fam.generation}</Link>
          )}
        </dd>
      </dl>
    </div>
  );
}

export function RulesView({ rules }: { rules: Rules }) {
  const { rows, families, walk } = rules;
  const [q, setQ] = useState("");
  const [study, setStudy] = useState("all");
  const [origin, setOrigin] = useState<Origin>("all");
  const [sign, setSign] = useState<Sign>("all");
  const [picked, setPicked] = useState(false);
  const [famKey, setFamKey] = useState<string | null>(null);
  const [sort, setSort] = useState<Sort>("sharpe");
  const [limit, setLimit] = useState(PAGE);
  const [open, setOpen] = useState<string | null>(null);

  const studies = useMemo(() => [...new Set(rows.map((r) => r.study))].sort(), [rows]);
  const famOf = (r: RuleRow) => families[`${r.study}|${r.family}`];

  const filtered = useMemo(() => {
    const needle = q.trim().toLowerCase();
    const out = rows.filter((r) => {
      const f = families[`${r.study}|${r.family}`];
      if (famKey && `${r.study}|${r.family}` !== famKey) return false;
      if (study !== "all" && r.study !== study) return false;
      if (origin === "hand" && f?.generation !== null) return false;
      if (origin === "gen" && (f?.generation ?? null) === null) return false;
      if (sign === "pos" && !(r.bps > 0)) return false;
      if (sign === "neg" && !(r.bps < 0)) return false;
      if (picked && !r.pick && r.folds === 0) return false;
      if (needle) {
        const hay = `${f?.name ?? ""} ${r.family} ${r.id} ${settingsText(r.params)} ${f?.statement ?? ""}`.toLowerCase();
        if (!needle.split(/\s+/).every((w) => hay.includes(w))) return false;
      }
      return true;
    });
    const dist = (r: RuleRow) => families[`${r.study}|${r.family}`]?.short ?? 99;
    const by: Record<Sort, (a: RuleRow, b: RuleRow) => number> = {
      sharpe: (a, b) => b.sharpe - a.sharpe,
      bps: (a, b) => b.bps - a.bps,
      trades: (a, b) => b.trades - a.trades,
      distance: (a, b) => dist(a) - dist(b) || b.sharpe - a.sharpe,
    };
    return out.sort(by[sort]);
  }, [rows, families, q, study, origin, sign, picked, famKey, sort]);

  const familyRank = useMemo(() => {
    const m = new Map<string, { rank: number; size: number }>();
    const groups = new Map<string, RuleRow[]>();
    for (const r of rows) {
      const k = `${r.study}|${r.family}`;
      groups.set(k, [...(groups.get(k) ?? []), r]);
    }
    for (const g of groups.values()) [...g].sort((a, b) => b.sharpe - a.sharpe).forEach((r, i) => m.set(`${r.study}|${r.id}`, { rank: i + 1, size: g.length }));
    return m;
  }, [rows]);

  const positive = rows.filter((r) => r.bps > 0).length;
  const silent = rows.filter((r) => r.trades === 0).length;
  const last = rows.map((r) => r.at).sort().at(-1);
  const famCount = new Set(rows.map((r) => `${r.study}|${r.family}`)).size;
  const reset = () => {
    setQ("");
    setStudy("all");
    setOrigin("all");
    setSign("all");
    setPicked(false);
    setFamKey(null);
    setLimit(PAGE);
  };
  const touch = <T,>(set: (v: T) => void) => (v: T) => {
    set(v);
    setLimit(PAGE);
    setOpen(null);
  };
  const head = (
    <PageHead
      question="Which rules has the lab tried, and which settings keep winning?"
      lede={
        <>
          A rule is one point on a family&apos;s grid: the family&apos;s idea with every setting fixed. These are all of them, from every family,
          hand-written or bred by the loop. Every number is in sample, after costs, and the best of a search: exactly what the{" "}
          <Link href="/lab" className="text-foreground underline decoration-border underline-offset-4 hover:decoration-foreground">gate</Link> exists to discount. Read them for patterns, not picks.
        </>
      }
      verdict={
        <Verdict
          rule={false}
          kind="calm"
          label="Rules"
          meta={last ? `Last added ${new Date(last).toLocaleString("en-GB", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", timeZone: "UTC" })} UTC` : "Nothing simulated yet"}
          line={`${rows.length.toLocaleString("en-US")} rules, ${plural(famCount, "family", "families")}`}
          detail={`${positive.toLocaleString("en-US")} earned money in sample${silent ? `, ${silent} never traded` : ""}. Each one is a trial on the ledger, and each trial raises the bar the next family must clear.`}
        />
      }
    />
  );

  if (rows.length === 0)
    return (
      <div className="flex flex-col gap-6">
        {head}
        <Panel title="No rules on the ledger">
          <Empty title="Nothing simulated yet">
            <code className="font-mono">jev-trader lab validate</code> or <code className="font-mono">lab evolve</code> writes every rule it simulates to <code className="font-mono">lab-data/trials.jsonl</code>; they show up here.
          </Empty>
        </Panel>
      </div>
    );

  const fam = famKey ? families[famKey] : null;
  const shown = filtered.slice(0, limit);
  const viewKeys = new Set(filtered.map((r) => `${r.study}|${r.family}`));

  return (
    <div className="flex flex-col gap-6">
      {head}

      <div className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center gap-2">
          <label className="relative min-w-[240px] flex-1">
            <span className="sr-only">Search rules</span>
            <Search aria-hidden className="absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-faint" />
            <input
              value={q}
              onChange={(e) => touch(setQ)(e.target.value)}
              placeholder="Search by family, setting or idea, e.g. trend n 384"
              className="h-8 w-full rounded-md border bg-card pr-3 pl-8 text-[13px] outline-none placeholder:text-faint focus-visible:ring-2 focus-visible:ring-ring"
            />
          </label>
          <Segmented label="Study" value={study} onChange={touch(setStudy)} options={[{ value: "all", label: "All studies" }, ...studies.map((s) => ({ value: s, label: s.replace("-", " ") }))]} />
          <Segmented<Origin> label="Origin" value={origin} onChange={touch(setOrigin)} options={[{ value: "all", label: "All" }, { value: "hand", label: "Hand-written" }, { value: "gen", label: "Generated" }]} />
          <Segmented<Sign> label="Result" value={sign} onChange={touch(setSign)} options={[{ value: "all", label: "Any result" }, { value: "pos", label: "Made money" }, { value: "neg", label: "Lost" }]} />
          <button
            type="button"
            role="switch"
            aria-checked={picked}
            onClick={() => touch(setPicked)(!picked)}
            className={`h-8 rounded-md border px-3 text-xs transition-colors ${picked ? "border-foreground/40 bg-muted text-foreground" : "bg-card text-muted-foreground hover:text-foreground"}`}
          >
            Picked only
          </button>
        </div>
        <p className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[12px] text-muted-foreground">
          <span>
            {filtered.length.toLocaleString("en-US")} of {rows.length.toLocaleString("en-US")} rules
          </span>
          {fam && (
            <span className="inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-foreground">
              {fam.name} · {fam.study.replace("-", " ")}
              <button type="button" onClick={() => touch(setFamKey)(null)} aria-label="Clear family filter" className="text-faint hover:text-foreground">
                ×
              </button>
            </span>
          )}
          {filtered.length !== rows.length && (
            <button type="button" onClick={reset} className="underline decoration-border underline-offset-4 hover:text-foreground hover:decoration-foreground">
              Clear filters
            </button>
          )}
        </p>
      </div>

      <SettingsStrip rows={filtered} families={families} onFamily={(k) => touch(setFamKey)(k)} />

      {filtered.length > 0 && walk.windows.length > 0 && (
        <Panel
          flush
          title="Which rule each window picked, and how it did"
          description={
            fam
              ? `${fam.name} opened into a lane per rule its walk-forward picked, in the order it first picked them. Each cell is out of sample. Select a cell to open that rule in the table.`
              : "Out of sample, window by window. Before each 60-day window a family re-picks its best rule on the 180 days before it, then trades it blind; each cell is that rule's result. A long run with no tick means one setting kept being chosen. Rules never picked do not appear. Select a family to see its rules one lane each."
          }
        >
          <WalkCalendar
            key={famKey ?? "all"}
            walk={walk}
            families={families}
            keys={viewKeys}
            single={famKey}
            selected={open}
            ruleLabel={ruleLabel}
            onFamily={(k) => touch(setFamKey)(k)}
            onRule={(k, rule) => {
              if (open === rule) return setOpen(null);
              setFamKey(k);
              setOpen(rule);
              setLimit(1000);
              // Bring the opened rule into view once the table has rendered it.
              setTimeout(() => {
                const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
                document.getElementById(`rule-${rule}`)?.scrollIntoView({ block: "center", behavior: still ? "auto" : "smooth" });
              }, 60);
            }}
          />
        </Panel>
      )}

      <Panel flush title="Every rule" description="Select a rule for its idea, its family's verdict and where it ranks. Picked means it was its family's best in sample or a walk-forward fold chose it.">
        {filtered.length === 0 ? (
          <Empty title="No rule matches these filters">
            <button type="button" onClick={reset} className="underline decoration-border underline-offset-4 hover:decoration-foreground">
              Clear the filters
            </button>{" "}
            to see all {rows.length.toLocaleString("en-US")}.
          </Empty>
        ) : (
          <>
            <DataTable minWidth={860} label="Every rule the lab simulated, with its in-sample results and its family's distance to the goal">
              <thead>
                <tr>
                  <Th className="w-8" />
                  <Th>Rule</Th>
                  <Th>Study</Th>
                  <SortTh sort={sort} onSort={touch(setSort)} k="trades" className="text-right">Trades</SortTh>
                  <SortTh sort={sort} onSort={touch(setSort)} k="bps" className="text-right"><Gloss id="bps">bps</Gloss> a day</SortTh>
                  <SortTh sort={sort} onSort={touch(setSort)} k="sharpe" className="text-right"><Gloss id="sharpe">Sharpe</Gloss></SortTh>
                  <SortTh sort={sort} onSort={touch(setSort)} k="distance" className="text-right">Family distance</SortTh>
                  <Th>Picked</Th>
                </tr>
              </thead>
              <tbody>
                {shown.map((r) => {
                  const f = famOf(r);
                  const key = `${r.study}|${r.id}`;
                  const isOpen = open === key;
                  const rk = familyRank.get(key);
                  return (
                    <Fragment key={key}>
                      <tr id={`rule-${key}`} onClick={() => setOpen(isOpen ? null : key)} className={`cursor-pointer ${isOpen ? "bg-muted/40" : ""}`} aria-expanded={isOpen}>
                        <Td>
                          <ChevronRight aria-hidden className={`size-3.5 text-muted-foreground transition-transform ${isOpen ? "rotate-90" : ""}`} />
                        </Td>
                        <Td className="max-w-[420px]">
                          <div className="flex items-baseline gap-2">
                            <span className={`whitespace-nowrap ${f?.generation !== null && f?.generation !== undefined ? "font-mono" : "font-medium"}`}>{f?.name ?? r.family}</span>
                            <span className="text-[11px] whitespace-nowrap text-faint">{f?.generation !== null && f?.generation !== undefined ? `gen ${f.generation}` : "hand-written"}</span>
                          </div>
                          <div className="truncate font-mono text-[12px] text-muted-foreground" title={settingsText(r.params)}>
                            {settingsText(r.params) || "no settings"}
                          </div>
                        </Td>
                        <Td className="font-mono text-[12px] whitespace-nowrap text-muted-foreground">{r.study}</Td>
                        <Td className="text-right tabular-nums">{r.trades === 0 ? <span className="text-faint">none</span> : r.trades.toLocaleString("en-US")}</Td>
                        <Td className="text-right font-mono tabular-nums">{signed(r.bps, 1)}</Td>
                        <Td className="text-right font-mono tabular-nums">{signed(r.sharpe, 2)}</Td>
                        <Td className="text-right tabular-nums">{f?.short === null || f?.short === undefined ? <span className="text-faint">–</span> : `${f.short} short`}</Td>
                        <Td className="text-[12px] whitespace-nowrap text-muted-foreground">
                          {[r.pick ? "in sample" : "", r.folds ? `${plural(r.folds, "fold")}` : ""].filter(Boolean).join(" · ") || <span className="text-faint">–</span>}
                        </Td>
                      </tr>
                      {isOpen && (
                        <tr className="hover:bg-transparent">
                          <td colSpan={8} className="p-0">
                            <Detail r={r} fam={f} rankInFamily={rk?.rank ?? 0} familySize={rk?.size ?? 0} />
                          </td>
                        </tr>
                      )}
                    </Fragment>
                  );
                })}
              </tbody>
            </DataTable>
            <div className="flex items-center justify-between border-t px-5 py-3 text-[12px] text-muted-foreground">
              <span>
                Showing {shown.length.toLocaleString("en-US")} of {filtered.length.toLocaleString("en-US")}
              </span>
              {filtered.length > shown.length && (
                <button type="button" onClick={() => setLimit((l) => l + PAGE)} className="rounded-md border px-3 py-1.5 text-foreground transition-colors hover:bg-muted">
                  Show {Math.min(PAGE, filtered.length - shown.length)} more
                </button>
              )}
            </div>
          </>
        )}
      </Panel>
    </div>
  );
}
