#!/usr/bin/env bash
# One hourly evolve tick, run by cron: bring the Binance archive up to date, then
# `lab evolve --auto` until its stop rules. Never promotes; never trades.
# The loop's trial budget ([evolve] auto_trial_budget, per study) counts every run
# in the lineage, so once it is spent each tick stops at once with "nothing new".
#   crontab: 17 * * * * /path/to/trading-v3/scripts/v3-evolve-hourly.sh
set -uo pipefail
cd "$(dirname "$0")/.."
mkdir -p data-v3
exec >> data-v3/evolve.log 2>&1
exec 9> data-v3/evolve.lock
flock -n 9 || { echo "$(date -u +%FT%TZ) previous tick still running, skipped"; exit 0; }
echo "=== $(date -u +%FT%TZ) evolve tick"
nice -n 10 target/lab/jev-trader lab fetch --venue binance 2>&1 | tail -1
nice -n 10 target/lab/jev-trader lab evolve --auto; rc=$?
echo "=== $(date -u +%FT%TZ) done (exit $rc)"
