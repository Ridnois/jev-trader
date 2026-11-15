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
- State only gets colour: `--pos` passed or up, `--neg` fatal or down, `--warn` short of the bar or provisional, `--info` sealed, opened or active. Charts use the five family colours. One exception: position side has its own pair, `--long` and `--short`, because direction is not a result.
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

- 240px sidebar: wordmark, seven destinations, How this works, engine footer with venue, money and read-only.
- 56px header: page title, engine state and the time of the last status write.
- Page body max 1320px, 32px gutters on desktop, 16px on phones.
- No second navigation strip. The four stations are a footer index (`StationIndex`) on station pages, and the full four-column lifecycle on Now and How this works. A duplicate strip was tried and removed: it restated the sidebar at the cost of 44px and became a 1002px scroller on a phone.
- Page order: question, state line, the explanation, then the evidence. On phones the state line follows the question, before the explanation.
- `PageHead` puts the question and state readout side by side from `lg`; below it the state sits between the question and the lede.

## Home (Now)

The home is a console, not a reading page. One answer line, then the four stations as an instrument rail: a mono value, its unit, an evidence bar where a denominator exists, and one note. Below it the paper book with a gauge per strategy, three book figures, four latest headlines and six prices. The explanation lives on How this works and on each station page; the home carries no definitions, no lede and no station index. Every rail cell is the link to its station. The sidebar is navigation only; station state lives in the rail, not in nav dots.

## Open positions

`/paper` is three things in order: **Runs**, the book, and what the page cannot tell you. Runs is one row per strategy (its tag, the test now, full paper days, drawdown against its band, the days the decision is expected in); a row opens, as a native `details` so it works on touch, into the Lab's verdict, the sequential chart (whole test or so far) and one line each for the card's forecast, parity and the spec. What every row shares is said once in the description, not per row: the kill and graduate lines when all cards share them, "all are unproven candidates", "no run has a call yet". A call pill appears on a row only once it is kill or graduate. Under a 48rem panel each row is the tag and one muted line. The page once had a card per strategy (about 620px each) restating the table; distilled 2026-11-14.

