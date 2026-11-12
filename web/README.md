# jev-trader v3 dashboard

A read-only Next.js page over the v3 files: it reads `../data-v3/`, `../lab-data/`, `../strategies/`, `../lab.toml` and
`../hypotheses.toml` on every request and has no controls. v3 places no real orders.

| Page | Shows |
|---|---|
| Overview | engine health, paper equity, the sequential test of each strategy on paper, latest scored headlines, markets |
| Catalysts | every headline Jev scored, its three checks, and its markouts after costs |
| Lab | the gate line map, verdicts, each family's checks, walk-forward curves, gate calibration |
| Jev probes | probe scores per question and the hypotheses about Jev |
| Markets | the 20 perps from the 1-second recorder |

## Run

```sh
../scripts/v3-services.sh start        # trader (if not running) + dashboard on http://localhost:8787
../scripts/v3-services.sh status
../scripts/v3-services.sh stop web     # or: stop trader
JEV_WEB_REBUILD=1 ../scripts/v3-services.sh start web   # after changing the code
```

Development: `pnpm dev` (port 3000). `JEV_V3_ROOT` points the page at another worktree; `JEV_WEB_PORT` changes the port.

## Layout

- `lib/v3/server.ts` reads the files (`getLive`, `getLab`); the tick reader only reads bytes appended since the last poll.
- `app/api/live` serves the live snapshot; `components/live.tsx` polls it every 2 s and pauses while the tab is hidden.
- `components/charts.tsx` holds the hand-drawn SVG charts; `components/ui/` is shadcn (plus magicui's number ticker).
