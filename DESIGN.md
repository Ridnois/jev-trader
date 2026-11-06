---
name: Jev Wire Desk
description: A newsroom wire desk for a news-driven paper-trading bot; every headline is copy, every verdict is a rubber stamp.
colors:
  desk: "#2F4A3F"
  desk-2: "#3B5A4D"
  desk-3: "#24392F"
  desk-ink: "#E4EDE7"
  desk-mute: "#AFC4B8"
  canary: "#F2E46B"
  canary-paper: "#FBF3B9"
  canary-edge: "#E3D35A"
  pink: "#E8607A"
  pink-ink: "#B3304C"
  pink-wash: "#FBE7EB"
  pos: "#2F6B4F"
  amber: "#D39A1E"
  amber-ink: "#80540A"
  ground: "#ECEDE6"
  sheet: "#FFFFFF"
  sheet-2: "#F6F6F1"
  rule: "#D5D6CE"
  rule-2: "#E4E5DE"
  ink: "#1C1E22"
  mute: "#575F59"
  faint: "#5E655F"
typography:
  display:
    fontFamily: "Archivo Narrow, Arial Narrow, sans-serif"
    fontSize: "42px"
    fontWeight: 700
    lineHeight: 1
    letterSpacing: "-0.02em"
    fontFeature: "tnum"
  headline:
    fontFamily: "Archivo Narrow, Arial Narrow, sans-serif"
    fontSize: "28px"
    fontWeight: 700
    lineHeight: 1.1
    letterSpacing: "-0.01em"
  wire-headline:
    fontFamily: "Courier Prime, ui-monospace, monospace"
    fontSize: "18px"
    fontWeight: 700
    lineHeight: 1.28
    letterSpacing: "-0.01em"
  title:
    fontFamily: "Archivo Narrow, Arial Narrow, sans-serif"
    fontSize: "16px"
    fontWeight: 700
    lineHeight: 1.3
  body:
    fontFamily: "Archivo Narrow, Arial Narrow, sans-serif"
    fontSize: "14px"
    fontWeight: 450
    lineHeight: 1.45
    fontFeature: "tnum"
  slug:
    fontFamily: "Courier Prime, ui-monospace, monospace"
    fontSize: "12px"
    fontWeight: 700
    letterSpacing: "0.06em"
  label:
    fontFamily: "Archivo Narrow, Arial Narrow, sans-serif"
    fontSize: "12px"
    fontWeight: 700
    letterSpacing: "0.07em"
  stamp:
    fontFamily: "Archivo Narrow, Arial Narrow, sans-serif"
    fontSize: "14px"
    fontWeight: 700
    lineHeight: 1.15
    letterSpacing: "0.09em"
rounded:
  stamp: "3px"
  sheet: "4px"
  control: "5px"
  panel: "6px"
  dot: "50%"
spacing:
  xs: "4px"
  sm: "8px"
  slip-gap: "10px"
  md: "14px"
  lg: "18px"
  xl: "22px"
  gutter: "28px"
