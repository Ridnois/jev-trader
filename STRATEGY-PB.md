# P/B range strategy (proposed third engine)

Source: the YouTube video "Everyone Laughed at His Strategy Until He Dominated the World Cup 12 Times" (`media/`). It summarises Patrick Nill's public interviews. Nill is a futures trader with 12 top-five finishes in the Robbins World Cup.

> **Status: proposal, untested.** The video is a promotional summary that includes an indicator ad. Competition returns such as "~300%" come from a leveraged futures account and say nothing about this bot. Nill does **not** share his exact rules for drawing and resizing zones, so every numeric threshold below is our own guess at them. Each rule that matters is a hypothesis in `hypotheses.toml` (H-P01 to H-P06), and none goes live before its test passes.

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
| No range trades on a P completed in the evening (8h unwatched) | Not needed: the bot watches 24/7. **Do keep** "no resting orders while the process is down" (no exchange-side stops, see README). |
| Stop below structure / the zone / a profile level; otherwise derive it from the target with RR ≥ 3 | §4.1, global `min_rr = 3` |
| Stop decided first, size follows | §4.2 |
| **Never move the stop to give the trade more room** | Hard invariant: stop only ratchets toward profit (§4.3) |
| Trail the stop under each new P as the trade runs | §4.3 |
| Targets fair value → fair value. Thirds: nearest zone edge, decide the next on the spot, last third at the move's origin | §4.4 |
| At all-time highs (no prior fair value): project the impulse 1:1 | §4.4 fallback |
| Small range after a big move = breakout (RR 8+). Big range = range trade, never a breakout. | Setup selection by `box_height / impulse` (§3.3) |
| 3–4 losses in a row in a day → close everything, restart tomorrow | `max_daily_losses = 3` (§5) |
| One or two setups a day, same rules every day | Expect ~1–2 candidates/day/coin. Config is changed only through `backtest --compare`. |

## 3. Detection (15m closed candles, per coin)

ATR = ATR14 on 15m. All tolerances scale with ATR, so one config serves BTC and SOL.

### 3.1 Impulse
A run of ≤ `impulse_max_bars = 8` candles whose net close-to-close move is ≥ `impulse_atr = 3.0` × ATR, ending at the run's extreme. Up = P candidate, down = B candidate. Record `origin` (the run's start price) and `impulse = |end − origin|`.

### 3.2 Box
The box starts at the first candle after the impulse. While it is **unvalidated**, `high`/`low` are the max high / min low of its candles. It may grow as long as `height ≤ box_max_frac = 0.6 × impulse`. It is discarded if a later candle would push it past that limit, or if a candle **closes** beyond the impulse origin.

A **touch** is a candle whose wick comes within `touch_tol = 0.15 × ATR` of an edge, at least `touch_gap = 3` bars after the previous touch on that edge.

**Valid** when: touches ≥ `min_touches = 3` (with at least one on each edge), age ≥ `min_range_bars = 48`, and height ≥ `cost_mult × round_trip_cost` (otherwise fees eat the range). Once valid, the box is **frozen**: edges no longer move. Freezing stops the resizing problem the video admits is the hard part, at the price of some ranges being drawn slightly wrong.

A box **expires** after `box_max_bars = 192` (2 days) without a setup, or after a confirmed breakout or failed-break trade completes.

### 3.3 Setup selection by box size
- `height / impulse ≤ 0.35` → small box: Setups B and C allowed, A not (too little room).
- `height / impulse ≥ 0.35` → big box: Setups A and C allowed, B not (stop too far).
C is allowed in both, because its stop is the sweep wick, not the box.

