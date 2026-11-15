# What is running (verify before acting)

Last checked: 2026-11-14 06:27 UTC.

Both services run as transient systemd user units (`v3-trader`, `v3-web`; linger is on) since 2026-11-14 06:26 UTC. Before that, setsid children were killed when the launching agent session ended (both died ~06:22 UTC with no shutdown line). Check with `systemctl --user status v3-trader v3-web`.

## Paper trader
- `./target/lab/jev-trader --env-file ../trading/.env.local trade run`, started detached from the worktree; pid in `data-v3/trade.pid`, log `data-v3/trade.log`.
- Running since 2026-11-11 16:59 UTC with restarts; current process started 2026-11-14 06:26 UTC.
- Manage with `scripts/v3-services.sh start|stop|status trader`. Check health: `trade monitor`.
- Runs `strategies/*.json`, all unproven candidates (none passed the gate): trend, trend_fast (hand-written), g0019, g0030, g0034, g0044, g0045, g0046, g0050, g0078, g0084, g0121, g0150 (13 as of 2026-11-14 06:40 UTC; g0019-g0084 promoted 06:33-06:37 UTC through the dashboard's Promote button, which restarts the trader each time). g0078 is the same rule as g0034 (identical card); g0034 and g0043 share a rule id. 2026-11-15 01:10 UTC: 23 strategies after the user asked to promote every near miss: g0149, g0191, g0192, g0223, g0261, g0262, g0263 added with `lab promote --candidate` (files renamed from the family hash to their gNNNN label so book names match), plus g0137, g0153, g0195 promoted earlier. All are variants of one volume/breakout idea (hold 3d, stop 6%), highly correlated.

## Dashboard
- `next start` on :8787, pid `data-v3/web.pid`, log `data-v3/web.log`. Production build: UI edits need `pnpm exec next build` + restart (`JEV_WEB_REBUILD=1 scripts/v3-services.sh start web`).

## Evolve loop
- User crontab: `17 * * * * scripts/v3-evolve-hourly.sh` (since 2026-11-13): Binance fetch, then `lab evolve --auto`, flock, log `data-v3/evolve.log`.
- Budget per study is cumulative over the lineage (default 200; `binance-1h = 300`), max 20 trials per hourly run. Ticks go idle once a budget is spent.
- 162 generated families in `families/gen/` as of 2026-11-14. Last run 2026-11-14 06:18 UTC: nothing to promote.

## Main tree (`../trading`, branch main)
- Old bots (`run` news bot, `scalp`, old dashboard) stopped since 2026-11-11 17:25 UTC at the user's request. `data-hl/scalp_state.json` still holds an unclosed paper NEAR short that resumes if `scalp` restarts.
