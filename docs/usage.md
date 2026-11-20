# Usage

## TUI

![melodias-studio TUI](assets/tui.png)

```bash
melodias
```

- `Tab` switches between the URL and the vibe.
- `Enter` moves to the next field, then starts the run.
- `q` quits, only once the run finished.

Each stage shows its state, the progress bar tracks how many are done, and the log prints every step, including the prompt the LLM wrote and the seed of the take. A splash with the version appears for a moment on start.

## Headless run

Passing the URL and the vibe as arguments runs the same pipeline without the full screen UI. Each stage is printed as its own colored group with an icon, and tool output from `yt-dlp`, `ffmpeg` and `ssh` goes to the run log instead of the screen:

```bash
MELODIAS_LOG=output/run.log melodias 'https://www.youtube.com/watch?v=...' 'epic 8-bit orchestral finale'
```

The log receives the same lines as plain text, so it can be read later without escape codes. Quote the URL: a YouTube link copied from a playlist carries `&list=` and `&index=`, which the shell would otherwise split. The exit code is non-zero when a stage fails.

The image stage reports its own duration and the size of the PNG it got back, and warns when the server returned something other than the requested size.

## Make shortcuts

| Target | What it does |
| --- | --- |
| `make run` | Opens the TUI |
| `make install` / `make uninstall` | Installs or removes `~/.local/bin/melodias` |
| `make generate URL='...' VIBE='...'` | Runs headless and writes `output/run-<timestamp>.log` |
| `make log` | Prints the last 40 lines of the newest log |
| `make check` | `fmt --check`, build, `clippy -D warnings` and tests |
| `make status` | Service state on the host over SSH |
| `make deploy` | Copies `deploy/` to the host and fixes permissions |

`make generate` builds the release binary first, so the run does not pay debug overhead.

## Output

```
output/<slug>/
├── audio.mp3      # 3 min song, 320 kbps, -15 LUFS
├── thumbnail.png  # 1280x720
├── meta.json      # seed, prompts, parameters and source URL
└── work/          # source, reference and raw take
```

## Screenshots

The images in these docs come from the test backend, not from a manual capture. Regenerate them with:

```bash
DUMP_TUI_HTML_DIR=/tmp/tui cargo test draws_without_leaving_the_terminal
```

That writes `tui.html` and `tui-splash.html`; render them with a browser at the size you need.
