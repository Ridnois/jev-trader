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

# Launch detached. Prefer a transient systemd user unit: a plain setsid child still lives in the
# caller's cgroup and is killed when that terminal or agent session is torn down (seen 2026-11-14).
detach() { # unit-name logfile cmd...
  local unit=$1 log=$2; shift 2
  if command -v systemd-run >/dev/null && systemctl --user is-system-running >/dev/null 2>&1; then
    systemd-run --user --quiet --collect --unit="$unit" --same-dir \
      -p StandardOutput="append:$(realpath -m "$log")" -p StandardError="append:$(realpath -m "$log")" "$@"
    systemctl --user show -p MainPID --value "$unit"
  else
    setsid nohup "$@" >> "$log" 2>&1 < /dev/null &
    echo $!
  fi
}

alive() { [ -f "$1" ] && kill -0 "$(cat "$1")" 2>/dev/null; }

start_trader() {
  if alive $DATA/trade.pid; then echo "trader: already running (pid $(cat $DATA/trade.pid))"; return; fi
  [ -x $BIN ] || { echo "trader: $BIN is missing; build it with: cargo build --profile lab -j 2" >&2; return 1; }
  mkdir -p $DATA
  detach v3-trader $DATA/trade.log ./$BIN --env-file "$ENV_FILE" trade run > /dev/null
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
    detach v3-web ../$DATA/web.log node node_modules/next/dist/bin/next start -p "$PORT" > ../$DATA/web.pid
  )
  sleep 2
  alive $DATA/web.pid && echo "web: started (pid $(cat $DATA/web.pid)) on http://localhost:$PORT" || { echo "web: did not start; see $DATA/web.log" >&2; return 1; }
}

stop_one() { # name pidfile unit
  if ! alive "$2"; then echo "$1: not running"; return; fi
  local pid; pid=$(cat "$2")
  kill "$pid"
  for _ in $(seq 20); do kill -0 "$pid" 2>/dev/null || break; sleep 0.5; done
  # next-server can ignore SIGTERM; its systemd unit can still stop it.
  if kill -0 "$pid" 2>/dev/null && systemctl --user is-active --quiet "$3" 2>/dev/null; then systemctl --user stop "$3"; fi
  if kill -0 "$pid" 2>/dev/null; then echo "$1: pid $pid did not stop" >&2; return 1; fi
  echo "$1: stopped (pid $pid)"
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
    stop) stop_one $svc $pidfile v3-$svc ;;
    status) status_one $svc $pidfile ;;
    *) echo "usage: $0 start|stop|status [trader|web]" >&2; exit 2 ;;
  esac
done
