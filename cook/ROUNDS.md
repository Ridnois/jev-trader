# Cook ledger: jev-trader v3 dashboard legibility

Started: 2026-11-12 15:43 -03
Time: floor 8h | cap none
Bar: comprehension 9.0 | state 9.0 | hierarchy 9.0 | craft 9.0 | distinctiveness 8.5 | copy 9.0 | a11y 9.0 | mobile 9.0 (0-10)
Calibration offset: self-scores run about +2 high against the first fresh-context critics; apply -2 until a user estimate lands.
Usage: no readable meter; rounds of 30-90 min
Gate: stills approved no | final build not started
Best so far: round 6, home console, `cook/stills/r06/`

## Decisions

- 15:43 Intake timed out; took the recommended defaults (keep shell, always-visible onboarding, 8h floor, no modal, split paper page out). Logged in BRIEF.md.
- 15:50 Read the Rust producers and every page before writing copy, because explanatory sentences are claims about product state.
- 15:52 No new dependencies; use the shadcn primitives already installed.
- 15:52 Keep `SequentialChart`, `GateLineMap`, `Reliability`, `Sparkline`, `RangeBar`; the problem is what surrounds them, not the figures.
- 15:54 Add `/start` and `/paper`; rename nav to plain names; no controls added.
- 16:20 The first critics put state legibility lowest (median 6) and distinctiveness at 4. Round 2 makes the state line the hero and removes the duplicate strip, rather than adding more cards.
- 16:35 Removed the stage strip entirely. It restated the sidebar character for character and became a 1002px scroller on a phone; the footer StationIndex carries the same orientation and works at 390.
- 16:40 A rejected family that runs on paper is a deliberate product state, not a bug; the Paper page now says the rejection stands and the run tests it forward.

## User feedback inbox

- 15:43 "the web app is hard to understand and too generic, i want you to make this easy to understand to just onboarded users. understand how this app works and improve the interface so users understand every step and the corrent state of the app" | round 0 | open

---

## Round 0, baseline

Time: 15:43 to 16:00 | total elapsed 0.3h
Focus: measure the incumbent before changing it.

Measured (browser at 1440x900, dev server):
- Overview: 1,699 px tall, 2,657 chars, no horizontal overflow.
- Catalysts: 15,692 px tall, 30,725 chars, 200 rows rendered.
- Lab: 3,545 px; Probes: 2,498 px; Markets: 1,106 px.
- Nav: 5 nouns, no lifecycle indication. Sidebar has no explanation of the product.
- Family name map covers 5 of 8 (`hvt`, `ensemble` render raw).
- Catalysts plural bug renders "2 | day | s".

Own score (logged): comprehension 4, state 4, hierarchy 4, craft 6, distinctiveness 3, copy 5, a11y 6, mobile unknown.

Next focus: the home screen and the lifecycle model.

---

## Round 1

Time: 16:00 to 16:15 | total elapsed 0.6h
Focus: the first screen and the lifecycle model.

Changes:
- State model `web/lib/v3/stations.ts`, glossary `web/lib/glossary.ts` + `components/gloss.tsx`.
- Nav rebuilt; stage strip under the header; PipelineSpine on Now and Start.
- Now rewritten; Paper run split out; Lab, News recorder, Jev tests and Markets given state bands, figures and limits.
- Server: Lab summary in the live snapshot, 5 s Lab cache, verdict dates, parity parsed from monitor.md, news day counts.
- Stills: `cook/stills/r01/`; before/after `cook/compare/r01/now-before-after.jpg`.

Measured:
- No horizontal overflow at 1440 or 390. Heights: Now 2,041; Start 2,328; Paper 2,571; Lab 3,820; News 4,585 (was 15,692); Jev 2,906; Markets 1,579.
- 0 WCAG AA contrast failures on Now, Lab, News, Paper. tsc, eslint and build pass. 8787 restarted.

Fresh-context critics, round 1 (raw, before calibration):
- Newcomer (comprehension / state / hierarchy / copy): Now 7/6/6/6, Start 7/4/5/5, Paper 5/4/5/4, Lab 6/6/6/5, News 6/7/5/5, Jev 4/5/4/4, Markets 6/7/6/5.
- Design: craft 6.5, distinctiveness 4, density 6, typography 5, colour 7, mobile 4; overall 5.5.
- Weakest: state legibility. "The site has no shared grammar for a status"; counts without nouns; the same condition in four vocabularies; Paper contradicts itself (rejected vs running); Jev's amber chip argues with its own banner; the strip duplicates the sidebar and is a 1002px scroller at 390.
- Signature suggestion: set the verdict as a mono state line at 24-28px with the state colour on that line only.

