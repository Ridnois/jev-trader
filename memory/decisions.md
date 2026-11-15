# Decisions (newest first, absolute dates, UTC)

- **2026-11-14**: Project memory moved into the repo (`AGENTS.md`, `memory/`, skill `project-memory`) so every agent shares it, at the user's request.
- **2026-11-13 23:43:56 UTC**: Ledger era 2 (`era_start` in lab.toml). trials.jsonl (1,540 rows) and lockbox.jsonl (2 openings) archived to `lab-data/archive/era-before-2026-11-13/`; lineage kept whole. Recorded as H-ERA-2.
- **2026-11-13**: History start moved from 2024-01-01 to 2021-01-01 (H-HISTORY); lockbox held at ≥ 2026-04-19 so it only ever holds unseen days.
- **2026-11-13**: Evolve loop runs hourly by cron; `binance-1h` budget raised to 300.
- **2026-11-12**: `lab evolve --auto` chosen as run-until-stop (budget 200/study, stall 3); promotion stays manual. Deterministic proposer first; an LLM proposer later only through the same file format and its own calibration.
- **2026-11-11**: v3 built on branch `v3` (worktree). Research on Binance USDT-M history as a proxy; costs always Hyperliquid's. First verdicts: meanrev, trend, xs_reversal, funding_carry all REJECT. Jev `fade_judge` probe showed no skill (H-J01 rejected).
- **2026-11-11**: Main's bots stopped; v2 discarded (survives on origin/v2).
- **2026-11-11**: Dashboard redone in Next.js + shadcn; user picked the plain dark Vercel look over themed directions.
