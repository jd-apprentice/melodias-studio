#!/usr/bin/env bash
set -euo pipefail

NAME=sd
BIN=/home/ubuntu/stable-diffusion.cpp/build/bin/sd-server
MOSS=/home/ubuntu/moss
PIDFILE=/run/melodias-sd.pid
LOG=/var/log/melodias-sd.log

case "${1:-}" in
  start)
    if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
      echo "$NAME ya corre (pid $(cat "$PIDFILE"))"
      exit 0
    fi
    setsid "$BIN" \
      --diffusion-model "$MOSS/MOSS-Image-SFT-UD-Q4_K_XL.gguf" \
      --llm "$MOSS/Qwen3.5-9B-Q8_0.gguf" \
      --vae "$MOSS/vae/diffusion_pytorch_model.safetensors" \
      --tokenizer "$MOSS/tokenizer/tokenizer.json" \
      --model-dir "$MOSS" \
      --listen-ip 0.0.0.0 --listen-port 1234 \
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
