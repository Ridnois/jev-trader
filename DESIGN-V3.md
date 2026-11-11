# v3: a lab that can't fool itself, and a trader that only runs what passes it

2026-11-11. Branch `v3`. Everything here is added beside v1 (`run`, `scalp`, `dashboard` still work).

## 1. Why

The project kept finding rules that looked good in a backtest and then lost on paper. A review of how H-R01 (the 15m mean-reversion rule) was chosen found the reasons:

| Problem | What it did |
|---|---|
| The search was never paid for | H-R01 was the best of 380 rules. Its trade-level t-statistic was 2.44. The best of 380 rules with no edge is expected to score about 2.8. |
| Trades were counted as independent | 422 trades on 8 correlated coins with overlapping 6-hour holds are a few dozen market episodes, not 422 observations. |
| 52 days, one falling market | Hyperliquid serves only its latest 5000 candles. The 8 coins were picked by a volatility scan of the same weeks. |
| Flattering costs | No funding, a fixed spread, stops filling exactly at the stop. |
| Two code paths | The backtest re-implemented the live rules, and paper trading had no kill rule fixed in advance. |
| Jev only read headlines | Its entry reviews had been uninformative. |

v3 changes the process, not the rule: a strategy has to survive a gate that accounts for how it was found, and the trader runs the same code the gate simulated.

## 2. The pipeline

```
lab fetch      candles + funding → local archive (Hyperliquid, and Binance as a research proxy)
lab search     a family's declared grid on the design window → leaderboard, every spec logged
lab validate   search + the gate → PROMOTE_TO_PAPER / REVISE / REJECT / INSUFFICIENT_EVIDENCE
lab promote    hand the spec and its prediction card to the trader
trade run      paper: same plan and rules as the simulator, live Hyperliquid book
trade monitor  parity with the simulator, sequential test against the card, drawdown envelope
```

Jev sits in two places: `lab jev replay | score` (a typed market read replayed over history) and the catalyst recorder inside `trade run` (headlines, forward only).

## 3. The lab (`src/lab/`)

**Data** (`data.rs`). Append-only CSV under `lab-data/`. Hyperliquid is the trading venue. Binance USDT-M futures give about 2.8 years of 15m bars and funding prints for 20 coins. Binance is used for prices only: costs are always Hyperliquid's. The two venues' 15m returns correlate 0.995 to 0.998 over the shared window.

**Universe** (`panel.rs`). A fixed list of 20 perps listed on both venues. A coin is tradable on a day only if its median daily dollar volume over the 30 days *before* was at least $50M. The list itself was drawn up today, so it favours coins that survived; the volume filter removes only part of that bias.

**Strategies as data** (`strategy.rs`, `families/*.toml`). A family is a rule plus the grid the search may try. Nothing outside the grid is tested, and both directions of an idea go in the grid when both were considered. Every family is checked for look-ahead: its signals up to a bar must not change when the future is cut off.

**One simulator** (`sim.rs`, `rules.rs`). A signal at a bar's close fills at the next open. Order within a bar: all exits, then entries, then intrabar stops (a stop fills at its price, or at the open if price gapped through, plus extra stop slippage). Costs per side: 4.5 bps fee, half a 2 bps spread, 1 bps slippage. Funding prints are charged while a position is open. Size is volatility-targeted: a one-sigma day in the coin moves the account 0.4%, capped at 15% per position and 100% gross. Results are per UTC day on a fixed capital base, because days, not trades, are the independent unit.

**The trial ledger** (`search.rs`). Every spec ever simulated on a data set is appended to `lab-data/trials.jsonl`. The gate reads the trial count from there.

**The gate** (`gate.rs`, thresholds in `lab.toml`, fixed before any real data was judged):

| Check | Needs | Fatal when |
|---|---|---|
| Evidence | ≥ 250 design days, ≥ 100 trades | (gives INSUFFICIENT_EVIDENCE) |
| Walk-forward (train 180 d, skip 5, test 60) | out-of-sample mean > 0 with the 95% bootstrap interval above 0 | mean ≤ 0 |
| Reality check over the whole family | p < 0.05 | p > 0.20 |
| Deflated Sharpe, trials from the ledger | ≥ 0.95 | < 0.50 |
| Probability of backtest overfitting (10 blocks) | ≤ 0.30 | > 0.50 |
| Plateau | grid neighbours all positive, median ≥ half the pick | |
| Costs ×2, and entering one bar late | mean still > 0 | |
| Breadth | profitable without any single coin, ≥ 60% of quarters positive, positive without the best 1% of days | |
| Timing baseline | beats 95% of copies with each coin's signals shifted in time | |
| Market exposure | alpha against the coin basket > 0 (reported with beta) | |
| **Lockbox**: the newest 20% of days | mean > 0, one-sided p < 0.20 on the first opening (0.10 on the second…), not below what the design window predicts | mean ≤ 0 |
| Trading venue | positive on Hyperliquid's own candles and funding, daily returns correlate ≥ 0.7 | |

