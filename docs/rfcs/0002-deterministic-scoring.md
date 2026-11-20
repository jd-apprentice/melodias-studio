# 0002 · Deterministic scoring and feedback loop

Status: accepted. Phases 1 and 2 are implemented, plus a first dashboard; the retry loop is not.

Date: 2026-11-20

Depends on: [0001](0001-adversary-review.md)

## Summary

Give every run a number from 0 to 100, split between a deterministic **technical** score computed locally and an optional **editorial** score from the judge. The technical score is made of measurable image and audio properties, each one producing a finding with a code, a value and a threshold. The findings are stored in SQLite, and a bounded retry loop turns the worst finding into a prompt patch for the next round.

The goal is a total of 80, with the technical part carrying most of the weight so the number is reproducible, cheap and testable without a cloud call.

## Motivation

The first real judge run scored the finished pair 67 and the best thumbnail 68. That number is useful as a ranking, but it is opaque and unstable: it comes from an LLM, it changes with the prompt, and it cannot be reproduced offline.

Measuring the same run locally shows the same problems with numbers:

| Signal | Measured | What it says |
| --- | --- | --- |
| Unique colors in the thumbnail | 216,853 | Not pixel art. A posterized sprite should be under 2,048 |
| Luma std of the thumbnail at 168x94 | 0.173 | Readable, good contrast |
| Pixels with high value and high saturation | 0.0 % | No neon glow, matches the repaired eyes |
| OCR words with confidence >= 75 and length >= 4 | 0 | No text or watermark |
| Integrated loudness | -14.6 LUFS | Inside the -15 ± 1 target |
| True peak | -1.4 dBFS | Below the -1 ceiling |
| Energy above 5 kHz | 4.0 % | Weak chiptune character, mostly orchestral |
| Intro RMS vs middle RMS | +0.1 dB | No build, the track starts at full energy |
| Silent gap in the body | 0 s | The 4.6 s at the end is the fade-out |

The palette metric alone explains the judge's complaint ("anime with pixel overlays, not true pixel art"). The judge is not needed to see it; it is needed for composition, mood and eyes.

## Goals

- One reproducible score per candidate and per run, computed with `ffmpeg` and pure Rust.
- Every point lost has a code, a measured value and a threshold, so a human or an agent can act on it.
- A bounded loop that regenerates only the weak side, with a hard budget.
- A local history that answers "which finding costs the most, and did the patch help".
- Every run records itself in that history, without a service and without a second command.
- No cloud call required. The judge stays optional and only fills the editorial part.

## Non-goals

- Replacing the judge. Composition, mood and eyes are editorial; the rubric does not pretend to see them.
- A dashboard. It comes after the database has enough runs.
- Scoring the video. This RFC scores the thumbnail and the track.

## Scoring model

Two components and one total:

```
technical = image (50) + audio (50)          deterministic, always computed
editorial = judge best-pair score (0..100)   optional, only when the judge runs
total     = 0.6 * technical + 0.4 * editorial
```

Each side is normalized to 0..100. When `tesseract` is missing, `IMG_TEXT` is skipped and the image side is normalized over the 40 points it can still measure.

Without the judge, `total = technical` and the target is the technical threshold. The 60/40 split means a perfect editorial score cannot carry a weak run: with technical 72, even a judge score of 100 gives 83.2. Fixing the palette alone (technical 92) gives 82.1, which is the intended path.

### Acceptance

A run is accepted when all of these hold:

| Rule | Value |
| --- | --- |
| Hard gates | all pass |
| `technical` | >= 85 |
| `editorial` | >= 70, only when the judge ran |
| `total` | >= 80 |

Otherwise the loop runs, or the run ends marked `accepted_by_limit`.

### Image, 50 points

| Code | Metric | Threshold | Points |
| --- | --- | --- | --- |
| `IMG_PALETTE` | unique RGB values in the PNG | <= 256: full; 256..2,048: linear 20..10; 2,048..16,384: linear 10..0; above: 0 | 20 |
| `IMG_CONTRAST` | luma standard deviation at 168x94 | >= 0.18: full; 0.10..0.18: linear; below 0.10: 0 | 10 |
| `IMG_TEXT` | OCR words: confidence >= 75, length >= 4, at least 2 alphanumeric | 0 words: full; -10 per word | 10 |
| `IMG_GLOW` | share of pixels with V >= 0.95 and S >= 0.85 | <= 0.2 %: full; 0.2..2 %: linear down; above 2 %: 0 | 5 |
| `IMG_SATURATION` | mean S in HSL | 0.30..0.75: full; outside: linear to 0 at 0.15 away | 5 |

