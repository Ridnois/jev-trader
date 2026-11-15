---
target: Open positions panel, visual and mobile
total_score: 20
max_score: 32
na_heuristics: 3,5
p0_count: 1
p1_count: 2
target_identity: "file:/home/ridnois/Documents/bussiness/trading-v3/web/components/positions.tsx"
target_fingerprint: "sha256:8f4c979aa6e811f18afc2e5a1750be9978b6c281246a7fad76a059b487d353e2"
target_path: /home/ridnois/Documents/bussiness/trading-v3/web/components/positions.tsx
timestamp: 2026-11-14T05-46-36Z
slug: web-components-positions-tsx
---
# Critique: Open positions panel (web/components/positions.tsx)

Method: dual-agent (A: design review · B: detector + browser)

## Design Health Score
| # | Heuristic | Score | Key issue |
|---|---|---|---|
| 1 | Visibility of system status | 3 | Stale, near-stop and no-stop states handled; "No price" styled as the quietest text on the panel |
| 2 | Match system / real world | 3 | Trader language; "Move" is position-relative (green +3.10% on a short whose price fell) with no sign convention |
| 3 | User control and freedom | n/a | Read-only by design |
| 4 | Consistency and standards | 3 | Matches tokens; prose sentence in the totals row under numeric columns |
| 5 | Error prevention | n/a | Nothing to act on |
| 6 | Recognition rather than recall | 2 | Full rule and stale reason only in title tooltips; Coin scrolls away on phone |
| 7 | Flexibility and efficiency | 1 | Alphabetical rows; nothing surfaces the position closest to its stop |
| 8 | Aesthetic and minimalist design | 2 | Rule said 3 times (description, 8 identical rows, footnote); at-risk said twice; 469-char footnote |
| 9 | Error recovery | 3 | "No price", "No stop", unmarked count are good; hover-only explanation |
| 10 | Help and documentation | 3 | Thorough but as prose, not on the columns |
| **Total** | | **20/32** | Acceptable, real work needed |

## Design specificity
Content is product-specific (26h of 30d, "$400 if every stop fills", "No price" instead of a stale mark). The container is a stock 11-column shadcn grid, alphabetical, equal weight: it knows what matters but only says it in a 13px muted sentence. Detector: CLI 0 findings; overlay found line-length on the footnote (positions.tsx:168-173, ~224 chars per line). tiny-text x10 (chart SVG labels, outside the panel) and overused-font (Geist, intentional) are false positives.

## Priority issues
1. [P0] Phone/tablet hides the outcome and the row names. Table min-width 1120px in a 350px (390 viewport) / 440px (768) container: 4 of 11 columns visible, Open P&L off-screen, Coin not sticky, Total row shows "$5,000" as if it were the result. Fix: below sm render each position as a two-line row (BCH · Short · $750 | +$22.95 / +3.10% · stop 11.5% away · 26h of 30d) with a summary line; at md+ sticky Coin column and reorder columns so P&L and To stop come right after Side. → adapt
2. [P1] Hierarchy doesn't answer "how is the book, is anything in trouble?". P&L and nearest stop live mid-sentence in muted 13px. Fix: header stat line (Open P&L large, pos/neg colour; nearest stop; at risk), rows sorted by To stop ascending with stale rows first, whole-row warn tint for near-stop/no-price. → layout
3. [P1] Rule repeated with no information: description, 8 identical "Trend · Short when it closes below the 4d low" cells (342px, widest column), footnote. Fix: when one strategy, drop the column and state the family + fired clause once; keep the full rule behind a disclosure (works on touch). Show the column only when strategies differ. → distill
4. [P2] Footnote and totals are prose where structure belongs. 469 chars, ~9 lines (205px) at 390px; mixes disclaimer, method, legend; at-risk repeated. Fix: one-line footnote; column meanings into Gloss/glossary on the headers; at-risk once in the stat line. → clarify
5. [P2] Accessibility: title-only info (rule, stale reason, deadline) unreachable on touch/keyboard; the overflow-x-auto scroller (bits.tsx:224) has no tabIndex/role/label; "Move" header ambiguous. Contrast is fine (text-faint 5.85:1, muted 7.33:1 on bg-card). → harden

## Persona red flags
- Power user, second monitor: the one number is mid-sentence; the one warning is a single amber cell among 88; three price columns to scan for nothing.
- Operator on a phone: 7 lines of rule prose, then Coin/Side/Size/Entry, a "$5,000" total that reads like the result; swipe right loses coin names; tooltips do nothing.
- Skeptical reviewer: honest caveats served well, but an all-green column and green +3.10% on a falling price read as more confident than intended.

## Minor observations
- "$5,000" under Size isn't labelled gross notional.
- Group subtotal rows push P&L to a third line under the full rule.
- Uneven precision (80.1890 vs 70,203.0).
- No `empty` prop → panel returns null silently (positions.tsx:71).
- On the 768 tablet the sidebar leaves the panel ~440px.

## Questions
- One thing from this panel: book P&L or distance to the nearest stop? Make that the largest element.
- Do Entry, Mark and Stop need to be visible by default, given Move and To stop derive from them?
- Eight shorts from one strategy: a table of 8 positions, or one bet across 8 coins?
