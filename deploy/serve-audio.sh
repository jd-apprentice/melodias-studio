#!/usr/bin/env bash
set -euo pipefail

NAME=audio
BIN=/root/audio.cpp/build/linux-cuda-release/bin/audiocpp_server
CONFIG=/home/ubuntu/audio-server.json
PIDFILE=/run/melodias-audio.pid
LOG=/var/log/melodias-audio.log

case "${1:-}" in
  start)
    if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
      echo "$NAME ya corre (pid $(cat "$PIDFILE"))"
      exit 0
    fi
    setsid "$BIN" --config "$CONFIG" >"$LOG" 2>&1 </dev/null &
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
