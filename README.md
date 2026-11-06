# jev-trader

A news-driven crypto **paper-trading** bot written in Rust. Each tick it:

1. pulls RSS headlines (CoinDesk, Cointelegraph, Decrypt);
2. asks TypeSafe's Jev model typed questions about each one (asset, event type, sentiment, new-fact probability);
3. combines the decayed news score with price momentum from Binance public candles;
4. applies hard risk rules (per-asset, total and heat caps, stop loss, daily kill switch, optional weekly/monthly halts and losing-streak cooldown, cooldowns);
5. places paper fills;
6. later scores every headline, decision and blocked entry against what the price actually did.

Inputs fail closed: a missing price, too few closed candles, a stale or non-finite value, or a degraded Jev puts the asset (or the account) in **REVIEW**. That means no new entries, while exits and stops keep running on verified prices.

No real orders are sent unless you arm live trading on Binance (see *Live trading*).

## Run it

Put your key in `.env.local` (git-ignored; loaded automatically, a real environment variable still wins):

```sh
echo 'TYPESAFE_API_KEY=...' > .env.local && chmod 600 .env.local
cargo run --release -- run          # the trading loop (Ctrl+C to stop)
cargo run --release -- dashboard    # the monitoring page → http://127.0.0.1:8787
```

The loop and the dashboard are separate processes; either one can restart without the other. Other commands: `news` lists headlines without calling Jev, `analyze "<text>"` runs Jev on one headline, `status` prints the account, and `run --once` runs a single tick.

## Live trading (Binance spot)

`config.live.toml` runs the same strategy with real market orders. It is armed only when **both** `[exchange] mode = "live"` is set **and** you pass `--live`:

```sh
# BINANCE_API_KEY / BINANCE_SECRET_KEY in .env.local (spot trading on, withdrawals off, IP-restricted ideally)
cargo run --release -- --config config.live.toml binance check     # balances, pair rules, signed test orders (nothing executes)
cargo run --release -- --config config.live.toml run --live        # real orders
cargo run --release -- --config config.live.toml dashboard         # LIVE badge; Flatten sells for real
```

- **Separate ledger:** `data-live/`. A live ledger refuses paper commands and vice versa.
- **Capital cap:** `max_capital_usd` (it refuses to start at 0). The ledger starts with min(cap, free USDT) and never touches the rest of the account.
- **Real fills:** fills are booked from Binance's reported quantity, average price and commission. Buys spend `quoteOrderQty`; sells are floored to the pair's step size and respect min notional.
- **Every tick:** the bot checks that Binance still holds what the ledger holds. If not, that asset goes to REVIEW (no entries; exits continue). If the account check fails, no orders go out that tick.
- **Unconfirmed orders:** if a timeout or 5xx leaves an order's outcome unknown, the bot looks it up by client id. If it still can't confirm it, it sets a hold that blocks all entries until you check Binance and run `binance clear-hold`.

## Order-flow scalper (`scalp`)

A separate engine where **market microstructure decides and news only informs**:

| Layer | Role | Timing |
|---|---|---|
| Binance websocket (bookTicker + aggTrade) | decides: activity burst, trade-flow imbalance, book imbalance, 60s push, no chasing, **cost gate** (σ₁ₛ·√hold ≥ `cost_mult` × round-trip cost) | every `eval_ms` (500ms) |
| Jev entry review | reads recent headlines + a plain-language description of the move; vetoes on adverse news or event risk, halves size when the news contradicts the move; never starts a trade | only when an entry is about to happen (cached `jev_cache_secs`) |
| News context (RSS → Jev) | tilts the flow threshold by up to `news_tilt`; a hack alert blocks entries and forces exits | its own 60s loop |

**Long and short.** The same gates, mirrored, open shorts. Spot can't borrow, so the desk holds a neutral coin inventory (`inventory_usd` per coin, taken from free balances on testnet) and a short sells part of it and buys it back; it never sells below zero. Trade P&L (cash change + coin change at the exit price) is reported apart from inventory drift, and the daily loss limit applies to trade P&L only. News tone tilts each side the opposite way; a hack alert blocks longs, not shorts. Jev's review is asked the mirrored questions for a short.

Exits: take profit, stop (scaled to volatility and at least the round-trip cost; once the trade is `trail_after_cost_mult` × cost in profit it moves to break-even after fees and trails the best price), flow reversal, hack alert (longs), time stop (`hold_secs`).

```sh
jev-trader --config config.scalp.toml --env-file .env.test scalp --live   # Binance testnet
```

**Which coins.** `python3 -I scripts/vol_scan.py .env.test` ranks liquid USDT pairs by how often a 10-minute move clears the cost hurdle (with spread, volume, testnet listing and testnet inventory). The scalp universe in `config.scalp.toml` comes from it (2026-11-05: ZEC, NEAR, UNI, WLD, AAVE, HYPE, SUI, SOL; BTC/ETH cleared the hurdle in < 6% of windows). Rankings drift, so re-run it now and then; changing the universe is hypothesis H-S05.

