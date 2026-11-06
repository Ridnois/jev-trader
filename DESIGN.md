---
name: Jev Departures
description: A split-flap departure board for two trading bots; every coin is a row, and the check that holds it is read straight off the board.
colors:
  steel: "#C9CDD2"
  steel-lt: "#DDE0E3"
  steel-dk: "#6F757D"
  steel-dkr: "#5C6168"
  steel-ink: "#33383E"
  rail: "#2A2D31"
  rail-2: "#3A3E43"
  rail-3: "#23262A"
  rail-ink: "#E9EBED"
  rail-mute: "#AEB3B9"
  flap: "#141414"
  flap-leaf-top: "#282828"
  flap-leaf-bot: "#151515"
  flap-pin: "#5E6268"
  flap-ink: "#F2F2F0"
  flap-dim: "#9A9EA4"
  board-ink: "#EEF0F2"
  lamp-off: "#2B2E33"
  amber: "#E3A21A"
  amber-ink: "#885800"
  amber-wash: "#FBF1DA"
  red-flap: "#F0675C"
  red: "#B4372F"
  red-deep: "#962A23"
  red-wash: "#FBE9E7"
  green-flap: "#62C98E"
  green: "#22704A"
  green-wash: "#E5F3EB"
  sheet: "#FFFFFF"
  sheet-2: "#F3F4F5"
  ink: "#15171A"
  mute: "#4A5058"
  rule: "#D9DCE0"
  rule-2: "#EBEDEF"
typography:
  title:
    fontFamily: "Barlow, system-ui, sans-serif"
    fontSize: "32px"
    fontWeight: 700
    lineHeight: 1.05
    letterSpacing: "-0.015em"
  figure:
    fontFamily: "Barlow, system-ui, sans-serif"
    fontSize: "28px"
    fontWeight: 700
    lineHeight: 1
    letterSpacing: "-0.02em"
    fontFeature: "tnum"
  wordmark:
    fontFamily: "Barlow, system-ui, sans-serif"
    fontSize: "21px"
    fontWeight: 700
    lineHeight: 1
    letterSpacing: "-0.01em"
  heading:
    fontFamily: "Barlow, system-ui, sans-serif"
    fontSize: "17px"
    fontWeight: 700
    lineHeight: 1.2
  body:
    fontFamily: "Barlow, system-ui, sans-serif"
    fontSize: "15px"
    fontWeight: 400
    lineHeight: 1.45
    fontFeature: "tnum"
  small:
    fontFamily: "Barlow, system-ui, sans-serif"
    fontSize: "12.5px"
    fontWeight: 400
    lineHeight: 1.3
  flap:
    fontFamily: "Barlow Condensed, Barlow, system-ui, sans-serif"
    fontSize: "clamp(10px, calc((100cqi - 236px) / 41), 23px)"
    fontWeight: 600
    lineHeight: 1
  board-title:
    fontFamily: "Barlow Condensed, Barlow, system-ui, sans-serif"
    fontSize: "20px"
    fontWeight: 700
    lineHeight: 1
    letterSpacing: "0.14em"
  nav:
    fontFamily: "Barlow Condensed, Barlow, system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: "0.09em"
  button:
    fontFamily: "Barlow Condensed, Barlow, system-ui, sans-serif"
    fontSize: "13.5px"
    fontWeight: 600
    lineHeight: 1.1
    letterSpacing: "0.08em"
  column-head:
    fontFamily: "Barlow Condensed, Barlow, system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: "0.14em"
  label:
    fontFamily: "Barlow Condensed, Barlow, system-ui, sans-serif"
    fontSize: "11.5px"
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: "0.1em"
  tag:
    fontFamily: "Barlow Condensed, Barlow, system-ui, sans-serif"
    fontSize: "11px"
    fontWeight: 700
    lineHeight: 1
    letterSpacing: "0.09em"
rounded:
  flap: "2px"
  tag: "3px"
  row: "4px"
  control: "5px"
  sheet: "6px"
  board: "8px"
  pill: "999px"
  dot: "50%"
spacing:
  flap-gap: "2px"
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "20px"
  page-x: "32px"
