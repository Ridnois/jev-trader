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
  funding: number;
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

export type Lab = {
  families: FamilyReport[];
  probes: ProbeReport[];
  hypotheses: Hypothesis[];
  calibrations: Calibration[];
  trials: number;
  trialsByStudy: { study: string; n: number }[];
};
