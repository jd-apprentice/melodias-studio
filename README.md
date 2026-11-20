# melodias-studio

A Rust TUI that turns a YouTube song into two deliverables for the channel: **one thumbnail** and **one three-minute instrumental track**, using only models that run on the home P40.

```
YouTube URL + vibe
        │
        ├─ llama-server ───► thumbnail prompt (needs the GPU)
        ├─ sd-server ──────► 1280x720 thumbnail
        ├─ yt-dlp ─────────► source audio
        ├─ ffmpeg ─────────► 60 s excerpt with the most energy (timbre reference)
        ├─ audiocpp_server ► 210 s take with reference_audio
        └─ ffmpeg ─────────► 3 s loop + loudnorm -15 LUFS
```

The order is deliberate: the image runs first because it is the most fragile stage, and the download plus audio form one block that only needs the audio server.

## Requirements

- Rust 1.85+ (`rustup default stable`).
- `yt-dlp`, `ffmpeg`, `ffprobe`, `scp` and `ssh` on `PATH`.
- Three services on the GPU VM, started by the scripts in [`deploy/`](deploy/):
  - `llama-server` with `Qwen3-4B-Instruct-2507` on `:8080`
  - `sd-server` with MOSS-Image on `:1234`
  - `audiocpp_server` with ACE-Step on `:8090`
- Passwordless SSH to the VM, used to stage the audio reference and to run the lifecycle commands.

The P40 has 24 GB. A 4B model only keeps ~3.4 GB busy, so the prompt step and `sd-server` (11.4 GB) can hold the GPU at the same time. The audio stage stops both `sd-server` and the LLM: ACE-Step with a timbre reference needs the whole card. `deploy/audio-server.json` enables `ace_step.mem_saver`, which returns the session to ~14.7 GB between requests.

## Deploy the server scripts

```bash
ssh root@192.168.88.38 'mkdir -p /home/ubuntu/bin'
scp deploy/serve-*.sh root@192.168.88.38:/home/ubuntu/bin/
scp deploy/audio-server.json root@192.168.88.38:/home/ubuntu/
ssh root@192.168.88.38 'chmod +x /home/ubuntu/bin/serve-*.sh'
```

Each script answers `start`, `stop` and `status`, keeps a pidfile in `/run` and a log in `/var/log`. Paths inside them assume the layout used on this VM: llama.cpp and stable-diffusion.cpp built under `/home/ubuntu`, audio.cpp under `/root`, models under `/models`.

## Usage

```bash
cargo run
```

- `Tab` switches between the URL and the vibe.
- `Enter` moves to the next field, then starts the run.
- `q` quits, only once the run finished.

Each stage shows its state, the progress bar tracks how many are done, and the log prints every step, including the prompt the LLM wrote and the seed of the take.

### Headless run

Passing the URL and the vibe as arguments runs the same pipeline without the full screen UI. Each stage is printed as its own colored group with an icon, and tool output from `yt-dlp`, `ffmpeg` and `ssh` goes to the run log instead of the screen:

```bash
MELODIAS_LOG=output/run.log cargo run -- 'https://www.youtube.com/watch?v=...' 'epic 8-bit orchestral finale'
```

The log receives the same lines as plain text, so it can be read later without escape codes. Quote the URL: a YouTube link copied from a playlist carries `&list=` and `&index=`, which the shell would otherwise split. The exit code is non-zero when a stage fails.

The image stage reports its own duration and the size of the PNG it got back, and warns when the server returned something other than the requested size.

### Make shortcuts

The `Makefile` wraps the common commands:

| Target | What it does |
| --- | --- |
| `make run` | Opens the TUI |
| `make generate URL='...' VIBE='...'` | Runs headless and writes `output/run-<timestamp>.log` |
| `make log` | Prints the last 40 lines of the newest log |
| `make check` | `fmt --check`, build, `clippy -D warnings` and tests |
| `make status` | Service state on the VM over SSH |
| `make deploy` | Copies `deploy/` to the VM and fixes permissions |

`make generate` builds the release binary first, so the run does not pay debug overhead.

## Configuration

Everything is read from the environment, with defaults pointing at the VM:

| Variable | Default | Meaning |
| --- | --- | --- |
| `AUDIO_URL` | `http://192.168.88.38:8090` | `audiocpp_server` |
| `AUDIO_MODEL` | `ace-step` | model id registered on the server |
| `SD_URL` | `http://192.168.88.38:1234` | `sd-server` |
| `LLM_URL` | `http://192.168.88.38:8080` | `llama-server` |
| `LLM_MODEL` | `local` | model name the server expects |
| `SSH_HOST` | `root@192.168.88.38` | destination for the reference and lifecycle commands |
| `REMOTE_REF_DIR` | `/models/ref` | remote folder for references |
| `OUT_DIR` | `output` | local output |
| `DURATION_SEC` | `210` | take length |
| `STEPS` | `32` | diffusion steps |
| `GUIDANCE` | `5` | CFG |
| `SAMPLER` | `heun` | `euler` or `heun` |
| `WIDTH` / `HEIGHT` | `1280` / `720` | thumbnail size |

### GPU lifecycle commands

They run over SSH between stages. The defaults point at the `deploy/` scripts; override or empty them to manage the servers by hand:

| Variable | Default | When it runs |
| --- | --- | --- |
| `LLM_START_CMD` | `/home/ubuntu/bin/serve-llm.sh start` | before the thumbnail prompt |
| `LLM_STOP_CMD` | `/home/ubuntu/bin/serve-llm.sh stop` | before the audio stage, to free 3.4 GB |
| `SD_START_CMD` | `/home/ubuntu/bin/serve-sd.sh start` | before the thumbnail |
| `SD_STOP_CMD` | `/home/ubuntu/bin/serve-sd.sh stop` | before the audio stage |
| `AUDIO_START_CMD` | `/home/ubuntu/bin/serve-audio.sh start` | before the audio stage |
| `AUDIO_STOP_CMD` | `/home/ubuntu/bin/serve-audio.sh stop` | after the take, even if it failed |
| `AUDIO_NEGATIVE_PROMPT` | `vocals, singing, voice, choir, lyrics, humming, vocal chops, spoken word` | steering text that keeps voices out |

The app waits for `/health` on the service it just started, up to 180 s, before moving on.

## Output

```
output/<slug>/
├── audio.mp3      # 3 min loop, 320 kbps, -15 LUFS
├── thumbnail.png  # 1280x720
├── meta.json      # seed, prompts, parameters and source URL
└── work/          # source, reference and raw take
```

## Architecture diagram

[![Pipeline architecture](docs/arquitectura.png)](docs/arquitectura.html)

[`docs/arquitectura.html`](docs/arquitectura.html) is a self-contained interactive map of the pipeline, the three GPU services and the VRAM order. It opens offline in a browser and needs no server. The source is [`docs/arquitectura.json`](docs/arquitectura.json), so the diagram can be regenerated with the [Archify](https://github.com/tt-a1i/archify) skill:

```bash
node ~/.agents/skills/archify/bin/archify.mjs finalize architecture docs/arquitectura.json docs/arquitectura.html --repo-root . --quality showcase
```

## Design notes

- **The reference is an excerpt, not the whole song.** ACE-Step samples three 10 s windows from the file, so the app picks the minute with the highest average energy and skips intros and breakdowns. It conditions timbre and style, not the melody: the result is not a cover of the source.
- **Every track is instrumental.** The vibe is sent as `<vibe>, instrumental` and `options.negative_prompt` lists vocals, singing, choir, lyrics and humming. ACE-Step uses that text as the unconditional CFG prompt, so it steers away from voices instead of hoping the caption is ignored.
- **The audio parameters travel inside `options`.** `/v1/tasks/run` only forwards the nested `options` object to the ACE-Step parser, so `duration_sec`, `reference_audio` and `sampler_mode` at the top level of `request` are silently dropped. Sent inside `options`, `duration_sec` returns an exact length.
- **The loop is built in ffmpeg.** A 3 s crossfade between the end of the outro and the start of the intro. It does not depend on the model or on its tempo.
- **The LLM writes the thumbnail prompt.** The vibe goes in as a concept and comes back as an English prompt, with the pixel-art style pinned in the system prompt.
- **Stages fail loudly and stop the run.** Each step reports its own error in the log so you know which service to look at.

## Roadmap

- [ ] Tempo detection and bar-aligned cuts instead of a fixed 3 s crossfade.
- [ ] Cancel a run in progress and retry a single stage.
- [ ] Read the video title to name the output folder instead of the vibe.
- [ ] Config file at `~/.config/melodias-studio/config.toml`.
- [ ] Write the video entry straight into `jd-llm/projects/melodias8bit/videos/`.
- [ ] Compare two takes side by side before picking one.
