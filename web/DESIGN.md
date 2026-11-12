# v3 dashboard design record

The design record for `web/`, the read-only Next.js dashboard on branch `v3`. Product truth lives in `../PRODUCT.md`; this file records how the surface is built and why.

## Surface

Mode: Operate. The reader is the bot's author, opening a tab to answer one question: **what is happening, and does anything need me?** Everything else on the page exists to answer that or to explain the evidence behind it.

The honest headline is that nothing has passed. The design must make "nothing passed" read as a result, not a failure or a missing loader.

## The idea

Four stations, one decision:

```
News recorder ─┐
               ├─→ Lab ─→ Paper run ─→ your decision (not a station)
Jev tests ─────┘
```

- **Lab**: is the rule real?
- **Paper run**: does the live market agree?
- **News recorder**: is there money in the news?
- **Jev tests**: can Jev read a market?

The first two form the decision line. The recorders feed the Lab. The site never arms, pauses or orders; that decision is made in the terminal.

The organising device is the **state line**: every station returns a short sentence from a closed vocabulary, with a noun and a unit, set in Geist Mono at 22-26px. The reader should be able to read the four states and stop.

Examples:

- `0 of 7 families passed`
- `2 strategies on paper, 0 full paper days`
- `2 of 20 evidence days, 24 of 30 triggered headlines`
- `0 of 4 questions show skill`

## World

- Ground `#0a0a0a`, panels `#111111`, popovers `#171717`, hairlines `#242424`.
- Text: `#ededed` primary, `#a1a1a1` secondary, `#8f8f8f` faint.
- State only gets colour: `--pos` passed or up, `--neg` fatal or down, `--warn` short of the bar or provisional, `--info` sealed, opened or active. Charts use the five family colours.
- Geist for prose, Geist Mono for state lines, prices, spec ids and commands. Tabular numerals everywhere.
- Radii: 8px panels, 6px controls. No gradients, glow, texture, decorative particles, 01/02/03 counters or eyebrow labels.
- Dark only, by decision, not by OS preference.

## Type scale

| Size | Use |
|---|---|
| 26px / 15px | Page question, mono state line on station pages (22px on phones) |
| 19px | Figure values |
| 15px | Section headings on the guide, pipeline station state |
| 13px | Body, table cells, panel descriptions |
| 12px | Secondary labels, chart ticks |
| 11px | Meta lines, micro labels, verdict label row |

## Layout

- 240px sidebar: wordmark, six destinations, How this works, engine footer with venue, money and read-only.
- 56px header: page title, engine state and the time of the last status write.
- Page body max 1320px, 32px gutters on desktop, 16px on phones.
- No second navigation strip. The four stations are a footer index (`StationIndex`) on station pages, and the full four-column lifecycle on Now and How this works. A duplicate strip was tried and removed: it restated the sidebar at the cost of 44px and became a 1002px scroller on a phone.
- Page order: question, state line, the explanation, then the evidence. On phones the state line follows the question, before the explanation.
- `PageHead` puts the question and state readout side by side from `lg`; below it the state sits between the question and the lede.

## Home (Now)

The home is a console, not a reading page. One answer line, then the four stations as an instrument rail: a mono value, its unit, an evidence bar where a denominator exists, and one note. Below it the paper book with a gauge per strategy, three book figures, four latest headlines and six prices. The explanation lives on How this works and on each station page; the home carries no definitions, no lede and no station index. Every rail cell is the link to its station. The sidebar is navigation only; station state lives in the rail, not in nav dots.

## Motion

One authored sequence in the product: on Jev tests, the probe flow line draws across Hide, Ask, Replay and Score in about two seconds, lighting each node as it reaches it. Play again restarts it; `prefers-reduced-motion` shows the finished state with no movement. No other page-load choreography and no looping decoration.

## Components

- `Verdict` (`components/bits.tsx`): the state line. Label row (station, freshness, link), mono line at 22-26px, one sentence of meaning, optional children for figures. No card.
- `Figures`: one band of 3-4 figures separated by hairlines, not four floating cards. An item may carry a 2px ratio rule, filled to n of N and turned `--pos` when the bar is met.
- `Panel`: the only card. Header, optional description and action, flush mode for tables.
- `PipelineSpine`: the four stations as one four-column band in the order evidence -> gate -> paper; on phones it stacks. The terminal decision sits below the band, not as a fifth station.
- `StationRail`: the home's four-cell instrument rail. Value, unit, optional 3px evidence bar, one note; each cell is a link. It replaces the prose pipeline on Now.
- `StationIndex`: all four stations with their state lines, used at the foot of station pages. Works at 390 without sideways scrolling.
- `ProbeFlow`: the Jev-tests four-step flow (Hide, Ask, Replay, Score). A motion line draws once, each node lights as it passes, and Play again restarts it. Reduced motion renders the final state with no movement.
- `Gloss` and `GlossList`: one typed record per term in `lib/glossary.ts`, shown on hover, focus and in a page disclosure. The wording never forks.
- `Tag`, `Mark`, `Side`, `Delta`: state and verdict marks. `Delta` is for returns only; a signed score such as Jev impact is neutral text.
- Charts: `SequentialChart`, `GateLineMap`, `OosChart`, `Reliability`, `Sparkline`. Each has an `aria-label` stating the current reading.

## Copy rules

1. Every count has a noun and a unit: `2 rules`, `2 of 20 evidence days`, `210 days held back`, not `2 on paper` or a bare `548`.
2. One condition, one wording. The state strings in `lib/v3/stations.ts` are the source; pages render them, not paraphrases.
3. Provisional data loses its colour until the bar is met. The news recorder shows means in neutral text and a Provisional tag until 20 days and 30 triggers.
4. A rejected family that still runs on paper says so: the Lab's rejection stands; the paper run tests it forward.
5. A card is a forecast, not a result. A paper run is not live. A probe is not a strategy. Each page says what it cannot tell you.
6. Machine records (spec ids, hypothesis results, report values) are shown verbatim; the plain verdict goes above them.
7. No em dashes in UI copy. No edge claims. No number is presented without its mode (paper, record, research, synthetic).

## Accessibility

- Skip link first in the DOM; one `main` landmark (the SidebarInset); `#main` is the focus target.
- Every state has a word plus a dot or shape; colour is never the only carrier.
- Glossary terms are buttons: they open on hover and on keyboard focus.
- Contrast: all text meets WCAG AA against its effective background (checked on Now, Lab, News and Paper at 1440).
- Charts carry a current-reading `aria-label`; tables use real `<th>`/`<td>`; truncated text keeps a `title`.
- Motion is limited to the engine ticker and hover transitions. Nothing moves on scroll.

## Known debt

- The Lab gate map's column labels are rotated and small; the Verdicts table below is the readable alternative.
- Parity comes from `data-v3/monitor.md` as parsed text, not recomputed.
- Dense tables scroll sideways inside their panel on a phone; no stacked card view yet.
- The news table keeps only the newest 200 scored headlines in view; totals live in the figures band.
- There is no first-run tour. Onboarding is the always-visible state line, the guide, and empty states that name what will arrive.

## Verify

```sh
cd web
pnpm exec tsc --noEmit
pnpm exec eslint .
pnpm build
pnpm dev --port 3000      # or JEV_WEB_REBUILD=1 ../scripts/v3-services.sh start web
```

Then at 1440x900 and 390x844: no horizontal page overflow, every route 200, the state line readable in the first screen, and skip link to `#main` first in tab order.