One component, `components/positions.tsx`, on Now and Paper run, so the two copies cannot drift. It answers **how is the book doing right now, and is anything in trouble?** Each position is valued at the recorder's latest mid, the way the engine's own equity line values it (`openBook` in `lib/v3/stations.ts`). The head is a mono state line, `+$170.21 open  nearest stop 9.0% DOGE`, with `$5,000 gross · −$400 if every stop fills` under it; `at_risk` is the loss against entry if every stop fills, never "notional". Below it, one disclosure line per strategy: its tag, the entry clause that opened its positions (`Short when it closes below the 4d low`), and the whole rule (`lib/v3/rule.ts`, read from the strategy file, or `trend`'s composed form) plus its hypothesis when opened; it works on touch, unlike a title. The strategy tag is `G<generation>/<family>·<variant>`: `G0/trend·4d` for a hand-written family and its lookback, `G12/g0045` for a generated one. Rows sort by risk: no price, then no stop, then distance to stop; a row in trouble (no price, no stop, or under 2% to its stop) gets an amber tint. Side is an arrow in its own colour (`--long` blue up, `--short` violet down): direction, never result, so neither green nor red. The layout follows the panel's own width (a container query, because the sidebar leaves a tablet about 440px): under 56rem each position is two lines, coin and Open P&L first, move, distance to stop and time held second; from 56rem a table of Coin, Open P&L, Move, To stop, Size, Held, Entry → mark, Stop, with the column words explained by Gloss and the "What these words mean" list. Strategy rows group under a tag subtotal when more than one strategy holds positions. A quote older than 120 s is not a mark: the row says `No price` in amber rather than counting it at entry.

## Lab

`/lab` answers **how close is any family to the paper run?** A family is a family: hand-written or bred by the loop, it faces the same gate, sits in the same ranking and is coloured by the same rule. Origin is an attribute in small print (`hand-written`, `generation 12`), never a grouping.

The state line is the distance itself (`1 check short`), or `N of M families passed` once something passes; the detail names the closest family and the checks it failed. Figures: families judged (and how many the gate stopped before the checks), how many are within 1 and within 3 checks, trials on the ledger, the lockbox.

**Distance to the goal** is the page's centre: every family in one gate map, ranked by checks failed (families with no distance last), the closest 12 shown and the rest one button away. One count line for all families says how many were still in at each check, so the column where it drops is what stands in the way. The last column, under the lockbox, is each family's distance (`1 short`). Checks are aligned by name, `–` where a family never ran one. Selecting a row (or a line in the chart) fills **one detail panel** for that family: checks measured against needs, the gate's notes, and the best in-sample rules collapsed behind a one-line summary. The calibration panel follows the chart.

**Out of sample, walk-forward** puts every family on one chart against calendar dates, one colour rule for all: ranked by final return, `color-mix(in oklch, var(--pos) p%, var(--neg))`, greenest highest and reddest lowest. Hand-written lines are 2.25px; generated lines are 1px at 45%, with the overall best and worst drawn heavier; no end labels (removed at the user's request), names come from hover; the generated median is dashed. The lockbox (panel start + design days → panel end) is a faint `--info` band labelled "Lockbox, sealed", so the reader sees why every line stops: walk-forward runs only on the design window, in whole 60-day folds. Hovering names the nearest line with its date, value and place; clicking selects it.

**Ledger eras.** When the ledger restarts (`era_start` in lab.toml), every verdict judged before it faced the old ledger's higher bar. Those families keep their reports but are flagged `prior`: they leave the ranking, the closest line, the within-1/within-3 counts and the Now rail, and sit in an **Earlier verdicts** table (closest first, with the trials on the ledger each one faced) that fills the same detail panel, tagged "Earlier era". On Generations they are hollow dots off the closest line. The walk-forward and return-per-generation charts keep them, because returns do not depend on the bar. Re-judging them is not offered: it would charge every rule again.

## Rules

`/rules` lists every rule on `lab-data/trials.jsonl` (one spec: a family's idea with every setting fixed), from every family, joined to the family's name, origin, verdict, distance to the goal (checks failed) and hypothesis statement. Its two jobs: find one rule, and see which settings keep winning. The lede says once that every number is in sample, after costs and the best of a search; numbers are neutral mono text, never green or red, so no row reads as a buy list.

**Which rule each window picked** (`components/walk-calendar.tsx`) sits between the settings panel and the table: the page's only view of rules through time, and the only one out of sample. A row per family in view (the page's filters drive it), a column per 60-day walk-forward test window on calendar dates, each cell the rule that window's fold picked and its out-of-sample bps a day. Cells are the one exception to neutral numbers on this page, because they are results, not rankings: muted `--pos`/`--neg` fills scaled to the view's 90th percentile of |bps|. A 2px tick marks a window where the family switched rules; a run without ticks is one setting kept being chosen. A window a fold left out is flat (faint fill). The top row counts families that made money per window, with a dashed half line, so a market-wide bad stretch reads as the market, not the rule. Rows rank by mean out-of-sample bps a day, top 12 then all. Selecting a family filters the page to it and opens one lane per picked rule, labelled only by the settings that differ. Selecting a cell opens that rule in the table and scrolls to it; an opened rule highlights its windows and dims the rest. The readout line under the grid replaces tooltips and is reached by keyboard: the grid is one tab stop, arrows move, Enter opens. The lockbox is an untested `--info` column. Data: `getRules().walk`, joined from each report's `walk.folds` and `fold_picks`.

Controls: free-text search (family, settings, idea), segmented study / origin / result filters, a "Picked only" switch (the family's in-sample pick or a walk-forward fold's pick). Above the table, the comparison strip: across several families, "What the families say" ranks them by mean bps a day with a bar around zero and filters to one on click; with one family, "What <family>'s settings say" shows each varied setting value by value (mean bps a day, a bar around zero, rules that tried it), the setting with the widest spread first, fixed settings listed after. The table is sortable by trades, bps a day, Sharpe (default) and family distance, 50 rows at a time; a row expands in place to the idea, the full spec id, the family's verdict and distance, its rank in the family, fold picks and links. Recombined settings are translated: `b_b_look` reads "ingredient 3 · look", ordered by depth.

## Generations

`/generations` answers one question: **is the loop getting closer?** It reads `lab-data/lineage.jsonl`, `lab-data/reports/evolve/(dry-)gNNN.json`, every `auto-<time>.json`, the user's crontab line for `scripts/v3-evolve-hourly.sh` and the ticks in `data-v3/evolve.log`.

**Three words, nested.** The hourly cron starts a **run**; a run repeats **generations** until a stop rule; a generation proposes up to four **families** for the gate. The page never says cycle. The goal is stated once and in the same words everywhere: one family that fails none of the gate's checks. "Checks failed" is the distance to it, glossed with its misreading (checks are not equal).

Order: question and state (`0 of N generated families passed this era`, the closest family and how many checks short, the next start in the meta), **Distance to the goal**, **What stands in the way** (each check, families it stopped overall and in the latest generation; a 0 there is the only green, because it is progress), **Every generation** (reports grouped by run, newest run first, all collapsed), then two closed disclosures: **How the loop runs** (its summary line carries the live facts: schedule, runs so far, why the last stopped, trials this era; inside, the four cells, the trial budget and the glossary) and the **Family tree**. Explanation sits last because the reader is a returning operator; distilled 2026-11-15.

**Distance to the goal** answers "is the loop closer?" in two parts. The chart (HTML dots, SVG step line) draws only this era's scored generated families: a faint dot per family by checks failed, 0 at the top as a dashed `--pos` goal line, and a **record step line**, the closest any family has come so far, which can only fall. Only record-setting families are solid and labelled. The x axis starts at the era's first generation with ticks every 10. A scope line under it says what is drawn and what is not (too few trades to reach the checks, judged against the old ledger's bar, hand-written families on the Lab), because "am I looking at every family?" is the first question the chart raised. Replaced 2026-11-15: a closest-per-generation zigzag over 83 columns with prior-era hollow dots and a truncated run-label strip; each generation is a fresh draw of up to four families, so the zigzag was noise and the record is the signal.

**How short is short** follows inside the same panel: every family this era one or two checks short (`near` in `getGenerations`, from the Lab's reports, hand-written included if any), one row each with the checks it missed, the measured value against the need (lockbox reduced to its p against the threshold), the trials on the ledger when it was judged, and when. The lead names which checks stop the one-short families and, for checks whose bar moves (deflated Sharpe with every trial, lockbox with every opening), says so: an equal count of 1 can be drifting further away.

Display rules kept: no fact printed twice, repeated machine prose hoisted once, in-sample and paper comparisons never coloured, verdict tallies in words (`3 rejected · 1 not enough evidence`, never "better than rejected").

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