### 3.4 Context (record-only at first)
- **4h bias**: close vs 4h SMA20 and the SMA's slope. Up / down / flat.
- **Weekly value area**: build a volume profile from the last 7 days of 15m candles (each candle's volume spread evenly over its high–low in `profile_bins = 100` bins). POC = top bin. Value area = 70% of volume around the POC → VAH/VAL. A box edge within `0.25 × ATR` of VAH, VAL or POC counts as **confluence**.
- **Stacked range**: a box edge within `0.25 × ATR` of an earlier expired box's edge counts as confluence ("two ranges at the same level work better").

Both are logged on every candidate. They gate or resize trades only after H-P03 passes.

## 4. Setups and trade management

`buf = 0.1 × ATR` (stop buffer beyond the structure).

| | Trigger (on candle close) | Entry | Stop | Target |
|---|---|---|---|---|
| **A: range fade** (big box) | Price inside a valid box, 4h bias not against the side | Resting ALO limit at the edge (buy low / sell high) | Box edge ∓ `buf`, or a nearer profile level beyond the edge | Opposite edge, 100% out, no partials |
| **B: confirmed breakout** (small box) | Candle 1 closes outside. Within ≤ 4 bars a candle retests the edge (wick within `touch_tol`) **without closing back inside**. Then a candle closes beyond candle 1's extreme. | Limit at candle 1's extreme (retest fill) or market on the next open, whichever is first, inside `max_slippage_bps` | Beyond the retest low/high ∓ `buf` | §4.4 ladder |
| **C: failed breakout** (favourite) | A candle trades beyond an edge (wick or close). Within ≤ 3 bars a candle **closes back inside**. | Limit at the broken edge | Beyond the sweep extreme ∓ `buf` (small by construction) | §4.4 ladder, **not** capped at the far edge |

- C long: full size. C short (fake breakout *above*, close back inside): `c_short_size = 0.5`, pending H-P04.
- Every setup must clear `min_rr = 3.0` measured to the **first** target, and `stop_distance ≥ stop_cost_mult (2) × round_trip_cost`. A stop tighter than fees is noise, not edge.
- Entries are maker (ALO), as in the scalper. An unfilled entry is cancelled when the trigger's box expires or price reaches the first target first (a miss, logged with its markout).

### 4.1 Stop placement order
1. Structure as in the table.
2. Nearer weekly-profile level beyond it, if one exists.
3. Nothing usable → derive the stop from the target: `stop = entry ∓ (target − entry) / min_rr`.

### 4.2 Sizing (stop first)
`qty = risk_per_trade × equity / stop_distance`, then capped by `max_asset_weight × equity / price` and the account's 1× notional limit. The binding constraint is logged, as in the news bot. Start at `risk_per_trade = 0.0025`, half the news bot's default, until H-P01 has data.

### 4.3 Stop invariant and trailing
- `new_stop` is accepted only if it is **closer to profit** than the current stop: `assert!(long ⇒ new ≥ old; short ⇒ new ≤ old)`. Any code path that would widen it is a bug and must panic in tests and refuse in production.
- After the first partial: move the stop to break-even plus fees.
- While in a trade, if a new valid P (long) or B (short) forms in the trade's direction, move the stop to that box's far edge ∓ `buf`.

### 4.4 Target ladder (B and C)
Positions are sized in thirds.
1. **T1**: nearest fair-value edge in the trade's direction (for C, the box's far edge). Close 1/3 and move the stop to break-even.
2. **T2**: the next expired box edge, or the next VAH/VAL/POC. Close 1/3.
3. **T3**: the impulse origin (for counter-trend C/B trades back through the move), or, with no prior fair value (new high/low), `entry ± impulse` (1:1 projection). Close the rest.

If a ladder level doesn't exist, its third moves to the next one. A time stop of `max_hold_bars = 192` closes whatever is left.

## 5. Daily discipline
- `max_daily_losses = 3`: after the 3rd losing round trip in a UTC day, cancel resting entries and flatten open P/B positions. No entries until 00:00 UTC.
- Existing breakers still apply: daily loss limit, REVIEW on stale/non-finite data, reconciliation holds.
- Jev, if enabled, works as the scalper's does. It can veto an entry or halve its size on adverse news or event risk. It never creates a trade.

## 6. Fitting it into jev-trader

- **New subcommand `pb`**, a separate engine with its own ledger (`data-pb/`), the same pattern as `scalp`. It needs shorts and multi-leg exits, which the long-only news-bot `State` lacks.
- **Reuse:** `market::klines` / `split_closed` (15m and 4h, closed candles only, stale check). Hyperliquid exchange + signing, ALO entries, reduce-only exits, cloid lookup, reconciliation from the live layer. Fee/funding accounting from paper. Settlement, block-permutation report and `backtest --compare` for verdicts.
- **New code:** detector (§3, a pure function from candles to boxes and candidates, easy to unit-test on hand-built candle series), trade manager (§4), daily-loss counter (§5).
- **Logs** (`data-pb/`): `boxes.jsonl` (every box: impulse, touches, validated/expired, confluence flags), `candidates.jsonl` (every trigger with gates passed/failed, taken or not), `markouts.jsonl` (each candidate's R at T1/T2/T3/stop, whether taken or not), `trades.jsonl`.
- **Universe:** start with BTC, ETH, SOL, HYPE.
- **Loop:** wake 5s after each 15m close, plus a fast tick (`eval_ms`) only while an order rests or a position is open, for fills and stops.

### Backtest reach
Hyperliquid serves the latest 5000 candles per interval: about **52 days of 15m** and about 833 days of 4h. Plan:
1. Backtest the 52 days now (expect roughly 50–150 C candidates across 4 coins: a first read, not proof).
2. Start the engine on paper the same day. It caches every 15m candle so the history grows.
3. Decide H-P01 only when it reaches its n, from backtest plus forward paper together.

## 7. Parameters (initial guesses, every one a hypothesis knob)

```toml
[pb]
interval = "15m"
bias_interval = "4h"
impulse_max_bars = 8
impulse_atr = 3.0
box_max_frac = 0.6
small_box_frac = 0.35
touch_tol_atr = 0.15
touch_gap = 3
min_touches = 3
min_range_bars = 48
box_max_bars = 192
stop_buf_atr = 0.1
min_rr = 3.0
stop_cost_mult = 2.0
cost_mult = 3.0
c_short_size = 0.5
confluence_mode = "record"   # "record" | "boost" | "require" (after H-P03)
max_hold_bars = 192
max_daily_losses = 3
risk_per_trade = 0.0025
max_asset_weight = 0.20
```
