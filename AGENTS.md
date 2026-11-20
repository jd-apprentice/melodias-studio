# melodias-studio

Rust TUI plus CLI (binary `melodias`) for a YouTube channel: a reference URL and a vibe produce `output/<slug>/` with `thumbnail.png`, `audio.mp3`, `video.mp4`, `meta.json`, `candidates/` and `work/`. No args opens the TUI; `<url> <vibe>` runs the same pipeline headless; `image|judge|score|history|dashboard|render|video` act on a finished run.

## Commands

- Rust 1.85+ (edition 2024). `make check` is the full gate: `cargo fmt --check`, `cargo build`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. There is no CI, so run it before finishing.
- Single test: `cargo test <name>`. Unit tests live in `src/tests/<module>.rs` and are pulled in by a `#[cfg(test)] #[path = "tests/<module>.rs"] mod tests;` stub; they need no services, GPU or network. `cargo fmt` follows the stubs, so `make check` covers them.
- `make run` (TUI), `make generate URL='...' VIBE='...'` (release build + headless, log in `output/run-<timestamp>.log`), `make log`.
- `make backfill [ARGS=--replace]` scores the finished runs in `output/` into `~/.config/melodias-studio/history.db` (`MELODIAS_DB` overrides it); it needs `jq`, `sqlite3`, ImageMagick and `tesseract`.
- `make history [ARGS='findings']` reads that database (`melodias history [runs|findings|trend] [--json|--csv] [--by day|week|month]`). It never writes; the pipeline appends a row at the end of every run.
- `make dashboard` serves the read-only charts at `http://127.0.0.1:7373/`; `melodias dashboard [--port N] [--limit N]` is the same without the release build. It reads the database on every request and never writes it.
- `SCORE_ROUNDS` defaults to 2 and caps the deterministic retry rounds per side (budgets: 2 image, 1 audio); set it to `0` to disable the loop.
- `make image|judge|score|render|video DIR=output/<slug>` map to the subcommands. `video` only rebuilds the ffmpeg video; `image` regenerates a thumbnail through sd-server and rebuilds that video; `render` needs `cd video && pnpm install` and a run from the repo root (Remotion, about 12 min); `judge` needs an API key.
- The screenshots in `docs/` come from the test backend, not a manual capture: `DUMP_TUI_HTML_DIR=/tmp/tui cargo test draws_without_leaving_the_terminal`. `PRINT_TUI=1` prints the buffer as ASCII.
- `make deploy` and `make status` need SSH access to the GPU host; the rest runs locally.

## Wiring

- `src/pipeline.rs` owns the 11-stage order in `STAGES` (prompt, prepare, image pool, download, reference, upload, drafts, review, final take, master, video). The TUI and headless reporter share the `Msg` channel and answer the review through the `Sender<Selection>` returned by `pipeline::spawn`. Adding a stage touches `STAGES` and `run()`, plus the hardcoded `1/11` assertions in the test in `src/main.rs`. The retry loop lives inside stages 2 and 6: image rounds run while sd-server is up, the audio round while ACE-Step is up.
- Entry points: `src/main.rs` dispatches TUI/headless/subcommands and is the headless reporter; `src/app.rs` is the TUI; `src/judge.rs` is the optional cloud judge; `src/score.rs` is the deterministic rubric (`score-v1`) behind `melodias score`, which writes `work/score.json`; `src/retry.rs` is the fixed patch table, the finding picker and the prompt application; `src/history.rs` owns the SQLite schema (v2), the insert at the end of a run and the read views behind `melodias history`; `src/dashboard.rs` serves the read-only page with the assets in `dashboard/` embedded at compile time; `src/clients.rs` speaks to llama-server, sd-server and audiocpp; `src/media.rs` shells out to `yt-dlp`, `ffmpeg`, `ffprobe`, `ssh` and `scp`.
- Config precedence is env var > `config.toml` (`MELODIAS_CONFIG`, else `$XDG_CONFIG_HOME/melodias-studio/config.toml`) > defaults. The judge is enabled by default but no-ops without a key; `auto` prefers `OPENCODE_API_KEY`, otherwise `OPENROUTER_API_KEY`. A failed judge is logged and does not abort the run.
- The default judge is one `generateContent` call to `gemini-4-argon` through OpenCode Go: it sees 768 px JPEGs, hears 15 s MP3s, and receives the deterministic findings so it does not repeat them. It returns the verdict plus up to five `must_fix` codes, kept in `work/judge/verdict.json` and `meta.json`. `JUDGE_PROVIDER=openrouter` keeps the older two-stage path with the free omni and reasoner models. `work/judge/verdict.json` records provider, model, usage, the reports and the selection; the measured cost is in `docs/configuration.md`.
- Headless mode accepts the judge suggestion when present; without one it picks the best-scored pair. The TUI starts the review the same way and still allows `1-9`/`Tab`/`Enter` overrides.
- Child process output goes to the run log through `media::set_child_log` (TUI: `output/run-<unix>.log`, headless: `MELODIAS_LOG`). Use `attach_output`/`attach_stderr` for new subprocesses so they do not fight the TUI.
- `docs/architecture/arquitectura.html` is generated from `docs/architecture/arquitectura.json` with the archify skill (command in `docs/architecture/README.md`); edit the JSON, not the HTML. `.archify/` is gitignored.

