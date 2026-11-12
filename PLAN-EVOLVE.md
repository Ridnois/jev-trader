# Plan: a lab that learns and proposes its own strategies

*Written 2026-11-12 for the agent that will implement it. Branch `v3`, worktree `trading-v3`.*

Read `DESIGN-V3.md` first (15 minutes). This plan changes how strategies are **created**; it must not weaken how they are **judged**.

## 1. The goal

Today a new strategy needs a person to write a Rust function, a family file and a hypothesis, then run four commands. Nothing reads the results back: a REJECT ends as a markdown file, and paper results end in `data-v3/monitor.md`.

After this work, one command, run when the operator asks for it, does this:

```
lab evolve
  1. learn     read every backtest verdict and every paper result → a digest of lessons
  2. propose   turn the lessons into a few new strategies, each with a written reason
  3. test      run them through the existing search and gate, every spec charged to the ledger
  4. compare   rank challengers against incumbents on the same days
  5. report    one page: what was learned, what was proposed, what survived, what it cost
```

"On demand" means a command that runs once and exits. No daemon, no schedule, no automatic promotion to paper.

## 2. What must not change

These are the reason v3 exists. A change that breaks one of them is wrong even if every test passes.

1. **Every simulated spec is charged.** Anything the proposer causes to be simulated on a study goes to `lab-data/trials.jsonl` before its result is read. That includes specs simulated while *deciding what to propose*.
2. **The lockbox stays sealed.** The learner and the proposer see design-window days only. Use `Panel::head(design_bars)` the way `lab run` does. The lockbox opens only inside `gate::evaluate`, under the existing rule.
3. **Gate thresholds in `lab.toml` are not touched.** No per-generation loosening.
4. **One simulator, one plan.** A generated strategy runs through `strategy::plan` → `sim::simulate` in the lab and through the same `plan` → `Book` in the trader. No second code path.
5. **Hypothesis before test.** Each proposal gets a `hypotheses.toml` entry, written before its first simulation.
6. **Nothing is promoted automatically.** `lab evolve` ends with a report. `lab promote` stays a separate, deliberate command.
7. **The no-look-ahead check covers every rule**, generated ones included, automatically.

## 3. Where the code is now

| Piece | Where | Note |
|---|---|---|
| Rules | `src/lab/strategy.rs`, `plan()` at line 147 | A `match` on the family name; each family is a hand-written function. `Spec` is a family name plus numeric params. |
| Families | `families/*.toml` | Rule name, interval, hypothesis, `[grid]`, `[fixed]`. |
| Search and ledger | `src/lab/search.rs` | `run_grid`, `walk_forward`, `record_trials`. Ledger rows: study, family, spec, days, trades, sharpe_daily, mean_bps_day. |
| Gate | `src/lab/gate.rs`, `evaluate()` | Writes `lab-data/reports/<study>/<family>.{json,md}`. The JSON has `checks`, `walk`, `fold_picks`, `leaders`, `pick`, `verdict`, `card`, `notes`. |
| Promotion | `src/lab/mod.rs`, `promote()` | Writes `strategies/<name>.json` (status, hypothesis, prediction card). |
| Paper trader | `src/trade/mod.rs`, `book.rs` | Loads `strategies/*.json`, splits capital equally, writes ledger, trades and daily rows under `data-v3/`. |
| Monitor | `src/trade/monitor.rs` | Parity with the simulator, sequential test, drawdown envelope. Writes `data-v3/monitor.md` only. |
| Calibration | `src/lab/synth.rs`, `lab calibrate` | The gate on synthetic markets with a known answer. |
| Dashboard | `web/` (Next.js, read-only) | Reads the files above. |

State on 2026-11-12: seven verdicts on file (five REJECT, two INSUFFICIENT_EVIDENCE), 1,152 ledger rows, lockbox never opened, two unvalidated trend candidates on paper with zero full days. The `hvt` and `ensemble` families and the dashboard are uncommitted work in the tree: do not revert or reformat them.

## 4. Design

### 4.1 Strategies fully as data: a composed rule

