# Adversary review and video assembly

Status: in progress. Phase 1 (image seed and steps) is implemented. Candidate pools, critics and the judge are not.

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
- Image seeds and steps are sent through `sd_cpp_extra_args`; the same seed and steps reproduce the image.
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

## Cost control and runtime

More candidates only make sense if a round stays affordable. The current costs come from three choices: the image request sends no steps, so the server uses its 20-step recipe; the audio request uses 32 steps with heun, which evaluates the model twice per step; and every candidate pays the full duration.

Use two tiers:

| Tier | Image | Audio |
| --- | --- | --- |
| Draft (the pool) | 1280x720, 10 to 12 steps | 60 s, 16 steps, euler |
| Final (the winner) | 1280x720, 20 steps | configured duration, 32 steps, heun |

Why each choice:

- **Image steps before resolution.** Keep the same 1280x720 canvas for drafts. Changing the canvas changes the latent layout, and with the same seed the final would not match the candidate the critic picked. Steps and seed travel inside `sd_cpp_extra_args`, which the server strips before generation.
- **A low-step draft is close enough.** With the same seed and size, 12 steps and 20 steps usually share the composition. The critic ranks the draft, and the winner is regenerated at 20 steps.
- **Image resolution is the last lever.** 1024x576 saves more time but changes the composition. If it is used, the critic must rank the draft and the final must be reviewed again, which defeats part of the saving.
- **Short audio drafts rank style, not the arc.** A 60 s draft at 16 euler steps is enough to compare timbre, energy and prompt adherence. The full 210 s take is generated once, at 32 heun steps, and passes the hard gates for intro, outro and duration.
- **Keep the model loaded across the pool.** One load per modality, all candidates inside it.
- **Turbo for audio drafts is optional.** ACE-Step Turbo caps at 8 steps but requires switching the DiT and reloading, so it only pays off if the reload is cheaper than the saved steps. Measure before adopting it.

Rough wall time on the P40, to be measured rather than trusted:

| Step | Today | N=2 draft/final | N=3 draft/final |
| --- | --- | --- | --- |
| Image pool | ~6 min for one | ~6 min + ~6 min final | ~9 min + ~6 min final |
| Audio pool | ~14 min for one | ~4 min + ~14 min final | ~6 min + ~14 min final |
| Total | ~20 min | ~30 min | ~35 min |

For comparison, three full candidates would be about 60 minutes. The saving comes mostly from the short audio drafts, not from the images.

Two quality questions to settle with an A/B before shipping the policy:

- Does 12-step MOSS-Image hold up against 20 steps on this style?
- Does a 60 s draft rank the same take that a full 210 s run would produce?

If either answer is no, keep the full parameters for that modality and accept the longer round.

## Configuration sketch

```
CANDIDATES=2
IMAGE_STEPS=20
IMAGE_SEED=
IMAGE_DRAFT_STEPS=12
AUDIO_STEPS=32
AUDIO_DRAFT_STEPS=16
AUDIO_DRAFT_SEC=60
AUDIO_DRAFT_SAMPLER=euler
REVIEW_ROUNDS=2
REVIEW_THRESHOLD=0.75
JUDGE_ENABLED=false
JUDGE_MODEL=
JUDGE_API_KEY=
ASSEMBLY_PRESET=auto
```

The same keys belong in `config.toml` once the feature exists. Image seeds are not sent today, so a candidate cannot be replayed; sending `seed` in `sd_cpp_extra_args` is the first prerequisite.

## Phased plan

1. Image steps and seed through `sd_cpp_extra_args`, plus the draft/final policy. Seed and steps are done and validated; the draft/final policy is pending.
2. Manifest and candidate pools, with a manual pick in the TUI. No new models.
3. Deterministic hard gates and a local ranking.
4. Review loop with rounds and thresholds.
5. Optional cloud judge with the JSON contract.
6. Video assembly.
7. Bar-aligned splicing, only if the seams measure clean.

## Risks and open questions

- Vocal detection for the instrumental rule. Source separation is the reliable option but adds a model and time.
- The cloud judge cannot hear the track. Either accept a metrics-only review or use an audio-capable model.
- Cost and time grow linearly with the number of candidates. Two is the safe default.
- Judge reliability: strict schema, one revision, and fail-open to the local pick.
- Reproducibility: image seeds and steps are now sent; recheck that a server update keeps the same output for the same input.
- Blending audio across takes can sound worse than the best single take. Measure before shipping it.
