# jev-trader v3: agent hot cache

Read this first. Details live in `memory/` (skill: `project-memory`, in `.agents/skills/`).
Memory is a snapshot: check anything about running processes before acting on it.

## What this is
Rust crypto research lab + **paper** trader for Hyperliquid perps. Branch `v3`, a git worktree
of the main repo (`../trading`, branch `main`, old bots, stopped). Public GitHub repo.
Goal: find a strategy that survives an honest, search-corrected gate, then prove it on paper.
Design: `DESIGN-V3.md`. Loop plan: `PLAN-EVOLVE.md`. Pre-registered hypotheses: `hypotheses.toml`.

## Running now (checked 2026-11-14)
| What | How to check |
|---|---|
| Paper trader `trade run` | `scripts/v3-services.sh status`, `target/lab/jev-trader trade monitor` |
| Dashboard on :8787 (Next.js prod build) | `scripts/v3-services.sh status web` |
| Evolve loop, hourly at :17 by user crontab | `scripts/v3-evolve-hourly.sh`, log `data-v3/evolve.log` |
→ detail: `memory/state.md`

## Guardrails (full list + reasons: `memory/guardrails.md`)
- Build with `-j 2`: `cargo build --profile lab -j 2`; tests `cargo test --profile lab -j 2`.
- Never place real orders, set `max_capital_usd`, or start a live run.
- Don't stop/restart the trader or write `strategies/`, `data-v3/` without asking. Stop by explicit PID, never `pkill -f`.
- Real `lab evolve`/`validate`/`compare` runs spend trials irreversibly: ask first (`--dry-run` is free).
- Never open the lockbox to "see if it works". Promotion is always the user's decision.
- Don't change gate thresholds, cost model or universe in `lab.toml` after seeing results.
- New strategy ideas: hypothesis in `hypotheses.toml` first, then through the lab.
- Public repo: scan staged files for keys before any push. Jev key: `../trading/.env.local` (pass `--env-file`).
- Commit/push only when asked.

## Terms (full: `memory/glossary.md`)
| Term | Meaning |
|---|---|
| **lab** | `jev-trader lab …`, `src/lab/`: backtests + the gate |
| **gate** | Fixed pass/fail checks in `lab.toml`; verdicts PASS / REVISE / REJECT |
| **family** | One idea as a rule + a grid of variants (`families/*.toml`, generated: `families/gen/gNNNN.toml`) |
| **trial** | One rule simulated once; logged forever in `lab-data/trials.jsonl`; raises the bar for all later ones |
| **lockbox** | Newest 20% of days (≥ 2026-04-19); no search may see them; each opening tightens the next |
| **era 2** | Ledger reset 2026-11-13 23:43:56 UTC (`era_start`); old trials in `lab-data/archive/` (H-ERA-2) |
| **evolve / the loop** | `lab evolve --auto`: learn → propose → compare, generation after generation |
| **G12/g0045** | Strategy tag: generation 12, family g0045. G0 = hand-written |
| **study** | Venue + interval data set, e.g. `binance-1h`; budgets are per study |
| **Jev** | TypeSafe's typed LLM used as a market/news reader (`probes/`) |
| **bps** | 1/100 of a percent |

## Where things are
| Path | What |
|---|---|
| `src/lab/` | gate, search, evolve (learn/propose/compare/compose/lineage) |
| `src/trade/` | paper trader, book, catalyst recorder |
| `web/` | dashboard (Next 16 + shadcn), `web/DESIGN.md` |
| `lab-data/` | archive, ledger, lineage, reports (`reports/evolve/gNNN.md`) |
| `data-v3/` | trader state, ticks, logs, pids |
| `strategies/*.json` | promoted (paper) strategies |

## User preferences
- Wants profitability, but honest evidence over good-looking backtests.
- Dashboard: plain dark Vercel-style look; no themed "worlds".
- Spare RAM (`-j 2`). Ask before anything irreversible.
