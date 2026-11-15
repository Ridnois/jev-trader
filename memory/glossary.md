# Glossary

Project terms, IDs and shorthand. The dashboard's reader-facing definitions live in `web/lib/glossary.ts`; keep wording consistent with it.

## Lab
| Term | Meaning |
|---|---|
| family | One market idea: a rule plus the grid of variants the lab may try. Hand-written in `families/*.toml`, generated in `families/gen/gNNNN.toml` |
| grid | The variants a family allows; nothing outside it is tested. Bigger grid = harder-to-trust winner |
| rule / spec | One grid point, named by its settings, e.g. `trend:n=384,out_frac=0.25,shorts=0,stop=800` |
| trial | One rule simulated on one data set; appended to `lab-data/trials.jsonl` forever |
| ledger | `lab-data/trials.jsonl` (+ `lockbox.jsonl`); only rows after `era_start` count |
| era / era 2 | Ledger era started 2026-11-13 23:43:56 UTC; earlier rows archived |
| study | A data set: venue + interval, e.g. `binance-1h`, `binance-15m`; budgets and lockbox looks are per study |
| design window | All days before the lockbox; searches and walk-forward see only these |
| lockbox | Newest 20% of days (held at ≥ 2026-04-19); opened only to confirm a candidate; each opening (and each generated family judged) tightens the next to α/k |
| gate | Fixed checks (`lab.toml`): deflated Sharpe, walk-forward, costs, breadth...; verdict PASS / REVISE / REJECT |
| deflated Sharpe | Sharpe corrected for how many trials were run; bar 0.95 |
| luck bar | The Sharpe the best of N no-edge rules would show by chance |
| REVISE | Close but failed a check; e.g. g0045 short on deflated Sharpe 0.938 vs 0.95 |
| calibration | `lab calibrate`: run the gate/loop on synthetic no-edge markets; must promote ~0 |
| H-xxx | A hypothesis ID in `hypotheses.toml` (H-R01 meanrev scalper, H-J01 Jev fade judge, H-ERA-2, H-HISTORY...) |

## Evolve loop
| Term | Meaning |
|---|---|
| the loop / evolve | `lab evolve [--auto]`: learn → propose → compare → report |
| generation (G) | One loop cycle. G0 = hand-written families |
| strategy tag | `G12/g0045` = family g0045 proposed in generation 12; `G0/trend·4d` = hand-written trend, 4-day lookback |
| lessons | `lab-data/lessons/<date>.{json,md}`; under-sampled ones are "anecdotes" and ignored |
| operators | condition, restrict, invert, recombine, shrink, explore |
| lineage | `lab-data/lineage.jsonl`, every proposal/verdict/promotion; `lab lineage` prints the tree |
| composed rule | Family as data from a closed block library (`src/lab/compose.rs`), named by rule hash |

## Trading
| Term | Meaning |
|---|---|
| paper | Simulated fills on live prices; never real money |
| candidate / unproven | Promoted strategy still being watched on paper |
| catalyst recorder | Jev reads news, records markouts (record mode only) |
| Jev | TypeSafe's typed-question LLM (choice/score/noul), ~210-390 ms, ~1k tokens per call |
| probe | `probes/*.toml`: a Jev question replayed over bars (`lab jev replay`) |
| bps | Basis points, 1/100 of a percent |
| HL | Hyperliquid (venue traded); Binance = research proxy |

## Places and names
| Name | Meaning |
|---|---|
| main tree | `../trading`, branch main, old bots (stopped) |
| v3 / worktree | this directory, branch `v3` |
| v2 | discarded engine; origin/v2 |
| H-R01 | old 15m mean-reversion scalper rule (lab: -19.8 bps/trade) |
