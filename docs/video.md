# Video

## Current renderer

`media::assemble_video` builds the final video with ffmpeg, deterministically, from three inputs: the chosen `thumbnail.png`, the mastered `audio.mp3` and a radial sprite generated on the fly.

- 1920x1080 at 30 fps, H.264 video and AAC audio. The duration matches the track.
- A slow zoom on the still, so the image is not frozen.
- Soft moving particles: a radial sprite is scaled, tinted and overlaid several times with `eval=frame`, plus the waveform band and edge fades.
- `melodias video <dir>` rebuilds it without the GPU. The pipeline calls the same function at the end of every run.
- `melodias image <dir>` regenerates the thumbnail from the stored prompt and seed, then rebuilds the video. The old image stays as `thumbnail-old.png`.

## Why a sprite

`drawbox` positions are evaluated once, which produced static squares. The overlay filter with `eval=frame` evaluates the position on every frame, so the particles actually move. The sprite is a 128x128 radial alpha generated with ffmpeg `geq`, tinted per particle with `colorchannelmixer`. It adds no dependency and renders in about two minutes for a three-minute track.

## Options for richer effects

| Option | What it gives | Cost |
| --- | --- | --- |
| **Remotion** (React/TS) | Animated titles, transitions, branded intros and outros, audio-reactive scenes, any CSS/SVG effect | Node project, slower render because every frame is a browser frame |
| **Motion Canvas / Revideo** (TS) | Canvas animation with a scene graph; good for particles and typography | Node project, another rendering pipeline |
| **Kdenlive, Shotcut, DaVinci** | Full manual editing for one-offs; best creative control | Manual, not reproducible |
| **Blender, MLT, VapourSynth** | Scripted compositing, 3D scenes, advanced keying | Heavy, complex setup |

## Recommendation

Keep ffmpeg for the automated pipeline: it is fast, deterministic and enough for a still plus a waveform and particles.

If we want a produced look (channel branding, animated titles, scene transitions, more elaborate particles), **Remotion** is the best next step. It turns the video into a React composition that can read the run's `meta.json` and render the same way every time.

Do not add a dependency just to draw dots. The current sprite overlay covers that case.

## Open items

- Title and channel overlays are not implemented.
- The assembly agent that picks a preset from the vibe is still a design item.
- The particle palette is fixed (cyan, amber, white); it can move to `config.toml` when the video gets more settings.
