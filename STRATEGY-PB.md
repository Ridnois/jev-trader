# P/B range strategy (proposed third engine)

Source: the YouTube video "Everyone Laughed at His Strategy Until He Dominated the World Cup 12 Times" (`media/`). It summarises Patrick Nill's public interviews. Nill is a futures trader with 12 top-five finishes in the Robbins World Cup.

> **Status: proposal, untested.** The video is a promotional summary that includes an indicator ad. Competition returns such as "~300%" come from a leveraged futures account and say nothing about this bot. Nill does **not** share his exact rules for drawing and resizing zones, so every numeric threshold below is our own guess at them. Each rule that matters is a hypothesis in `hypotheses.toml` (H-P01 to H-P06), and none goes live before its test passes.
>
> Revised 2026-11-07: removed contradictions (Setup A's stop, Setup B's entry, the T3 target, Setup C's trigger, the expected rate) and added the missing definitions (box lifecycle, touch counting, costs, exposure caps, what a loss is, crash safety, how results are measured).

## 1. The idea in one paragraph

Markets range about 70% of the time and trend about 30%. A strong push followed by a sideways range is a **P** (push up) or a **B** (push down). Once a range is mature (3 touches, at least half a day on 15m), trade it in one of three ways: fade its edges, buy a *confirmed* breakout, or (the favourite) buy a **failed breakdown** that closes back inside. Choose the stop from structure first, then size from the stop. Never widen a stop. Take targets at the next area of fair value. After 3 losses in one day, stop trading until the next day.

## 2. Transcript → rules

| What the video says | Bot rule |
|---|---|
| 4h for the big picture, 15m for the decision, 3–5m only to time the entry | Detection runs on **closed 15m** candles. Bias comes from 4h. Entries are limit orders, so no lower timeframe is needed. |
| P = strong move up then a range; B = strong move down then a range | §3.1 impulse + §3.2 box |
| Valid only after 3 touches; on 15m this takes ≥ half a day, often a full day | `min_touches = 3`, `min_range_bars = 48` (12h) |
| Waiting longer means more misses, and that is accepted | No early entries. A missed move is logged as a miss, not chased. |
| Weekly volume profile: a P/B sitting on VAH/VAL is stronger | §3.4 confluence, record-only until H-P03 passes |
| Range trade: limit at one edge, target the other, no partials | Setup A |
| Breakout: close outside, retest of the edge, next candle closes beyond the breakout candle | Setup B |
| Fake breakout below, close back inside → long, tiny stop, hold past the top. The mirror short is traded much less. | Setup C (long full size, short half size or off) |
| No range trades on a P completed in the evening (8h unwatched) | Not needed: the bot watches 24/7. The real risk is the process dying with orders out: §5.1 requires no resting entries and an exchange-side stop on every position while it is down. |
| Stop below structure / the zone / a profile level; otherwise derive it from the target with RR ≥ 3 | §4.1: structure first, then a cost floor; no trade unless RR ≥ 3 to the first target |
| Stop decided first, size follows | §4.2 |
| **Never move the stop to give the trade more room** | Hard invariant: stop only ratchets toward profit (§4.3) |
| Trail the stop under each new P as the trade runs | §4.3 |
| Targets fair value → fair value. Thirds: nearest zone edge, decide the next on the spot, last third at the move's origin | §4.4 |
| At all-time highs (no prior fair value): project the impulse 1:1 | §4.4, with-trend T3 |
| Small range after a big move = breakout (RR 8+). Big range = range trade, never a breakout. | Box size recorded on every candidate (§3.3); enforced only after H-P06 |
| 3–4 losses in a row in a day → close everything, restart tomorrow | `max_daily_losses = 3` (§5) |
| One or two setups a day, same rules every day | That is his rate across his markets. Ours per coin is unknown: the first record-mode backtest measures it (§6). Config changes go through `pb backtest --compare`. |

## 3. Detection (15m closed candles, per coin)

ATR = ATR14 on 15m. All tolerances scale with ATR, so one config serves BTC and SOL. `band = touch_tol_atr × ATR` (0.15 ATR) and `buf = stop_buf_atr × ATR` (0.1 ATR) are used throughout.

### 3.1 Impulse
Each closed bar t is tested as the end of an **up impulse**: a run of bars s..t, 1 to `impulse_max_bars = 8` bars long, with `close[t] − close[s−1] ≥ impulse_atr (3.0) × ATR[t]` and `high[t]` the highest high of the run. Of the qualifying runs ending at t, take the one with the largest close-to-close move (ties: the shorter run). Down impulses mirror this. Up = P, down = B.

Consecutive qualifying bars extend the same impulse. It ends at the last qualifying bar, and the first bar that doesn't qualify is the box's first bar. Recorded: `origin` (the run's lowest low for a P, highest high for a B), `end` (the run's extreme), `impulse = |end − origin|`.

### 3.2 Box
**One active box per coin.** While it is **unvalidated**, its top and bottom are the max high and min low of its bars. It ends as follows:

| End | When |
|---|---|
| `too_wide` | a bar would make `height > box_max_frac (0.6) × impulse` |
| `origin_breached` | a bar **closes** beyond the impulse origin |
| `superseded` | a new impulse (either direction) completes while the box is unvalidated; that impulse starts the next box |
| `expired` | `box_max_bars` (192, 2 days) after its first bar with no entry filled |
| `broken` | frozen box only: a bar closes beyond an edge by more than one box height |
| `consumed` | an entry on it fills |

A setup that is armed (waiting for a retest, a confirmation, a reclaim or a fill) keeps its box alive until it resolves. While a frozen box is active, new impulses are ignored. After a box ends, the next completed impulse starts a new box, even on a coin with an open position. Those boxes serve trailing only (§4.3), because a coin holds one position at a time (§4.6). Valid boxes that end join the coin's history (the last `history_boxes = 20`), which supplies stacked-range confluence and ladder levels.

**Touches** (unvalidated box) are recounted from scratch after every bar against the current edges. A bar touches the top if its high ≥ top − band, so the bar that sets an edge always touches it. The bottom mirrors this. On each edge, touches count greedily from the earliest and must be at least `touch_gap = 3` bars apart.

**Valid** when touches ≥ `min_touches = 3` with at least one on each edge, age ≥ `min_range_bars = 48`, and height ≥ `cost_mult × round_trip_cost` (§4.5). Once valid, the box is **frozen**: edges no longer move. Freezing stops the resizing problem the video admits is the hard part, at the price of some ranges being drawn slightly wrong.

**Beyond a frozen edge:** a wick past an edge by at most `band` is still a touch (noise). Past that it is a **sweep**. Setup A's stop sits outside this band (§4), and Setup C triggers only on a sweep.

### 3.3 Box size (recorded, enforced after H-P06)
Every candidate logs `height / impulse`. With `box_gate = "record"` (the default), every setup may trigger in every box. With `box_gate = "enforce"`, allowed only after H-P06 passes:
- `height / impulse ≤ small_box_frac (0.35)` → small box: Setups B and C only.
- `height / impulse > 0.35` → big box: Setups A and C only.

C stays allowed in both, because its stop is the sweep, not the box. Until H-P06 decides, `min_rr` does the filtering: A in a small box and B with a far stop usually fail it.

### 3.4 Context
- **4h bias**: up when the 4h close is above its SMA20 and the SMA20 is higher than one 4h bar earlier; down when both are reversed; otherwise flat. Setup A long needs bias ≠ down (A short: ≠ up). B and C record it only.
- **Weekly value area**: build a volume profile from the last 7 days of 15m candles (each candle's volume spread evenly over its high–low in `profile_bins = 100` bins). POC = top bin. Value area = 70% of volume around the POC → VAH/VAL. A box edge within `0.25 × ATR` of VAH, VAL or POC counts as **confluence**.
- **Stacked range**: a box edge within `0.25 × ATR` of a history box's edge counts as confluence ("two ranges at the same level work better").

Profile levels and history-box edges serve as ladder targets (§4.4) from the start, because targeting fair value is the method itself. Confluence gates, resizes or tightens stops only after H-P03 passes; until then it is logged on every candidate.

## 4. Setups and trade management

All setups need a frozen (valid) box. The long side is shown; shorts mirror it.

| | Trigger (on bar close) | Entry (post-only ALO limit) | Entry is cancelled when | Structure stop | Target |
|---|---|---|---|---|---|
| **A: range fade** | The box freezes, or the opposite A side resolves (below). 4h bias not down. | Buy at the bottom edge | the box ends, or price touches the top edge first | bottom − band − buf | top edge, all out |
| **B: confirmed breakout** | Candle 1 closes above the top. Within `b_retest_bars` (4) bars, a bar's low comes within `band` of the top without closing back inside. Within `b_confirm_bars` (4) bars after that retest, a bar closes above candle 1's high. | Buy at candle 1's high, posted at the confirming close | `b_entry_bars` (4) bars pass, a bar closes back inside the box, or price reaches T1 first | retest bar's low − buf | §4.4 ladder |
| **C: failed breakdown** | A **sweep** bar: low < bottom − band. Within `c_reclaim_bars` (3) bars, counting the sweep bar itself, a bar closes back above the bottom. A same-bar reclaim counts. The sweep extreme is the lowest low from the sweep bar to the reclaim bar. | Buy at the bottom edge, posted at the reclaim close | `c_entry_bars` (8) bars pass, a bar closes below the sweep extreme, or price reaches T1 first | sweep extreme − buf | §4.4 ladder, **not** capped at the far edge |

- **Entries are maker only.** Each limit is below the market when posted (above it for shorts): A posts while price is inside the box, B after a close above candle 1's high, C after a close back above the bottom. So ALO always applies and there are no taker entries. A miss is logged with its markouts (§8.2).
- **A's two sides:** at the freeze, A arms the side whose edge is nearer the last close, if the bias allows it, otherwise the other side. When that side resolves (cancelled, or its round trip ends) and the box is still active, the opposite side arms. Each side arms at most once per box.
- **One entry rests per coin.** When setups conflict, priority is C, then B, then A. A higher-priority trigger replaces a resting lower-priority entry, which is logged as `replaced`. Between equal priorities, the first to post keeps the slot.
- A sweep that stops out a Setup A trade can arm Setup C on the same bar. That's by design: it is the video's favourite trade.
- C long: full size. C short (sweep above the top, close back inside): `c_short_size = 0.5`, pending H-P04.

### 4.1 Stop placement and the trade filter
1. **Structure** stop from the table.
2. **Cost floor:** stop distance = max(structure distance, `stop_cost_mult (2) × round_trip_cost`). Each candidate logs `stop_basis = structure | cost_floor`. The stop is fixed at placement and never widens later (§4.3).
3. **No trade** unless the distance to the first target is ≥ `min_rr (3.0)` × the stop distance.

Profile levels don't tighten stops until H-P03 passes. The tighter alternative is logged so H-P03 can test it.

### 4.2 Sizing (stop first)
`qty = risk_per_trade × equity / stop_distance`, then capped by `max_asset_weight × equity / price`, the account's 1× notional limit and the caps in §4.6. The binding cap is logged, as in the news bot. Start at `risk_per_trade = 0.0025`, half the news bot's default, until H-P01 has data.

If a third of the position would be under the exchange minimum (`live::MIN_ORDER_USD`, $10.50), the position takes a single exit at T1 instead of thirds.

### 4.3 Stop invariant and trailing
- `new_stop` is accepted only if it is **closer to profit** than the current stop: `assert!(long ⇒ new ≥ old; short ⇒ new ≤ old)`. Any code path that would widen it is a bug and must panic in tests and refuse in production.
- After T1 fills: move the stop to break-even after costs, `entry × (1 + round_trip_cost)` for a long.
- While the trade is open, if a new box on the same coin becomes valid in the trade's direction (a P for a long), propose `that box's bottom − band − buf`. It applies only under the ratchet rule.

### 4.4 Target ladder (B and C)
Positions are sized in thirds. Ladder levels are fair value in the trade's direction: history-box edges and the weekly VAH, VAL and POC.
1. **T1**: for C, the box's far edge; for B, the nearest level beyond entry. Close 1/3 and move the stop to break-even (§4.3).
2. **T2**: the next level beyond T1. Close 1/3.
3. **T3**: close the rest.
   - **Counter-trend** (a long on a B box, a short on a P box): the impulse origin, back through the move.
   - **With-trend** (a long on a P box, a short on a B box): `far edge ± impulse`, the 1:1 measured move. This covers new highs with no prior fair value.

Levels are processed in order. A level that is missing, or not beyond the last level kept, is dropped, and its third moves to the next one. The last level kept takes whatever is left. §4.1's RR is measured to the first level kept. Targets rest as reduce-only ALO limits (maker). Stops and the time stop exit with reduce-only IOC orders (taker). A time stop of `max_hold_bars = 192` from the fill closes whatever is left.

### 4.5 Costs
`round_trip_cost` (a fraction of the entry price) = maker fee in + taker fee out + slippage + half the spread + the funding the side would pay over `funding_hold_hours = 12` at the current rate (received funding is not credited). It is the scalper's `round_trip_cost_bps` with a longer hold. Pricing the exit as taker is conservative, since target exits are maker. Funding actually paid or received accrues hourly on the open size and is part of every result.

### 4.6 Exposure caps
- One position or resting entry per coin.
- `max_positions = 2` across coins.
- `max_gross = 0.40`: total notional ≤ 40% of equity.
- `max_heat = 0.0075`: Σ qty × stop distance ≤ 0.75% of equity, i.e. three trades at full risk.

Resting entries count as filled for every cap, so simultaneous fills can't breach one.

## 5. Daily discipline and safety
- A **round trip** runs from the entry fill (partial fills included) to the exit of its last third. Its result is the sum of all legs after fees and funding. A **loss** is a result below $0. A trade that banks T1 and then stops at break-even after costs is a small win.
- `max_daily_losses = 3`: after the 3rd loss in a UTC day, cancel resting entries and flatten open P/B positions. No entries until 00:00 UTC.
- Existing breakers still apply: daily loss limit, REVIEW on stale/non-finite data, reconciliation holds.
- Jev, if enabled, works as the scalper's does. It can veto an entry or halve its size on adverse news or event risk. It never creates a trade.

### 5.1 While the process is down (live only)
**Invariant:** no entry order rests, and every open position has an exchange-side stop. It is required before `--live`, not before paper.
- **Entries:** a dead-man switch. Hyperliquid's `scheduleCancel` action, refreshed every 30 s for 90 s ahead, cancels resting orders if the process stops refreshing it.
- **Positions:** after each fill, a reduce-only stop-market trigger order sits at the current stop. When the stop ratchets, the new trigger is placed before the old one is cancelled, so there is never a moment without one. The bot's own stop logic stays primary; the trigger is the backstop.
- **Neither exists yet.** `src/hyperliquid/exchange.rs` has no trigger or schedule-cancel call; `sign.rs` only has a trigger-order test vector.
- **What the docs settle (checked 2026-11-07):** the cancel time must be at least 5 s ahead. It may fire at most 10 times per day, with the count reset at 00:00 UTC; refreshing doesn't count, only firing does. It also fires if the bot is alive but can't reach the exchange for longer than the window, so after reconnecting the bot re-places stops before anything else.
- **Open question:** the docs say it cancels "all open orders" without saying whether trigger orders are included. If they are, it deletes the backstop stop exactly when the bot is down, and the two halves of the invariant conflict. A possible way out: an entry sent with `grouping = "normalTpsl"` appears to carry its own stop, live from the moment it fills. If so, a resting entry that fills while the bot is down is already protected, and the dead-man switch isn't needed for safety. The docs don't describe `grouping`, so both points need a testnet run before the live layer is built (`NEXT-STEPS.md`).

## 6. Fitting it into jev-trader

- **New subcommand `pb`**, a separate engine with its own ledger (`data-pb/`), the same pattern as `scalp`. It needs shorts and multi-leg exits, which the long-only news-bot `State` lacks.
- **Reuse (exists today):** `market::klines`, `klines_range`, `split_closed` and `atr_pct`. From the live layer: maker posts, cancels, reduce-only fills, cloids, booking from `userFills`, reconciliation. Shorts and funding accrual from the scalper's ledger. Spearman and block-permutation nulls from `stats`. The PASS / REVISE / REJECT verdict format of `backtest --compare`.
- **Does not carry over:** `backtest`, `backtest --compare`, `settle` and `report` replay the news bot (cached Jev events against a momentum baseline). P/B needs its own versions.
- **New code:** the detector (§3, a pure function from candles to boxes and candidates, unit-tested on hand-built candle series), the trade manager (§4), costs and caps (§4.5–4.6), the daily-loss counter (§5), a 15m candle cache, `pb backtest` and `pb backtest --compare` (the same detector and manager run bar by bar under §8.3's fill rules), `pb report` (the `scalp-report` pattern: candidates, R, verdicts, backlog), and a block bootstrap in `stats.rs`. For live only, §5.1.
- **Logs** (`data-pb/`): `boxes.jsonl` (every box with its impulse, touches, end reason and confluence flags), `candidates.jsonl` (every trigger with its gates, stop basis, box size and confluence, taken or not), `markouts.jsonl` (each candidate's three R results, §8.2), `trades.jsonl`, `candles/<coin>_15m.jsonl`.
- **Universe:** start with BTC, ETH, SOL, HYPE.
- **Loop:** wake 5s after each 15m close, plus a fast tick (`eval_ms`) only while an order rests or a position is open, for fills and stops.

### Backtest reach and sample size
Hyperliquid serves the latest 5000 candles per interval: about **52 days of 15m** and about 833 days of 4h. Plan:
1. Run the detector alone in record mode on the 52 days: boxes, and candidates per setup per coin per day. No trade manager yet. This measures the rate instead of guessing it.
2. Project the time to H-P01's sample (60 filled C-long candidates across ≥ 30 days). If it is beyond about 6 months, revisit the thresholds through a new hypothesis before building the rest.
3. Start the engine on paper. The candle cache, appended at every close, lets history grow past 52 days.
4. `pb backtest --compare` judges only the last 30% of the window, about 16 days today. Compare verdicts wait until the cache holds at least 90 days (27 days out of sample).

## 7. Parameters (initial guesses, every one a hypothesis knob)

```toml
[pb]
interval = "15m"
bias_interval = "4h"
impulse_max_bars = 8
impulse_atr = 3.0
box_max_frac = 0.6
small_box_frac = 0.35
box_gate = "record"          # "record" | "enforce" (after H-P06)
touch_tol_atr = 0.15         # band: noise past an edge; beyond it is a sweep
touch_gap = 3
min_touches = 3
min_range_bars = 48
box_max_bars = 192
history_boxes = 20
stop_buf_atr = 0.1
min_rr = 3.0
stop_cost_mult = 2.0
cost_mult = 3.0
funding_hold_hours = 12
b_retest_bars = 4
b_confirm_bars = 4
b_entry_bars = 4
c_reclaim_bars = 3
c_entry_bars = 8
c_short_size = 0.5
confluence_mode = "record"   # "record" | "boost" | "require" (after H-P03)
max_hold_bars = 192
max_daily_losses = 3
risk_per_trade = 0.0025
max_asset_weight = 0.20
max_positions = 2
max_gross = 0.40
max_heat = 0.0075
```

## 8. Measuring it

### 8.1 R
1R = |entry − initial stop| × qty, in dollars, fixed at the fill. A result in R = the round trip's result (fees and funding included) ÷ 1R.

### 8.2 Three results per candidate
Every candidate is simulated through the full management (§4), whether or not the risk layer let it trade:
- **Filled R**: only candidates whose limit traded through within the entry window. H-P01's "mean R per trade" is this one.
- **As-run R**: every candidate, with misses counted as 0.
- **Taker R** (B and C): entry at the trigger bar's close, paying the taker fee plus slippage, with the same stop and targets.

The scalper's maker fills were the ones the market ran over: filled −4.6 vs missed +4.8 bps at 600s. Reporting all three shows whether waiting at the edge selects the losers.

### 8.3 Fill rules (backtest and markouts)
- A limit entry or target fills only if a bar trades **through** it (low < buy limit, high > sell limit), at the limit price and the maker fee.
- A stop fills at the stop price less slippage, at the taker fee.
- If one bar spans both the stop and a target, the stop fills first.
- Funding accrues for every hour a position is open.

Paper trading fills on trades printing through the price, as the scalper does.

### 8.4 Random-entry baseline (H-P01)
Each C-long candidate gets 20 random entries at closed bars of the same coin within ±2 days. Bars within 8 bars of any P/B candidate are skipped. Each draw fills at that bar's close at the maker fee, with the same stop and target distances in ATR units and the same management. "Beats the baseline" means the mean of (candidate R − mean R of its draws) is above 0 with a 90% block-bootstrap interval above 0. Draws use a fixed seed, so reports reproduce.

### 8.5 Intervals
Results are clustered by UTC day. Block bootstrap: resample whole UTC days with replacement, 2000 resamples, percentile interval. Effects between groups (H-P03) use block-permutation nulls, as `report` does.
