---
version: 1
slug: "dashboard-index-html"
primary_target: "dashboard/index.html"
related_targets: []
---

# Dashboard surface brief

Scope: `jev-trader dashboard`, the local monitoring page served by the bot binary. Mode: Operate. Desktop tab left open all day; glanced at, examined on events. Controls limited to Pause trading and Flatten all.

## Direction contract

THESIS: The dashboard is a wire desk. Every headline comes in as copy, Jev stamps it, and the slot editor (the bot) either runs it into a trade or spikes it. It refuses the category default: a charcoal KPI-tile dashboard with green/red numbers and a log tail.

OWN-WORLD: Desk-green rail (#2F4A3F) on a cool newsprint ground (#ECEDE6) with white sheets. Canary teleprinter paper (#F2E46B, slip fill #FBF3B9) marks copy the operator hasn't read yet; whether copy ran is told by its stamp (RUN · BUY/SELL, ALERT, FILED, SPIKED). Carbon ink (#1C1E22). Correction pink (#C23B57 text, #E8607A fill) for exits, alerts and stamps. Courier Prime for slugs and wire headlines, Archivo Narrow for UI and tabular figures. Verdicts are rubber stamps: 2px ink box, caps, slight rotation.

STORY: The operator sees at once whether the bot is alive, what ran since they last looked, why each position exists (traced back to its slips), and the health of every pipeline stage. They can pause or flatten in one deliberate action.

FIRST VIEWPORT: 190px green rail (wordmark, Wire / Run list / Pipeline / Trades, heartbeat footer). Centre: "The wire" heading with today's counts, filters, and Pause/Flatten at the right; slips newest first, each with a slug (ASSET-EVENT · SOURCE · time), the wire headline, Jev's reading, and a stamp on the right. Right 400px column: paper equity with its intraday history, the run list, the pipeline health grid.

FORM: Wire Desk, #1 on the grounded list (chosen as the pick card). seed:7e9083b6c40e291b. Signature interaction: selecting a position lights its source slips and dims the rest of the wire; new copy comes off the teleprinter and stays canary-unread until seen.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
