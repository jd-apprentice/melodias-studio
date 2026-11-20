#!/usr/bin/env bash
set -euo pipefail

NAME=llm
BIN=/home/ubuntu/llama.cpp/build/bin/llama-server
MODEL=/models/qwen3-4b-instruct/Qwen3-4B-Instruct-2507-Q4_K_M.gguf
PIDFILE=/run/melodias-llm.pid
LOG=/var/log/melodias-llm.log

case "${1:-}" in
  start)
    if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
      echo "$NAME ya corre (pid $(cat "$PIDFILE"))"
      exit 0
    fi
    setsid "$BIN" -m "$MODEL" \
      --host 0.0.0.0 --port 8080 \
      -c 8192 -ngl 99 -fa on -ctk q8_0 -ctv q8_0 --jinja \
      >"$LOG" 2>&1 </dev/null &
    echo $! >"$PIDFILE"
    echo "$NAME arrancado (pid $(cat "$PIDFILE"))"
    ;;
  stop)
    if [ ! -f "$PIDFILE" ]; then
      echo "$NAME no corre"
      exit 0
    fi
    pid=$(cat "$PIDFILE")
    kill -TERM -"$pid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true
    for _ in $(seq 1 20); do
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.5
    done
    rm -f "$PIDFILE"
    echo "$NAME detenido"
    ;;
  status)
    if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
      echo "$NAME corriendo (pid $(cat "$PIDFILE"))"
    else
      echo "$NAME detenido"
      exit 1
    fi
    ;;
  *)
    echo "uso: $0 start|stop|status"
    exit 2
    ;;
esac
