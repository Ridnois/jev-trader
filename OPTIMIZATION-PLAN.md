# jev-trader optimization plan

Ideas from two open-source projects, adapted to jev-trader and checked against the current code.

- **[TauricResearch/TradingAgents](https://github.com/TauricResearch/TradingAgents)** (TA): a pipeline of LLM agents (analysts → bull/bear debate → trader → risk debate → portfolio manager). Its changelog is a record of real pitfalls it fixed.
- **[tradermonty/claude-trading-skills](https://github.com/tradermonty/claude-trading-skills)** (CTS): about 75 Claude skills for a discretionary trader. They cover regime scoring, sizing, circuit breakers, postmortems, and edge-research tooling. Its stance is that AI structures the process and never takes the decision, which matches PRODUCT.md principle 3.

> **Note on numbers:** This plan claims no improvement in returns. The bot hasn't run with a Jev key yet. The order is **measure first, then change**: every strategy change below is a hypothesis to test on settled paper data. Most CTS thresholds are labelled by their own authors as "heuristic, conservative defaults, not backtested optima", and this plan treats them the same way.

---

## 1. What each source contributes

### TradingAgents

| Idea | What it fixes |
|---|---|
| Decision log + settlement + reflection | Decisions are scored against the realized return and alpha vs a benchmark once their holding period ends. |
| `run_backtest` into a separate log | Replays never touch live memory, and runs can be resumed. "Hold reports no hit rate." |
| Point-in-time discipline | Entry is the first close after the decision, and nothing dated later than the decision is used. |
| No silent defaults (`REVIEW`) | An unparseable answer stays visibly unparsed instead of becoming Hold. |
| Verified data snapshot | Only prices the vendor actually reported, never gap-filled values. |
| Bull/bear debate | Each proposed trade is reviewed adversarially before it goes ahead. |
| Deep/quick model tiers | Expensive analysis is spent only where a decision is made. |
| Retry budgets and caps | After the budget runs out, continue with what you have and say so. |

### claude-trading-skills

| Skill | Idea for us |
|---|---|
| **crypto-regime-analyzer** | A 0–100 crypto regime score built from six keyless components: BTC 50/200-day trend (25%), alt breadth above the 200-day average (20%), BTC dominance (15%), perp funding, contrarian at extremes (15%), drawdown and volatility percentile (15%), 30-day momentum thrust (10%). Missing components have their weight redistributed. Bands: RISK_OFF 0–39, NEUTRAL 40–79, RISK_ON 80–100. |
| **drawdown-circuit-breaker** | Beyond a daily loss limit: a **weekly (5%) and monthly (8%) drawdown halt**, a **losing-streak cooldown** (2 losing trades in a row means 24h with no entries), and a **fail-closed** rule that halts on incomplete, duplicated, or non-finite state. States are TRADING_ALLOWED, COOLDOWN, and HALTED. Halts block entries but never force liquidation. |
| **news-reaction-failure-analyzer** | Detects "good news, but price didn't follow". Its method matters more than the feature. A naive failure ratio "confirms" on pure noise **48–83%** of the time. Their fix is a volatility-normalised drift z-score, with **overlapping event windows merged into one independent observation**, checked by Monte Carlo against i.i.d. and autocorrelated noise. |
| **signal-postmortem** | Labels each outcome TRUE_POSITIVE, FALSE_POSITIVE, **MISSED_OPPORTUNITY** (a signal not taken that would have paid), or **REGIME_MISMATCH**. It records the regime at signal time and at exit. Weight suggestions need **at least 20 samples** and go to an improvement backlog, never straight into the rules. |
| **stockbee-setup-fluency-trainer** | Per setup: **MFE/MAE** over each horizon, whether the stop was hit and when, and outcome tags (WORKED, FAILED_STOP, FAILED_FADE, CHOPPY_FAILURE, NEUTRAL). A record stays PENDING until enough bars exist. **Rejected candidates are kept as negative examples.** Cohort stats need at least 5 samples. |
| **position-sizer** | Volatility (ATR) sizing, a **portfolio heat** cap (total open risk), the strictest constraint wins, and **the binding constraint is reported**. Half-Kelly at most, never full Kelly. |
| **residual-edge-analyzer** | Regresses strategy returns on simple baselines using HAC/Newey-West errors. The **primary baseline is declared before looking at results**, and at least one alternate baseline is required. Verdicts: RESIDUAL_EDGE, BASELINE_EXPLAINED, RESIDUAL_FRAGILE, INSUFFICIENT_EVIDENCE. |
| **edge-strategy-reviewer** | Scores a proposed strategy change on plausibility, **overfitting risk (filter/parameter count)**, sample adequacy, regime dependency, exit calibration, and execution realism, then returns PASS/REVISE/REJECT. Has an optional bias checklist. |
| **pre-trade-discipline-gate**, **contrarian-setup-gate** | Fail-closed state machines. An entry goes ahead only when every check passes, and the result is logged. |
| **exposure-coach** | A one-line "market posture": an exposure ceiling plus *new entries allowed* or *cash first*. |
| **trade-hypothesis-ideator** | Ideas are written as **falsifiable hypotheses** with an explicit invalidation condition. |
| **data-quality-checker** | Offline checks for units, dates, and scale before anything is published. |

---

## 2. Gaps in the current code

1. **Missing momentum silently becomes `0.0`.** `src/bot.rs` uses `moms.get(..).unwrap_or(0.0)`. *(TA: REVIEW)*
2. **A missing publish date silently becomes now.** `src/news.rs` uses `unwrap_or_else(Utc::now)`. Undated items get full freshness and slip past `max_age_hours`. *(TA: point-in-time)*
3. **A missing price values a position at entry.** `State::equity` falls back to `p.entry`, and the kill switch then runs on that made-up equity.
4. **The open candle is used as if closed.** Binance returns the in-progress 1h candle last, so momentum changes within the hour. There's also no staleness check.
5. **No check for non-finite or out-of-range values.** A NaN from Jev or klines would flow straight into sizing and the kill switch. *(CTS: fail-closed)*
6. **Only the first gate reason is logged.** The `why` chain in `bot::tick` stops at the first match. `apply_caps` scales weights down without recording which cap bound. Neither the operator nor a later analysis can tell what actually limited a trade. *(CTS: binding constraint)*
7. **Sizing ignores volatility.** Weight is `max_asset_weight × news_score` for every asset, and the stop is a flat 4%. SOL typically moves 2–3× more than BTC, so the same weight carries very different risk, and a 4% stop is far tighter for SOL than for BTC. *(CTS: position-sizer)*
8. **Thin drawdown control.** Only a daily −3% kill switch exists. There's no weekly or monthly halt and no losing-streak cooldown, and the cooldown applies only to an asset after its own stop. Fills don't record realized P&L, so a streak can't even be computed. *(CTS: circuit breaker)*
9. **No awareness of the market regime.** Entry logic is the same in a broad crypto selloff as in a broad rally. *(CTS: regime)*
10. **Nothing is scored.** That includes trades the gates blocked: nobody knows what the kill switch, cooldown, or pause cost or saved. *(TA settlement + CTS MISSED_OPPORTUNITY)*
11. **The same story is counted several times.** CoinDesk, Cointelegraph, and Decrypt often cover one event, and the score grows with the number of outlets. *(CTS independence guard)*
12. **No Jev retry budget or circuit breaker.** A failing headline can be retried up to ~360 times. *(TA: retry budgets)*
13. **The time horizons don't match.** News half-life is 2h, momentum is a 20h SMA, and nothing has measured which works.
14. **No replay or backtest path**, even though Jev answers are already cached in `events.jsonl`.

---

## 3. Working rules for the whole plan

These come from both repos and apply to every phase:

- **Fail closed.** Missing, stale, non-finite, or contradictory input means **no new entries** for that asset, shown as `REVIEW`. Exits and stops still run, but only on verified prices.
- **Suggest, never self-apply.** Analyses produce suggestions with a sample size n. Only the operator edits `config.toml`.
- **Minimum samples.** n ≥ 20 before any weight suggestion. Statistics with n < 30 are shown greyed out. Cohorts with n < 5 are hidden.
- **Defaults keep today's behaviour.** Each new config knob ships with a default that reproduces the current strategy, so adding it changes nothing until the operator changes it.
- **Write it down before testing.** Each strategy change is entered in `hypotheses.toml` *before* it's tested (see §4.6). The baseline is declared before results are seen.

---

## 4. The plan

### Phase 0: Honest, fail-closed inputs (small, do first)

- **0.1 `REVIEW` state.** Make momentum an `Option` all the way into `signals::target`. When momentum is `None`, the target is `Hold`, with reason `review: no momentum (N candles)`. Add `review: Option<String>` to `TickRecord`. The dashboard shows REVIEW as a state distinct from Hold.
- **0.2 Undated news never counts as fresh.** Drop items with no date, or give them weight 0 with a `dated: false` flag. Add `first_seen_at` to `NewsEvent`, which Phases 1 and 2 rely on.
- **0.3 Use closed candles only.** Parse `close_time` (kline field 6) and drop candles with `close_time > now`. If the newest closed candle is older than 2× the interval, the Prices stage is `Degraded` and the asset is `REVIEW`.
- **0.4 Missing price means REVIEW, not entry price.** Don't evaluate the kill switch on fallback prices.
- **0.5 Validate every number.** Add a `validate()` pass for Jev answers (finite values, probabilities sum to about 1, sentiment within −1..1), for candles (finite and positive, high ≥ low), and for `state.json` (finite cash, positions, and equity). Any failure sets the asset or the whole account to REVIEW/HALTED for entries and logs the reason. *(CTS data-quality-checker plus the circuit breaker's "non-finite → halt")*
- **0.6 Record every gate.** Replace the single `why` string with a typed `Vec<GateResult { gate, passed, detail }>` covering: inputs, flatten, kill switch, drawdown halts, streak cooldown, stop, asset cooldown, regime (later), challenge (later), pause. Have `apply_caps` return the **binding constraint** for each asset: asset cap, gross cap, heat cap, rebalance band, or min trade. Store both on `TickRecord` and `Fill`. This gives the dashboard's "explain this decision" a complete trace and makes the gate-by-gate scoring in Phase 1 possible.
- **0.7 Tests** for every case above.

### Phase 1: Settlement and postmortem (core measurement)

Goal: score every decision, headline, and blocked trade once its horizon has passed. This combines TA's settle-and-reflect with CTS's signal-postmortem and setup-fluency trainer, using plain statistics and no LLM.

- **1.1 A `settle` step at the end of each tick, writing `data/settlements.jsonl`.** Horizons are **1h, 4h, 24h**, all on closed 1h candles. A record stays `PENDING` until its last candle closes. It's settled exactly once, and a failed settlement is retried without blocking trading.
- **1.2 What gets settled:**
  - **Headlines.** Return of the asset from `first_seen_at`, never `published`, to +h. Also a **volatility-normalised reaction z-score**: forward return divided by trailing realized vol × √h. This puts BTC and SOL reactions on the same scale.
  - **Decisions** (each `TickRecord` with a change of direction). Forward return, excess return vs BTC (BTC is the benchmark for ETH/SOL), **MFE and MAE** over each horizon, whether the stop would have been hit and when, and an **outcome tag**: WORKED, FAILED_STOP, FAILED_FADE, CHOPPY, NEUTRAL. The thresholds are set in units of realized vol, not in fixed percentages.
  - **Blocked and held decisions (counterfactuals).** When a gate turned a would-be entry into a no-trade, settle the hypothetical entry too and label it **MISSED_OPPORTUNITY** or **CORRECTLY_BLOCKED**. Gate results come from 0.6. This gives each gate a value: what the kill switch, cooldown, pause, and later the regime and challenge gates saved or cost. That's the only honest way to tune a gate.
  - **Regime at signal and at exit** (once Phase 4 exists), so failures can be labelled **REGIME_MISMATCH**.
- **1.3 Statistics, with defences against noise.** This is CTS's main methodological lesson:
  - **Independence guard.** Headlines and decisions whose windows overlap for the same asset are merged into one cluster before any statistic is computed. With 60s ticks and a 24h horizon, unmerged samples are heavily overlapping and would overstate significance by orders of magnitude.
  - **Null test.** For each claimed effect (IC, hit rate, the gap between buckets), run a **block-bootstrap or permutation null**. Shuffle sentiment against forward returns in blocks of 24h or more to preserve autocorrelation, and report the effect's percentile within that null. Show "indistinguishable from noise" whenever p > 0.1.
  - **Metrics:** direction hit rate (Hold gives no hit rate), information coefficient (Spearman of `sentiment × new_info` against the z-scored forward return) **for each horizon**, calibration (high vs low `sentiment_conf`), and breakdowns by `event`, `source`, `asset`, and regime.
  - **Stop calibration from MAE.** Look at the distribution of MAE among *winners*. If a large share of winners first went below −4%, the stop is cutting off good trades. That feeds 3.2.
- **1.4 Outputs:**
  - `jev-trader report`: a markdown summary in a weekly-digest style covering hit rate, expectancy, IC by horizon, gate values, and stop calibration. Every figure carries its n and its percentile within the null.
  - `data/backlog.jsonl`: **improvement backlog** entries. Each has a pattern, evidence (n, effect, null percentile), severity, and a suggested action, for example *"`listing` events: IC 0.01, n=212 clusters, p=0.44 → consider weight 0"*. These are suggestions only.
  - A dashboard **Scorecard** panel, labelled *paper · settled · n=…*.

### Phase 2: Replay, backtest, and checking for a real edge

- **2.1 Split `bot::tick`** into an I/O part (feeds, Jev, klines) and a pure function `decide(state, events, candles, regime, now, cfg) -> Decision`. Live trading and backtests share one decision path, and `now` is passed in instead of read from `Utc::now()`.
- **2.2 `jev-trader backtest --from --to --config alt.toml --run-id X`.** Replays cached `events.jsonl` plus historical klines, so it makes no new Jev calls. Output goes to `data/backtests/<run_id>/` and the run can be resumed. Point-in-time rules: an event becomes visible at `first_seen_at`; a decision at *t* fills at the **next candle's open** with fees and slippage; momentum uses only candles that have closed; cooldowns, halts, and decay run on simulated time.
- **2.3 Ablation baselines, run on every backtest.** Use the same window, costs, and risk rules:
  1. **Momentum-only**: same rules, news score treated as always above the entry threshold. **This is the declared primary baseline**, because it is the simple strategy the bot could turn out to be copying.
  2. **News-only**: no momentum confirmation.
  3. **BTC buy-and-hold**, plus an **equal-weight BTC/ETH/SOL basket**.
- **2.4 Residual-edge test.** Regress the strategy's hourly returns on the primary baseline, with BTC buy-and-hold as the alternate, using HAC/Newey-West errors. Report alpha with its HAC t-stat, R², and residual edge ratio (alpha ÷ residual vol; don't compute a Sharpe on the residuals), plus **rolling alpha stability**. Verdicts follow CTS: **RESIDUAL_EDGE / BASELINE_EXPLAINED / RESIDUAL_FRAGILE / INSUFFICIENT_EVIDENCE**. This answers the question the whole product depends on: **does Jev's news reading add anything beyond momentum?**
- **2.5 Change review gate.** `backtest --compare base.toml candidate.toml` produces a **PASS / REVISE / REJECT** verdict, modelled on edge-strategy-reviewer. The checks:
  - Out-of-sample: tune on the first 70% of the window, judge only on the last 30%.
  - Sample adequacy: at least 20 independent decision clusters in the out-of-sample part.
  - Overfitting: the number of parameters changed. More than 2 at once means REVISE, unless split into separate tests.
  - Regime dependency: does the result hold in both RISK_ON and RISK_OFF halves?
  - Costs: fees, slippage, and turnover are included, and turnover doesn't rise without a gain to match.
  - It isn't explained by the baseline (2.4).

  A REJECT is shown prominently. The operator can still apply the change, but the report records that it was applied against the verdict.
- **2.6 Label results honestly.** Results say *backtest · replay of cached events*, and state the survivorship limit: the event set contains only what the feeds happened to serve while the bot was running.

### Phase 3: Sizing and drawdown control

Each item ships with a default that reproduces today's behaviour (§3).

- **3.1 Volatility-scaled sizing.** Add `sizing = "fixed" | "vol"` (default `fixed`, which is today's behaviour). In `vol` mode: `w = min(max_asset_weight, conviction × risk_per_trade / stop_dist)`, where `stop_dist = stop_atr_mult × ATR14% on 1h` and `conviction = news_score` clamped to 0..1. The effect is that SOL gets a smaller weight than BTC for the same conviction.
- **3.2 ATR-based stop.** Add `stop = "fixed_pct" | "atr"` (default `fixed_pct` at 4%). Set the ATR multiple from Phase 1's MAE-of-winners data, not by guessing.
- **3.3 Portfolio heat cap.** `max_heat` is the sum over assets of weight × stop distance. Its default is today's implied maximum: 60% gross × 4% = **2.4%**. It's applied in `apply_caps` and reported as the binding constraint when it binds.
- **3.4 Stronger circuit breaker.** Add to `[risk]`: `weekly_loss_limit_pct` and `monthly_loss_limit_pct` (TA's equivalents of 5% and 8%; the defaults are off, then set by the operator), plus `losing_streak_n` and `streak_cooldown_hours` (e.g. 2 and 24). Required groundwork:
  - Add `realized_pnl` to `Fill` (sell fill vs average entry) so trades can be classified as winners or losers.
  - Add `week_start_equity` and `month_start_equity` to `State`, all in UTC.
  - States are `TRADING_ALLOWED`, `COOLDOWN`, `HALTED`, each with `active_until`. Halts block entries only, never forcing liquidation (flatten remains the operator's separate button).
  - **Fail closed on corrupt state** (0.5): duplicated or non-finite ledger data means HALTED, with a reason.
- **3.5 Posture line.** The exposure-coach idea: one line at the top of the dashboard combining the breaker, Jev health (5.2), and regime (Phase 4) into a **single posture**: `ENTRIES ALLOWED · ceiling 60%`, `EXITS ONLY · weekly halt until Mon 00:00 UTC`, or `REVIEW · Jev degraded`. This serves principle 1 directly: anything that needs attention is clear at a glance.

### Phase 4: Crypto market regime

Adapted from crypto-regime-analyzer. Deployed in two steps: **display and record first, gate later.**

- **4.1 Compute hourly and cache in `data/regime.json`, without calling it every tick.** All sources are keyless:
  - BTC trend structure: Binance daily klines, close vs the 50/200-day SMAs and the slope of the 200-day.
  - Alt breadth: the top 20 USDT pairs by volume from Binance `ticker/24hr`, excluding stablecoins and wrapped assets, measured as the share above their 200-day SMA. Daily klines, fetched once a day and cached.
  - BTC dominance: CoinGecko `/global`, one stored observation per day. Marked unavailable until 31 days of history exist.
  - Funding: Binance futures `premiumIndex`, averaged across the majors, contrarian at the extremes.
  - Drawdown from the 1-year high, plus the realized-vol percentile.
  - Thrust: the share of the universe with a positive 30-day return.
  - Missing components have their weight redistributed. **If fewer than 4 of 6 are present, the regime is `UNKNOWN`**, which fails closed if it's used as a gate.
- **4.2 Record and display.** Record `regime_score` and `zone` on every `TickRecord` and settlement. Show the regime on the dashboard. **It has no effect on trading at this step.**
- **4.3 Gate only on evidence.** Once Phase 1 shows a REGIME_MISMATCH pattern with n ≥ 20 clusters that beats the null, consider `regime_mode = "off" | "ceiling" | "block"`. In `ceiling` mode, `max_gross` scales by zone. In `block` mode, there are no new entries in RISK_OFF. Either goes through the 2.5 review gate first.

### Phase 5: Signal quality

Each of these is a hypothesis to be recorded in `hypotheses.toml` and validated by Phases 1 and 2.

- **5.1 Group duplicate stories.** Run a title-token Jaccard pre-filter, then ask Jev a `choice` question on the borderline cases: "is this the same event as one of these recent headlines?". Each group gets one influence weight, and additional outlets add a little confidence rather than adding to the sum. Store a `cluster_id` and show "3 outlets" on the dashboard. This uses the same clustering as 1.3's independence guard.
- **5.2 Adversarial check before entry** (TA's bull/bear idea, made cheap). Make one extra Jev call **only when an entry is about to happen**, using the top driver headlines as state:
  - `noul`: already priced in or widely known
  - `noul`: contradictory or single unverified source
  - `score`: strength of the strongest bear case over 24h

  Above a configured threshold, block the entry or halve its size. It's recorded as a gate (0.6), and its value is measured by Phase 1's counterfactuals.
- **5.3 Track how price reacted to the news.** From news-reaction-failure-analyzer. For each asset, compute a rolling **drift z-score** of how price responded to positive news clusters (vol-normalised, with overlapping windows merged). The hypothesis: *if strongly positive clusters repeatedly fail to rally, with drift significantly below zero across 3 or more clusters, then new positive news has a lower expected value, so don't enter*. This is a sharper version of the momentum confirmation the bot already uses. Use their thresholds as a starting point: a minimum of 3 clusters, and a z threshold tuned so that a Monte Carlo test on noise gives a false-confirmation rate below 10%.
- **5.4 Stance counts.** Show `+4 / −1 / ~2` next to each score, and optionally require at least 2 independent clusters before entering.
- **5.5 Learned per-source and per-event weights.** Add `[news.event_weight]` and `[news.source_weight]` to config. Values come from the Phase 1 backlog, are applied by the operator, and must pass the 2.5 gate.
- **5.6 Align the horizons.** If 1.3 shows the IC peaks at 4h, move `half_life_hours` and the momentum window toward 4h, via the 2.5 gate.

### Phase 6: Reliability and cost

- **6.1 Jev retry budget.** Track attempts, exponential backoff, and a maximum of 5 attempts per headline, then `spiked: jev_failed`.
- **6.2 Jev circuit breaker.** If more than 50% of calls fail over the last K ticks, block entries (posture `REVIEW · Jev degraded`). Exits still run.
- **6.3 Crash consistency.** Dedupe `events.jsonl` by id on load and replay, or save state immediately after the Jev stage.
- **6.4 Token telemetry.** `Response.usage` is parsed but never used. Show it in the Jev stage and in a daily rollup.
- **6.5 Speed up `seen` lookups.** Add a `HashSet` index alongside the 5000-entry `VecDeque` that `is_seen` currently scans for every item.

### Phase 7: New inputs (after Phases 1, 2 and 4)

Each starts as a recorded column, scored by Phase 1 before it's allowed to affect `target`.

- **Funding and open interest crowding** (Binance futures, public). This partly arrives with Phase 4. It's the crypto version of CTS's COT crowding detector.
- **Macro calendar.** Tag ticks within ±1h of FOMC, CPI, or NFP releases. Hypothesis: news-driven entries just before these releases do worse. If confirmed, add a "no new entries" window. This is CTS's economic-calendar idea.
- **Polymarket odds** on crypto-relevant events. TA uses Polymarket as a data vendor.
- **Jev-screened social posts** (Reddit), kept separate from the news score until Phase 1 shows they add anything. TA does this already, using our own Jev provider.

---

## 4.6 Hypothesis register

Add a `hypotheses.toml` file, adapted from CTS's trade-hypothesis-ideator. Every strategy change in Phases 3–7 gets an entry **before it's tested**:

```toml
[[hypothesis]]
id = "H-005"
statement = "Requiring ≥2 independent clusters before entry raises 4h hit rate"
change = "news.min_clusters = 2"
metric = "4h direction hit rate, out-of-sample, clustered"
baseline = "current config"
pass_if = "hit rate +5pp vs baseline, null percentile > 90, n >= 20 clusters"
invalidate_if = "fewer trades with no hit-rate gain, or REJECT from the 2.5 gate"
status = "proposed"   # proposed | testing | accepted | rejected
result = ""
```

`jev-trader report` lists open hypotheses and their current evidence. This blocks the most common way a bot like this fools its author: adjusting thresholds until a backtest looks good.

---

## 5. Dashboard changes these phases need

Kept within PRODUCT.md (monitoring and safety controls only):

- **Posture line** at the top (3.5): entries allowed, exits only, or review, plus the reason and when it ends.
- **REVIEW** state distinct from Hold (0.1).
- A **full gate trace and binding constraint** per decision (0.6). The decision chain becomes headline → Jev → cluster → signal → regime → challenge → gates → caps (binding) → fill.
- **Scorecard**: hit rate, IC by horizon with null percentile, gate values (missed vs correctly blocked), stop calibration, and the residual-edge verdict, all showing n and the *paper* or *backtest* label.
- **Regime** score, zone, and components (4.2).
- "N outlets" and stance counts on headlines (5.1, 5.4). Jev circuit-breaker and spiked-headline indicators (6.1, 6.2).

---

## 6. Not adopted, and why

- **A full multi-agent LLM debate, or LLM risk and portfolio managers (TA).** Too slow and costly for a 60s loop, not deterministic, and harder to explain. 5.2 keeps the useful part.
- **Free-text "reflections" fed back into prompts (TA).** Jev takes typed questions. Statistical settlement plus the backlog is the auditable equivalent.
- **A 0–100 LLM conviction score combining many inputs (CTS stanley-druckenmiller).** It's opaque. Our score must stay traceable to headlines and gates.
- **Chart-image reading, equity screeners (CANSLIM, VCP, dividends), options, MT5, SEC/13F (CTS).** They don't apply to a spot crypto news bot.
- **Kelly sizing (CTS position-sizer)**, until Phase 1 has hundreds of settled clusters. Even then, half-Kelly at most, used as a cap rather than a target.

---

## 7. Order and size

| Step | Size | Depends on |
|---|---|---|
| Phase 0 (0.1–0.7) | S–M | none |
| 6.1, 6.2, 6.5 | S | none |
| 2.1 split `tick` → pure `decide` | M | Phase 0 |
| 3.4 groundwork (`realized_pnl`, week/month equity) | S | none |
| **Phase 1 settlement + postmortem + report** | M–L | 0.2, 0.6 |
| 4.1–4.2 regime: record and display only | M | none (needed for regime breakdowns) |
| 3.5 posture line | S | 3.4, 6.2 |
| Phase 2 backtest + ablations + residual edge + 2.5 gate | L | 2.1, 0.2, 0.3 |
| `hypotheses.toml` + report integration | S | Phase 1 |
| 3.1–3.3 vol sizing / ATR stop / heat, behind flags | M | Phase 1 (MAE), Phase 2 (to validate) |
| 5.1 clustering, 5.4 stance counts | S–M | Phase 1 |
| 5.2 challenge gate, 5.3 reaction drift | M | Phases 1 and 2 |
| 4.3 regime gate, 5.5 / 5.6 tuning | S | weeks of settled data + 2.5 PASS |
| Phase 7 inputs | M each | Phases 1, 2 and 4 |

**Milestone 1 (fail-closed and measured):** Phase 0, 6.1/6.2, the 3.4 groundwork, Phase 1, and regime recording. After it, the bot never acts on a made-up input. Every decision, headline, and blocked trade gets scored with noise-aware statistics. The posture line answers "is it OK right now?" at a glance. That's the right moment to start the first real Jev-keyed paper run.

**Milestone 2 (is there an edge?):** Phase 2. Its first real output is the residual-edge verdict of the current strategy against momentum-only. If the verdict is BASELINE_EXPLAINED, that's the most important thing to learn before tuning anything.