Stop check: bar no | floor no | stills approved no -> round 2.

---

## Round 2

Time: 16:20 to 16:55 | total elapsed 1.2h
Focus: the round 1 critic findings, in their order: state legibility, contradictions, then visual identity.

Changes:
- `Verdict` replaces every state band: fixed label row, Geist Mono state line at 21-24px with the state colour on the line only, one sentence of meaning, optional figures. No card.
- State strings rewritten with nouns and units: `0 of 7 families passed`, `2 rules on paper, 0 full days`, `2 of 20 evidence days`, `0 of 4 questions show skill`, `lockbox holds back 210 days`.
- Now opens on a two-line readout: `0 of 7 families passed · 2 rules on paper, 0 full days`, then "Engine running. Nothing needs you." with the news and Jev shortfalls in the same sentence.
- Stage strip deleted. `StationIndex` at the foot of every station page: all four names, states and links, no horizontal scroll at 390. Sidebar keeps names and state dots only; the bare `548` badge is gone.
- Paper: duplicate "reality check check" fixed; "the Lab's rejection stands" sentence added; decision horizon shows both numbers (685 to 973 days, earlier if no edge); chart caption names the log-likelihood axis; current point labelled "now".
- News: means lose all colour and carry a Provisional tag until 20 days and 30 triggers; headers say `Triggered, net bps` / `n` / `All scored, net bps` / `n`; tab counts now say what is in view; footnote names the 200-row cache and the 30-trigger bar.
- Jev: partial ranking signals explained in the verdict ("2 questions show a ranking signal but do not beat the shuffle test"); the amber chip is neutral; meta says `2 probes · 4 questions`.
- Markets: headline matches the page ("Are the markets alive while the paper run waits?"), the lead states how many positions the paper book holds, tick age is seconds rather than a repeated "Live", prices and changes are mono, and the footnote explains the range dot and that sparkline colour is 3 hours, not today.
- Lab: legend above the map, selected family gets a 3px colour bar, the panel names the family count.
- Type scale consolidated to 11/13/15/19/24; removed unused `Stat` and `PageLead`; skip link first in the DOM; one `main` landmark; `#main` focus target.
- Wrote `web/DESIGN.md`: world, type scale, components, copy rules, accessibility, known debt, verification.

Stills: `cook/stills/r02/` (seven at 1440, seven at 390) | Before/after: `cook/compare/r02/`

Measured:
- Heights: Now 1,825 (round 1: 2,041); Start 2,306; Paper 2,767; Lab 3,932; News 4,692; Jev 3,026; Markets 1,572.
- At 390: no horizontal page overflow on any route. Now 3,460; Start 3,594; Paper 4,527; Lab 4,918; News 6,766; Jev 5,342; Markets 1,735.
- 0 WCAG AA contrast failures on all seven routes at 1440.
- Nav targets 36px tall in the phone drawer. Skip link is first in tab order and moves focus to `#main`. One `main` element.
- `pnpm exec tsc --noEmit`: pass. `pnpm exec eslint .`: pass. `pnpm build`: pass. 8787 rebuilt and restarted; `/` contains the new readout.

Scores, round 2 self-scored (raw; subtract 2 for calibration): comprehension 8.5, state 8.5, hierarchy 8.0, craft 7.5, distinctiveness 6.5, copy 8.5, a11y 9.0, mobile 8.0.
Fresh-context critics for round 2 are running on the r02 stills; their medians replace these.

Top gaps, own review:
1. The pages are still built from rounded cards below the readout; the instrument idea lives in the verdict, not yet in the panels.
2. Lab is still the densest screen: 13 rotated check labels and a 7-row line map before the readable table.
3. News is 6,766 px at 390; the table is usable but long.
4. The 200-headline cache means "About a coin" and "Triggered" can be smaller than the totals in the readout, even with the footnote.

Stop check: bar no | floor no | stills approved no | final checked no -> next round after round 2 critics
Next focus: critic round 2, then panel chrome and the Lab check list.

---

## Round 3

Time: 16:20 to 16:50 | total elapsed 1.1h
Focus: the round 2 critics' weakest area, "the counts do not reconcile with what is on screen", then the empty chart and the page band.

