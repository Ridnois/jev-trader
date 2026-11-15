import type { StrategyTag } from "@/lib/format";
export type Gate = { gate: string; passed: boolean; detail: string };

export type Position = {
  coin: string;
  side: number;
  qty: number;
  entry_px: number;
  notional: number;
  stop_px: number | null;
  entry_bar_ms: number;
  deadline_ms: number;
  opened_ms: number;
  /** Funding paid (negative) or received (positive) so far, dollars. */
  funding: number;
  /** Fee paid on entry, dollars. Older status files may lack it. */
  entry_fee?: number;
  entry_mid?: number;
};

export type BookStatus = {
  strategy: string;
  status: string;
  spec: string;
  interval: string;
  capital: number;
  equity_change: number;
  realized: number;
  fees: number;
  funding: number;
  at_risk: number;
  positions: Position[];
};

export type EngineStatus = {
  ts: string;
  pid: number;
  venue: string;
  coins: number;
  quotes: number;
  feed: { connected: boolean; last_error: string | null; last_msg_ms: number; reconnects: number };
  catalyst: { enabled: boolean; candidates_waiting_for_markouts: number };
  books: BookStatus[];
};

export type Card = {
  spec: { family: string; params: Record<string, number> };
  interval: string;
  mu_daily: number;
  sd_daily: number;
  alpha: number;
  beta: number;
  days_if_real: number;
  days_if_none: number;
  dd99: [number, number, number];
  issued: string;
  study: string;
};

export type DayRow = { day: string; ret: number; up_secs: number; full: boolean; llr: number };

export type StrategyView = {
  name: string;
  status: string;
  hypothesis: string;
  card: Card;
  /** Screen name of the family: `Trend`, or `g0045` for a generated one. */
  family: string;
  /** `G<generation>/<family>·<variant>`, the compact name on tables. */
  tag: StrategyTag;
  /** The entry rule in words with the spec's settings filled in; null when the family has no composed form. */
  rule: string | null;
  /** The entry clause per side, the part of the rule that opens one order. */
  entry: { long: string; short: string | null } | null;
  book: BookStatus | null;
  /** Sequential test: log-likelihood ratio after each full paper day. */
  days: DayRow[];
  llr: number;
  kill: number;
  graduate: number;
  fullDays: number;
  partialDays: number;
  drawdown: number;
  drawdownLimit: number;
  call: "continue" | "kill" | "graduate";
  callWhy: string;
  /** The monitor's parity line: do paper entries match the simulator's. */
  parity: string | null;
  /** What the lab's gate said about this family, and the check that ended it. */
  labVerdict: string | null;
  labStoppedAt: string | null;
};

export type Markout = { ret_bps: number; net_bps: number };

export type Candidate = {
  id: string;
  ts: string;
  coin: string;
  side: number;
  title: string;
  source: string;
  impact: number;
  new_info: number;
  lag_min: number;
  triggered: boolean;
  gates: Gate[];
  mid: number;
  cost_bps: number;
  markouts: Record<string, Markout>;
};

export type NewsEvent = {
  id: string;
  title: string;
  source: string;
  published: string;
  first_seen: string;
  lag_min: number;
  coins: string[];
  market: boolean;
  impact: number | null;
  impact_conf: number | null;
  new_info: number | null;
  hack: unknown;
  model: string | null;
};

export type HorizonSummary = {
  secs: number;
  triggeredN: number;
  triggeredMean: number | null;
  allN: number;
  allMean: number | null;
};

export type Quote = {
  coin: string;
  mid: number;
  lastSec: number;
  open: number;
  low: number;
  high: number;
  chg1h: number | null;
  chgDay: number | null;
  /** One mid per minute, oldest first. */
  spark: number[];
};

export type Trade = {
  strategy: string;
  coin: string;
  side: number;
  opened_ms: number;
  closed_ms: number;
  entry_px: number;
  exit_px: number;
  notional: number;
  fees: number;
  funding: number;
  pnl: number;
  pnl_bps: number;
  reason: string;
};

export type LabSummary = {
  families: number;
  /** Of `families`, how many the evolve loop generated. */
  generated: number;
  /** Families judged before the ledger's current era, kept as a record and not counted above. */
  prior: number;
  era: string | null;
  /** The family that failed the fewest checks, and the first check it fell short on. */
  closest: { name: string; generation: number | null; failed: number; check: string; value: string; need: string } | null;
  promoted: number;
  lastJudged: string | null;
  lockboxSealed: boolean;
  lockboxDays: number;
  questions: number;
  acceptedQuestions: number;
  lastScored: string | null;
};

