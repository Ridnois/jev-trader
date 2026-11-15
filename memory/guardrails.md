# Guardrails

Each rule has its reason so edge cases can be judged. Update when the user adds or lifts one.

| Rule | Why |
|---|---|
| Build and test with `-j 2` | The user asked to spare RAM; full parallel builds starve the machine. |
| Never place real orders, set `max_capital_usd`, or start a live run | Paper only until a strategy passes the gate and proves itself on paper; the user decides. |
| Don't stop/restart `trade run`, or write into `strategies/` or `data-v3/`, without asking | Paper days are the only clean evidence; gaps or edited state corrupt it. |
| Stop processes by explicit PID (`ps -C jev-trader`), never `pkill -f` | `pkill -f` also matches the user's other processes and the main tree's bots. |
| `stop web` can leave next-server alive without a listener: wait for the pid to exit (kill -9 after ~10 s) before `start web` | Otherwise the new server fails to bind :8787. |
| Real `lab evolve`, `validate`, `compare`, `run` on the real `lab-data/` spend trials: ask first. `--dry-run` and scratch `data_dir` are free | Every trial raises the luck bar for all later families on that study; it cannot be undone. |
| Never open the lockbox to "see if it works"; tests use synthetic panels | Each opening tightens the next (α/k); peeking spends the only clean test. |
| Promotion (`lab promote`) is always the user's call; the loop never promotes | Design decision in PLAN-EVOLVE.md. |
| Don't change gate thresholds, costs or universe in `lab.toml` after seeing a result; if changed, it's a new hypothesis | Moving the bar after looking is how backtests fooled the project before. |
| New strategy ideas go in `hypotheses.toml` first, then through `lab validate` | Pre-registration; the ledger counts them. |
| No network calls in learn/propose/compare; no new deps without a stated reason | Reproducibility (PLAN-EVOLVE.md §7). |
| Hyperliquid API: 1200 weight/min shared by IP with the running trader: pace bulk downloads | Rate-limit bans would stop the paper trader. |
| Public repo: scan staged files for keys before any push; never write secrets into memory | github.com/Ridnois/jev-trader is public. |
| Jev key lives in `../trading/.env.local`; from this worktree pass `--env-file ../trading/.env.local` | The worktree has no `.env.local`. |
| If a new-era promotion fails on paper, restore the era-1 archive before promoting anything else | H-ERA-2, recorded when the ledger was reset. |
| Git stash is shared with the main checkout: use WIP commits, not bare `git stash` | Worktree. |
