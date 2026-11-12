/**
 * One record per term a reader has to hold to read the numbers.
 * A page renders the term through <Gloss id> and lists the same records in its
 * "What these words mean" disclosure, so the wording never forks.
 *
 * Rules: plain words, a real unit, the bar where one exists, and the misreading
 * the record exists to prevent. Never a new term invented by the copy.
 */

export type GlossaryRecord = {
  /** The word as it appears on screen. */
  term: string;
  /** One sentence, plain words. */
  meaning: string;
  /** The bar or direction of good, stated as a number where one exists. */
  good?: string;
  /** What a reader must not conclude from it. */
  misreading: string;
};

const RECORDS = {
  bps: {
    term: "bps",
    meaning: "Basis points: 1/100 of a percent, so 100 bps is 1%.",
    misreading: "Not dollars, and not an annual rate.",
  },
  family: {
    term: "Family",
    meaning: "One idea about how a market behaves, written as a rule plus the list of variations the lab may try.",
    misreading: "A family is not a strategy, and not a single rule.",
  },
  grid: {
    term: "Grid",
    meaning: "The variations a family allows. Nothing outside the declared grid is tested.",
    misreading: "A larger grid is not more evidence; it makes the winner harder to trust.",
  },
  spec: {
    term: "Rule",
    meaning: "One point on a family's grid, named by its settings, such as trend:n=384,out_frac=0.25,shorts=0,stop=800.",
    misreading: "A rule is not the family it came from.",
  },
  trial: {
    term: "Trial",
    meaning: "One rule simulated on one data set, appended to the ledger forever.",
    good: "Fewer, better-motivated trials.",
    misreading: "Trials are not free: every one raises the bar the next winner must clear.",
  },
  strategy: {
    term: "Strategy",
    meaning: "A rule that was promoted, given a fixed forecast (its card), and is running on paper.",
    misreading: "Unproven means watched, not trusted. Paper is not live money.",
  },
  card: {
    term: "Card",
    meaning: "The forecast fixed when a strategy was promoted: expected return per day, typical daily swing, and when the test can decide.",
    misreading: "It is a forecast made at promotion, not a result achieved since.",
  },
  gate: {
    term: "Gate",
    meaning: "The fixed set of checks a family must pass before it may run on paper. The checks are set before any real data is judged.",
    good: "Every check passes, then the lockbox passes.",
    misreading: "A rejected family is not proof that no version could work; it is proof that this search did not find one.",
  },
  lockbox: {
    term: "Lockbox",
    meaning: "The newest days, kept out of every search. Opened only for a family that has already passed everything else.",
    good: "Sealed is the normal state.",
    misreading: "Opening it again doubles the bar for the next look.",
  },
  "in-sample": {
    term: "In sample",
    meaning: "The days the search was allowed to look at. Its winner was chosen from many rules, so it flatters itself.",
    misreading: "The best in-sample number is the view that fools people; it is not what the gate judged.",
  },
  sharpe: {
    term: "Sharpe",
    meaning: "A rule's yearly return divided by how much it swings day to day.",
    good: "Higher is better. 1 is respectable, 2 is rare.",
    misreading: "This dashboard annualises daily returns by sqrt(365), not sqrt(252).",
  },
  "walk-forward": {
    term: "Walk-forward",
    meaning: "Fit the rule on 180 days, trade it on the next 60 it never saw, repeat across the history. The figure is the mean after costs.",
    good: "Above 0, with the whole 95% bracket above 0.",
    misreading: "The bracket is about the estimate, not a promise about the next 60 days.",
  },
  "reality-check": {
    term: "Reality check",
    meaning: "How often a reshuffled history with no edge left in it produces a best rule this good.",
    good: "Under 0.05 passes. Over 0.20 is fatal.",
    misreading: "It is not the probability that the strategy is luck. p = 0.22 means luck does this about one time in five.",
  },
  "deflated-sharpe": {
    term: "Deflated Sharpe",
    meaning: "The chance the winner beats the luckiest of all the rules tried, after adjusting for how many were tried.",
    good: "0.95 or more passes. Under 0.50 is fatal.",
    misreading: "It is not a 95% chance the strategy is real.",
  },
  pbo: {
    term: "Overfitting (PBO)",
    meaning: "The chance the in-sample winner lands below the median out of sample.",
    good: "0.30 or lower.",
    misreading: "It judges the selection procedure, not the single pick.",
  },
  plateau: {
    term: "Plateau",
    meaning: "Whether the rule's neighbours on the grid also work, or the pick is a lone spike.",
    good: "Every neighbour positive, median at least half the pick.",
    misreading: "A sharp isolated peak is a warning even when the pick looks good.",
  },
  "paper-day": {
    term: "Full paper day",
    meaning: "A UTC day in which the engine was up at least 90% of the time.",
    misreading: "Partial days are shown but left out of the test, the return and the drawdown.",
  },
  "sequential-test": {
    term: "Sequential test",
    meaning: "Running evidence for the card's forecast against no edge, updated once per full paper day.",
    good: "Rising above 0 is evidence for the card; falling below 0 is evidence against it.",
    misreading: "At day 0 nothing has been counted yet; later, 0 means the counted days balance. A flat line means few full days, not no news.",
  },
  llr: {
    term: "Log-likelihood ratio",
    meaning: "The running score for the card's forecast against no edge, updated once per full paper day.",
    good: "Rising means the card is beating no edge; falling means it is losing to it.",
    misreading: "Not a probability and not a P&L. A small number after many days is evidence against the card.",
  },
  parity: {
    term: "Parity",
    meaning: "Whether the paper engine and the simulator entered the same trades on the same bars, counted from both sides.",
    good: "The same entries on both sides; a late join is not a disagreement.",
    misreading: "A late join means paper started flat on a position the simulator already held. It is not an engine bug.",
  },
  drawdown: {
    term: "Drawdown",
    meaning: "The fall from the highest paper return so far, compared with the fall the backtest's own history allows.",
    good: "Below the band for the days elapsed.",
    misreading: "Crossing the band is a kill signal; it does not mean the rule is broken.",
  },
  markout: {
    term: "Markout",
    meaning: "What the price did 5 min, 30 min, 2 h and 6 h after a headline, net of the round-trip cost.",
    good: "Positive and better for triggered headlines than for all headlines.",
    misreading: "A markout is not a trade. Nothing was bought or sold.",
  },
  trigger: {
    term: "Triggered",
    meaning: "A headline that passed all three checks: impact, new information, and freshness.",
    misreading: "Triggered does not mean profitable; check the markout table.",
  },
  impact: {
    term: "Impact",
    meaning: "Jev's score from -1 to +1 for how big a move the headline argues for. The sign carries the direction.",
    good: "Absolute size at least the impact bar set in lab.toml, printed under the news table.",
    misreading: "A score, not a return, and not a probability.",
  },
  "new-info": {
    term: "New info",
    meaning: "How much of the story was already known, from 0 to 1.",
    good: "At least the new-info bar set in lab.toml, printed under the news table.",
    misreading: "It is Jev's judgement, not a measurement.",
  },
  lag: {
    term: "Late by",
    meaning: "Minutes between publication and the moment Jev read the headline.",
    good: "Under the maximum lag set in lab.toml, printed under the news table.",
    misreading: "A big lag means the move may already have happened.",
  },
  jev: {
    term: "Jev",
    meaning: "The language model that scores headlines and answers the probes. It is not a strategy and it never trades.",
    misreading: "Its answers are evidence about what may happen, not orders.",
  },
  probe: {
    term: "Probe",
    meaning: "A fixed question asked about a stretch of market state with the coin, date and price level hidden.",
    misreading: "A probe tests Jev's read. It does not trade and does not prove a strategy.",
  },
  auc: {
    term: "AUC",
    meaning: "The chance a higher answer ranks a true outcome above a false one.",
    good: "Above 0.55 to count as skill, with the shuffle test.",
    misreading: "AUC is ranking skill, not calibration.",
  },
  "rank-correlation": {
    term: "Rank correlation",
    meaning: "How closely Jev's answer tracks the signed return that followed.",
    good: "Above 0.",
    misreading: "A negative reading is reliably wrong, not useless.",
  },
  "shuffle-test": {
    term: "Shuffle test",
    meaning: "The score is compared with many histories reshuffled in day blocks, so any edge in them is destroyed, and the percentile says how often chance did as well.",
    good: "Beating 95% of reshuffles.",
    misreading: "It is a percentile of a no-edge distribution, not a p-value for the strategy.",
  },
  brier: {
    term: "Brier skill",
    meaning: "Accuracy against always answering the base rate.",
    good: "Above 0. 0 is no better than the base rate.",
    misreading: "Calibration and skill are different; a calibrated answer can still be useless.",
  },
  "evidence-days": {
    term: "Evidence days",
    meaning: "Distinct UTC days that contain at least one measured outcome.",
    good: "20 days and 30 triggered headlines before the averages are worth reading.",
    misreading: "Below that it is an anecdote, not a result.",
  },
} as const;

export type GlossId = keyof typeof RECORDS;

/** Widened values, exact keys: every page gets a checked id without losing `good`. */
export const GLOSSARY: Record<GlossId, GlossaryRecord> = RECORDS;

export const gloss = (id: GlossId): GlossaryRecord => GLOSSARY[id];