components:
  button-outline:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    padding: "8px 14px"
  button-outline-hover:
    backgroundColor: "{colors.sheet}"
  button-ink:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.sheet}"
    rounded: "{rounded.control}"
    padding: "8px 14px"
  button-danger:
    backgroundColor: "{colors.pink-ink}"
    textColor: "{colors.sheet}"
    rounded: "{rounded.control}"
    padding: "8px 14px"
  button-quiet:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
    rounded: "{rounded.control}"
    padding: "8px 14px"
  button-quiet-hover:
    backgroundColor: "{colors.rule-2}"
  slip:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    rounded: "{rounded.stamp}"
    padding: "14px 18px 14px 20px"
  slip-unread:
    backgroundColor: "{colors.canary-paper}"
  stamp-run-buy:
    backgroundColor: "transparent"
    textColor: "{colors.pos}"
    typography: "{typography.stamp}"
    rounded: "{rounded.stamp}"
    padding: "6px 10px 7px"
    width: "118px"
  stamp-run-sell:
    backgroundColor: "transparent"
    textColor: "{colors.pink-ink}"
    typography: "{typography.stamp}"
    rounded: "{rounded.stamp}"
    padding: "6px 10px 7px"
  stamp-alert:
    backgroundColor: "{colors.pink-ink}"
    textColor: "{colors.sheet}"
    typography: "{typography.stamp}"
    rounded: "{rounded.stamp}"
    padding: "6px 10px 7px"
  stamp-filed:
    backgroundColor: "rgba(47,107,79,0.07)"
    textColor: "{colors.pos}"
    typography: "{typography.stamp}"
    rounded: "{rounded.stamp}"
    padding: "6px 10px 7px"
  stamp-spiked:
    backgroundColor: "transparent"
    textColor: "{colors.faint}"
    typography: "{typography.stamp}"
    rounded: "{rounded.stamp}"
    padding: "6px 10px 7px"
  rail-nav-item:
    backgroundColor: "transparent"
    textColor: "{colors.desk-mute}"
    rounded: "{rounded.panel}"
    padding: "9px 10px"
  rail-nav-item-hover:
    backgroundColor: "{colors.desk-3}"
    textColor: "{colors.desk-ink}"
  rail-nav-item-active:
    backgroundColor: "{colors.desk-2}"
    textColor: "{colors.sheet}"
  filter-tabs:
    backgroundColor: "{colors.rule-2}"
    rounded: "{rounded.panel}"
    padding: "3px"
  filter-tab-active:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    rounded: "{rounded.stamp}"
    padding: "6px 12px"
  unread-flag:
    backgroundColor: "{colors.canary}"
    textColor: "{colors.ink}"
    typography: "{typography.slug}"
    rounded: "{rounded.stamp}"
    padding: "2px 7px"
  table-sheet:
    backgroundColor: "{colors.sheet}"
    rounded: "{rounded.sheet}"
    padding: "10px 14px"
---

# Design System: Jev Wire Desk

## Overview

**Creative North Star: "The Wire Desk"**

The interface is a newsroom slot desk, not a trading terminal. Headlines arrive as copy on canary teleprinter paper, the operator reads them, and the slot editor (the bot) either runs each one into a trade or spikes it. Every verdict is a rubber stamp in the margin of the slip. The world sits on a cool newsprint ground with white sheets laid on it and a desk-green rail along the left edge; ink is carbon black, corrections are pink.

Density is working-desk density: a light, all-day tab that reads at a glance (is the bot alive, what ran, what needs attention) and rewards a closer look (each position traces back to its source slips). Type is split between the copy (Courier Prime, a typewriter face for slugs and wire headlines) and the desk (Archivo Narrow with tabular figures for every piece of UI and every number). Color is semantic and scarce: canary means unread, green means it ran long or was filed, pink means an exit, an alert, or a warning about the data itself.

The world explicitly rejects the category default: a charcoal dashboard of KPI tiles with green/red numbers over a log tail.

**Key Characteristics:**
- Cool newsprint ground, white paper sheets, desk-green rail.
- Canary paper is reserved for unread copy.
- Verdicts are rotated rubber stamps, the system's signature device.
- Typewriter face for copy, narrow grotesque with tabular figures for everything else.
- One authored motion moment: a slip feeds off the teleprinter and its stamp thunks down.

## Colors

A newsroom palette: green desk furniture, newsprint, canary copy paper, carbon ink and a correction pen.

### Primary
- **Desk Green** (desk): the rail background, the focus ring, the trace banner and the keyboard focus outline. It is the furniture and the operator's hand; it never marks a trade outcome.
- **Desk Green Lifted** (desk-2) and **Desk Green Sunk** (desk-3): the active and hover states of rail navigation, and the rail's divider rule.
- **Desk Ink** (desk-ink) and **Desk Mute** (desk-mute): text on the rail, active and resting.