components:
  button-outline:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
    typography: "{typography.button}"
    rounded: "{rounded.control}"
    padding: "8px 14px"
  button-outline-hover:
    backgroundColor: "{colors.sheet}"
  button-flatten:
    backgroundColor: "transparent"
    textColor: "{colors.red}"
    typography: "{typography.button}"
    rounded: "{rounded.control}"
    padding: "8px 14px"
  button-flatten-hover:
    backgroundColor: "{colors.red-wash}"
  button-danger:
    backgroundColor: "{colors.red}"
    textColor: "{colors.sheet}"
    typography: "{typography.button}"
    rounded: "{rounded.control}"
    padding: "8px 14px"
  button-danger-hover:
    backgroundColor: "{colors.red-deep}"
  button-quiet:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
    typography: "{typography.button}"
    rounded: "{rounded.control}"
    padding: "8px 14px"
  flap-cell:
    backgroundColor: "{colors.flap-leaf-top}"
    textColor: "{colors.flap-ink}"
    typography: "{typography.flap}"
    rounded: "{rounded.flap}"
    width: "calc(var(--fs) * .64)"
    height: "calc(var(--fs) * 1.36)"
  board:
    backgroundColor: "{colors.steel-dk}"
    textColor: "{colors.board-ink}"
    rounded: "{rounded.board}"
    padding: "14px 16px 16px"
  board-row:
    backgroundColor: "transparent"
    rounded: "{rounded.row}"
    padding: "3px 4px"
  lamp:
    backgroundColor: "{colors.lamp-off}"
    rounded: "{rounded.dot}"
    size: "10px"
  trace:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    rounded: "{rounded.control}"
    padding: "16px 18px"
  sheet-bot:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    rounded: "{rounded.sheet}"
    padding: "14px 18px 16px"
  rail:
    backgroundColor: "{colors.rail}"
    textColor: "{colors.rail-ink}"
    width: "228px"
    padding: "22px 16px 18px"
  rail-nav-item:
    backgroundColor: "transparent"
    textColor: "{colors.rail-mute}"
    typography: "{typography.nav}"
    rounded: "{rounded.control}"
    padding: "8px 10px"
  rail-nav-item-hover:
    backgroundColor: "{colors.rail-3}"
    textColor: "{colors.rail-ink}"
  rail-nav-item-active:
    backgroundColor: "{colors.rail-2}"
    textColor: "{colors.sheet}"
  whose-money:
    backgroundColor: "{colors.rail-3}"
    textColor: "{colors.rail-ink}"
    rounded: "{rounded.sheet}"
    padding: "14px 12px"
  venue-paper:
    backgroundColor: "{colors.rail-2}"
    textColor: "{colors.rail-ink}"
    rounded: "{rounded.tag}"
    padding: "3px 6px 2px"
  venue-testnet:
    backgroundColor: "{colors.amber}"
    textColor: "{colors.ink}"
    rounded: "{rounded.tag}"
    padding: "3px 6px 2px"
  venue-live:
    backgroundColor: "{colors.red-flap}"
    textColor: "{colors.ink}"
    rounded: "{rounded.tag}"
    padding: "3px 6px 2px"
  tab-active:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
    typography: "{typography.nav}"
    padding: "9px 14px 8px"
  chip:
    backgroundColor: "transparent"
    textColor: "{colors.steel-ink}"
    rounded: "{rounded.pill}"
    padding: "5px 11px"
  chip-active:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.sheet}"
  tag-ok:
    backgroundColor: "{colors.green-wash}"
    textColor: "{colors.green}"
    typography: "{typography.tag}"
    rounded: "{rounded.tag}"
    padding: "4px 7px 3px"
  tag-no:
    backgroundColor: "{colors.red-wash}"
    textColor: "{colors.red}"
    typography: "{typography.tag}"
    rounded: "{rounded.tag}"
    padding: "4px 7px 3px"
  tag-wait:
    backgroundColor: "{colors.amber-wash}"
    textColor: "{colors.amber-ink}"
    typography: "{typography.tag}"
    rounded: "{rounded.tag}"
    padding: "4px 7px 3px"
  table-sheet:
    backgroundColor: "{colors.sheet}"
    rounded: "{rounded.sheet}"
    padding: "9px 14px"
---

# Design System: Jev Departures

## Overview

**Creative North Star: "The Departure Board"**