`IMG_GLOW` catches neon bloom, not eyes. A natural red iris does not trip it, and a subtle glow can pass it. Eyes stay editorial.

### Audio, 50 points

| Code | Metric | Threshold | Points |
| --- | --- | --- | --- |
| `AUD_LOUDNESS` | integrated LUFS | within 1 LU of -15: full; within 2: 5; outside: 0 | 8 |
| `AUD_PEAK` | true peak | <= -1 dBFS: full; -1..-0.3: 4; above: 0 | 8 |
| `AUD_CENTROID` | mean spectral centroid | 2.0..4.5 kHz: full; 1.2..2.0 or 4.5..6.0: linear 4..8; outside: 0 | 8 |
| `AUD_HIGHS` | energy above 5 kHz, relative to total | >= 8 %: full; 3..8 %: linear 4..8; below 3 %: 0..4 | 8 |
| `AUD_BUILD` | intro RMS minus middle RMS | <= -2 dB: full; -2..0: linear 8..4; above 0: linear 4..0 | 8 |
| `AUD_SILENCE` | longest silent gap before the last 5 s | <= 1 s: full; 1..2 s: linear; above 2 s: 0 | 5 |
| `AUD_DURATION` | length against `DURATION_SEC` | within 2 s: full; outside: 0 | 5 |

`AUD_BUILD` is the honest measure of "epic": a track that starts at full energy and never builds loses points. The current run loses 3.5 of 8 there.

`AUD_SILENCE` excludes the tail because `FADE_OUT_SEC` leaves an intentional near-silence; the current 4.6 s at the end is not a defect.

### Hard gates

Candidates that fail a gate are rejected before scoring, not penalized:

- `GATE_TYPE`: 16:9 within 1 %, at least 1280x720.
- `GATE_DURATION`: audio length within 2 s of `DURATION_SEC`.
- `GATE_PEAK`: true peak above 0 dBFS.
- `GATE_DECODE`: `ffprobe` cannot read the file.

### Editorial, 100 points

The judge keeps the rubric from 0001 but stops returning a single opaque score. It returns 0..100 for style, composition, eyes, retro character and pair coherence, plus `must_fix` codes. `editorial` is the weighted sum of those axes, and the judge receives the deterministic findings so it does not repeat them and does not contradict the gates.

When the judge is off, `editorial` is absent and the total equals the technical score.

## Findings

Every point lost becomes a row, not just a number:

```json
{
  "code": "IMG_PALETTE",
  "source": "deterministic",
  "severity": "high",
  "metric": "unique_colors",
  "value": 216853,
  "threshold": 16384,
  "delta": -20.0,
  "message": "216853 colors: the frame is a full-color render, not pixel art"
}
```

Severity is derived from the delta: `high` at 10 points or more, `medium` from 4, `low` below 4. The run report shows the top three findings per candidate. `work/score.json` keeps the full list, and `verdict.json` keeps the editorial ones.

## Retry loop

```
round 0: generate the pool, score, pick the best pair
if accepted: stop
for round in 1..=SCORE_ROUNDS:
    pick the highest-severity high finding per side
    apply the deterministic patch for that code
    regenerate only that side, keep the best candidate of the pool
    re-score
```

Rules:

- One patch per side per round. A finding without a patch entry in the table does not trigger a round.
- Never patch the same code twice; if it still fails, the metric or the threshold is wrong.
- `SCORE_ROUNDS` defaults to 2. The loop stops early when accepted.
- Budget: two image rounds and one audio round. Audio drafts are the expensive side.
- A run that ends below the target is still valid: it is marked `accepted_by_limit` and the report says why.

The patch table is fixed data, not a model. The agent chooses the finding code; the code chooses the text:

| Code | Patch |
| --- | --- |
| `IMG_PALETTE` | append "limited palette, posterized, flat shading, 32 colors, pixel art" |
| `IMG_CONTRAST` | append "high contrast, clear silhouette, bright rim light" |
| `IMG_GLOW` | add "glowing eyes, neon glow" to the negative prompt, "natural eyes, visible iris and pupil" to the positive |
| `IMG_TEXT` | add "text, letters, watermark, signature, logo" to the negative prompt |
| `AUD_HIGHS` / `AUD_CENTROID` | append "8-bit chiptune, square and triangle waves, bitcrushed" |
| `AUD_BUILD` | append "soft intro that builds, layered arrangement, resolved outro" |
| `AUD_SILENCE` | append "continuous arrangement, no long silence" |

