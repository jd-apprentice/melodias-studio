#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=env.sh
. "$SCRIPT_DIR/env.sh"

NAME=sd
BIN=$SD_BIN
PIDFILE="$MELODIAS_RUN_DIR/melodias-$NAME.pid"
LOG="$MELODIAS_LOG_DIR/melodias-$NAME.log"

case "${1:-}" in
  start)
    if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
      echo "$NAME already running (pid $(cat "$PIDFILE"))"
      exit 0
    fi
    setsid "$BIN" \
      --diffusion-model "$SD_DIFFUSION_MODEL" \
      --llm "$SD_LLM" \
      --vae "$SD_VAE" \
      --tokenizer "$SD_TOKENIZER" \
      --model-dir "$MOSS_DIR" \
      --listen-ip "$SD_LISTEN_IP" --listen-port "$SD_PORT" \
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
