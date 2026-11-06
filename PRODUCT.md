# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

Rust server + plain HTML/JS: a `jev-trader dashboard` subcommand (axum) serves one static page with a live event stream (SSE). Single binary, no Node toolchain.

## Users

A solo operator (the bot's author) running jev-trader on their own machine. The dashboard is a desktop browser tab or second monitor left open all day while they do other work: glanced at periodically, examined closely when something happens (a trade, a hack alert, a kill-switch trip, a failing feed).

## Product Purpose

jev-trader is a news-driven crypto paper-trading bot written in Rust. Each tick it pulls RSS headlines, asks TypeSafe's Jev model typed questions about them (asset, event type, sentiment, new-information probability), combines the decayed news score with price momentum from Binance public klines, applies hard risk rules, and executes paper fills. The dashboard lets the operator monitor every stage of that pipeline, confirm the bot is alive and healthy, understand why each decision was made, and stop trading safely when needed. Success: from a glance, the operator knows whether the bot is alive, whether anything needs attention, and what it did since the last look.

## Positioning

Decisions are explainable end to end: every position traces back to specific headlines, Jev's calibrated probabilities, the momentum reading, and the risk gate that allowed or blocked it. The dashboard shows that chain of reasoning, not just P&L.

## Operating Context

- Pipeline stages per tick (default 60s): feed fetch (CoinDesk, Cointelegraph, Decrypt RSS) → Jev analysis (concurrent, ~70–500ms each) → price fetch (BTCUSDT, ETHUSDT, SOLUSDT, 1h candles) → signal (news score with 2h half-life, 20-candle SMA momentum) → risk gates (20% per asset, 60% gross, 4% stop, 3% daily kill switch, 6h cooldown) → paper execution (0.1% fee, 5bps slippage).
- Data on disk in `data/`: `state.json` (cash, positions, kill-switch day, cooldowns, retained events), `events.jsonl` (Jev features per headline), `ticks.jsonl` (per-asset price, momentum, news signal, target, final weight), `trades.jsonl` (fills).
- The bot process (`jev-trader run`) and the dashboard may run as separate processes.

## Capabilities and Constraints

- Two desks: the news desk (paper by default) and the order-flow scalp desk (`jev-trader scalp`, currently on the Binance spot testnet with fake money). Real orders need `mode = "live"` plus `--live`; every number on screen names its desk and whose money it is.
- Dashboard controls are limited to safety: **Pause trading** and **Flatten all positions**. Strategy thresholds and risk limits are not editable from the browser; they live in `config.toml`.
- Long-only spot, assets BTC/ETH/SOL (configurable).
- Jev is early-access; its calls can fail or time out, and failed headlines retry next tick.
- Undecided: per-stage health telemetry format (to be added to the bot for the dashboard).

## Evidence on Hand

No historical performance yet; the bot has not run with a Jev key. Any numbers shown before real runs are synthetic and must be labelled as such. No benchmarks, returns, or claims of edge may be presented.

## Product Principles

1. Alive or not, at a glance: process health and staleness are never ambiguous.
2. Every decision is explainable: headline → Jev answer → signal → risk gate → fill.
3. Safety over convenience: stopping is one deliberate action; changing strategy is not a browser action.
4. Honest numbers: paper results are labelled paper, testnet results testnet, synthetic data synthetic.
5. Self-explaining: each view says which desk it belongs to, how to read it, and what its jargon means, without leaving the page.
