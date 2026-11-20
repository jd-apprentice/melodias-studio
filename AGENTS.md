# melodias-studio

Rust TUI (ratatui) that turns a YouTube URL plus a vibe into `output/<vibe-slug>/` with `thumbnail.png`, `audio.mp3`, `meta.json` and `work/`. The binary is named `melodias`: no args opens the TUI, two args (`url`, `vibe`) run the same pipeline headless.

## Commands

- `make check` is the gate: `cargo fmt --check`, `cargo build`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. There is no CI, so run it before finishing.
- Single test: `cargo test <name>`. All tests are inline `#[cfg(test)]` modules and need no services, GPU or network.
- `make run` (TUI), `make generate URL='...' VIBE='...'` (release build + headless, timestamped log in `output/`), `make log` (last lines of newest log).
- The screenshots in `docs/` come from the test backend, not a manual capture: `DUMP_TUI_HTML_DIR=/tmp/tui cargo test draws_without_leaving_the_terminal`. `PRINT_TUI=1` prints the buffer as ASCII.
- `make deploy` and `make status` need SSH access to the GPU host; the rest runs locally.

## Wiring

- `src/pipeline.rs` owns the 10-stage order in `STAGES` (image pool, download/reference, audio drafts, review, final take, master/save). The TUI and the headless reporter consume the same `Msg` channel and answer the review through the `Sender<Selection>` returned by `pipeline::spawn`, so adding a stage means touching `STAGES` and `run()`, plus the hardcoded `1/10` assertions in `src/main.rs`.
- Config precedence is env var > `config.toml` (`MELODIAS_CONFIG`, else `$XDG_CONFIG_HOME/melodias-studio/config.toml`) > defaults. `src/clients.rs` calls llama-server (`/v1/chat/completions`), sd-server (`/v1/images/generations`) and audiocpp (`/v1/tasks/run`); `src/media.rs` shells out to `yt-dlp`, `ffmpeg`, `ffprobe`, `ssh` and `scp`.
- Child process output goes to the run log through `media::set_child_log` (TUI: `output/run-<unix>.log`, headless: `MELODIAS_LOG`). Use `attach_output`/`attach_stderr` for new subprocesses so they do not fight the TUI.
- `docs/architecture/arquitectura.html` is generated from `docs/architecture/arquitectura.json` with the archify skill; edit the JSON, not the HTML.

## Gotchas

- Empty environment variables are ignored (`config.rs::env` filters them), so `SSH_HOST=` does not switch to local mode: it falls back to the file and then to `root@192.168.88.38`. Disable SSH and lifecycle commands in `config.toml` (`ssh_host = ""`, `*_cmd = ""`).
- `media::preflight` requires `scp` on `PATH` even when `SSH_HOST` is empty.
- ACE-Step only reads the nested `options` object: `duration_sec`, `reference_audio` and `sampler_mode` are silently dropped at the top level of `request`.
- A thumbnail size mismatch is only a warning; `media::png_size` validates the PNG header, not the pixels, so trust the log or `meta.json` for the real size.
- The run is not resumable and a failed stage aborts it; `output/` and `target/` are gitignored, the generated `docs/` images and HTML are committed. Candidate images and drafts live in `output/<slug>/candidates/`.

## Ops

- Defaults point at one GPU box (`192.168.88.38`). `deploy/env.sh` holds host paths, ports and model paths; `serve-{llm,sd,audio}.sh` each answer `start|stop|status`; `make deploy` copies them plus `audio-server.json` to `REMOTE_BIN_DIR`/`REMOTE_CONFIG_DIR`.
- `docs/configuration.md` is the reference for every env var, config key and output format. `docs/README.md` indexes the docs. `docs/adversary-review.md` is an unimplemented design, not current behavior.