Changes:
- `PageHead`: question and lede left, state readout right, on every page. It fills the 500-700px empty band and puts the numbers in the first 300px. On phones it stacks.
- The object on paper is called a **strategy**, not a rule: `2 strategies on paper, 0 full paper days`. The Now lead and the Paper lead both say a rejected family may run as a labelled candidate.
- Paper: both strategies are named above the fold with anchors. The 870x310 chart is hidden until there is a full paper day; the gauge and the decision horizon lead instead, with a "Show the chart layout" button for the curious. Decision is a range (685 to 973 days), not one number.
- Counts now reconcile or say what they are: `24 triggered headlines · markouts recorded on 2 days`; news verdict shows both legs (`2 of 20 days · 24 of 30 triggered`); table headers are `Triggered with a markout, net bps` / `n markouts` / `All headlines with a markout, net bps` / `n markouts`; the panel says a good result is positive and better than the all-headlines column.
- Jev: a probe index at the top (`Fade judge, 2 questions`), `2 probes · 4 questions` in the meta, a sentence defining skill (rank above chance **and** beat the shuffle test), neutral AUC/IC/Brier values, "Ranks, fails shuffle" instead of "Signal, no payoff", and calls vs measured outcomes in the header.
- Colour is state only: Long and Short are neutral, every market change is neutral signed text, every sparkline is neutral, and Jev's metrics lost their signed colours. P&L stays coloured because it is the paper book's result.
- Markets: PageHead with recorder status (`20 of 20 live`, slowest tick), headers `1h %` and `Today %`, `Today's range, dot = now`, and the redundant "all ticking" line is gone.
- Lab: the readable Verdicts table now comes **before** the line map; the map is "the same result, drawn as a line". Lockbox figure shows the per-family range (11 to 210 days) instead of a max, and the revise count is included.
- Start: the lifecycle diagram runs in guide mode, showing what each station is for rather than repeating Now's live numbers.
- Money after costs is printed with `usd()`, not a bare number; "hundreds of days of paper days" fixed.

Stills: `cook/stills/r03/` (seven at 1440, seven at 390)

Measured:
- Heights: Now 1,880; Start 2,222; Paper 2,311 (round 2: 2,767); Lab 3,839; News 4,699; Jev 2,982; Markets 1,585.
- At 390: no horizontal page overflow. Now 3,530; Start 3,366; Paper 3,940; Lab 4,836; News 6,773; Jev 5,494; Markets 1,778.
- Paper renders no chart at 0 full days (verified: no `svg[aria-label*="Sequential test"]`, gauge and button present).
- tsc, eslint and build pass. 8787 rebuilt and restarted; `/` contains `2 strategies on paper`.
- Contrast: 0 WCAG AA failures on all seven routes after the neutral colour pass.

Scores, round 3 self-scored (raw; subtract 2 for calibration): comprehension 9.0, state 9.0, hierarchy 8.5, craft 8.0, distinctiveness 7.0, copy 9.0, a11y 9.0, mobile 8.5.
Round 3 critics are running on the r03 stills and the current source; their medians replace these.

Top gaps, own review:
1. Distinctiveness still rests on the state line and the gate map; the panels are still rounded cards.
2. News at 390 is 6,773 px, and the triggered/all split still needs the 200-row cache footnote to reconcile.
3. The Lab line map's 13 rotated check labels remain the densest thing in the product.
4. The `Verdict` line on Now runs to three lines in the right column on a narrow desktop; it could be two.

Stop check: bar no | floor no | stills approved no | final checked no -> next round after round 3 critics
Next focus: round 3 critic findings, then a single gate-rail component and the Lab labels.

---

## Round 4

Time: 16:50 to 17:00 | total elapsed 1.3h
Focus: three fresh critics on r03 found contradictions I had introduced, not taste. This round is a correction pass plus the design critic's signature move.

Critic scores at r03: newcomer comprehension 5-7, state 4-6, hierarchy 4-6, copy 4-6; design craft 6.0, distinctiveness 5.0, density 4.5, typography 5.5, colour 6.0, mobile 5.0. Both said progress cannot be read as progress.

