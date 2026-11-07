# jev-trader

Two crypto trading bots for **Hyperliquid perps**, written in Rust and powered by TypeSafe's Jev model. Hyperliquid is the only venue: prices, funding, open interest, the regime inputs, settlement and backtest candles all come from its public API, and real orders (when you arm them) go to it. Why Hyperliquid: [CRYPTO-REDESIGN.md](CRYPTO-REDESIGN.md).

- **News bot** (`run`): long-only, hourly-candle strategy driven by headlines.
- **Scalper** (`scalp`): long and short, second-by-second order flow decides; news only informs.

Both run on **paper by default**: simulated fills on real Hyperliquid prices, charged Hyperliquid's real fees and funding. Real orders need `[exchange] mode = "live"` **and** `--live` (see *Live trading*).

## News bot

Each tick it:

1. pulls RSS headlines (CoinDesk, Cointelegraph, Decrypt);
2. asks Jev typed questions about each one (asset, event type, sentiment, new-fact probability);
3. combines the decayed news score with price momentum from Hyperliquid 1h perp candles;
4. applies hard risk rules (per-asset, total and heat caps, stop loss, daily kill switch, optional weekly/monthly halts and losing-streak cooldown, cooldowns);
5. places fills (paper, or IOC orders on Hyperliquid when live), at 1× notional, accruing funding on open longs;
6. later scores every headline, decision and blocked entry against what the price actually did.

Inputs fail closed: a missing price, too few closed candles, a stale or non-finite value, or a degraded Jev puts the asset (or the account) in **REVIEW**. That means no new entries, while exits and stops keep running on verified prices.

## Run it

Put your key in `.env.local` (git-ignored; loaded automatically, a real environment variable still wins):

```sh
echo 'TYPESAFE_API_KEY=...' > .env.local && chmod 600 .env.local
cargo run --release -- run          # the news bot (Ctrl+C to stop)
cargo run --release -- dashboard    # the monitoring page → http://127.0.0.1:8787
```

The loop and the dashboard are separate processes; either one can restart without the other. Other commands: `news` lists headlines without calling Jev, `analyze "<text>"` runs Jev on one headline, `status` prints the account, and `run --once` runs a single tick.

| Config | What | Ledger |
|---|---|---|
| `config.toml` | news bot, paper | `data/` |
| `config.scalp.toml` | scalper, paper | `data-hl/` |
| `config.testnet.toml` | scalper, **Hyperliquid testnet** orders (fake money) | `data-hl-testnet/` |
| `config.live.toml` | news bot, **live** on mainnet | `data-live/` |
| `config.scalp.live.toml` | scalper, **live** on mainnet | `data-hl-live/` |

A config that still carries Binance settings (`base_url`, `quote`, `stream_url`, `inventory_usd`, `pair = "BTCUSDT"`) refuses to load.

## Live trading (Hyperliquid)

Live mode signs orders with an **API agent wallet**: create one at app.hyperliquid.xyz/API and approve it for your account. An agent can place and cancel orders but **cannot withdraw or transfer**. The bot refuses to start with the main wallet's key.

```sh
# .env.local (or a separate --env-file per network):
#   HL_ACCOUNT_ADDRESS=0x...      your main account (public)
#   HL_AGENT_PRIVATE_KEY=0x...    the agent wallet's key
jev-trader --config config.live.toml hl check            # account, margin, coins, agent approval; sends no orders
jev-trader --config config.live.toml run --live          # news bot, real orders
jev-trader --config config.scalp.live.toml scalp --live  # scalper, real orders
jev-trader --env-file .env.hl-testnet --config config.testnet.toml scalp --live   # testnet first
```

Set `[exchange] max_capital_usd` yourself: it is 0 in every live config, which refuses to start.

Safety:

- **Separate ledgers** per config. A live ledger records its network and refuses paper commands, the other network, and ledgers made for Binance. It starts only if the account holds no position in the configured coins, so the books begin in sync.
- **Capped capital:** the ledger manages min(`max_capital_usd`, withdrawable USDC) and never sees the rest. `leverage = 1` sets cross 1× on each coin, so the exchange enforces the same 1× notional the ledgers size at.
- **Orders:** entries are post-only (ALO) for the scalper, IOC limits for the news bot; IOC prices sit at most `max_slippage_bps` through the reference price. Every exit is **reduce-only**, so it can't open or flip a position. A partly filled exit keeps closing on later ticks.
- **Fills are the source of truth:** sizes, prices and fees are booked from `userFills`, not from the order leaving the book. Every order carries a client id (cloid).
- **Reconciliation:** the exchange's positions are compared with the ledger (news bot every tick, scalper every 30s). A mismatch puts that coin in REVIEW: no entries, exits continue. A failed account check blocks entries.
- **Unknown outcomes:** after a timeout or 5xx the bot looks the order up by cloid. If it still can't confirm it, it sets a hold that blocks all entries until you check Hyperliquid and run `jev-trader hl clear-hold`.
- On Ctrl+C the scalper cancels its resting entries. Open positions stay in the ledger and are managed on restart. There is no exchange-side stop: if the process dies, positions wait until it comes back.