## History

SQLite, one file, next to the user config by default:

```
$XDG_CONFIG_HOME/melodias-studio/history.db
```

`MELODIAS_DB` overrides it, so a run can write into a repo checkout during development. `.gitignore` gets `*.db`, `*.db-wal` and `*.db-shm`. WAL mode, one writer at a time, no server.

Writing is part of the run. When the pipeline finishes, it records one `runs` row, one row per candidate and one `findings` row per lost point, including the judge verdict when there is one. The score is computed right after the video and also written to `work/score.json`. A failed write is logged and never aborts the run, the same way the judge fails open. `melodias history` only reads; it never writes.

```sql
create table runs (
  id integer primary key,
  started_at text not null,
  slug text not null,
  vibe text not null,
  rubric text not null default 'score-v1',
  technical real,
  editorial real,
  total real,
  accepted integer not null,
  rounds integer not null,
  cost_usd real,
  seconds real,
  config_json text
);

create table candidates (
  id integer primary key,
  run_id integer not null references runs(id),
  kind text not null,              -- image | audio
  idx integer not null,
  seed integer,
  score real,
  score_json text
);

create table findings (
  id integer primary key,
  run_id integer not null references runs(id),
  candidate_id integer references candidates(id),
  code text not null,
  source text not null,            -- deterministic | judge
  severity text not null,
  metric text,
  value real,
  threshold real,
  delta real not null,
  message text
);

create table patches (
  id integer primary key,
  run_id integer not null references runs(id),
  code text not null,
  prompt_before text,
  prompt_after text,
  applied_at text not null
);
```

The `rubric` column matters: a threshold change starts a new version, and trends only compare rows with the same one. Without it, "the score went up" can just mean "the rubric got softer".

### Backfill

`scripts/backfill-history.sh`, also available as `make backfill`, scores the runs already in `output/` with the same rubric, so the history starts with data. It skips a slug that is already present and rewrites it with `--replace`. It writes the same tables as the pipeline, so `melodias history` reads both. The first database was built this way, before any pipeline change. All three runs scored below the target, which is the expected starting point:

| Run | Technical | Editorial | Total |
| --- | --- | --- | --- |
| `epic-8-bit-emotional-artcore-orchestra` | 66.2 | - | 66.2 |
| `epic-8-bit-orchestral-finale` | 67.1 | - | 67.1 |
| `world-s-most-epic-emotional-violin` | 72.4 | 67.0 | 70.2 |

The palette costs 20 points in every run, with 205k to 368k colors. The build and the high band lose about 4 points each in all three. One legacy run is 72 s long instead of 210 s, which the duration rule catches. Only the newest run has an editorial score, because the judge did not exist before it.

The script measures with ImageMagick and the tool with ffmpeg, so the same run can differ by up to 0.3 points. The table keeps the script output; `melodias score` is the reference from phase 1 on.

The backfill also tightened the OCR filter: a three-character junk token at confidence 73 cost 10 points on a clean thumbnail. Requiring four characters and confidence 75 removed the false positive.

The useful view is the cost of each finding over time:

```sql
select code, count(*) as hits, avg(delta) as lost,
       sum(case when run_id in (select id from runs where accepted = 1) then 1 else 0 end) as accepted
from findings group by code order by lost asc;
```

## CLI

```
melodias score <dir>              # table + work/score.json, no GPU, no cloud
melodias score <dir> --json
melodias history                  # last 20 runs
melodias history --json | --csv
melodias history findings         # cost per code
melodias history trend --by day
melodias dashboard                # local read-only charts at 127.0.0.1:7373
```

`score` works on any finished run, including runs made before the rubric existed. That makes the first rollout measurable without regenerating anything.

## Worked example

The current run, scored with this rubric:

| Side | Metric | Value | Points |
| --- | --- | --- | --- |
| Image | palette | 216,853 colors | 0.0 / 20 |
| Image | contrast | 0.172 | 9.0 / 10 |
| Image | text | 0 tokens | 10.0 / 10 |
| Image | glow | 0.0 % | 5.0 / 5 |
| Image | saturation | 0.378 | 5.0 / 5 |
| Audio | loudness | -14.6 LUFS | 8.0 / 8 |
| Audio | peak | -1.4 dBFS | 8.0 / 8 |
| Audio | centroid | 3.3 kHz | 8.0 / 8 |
| Audio | highs | 4.0 % | 4.8 / 8 |
| Audio | build | -0.2 dB | 4.5 / 8 |
| Audio | silence | 0 s | 5.0 / 5 |
| Audio | duration | 210.0 s | 5.0 / 5 |