### Secondary
- **Canary Teleprinter** (canary): the wordmark, the active rail icon, the unread counter, text selection, and the NEW flag (canary on ink).
- **Canary Slip Paper** (canary-paper): the fill of an unread slip and of a warning notice. **Canary Edge** (canary-edge) is its darker margin.

### Tertiary
- **Correction Pink Ink** (pink-ink): RUN · SELL stamps, the solid ALERT stamp, sell sides and negative figures, error dots, the danger button and the SYNTHETIC DATA flag.
- **Correction Pink** (pink): the stale/stopped heartbeat dot and exit zones.
- **Pink Wash** (pink-wash): the fill of a bad-news notice (synthetic data, kill switch).
- **Ledger Green** (pos): RUN · BUY and FILED stamps, buy sides, positive figures, healthy stage dots.
- **Amber** (amber) and **Amber Ink** (amber-ink): degraded pipeline stages only, as a dot or as text.

### Neutral
- **Newsprint Ground** (ground): the page behind everything; also the sticky header's fill.
- **Sheet White** (sheet): slips, tables, run-list sheets; text on solid ink, desk or pink surfaces.
- **Off Sheet** (sheet-2): the glance column and empty-state panels.
- **Rule** (rule) and **Hairline** (rule-2): column dividers, table header rules, row hairlines and the filter-tab tray.
- **Carbon Ink** (ink): all primary text, the ink button, the gauge needle.
- **Mute** (mute) and **Faint** (faint): secondary text (readings, slugs, captions) and the dashed SPIKED stamp. Faint is held at legible contrast; it is not a lighter tier.

### Named Rules
**The Canary Means Unread Rule.** Canary paper fills a slip only while the operator has not seen it. It is never used to signal importance, profit or warning tone on a slip.

**The Stamp Tells Rule.** Whether copy ran, was filed or was spiked is told by its stamp, not by the slip's fill, border or a colored stripe.

**The Correction Pen Rule.** Pink marks exits, alerts, sell stamps and warnings about the data's honesty. It is never decoration.

## Typography

**Display / UI Font:** Archivo Narrow (with Arial Narrow, sans-serif), variable 400 to 700
**Copy Font:** Courier Prime (with ui-monospace, monospace), 400 and 700

**Character:** A typewriter face for what came over the wire, a condensed grotesque for what the desk does with it. All figures are tabular.

### Hierarchy
- **Display** (700, 42px, line-height 1): the paper equity figure only.
- **Headline** (700, 28px, 1.1): the view title in the sticky header ("The wire").
- **Wire Headline** (Courier Prime 700, 18px, 1.28, max 68ch): the headline on each slip; also empty-state titles.
- **Title** (700, 16px): sheet and table headings.
- **Body** (450, 14px, 1.45): all UI text, readings at 13.5px, captions at 12 to 13px.
- **Slug** (Courier Prime 700, 12px, 0.06em, uppercase): ASSET-EVENT · SOURCE · time above each headline; also the unread counter and rail flags.
- **Label** (700, 12 to 13px, 0.07em, uppercase): glance panel headings and table column headers.
- **Stamp** (700, 14px, 0.09em, uppercase): stamp verdicts, with a 12px sentence-case line beneath.

### Named Rules
**The Copy/Desk Split Rule.** Courier Prime is for wire copy only: slugs, wire headlines, the wordmark, code and small flags. Every control, number and label is Archivo Narrow with tabular figures.

## Layout

A three-column desk filling the viewport: a 200px desk-green rail (wordmark, four views, heartbeat footer pinned to the bottom), a fluid work column (28px side padding) with a sticky header holding the view title, counts and the Pause / Flatten controls, and a 400px glance column on off-sheet holding paper equity with its intraday line, the run list and the pipeline health grid. Each column scrolls on its own.

Slips stack newest first with a 10px gap; each slip is a two-column grid with copy on the left and a 140px stamp column on the right. Glance sections are separated by 14px.