Signing is a port of the official Python SDK (msgpack action hash → EIP-712 "Agent" signature) and is tested against the SDK's published vectors.

## Order-flow scalper (`scalp`)

A separate engine where **market microstructure decides and news only informs**:

| Layer | Role | Timing |
|---|---|---|
| Hyperliquid websocket (`bbo`, `trades`, `activeAssetCtx`) | decides: activity burst, trade-flow imbalance, book imbalance, 60s push, no chasing, **cost gate** (σ₁ₛ·√hold ≥ `cost_mult` × round-trip cost incl. funding) | every `eval_ms` (500ms) |
| Jev entry review | reads recent headlines + a plain-language description of the move; vetoes on adverse news or event risk, halves size when the news contradicts the move; never starts a trade | only when an entry is about to happen (cached `jev_cache_secs`) |
| News context (RSS → Jev) | tilts the flow threshold by up to `news_tilt`; a hack alert blocks entries and forces exits | its own 60s loop |

**Long and short.** The same gates, mirrored, open shorts: a perp short is a plain sell. Positions are 1× notional; funding is accrued while open. Entries post at the bid (long) or ask (short) and count as filled only when trades print through the price (paper) or the exchange reports the fill (live). Unfilled entries are cancelled after `maker_timeout_secs` and logged as misses. News tone tilts each side the opposite way; a hack alert blocks longs, not shorts.

Exits: take profit, stop (scaled to volatility and at least the round-trip cost; once the trade is `trail_after_cost_mult` × cost in profit it moves to break-even after fees and trails the best price), flow reversal, hack alert (longs), time stop (`hold_secs`).

```sh
jev-trader --config config.scalp.toml hl scan        # rank perps by how often a 10m move pays the cost
jev-trader --config config.scalp.toml scalp          # paper fills on the real feed (no keys)
jev-trader --config config.scalp.toml dashboard      # Scalper pages read data-hl/ by default
```

**Which coins.** `hl scan` ranks liquid perps by how often a 10-minute move clears the cost hurdle at the config's fees. The universe in `config.scalp.toml` comes from it (2026-11-06: ZEC, HYPE, SOL, NEAR, ETH, XRP, kPEPE, ENA). Changing it is hypothesis H-C03.

