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
| `IMAGE_NEGATIVE_PROMPT` | `glowing eyes, neon eyes, empty eyes, oversaturated eyes, text, watermark, signature, extra fingers, deformed hands` | negative prompt sent to sd-server |
| `DURATION_SEC` | `210` | take length |
| `STEPS` | `32` | diffusion steps |
| `GUIDANCE` | `5` | CFG |
| `SAMPLER` | `heun` | `euler` or `heun` |
| `WIDTH` / `HEIGHT` | `1280` / `720` | thumbnail size |
| `VIDEO_WIDTH` / `VIDEO_HEIGHT` | `1920` / `1080` | final video canvas |
| `JUDGE_ENABLED` | `true` | runs the judge when the provider has a key |
| `JUDGE_PROVIDER` | `auto` | `opencode`, `openrouter` or `auto` (OpenCode when its key is set) |
| `OPENCODE_API_KEY` | empty | OpenCode Go key; the default backend |
| `OPENROUTER_API_KEY` | empty | alternative backend; export it in the shell that runs `melodias` |
| `JUDGE_MODEL` | `gemini-4-argon` | OpenCode Go model, native Gemini API |
| `JUDGE_OMNI_MODEL` | `nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free` | OpenRouter perception model |
| `JUDGE_REASONER_MODEL` | `nvidia/nemotron-3-ultra-550b-a55b:free` | OpenRouter text-only final choice |
| `JUDGE_IMAGE_PX` | `768` | JPEG size sent to the judge |
| `JUDGE_AUDIO_SEC` | `15` | clip length sent to the judge |
| `JUDGE_TIMEOUT_SEC` | `300` | per-call timeout |
| `IMAGE_STEPS` | `20` | diffusion steps for the thumbnail |
| `IMAGE_SEED` | random | fixed seed to reproduce a thumbnail |
| `CANDIDATES` | `3` | candidates per modality |
| `AUDIO_DRAFT_SEC` | `60` | length of each audio draft |
| `AUDIO_DRAFT_STEPS` | `16` | steps for the audio drafts |
| `AUDIO_DRAFT_SAMPLER` | `euler` | sampler for the audio drafts |
| `REFERENCE_SEC` | `60` | length of the timbre reference, clamped to 5..120 |
| `FADE_IN_SEC` | `1.5` | master fade-in |
| `FADE_OUT_SEC` | `3.0` | master fade-out |
| `LOUDNESS_LUFS` | `-15` | master loudness target |

The judge is optional: it runs when `JUDGE_ENABLED` is not `false` and the selected provider has a key. It compresses the candidates to 768 px JPEGs and 15 s mono clips, sends them to a model that can see and hear, and preselects the best pair in the review; `Enter` accepts it. If the key is missing, the API fails or the JSON is invalid, the run falls back to the manual pick and logs it.

`JUDGE_PROVIDER=auto` uses OpenCode Go when `OPENCODE_API_KEY` is set and OpenRouter otherwise. The OpenCode backend is a single `generateContent` call to `gemini-4-argon`, which accepts images and MP3 in the same request (about 20 s and $0.019 of list cost per run, drawn from the Go monthly allowance). The OpenRouter backend is two stages: the free omni model perceives, the free text model decides. OpenRouter requires at least **$0.50 of balance for audio inputs**, even on a free model; image-only requests work without balance.

Keys are read from the environment of the process that runs `melodias` (or from `opencode_api_key` / `openrouter_api_key` in this file). They do not belong in the `video/` Remotion project, which only renders local assets.

#### What the judge costs

Measured on real runs with `gemini-4-argon`; the gateway reports the debit in `usage.opencode.allowance`, and `work/judge/verdict.json` stores it:

| Pool | Input tokens | Output | Allowance | List price | Time |
| --- | --- | --- | --- | --- | --- |
| 3 images + 3 clips | 5,303 (3,300 image, 1,353 audio, 650 text) | 517 | $0.0316 | $0.0158 | ~30 s |
| 4 images + 3 clips | 6,384 | 632 | $0.0382 | $0.0191 | ~20 s |

Each extra image adds about 1,100 input tokens and one row per clip in the verdict; each extra 15 s clip adds about 450. The debit follows `(650 + 1100·images + 451·clips) × $4/M + (175 + 38·images·clips) × $20/M`.

Go includes $30 of Argon usage per month ($120 on Go Plus), so the default 3 + 3 pool is about 950 runs per month, 190 per five-hour window and 475 per week. Those caps are shared with every other Go model, so a month of daily coding spends more than the judge. The advertised list price is half the debit until November 30, 2026; from December 1 the two match, and the debit is the number that decides when requests start failing.

A run costs about $0.03 of the allowance for 20 to 30 s of judge time, next to roughly 36 minutes of GPU work.

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