The lockbox is opened only for a candidate that passed everything before it, each opening is logged, and `lab run` and the Jev replays never show lockbox days.

One deliberate deviation from the textbook: the deflated Sharpe takes the no-edge variance of a Sharpe ratio (1/T) instead of the variance across the trials. The cross-trial variance also counts real differences between rules (a 3-hour hold pays four times the costs of a 24-hour hold), and with it planted edges with Sharpe 3.5 never passed.

**Calibration** (`lab calibrate`, `synth.rs`). The whole gate is run on synthetic markets with a known answer: correlated random walks with volatility clustering and fat tails, optionally with a planted mean-reverting component.

| Synthetic markets (700 days × 8 coins, 384-rule grid) | Typical best in-sample Sharpe | Promoted |
|---|---|---|
| Nothing to find, 200 panels | 0.65 (up to 2.5) | 0 of 200 |
| Planted edge, moderate, 40 panels | 3.7 | 15% (78% REVISE, 8% REJECT) |
| Planted edge, strong, 40 panels | 6.2 | 85% (15% REVISE) |

It does not promote noise. It is strict: with 384 rules tried on two years, an edge has to be large to be told from luck, and a real but moderate one mostly comes back as REVISE. That is the price of searching, stated up front.

## 4. First verdicts (2026-11-11)

Design window 2024-01-01 to 2026-04-15 (836 days); the 210 days after it are the lockbox, still sealed for every family.

| Family (hypothesis) | Rules | Best in sample | Walk-forward, bps/day | Reality check p | Verdict |
|---|---|---|---|---|---|
| `meanrev`, 15m (H-L01) | 384 | Sharpe −0.20 | −10.0 [−21.4, +1.2] | 1.00 | REJECT |
| `trend`, 1h (H-L02) | 40 | Sharpe 1.19 | +3.9 [−13.4, +22.0] | 0.22 | REJECT |
| `xs_reversal`, 1h (H-L03) | 108 | Sharpe 1.23 | −3.0 [−9.5, +3.7] | 0.77 | REJECT |
| `funding_carry`, 1h (H-L04) | 54 | Sharpe 0.82 | +0.6 [−6.9, +7.5] | 0.53 | REJECT |

**H-R01.** Its exact rule loses on the long sample: 15,974 trades at −19.8 bps each, 0 of 20 coins and 0 of 9 quarters profitable. An independent recomputation outside the simulator (6 coins, no stops) gives −17 bps gross per fade: most fades win a little, and the ones that keep running lose a lot. The same simulator reproduces the old backtest on Hyperliquid's last 49 days (435 trades at +37.4 bps against 422 at +35.0). Both numbers are right. The 52 days were a stretch in which fading worked.

**Trend** is the nearest miss: long-only 16-day breakouts made +13.9 bps/day in sample with a t of 1.9 against the basket, but the best of 40 rules does that by luck about one time in five. It runs on paper as a labelled, unproven candidate. Its card says an edge that size would take about 970 days of paper trading to confirm, which is the honest answer to "can we know soon".

Nothing was promoted. That is the result.

## 5. Jev as a market reader (`src/lab/probe.rs`, `probes/*.toml`)