Changes:
- Jev calls: "1,144 calls, 1,784 with a measured outcome" was wrong (one call answers two questions). Now: "1,144 calls, each answering 2 questions; n below is the entries per question."
- Probe index carries the venue: "Fade judge on Hyperliquid, 2 questions" and "Fade judge on Binance, 2 questions", so two probes are no longer identical.
- News: every markout cell is neutral until 20 evidence days and 30 triggered headlines, matching the sentence that claims no colour. The failed checks lost the red fatal X and use the amber "short of the bar" mark. Triggered is a neutral tag, not the Lab's green "Validated".
- One readiness bar in `stations.ts`: 20 evidence days and 30 triggered headlines, everywhere. The old string claimed a first read at 20 days alone.
- Day 0 vs zero: every caption now says "at day 0 nothing has been counted; later, 0 means the counted days balance". The paper card said "by construction" beside a caption that said "no evidence either way".
- Paper provenance: "Fixed when this strategy entered the paper book", not "promoted" for a family the same card says was rejected. "Unproven candidate" is the one name on Now, Paper, the tag and the station role.
- Drawdown has one name: "Drawdown against the band", "Band for the days elapsed: 25.6%", "of the 25.6% band".
- Lab: "Trials on the ledger" (not rules), "Stopped at" in foreground, family identity marks are neutral so only verdicts carry state colour, and a line states that a check passes when Measured meets Needs.
- Jev is defined in the glossary and glossed on News, Jev tests and the guide.
- Markets: "All 20 coins live", and the slowest tick names the coin (`on DOGE`), so an outlier is not invisible.
- Pipeline is one four-column band (News, Jev, Lab, Paper) with equal heights; the five unrelated heights and the dead feeder arrow are gone. The decision is one sentence below the band, not a fifth station.
- State line is 26px on desktop (22px phone), not the same size as the page question.
- News figures carry the evidence rule: a 2px bar filled to n of N, `--pos` only when the bar is met. Triggered 24/30 reads 80%; evidence days 2/20 reads 10%.

Measured:
- Heights: Now 1,647; Start 2,041; Paper 2,315; Lab 3,904; News 4,752; Jev 3,026; Markets 1,589. Now is 233px shorter than r03.
- Four pipeline cells share top 417 and height 220 (was five different heights, 163/184/220 gaps).
- At 390: no horizontal overflow (paper/lab/news/jev were 417 before the `StationIndex` min-width fix). Now 3,490; Start 3,262; Paper 3,952; Lab 4,961; News 6,908; Jev 5,505; Markets 1,770.
- Zero WCAG AA contrast failures on all seven routes at 1440.
- News tables: 0 red/green text cells, 86 neutral, 0 "Fatal" marks, 88 "not met" marks.
- tsc, eslint and build pass. 8787 rebuilt; `/` serves the r04 string.

Scores, round 4 self-scored (raw; subtract 2): comprehension 9.5, state 9.5, hierarchy 9.0, craft 8.5, distinctiveness 7.5, copy 9.5, a11y 9.0, mobile 9.0.

Top gaps, own review:
1. Distinctiveness is still one device (the state line + evidence rule), not a system. The panels are still rounded cards.
2. The Lab line map's 13 rotated labels remain the densest object; it now sits below a 390px table on a phone.
3. News at 390 is 6,908px. It is the longest page and the one with the least to say until 20 days.
4. `StationIndex` truncates the long readiness sentence at 390, so the foot index says less than the page.

Stop check: bar no | floor no | stills approved no | final checked no -> next round on r04 critic findings
Next focus: run critics on r04; if the weakest read is still "progress", replace the Lab map's labels and cut the News page by moving the per-headline table behind a disclosure.

---

## Round 5

Time: 17:00 to 17:35 | total elapsed 1.9h
Focus: the r04 critics' contradictions. Newcomer r04 scores: Now 7.25, Start 6.0, Paper 5.5, Lab 6.0, News 5.5, Jev 4.5, Markets 5.75. Design: craft 6.0, distinctiveness 5.5, density 6.0, typography 5.0, colour 6.0, mobile 5.5.

Changes:
- Start no longer says "The Lab is the only door" beside a Paper page that runs rejected families. It says the Lab judges what is worth running, a rejected family can run on paper as an unproven candidate, and nothing reaches live money without it. The stale "fifth block, in the right column" is gone.
- Start's state legend is live: the four engine states plus the four exact station strings from `stations.ts`, so the page cannot define a vocabulary the screens do not print.
- `TestGauge` no longer draws red and green fills that read as progress. It draws a neutral track, a red boundary at kill, a green boundary at graduate, and a dot coloured only when the test has actually crossed a line.
- Lab: `INSUFFICIENT_EVIDENCE` is amber in the check table, not red; in-sample Sharpe is neutral text, not a green/red result; trials reconciled ("across every study; the Rules column covers these families only"); the Verdicts description says rows are families, not a ranking, so the best Sharpe can sit on a family that did not pass; the check table says a family can stop at Evidence with good numbers.
- News: `Headlines awaiting last markout` carries its noun; paging says "headlines left"; the state line drops the fraction once the bar is met; triggered/awaiting are explained as different counts; a mean with n under 10 is faint; the evidence bar track is visible (3px, `foreground/15`).
- Jev: "No skill" is a neutral tag, not red; a key states that skill needs AUC over 0.55 and 95% of reshuffles, and what each tag means.
- Empty paper book has one sentence on Now and Paper; the unreachable empty-positions panel on Now is gone, so the panel always says there is nothing open.
- Markets: the sampler is "about once a second" and the tick age carries freshness; `Today % (UTC)`; a note says prices use each coin's tick precision; the stale "sparkline colour is the last 3 hours" sentence is gone.
- Glossary `good` notes are guidance, not status, so "Sealed is the normal state" is no longer green.
- Sidebar has a dot legend: blue active, grey waiting, amber short of the bar, green passed.
- Now's duplicate "Last status" is gone; the header already carries the time.