## Gotchas

- Empty environment variables are ignored (`config.rs::env` filters them), so `SSH_HOST=` does not switch to local mode: it falls back to the file and then to `root@192.168.88.38`. Disable SSH and lifecycle commands in `config.toml` (`ssh_host = ""`, `*_cmd = ""`).
- `media::preflight` requires `yt-dlp`, `ffmpeg`, `ffprobe` and `scp` on `PATH`, including `scp` when `SSH_HOST` is empty. Remotion adds a `pnpm` requirement.
- ACE-Step only reads the nested `options` object: `duration_sec`, `reference_audio`, `sampler_mode` and `negative_prompt` are silently dropped at the top level of `request`.
- A thumbnail size mismatch is only a warning; `media::png_size` validates the PNG header, not the pixels, so trust the log or `meta.json` for the real size.
- The run is not resumable and a failed stage aborts it. `output/` and `target/` are gitignored; generated `docs/` images and HTML are committed. Candidate images and drafts live in `output/<slug>/candidates/`.
- `melodias render` resolves `video/` from the current directory, copies the still and track into ignored `video/public/current/`, writes ignored `video/props.json`, and keeps a previous `video.mp4` as `video-ffmpeg.mp4`.
- `melodias judge` only prints the verdict and writes `work/judge/verdict.json`; it does not update `meta.json`. The OpenCode Go endpoint needs `x-opencode-session` and a non-generic User-Agent, otherwise it answers 400 `MissingSessionID`. OpenRouter audio input needs at least $0.50 of balance.
- The retry loop picks `medium` or worse (delta <= -4), not only `high`: the audio metrics top out at 8 points, so a strict high filter would never reach the audio patches. The image rounds run before the audio stage because of the GPU lifecycle; the per-side budgets are 2 and 1, and the pool stops at 9 per side so `1-9` still selects everything in the review.
- The same patch key is never applied twice; `AUD_HIGHS` and `AUD_CENTROID` share `AUD_TIMBRE`. `patches.prompt_before` and `prompt_after` hold the positive and negative prompt as a JSON object.
- History schema v2 adds `runs.outcome` and `runs.outcome_reason`. `open()` migrates old files on the next write; readers tolerate the missing columns. A run that misses the target after the loop is `accepted_by_limit`, not a failure. The backfill writes `below_target` for legacy rows.
- `melodias image` regenerates from the selected candidate's stored `prompt` and `negative_prompt` (falling back to the global ones), so a patched retry is reproducible without the config file.
- The history write happens after the video and re-scores the run with `ffmpeg` (about 20 s more). It is best effort: a failure is one log line and the run stays valid. The write also refreshes `work/score.json`. `melodias history` on a machine without the file prints a hint and exits 0; it never creates the database.
- `scripts/backfill-history.sh` writes the same tables as the pipeline but attaches findings to the selected pool candidate, while the pipeline stores the final pair's findings with a null `candidate_id`. Both feed the same per-code views.
- `rusqlite` is pinned to 0.32 with `bundled` because 0.43 needs Rust 1.89. Keep 1.85 until the repo floor moves.
- The dashboard binds `127.0.0.1` only, opens the database read-only (with a read-write fallback for a WAL edge case) and never migrates it. It polls once a minute and has no Node or build step; the HTML, CSS and JS are `include_str!` so editing them needs a rebuild. A database without the expected tables answers 500 and logs the SQL error; a missing file renders an empty state.

## Ops

- Defaults point at one GPU box (`192.168.88.38`). `deploy/env.sh` holds host paths, ports and model paths; `serve-{llm,sd,audio}.sh` each answer `start|stop|status`; `make deploy` copies them plus `audio-server.json` to `REMOTE_BIN_DIR`/`REMOTE_CONFIG_DIR`.
- `docs/configuration.md` is the reference for every env var, config key and output format. `docs/README.md` indexes the docs.
- `docs/rfcs/0001-adversary-review.md` is partly implemented: pools, cloud judge and ffmpeg/Remotion assembly exist; local hard gates and the retry loop do not. The pipeline still uses the ffmpeg renderer, not Remotion. `docs/rfcs/0002-deterministic-scoring.md` is partly implemented: the rubric, `melodias score`, the history write, `melodias history`, the dashboard and the deterministic retry loop exist; the five editorial axes and their weights do not.
