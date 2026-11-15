# Lab (`src/lab/`)

- Archive: Binance USDT-M 15m (from 2021-01-01) as research proxy + Hyperliquid 15m/1h (only the latest 5000 candles). Costs always Hyperliquid tier 0 (`[costs]` in lab.toml).
- Universe: 20 fixed coins listed on both venues on 2026-11-11; survivors only (delisted coins absent).
- Binary: `target/lab/jev-trader` (profile `lab`, opt-level 2; bootstraps are slow unoptimized).
- Commands: DESIGN-V3.md §8. Reports: `lab-data/reports/<study>/`, loop reports `lab-data/reports/evolve/gNNN.md`, `auto-<time>.md`.
- The gate is conservative on purpose: a real Sharpe 1-2 edge can be rejected on this data. The way through is fewer, better-motivated trials, not looser thresholds.
- For development and tests use a scratch `data_dir`, never the real `lab-data/`.
- Generated families' walk-forward folds are not truly out of sample; lockbox and paper days are their only clean evidence.

## Slow trend tested (2026-11-14, at the user's request)
- H-L05 `families/slow_trend.toml` and H-L06 `families/slow_trend_uncrowded.toml`: both REJECT, 12 trials charged to binance-1h. Return comes from a few bull runs (fails best-days-removed and quarters), not costs. The funding cap helped a little (walk-forward +3 vs 0 bps/day). Details in hypotheses.toml.
- Vol-targeted sizing and a market-wide funding feature were left out: both widen the closed library, so they need their own hypothesis first.
