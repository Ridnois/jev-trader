---
version: 1
slug: "web-app-page-tsx"
primary_target: "web/app/page.tsx"
related_targets: []
---

# v3 dashboard surface brief

Scope: `web/`, the Next.js dashboard for jev-trader v3 (branch `v3`). Mode: Operate. Read-only: it reads `data-v3/`, `lab-data/`, `strategies/` and `hypotheses.toml` from disk and has no controls, because v3 places no real orders and has no pause. Desktop tab left open all day; also usable on a phone. Replaces the departure-board dashboard for v3 (the old `jev-trader dashboard` page still belongs to v1).

User answers (2026-11-11): chose the category standard over the dealt directions; quality bar is the Vercel dashboard; the Gate Line Map (declined-to-alternate challenger) is kept as the Lab page's main figure. Stack pinned by the user: Next, shadcn, 21st.dev components.

Pages: Overview (engine, paper run, sequential test, latest headlines, markets), Catalysts (scored headlines, gates, markouts), Lab (gate line map, verdicts, per-family checks, walk-forward, trial ledger), Jev probes (fade_judge scores, hypotheses), Markets (20 perps from the 1-second recorder).

## Direction contract

THESIS: The standard dark product dashboard, played straight at Vercel's craft level, for a system whose honest headline is that nothing passed. It refuses invented P&L drama: no equity hero when there is no equity history, and every number names paper, record mode, or research.

OWN-WORLD: Near-black ground (#0A0A0A), raised panels (#111111 to #171717) separated by 1px hairlines (#262626), white and grey text in three steps, Geist for everything with tabular numerals, Geist Mono only for rule specs and commands. Colour is state only: green passed or up, red fatal or down, amber short of the bar or candidate, blue sealed or informational. Five family line colours appear only on the Lab map and its legend. shadcn components, 8px radii on panels and 6px on controls, no gradients, no glow.

STORY: The operator sees in one glance that the engine is alive and paper-only, where the trend candidate stands between kill and graduate, and what Jev scored most recently. On Lab they read off the map which check stopped each family and that the lockbox is sealed.

FIRST VIEWPORT: Left sidebar 240px (wordmark with v3 tag, five nav items, engine status and "Paper only" at the foot). Header row with page title and the time of the engine's last status write. Four stat panels (Engine, Paper equity, Sequential test, Headlines scored). Below: the trend candidate panel with the sequential-test chart at left two thirds, latest scored headlines at right third.

FORM: The category standard (the standing exit), chosen by the user over the assigned Field Trial and the pick. seed:85f4e18b908ed6c5. Signature move: the Gate Line Map on Lab, each family a coloured line through the gate's checks, solid until the first fatal check and dotted after it, with the lockbox as a closed terminus. Motion: figures tick to new values in place; new headlines slide into the list; nothing else moves.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
