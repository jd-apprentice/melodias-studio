#!/usr/bin/env bash
# Host layout for the three services. Override any value from the environment
# before running a script, or edit this file on the host after deploying it.
# shellcheck shell=bash

: "${MELODIAS_RUN_DIR:=/run}"
: "${MELODIAS_LOG_DIR:=/var/log}"

# llama.cpp
: "${LLM_BIN:=/home/ubuntu/llama.cpp/build/bin/llama-server}"
: "${LLM_MODEL:=/models/qwen3-4b-instruct/Qwen3-4B-Instruct-2507-Q4_K_M.gguf}"
: "${LLM_LISTEN_IP:=0.0.0.0}"
: "${LLM_PORT:=8080}"
: "${LLM_ARGS:=-c 8192 -ngl 99 -fa on -ctk q8_0 -ctv q8_0 --jinja}"

# stable-diffusion.cpp
: "${SD_BIN:=/home/ubuntu/stable-diffusion.cpp/build/bin/sd-server}"
: "${MOSS_DIR:=/home/ubuntu/moss}"
: "${SD_DIFFUSION_MODEL:=$MOSS_DIR/MOSS-Image-SFT-UD-Q4_K_XL.gguf}"
: "${SD_LLM:=$MOSS_DIR/Qwen3.5-9B-Q8_0.gguf}"
: "${SD_VAE:=$MOSS_DIR/vae/diffusion_pytorch_model.safetensors}"
: "${SD_TOKENIZER:=$MOSS_DIR/tokenizer/tokenizer.json}"
: "${SD_LISTEN_IP:=0.0.0.0}"
: "${SD_PORT:=1234}"

# audio.cpp
: "${AUDIO_BIN:=/root/audio.cpp/build/linux-cuda-release/bin/audiocpp_server}"
: "${AUDIO_CONFIG:=/home/ubuntu/audio-server.json}"
