# Crypto redesign: low-fee venues

2026-11-06. The research behind moving from Binance spot to Hyperliquid perps, what we took from published crypto-trading agent skills, and the phases.

**Update (2026-11-06, later):** the whole project now runs on Hyperliquid only. Binance code, configs and data sources are gone, the news bot uses Hyperliquid candles and funding, and Hyperliquid order placement (phase 2) is built. See §6.

## 1. The problem fees caused

The order-flow scalper paid about **22 bps per round trip** on Binance spot: 10 bps taker each way, 1 bps slippage each way, plus the spread. Its cost gate requires the typical 10-minute move to be at least 2× the cost. In the 2026-11-05 scan, BTC and ETH cleared that in under 6% of windows and the best alt (ZEC) in 36%. Testnet charges no fees, so the testnet P&L looked better than the strategy really was.

Lower fees don't create an edge. They lower the bar an edge has to clear.

## 2. Venues compared (fees checked 2026-11-06)

| Venue | Chain | Instrument | Maker / taker | Shorts | Depth | Verdict |
|---|---|---|---|---|---|---|
| **Hyperliquid** | own L1 | perps | **1.5 / 4.5 bps** tier 0, down to 0 / 2.4; rebates to −0.3 | native | deepest on-chain book (~$3B/day BTC, $850M ETH, $860M ZEC on scan day) | **Chosen** |
| Binance spot (old) | CEX | spot | 10 / 10 (7.5 with BNB) | inventory workaround | deepest overall | Too expensive for 10-minute holds |
| Lighter | ZK L2 | perps | 0 / 0 standard (300 ms taker delay), 0.2 bps premium | native | ~¼ of Hyperliquid | Watch: zero fees, but the 300 ms taker delay hurts a scalper's stops |
| Phoenix | **Solana** | perps (on-chain CLOB) | 0.5 / 3.5 bps | native | ~$30–37M/day across 104 markets, ~$6M/day SOL | Best Solana option; too thin for now. Second adapter. |
| Jupiter swap | **Solana** | spot via aggregator | 0–10 bps platform fee + price impact + priority fee | none | aggregated AMMs | Wrong tool: costs more per round trip than Hyperliquid taker; MEV/sandwich exposure; ~400 ms slots |
| Drift → "Velocity" | **Solana** | perps | ~−0.3 / 3.5 bps (reported) | native | relaunching | Avoid for now: $285–295M exploit 2026-04-01, relaunch status unclear |

### Why not Solana first

Solana is cheap per transaction, but for a scalper the costs that matter are spread, fee and fill quality, not gas.

- **Spot on Solana (Jupiter)** means AMM price impact and priority fees on both legs. You can't short, and swaps are exposed to sandwiching. A round trip costs more than Hyperliquid's taker-taker 9 bps.
- **Perps on Solana:** Phoenix has the right design (on-chain CLOB, 3.5 bps taker) but about 1/100th of Hyperliquid's volume. Drift, the largest, was drained in April 2026.
- Solana stays in the design: the `Venue` enum takes a Phoenix adapter (Rust Solana SDK) once its depth justifies it. `hl scan`'s method (hurdle-clear rate at that venue's real cost) is the test it has to pass.

### What changes in practice (`hl scan`, 72h, 2× cost hurdle)

| Coin | Cost (maker in / taker out) | Clears at Hyperliquid cost | Clears at Binance cost |
|---|---|---|---|
| ZEC | 7.6 bps | 75% | 37% |
| NEAR | 8.2 | 70% | 26% |
| HYPE | 7.8 | 61% | 19% |
| SOL | 7.6 | 55% | 10% |
| ETH | 7.4 | 42% | 6% |
| BTC | 8.1 | 32% | 3% |

This is room for an edge, not proof of one. H-C01 is the test.

## 3. What we took from crypto-trading agent skills

These skills on skills.sh were read as reference data, not installed: `crypto-trading-bots` (patterns, sharp edges, validations), `crypto-trading-advisor`, Traderton `crypto-trading`, Jupiter `integrating-jupiter`, Starchild `hyperliquid`.

| Lesson | Source | Applied here |
|---|---|---|
| Fills are the source of truth; an order leaving the open list doesn't mean it filled | hyperliquid | Maker entries count as filled only when trades print *strictly through* the price in a later second. Misses are logged (`scalp_orders.jsonl`). Live phase: reconcile against `userFills`. |
| Post-only (ALO) orders cut fees | hyperliquid | `entry = "maker"`: cost 7.5 bps instead of 12. Stops stay taker so exits are certain. |
| Funding and open interest come before technicals; extreme funding signals a crowded side | crypto-trading-advisor | Funding, OI and premium are on every candidate. The cost gate charges funding the side would pay. A contrarian tilt is H-C04: recorded only until evidence supports it. |
| Liquidity red lines (volume < $1M, spread > 1%) | crypto-trading-advisor | `hl scan --min-volume` (default $5M/day) plus `max_spread_bps`. The chosen universe is ≥ $18M/day. |
| Modes go paper → shadow → live; going live is the user's call | traderton | Paper fills on the real Hyperliquid feed now. Live needs the user's key and an explicit flag, and Claude never arms it. |
| Daily loss limit blocks new trades; report risk in money | traderton | Already present (daily limit on trade P&L). The dashboard speaks in dollars. |
| Sandwiching, zero `minOut`, keys in source | crypto-trading-bots | Main reason not to scalp Jupiter spot. Keys stay in git-ignored env files. A Solana adapter must set a slippage bound. |
| Sniping, honeypot tokens, copy trading | crypto-trading-bots | Rejected: not this product, and hard to verify honestly. |