Generating a strategy cannot mean generating Rust. Add one new family kind whose rule is built from a fixed library of blocks, described in the family file.

- **Features** (each a pure function of bars `0..=i`, per coin): return over n bars, rolling z of close, position inside the n-bar channel, volume against its rolling median, realised volatility, funding rate, residual against the coin basket, cross-sectional rank of any of these. Reuse what `strategy.rs` and `probe.rs::Featurizer` already compute; do not write second copies.
- **Entry**: a side rule (long when a condition holds, short when its mirror holds) plus zero or more filters that must also hold (for example "only when basket volatility is below its median").
- **Exit**: time stop, hard stop, opposite signal, or a feature crossing a level. These map onto the existing `Policy` and `Plan.exit`.
- **Parameters** stay numeric and stay in `[grid]` / `[fixed]`, so `Spec`, spec ids, the ledger and the plateau check keep working.

Sketch (the agent may refine the syntax, not the idea):

```toml
family = "g0007"
interval = "1h"
hypothesis = "H-G0007"
parent = "trend"                     # lineage
reason = "lessons/2026-11-12.json#L3"

[rule]
long  = { feature = "channel_pos", n = "$n", op = ">=", level = 1.0 }
short = "mirror"
filter = [{ feature = "basket_rvol_rank", n = 720, op = "<", level = "$vol_cap" }]
exit  = { kind = "channel", frac = "$out_frac" }

[grid]
n = [96, 384]
vol_cap = [0.5, 0.8]
out_frac = [0.25]
```

Requirements:
- `plan()` needs the rule structure, not just the spec. Change it so a composed family resolves its rule from a registry loaded from `families/`; hand-written families keep working unchanged.
- A promoted composed strategy must run in the trader without depending on a family file that may later change: embed the rule (and its hash) in `strategies/<name>.json`.
- Two rules with the same structure and params must get the same id, so a re-proposed idea is recognised as already tried instead of being charged and judged twice under a new name.
- The look-ahead test must enumerate every family file under `families/` instead of a hand-kept list.
- Prove the block library is honest by re-expressing `trend` as a composed rule and requiring identical trades to the hand-written one on the test panel.

### 4.2 The learner: `lab learn`

Reads files only, writes `lab-data/lessons/<date>.json` and a readable `.md`. Simulates nothing new unless a breakdown is missing, and then it charges the ledger.

Backtest evidence, per family:
- which gate checks failed and by how much (from `checks`)
- parameter response: which grid dimensions move the result and which are flat (from the ledger rows of that family)
- where the pick made and lost money: by coin, by quarter, by long against short, by market state (basket trend up or down, volatility high or low). Per-quarter and per-state breakdowns are not stored today; extend the evaluation JSON to keep the pick's daily returns and per-coin P&L for the design window, so the learner does not have to re-simulate.
- walk-forward instability: how often the fold pick changed
- redundancy: correlation of daily returns between families' picks

Forward evidence, per paper strategy:
- the monitor's calls (parity, sequential test, drawdown). First refactor `monitor.rs` to return a struct and write `data-v3/monitor.json` beside the markdown; the learner reads that.
- execution reality against the model: realised entry and stop slippage, funding paid, late joins, entries only in one engine
- catalyst recorder markouts

Each lesson is a typed record: `{id, kind, subject, finding, evidence: [file refs], strength}`. A lesson with fewer observations than a stated minimum is labelled an anecdote and the proposer may not use it.

Two rules for forward evidence:
- **Paper days tune nothing in place.** A paper result may kill a strategy or motivate a child. It never adjusts the running strategy's parameters, and a child starts its own paper clock with its own card.
- **Execution lessons go to the operator, not into `lab.toml`.** If paper slippage disagrees with `[costs]`, the report says so and proposes the change. Editing the cost model re-prices every past trial, so it stays a human decision.

### 4.3 The proposer: `lab propose`

Input: the latest lessons file and the lineage registry. Output: up to N family files under `families/gen/`, each with a hypothesis entry and a one-paragraph reason that cites lesson ids.

