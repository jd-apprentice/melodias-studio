# Adversary review and video assembly

Status: proposal. Nothing in this document is implemented yet.

## Why

Today the pipeline generates one thumbnail and one take. If either is weak, the run has to be repeated by hand and there is no record of why one result was accepted. For three to four minute uploads that is expensive: a rejected take costs a full audio stage.

The proposal is to generate a small pool of candidates per modality, let local critics pick the best one inside a fixed number of rounds, optionally ask a stronger cloud judge for a second opinion, and then assemble the final video from the chosen still, an audio-reactive effect and the mastered track.

## Goals

- Improve the average quality without a human in the loop.
- Bound the review with an explicit round limit and a score threshold.
- Keep the local-only mode working: the cloud judge is optional.
- Record every candidate, score and decision in a manifest.
- Assemble a publishable video from the chosen pieces.

## Non-goals

- Training or fine-tuning models.
- Automatic publishing to YouTube.
- Reviewing with raw media on the cloud when the user disables it.

## Flow

```
brief (vibe + fixed rules)
        │
        ├─ image pool (N candidates) ──► image critic ──┐
        │                                              │
        ├─ audio pool (N candidates) ──► audio critic ──┤
        │                                  ▲           │
        │                        retry only the weak side
        │                        while rounds remain    │
        └──────────────────────────────────┴───────────┘
                                               │
                                    shortlist + metrics
                                               │
                                    cloud judge (optional)
                                     approve / revise once
                                               │
                                    assembly (still + effects + audio)
                                               │
                                            final/
```

The pools run sequentially because a single GPU cannot hold the image model and ACE-Step at the same time. The image pool still runs first, as it does now.

## Candidate pools

- `IMAGE_CANDIDATES` and `AUDIO_CANDIDATES`, default 2 each.
- Every candidate keeps its seed and parameters in `manifest.json`.
- Image seeds are not sent yet. The first prerequisite is adding a seed to the sd-server request so a candidate can be reproduced.
- Each pool loads its model once and generates all candidates before unloading it.
- Candidates are not regenerated at full steps; the same parameters are used for the whole pool.

## Local critics

Two layers, cheapest first.

### Hard gates

Deterministic checks that do not need a model:

- Image: valid PNG, exact size, not blank (pixel variance), no text or watermark, palette distance to the channel palette.
- Audio: duration tolerance, no clipping, loudness window, no silence at the edges, intro ramp and outro decay present, band balance against the reference, stable tempo.
- Both: the fixed rules, such as instrumental only.

A candidate that fails a hard gate is discarded before any scoring.

### Soft score

- Image to text alignment with a small CLIP model.
- Audio to text alignment with a small CLAP model.
- Channel style: palette, composition, energy envelope.
- A local text LLM receives the metrics plus the brief and returns a ranking with one line of reasoning per candidate.
- A small vision model can caption the image when the LLM needs more than metrics.

Start with one critic. If two critics are desired, use a technical one and a creative one, then a local arbiter on the text model.

## Selection versus merging

Default to selecting the best candidate. Blending three images or splicing arbitrary audio sections usually creates visible or audible seams.

Merging is a later experiment: choose the best intro, body and outro only when tempo and key match, crossfade on bar boundaries, and reject the result if the seam measures above a threshold.

## Iteration policy

- `REVIEW_ROUNDS`, default 2.
- `REVIEW_THRESHOLD`, for example 0.75 on a normalized score.
- Each round regenerates only the modality below the threshold.
- Rejected candidates are deleted, but their scores stay in the manifest.
- When the rounds run out, take the best candidate, mark it as accepted by limit, and continue. The run never loops forever.

## Cloud judge

Optional and off by default.

- Input: brief, fixed rules, metrics, local reasoning and the chosen image. Audio is described by its metrics and the local score; the judge is not expected to listen.
- Output, strict JSON:

```json
{
  "verdict": "approve",
  "score": 86,
  "must_fix": [],
  "nice_to_have": ["warmer highlight"],
  "reason": "one or two sentences"
}
```

- The judge may send the work back once with `must_fix`. A second rejection ends the loop and publishes the best local candidate.
- If the API fails or returns invalid JSON, the run continues with the local pick and logs the failure. The judge never writes files.
- Privacy: only enable it when sending the brief and the image to a third party is acceptable. Log exactly what was sent.

## Assembly

The final video is the chosen still, an audio-reactive effect and the mastered track.

- Canvas 1920x1080, H.264 video and AAC audio.
- The still gets a slow zoom or parallax; no cuts, since the image is static.
- A low band at the bottom shows a waveform or spectrum driven by the track.
- Title and channel overlays come from the source metadata and the channel config.
- Video fades in and out with the audio.
- The assembly agent only chooses a preset (`minimal`, `pixel`, `retro`) from the vibe. The rendering itself is a deterministic ffmpeg graph.
- Output: `video.mp4`, `thumbnail.png`, `audio.mp3` and `manifest.json`.

## Data model

```
output/<slug>/
├── manifest.json
├── candidates/
│   ├── image-01.png
│   ├── image-02.png
│   ├── audio-01.wav
│   └── audio-02.wav
├── final/
│   ├── thumbnail.png
│   ├── audio.mp3
│   └── video.mp4
└── work/
```

The manifest records the brief, resolved config, seeds, hard gate results, soft scores, rounds used, judge verdict and the chosen paths.

## Runtime and VRAM

Approximate numbers from the current P40 setup:

| Step | Today | With two candidates |
| --- | --- | --- |
| Image stage | ~6 min | ~12 min |
| Audio stage | ~14 min at 32 steps | ~28 min |
| Local critics | none | seconds per model load |
| Judge | none | seconds, no GPU |
| Assembly | none | under a minute |

The critics use small CLIP and CLAP models. Run them after the big model is unloaded, and keep the LLM stopped during audio as the pipeline already does.

## Configuration sketch

```
IMAGE_CANDIDATES=2
AUDIO_CANDIDATES=2
REVIEW_ROUNDS=2
REVIEW_THRESHOLD=0.75
JUDGE_ENABLED=false
JUDGE_MODEL=
JUDGE_API_KEY=
ASSEMBLY_PRESET=auto
```

The same keys belong in `config.toml` once the feature exists.

## Phased plan

1. Manifest and candidate pools, with a manual pick in the TUI. No new models.
2. Deterministic hard gates and a local ranking.
3. Review loop with rounds and thresholds.
4. Optional cloud judge with the JSON contract.
5. Video assembly.
6. Bar-aligned splicing, only if the seams measure clean.

## Risks and open questions

- Vocal detection for the instrumental rule. Source separation is the reliable option but adds a model and time.
- The cloud judge cannot hear the track. Either accept a metrics-only review or use an audio-capable model.
- Cost and time grow linearly with the number of candidates. Two is the safe default.
- Judge reliability: strict schema, one revision, and fail-open to the local pick.
- Reproducibility: image seeds are not sent today, so image candidates cannot be replayed. Fix that first.
- Blending audio across takes can sound worse than the best single take. Measure before shipping it.