export type Live = {
  now: number;
  /** The Lab's state, small enough for the live poll. Full reports stay on /lab. */
  lab: LabSummary;
  engine: {
    status: EngineStatus | null;
    alive: boolean;
    stale: boolean;
    ageSec: number | null;
    started: string | null;
    /** Time of the last status write, for "data is this old" readings. */
    lastStatusTs: string | null;
    catalystCfg: { min_impact: number; min_new_info: number; max_lag_min: number; markouts_secs: number[] };
  };
  strategies: StrategyView[];
  trades: Trade[];
  candidates: Candidate[];
  candidateCount: number;
  triggeredCount: number;
  news: NewsEvent[];
  newsCount: number;
  horizons: HorizonSummary[];
  /** Distinct UTC days that have at least one markout. */
  markoutDays: number;
  /** Distinct UTC days that have at least one scored headline. */
  newsDays: number;
  firstScoredAt: string | null;
  lastScoredAt: string | null;
  lastTradeAt: number | null;
  quotes: Quote[];
};

export type Check = { name: string; value: string; need: string; pass: boolean; fatal: boolean };

export type Leader = {
  spec: string;
  sharpe_annual: number;
  mean_bps_day: number;
  trades: number;
  win_pct: number;
  mean_trade_bps: number;
};

export type FamilyReport = {
  study: string;
  family: string;
  hypothesis: string;
  panel: string;
  specs: number;
  trials_ledger: number;
  trials_effective: number;
  design_days: number;
  lockbox_days: number;
  lockbox_opened: boolean;
  leaders: Leader[];
  checks: Check[];
  /** Cumulative out-of-sample return, percent, one point per walk-forward day. */
  oosCum: number[];
  folds: { train_sharpe: number; test_sharpe: number; test_mean: number }[];
  verdict: string;
  notes: string[];
  /** When the verdict file was written, ISO. */
  judged: string;
  /** UTC date (YYYY-MM-DD) of the first out-of-sample day, the first point of `oosCum`. */
  oosStart: string | null;
  /** First lockbox day (panel start + design days) and the panel's last day, YYYY-MM-DD. */
  lockboxStart: string | null;
  panelEnd: string | null;
  /** Screen name for a generated family (`g0045`); hand-written families use their family name. */
  name?: string;
  /** Judged before the ledger's current era: its verdict faced the old ledger's bar, so it is a record, not ranked. */
  prior: boolean;
  /** Generation number, for a family the evolve loop proposed. */
  generation?: number;
};

export type ProbeQuestion = {
  id: string;
  outcome: string;
  n: number;
  days: number;
  base_rate: number;
  mean_answer: number;
  brier_skill: number;
  auc: number;
  ic: number;
  ic_null_pct: number;
  /** [mean answer, observed rate, mean bps, n] per quintile of the answer. */
  buckets: [number, number, number, number][];
};

export type ProbeReport = {
  probe: string;
  venue: string;
  models: string[];
  calls: number;
  tokens_per_call: number;
  median_ms: number;
  questions: ProbeQuestion[];
  choices: [string, string, number, number][];
  /** When the score file was written, ISO. */
  scored: string;
};

export type Hypothesis = { id: string; statement: string; status: string; result: string; pass_if: string; invalidate_if: string };

export type Calibration = { title: string; rows: { outcome: string; panels: number; share: string }[] };

/** One `calibration-evolve-*.md`: the evolve loop run on synthetic panels with a known answer. */
export type LoopCalibration = {
  file: string;
  seed: string;
  interval: string;
  panels: number;
  generations: number;
  edge: number;
  /** Ran under `lab evolve --auto` stop rules (file name has `auto`). */
  auto: boolean;
  anyPromoted: number;
  seedPromoted: number;
  loopOnly: number;
};

export type Lab = {
  /** Hand-written families (one verdict per family and study). */
  families: FamilyReport[];
  /** Families the evolve loop generated and the gate judged. */
  generated: FamilyReport[];
  /** Trials the evolve loop charged (lineage), part of `trials`. */
  loopTrials: number;
  loopCalibrations: LoopCalibration[];
  probes: ProbeReport[];
  hypotheses: Hypothesis[];
  calibrations: Calibration[];
  trials: number;
  trialsByStudy: { study: string; n: number }[];
  /** What `strategies/` hands the paper trader: one row per file, keyed to the family its spec belongs to. */
  onPaper: OnPaper[];
  /** Start of the ledger's current era, or null when it never restarted. */
  era: string | null;
};

