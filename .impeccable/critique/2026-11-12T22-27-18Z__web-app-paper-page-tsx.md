---
target: paper section
total_score: 29
max_score: 40
na_heuristics: 
p0_count: 0
p1_count: 2
target_identity: "file:/home/ridnois/Documents/bussiness/trading-v3/web/app/paper/page.tsx"
target_fingerprint: "sha256:5548b7a843115f067f4e5e51e6232da53df9c5944869b5fbcf8ebfcdc4093ebe"
target_path: /home/ridnois/Documents/bussiness/trading-v3/web/app/paper/page.tsx
timestamp: 2026-11-12T22-27-18Z
slug: web-app-paper-page-tsx
---
# Critique: Paper page (web/app/paper/page.tsx)
Method: dual-agent. Score 29/40 (Good).

| # | Heuristic | Score | Key issue |
|---|---|---|---|
| 1 | Visibility of status | 3 | Staleness only shown in the header; page body ignores `offline` (page.tsx:142); phones drop the timestamp |
| 2 | Match real world | 3 | "log-likelihood ratio" ×3; "Show the chart layout" names the screen, not the task |
| 3 | User control | 3 | Read-only; the chart toggle only works one way |
| 4 | Consistency | 3 | "Hundreds of days" (:288) vs 2,573–3,657; empty trades state names one strategy |
| 5 | Error prevention (misreading) | 3 | "+14.0 bps a day" is the hero figure (:93) |
| 6 | Recognition over recall | 3 | Differences between strategies scattered; comparing means remembering card 1 |
| 7 | Flexibility | 2 | No compact view; same explanations scrolled past every visit |
| 8 | Minimalist | 2 | "No full paper day yet" ×5; test value ×3 and drawdown ×2 per card |
| 9 | Error recovery | 3 | Empty states name the command; staleness has no effect in the body |
| 10 | Help | 4 | Inline glossary, "cannot tell you" panel |

Design specificity: built for this product (honest copy, status line, fixed kill/graduate lines). Weakness is repetition and inverted emphasis, not a generic look.
Detector: source scans clean (text patterns only). Live page: tiny-text ×4 (shell.tsx:118 real, bits.tsx:149 borderline, page.tsx:133 ×2 mostly FP), line-length ×1 (bits.tsx:24, real, use ~52ch), nested-cards ×2 (bits.tsx:174, partly FP), overused-font Geist (FP, Vercel look chosen), straggler ×2 advisory. Overlay injected in the [Human] paper tab.

## Priority issues
- [P1] Forecast edge is the main number (page.tsx:93). Lead with test value and drawdown; give the forecast as a ratio to the daily swing (0.06σ). /impeccable clarify
- [P1] Test horizon (685–973 and 2,573–3,657 days) buried and understated; add a line per card; fix :288. /impeccable clarify
- [P2] Repetition doubles card length (:59/:68/:114, :95/:117); comparison band first, cards below. /impeccable distill
- [P2] Chart hidden behind a one-way button (:83-85); always show it on "Whole test". /impeccable layout
- [P2] Live positions below static explanations; body ignores staleness; active Verdict dot is grey (:162). /impeccable harden

## Personas
Alex: no comparison table or per-day table; faint 11px settings string. Sam: chart tooltip mouse-only; gauge text 11px at borderline contrast; glossary buttons add many tab stops. Glancing operator: forecast is the most prominent number; stale looks like healthy; positions below the first screen.

## Minor
Lede ~88 characters per line; n=384 unit (bars) not stated; "Running since" vs "Up 3h 59m" unclear; "evidence" vs "test stands at"; "Nothing on paper" title twice in a row; two straggler words.

## Questions
Should the page say the 4-day run can't answer in a useful time? Why show the forecast edge at all? Show the explanations once rather than on every visit?