The dashboard is a station concourse. A brushed-steel ground carries a graphite rail on the left, white working sheets for money and evidence, and one matte split-flap board where every coin both bots watch is a row. A row's status flips through the flaps (WATCHING, IN TRADE, HOLDING, HELD, ORDER WAITING, REVIEW, WARMING UP, PAUSED, ON HOLD) and its WAITING FOR column names the next thing that has to happen, so "why not" is read off the board instead of hunted for in a log. Clicking a row drops a white trace sheet into the board with the checks in order.

Density is concourse density: an all-day second-monitor tab that reads at a glance (heartbeats, whose money, which coins are lit) and rewards a closer look (trace, decisions, trades, evidence). Type splits by role, not by mood: Barlow Condensed caps for everything mechanical (flaps, labels, buttons, nav), Barlow for sentences and figures, all with tabular numerals.

The world rejects the KPI-tile trading terminal and the P&L-first equity chart. Money appears as three compact sheets above the board, never as a hero number.

**Key Characteristics:**
- Brushed-steel ground with a fine horizontal grain, graphite rail, white sheets, one flap board.
- The flap board is the signature: two-leaf cells split by a hairline with hinge pins, folding on change.
- Color is a four-state signal (white, amber, green, red) carried by flap lettering and a row lamp, never by panels.
- Condensed caps for machinery, a plain grotesque for sentences.
- Only two writing controls exist (Pause, Flatten), both for the news bot.

## Colors

A cool, nearly achromatic steel-and-graphite world in which the only saturated color is signal. Each signal exists twice: a bright flap version for lettering on black, and a dark ink version plus a pale wash for use on white.

### Primary
- **Brushed Steel** (steel): the page ground, overlaid with a 3px repeating horizontal grain (white at 7%, black at 1.8%). Tabs, chips and page-level notes sit directly on it in Steel Ink.
- **Board Steel / Board Steel Deep** (steel-dk to steel-dkr): the flap board's housing, a top-to-bottom gradient. Steel-dk also draws chip outlines, the tab baseline, the scrollbar, and the unlit heartbeat.
- **Graphite Rail** (rail), **Lifted** (rail-2, the active nav item and paper venue tag) and **Sunk** (rail-3, nav hover and the whose-money panel). Rail Ink and Rail Mute carry rail text.

