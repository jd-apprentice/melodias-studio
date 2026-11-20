#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=env.sh
. "$SCRIPT_DIR/env.sh"

NAME=llm
BIN=$LLM_BIN
MODEL=$LLM_MODEL
PORT=$LLM_PORT
PIDFILE="$MELODIAS_RUN_DIR/melodias-$NAME.pid"
LOG="$MELODIAS_LOG_DIR/melodias-$NAME.log"

case "${1:-}" in
  start)
    if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
      echo "$NAME already running (pid $(cat "$PIDFILE"))"
      exit 0
    fi
    # shellcheck disable=SC2086
    setsid "$BIN" -m "$MODEL" \
      --host "$LLM_LISTEN_IP" --port "$PORT" \
      $LLM_ARGS \
      >"$LOG" 2>&1 </dev/null &
    echo $! >"$PIDFILE"
    echo "$NAME started (pid $(cat "$PIDFILE"))"
    ;;
  stop)
    if [ ! -f "$PIDFILE" ]; then
      echo "$NAME not running"
      exit 0
    fi
    pid=$(cat "$PIDFILE")
    kill -TERM -"$pid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true
    for _ in $(seq 1 20); do
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.5
    done
    rm -f "$PIDFILE"
    echo "$NAME stopped"
    ;;
  status)
    if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
      echo "$NAME running (pid $(cat "$PIDFILE"))"
    else
      echo "$NAME not running"
      exit 1
    fi
    ;;
  *)
    echo "usage: $0 start|stop|status"
    exit 2
    ;;
esac