Below 1240px the glance column moves above the work column as an auto-fit grid of 300px-minimum panels, and Pause / Flatten move into the rail. Below 760px the rail becomes a sticky top bar (wordmark, scrolling view tabs, heartbeat), icons drop out of nav, the work column uses a 16px gutter, and stamps drop under the slip text, left-aligned.

**The Three-Column Desk Rule.** Rail for where you are and whether each desk lives, work column for copy, glance column for state. A new view goes into the work column; it never adds a fourth column.

## Elevation & Depth

Paper on a desk: sheets sit on the newsprint ground with a single soft, low shadow; there is no stacking of elevations. Sticky bars use the ground color so content passes under them. Focus and tracing are shown with rings, not lift.

### Shadow Vocabulary
- **Paper** (`box-shadow: 0 1px 2px rgba(28,30,34,.07), 0 3px 10px rgba(28,30,34,.05)`): slips, sheets, notices.
- **Trace ring** (`box-shadow: 0 0 0 2px #2F4A3F` plus Paper): a slip that sources the selected position.
- **Focus banner** (`box-shadow: 0 6px 14px rgba(28,30,34,.12)`): the sticky trace banner only, so the wire visibly slides under it.

**The Paper Sheet Rule.** One soft shadow for every sheet. No hard offset shadows, no glows, no stacked elevations.

## Shapes

Nearly square paper. Slips and stamps use a 3px corner, tables and sheets 4px, buttons and selects 5px, rail items, tabs trays and notices 6px. Status dots are 9px circles; health strips are 4px bars. Borders are 1px hairlines for structure, 1.5px for controls, 2px for stamps.

**The Rubber Stamp Rule.** A stamp is a 2px currentColor box, 3px corners, uppercase tracked caps, rotated -2.5deg. It is the only rotated element in the system.

## Components

### Buttons
Deliberate, inked, not soft.
- **Shape:** gently squared (5px), 1.5px carbon border, 600 weight, 16px icon at 7px gap.
- **Outline (Pause trading):** transparent on ground; hover fills sheet white.
- **Ink (Flatten all):** carbon fill, white text; hover goes pure black.
- **Danger (confirm flatten):** pink-ink fill and border; hover darkens.
- **Quiet (Cancel):** no border; hover fills hairline.
- **Press:** 1px downward nudge. Disabled at 45% opacity.
- **In the rail:** the same buttons invert: white outline, white ink button, pink danger.

### Slips (signature)
The unit of the wire.
- **Shape:** sheet white, 3px corners, Paper shadow, 14px 18px 14px 20px padding.
- **Content:** slug line (with a NEW flag while unread), wire headline, Jev's reading in mute body with bold values, and an outcome line in green or pink when the slip ran.
- **Unread:** canary slip paper fill until seen.
- **Tracing:** selecting a run-list row dims every other slip to 32% opacity and grayscale; source slips keep full color and gain the 2px desk ring. A desk-green banner states what is being traced and offers "Show everything".

### Stamps (signature)
- **RUN · BUY:** ledger-green ink outline.
- **RUN · SELL:** correction-pink ink outline.
- **ALERT:** solid pink-ink pad, white caps.
- **FILED:** ledger-green ink on a 7% green tinted pad.
- **SPIKED:** faint ink, dashed border.
- A 12px sentence-case line under the verdict carries the asset or the reason (e.g. "opinion / recap", "+0.36 BTC").

### Filter Tabs and Select
- **Tabs:** a hairline tray (6px, 3px inset); the active tab is a sheet-white chip with a faint shadow; counts sit in faint weight after each label.
- **Select:** sheet white, 1.5px rule border, 5px corners, inline chevron.

### Navigation (rail)
- Desk-mute items at 15px/500 with an 18px line icon; hover sinks to desk-3; the active item lifts to desk-2 with white text and a canary icon.
- **Heartbeat footer:** a 9px dot (pulsing mint when alive, pink when stale or stopped), "Bot alive" in white, the tick line in desk-mute, a pink SYNTHETIC DATA flag when the data is a demo, and PAPER TRADING in canary mono.