A probe says when to ask (at a rule's candidates, or every N bars), builds a state from bars already closed, and asks typed questions. Answers are cached by probe hash, venue, coin and bar, so reruns are free; `lab jev score` judges them against what happened next; the `jev_gate` and `jev_direction` families trade on them through the same gate as any rule.

The state is anonymous: percent moves, stretch in standard deviations, volume ratios, funding, the last 16 bars. No coin name, date or price level, so the model cannot recall what happened next. A `revealed` variant re-asks the same states with coin and date shown to measure exactly that, and `repeat` asks twice to measure stability.

Pilot, `fade_judge` at H-R01's entries, Jev `jev-1.14.0`, 1,485 calls (about 1.47M tokens, 991 per call, ~210 ms):

| Sample | n | `p_revert` IC | vs day-block shuffles | AUC | `p_run` AUC |
|---|---|---|---|---|---|
| Hyperliquid, last 49 days | 892 | −0.110 | 1st percentile | 0.45 | 0.59 |
| Binance, spread over 836 days | 341 | −0.042 | 22nd percentile | 0.52 | 0.64 |

- Jev does not tell fades that revert from fades that keep running (H-J01 rejected). The wrong-way reading in the recent window did not repeat over the long sample.
- It does read risk: `p_run` ("goes 2% against the trade first") beats chance in both samples. That did not sort winners from losers.
- No sign of memory: with coin and date revealed the IC is −0.040 against −0.073 anonymous on the same 150 states (H-J03).
- Answers are stable: the same state asked twice differs by about 0.01.

Not tested yet: `direction` (H-J02), the full multi-year replay (about 16,000 entries, roughly 16M tokens), and chart images, which Jev accepts.

## 6. The trader (`src/trade/`)

`trade run` loads `strategies/*.json`, splits capital equally, and for each strategy recomputes the plan at every bar close on Hyperliquid's own candles, then acts on the live book. Stops are checked every second, funding accrues, positions survive a restart. The book (`book.rs`) is a pure state machine; a test replays panels through it bar by bar and requires the simulator's trades.

`trade monitor` answers three questions from files:
1. **Parity**: do paper entries match the simulator's on the same days?
2. **Sequential test**: KILL when "no edge" has become the better explanation of the paper days, GRADUATE when the card's edge has (5% false positives, 80% power).
3. **Drawdown**: past the 99th percentile of the backtest's own bootstrap for the days elapsed is a KILL.

It writes a verdict and applies nothing. v3 places no real orders; graduating on paper is the precondition, and arming stays a human decision.

Also in the engine:
- **Jev catalyst scalper, record mode** (`catalyst.rs`, `intel/`, ported from v2): 18 feeds and 4 exchange announcement APIs, local tagging, batched Jev scoring under an hourly call budget. Every scored headline about a coin is logged with the live mid and marked out at 5 min, 30 min, 2 h and 6 h net of the round trip, triggered or not. Historical news replay leaks hindsight, so this evidence can only be collected forward.
- **1-second recorder** (`ticks/`): about 140 MB a day for 20 coins, pruned after 14 days.
- **Maker-exit record** (H-X01): for every time-stop exit, whether a resting order at the touch would have filled within 5 minutes.
- **Live probes** (off by default): any probe can be asked at each bar close into the same cache a replay writes.

## 7. Running it

```sh
cargo build --profile lab -j 2                      # opt-level 2: bootstraps are slow unoptimized
B=target/lab/jev-trader

$B lab fetch                                        # archive up to date (Binance ~15 min the first time)
$B lab validate --family families/trend.toml        # verdict → lab-data/reports/<study>/
$B lab run --family families/meanrev.toml --spec "meanrev:hold=24,n=288,resid=0,stop=500,z=2"
$B lab calibrate --family families/meanrev.toml     # the gate on no-edge markets
$B lab report

$B lab jev replay --probe probes/fade_judge.toml --dry-run          # shows the state, asks nothing
$B --env-file .env.local lab jev replay --probe probes/fade_judge.toml --entries-only --max-calls 200
$B lab jev score --probe probes/fade_judge.toml

$B lab promote --family families/trend.toml --candidate             # unproven, labelled as such
$B --env-file .env.local trade run                  # paper; needs the Jev key only for the catalyst recorder
$B trade monitor
```

## 8. Limits

- The universe is today's survivors; delisted coins are absent.
- Binance prices stand in for Hyperliquid before the venue check; funding in research is Binance's.
- Bars are the finest historical step (15m here). Fills between bars are modelled, not observed; the tick recorder starts fixing that from now on.
- The gate is conservative. A real edge with Sharpe 1 to 2 will be rejected on 2.3 years of data and 200 to 400 trials. The way through is fewer, better-motivated trials, not looser thresholds.
- Catalyst markouts pending when the engine stops are lost.
- No dashboard page for the lab yet; reports are markdown under `lab-data/reports/`.

## 9. Next

1. Let `trade run` collect catalyst markouts and the trend candidate's paper days; read `trade monitor` weekly.
2. New families go in `families/` with a hypothesis first, and a small grid: the ledger makes every extra rule cost something.
3. If Jev is to be tried further: the `direction` probe on a few hundred bars (H-J02), then chart images at candidates.
4. Decide H-R01's live paper run by its own rule (n ≥ 100); the lab's evidence says not to expect it to pass.