Operators, all deterministic given `lab.seed` and the generation number:

| Operator | When | Example |
|---|---|---|
| Condition | a rule earns in one market state and loses in another | add a filter for that state |
| Restrict | losses concentrate in one side or one holding length | drop shorts, shorten the hold |
| Recombine | two picks with low return correlation each have a strong part | one rule's entry with another's exit |
| Invert | a rule loses persistently and broadly | the opposite side, as its own hypothesis |
| Shrink | the grid has flat dimensions | the same rule with those fixed, fewer trials |
| Explore | budget is left over | a new feature pairing not yet in the lineage |

Constraints:
- Budget in a new `[evolve]` section of `lab.toml`: proposals per cycle (suggest 4), specs per proposal (suggest 12), trials per cycle (suggest 40). Hitting a limit stops the cycle with a message; it is never silently raised.
- Never re-propose a rule id already in the lineage.
- The proposer does not simulate. If a later version needs a quick look to choose between ideas, that look is charged like any other trial.

Keep the proposal file format as the interface. A second proposer (an LLM or the operator) can then write the same files and go through the same door; build only the deterministic one now. See "Open decisions".

### 4.4 The tournament: `lab compare`

- Runs each new proposal through the existing `search` + `gate::evaluate`. No new judging code.
- **Generation-wide multiple testing.** A cycle's proposals were chosen together, so add one reality check over the union of all specs simulated in the cycle, reported beside each family's own. A family that passes its own check while the generation as a whole fails gets REVISE at best.
- **Incumbents.** Every strategy in `strategies/` and the best spec of every family with a verdict. Compare on common design-window days: mean and Sharpe, a block-bootstrap interval on the *difference* in daily returns, and the return correlation. A challenger that is 0.9 correlated with an incumbent is a duplicate, whatever its Sharpe.
- **Forward comparison.** Where both have paper days, the same paired comparison on those days, with the day count printed first.

### 4.5 Lineage: `lab-data/lineage.jsonl`

Append-only, one row per event: proposed, tested (with verdict), promoted, killed, graduated. Each row carries the rule id, parent, generation, operator, lesson ids and trials charged. `lab lineage` prints the tree. This is the app's memory: the learner reads it so it stops proposing descendants of a branch that keeps dying.

### 4.6 The paper side

- Check what `trade run` does to existing books when a strategy file is added (capital is split equally today, so a third strategy would shrink the first two mid-run). Change to a fixed paper capital per strategy so adding a challenger does not disturb an incumbent's sizing or its sequential test.
- A promoted challenger records its parent in `strategies/<name>.json`, and the monitor shows parent and child side by side.
- A KILL from the monitor is written to the lineage. It still stops nothing by itself.

### 4.7 `lab evolve`

Runs learn → propose → compare → report in order and writes `lab-data/reports/evolve/<generation>.{json,md}`. Flags: `--dry-run` (learn and propose, simulate nothing, charge nothing), `--max-trials`, `--from-lessons <file>`.

The report states, in this order: trials charged this cycle and in total, what that did to the deflated-Sharpe bar, lessons used, each proposal with its reason and verdict, the comparison table, and what the operator might promote (possibly nothing).

## 5. The statistical problem this creates, and the test that answers it

A loop that reads results and proposes again is a search, and it is a wider one than a grid. Three consequences the implementation must handle:

1. **The walk-forward check is weaker for generated rules.** The rule's structure was chosen after looking at the whole design window, so its out-of-sample folds are not really out of sample. Say this in every report for a generated family. The lockbox and paper days are the only clean evidence for them.
2. **The deflated-Sharpe bar rises with every cycle.** That is correct. The report must show it, so the operator sees that ten cycles of four proposals cost more than they are likely to return.
3. **Each lockbox opening tightens the next** (already implemented per study). Generated families share their study's count.

**The acceptance test for the whole design:** extend `lab calibrate` to run the full evolve loop, several generations deep, on synthetic markets.

