# Configuration

Settings come from three places, in this order: environment variables, then `~/.config/melodias-studio/config.toml`, then the built-in defaults. `XDG_CONFIG_HOME` is respected and `MELODIAS_CONFIG` overrides the file path.

Start from the commented example:

```bash
mkdir -p ~/.config/melodias-studio
cp config.example.toml ~/.config/melodias-studio/config.toml
```

## Environment variables

| Variable | Default | Meaning |
| --- | --- | --- |
| `AUDIO_URL` | `http://192.168.88.38:8090` | `audiocpp_server` |
| `AUDIO_MODEL` | `ace-step` | model id registered on the server |
| `SD_URL` | `http://192.168.88.38:1234` | `sd-server` |
| `LLM_URL` | `http://192.168.88.38:8080` | `llama-server` |
| `LLM_MODEL` | `local` | model name the server expects |
| `SSH_HOST` | `root@192.168.88.38` | target for the reference and lifecycle commands; empty runs locally |
| `REMOTE_REF_DIR` | `/models/ref` | remote folder for references |
| `OUT_DIR` | `output` | local output |
| `GPU_LABEL` | `local models` | label in the header and the splash |
| `AUDIO_STYLE` | `8-bit chiptune` | fixed medium added to every audio prompt |
| `IMAGE_STYLE` | `8-bit pixel art` | style pinned in the image prompt system message |
| `DURATION_SEC` | `210` | take length |
| `STEPS` | `32` | diffusion steps |
| `GUIDANCE` | `5` | CFG |
| `SAMPLER` | `heun` | `euler` or `heun` |
| `WIDTH` / `HEIGHT` | `1280` / `720` | thumbnail size |
| `IMAGE_STEPS` | `20` | diffusion steps for the thumbnail |
| `IMAGE_SEED` | random | fixed seed to reproduce a thumbnail |
| `REFERENCE_SEC` | `60` | length of the timbre reference, clamped to 5..120 |
| `FADE_IN_SEC` | `1.5` | master fade-in |
| `FADE_OUT_SEC` | `3.0` | master fade-out |
| `LOUDNESS_LUFS` | `-15` | master loudness target |

The channel medium is a hard rule: the audio prompt always gets `AUDIO_STYLE`, even when the vibe does not mention it, and the image system message always pins `IMAGE_STYLE`. The vibe adds mood and references, not the medium.

An empty environment variable is ignored, so to run locally use `ssh_host = ""` in `config.toml` instead of `SSH_HOST=`.

## GPU lifecycle commands

They run between stages, over SSH when `SSH_HOST` is set. The defaults point at the `deploy/` scripts; override or empty them to manage the servers by hand:

| Variable | Default | When it runs |
| --- | --- | --- |
| `LLM_START_CMD` | `/home/ubuntu/bin/serve-llm.sh start` | before the thumbnail prompt |
| `LLM_STOP_CMD` | `/home/ubuntu/bin/serve-llm.sh stop` | before the audio stage, to free 3.4 GB |
| `SD_START_CMD` | `/home/ubuntu/bin/serve-sd.sh start` | before the thumbnail |
| `SD_STOP_CMD` | `/home/ubuntu/bin/serve-sd.sh stop` | before the audio stage |
| `AUDIO_START_CMD` | `/home/ubuntu/bin/serve-audio.sh start` | before the audio stage |
| `AUDIO_STOP_CMD` | `/home/ubuntu/bin/serve-audio.sh stop` | after the take, even if it failed |
| `AUDIO_NEGATIVE_PROMPT` | `vocals, singing, voice, choir, lyrics, humming, vocal chops, spoken word` | steering text that keeps voices out |

The app waits for `/health` on the service it just started, up to 180 s, before moving on. With `SSH_HOST` empty the reference is not copied and the audio server reads it from the local path.