export type OnPaper = { family: string; name: string; status: "validated" | "candidate" | string };

// ---- the evolve loop: `lab-data/lineage.jsonl` and `lab-data/reports/evolve/*.json` ----

/** One row of `lab-data/lineage.jsonl` (`src/lab/lineage.rs` `Event`). */
export type LineageEvent = {
  at: string | null;
  event: "proposed" | "tested" | "promoted" | "killed" | "graduated" | string;
  /** The family: the rule id (`c…`) for a composed family. */
  family: string;
  /** Readable name, such as `g0007`. */
  name: string;
  parent: string | null;
  generation: number;
  operator: string;
  lessons: string[];
  /** New specs charged to the ledger by this event; only `tested` rows carry any. */
  trials: number;
  verdict: string | null;
  study: string;
  key: string;
  note: string;
};

/** A proposed family in the lineage tree, with its newest state. */
export type GenNode = {
  family: string;
  name: string;
  generation: number;
  operator: string;
  study: string;
  lessons: string[];
  /** A gate verdict (`REJECT`, …), `PROMOTED`, `KILLED`, `GRADUATED`, or `proposed` when not tested yet. */
  state: string;
  /** Failed checks from the `tested` row, comma separated. */
  failed: string;
  trials: number;
  /** Judged before the ledger's current era (see `FamilyReport.prior`). */
  prior: boolean;
  children: GenNode[];
};

/** A root of the tree: a hand-written family (or none) that generated families descend from. */
export type GenRoot = { family: string; children: GenNode[] };

/** `StudyCost` in `src/lab/evolve.rs`. */
export type StudyCost = {
  study: string;
  trials_before: number;
  trials_after: number;
  design_days: number;
  luck_bar_before: number;
  luck_bar_after: number;
};

/** `ProposalRow` in `src/lab/evolve.rs`. */
export type ProposalRow = {
  name: string;
  family: string;
  operator: string;
  parent: string | null;
  lessons: string[];
  reason: string;
  rule: string;
  specs: number;
  new_trials: number;
  approximate: boolean;
  verdict: string | null;
  failed: string[];
  pick: string | null;
};

/** `Pairing` in `src/lab/compare.rs`: a challenger against one incumbent on common days, bps/day. */
export type Pairing = {
  challenger: string;
  incumbent: string;
  incumbent_spec: string;
  days: number;
  challenger_bps: number;
  incumbent_bps: number;
  diff_bps: number;
  lo_bps: number;
  hi_bps: number;
  corr: number;
  duplicate: boolean;
};

/** `AutoReport` in `src/lab/evolve.rs`: one `lab evolve --auto` run and why it ended. */
export type AutoRun = {
  started: string | null;
  finished: string | null;
  /** Why the run stopped: `passed`, `nothing_new`, `stalled` or `max_generations`. */
  stop: string;
  /** One row per generation the run completed, in order. */
  generations: { generation: number; proposals: number; verdicts: string[]; trials: number }[];
  /** Loop trials per study when the run ended. */
  loop_trials: { study: string; n: number }[];
  /** The per-study trial budget the run was allowed (`[evolve] auto_trial_budget`). */
  budget: number;
};

/** The cron entry that runs `scripts/v3-evolve-hourly.sh`, and the ticks it logged. */
export type EvolveSchedule = {
  /** The five cron fields, or null when no crontab line runs the script. */
  cron: string | null;
  /** Minutes between starts when the entry is a plain "M * * * *" (60), else null. */
  everyMinutes: number | null;
  /** The minute past the hour it starts at, for a plain hourly entry. */
  minute: number | null;
  /** The next start (ISO), for a plain hourly entry. */
  next: string | null;
  /** Logged ticks, oldest first: when each started, when it finished, its exit code. */
  ticks: { at: string; done: string | null; exit: number | null; skipped: boolean }[];
};

/** The `[evolve]` caps in `lab.toml` that the loop runs under. */
export type EvolveCfg = {
  proposals: number;
  specs_per_proposal: number;
  trials_per_cycle: number;
  auto_trial_budget: number;
  /** Per-study overrides (`[evolve.auto_trial_budgets]`). */
  auto_trial_budgets: Record<string, number>;
  /** Cap on new trials per study per `--auto` run; when set, the lifetime budget is unused. */
  auto_trials_per_run: number | null;
  auto_stall: number;
  auto_max_generations: number;
};

