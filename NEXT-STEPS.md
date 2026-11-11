# Next steps (handoff, 2026-11-07)

> **2026-11-11:** v3 adds a research lab and a paper trader. Its results change this page: H-R01's rule loses over 2.3 years, and no rule family passed the gate. Read [DESIGN-V3.md](DESIGN-V3.md) §4 and §9 first. What follows is the v1 handoff, kept for the P/B spec and the testnet question.

Where the work stopped and what comes next. Read this before resuming.

## Where things stand

**Scalper** (paper, `data-hl/`, running since 2026-11-07 05:51 UTC). After about 10 hours (a Saturday):
- 18 trades, 1 win, −$24.41 after $10.80 in fees.
- Candidates that passed every market check lose after costs at every horizon: −8.2 bps net at 600s (95% −10.8 to −5.6). That rests on 10 hours, under the 30 needed to count as solid.
- The entry inputs (`flow`, `obi`, 60s push, activity) show no relation to the 10-minute move (p ≥ 0.19). Funding is the only input with a signal (IC −0.171). Only 7 candidates had |funding| ≥ 0.5 bps/h, so it is mostly a coin × side effect.
- Maker fills are adversely selected (filled −4.6 vs missed +4.8 bps at 600s). Per posted order, though, maker still beats a taker entry (−4.1 vs −10.8 bps), because a missed order costs nothing.
- Known bug, not fixed: the trades/hour budget counts *posted* orders, not fills (`ledger.entries_ms.push_back(now_ms)` at the maker post, `src/scalp/mod.rs:1078`). Missed orders use up the budget, so entries bunch up right after the top of each hour.

**Measurement loop** (commit `812ed1c`): `jev-trader --config config.scalp.toml scalp-report` writes `data-hl/scalp_report.md` and `scalp_backlog.jsonl`. The dashboard's *Is it working?* page shows the same scorecard. The backlog stays empty until there are 20 hours or 20 cases of evidence.

**P/B engine** (`STRATEGY-PB.md`, H-P01 to H-P06 in `hypotheses.toml`): the spec is revised and free of contradictions; nothing is built. Confirmed 2026-11-07:
- Stops tighter than 2× the round-trip cost are widened to that floor, not rejected (§4.1).
- Priority when setups conflict on a coin: C, then B, then A (§4).

## Open question (blocks the P/B live layer only, not paper)

Does Hyperliquid's `scheduleCancel` (dead-man switch) also cancel trigger (stop) orders? Details are in `STRATEGY-PB.md` §5.1. To settle it on testnet (`config.testnet.toml`, fake money):
1. Open a tiny position.
2. Place a reduce-only stop-market trigger and one resting limit order.
3. Schedule the cancel 10 s ahead, wait, then list open orders. Did the stop survive?
4. In the same run, send an entry with `grouping = "normalTpsl"` plus a stop. Does the stop go live on the exchange when the entry fills?

What each outcome means:
- Stop survives → use the dead-man switch plus trigger stops, as specified.
- Stop is cancelled, but `normalTpsl` works → attach the stop to every entry; the dead-man switch isn't needed for safety.
- Neither works → choose which risk to accept before building the live layer.

Neither call exists in `src/hyperliquid/exchange.rs` yet, so this needs a small addition or the official Python SDK.

## Next steps, in order

1. **Let the scalper run through weekdays (3–5 days)**, then run `scalp-report`. Decide on the scalper from the backlog and the H-C verdicts. Expect "retire it, or write a hypothesis for a different signal", not threshold tuning.
2. *(Optional)* Fix the trades/hour budget to count fills instead of posts. It removes the top-of-hour bias but won't create an edge.
3. **P/B step 1: the detector only.** A pure function per `STRATEGY-PB.md` §3, unit-tested on hand-built candles, then run in record mode over the 52 days of 15m candles for BTC, ETH, SOL and HYPE. Output the boxes and the candidates per setup, per coin, per day. Project the time to H-P01's sample (60 filled C-longs across ≥ 30 days). If it is beyond about 6 months, revisit the thresholds through a new hypothesis before building more.
4. If the rate is viable, build the rest in this order: the 15m candle cache, the trade manager (§4), `pb backtest` with the §8.3 fill rules, `pb report`, a block bootstrap in `stats.rs`, then the paper engine.
5. Before any live P/B: run the testnet test above and build §5.1.

## Guesses in the spec to revisit with data

12 h funding hold (§4.5); caps of 2 positions / 40% gross / 0.75% heat (§4.6); the 90-day minimum before compare verdicts and the 6-month cutoff (§6); every number in §7.

## Housekeeping

- `media/` (the 64 MB source video) is untracked. Add it to `.gitignore` or keep it out of the repo.
- The remote branch `scalp-measurement-loop` is already merged into `main` and can be deleted: `git push origin --delete scalp-measurement-loop`.
