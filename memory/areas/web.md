# Dashboard (`web/`)

- Next 16 + Tailwind 4 + shadcn (radix-nova) + magicui number-ticker; read-only over `data-v3/`, `lab-data/`, `strategies/`, `hypotheses.toml`.
- Pages include Overview, Paper, Lab (Gate Line Map), Generations, Markets. Design rules: `web/DESIGN.md`; direction contract `.impeccable/surfaces/`.
- Look: plain dark, Vercel dashboard as the quality bar. No themed worlds.
- Production build: `pnpm exec next build` then restart web. npm/pnpm installs can take ~10 min here.
- 21st.dev registry needs an API key the machine lacks: install the same components from their source registries (e.g. `magicui.design/r/<name>.json`).
- Screenshots: headless browsers fail on this machine; use Claude in Chrome. Chrome window can't be resized (1536x774 @1.25 DPR): test phone widths with same-origin iframes.
- Copy terms through `<Gloss id>` from `web/lib/glossary.ts`.

## Lab: gate verdict vs. your approval (2026-11-14)
- Lab shows an "Approved by you" tag next to the gate's verdict for any family on paper as `--candidate`. It never replaces the gate verdict: the gate only says PROMOTE_TO_PAPER / REVISE / REJECT / INSUFFICIENT_EVIDENCE. Added at the user's request.
- Matching is by strategy name first, then by rule id, because two generated families can share a rule id (g0034/g0043).
- `scripts/v3-services.sh stop web`: next-server can ignore SIGTERM, so stop falls back to `systemctl --user stop v3-web`.
- Family counts (2026-11-14): Lab, Rules and Now count report files (hand-written + generated); Generations counts the lineage (generated only, so it says "generated families" and is lower by the hand-written count). The lineage holds the same rule id proposed twice (g0034/g0043, c4985153e362d); `getGenerations` keeps one proposal per id (last name wins, as in `getLab`), or every descendant is counted twice (was 220 vs 146).
