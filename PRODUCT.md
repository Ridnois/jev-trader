# Product

## Platform
web (local dashboard served by the Rust binary: `jev-trader dashboard`, axum + vanilla JS, no Node toolchain)

## Users
A solo operator running jev-trader v2 on their own machine; the dashboard is a tab left open, glanced at, examined when something happens.

## Purpose
One engine on Hyperliquid perps where Jev's news reading and order flow must agree before money moves. The page must answer at a glance: is it alive, whose money (paper / testnet / live), what is open and why (headline), what each coin is waiting for, what Jev is reading and what it costs in tokens, and whether the evidence says it pays.

## Principles
1. Alive or not, at a glance. 2. Every trade traces to a headline or a news bias. 3. Safety controls only (Pause entries, Flatten all); strategy lives in config files. 4. Honest numbers: paper/testnet/live labelled, "too little data" shown as such. 5. No claims of edge until `report` shows one.