Files in `data-hl/`: `scalp_status.json` (live view), `scalp_candidates.jsonl` (every candidate with its gates and Jev review, taken or not), `scalp_markouts.jsonl` (each candidate's move after 60/300/600s, net of cost), `scalp_orders.jsonl` (maker entries: filled, partial or missed), `scalp_trades.jsonl` (round trips), `scalp_events.jsonl` (news). Hypotheses H-C01 to H-C04 say what these must show before the scalper is trusted.

```sh
jev-trader --config config.scalp.toml scalp-report   # → data-hl/scalp_report.md, data-hl/scalp_backlog.jsonl
```

`scalp-report` is the scalper's measurement loop. It answers:

- **Does the signal pay?** Gross move, cost and net result at each markout horizon. It covers candidates that passed every market check, those placed as orders, those filled, and all of them.
- **Where does the money go?** Maker fill rate, filled vs missed markouts (adverse selection), and maker-as-run vs a taker entry on every order. Then each exit reason vs holding to the time stop, and what each check stopped vs what was taken.
- **Which inputs relate to the move?** Spearman IC of every logged feature, signed by side.
- **Where does each H-C hypothesis stand?** Its pass and invalidate rules are applied to the data. `hypotheses.toml` keeps the human status.

Means carry 95% intervals with errors clustered by UTC hour, the independent unit. Correlations carry a block-permutation null. The backlog suggests only; nothing edits a config. The dashboard's **Is it working?** page shows the same scorecard, recomputed at most once a minute.

## Measure before changing anything

```sh
cargo run --release -- report                                  # settled scorecard → data/report.md, data/backlog.jsonl
cargo run --release -- backtest --from 2026-11-01              # replay cached Jev answers, no Jev calls
cargo run --release -- backtest --compare config.toml alt.toml # PASS / REVISE / REJECT for a change
```

- **report** clusters overlapping windows before computing anything. It gives each figure with its n, and each effect with its percentile within a block-permutation null. It covers IC and hit rate per horizon, decision outcomes, what each gate cost or saved (blocked entries settled as if taken), and stop calibration from the MAE of winners. Its backlog only makes suggestions; nothing edits `config.toml`.
- **backtest** runs the live decision path on simulated time against momentum-only (the declared primary baseline), news-only and buy-and-hold. It then regresses the strategy on momentum-only with Newey-West errors: RESIDUAL_EDGE, BASELINE_EXPLAINED, RESIDUAL_FRAGILE or INSUFFICIENT_EVIDENCE. Runs are resumable under `data/backtests/<run_id>/`. Hyperliquid serves only the latest 5000 candles per interval, so 1h backtests reach back about 208 days.
- **--compare** judges only the last 30% of the window. It checks sample size, how many parameters changed, regime dependency (BTC 200h-SMA proxy), costs, and whether momentum explains the result. Verdicts go to `data/reviews.jsonl`, and `report` flags a config that was applied against a REJECT.
- Write every strategy change into `hypotheses.toml` **before** testing it.
- `data/macro_calendar.toml` (optional, `[[event]] name = "CPI"` / `at = "…Z"`) tags ticks within ±1h of a release. It is recorded only.

## Dashboard

`jev-trader dashboard` serves one page for both bots at http://127.0.0.1:8787. It reads the news bot's `data/` and the scalper's folder (`--scalp-dir`, default `data-hl/`). It is styled as a departure board, so each coin's status and the check holding it back read like a train's.

- **Departures** (home): what each bot is doing with whose money. The scalper and news bot sheets show the venue (paper / testnet / live), the result after fees, what's at risk right now (the loss if each stop hits), and the budgets that cap it: trades this hour, daily loss, open slots, exposure. An **Is it working?** sheet says plainly whether there's enough evidence yet. Below is the board: one row per coin for both bots, showing its side, status (WATCHING, IN TRADE, ORDER WAITING, HELD, REVIEW), what it's **waiting for** in plain words ("a rush of trades", "news above +0.30", "hourly limit") and its last result. Rows flip like split-flap characters when they change. Select a row to see every check in order, which one stopped it, and the live readings behind it.
- **Scalper**: what it does in four sentences, any open trade on a stop/entry/target ruler, then three tabs. **Decisions** lists every chance it found, what stopped it ("Jev vetoed", "Too small for fees", "Hourly limit full"…), and what a $1,000 trade would have made 1, 5 and 10 minutes later after fees, whether or not it was taken. **Trades** tells each round trip as a story. **How it decides** shows the checks in order with how many chances each one stopped.
- **News bot**: the same plain explanation. **Coins** shows each coin's news score on a scale with its buy and sell lines, the trend, the safety checks and the headlines driving it. **Headlines** shows Jev's four answers per headline, **Trades** lists the fills, and **Each look** shows the stages of the last tick with their timing.
- **Is it working?**: the scalper's results grouped by what happened to each decision, in dollars per $1,000 after fees, with n. Figures resting on fewer than 30 cases are greyed. Next, **does the signal pay?**: the same results with hour-clustered 95% ranges, followed by the scalper backlog from `scalp-report`. Then the tests from `hypotheses.toml`, grouped by bot, with progress toward each one's required sample. For scalper tests, this includes what the data says. Finally the news bot's settled scorecard (IC and hit rate per horizon) and the market regime.

The rail always shows **whose money** each bot is using and a heartbeat per bot (green alive, amber stale or paused, red stopped).

There are two controls, and both act on the **news bot only**:
- **Pause news bot** stops new buys. Stops, exits and the kill switch still act.
- **Flatten news bot** asks for a second click to confirm, then sells everything and pauses. The loop picks it up within about a second.

The scalper has no browser controls: stop it with Ctrl+C (resting entries are cancelled, open trades are managed again on restart). Thresholds and risk limits are changed in the config files, never from the browser.

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
| `backtests/` | `backtest` | per-run equity, trades, report and summary; cached Hyperliquid candles |

Older files stay readable: every new field is optional.