### Run List and Gauges
Each asset is a button row: symbol, state and why-line, P&L right-aligned, and a 16px gauge under it (pink exit zone, green buy zone, a midline, a carbon needle for the news score that slides on change). Pressed rows get a 1.5px inset desk ring.

### Pipeline Health Grid
A two-column hairline grid of stage cells on sheet white, each with a status dot (green ok, amber degraded, pink-ink error, hollow skipped), the stage name and a two-line clamp of detail.

### Notices
Full-width sheets under the header: sheet white by default, canary slip paper for warnings, pink wash with dark-pink text for bad news (synthetic data, kill switch).

### Tables
Sheet-white containers at 4px with the Paper shadow; uppercase label headers over a rule line, hairline rows, right-aligned tabular numbers, buy/sell sides as tracked green or pink caps.

### Desks (self-explanation)
The newsroom has two desks: the **News desk** (`jev-trader run`) and the **Scalp desk** (`jev-trader scalp`).
- **Rail groups:** each desk's views sit under a mono desk heading with its own status dot and a canary venue tag (paper / testnet / LIVE).
- **Heartbeat footer:** one row per desk (dot, name, what it is doing, venue line). A LIVE venue is a pink-ink pad.
- **Glance column follows the active desk** and every heading names it ("News desk · equity", "Scalp desk · equity"), with a "where the money is" line under the figure.
- **Header:** the subtitle starts with the desk slug; controls name their scope ("Pause news desk").

### Guide sheets
A "How to read this page" sheet opens each view: a title, plain sentences, and where useful a **flow strip** (mono step names over a 2px carbon rule, joined by faint arrows; a side input sits under the strip on a dashed rule), a **stamp legend** drawn with real stamps, or a **glossary** in the kv grid. "Got it, hide" collapses it into a header link; the choice is per viewer (localStorage). Jargon elsewhere is a **term**: dotted underline, definition on hover and focus.

### Motion
One authored moment: a newly arrived slip feeds down from its top edge (clip-path reveal with an 8px drop, 620ms), then its stamp thunks in from 1.35x scale and -9deg to rest (420ms, starting at 360ms). Both use `cubic-bezier(.16,1,.3,1)`. Everything else is 200ms color or background transitions; the heartbeat pulse is the only loop. All motion is removed under reduced motion.

## Do's and Don'ts

### Do:
- **Do** tell every verdict with a stamp from the five-stamp vocabulary (RUN · BUY, RUN · SELL, ALERT, FILED, SPIKED).
- **Do** keep canary slip paper for unread copy and clear it the moment the slip is seen.
- **Do** set wire copy in Courier Prime and every number in Archivo Narrow with tabular figures.
- **Do** put paper sheets on the newsprint ground with the single Paper shadow.
- **Do** label synthetic data and paper trading in the rail and in a pink-wash notice whenever they apply.
- **Do** trace by dimming: selected sources keep color and gain the desk ring, the rest go to 32% grayscale.

### Don't:
- **Don't** build charcoal KPI-tile dashboards or hero-metric tiles; state lives in the glance column as plain figures and rows.
- **Don't** add eyebrow or kicker labels above headings.
- **Don't** mark slips with colored side stripes or tinted borders; the stamp carries the outcome.
- **Don't** use canary for emphasis, or pink for decoration.
- **Don't** rotate anything except stamps, and don't animate anything beyond the feed-and-thunk moment, state transitions and the heartbeat.
- **Don't** use hard offset shadows or glows.

## 2026-11-05 plain-language redesign

The main dashboard (`/`) now explains the bot instead of mirroring its internals. The Wire Desk UI is still served at `/classic`.
- Plain sentences come first. Raw machine readings are set in Courier Prime beside them, so a reader can tell explanation from measurement.
- Highlighter yellow marks only what the bot actually did: a trade taken, a position open.
- Every percent is a percent; basis points are never shown outside the raw column.
- Testnet results always sit next to the same trades costed at real Binance fees.