/** `CycleReport` in `src/lab/evolve.rs`, one per `lab evolve` run. */
export type CycleReport = {
  /** `g001.json` or `dry-g001.json`. */
  file: string;
  generation: number;
  dry_run: boolean;
  at: string | null;
  lessons_file: string;
  lessons_total: number;
  lessons_usable: number;
  studies: StudyCost[];
  proposals: ProposalRow[];
  pairings: Pairing[];
  /** Child against parent on paper, where both run. */
  forward: Pairing[];
  messages: string[];
  promotable: string[];
  lockbox_openings: number;
};

/** A family judged this era that failed only one or two checks: the measured value of each check it missed. */
export type NearMiss = {
  name: string;
  family: string;
  /** 0 for a hand-written family. */
  generation: number;
  study: string;
  /** When the gate judged it (report time). */
  judged: string;
  /** Trials on its study's ledger when it was judged: the bar the deflated Sharpe check measured it against. */
  trials_ledger: number;
  failed: Check[];
};

export type Generations = {
  roots: GenRoot[];
  /** This era's families one or two checks short, closest first. */
  near: NearMiss[];
  /** Hand-written families on the Lab: not bred by the loop, so not on this page's chart. */
  handWritten: number;
  /** Proposed families in the lineage. */
  proposed: number;
  /** Judged families by their gate verdict (the newest `tested` row of each). */
  verdicts: Record<string, number>;
  /** The highest generation in the lineage. */
  generations: number;
  /** Trials charged by the loop in the ledger's current era. */
  trials: number;
  /** Start of the ledger's current era (`era_start`), or null when the ledger never restarted. */
  era: string | null;
  /** Per study: trials the loop charged, and every trial on the ledger. */
  byStudy: { study: string; loop: number; ledger: number }[];
  /** Family id or root name to its readable name (`c…` → `g0001`). */
  names: Record<string, string>;
  /** Newest first, dry runs included. */
  reports: CycleReport[];
  /** The newest `lab evolve --auto` summary, or null when every cycle was run by hand. */
  auto: AutoRun | null;
  /** Every `lab evolve --auto` run, oldest first. */
  runs: AutoRun[];
  /** When the loop starts on its own, read from the user's crontab and `data-v3/evolve.log`. */
  schedule: EvolveSchedule;
  /** The caps the loop runs under, read from `lab.toml`. */
  cfg: EvolveCfg;
};

// ---- the rules page: every rule on `lab-data/trials.jsonl` ----

/** One rule the lab simulated: a point on a family's grid, with its in-sample numbers. */
export type RuleRow = {
  /** Spec id, `family:k=v,...`. */
  id: string;
  study: string;
  /** Family id (rule hash for a generated family). */
  family: string;
  /** Settings parsed from the spec id. */
  params: Record<string, number>;
  trades: number;
  days: number;
  /** Annualised Sharpe (daily × √365) and mean bps per day, in sample after costs. */
  sharpe: number;
  bps: number;
  at: string;
  /** The family's in-sample pick, and how many walk-forward folds picked this rule. */
  pick: boolean;
  folds: number;
};

/** A family as the rules page needs it. */
export type RuleFamily = {
  key: string;
  family: string;
  study: string;
  name: string;
  generation: number | null;
  verdict: string | null;
  /** Checks failed; null when the gate never ran its checks. */
  short: number | null;
  /** Readable statement of the family's rule, from its hypothesis. */
  statement: string;
  specs: number;
};

/** One walk-forward test window: the rule a family's fold picked and how it did out of sample. */
export type WalkCell = {
  /** Spec id of the picked rule. */
  spec: string;
  /** Mean bps a day over the window, out of sample, after costs. */
  bps: number;
  /** Annualised Sharpe over the window. */
  sharpe: number;
};

/** A family's walk-forward, aligned to `RuleWalk.windows`; null where its fold picked nothing (flat). */
export type WalkFamily = { key: string; cells: (WalkCell | null)[]; mean: number };

export type RuleWalk = {
  /** Test windows on the calendar, oldest first: first day and the day after the last. */
  windows: { start: string; end: string }[];
  /** First sealed day, the earliest across studies. */
  lockbox: string | null;
  families: WalkFamily[];
  /** Families with a report but no walk-forward folds (stopped before the checks). */
  none: string[];
};

export type Rules = { rows: RuleRow[]; families: Record<string, RuleFamily>; walk: RuleWalk };
