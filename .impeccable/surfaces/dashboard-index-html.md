---
version: 1
slug: "dashboard-index-html"
primary_target: "dashboard/index.html"
related_targets: []
---

# Dashboard surface brief

Scope: `jev-trader dashboard`, the local monitoring page served by the bot binary for both bots (news bot `run`, order-flow scalper `scalp`) on Hyperliquid perps. Mode: Operate. Desktop tab or second monitor left open all day; glanced at, examined on events. Replaces both earlier dashboards (plain-language `/` and Wire Desk `/classic`). Controls limited to Pause and Flatten.

User answers (2026-11-06): both bots get equal weight; what must become clear is whose money / what's at risk, why each trade was taken or skipped, and whether it is working (evidence net of fees, hypotheses).

## Direction contract

THESIS: The dashboard is a departure board. Every coin each bot watches is a row; its status flips WATCHING → CLEARED → IN TRADE, or HELD at the named check that stopped it, so "why not" is read straight off the board. It refuses the KPI-tile trading terminal and the P&L-first equity chart.

OWN-WORLD: Brushed-steel ground (#C9CDD2), graphite rail (#2A2D31), white working sheets, and one matte flap-black board (#141414) with flap-white caps (#F2F2F0) split by a hairline. Amber (#E3A21A) means waiting or review, cancelled red (#B4372F) means stopped or loss, green (#4FB37A on flaps, darker on white) means cleared or profit. Barlow Condensed caps on flaps and labels, Barlow for sentences.

STORY: The operator knows at a glance whose money each bot is using and what is at risk, sees every coin's status and the check that holds it, and can open any row or trade to read its full reasoning in plain sentences. The evidence page says honestly whether either bot beats its fees yet.

FIRST VIEWPORT: 220px graphite rail (wordmark, Board / Scalper / News bot / Evidence nav, money note, two heartbeats). Main: "Departures" title with posture and Pause/Flatten at right; a row of three white sheets (Scalper money + result, News bot money + result, Is it working?); then the flap board, one row per coin for both bots, columns Coin · Bot · Side · Status · Stopped at · Cost · Last result.

FORM: Split-flap concourse board (catalog challenger, competitive verdict, chosen by the user over the assigned Flight Strip Board). seed:56582f45fa12f024. Signature interaction: when a row's value changes, its characters cascade through flaps to the new value; clicking a row opens its decision trace.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
