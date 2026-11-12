#!/usr/bin/env bash
# Start, stop or check the two v3 services: the paper trader and the web dashboard.
#   scripts/v3-services.sh start|stop|status [trader|web]
# Both run detached and write their pid to data-v3/. Paper only: nothing here can place a real order.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=target/lab/jev-trader
ENV_FILE=${JEV_ENV_FILE:-../trading/.env.local}
PORT=${JEV_WEB_PORT:-8787}
DATA=data-v3

alive() { [ -f "$1" ] && kill -0 "$(cat "$1")" 2>/dev/null; }

start_trader() {
  if alive $DATA/trade.pid; then echo "trader: already running (pid $(cat $DATA/trade.pid))"; return; fi
  [ -x $BIN ] || { echo "trader: $BIN is missing; build it with: cargo build --profile lab -j 2" >&2; return 1; }
  mkdir -p $DATA
  setsid nohup ./$BIN --env-file "$ENV_FILE" trade run >> $DATA/trade.log 2>&1 < /dev/null &
  sleep 2
  alive $DATA/trade.pid && echo "trader: started (pid $(cat $DATA/trade.pid))" || { echo "trader: did not start; see $DATA/trade.log" >&2; return 1; }
}

start_web() {
  if alive $DATA/web.pid; then echo "web: already running (pid $(cat $DATA/web.pid)) on http://localhost:$PORT"; return; fi
  [ -d web/node_modules ] || (cd web && pnpm install)
  [ -f web/.next/BUILD_ID ] && [ -z "${JEV_WEB_REBUILD:-}" ] || (cd web && pnpm exec next build)
  mkdir -p $DATA
  (
    cd web
    setsid nohup node node_modules/next/dist/bin/next start -p "$PORT" >> ../$DATA/web.log 2>&1 < /dev/null &
    echo $! > ../$DATA/web.pid
  )
  sleep 2
  alive $DATA/web.pid && echo "web: started (pid $(cat $DATA/web.pid)) on http://localhost:$PORT" || { echo "web: did not start; see $DATA/web.log" >&2; return 1; }
}

stop_one() { # name pidfile
  if alive "$2"; then kill "$(cat "$2")" && echo "$1: stopped (pid $(cat "$2"))"; else echo "$1: not running"; fi
}

status_one() { # name pidfile
  if alive "$2"; then echo "$1: running (pid $(cat "$2"))"; else echo "$1: stopped"; fi
}

cmd=${1:-status}
what=${2:-all}
for svc in trader web; do
  [ "$what" = all ] || [ "$what" = "$svc" ] || continue
  pidfile=$DATA/$([ $svc = trader ] && echo trade || echo web).pid
  case $cmd in
    start) start_$svc ;;
    stop) stop_one $svc $pidfile ;;
    status) status_one $svc $pidfile ;;
    *) echo "usage: $0 start|stop|status [trader|web]" >&2; exit 2 ;;
  esac
done