Files in `data-scalp/`: `scalp_status.json` (live view), `scalp_candidates.jsonl` (every candidate with its gates and Jev review, taken or not), `scalp_markouts.jsonl` (each candidate's move after 60/300/600s, net of cost), `scalp_trades.jsonl` (round trips), `scalp_events.jsonl` (news). Hypotheses H-S01 / H-S02 say what these must show before the scalper is trusted. Costs are modelled at real Binance spot fees even on testnet, so in quiet markets the cost gate blocks almost everything: that is intended.

## Measure before changing anything

```sh
cargo run --release -- report                                  # settled scorecard → data/report.md, data/backlog.jsonl
cargo run --release -- backtest --from 2026-11-01              # replay cached Jev answers, no Jev calls
cargo run --release -- backtest --compare config.toml alt.toml # PASS / REVISE / REJECT for a change
```

- **report** clusters overlapping windows before computing anything. It gives each figure with its n, and each effect with its percentile within a block-permutation null. It covers IC and hit rate per horizon, decision outcomes, what each gate cost or saved (blocked entries settled as if taken), and stop calibration from the MAE of winners. Its backlog only makes suggestions; nothing edits `config.toml`.
- **backtest** runs the live decision path on simulated time against momentum-only (the declared primary baseline), news-only and buy-and-hold. It then regresses the strategy on momentum-only with Newey-West errors: RESIDUAL_EDGE, BASELINE_EXPLAINED, RESIDUAL_FRAGILE or INSUFFICIENT_EVIDENCE. Runs are resumable under `data/backtests/<run_id>/`.
- **--compare** judges only the last 30% of the window. It checks sample size, how many parameters changed, regime dependency (BTC 200h-SMA proxy), costs, and whether momentum explains the result. Verdicts go to `data/reviews.jsonl`, and `report` flags a config that was applied against a REJECT.
- Write every strategy change into `hypotheses.toml` **before** testing it.
- `data/macro_calendar.toml` (optional, `[[event]] name = "CPI"` / `at = "…Z"`) tags ticks within ±1h of a release. It is recorded only.

## Dashboard

The "Wire Desk" page shows every stage of the pipeline:

- **Wire**: every headline as wire copy, stamped with what happened to it. RUN · BUY/SELL means it drove a fill, ALERT means a hack hold, FILED means it was counted in the score, SPIKED means it was ignored and why. Unread copy is canary yellow.
- **Run list**: why each asset is held or flat, the news-score history against the entry and exit lines, and the headlines driving it. Clicking an asset in the right-hand column traces it: its source copy is lit on the wire and everything else dims.
- **Pipeline**: whether the trading loop is alive (PID, last and next tick), the dashboard's own state, each stage's latest result and timing, a 60-tick history strip, and the active settings (read-only).
- **Trades**: every paper fill, with links back to the copy that caused it, plus realized P&L on sells.
- **Scorecard**: the market regime (score, zone, the six components) and the settled scorecard, labelled *paper · settled · n=…*, with low-n figures greyed out.

A **posture line** under the header combines the circuit breaker, Jev health and the regime gate into one line: *Entries allowed · ceiling 60%*, *Exits only · weekly halt until Mon 00:00*, or *Review · Jev degraded*. Each asset's Run list card shows its full **decision trace** (every gate, and which cap bound the size), its stance counts (+pos / −neg / ~neutral stories), and REVIEW when its inputs can't be trusted. Wire slips show how many outlets carried the same story.

The dashboard has two controls only:
- **Pause trading** stops new buys. Stops, exits and the kill switch still act.
- **Flatten all** asks for a second click to confirm, then sells everything and pauses. The loop picks it up within about a second.

Thresholds and risk limits are changed in `config.toml`, never from the browser.

The dashboard binds to `127.0.0.1` by default. Its controls have no authentication, so don't expose it with `--host 0.0.0.0` on a network you don't trust.

### Preview without a Jev key

```sh
python3 scripts/demo_data.py /tmp/jev-demo/data
sed 's#^data_dir = .*#data_dir = "/tmp/jev-demo/data"#' config.toml > /tmp/jev-demo/config.toml
cargo run -- --config /tmp/jev-demo/config.toml dashboard
```

This writes **synthetic** data (real headline titles; invented Jev readings, prices and fills) plus a `SYNTHETIC` marker, so the page labels everything as demo data. The script refuses to write into a folder that looks like real bot data.

## Files in `data/`

| File | Written by | Contents |
|---|---|---|
| `state.json` | bot | cash, positions, kill-switch day, cooldowns, retained events |
| `status.json` | bot | PID, tick counter, last and next tick, per-stage health |
| `events.jsonl` | bot | Jev's answers per headline |
| `ticks.jsonl` | bot | per-asset price, momentum, news score, target, reason, driving headlines |
| `trades.jsonl` | bot | paper fills, with the ids of the headlines that drove them |
| `equity.jsonl` / `pipeline.jsonl` | bot | equity curve and per-tick stage history |
| `control.json` | dashboard | pause flag and flatten requests (the bot only reads this file) |
| `settle_pending.json` / `settlements.jsonl` | bot | records waiting for their horizons / scored exactly once |
| `regime.json` (+ `regime_cache.json`, `dominance.json`) | bot | market regime, hourly; breadth daily; one BTC-dominance point per day |
| `spiked.jsonl` | bot | headlines given up on after the Jev retry budget |
| `report.md` / `backlog.jsonl` / `reviews.jsonl` | `report` / `backtest --compare` | scorecard, suggestions, change verdicts |
| `backtests/` | `backtest` | per-run equity, trades, report and summary; cached klines |

Older files stay readable: every new field is optional.
