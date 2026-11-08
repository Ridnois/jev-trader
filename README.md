# jev-trader v2

One trading engine for **Hyperliquid perps**, in Rust, built around TypeSafe's **Jev**. News (read by Jev) and order flow (read from the market) **must agree** before money moves. It runs on paper by default: real Hyperliquid prices, real fees and funding, simulated fills.

v1 (two separate bots) is preserved at git tag `v1`. Why v2 is shaped this way: [DESIGN-V2.md](DESIGN-V2.md).

## Quick start

```sh
echo 'TYPESAFE_API_KEY=...' > .env.local && chmod 600 .env.local
cargo build --release -j 2
./target/release/jev-trader run          # the engine (paper, data-v2/)
./target/release/jev-trader dashboard    # http://127.0.0.1:8787
./target/release/jev-trader report       # is it working? → data-v2/report.md
```

Other commands: `news` (every source through the local filter, no Jev calls), `analyze "<headline>"` (score one headline), `hl scan`, `hl check`, `hl clear-hold`.

## What it does

```
22 sources ──► local tagger (free) ──► batched Jev (≈180–220 tok/headline) ──► events ──► per-coin news bias
 18 RSS          coin? noise? price                impact −1..1                           │
 Binance/Upbit/  recap? duplicate?                 new concrete fact?                     ├─► CATALYST: strong fresh headline
 Bybit/OKX                                         hack?                                  │   + order flow confirms + not chased
 listings                                                                                 │   + Jev review (priced in?) ─► trade
                                                                                          └─► FLOW: order-flow burst
Hyperliquid websocket ──► flow, book, volatility, funding ───────────────────────────────────►    + news bias agrees ─► trade
```

**Catalyst trades** (news leads). Jev scores a headline as strong (|impact| ≥ 0.45), a new concrete fact (≥ 0.6), and we saw it within 20 min of publication. The engine then watches the coin for 10 min and enters only if buyers (or sellers) confirm it and price hasn't already run more than one hourly ATR. Before entering, one Jev call asks "already priced in?", which halves or vetoes the trade but never starts one. Stop at 1× ATR(1h), target at 2× the stop, resting as a maker order, trailing to break-even after costs once +1R, 4h time stop. It can trade any liquid Hyperliquid perp that the news names, for example a coin Binance or Upbit just listed.

**Flow trades** (market leads). The v1 scalper's burst gates (activity, trade-flow and book imbalance, push, no chase, a cost gate at 3× round trip), now allowed **only when the coin's news bias points the same way**. Maker entry, 30 min hold, the same stop/target/trail logic. v1's "flow reversed" exit is removed.

**Risk.** Size comes from risk (0.5% of capital per catalyst, 0.25% per flow trade, divided by stop distance), capped at 35% of capital per coin and 1× gross. Also: 3 positions max, 4 entries/hour, a 2% daily loss stop, a 30 min cooldown after a loss, no joining a side paying crowded funding, and no longs into a fresh hack.

**Evidence.** Every catalyst-grade headline is marked out 5/15/60/240 min later, taken or not. So is every flow candidate (with and without news support). `report` and the dashboard answer three questions: do trades pay after fees, do strong headlines move price in Jev's direction, and does the news filter improve flow trades? Intervals are clustered by hour. Under 20 independent hours is labelled "too little data".

## Jev tokens

Measured against the live API on 2026-11-08:

| | v1 | v2 |
|---|---|---|
| Tokens per headline | ~751 (1 per call, 4 long questions) | ~180–220 (batched 10 per call, 2–3 short questions) |
| Headlines sent | all of them, twice (both bots) | only tradable, non-noise, non-recap, non-duplicate: ~40% of fresh items |
| Per-trade calls | scalper review on every candidate (133 calls/2 days, uninformative) | one review per catalyst entry |
| Ceiling | none | `jev.max_calls_per_hour` (120) |

Each call carries ~270 tokens of fixed overhead, which is why batching matters. Jev is now used where it adds information: every news headline from 4× more sources, scored for impact, novelty and hacks, plus a priced-in check at the moment of a news trade.

## Configs

| Config | What | Ledger |
|---|---|---|
| `config.toml` | paper, mainnet prices | `data-v2/` |
| `config.testnet.toml` | real orders on **testnet** (fake money): plumbing check | `data-v2-testnet/` |
| `config.live.toml` | real orders on **mainnet** | `data-v2-live/` |

v1 configs are refused. Every setting has a default in `src/config.rs`.

## Live trading

Real orders need `[exchange] mode = "live"` **and** `run --live`. You set `max_capital_usd` yourself; at 0 the engine refuses to start. Keys go in an env file: `HL_ACCOUNT_ADDRESS` (main account) and `HL_AGENT_PRIVATE_KEY`, an API **agent** wallet approved at app.hyperliquid.xyz/API that can trade but not withdraw.

```sh
jev-trader --env-file .env.hl-testnet --config config.testnet.toml hl check
jev-trader --env-file .env.hl-testnet --config config.testnet.toml run --live
```

Safety:
- Separate ledgers per config. A ledger refuses the other mode or venue.
- IOC orders are capped at `max_slippage_bps`. Every exit, take-profit and backstop is **reduce-only**.
- Fills come from `userFills`, and every order carries a cloid. An unconfirmed order sets a hold until `hl clear-hold`.
- New in v2: every live position rests a **reduce-only stop-market on the exchange**, 1.5× the engine's stop distance. If the process dies, the position is still protected. Its order format is checked against the official SDK's signing vectors.
- Positions are reconciled every 30s. If a backstop or take-profit filled while the engine was away, it is booked from the fills. Any other mismatch puts the coin in REVIEW.
- On Ctrl+C, resting entries are cancelled. Positions, take-profits and backstops stay.

**Before going live:** paper `report` should show trades paying after fees with "pays", not "too little data", and the testnet plumbing should be checked. v1's paper run lost money. No strategy here has proven an edge yet.

## Files in `data-v2/`

`ledger.json` (account), `status.json` (live view), `news.jsonl` (every scored headline), `news_state.json` (restart memory), `jev_usage.jsonl` (every Jev call with tokens), `candidates.jsonl` (catalyst decisions and flow candidates with their gates), `markouts.jsonl` + `markouts_pending.json`, `orders.jsonl` (maker entries), `trades.jsonl` (round trips), `report.md`, `control.json` (written by the dashboard: pause, flatten).

The dashboard binds to 127.0.0.1. Its two buttons (Pause entries, Flatten all) have no authentication.