### Secondary
- **Matte Flap Black** (flap): the board's material name and the legend swatches. The cells themselves render as two leaves, **Upper Leaf** (flap-leaf-top) over **Lower Leaf** (flap-leaf-bot), so the top half catches more light than the bottom.
- **Flap White** (flap-ink): default flap lettering. **Flap Dim** (flap-dim): lettering for secondary or not-yet-meaningful values (WAITING FOR on a plain watching row, cost/move that doesn't clear the bar, "NO TRADES YET").
- **Hinge Pin** (flap-pin): the 2 x 4px pins at each cell's split. **Board Lettering** (board-ink): column heads, section headers and legend on the board housing. **Unlit Lamp** (lamp-off).

### Tertiary
- **Signal Amber** (amber): waiting, held by a limit, or needs review. Also the brand accent ("Jev"), the active-nav inset bar, focus rings, text selection, the testnet venue tag, and a budget meter at 75% or more. **Amber Ink / Amber Wash** carry it on white (wait tags, warning notices).
- **Flap Red** (red-flap) on black and **Cancelled Red** (red) on white: stopped, on hold, or a loss; also the Flatten button, short side, stop tick, and a full meter. **Red Deep** is the confirm button's hover only. **Red Wash** backs failed checks and bad notices.
- **Flap Green** (green-flap) on black and **Cleared Green** (green) on white: in a trade, a passed check, or a profit; long side; take-profit tick. **Green Wash** backs passed checks.

### Neutral
- **Working Sheet** (sheet) and **Sheet Recess** (sheet-2, hovered/open table rows and detail rows).
- **Ink** (ink) for text and filled controls; **Mute** (mute) for secondary text on white. **Rule** (rule, table head underline) and **Hairline Rule** (rule-2, row dividers, meter tracks).

### Named Rules
**The Two-Voice Signal Rule.** Every signal color has a flap voice and a paper voice. Bright flap colors (red-flap, green-flap, amber) letter black cells and light lamps; dark ink colors (red, green, amber-ink) on their washes mark white sheets. Never put a flap color on white or an ink color on a flap.

**The Lamp Not Panel Rule.** A row's state is shown by its lamp, its STATUS flaps and a faint same-hue tint behind the row (13% amber or green, 14% red). Sheets and the board housing never change color with state.

## Typography

**Sentence Font:** Barlow (400, 500, 600, 700), with system-ui fallback.
**Machine Font:** Barlow Condensed (500, 600, 700), falling back to Barlow.

The body sets `font-variant-numeric: tabular-nums` globally, so every figure on every surface aligns. Both families are self-hosted woff2 served by the bot binary; Barlow Condensed 600 and Barlow 400 are preloaded.

### Hierarchy
- **Title** (Barlow 700, 32px, 26px at 860px and below): the page H1 only (Departures, Scalper, News bot, Is it working?).
- **Figure** (Barlow 700, 28px): a bot sheet's result number. Larger 34px figures exist on detail sheets.
- **Heading** (Barlow 700, 17px): sheet and trace headings.
- **Body** (Barlow 400, 15px / 1.45): ledes and prose, capped near 78ch; 14px inside sheets, traces and tables.
- **Small** (Barlow 400, 12.5px): explanations under readings, beats, checks.
- **Flap** (Barlow Condensed 600, fluid size via `--fs`, uppercase): board cells only.
- **Board Title** (Barlow Condensed 700, 20px, 0.14em): the DEPARTURES flap heading; its cells are fixed at 19px.
- **Nav / Button / Column Head / Label / Tag** (Barlow Condensed 600-700, 11-14px, uppercase, 0.07-0.14em tracking): every mechanical word.

### Named Rules
**The Caps Are Machinery Rule.** Uppercase condensed type is for flaps, controls, nav, column heads and labels. Sentences, explanations and headlines are always Barlow in sentence case. If it is a full sentence, it is never in caps.

## Layout

A two-column shell: a sticky 228px graphite rail (full viewport height, scrolls on its own) and a fluid page padded 26px 32px 64px. The page and its view stack on a 20px grid gap. The board view runs: header (title, posture line, controls at right) / three bot sheets / the flap board.

Sheets lay out with `repeat(auto-fit, minmax(min(300px, 100%), 1fr))` at a 16px gap, so three sit in a row on desktop and stack when narrow.

### The board grid
The board is a size container. Rows and the column head share one grid: a lamp column, then six flap columns sized in character cells: COIN 6, SIDE 5, STATUS 13, WAITING FOR 18, COST/MOVE 7, LAST RESULT 15, spread with `justify-content: space-between` and a 12px column gap. The cell size derives from the board's own width:

- `--fs: clamp(10px, calc((100cqi - 236px) / 41), 23px)` (flap font size)
- `--cw: calc(var(--fs) * .64 + 2px)` (one cell plus its 2px gap)
- each column is `calc(var(--cw) * N)` for N characters.

Rows are grouped under section headers (Scalper, News bot) with a one-line description; there is no Bot column.

### Responsive
- **At 1100px and below:** detail rows (coin rows, hypothesis rows) drop to two columns.
- **At 860px and below:** the shell becomes one column and the rail becomes a sticky top bar (wordmark, heartbeats without their sublines, a horizontally scrolling nav). The whose-money panel is hidden and a one-line money note appears under the title instead. Controls align left. The board drops COST/MOVE and LAST RESULT, keeping lamp + COIN, SIDE, STATUS, WAITING FOR at `--fs: clamp(9px, calc((100cqi - 160px) / 27), 20px)`.
- **At 520px and below:** each board row stacks into two lines: lamp, COIN and STATUS on line one, WAITING FOR spanning beneath. SIDE is hidden; the WAITING FOR head is hidden. `--fs: clamp(9px, calc((100cqi - 60px) / 13), 16px)`, 10px column gap, 3px row gap, 6px vertical row padding.
- **At 400px and below:** the board's padding trims to 12px 8px 14px and rows to 2px side padding.

### Named Rules
**The Count-In-Cells Rule.** Board columns are measured in flap characters, never in pixels or percentages. A new column declares its width as a character count and every value is padded or truncated to it.

## Elevation & Depth

Depth is physical and shallow: paper on steel, a recessed board, flaps with a split.

- **Sheets** carry a soft two-layer paper shadow (1px/2px and 4px/12px of Ink at 8% and 6%).
- **The board** is a pressed metal panel: an inner 1px white ring at 18%, a 1px top highlight at 25%, and a 6px/18px drop at 18%.
- **The trace** sits on the board with a darker 2px/8px shadow at 25%, reading as a card laid onto the housing.
- **Lit lamps** carry a 2px same-hue halo at 25%; the unlit lamp is inset (1px dark inner shadow).
- **Flap cells** have no shadow; their depth is the hairline split between leaves.

### Named Rules
**The Paper On Steel Rule.** Shadows are ambient and soft. There are no hard offset shadows and no glows except the lamp halo and the heartbeat ring.

## Shapes

Small, mechanical radii that scale with the object: 2px flap cells, 3px tags and venue labels, 4px board rows, 5px buttons, nav items and the trace, 6px sheets and the whose-money panel, 8px for the board housing. Chips are full pills; lamps, heartbeats and stage dots are circles. Meters and gauge tracks are 4-6px bars with matching rounded ends.

## Components

### The split-flap cell (signature)
Each character is a cell `calc(var(--fs) * .64)` wide and `calc(var(--fs) * 1.36)` tall, 2px apart, with 2px corners. It is built from two absolutely positioned halves holding the same character: the upper leaf clipped to the top half (`clip-path: inset(0 0 calc(50% + .5px) 0)`) on Upper Leaf, the lower leaf clipped to the bottom half on Lower Leaf. The 1px left between the clips is the hairline split, and the board steel shows through it. Two Hinge Pin nubs (2 x 4px) sit at the split on each side. Lettering takes the flap's tone class: default white, amber, red, green, or dim.

**The fold.** When a value changes, only the changed characters move, each starting 24ms after the one before it and cycling through 1 to 3 random characters (by position) at 120ms steps before landing. Each step is a two-leaf fold: a temporary top leaf holding the old character folds down (`rotateX` 0 to -90deg, perspective 160px, 55ms ease-in, darkening to 55% brightness) while the static top already shows the new one; then a temporary bottom leaf with the new character drops into place (90deg to 0, 60ms ease-out after 55ms, starting at 140% brightness). Both temporary leaves are created for that fold and removed on `animationend`; there is no persistent `will-change` and no idle animation on the board.

**Reduced motion.** With `prefers-reduced-motion: reduce`, characters are written into both halves directly with no fold, and a global rule removes every animation and transition.

**Accessibility.** Each flap group carries `role="text"` and an `aria-label` with the readable value; individual cells are `aria-hidden`.

### Board row
A full-width button: lamp, then COIN, SIDE, STATUS, WAITING FOR, COST/MOVE, LAST RESULT. Hover tints the row with white at 7%; an open row holds white at 12%. `data-lamp` (amber, green, red, or empty) lights the lamp and tints the row in the same hue.
- **Tones:** IN TRADE / HOLDING are green; ORDER WAITING, REVIEW, WARMING UP, HELD, PAUSED are amber; ON HOLD is red; WATCHING is white with an unlit lamp. SIDE is white for LONG/SHORT, dim otherwise. WAITING FOR is dim on plain watching rows, white when a trade or order is in play, amber or red when a limit or hold applies. COST/MOVE is white only when the typical move clears the cost bar, otherwise dim. LAST RESULT is green for a profit, red for a loss, dim for none.
- **Column head and section headers** use Column Head type in Board Lettering; section headers carry a sentence-case Small description and a hairline rule (white at 16%) above every section but the first.
- **Legend** at the board foot repeats the four signals as small flap-black squares outlined in their color, each with a plain-language meaning.

### Trace panel
Opening a row reveals a white sheet inside the board (5px corners, 16px 18px padding). A Heading sentence states the situation ("SOL: watching, waiting for a rush of trading."), a muted sub line explains it, then two auto-fit columns (min 280px):
- **Checks, in order.** The scalper runs warm-up, fresh prices, tight spread, rush of trading, one-sided flow, book support, price moving, no chase, big enough move, calmer news, then the risk checks: its open trade, free slot, cooldown, hourly limit, daily loss limit, exchange match, cash, Jev review. Each check has an 18px mark: green check on Green Wash (passed), red x on Red Wash (the first one that failed, set in 500 weight), or an outlined dash (not reached; its explanation is hidden and the row is tighter). The news bot shows its own safety checks with the raw detail right-aligned in condensed type.
- **Readings now.** A grid of figure + explanation pairs (18px Barlow 600 over Small mute text). The news bot adds a score gauge (sell zone red, buy zone green at 35% opacity, a center tick and an ink needle) and the driving headlines.

### Bot sheets
Three compact white sheets above the board: Scalper, News bot, Is it working?. Each money sheet has the name as a link with its venue tag, a Figure-size result in green, red or mute, a muted sub line, an "At risk" line with a condensed label, and three budget meters (6px tracks on Hairline Rule; ink fill, amber at 75%, red when full). The evidence sheet is a stack of labelled sentences; figures resting on too few cases render muted.

### Whose-money rail
The rail holds the wordmark ("Jev" in amber, "trader" in Rail Ink), the nav, the whose-money panel and two heartbeats pinned to the bottom. The panel (Graphite Sunk, 6px corners) names each bot with a venue tag (Paper on Graphite Lifted, Testnet on amber, Live on Flap Red, all with ink or rail text) and one plain sentence about the money, then a closing sentence on whether real money is armed. Heartbeats are 9px dots: green with a 2.4s expanding ring when healthy (the only looping animation), amber when stale, red when down, steel when connecting.

### Controls: Pause and Flatten
Outline buttons with condensed caps and a 16px stroke icon: 1.5px border, 5px corners, 8px 14px padding, 1px press. Pause/Resume is the ink outline. Flatten is the red outline (Cancelled Red border and text, Red Wash on hover). Pressing it swaps in a confirm pair: a filled red "Yes, sell everything" (Red Deep on hover) and a borderless Cancel. Filled red appears nowhere else. A right-aligned note under the buttons says the controls are news-bot only.

### Navigation, tabs, chips, tables, notices
- **Nav items** are condensed caps in Rail Mute; hover lifts to Graphite Sunk, the current page sits on Graphite Lifted in white with a 2px amber inset bar on the left.
- **Tabs** sit on a 1.5px steel-dk baseline; the selected tab is Ink with a 3px ink underline, counts in Small Barlow beside the label.
- **Filter chips** are steel-dk outlined pills; pressed chips fill with Ink.
- **Tables** live in a white sheet with condensed caps heads over a Rule underline, Hairline Rule dividers, right-aligned numerics, and Sheet Recess on hover and open rows. Tags (ok, no, wait, ink) are 11px condensed caps on their washes.
- **Notices** are sheets with a 16px icon; warn notices sit on Amber Wash, bad notices on Red Wash, with dark text in the same hue.

### Pages
Four hash routes share the shell: **#board** (Departures: sheets plus flap board), **#scalper** (lede, tabs for Decisions with filter chips, Trades, How it decides), **#news** (lede with controls, tabs for Coins, Headlines, Trades, Each look), **#evidence** (Is it working?: markout table, hypothesis tests, headline scoring). Only the board uses flaps; the other pages are sheets, tabs and tables.

## Do's and Don'ts

### Do:
- **Do** size every board column in flap characters and pad or truncate values to that count.
- **Do** put the reason on the board: WAITING FOR names the next thing that must happen in plain words ("A RUSH OF TRADING", "COOLDOWN TO END", "YOU TO RESUME").
- **Do** use the flap voice of a signal on black and the paper voice on white, with its wash behind it.
- **Do** fold only the characters that changed, with leaves created for the fold and removed after it.
- **Do** honor reduced motion by writing the final character directly.
- **Do** keep money statements as plain sentences in the rail and sheets ("Real money on Hyperliquid mainnet, capped at $X").
- **Do** keep Flatten as a red outline and reserve filled red for its confirm step.

### Don't:
- **Don't** add KPI tiles, sparklines or an equity chart above the board.
- **Don't** color sheets, the board housing or the rail by state; state lives in lamps, flap lettering and the faint row tint.
- **Don't** set sentences in condensed caps, or flap lettering in Barlow.
- **Don't** leave `will-change` or idle animations on flap cells; the board is still unless a value changes.
- **Don't** add new writing controls beyond Pause and Flatten, or browser controls for the scalper.
- **Don't** use hard offset shadows or glows other than the lamp halo and heartbeat ring.
