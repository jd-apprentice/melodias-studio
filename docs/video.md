# Video

## Two renderers

Both take the chosen `thumbnail.png` and the mastered `audio.mp3` and write `video.mp4`.

### ffmpeg (fast)

`media::assemble_video` builds a 1080p30 video in about two minutes. Slow zoom, soft sprite particles, waveform band and fades. It is deterministic and needs no Node.

```bash
melodias video output/<slug>
```

The pipeline uses this renderer at the end of every run.

### Remotion (produced)

The `video/` project renders a richer composition in about twelve minutes:

- Slow zoom on the still.
- Bokeh and spark overlays from `assets/videos/`, screen blended.
- A bird layer clipped to the sky.
- An audio-reactive waveform from `@remotion/media-utils`.
- A subtle channel label and fades.

```bash
cd video
pnpm install
cd ..
melodias render output/<slug>
```

`melodias render` copies the still and track into `video/public/current/`, writes `video/props.json` with the duration, and runs `pnpm exec remotion render` with four workers. The previous `video.mp4` is kept as `video-ffmpeg.mp4`.

The 1080p overlay clips live in `video/public/overlays/` and are committed. The 4K masters in `assets/videos/` are local only and ignored by git.

## Setup notes

- Node 18+ and pnpm are required only for the Remotion renderer.
- `pnpm-workspace.yaml` allows esbuild's build script, which pnpm blocks by default.
- The first render downloads a Headless Shell for Remotion.
- Remotion writes a ~120 MB file for a three minute track, compared to ~42 MB for the ffmpeg version.

## Why a sprite for the ffmpeg path

`drawbox` positions are evaluated once, which produced static squares. The overlay filter with `eval=frame` evaluates the position on every frame, so the particles move. The sprite is a 128x128 radial alpha generated with ffmpeg `geq`, tinted per particle with `colorchannelmixer`.

## Options for richer effects

| Option | What it gives | Cost |
| --- | --- | --- |
| **Remotion** (implemented) | Animated titles, transitions, branded intros and outros, audio-reactive scenes, any CSS/SVG effect | Node project, ~12 min render |
| **Motion Canvas / Revideo** (TS) | Canvas animation with a scene graph | Another rendering pipeline |
| **Kdenlive, Shotcut, DaVinci** | Full manual editing for one-offs | Manual, not reproducible |
| **Blender, MLT, VapourSynth** | Scripted compositing, 3D scenes | Heavy, complex setup |

## Open items

- The pipeline still calls the ffmpeg renderer; wiring Remotion as the default stage is pending a decision on the ~50 minute total budget.
- The assembly agent that picks a preset from the vibe is still a design item.
- Title and channel overlays are fixed; they can move to `config.toml` when the video gets more settings.