`technical = 72.2`, `editorial = 67`, `total = 70.1`. The run would fail the 80 target, and the report would name `IMG_PALETTE` as the first fix. Posterizing to 2,048 colors takes technical to 92.2 and total to 82.1, before touching the audio. `melodias score` reproduces this table.

## Calibration

The editorial score is the weak part and the RFC does not hide it:

- Absolute LLM scores drift with the prompt and the model version. Compare candidates inside a run, not across months, unless the model and the rubric are pinned.
- Two or three anchor examples per band (a known 60, a known 80, a known 95) make the scale mean something.
- Pairwise comparison ("is A better than B") is more stable than an absolute number. The ranking is trustworthy even when the absolute score is not.
- The technical score never depends on the judge, so the total stays comparable when the backend changes.

## Phases

1. **Metrics and `melodias score`.** Done: `src/score.rs`, pure functions over pixel buffers plus parsers for `ffmpeg` output, `melodias score <dir>` and `work/score.json`, findings in the report. Every candidate and the final pair are scored; drafts skip the master-only metrics. No database, no loop.
2. **History in every run.** Done: `rusqlite` with the bundled feature, one write at the end of a run, `melodias history` reads. `MELODIAS_DB` and the default path. Findings of the final pair are stored at run level (`candidate_id` null); the pool candidates keep their full score in `candidates.score_json`. `scripts/backfill-history.sh` covers the runs made before it existed.
3. **Retry loop.** Patch table, budget, `accepted_by_limit`, `must_fix` codes in the judge prompt.
4. **Dashboard.** Done (first version): `melodias dashboard` serves a local read-only page over the database with the score per run and the cost per finding. It is a Rust server plus static files in `dashboard/`, polls every three seconds and needs no build step. A static export over the CSV is still open.

## Tests

- Pure functions take pixel buffers and parsed numbers, so most tests need no `ffmpeg`: palette buckets, contrast curve, glow share, score composition, delta and severity.
- Parsers get captured `ffmpeg` output as string fixtures: `ebur128`, `astats`, `silencedetect`, `aspectralstats`.
- One ignored test writes a real run from `output/` into a temporary database when a run exists.
- The judge JSON contract gets a test that rejects a score without findings.

## Risks

- **Gaming the metric.** Posterizing to hit `IMG_PALETTE` can make the thumbnail worse. The editorial weight, the gates and the review screen stay as counterweights.
- **Threshold drift.** Technical scores are only comparable within a rubric version. Store it.
- **New dependency.** `rusqlite` is a build-time addition; bundled SQLite avoids a system package. `tesseract` stays optional and `IMG_TEXT` is skipped when it is missing, with a note in the report.
- **False positives.** OCR on illustrated art invents words. The first backfill caught a three-character junk token at confidence 73, so the filter now requires four characters, confidence 75 and two alphanumerics. It stays a tuning problem; the threshold may need more runs.
- **Audio metrics are coarse.** Centroid and high-band energy describe timbre, not melody. They point in the right direction and do not replace listening.
- **History is personal data.** It stays local, in the config directory, and is never sent anywhere.

## Decisions

1. **Weights:** 60/40 technical/editorial. The deterministic score dominates and the judge decides the last four points of every ten.
2. **Palette:** 2,048 colors as the target, with the buckets above. 256 is unreachable for sd-server without destroying the image.
3. **Acceptance:** 80 total, 85 technical, 70 editorial. A perfect editorial score cannot carry a technical score below 72.
4. **OCR:** `tesseract` is optional. Without it, `IMG_TEXT` is skipped and the image side is renormalized over the remaining 40 points.
5. **Database:** the config directory, `MELODIAS_DB` overrides it. `.gitignore` covers `*.db`, `*.db-wal` and `*.db-shm` for checkouts that point into the repo.
6. **Audio round:** at most one, and only when the audio side lost 8 points or more and the image side already passed. The loop still stops at `SCORE_ROUNDS`.
7. **Candidates:** score all of them. Drafts skip `AUD_LOUDNESS`, `AUD_PEAK` and `AUD_DURATION`, which describe the mastered track.
