# Brief: jev-trader v3 dashboard, newcomer legibility

Target: make the existing read-only v3 dashboard (`web/`, Next.js 16) understandable to a just-onboarded user: what the product is, what every step in its lifecycle is doing, and the correct current state of each part. Stack stays Next.js app router + Tailwind + shadcn, no new dependencies.
References: none supplied. The current running app is the anti-reference (screens in `cook/stills/r00/`). Prose references: Linear's plain-label clarity, Vercel dashboard craft, Stripe docs explanation, Bloomberg terminal density without its costume.
Bar (0-10, critic median after calibration): comprehension for a newcomer 9.0, state legibility 9.0, information hierarchy 9.0, visual craft 9.0, distinctiveness (not a generic SaaS dashboard) 8.5, copy precision 9.0, accessibility 9.0, mobile 9.0.
Calibration offset: none yet (no user estimate).
Rules: no external assets, no new dependencies, no invented numbers or claims, no edge claims. Read-only stays read-only. Colour is state only; no gradients, glow, purple, beige, eyebrow labels, 01/02/03 counters, decorative particles, em dashes. Plain language over jargon, jargon defined in place. Every figure keeps its unit, source culture and misreading guard. No final render gate: this is a UI; ship when the stills are approved.
Time: floor 8h, rounds of 30-90 min. Started 2026-11-12 15:43 -03.
Usage: no readable meter; pace by round, report each round.
Hardware: desktop browser, also phone; Next dev/build on this host.
Deliverable: the running app at localhost:3000 (dev) and 8787 (built), stills per round in `cook/stills/rNN/`, a short changelog and design record. No video.
Permissions: no downloads, no push, no publish.
User's words: "the web app is hard to understand and too generic, i want you to make this easy to understand to just onboarded users. understand how this app works and improve the interface so users understand every step and the corrent state of the app"

## Decisions taken without an answer (intake timed out)

- Keep the near-black instrument-panel world; replace the generic stat-card grid with a product-specific lifecycle spine and a tighter type/density system. Rationale: impeccable Operate mode; a new costume would hurt the task.
- Onboarding is an always-visible layer, not a modal: stage strip on every page, a replayable `/start` guide, plain-language disclosure at the point of use, first-run empty states. No blocking tour.
- Split the paper run out of Overview so the home page can be a real "now" screen; add `/paper` and `/start`; rename nav to plain names.
- Nothing in the app gains a control. Arming stays a terminal/human decision.