## 4. What was built (phase 1)

- **`src/hyperliquid.rs`**: keyless info API (`metaAndAssetCtxs`, `candleSnapshot`) and `jev-trader hl scan`.
- **Hyperliquid feed** in the scalper: `bbo` (best bid/ask each block), `trades` (side "B" = taker bought, verified against the live book), and `activeAssetCtx` (funding, OI, mark/oracle), with a 30 s ping and reconnect backoff.
- **Perps accounting**: shorts are plain sells with no inventory, at 1× notional. Funding accrues per tick into cash and into each trade's P&L.
- **Cost model**: `round_trip_cost_bps(f, cfg, side)` covers maker or taker entry, taker exit, half or full spread, slippage, and funding paid over the hold.
- **Maker entry simulation**: post at the bid (long) or ask (short), fill only on a strict trade-through after posting, cancel after `maker_timeout_secs`.
- **`config.hl.toml`** → `data-hl/`, and hypotheses H-C01 to H-C04.

```sh
jev-trader --config config.scalp.toml hl scan       # rank coins at these fees
jev-trader --config config.scalp.toml scalp         # paper, real Hyperliquid feed
jev-trader --config config.scalp.toml dashboard     # watch it
```

## 5. Next phases

1. **Evidence (now):** run `config.hl.toml` for 7+ days and read H-C01, H-C02 and H-C03 off `scalp_markouts.jsonl` and `scalp_orders.jsonl`.
2. **Hyperliquid testnet orders:** built (§6). Run `config.testnet.toml` to check the plumbing. Trusting the strategy with money still waits on H-C01.
3. **Mainnet:** user-armed only, with `max_capital_usd` chosen by the user.
4. **Solana adapter (Phoenix):** when its depth on the chosen coins supports the strategy's size. Re-run the scan with Phoenix's real cost.
5. **Fee tier upkeep:** staking HYPE (5–40% off) or reaching maker-rebate tiers changes `fee_bps` and `maker_fee_bps`. Re-check the official fee page before changing them.

Sources: [Hyperliquid fees](https://hyperliquid.gitbook.io/hyperliquid-docs/trading/fees), [Hyperliquid websocket subscriptions](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions), [Phoenix SOL market](https://www.phoenix.trade/markets/SOL), [DefiLlama Phoenix](https://defillama.com/protocol/phoenix), [Drift exploit and relaunch](https://solanahub.de/en/news/drift-recovery-relaunch-update/), [Lighter vs Hyperliquid](https://www.truehold.xyz/blog/lighter-vs-hyperliquid), [Jupiter integration skill](https://skills.sh/jup-ag/agent-skills/integrating-jupiter), [crypto-trading-bots skill](https://skills.sh/omer-metin/skills-for-antigravity/crypto-trading-bots), [crypto-trading-advisor skill](https://skills.sh/0xrikt/crypto-skills/crypto-trading-advisor), [Hyperliquid skill](https://skills.sh/starchild-ai-agent/official-skills/hyperliquid).

## 6. Hyperliquid-only (phase 2 built)

- **One venue.** `src/binance.rs`, the Binance live path, the spot websocket feed, the coin-inventory shorts and `scripts/vol_scan.py` were removed. Configs carrying Binance keys refuse to load. Assets name Hyperliquid perps (`coin = "BTC"`).
- **News bot data.** 1h candles, BTC daily trend, alt breadth (top 20 perps by volume), funding and open interest all come from Hyperliquid's info API. Funding is hourly there and is stored ×8 for the regime's 8-hour scale. Paper fees are 4.5 bps taker, and funding accrues on open longs.
- **Signing** (`src/hyperliquid/sign.rs`): msgpack action hash → phantom agent → EIP-712 in the "Exchange" domain (chain 1337), via `k256` + `sha3`. It matches the Python SDK's test vectors, and a request signed with a throwaway key was parsed by testnet, which recovered the agent's address.
- **Client** (`src/hyperliquid/exchange.rs`): orders (ALO / IOC, cloid on every order), cancel by cloid, leverage, `clearinghouseState`, `orderStatus`, `userFillsByTime`, and tick/size rounding (5 significant figures, 6 − szDecimals decimals, integers always valid).
- **Execution** (`src/live.rs`): IOC limits capped `max_slippage_bps` through the price, reduce-only exits, fills and fees from `userFills`, lookup by cloid on unknown outcomes and a hold if still unknown. Live ledgers record their network. The scalper polls resting ALO entries, cancels them at timeout, opens partial fills, retries partial exits, and reconciles positions every 30 s.
- **Not built:** exchange-side protective stops (stops are managed by the process, so positions wait if it dies), websocket `userFills` (REST polling instead), and backtests older than about 208 days (the 5000-candle limit).
