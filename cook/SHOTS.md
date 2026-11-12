# Shots

One per page and state. No image references; the target is written as the reading a newcomer must get. Every still is 1440x900 and 390x844 unless noted.

| Shot | Page | Must read in the first screen | State shown |
|---|---|---|---|
| S01 | `/` Now | What this product is, that money is paper, whether the engine is alive, whether anything needs the user, and where each of the four stations stands | Live: engine running 20m, 2 unproven rules, 0 full days, 539 headlines, probes inconclusive |
| S02 | `/` Now, engine stopped | Same, but the status sentence says what stopped and what it means for the paper book; the command to restart is visible | Synthesised from the stopped branch of `engineState` |
| S03 | `/start` | The whole lifecycle in one screen: four stations, what each proves, in what order, and what the user may and may not do | Static copy |
| S04 | `/paper` | The sequential test in plain words: what is being tested, what the line means, how far it has to go, and that 0 full days means no evidence yet | Live: 2 strategies, 0 full days |
| S05 | `/lab` | Nothing passed, that this is the result, how many families, and what the gate map is | Live: 4 rejected + 3 insufficient, lockbox sealed |
| S06 | `/catalysts` (News recorder) | The question, the evidence collected so far and how far from an answer; the table is readable, not a 15k-px dump | Live: 2 days, 24 triggered, 539 headlines |
| S07 | `/probes` | Jev's read is not yet trustworthy, which questions were asked, and what follows | Live: 3 questions, none clears the bar |
| S08 | `/markets` | These are reference quotes, not the paper book's fills; freshness per coin | Live: 20 coins |
| S09 | Mobile `/` | The same reading in one column, nav reachable, no horizontal scroll | Live |
| S10 | Every page, mid-scroll | Section headers, table headers and footnotes stay legible; no column under the wrong label | Live |

## What to take from each existing screen

- S01: keep the four stat facts (engine, equity, test, headlines) but demote them into one status band; the paper chart must not dominate a first visit.
- S04: keep `SequentialChart`; it is the strongest existing idea. Add the plain verdict above it and the decision horizon in words.
- S05: keep `GateLineMap`; it is the signature move. Add a plain-language verdict band and fix the family coverage.
- S06: keep the trigger-vs-all comparison; it is the page's insight. Cut the 200-row dump to a first page of 50 with progressive disclosure.
- S07: keep the reliability plot and paired "Jev said / happened" rows; add a page-level verdict.
- S08: keep the range bar; add the distinction between recorder quotes and bar-close decisions.
