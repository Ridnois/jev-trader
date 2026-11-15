---
name: project-memory
description: Shared memory for the jev-trader v3 project, kept in the repo so every agent (Claude Code, Hermes, others) starts with the same facts. Use at the start of any task in this repo, when the user uses project shorthand (G12/g0045, era 2, lockbox, REVISE, the loop, H-ERA-2...), before touching running processes, the ledger, strategies/ or data-v3/, and at the end of any task that changed what is running, a decision, or a rule. Also when the user says "remember", "note that", "from now on", or asks what something means.
---

# Project memory

Two tiers, both plain markdown in the repo:

```
AGENTS.md              hot cache: under ~100 lines, read on every task
CLAUDE.md              imports AGENTS.md for Claude Code (do not duplicate content there)
memory/
  glossary.md          every term, tag, ID and nickname -> meaning
  guardrails.md        what agents must never do here, and why
  state.md             what is running right now, where, since when
  decisions.md         dated log of operator decisions (newest first)
  areas/lab.md         lab, gate, ledger, evolve loop
  areas/trade.md       paper trader, strategies, Jev
  areas/web.md         dashboard
```

## Read (start of a task)

1. Read `AGENTS.md`. It covers most requests.
2. Unknown term or ID: `grep -ri "<term>" memory/` and then the code/docs (`DESIGN-V3.md`, `PLAN-EVOLVE.md`, `hypotheses.toml`, `web/lib/glossary.ts`).
3. Before acting on running processes, the ledger, the lockbox, `strategies/` or `data-v3/`: read `memory/guardrails.md` and `memory/state.md`.
4. Memory is a snapshot. Before relying on a claim about what is running or what a file contains, check it (`ps -C jev-trader`, `scripts/v3-services.sh status`, `crontab -l`, `git log`). If memory is wrong, fix it in the same task.
5. Still unknown: ask the user, then write the answer down.

## Write (end of a task, or when told "remember")

Write when something changed that the next agent cannot cheaply rediscover from the code or git:

| What changed | Where |
|---|---|
| A process started/stopped, a cron added, a port moved | `memory/state.md` (+ AGENTS.md "Running now") |
| The user decided something or set a preference | `memory/decisions.md` (+ AGENTS.md if it affects most tasks) |
| A new "never do X" | `memory/guardrails.md` and the AGENTS.md short list |
| New term, family ID worth knowing, nickname | `memory/glossary.md` |
| How a subsystem works / gotcha | `memory/areas/<area>.md` |

Rules:
- **Absolute dates and UTC times**, never "yesterday" or "today". Say who asked ("at the user's request").
- **Public repo.** Never write keys, tokens, emails, account IDs, balances tied to a person, or the contents of `.env*`. Refer to secrets by location only ("Jev key: `../trading/.env.local`").
- Record the **why** with each decision or rule, so an agent can judge edge cases.
- Don't copy what git or the code already records (diffs, function lists). Point to the file instead.
- Edit the existing entry instead of appending a contradicting one. Delete what is no longer true; move finished history to `decisions.md`.
- Keep `AGENTS.md` under ~100 lines: promote what most tasks need, demote what went stale.
- Memory files are working-tree files: commit them only when the user asks for a commit, and scan them for secrets before any push.

## Shorthand decoding

Decode before acting. "promote g0045" means `lab promote --family families/gen/g0045.toml` with every guardrail that applies (promotion is the operator's call, not the loop's). "Restart the trader" touches a guarded process: confirm first. If two meanings are possible, ask.