Measured:
- Heights: Now 1,879; Start 2,009; Paper 2,347; Lab 3,944; News 4,813; Jev 3,116; Markets 1,609. Now is 232px taller than r04 because the empty positions panel now always renders; that panel is the answer to "is anything open".
- At 390: no horizontal overflow on any route. Now 3,726; Start 3,310; Paper 3,984; Lab 5,021; News 6,973; Jev 5,715; Markets 1,830.
- Zero WCAG AA contrast failures on all seven routes at 1440.
- Gauge: kill tick at 0%, graduate tick at 100%, dot at 35.98% for llr 0, matching (0 - -1.56) / (2.77 + 1.56).
- News bars: 80% and 10%; track visible.
- Lab check table: 0 red cells on the insufficient family; phrase "not enough evidence to judge" present.
- tsc, eslint and build pass. 8787 rebuilt; `/start` serves the new door sentence.

Scores, round 5 self-scored (raw; subtract 2): comprehension 9.5, state 9.5, hierarchy 9.0, craft 8.5, distinctiveness 7.5, copy 9.5, a11y 9.0, mobile 9.0. Critics on r05 not yet run.

Top gaps, own review:
1. The paper run still asks the reader to hold 685-973 days, three zero figures and a two-zero parity row at once. The gauge is honest now; the page is still heavy for the answer "nothing can happen yet".
2. News at 390 is 6,973px and the per-headline table is the bulk. A disclosure around it would shorten the page without hiding the means.
3. The Lab line map's rotated labels remain the densest object.
4. The station band repeats each page's state line; on Now the paper state appears twice within 400px.

Stop check: bar no | floor no | stills approved no | final checked no -> r05 critics, then approve or one more round
Next focus: r05 critics; if design still reads as generic, replace the Lab map labels and give the band a single aligned figure row per station.

---

## Round 6 — user direction: home is a console

Feedback, verbatim: "Home dashboard should be more optimal, less text more interface. use impecable to improve that"

Focus: the user overrides the critic queue. Distill Now; no other page changes.

Changes:
- Removed the page question, the four-line lede and the Current-state verdict block. The home opens on one answer line, "Nothing needs you.", and the interface.
- Replaced the four-cell prose pipeline with `StationRail`: Lab `0/7 families passed`, Paper `2 strategies on paper`, News `2/20 evidence days`, Jev `0/4 questions show skill`. Each cell is a link, with a 3px evidence bar where a denominator exists and one note (last judged, full days, triggered, skill).
- Paper book is two rows: strategy name, two tags, the gauge, full days. The definition of a full paper day and the "no evidence yet" paragraph are gone; they live on /paper.
- "Since you last look" is three numbers (headlines, triggered, trades) or one first-visit line, not a two-paragraph empty state.
- Book band is three figures: paper equity, open positions, and at-risk when anything is open (headlines scored when nothing is).
- Latest headlines is four rows; prices has a Coin / 3h / Price / 1h header row. Both panel descriptions removed.
- The empty positions panel is gone; when there are no positions the book band says 0 open and nothing exposed. The table only renders when there is something to show.
- Sidebar is navigation only: nav dots and the dot legend are removed. Station state lives in the rail, not the chrome.
- Explanation moved entirely to /start and the station pages.

Measured:
- Home: 1,026px at 1440, from 1,879 in r05 (a 45% cut) and 2,041 in r01. On a phone 2,074px, from 3,726.
- Rail cells: all four share top 187 and height 133, values at 26px mono.
- Zero WCAG AA contrast failures on the home. Keyboard: Tab reaches every rail cell with a visible 1px inset ring.
- No horizontal overflow on the home at 1440 or 390; the other six routes still render at 390 with no overflow.
- tsc, eslint and build pass; 8787 rebuilt and serving the new home.

Still open:
1. The Paper book shows two near-identical gauges; when both strategies share a card, the row does not say why there are two.
2. The first-visit panel is one line plus a link; it still occupies a third of the row.
3. The rest of the app remains long-form; the next pass should apply the same discipline to /paper and /catalysts, not the home again.

Stop check: user feedback addressed; stills not approved -> wait for the user on the home direction, then apply the same to the station pages.