| Synthetic market | Requirement |
|---|---|
| No edge, at least 100 panels, 5 generations each | 0 promotions |
| Planted strong edge | promotion rate not materially below the single-family figure in `DESIGN-V3.md` §3 |

If the loop promotes noise, fix the loop (budget, generation-wide check), not the thresholds. Record the calibration table in `DESIGN-V3.md`.

## 6. Order of work

Each phase ends with `cargo test` green and one commit. Build with `-j 2` (`cargo build --profile lab -j 2`).

| Phase | Work | Done when |
|---|---|---|
| 0 | Commit or set aside the existing uncommitted work with the operator's agreement; do not mix it into these commits. | Clean base agreed. |
| 1 | Composed rules: block library, rule registry, `plan()` dispatch, rule embedded in promoted files, look-ahead test over all family files. | `trend` re-expressed as a composed rule gives identical trades; the trader parity test passes for a composed rule. |
| 2 | Evidence plumbing: evaluation JSON keeps the pick's daily and per-coin series; monitor returns a struct and writes `monitor.json`. | Existing reports regenerate with the same verdicts. |
| 3 | `lab learn` and the lessons schema. | Run on today's files, it reports the known findings: trend is long-only and regime-dependent (5 of 9 quarters), meanrev loses on every coin. |
| 4 | Lineage registry, `lab propose`, `[evolve]` budget, automatic hypothesis entries. | Same seed and lessons give the same proposals; `--dry-run` charges nothing. |
| 5 | `lab compare` with the generation-wide reality check and paired comparisons. | Tested on a synthetic panel with one planted edge and three noise rules. |
| 6 | `lab evolve`, and `lab calibrate` over the whole loop. | The table in section 5 holds. |
| 7 | Paper side: fixed capital per strategy, parent and child in the monitor, kills written to lineage. | Adding a strategy file leaves existing books' sizing unchanged. |
| 8 | Dashboard: a read-only "Generations" page (lineage tree, per-cycle report, trials spent). Follow `web/DESIGN.md`. | Page renders from files with no generation yet and with one. |
| 9 | Update `DESIGN-V3.md` (pipeline, calibration table, limits) and the README run section. | |

Phases 1 to 6 are the feature. 7 and 8 can follow in a second pass.

## 7. Constraints on the agent

- **The paper trader is running** (`trade run`, started 2026-11-11, managed by `scripts/v3-services.sh`). Do not stop or restart it, and do not write into `strategies/` or `data-v3/`, without asking. Phase 1 changes the promoted-file format: old files must still load.
- Run the first real `lab evolve` only with `--dry-run`, then ask. Real cycles spend trials on the real ledger and that cannot be undone. Use a scratch `data_dir` for development and tests.
- Never open the lockbox to "see if it works". Tests that need a lockbox use synthetic panels.
- No new dependencies without a reason stated in the commit. No network calls in learn, propose or compare.
- Report what failed. If the calibration promotes noise, that result goes in the write-up.

## 8. Not in scope

- Automatic promotion, automatic capital allocation, or anything that places real orders.
- A scheduler or background service.
- Changing gate thresholds, the cost model or the universe.
- Generating Rust code, or free-form expression trees. The block library is closed and small on purpose: every block added widens the search.
- Tuning Jev probes. Jev families can be parents of a proposal only where their replay cache already exists.

## 9. Open decisions for the operator

1. **Who proposes.** The plan builds a deterministic proposer. An LLM writing proposal files would be more inventive and harder to calibrate, because the same lessons would not give the same proposals. Recommended: deterministic first, LLM later through the same file format, with its own calibration run.
2. **Budget per cycle.** 4 proposals, 12 specs each, 40 trials is a starting guess. The ledger has 1,152 rows today; each cycle of 40 adds about 3%.
3. **A learning window.** Stricter alternative to section 5 point 1: let the learner see only the first two thirds of the design window, and keep the last third as a confirmation window that generated rules never influenced. Costs about 280 days of evidence. Recommended only if the loop calibration shows promoted noise.
4. **Existing uncommitted work** (`hvt`, `ensemble`, dashboard): commit before starting, or keep separate.
