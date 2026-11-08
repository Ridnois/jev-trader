# v2 design: why it is shaped this way

2026-11-08. The goals were fewer Jev tokens, more news sources, one strategy combining news and order flow, more value from Jev, and a profitable bot. Every decision below starts from v1's own data (the paper runs of 2026-11-05 to 11-08, kept in `data/` and `data-hl/`) or from measurements against the live APIs.

## 1. What v1's data said

| | Finding | Consequence for v2 |
|---|---|---|
| Scalper | 56 trades, 4 wins, **−$59** (−10.6 bps/trade). Gross was negative before fees (−$25.6). | Order flow alone has no edge at this horizon. It only trades now when news agrees. |
| Scalper signal | Markouts of 2,300 candidates: +0.9 / +2.4 / +3.1 bps gross at 1 / 5 / 10 min, against 7.7 bps of cost. | 10-minute holds can't pay. Flow holds 30 min and needs a 3× cost gate. Catalyst trades target ~1–2 hourly ATR (50–150 bps), so the ~12 bps of cost is a small fraction. |
| Maker entries | Filled entries did worse than missed ones (−0.4 vs +2.4 bps at 5 min): adverse selection. | Catalyst entries are taker (speed matters after news). Flow keeps maker, and the report measures it. |
| Exits | "Flow reversed" closed 33 of 56 trades at −12.7 bps each, the worst exit. | Removed. Exits are stop, maker take-profit, trailing break-even, time, news turned, hack. |
| Jev review | 133 entry reviews. `event_risk` sat between 0.6 and 0.75 every time, so the vetoes were close to random, and vetoed trades did no worse than taken ones. | Replaced with one priced-in review, only at catalyst entries. |
| News bot | 0 trades in 3 days. 3 sources. 44% of its Jev calls were about coins it couldn't trade. | 22 sources, local coin tagging, a universe of every liquid Hyperliquid perp. |
| Duplication | Both bots analyzed the same headlines separately. | One pipeline, one process. |
| Headline edge | 32 scored events, 8 strong. Many strong ones were *recaps of a move that already happened* ("Bitcoin crashes 6%…"). | Price recaps are filtered locally and Jev is asked to score them as not news. Catalysts must be seen within 20 min, and the move since publication must stay under 1 ATR. |

## 2. Tokens (measured on the live API, 2026-11-08)

- Every call carries **~270 input tokens** of fixed overhead ("hi" plus one yes/no question = 268 tokens).
- v1's schema (4 questions, a 5-asset choice, a 6-event choice) cost about 655 tokens in and 185 out per headline, one per call. With summaries, the logs show ~751.
- Batching 3 headlines into one call cost 496 tokens against 979 for three single calls, with near-identical scores (3.42 vs 3.51, 0.57 vs 0.26, 1.99 vs 1.97).
- Lean schema: the coin is tagged locally, Jev answers impact (5-level score) + new fact (noul) + hack (noul, only when hack words appear), 10 per call. That is **~180 tokens/headline** in the test and **219** in the first live run (30 headlines in 4 calls).
- The local filters (noise, price recaps, untradable, duplicates) remove about 60% of fresh items before Jev sees them.

## 3. Sources

18 RSS feeds (CoinDesk, Cointelegraph, Decrypt, The Block, Blockworks, DL News, The Defiant, Bitcoin Magazine, CryptoSlate, crypto.news, BeInCrypto, cryptonews, NewsBTC, Bitcoinist, CryptoPotato, Google News "crypto, last hour", Federal Reserve and SEC press releases). Plus 4 keyless exchange announcement APIs: Binance listings and delistings, Upbit (Korean titles translated locally), Bybit and OKX. Stock and tokenized-securities items are dropped. All were checked returning 200 on 2026-11-08. The dashboard shows each source's median publication-to-first-seen lag, so slow sources can be identified and removed with evidence. On day one, Google News was often the fastest (minutes), while the outlet RSS feeds lagged 30–150 min.

## 4. What is honest to claim

Nothing here proves an edge yet. v2 drops what v1's data showed to lose (pure flow, 10-minute holds, flow-reversed exits, uninformative reviews). It concentrates on the one plausible edge (fresh, concrete, strong catalysts that the market confirms) and measures that edge directly: headline markouts taken or not, and flow with vs without news. Going live should wait until `report` says "pays" with 20+ independent hours, for trades after fees.

## 5. Kept from v1

Hyperliquid signing (SDK test vectors), the order client, live safety (agent wallet, `--live` + `mode = "live"`, `max_capital_usd = 0` by default, reduce-only exits, fills from `userFills`, cloid lookup and holds, reconciliation), microstructure features, and the ledger arithmetic, all with their tests. Removed: the news bot's hourly decision path, regime, settlement and backtest machinery (≈6,000 lines, tag `v1`). That machinery suited a 1h strategy that never traded.

## 6. Next steps, with evidence

1. Run paper for 1–2 weeks. Read `report`: strong-headline markouts by source and by kind (listing vs news), and flow agree vs no-support.
2. Drop sources whose headlines show no edge or arrive late. Raise or lower `catalyst.min_impact` / `min_new_info` only on markout evidence.
3. If listings show an edge but arrive late, poll announcements more often (`announcements_every_secs`).
4. Testnet plumbing check (`config.testnet.toml`), then live with a small `max_capital_usd` that you choose.
