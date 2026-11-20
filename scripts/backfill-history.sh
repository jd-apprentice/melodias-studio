#!/usr/bin/env bash
# Backfills the history database from finished runs in output/.
#
# It scores runs made before the pipeline wrote the database itself, using the
# same schema. It reads each meta.json, scores the final thumbnail and the final
# track with the rubric, and writes runs, candidates and findings. The outcome
# is below_target because those runs had no retry loop.
#
# Requires: jq, ffmpeg, ffprobe, sqlite3, ImageMagick (magick) and tesseract.
#
# Usage:
#   scripts/backfill-history.sh [output-dir] [--replace]
#
# The database is $MELODIAS_DB, or $XDG_CONFIG_HOME/melodias-studio/history.db.
# A run already present is skipped unless --replace is passed.

set -euo pipefail
export LC_ALL=C

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
out_dir=""
replace=0

usage() {
    sed -n '2,/^$/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

for arg in "$@"; do
    case "$arg" in
        --replace) replace=1 ;;
        -h|--help) usage; exit 0 ;;
        -*) echo "unknown option: $arg" >&2; usage >&2; exit 2 ;;
        *) out_dir=$arg ;;
    esac
done

out_dir=${out_dir:-"$repo_root/output"}
db=${MELODIAS_DB:-"${XDG_CONFIG_HOME:-$HOME/.config}/melodias-studio/history.db"}

for bin in jq ffmpeg ffprobe sqlite3 magick tesseract; do
    command -v "$bin" >/dev/null || { echo "missing $bin on PATH" >&2; exit 2; }
done

if [ ! -d "$out_dir" ]; then
    echo "no output directory: $out_dir" >&2
    exit 2
fi

mkdir -p "$(dirname "$db")"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

sqlq() { printf '%s' "$1" | sed "s/'/''/g"; }

sqlite3 "$db" >/dev/null <<'SQL'
pragma journal_mode=wal;
create table if not exists runs (
  id integer primary key,
  started_at text not null,
  slug text not null,
  vibe text not null,
  rubric text not null default 'score-v1',
  technical real,
  editorial real,
  total real,
  accepted integer not null,
  outcome text,
  outcome_reason text,
  rounds integer not null,
  cost_usd real,
  seconds real,
  config_json text
);
create table if not exists candidates (
  id integer primary key,
  run_id integer not null references runs(id),
  kind text not null,
  idx integer not null,
  seed integer,
  score real,
  score_json text
);
create table if not exists findings (
  id integer primary key,
  run_id integer not null references runs(id),
  candidate_id integer references candidates(id),
  code text not null,
  source text not null,
  severity text not null,
  metric text,
  value real,
  threshold real,
  delta real not null,
  message text
);
create table if not exists patches (
  id integer primary key,
  run_id integer not null references runs(id),
  code text not null,
  prompt_before text,
  prompt_after text,
  applied_at text not null
);
SQL
chmod 600 "$db"

# Older databases predate the retry columns; add them without losing rows.
for column in outcome outcome_reason; do
    if [ "$(sqlite3 "$db" "select count(*) from pragma_table_info('runs') where name = '$column';")" = "0" ]; then
        sqlite3 "$db" "alter table runs add column $column text;"
    fi
done

# Counts OCR words: confidence >= 75, length >= 4, at least two alphanumerics.
ocr_words() {
    tesseract "$1" - --psm 11 tsv 2>/dev/null | awk -F'\t' '
        NR > 1 && NF >= 12 {
            word = $12
            gsub(/^[ \t]+|[ \t]+$/, "", word)
            if (length(word) < 4) next
            alnum = 0
            for (i = 1; i <= length(word); i++) {
                ch = substr(word, i, 1)
                if (ch ~ /[A-Za-z0-9]/) alnum++
            }
            if (alnum >= 2 && $11 + 0 >= 75) n++
        }
        END { print n + 0 }'
}

image_metrics() {
    local file=$1 size
    size=$(magick identify -format '%wx%h' "$file")
    IMG_WIDTH=${size%x*}
    IMG_HEIGHT=${size#*x}
    IMG_ASPECT=$(awk -v w="$IMG_WIDTH" -v h="$IMG_HEIGHT" 'BEGIN { printf "%.4f", w / h }')
    IMG_COLORS=$(magick "$file" -format '%k' info:)
    IMG_SAT=$(magick "$file" -colorspace HSL -channel G -separate +channel -format '%[fx:mean]' info:)
    magick "$file" -resize 168x94! "$work/thumb.png"
    IMG_STD=$(magick "$work/thumb.png" -colorspace Gray -format '%[fx:standard_deviation]' info:)
    magick "$file" -colorspace HSV -channel R -separate +channel -threshold 95% "$work/value.png"
    magick "$file" -colorspace HSV -channel G -separate +channel -threshold 85% "$work/sat.png"
    IMG_GLOW=$(magick "$work/value.png" "$work/sat.png" -compose multiply -composite -format '%[fx:mean]' info:)
    IMG_OCR=$(ocr_words "$file")
}

overall_rms() {
    ffmpeg -v error -i "$1" -af "$2,astats=metadata=1,ametadata=print:file=-" -f null - 2>/dev/null |
        awk -F= '/Overall\.RMS_level=/ { v = $2 } END { print v + 0 }'
}

segment_rms() {
    ffmpeg -hide_banner -v info -ss "$2" -t "$3" -i "$1" -af astats -f null - 2>&1 |
        awk '/RMS level dB:/ { v = $NF } END { print v + 0 }'
}

audio_metrics() {
    local file=$1 target=$2 ebur
    ebur=$(ffmpeg -hide_banner -v info -i "$file" -af ebur128=peak=true:framelog=quiet -f null - 2>&1)
    AUD_LUFS=$(printf '%s\n' "$ebur" | sed -n 's/.*I:[[:space:]]*\(-\{0,1\}[0-9.]*\) LUFS.*/\1/p' | tail -1)
    AUD_PEAK=$(printf '%s\n' "$ebur" | sed -n 's/.*Peak:[[:space:]]*\(-\{0,1\}[0-9.]*\) dBFS.*/\1/p' | tail -1)
    AUD_DURATION=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$file")
    AUD_CENTROID=$(ffmpeg -v error -i "$file" -af 'aspectralstats=measure=centroid,ametadata=mode=print:file=-' -f null - 2>/dev/null |
        awk -F'centroid=' '/centroid=/ { s += $2; n++ } END { if (n) printf "%.2f", s / n; else print 0 }')
    AUD_FULL=$(overall_rms "$file" 'anull')
    AUD_HIGH=$(overall_rms "$file" 'highpass=f=5000')
    AUD_HIGH_SHARE=$(awk -v f="$AUD_FULL" -v h="$AUD_HIGH" 'BEGIN { printf "%.6f", 10 ^ ((h - f) / 10) }')
    AUD_SILENCE=$(ffmpeg -hide_banner -v info -i "$file" -af 'silencedetect=noise=-50dB:d=0.5' -f null - 2>&1 |
        awk -v dur="$AUD_DURATION" '
            /silence_start:/ { start = $NF }
            /silence_duration:/ {
                if (start < dur - 5 && $NF > max) max = $NF
            }
            END { printf "%.2f", max + 0 }')
    AUD_INTRO=$(segment_rms "$file" 0 20)
    AUD_MID=$(segment_rms "$file" "$(awk -v d="$AUD_DURATION" 'BEGIN { printf "%.2f", d / 2 - 10 }')" 20)
    AUD_BUILD=$(awk -v i="$AUD_INTRO" -v m="$AUD_MID" 'BEGIN { printf "%.4f", i - m }')
    AUD_TARGET=$target
}

# Masks the 0..100 inputs and prints: code, metric, points, max, value, threshold, message.
image_score() {
    awk -v u="$IMG_COLORS" -v s="$IMG_STD" -v o="$IMG_OCR" -v g="$IMG_GLOW" -v sat="$IMG_SAT" '
        function lerp(x, x0, x1, y0, y1) {
            if (x1 == x0) return y1
            v = y0 + (x - x0) * (y1 - y0) / (x1 - x0)
            lo = (y0 < y1 ? y0 : y1); hi = (y0 < y1 ? y1 : y0)
            return (v < lo ? lo : (v > hi ? hi : v))
        }
        function emit(code, metric, pts, max, val, thr, msg) {
            printf "%s\t%s\t%.2f\t%d\t%.6g\t%.6g\t%s\n", code, metric, pts, max, val, thr, msg
        }
        BEGIN {
            log2 = log(2)
            if (u <= 256) p = 20
            else if (u <= 2048) p = lerp(log(u) / log2, 8, 11, 20, 10)
            else if (u <= 16384) p = lerp(log(u) / log2, 11, 14, 10, 0)
            else p = 0
            emit("IMG_PALETTE", "unique_colors", p, 20, u, 16384, sprintf("%.0f colors: full-color render, not pixel art", u))

            if (s >= 0.18) p = 10
            else if (s >= 0.10) p = 10 * (s - 0.10) / 0.08
            else p = 0
            emit("IMG_CONTRAST", "luma_std", p, 10, s, 0.18, sprintf("luma std %.3f at 168x94", s))

            p = 10 - 10 * o; if (p < 0) p = 0
            emit("IMG_TEXT", "ocr_words", p, 10, o, 0, sprintf("%d OCR words", o))

            if (g <= 0.002) p = 5
            else if (g <= 0.02) p = 5 * (0.02 - g) / 0.018
            else p = 0
            emit("IMG_GLOW", "glow_share", p, 5, g, 0.002, sprintf("%.2f%% of pixels glow", g * 100))

            if (sat >= 0.30 && sat <= 0.75) p = 5
            else {
                dist = (sat < 0.30 ? 0.30 - sat : sat - 0.75)
                p = 5 * (1 - dist / 0.15); if (p < 0) p = 0
            }
            emit("IMG_SATURATION", "saturation", p, 5, sat, 0.30, sprintf("mean saturation %.2f", sat))
        }'
}

audio_score() {
    awk -v l="$AUD_LUFS" -v p="$AUD_PEAK" -v c="$AUD_CENTROID" -v h="$AUD_HIGH_SHARE" \
        -v b="$AUD_BUILD" -v z="$AUD_SILENCE" -v d="$AUD_DURATION" -v t="$AUD_TARGET" '
        function lerp(x, x0, x1, y0, y1) {
            if (x1 == x0) return y1
            v = y0 + (x - x0) * (y1 - y0) / (x1 - x0)
            lo = (y0 < y1 ? y0 : y1); hi = (y0 < y1 ? y1 : y0)
            return (v < lo ? lo : (v > hi ? hi : v))
        }
        function emit(code, metric, pts, max, val, thr, msg) {
            printf "%s\t%s\t%.2f\t%d\t%.6g\t%.6g\t%s\n", code, metric, pts, max, val, thr, msg
        }
        BEGIN {
            k = (l + 15 < 0 ? -(l + 15) : l + 15)
            if (k <= 1) pts = 8; else if (k <= 2) pts = 5; else pts = 0
            emit("AUD_LOUDNESS", "lufs", pts, 8, l, -15, sprintf("%.1f LUFS against -15", l))

            pts = (p <= -1 ? 8 : (p <= -0.3 ? 4 : 0))
            emit("AUD_PEAK", "true_peak", pts, 8, p, -1, sprintf("%.1f dBFS true peak", p))

            if (c >= 2000 && c <= 4500) pts = 8
            else if (c >= 1200 && c < 2000) pts = lerp(c, 1200, 2000, 4, 8)
            else if (c > 4500 && c <= 6000) pts = lerp(c, 4500, 6000, 8, 4)
            else pts = 0
            emit("AUD_CENTROID", "centroid_hz", pts, 8, c, 2000, sprintf("%.0f Hz centroid", c))

            if (h >= 0.08) pts = 8
            else if (h >= 0.03) pts = lerp(h, 0.03, 0.08, 4, 8)
            else pts = lerp(h, 0, 0.03, 0, 4)
            emit("AUD_HIGHS", "high_share", pts, 8, h, 0.08, sprintf("%.1f%% of energy above 5 kHz", h * 100))

            if (b <= -2) pts = 8
            else if (b <= 0) pts = lerp(b, -2, 0, 8, 4)
            else pts = lerp(b, 0, 2, 4, 0)
            emit("AUD_BUILD", "build_db", pts, 8, b, -2, sprintf("intro is %.1f dB against the middle", b))

            if (z <= 1) pts = 5; else if (z <= 2) pts = 5 * (2 - z); else pts = 0
            emit("AUD_SILENCE", "silence_sec", pts, 5, z, 1, sprintf("%.2f s silence in the body", z))

            pts = ((d - t <= 2 && t - d <= 2) ? 5 : 0)
            emit("AUD_DURATION", "duration_sec", pts, 5, d, t, sprintf("%.1f s against %.1f", d, t))
        }'
}

severity() {
    awk -v d="$1" 'BEGIN { d = (d < 0 ? -d : d); print (d >= 10 ? "high" : (d >= 4 ? "medium" : "low")) }'
}

insert_candidate() {
    local run_id=$1 kind=$2 idx=$3 seed=$4
    if [ "$seed" = "null" ] || [ -z "$seed" ]; then
        sqlite3 "$db" "insert into candidates (run_id, kind, idx, seed) values ($run_id, '$(sqlq "$kind")', $idx, null) returning id;"
    else
        sqlite3 "$db" "insert into candidates (run_id, kind, idx, seed) values ($run_id, '$(sqlq "$kind")', $idx, $seed) returning id;"
    fi
}

insert_findings() {
    local run_id=$1 candidate_id=$2 side=$3 code metric points max value threshold message delta sev
    while IFS=$'\t' read -r code metric points max value threshold message; do
        delta=$(awk -v p="$points" -v m="$max" 'BEGIN { printf "%.2f", p - m }')
        [ "$(awk -v d="$delta" 'BEGIN { print (d < 0 ? 1 : 0) }')" = "1" ] || continue
        sev=$(severity "$delta")
        sqlite3 "$db" "insert into findings (run_id, candidate_id, code, source, severity, metric, value, threshold, delta, message)
            values ($run_id, $candidate_id, '$(sqlq "$code")', 'deterministic', '$sev', '$(sqlq "$metric")', $value, $threshold, $delta, '$(sqlq "$message")');"
    done < <("${side}_score")
}

printf 'database: %s\n\n' "$db"
printf '%-42s %7s %7s %7s %s\n' slug tech edit total accepted
seen=0

for meta in "$out_dir"/*/meta.json; do
    [ -f "$meta" ] || continue
    dir=$(dirname "$meta")
    slug=$(jq -r '.slug // empty' "$meta")
    [ -n "$slug" ] || slug=$(basename "$dir")
    vibe=$(jq -r '.vibe // ""' "$meta")
    target=$(jq -r '.audio.final.duration_sec // .audio.duration_sec // 210' "$meta")
    image_sel=$(jq -r '.image.selected // 1' "$meta")
    audio_sel=$(jq -r '.audio.draft.selected // 1' "$meta")
    image_path="$dir/thumbnail.png"
    audio_path="$dir/audio.mp3"

    if [ ! -f "$image_path" ] || [ ! -f "$audio_path" ]; then
        printf '%-42s skipped, missing thumbnail or audio\n' "$slug" >&2
        continue
    fi

    existing=$(sqlite3 "$db" "select id from runs where slug = '$(sqlq "$slug")' limit 1;")
    if [ -n "$existing" ]; then
        if [ "$replace" = "0" ]; then
            printf '%-42s already in the database, use --replace\n' "$slug" >&2
            continue
        fi
        sqlite3 "$db" "delete from findings where run_id = $existing;
            delete from candidates where run_id = $existing;
            delete from patches where run_id = $existing;
            delete from runs where id = $existing;"
    fi

    image_metrics "$image_path"
    audio_metrics "$audio_path" "$target"

    technical=$(awk -v u="$IMG_COLORS" -v s="$IMG_STD" -v o="$IMG_OCR" -v g="$IMG_GLOW" -v sat="$IMG_SAT" \
        -v l="$AUD_LUFS" -v p="$AUD_PEAK" -v c="$AUD_CENTROID" -v h="$AUD_HIGH_SHARE" \
        -v b="$AUD_BUILD" -v z="$AUD_SILENCE" -v d="$AUD_DURATION" -v t="$AUD_TARGET" '
        function lerp(x, x0, x1, y0, y1) {
            if (x1 == x0) return y1
            v = y0 + (x - x0) * (y1 - y0) / (x1 - x0)
            lo = (y0 < y1 ? y0 : y1); hi = (y0 < y1 ? y1 : y0)
            return (v < lo ? lo : (v > hi ? hi : v))
        }
        BEGIN {
            log2 = log(2)
            if (u <= 256) u_p = 20
            else if (u <= 2048) u_p = lerp(log(u) / log2, 8, 11, 20, 10)
            else if (u <= 16384) u_p = lerp(log(u) / log2, 11, 14, 10, 0)
            else u_p = 0
            if (s >= 0.18) s_p = 10; else if (s >= 0.10) s_p = 10 * (s - 0.10) / 0.08; else s_p = 0
            o_p = 10 - 10 * o; if (o_p < 0) o_p = 0
            if (g <= 0.002) g_p = 5; else if (g <= 0.02) g_p = 5 * (0.02 - g) / 0.018; else g_p = 0
            if (sat >= 0.30 && sat <= 0.75) sat_p = 5
            else { dist = (sat < 0.30 ? 0.30 - sat : sat - 0.75); sat_p = 5 * (1 - dist / 0.15); if (sat_p < 0) sat_p = 0 }

            k = (l + 15 < 0 ? -(l + 15) : l + 15)
            l_p = (k <= 1 ? 8 : (k <= 2 ? 5 : 0))
            p_p = (p <= -1 ? 8 : (p <= -0.3 ? 4 : 0))
            if (c >= 2000 && c <= 4500) c_p = 8
            else if (c >= 1200 && c < 2000) c_p = lerp(c, 1200, 2000, 4, 8)
            else if (c > 4500 && c <= 6000) c_p = lerp(c, 4500, 6000, 8, 4)
            else c_p = 0
            if (h >= 0.08) h_p = 8; else if (h >= 0.03) h_p = lerp(h, 0.03, 0.08, 4, 8); else h_p = lerp(h, 0, 0.03, 0, 4)
            if (b <= -2) b_p = 8; else if (b <= 0) b_p = lerp(b, -2, 0, 8, 4); else b_p = lerp(b, 0, 2, 4, 0)
            z_p = (z <= 1 ? 5 : (z <= 2 ? 5 * (2 - z) : 0))
            d_p = ((d - t <= 2 && t - d <= 2) ? 5 : 0)

            printf "%.2f", u_p + s_p + o_p + g_p + sat_p + l_p + p_p + c_p + h_p + b_p + z_p + d_p
        }')

    editorial=""
    provider=""
    model=""
    allowance="null"
    verdict="$dir/work/judge/verdict.json"
    if [ -f "$verdict" ]; then
        editorial=$(jq -r '.omni.best.score // empty' "$verdict")
        provider=$(jq -r '.provider // empty' "$verdict")
        model=$(jq -r '.model // empty' "$verdict")
        allowance=$(jq -r '.usage.opencode.allowance // "null"' "$verdict")
    fi

    total=$(awk -v t="$technical" -v e="$editorial" 'BEGIN { printf "%.2f", (e == "" ? t : 0.6 * t + 0.4 * e) }')

    gates_ok=$(awk -v a="$IMG_ASPECT" -v w="$IMG_WIDTH" 'BEGIN {
        target = 16 / 9
        d = a - target; if (d < 0) d = -d
        print (d < 0.02 && w >= 1280 ? 1 : 0)
    }')

    accepted=$(awk -v t="$technical" -v e="$editorial" -v tot="$total" -v g="$gates_ok" 'BEGIN {
        ok = (g == 1 && t >= 85 && tot >= 80 && (e == "" || e >= 70))
        print (ok ? 1 : 0)
    }')

    config_json=$(jq -cn \
        --arg image "$image_path" \
        --arg audio "$audio_path" \
        --argjson aspect "$IMG_ASPECT" \
        --argjson gates "$gates_ok" \
        --arg provider "$provider" \
        --arg model "$model" \
        --arg allowance "$allowance" \
        '{files: {image: $image, audio: $audio}, aspect: $aspect, gates_ok: ($gates == 1),
          judge: (if $provider == "" then null else {provider: $provider, model: $model, allowance: ($allowance | tonumber?)} end)}')

    started=$(date -r "$meta" --iso-8601=seconds)
    run_id=$(sqlite3 "$db" "insert into runs (started_at, slug, vibe, rubric, technical, editorial, total, accepted, outcome, rounds, cost_usd, seconds, config_json)
        values ('$started', '$(sqlq "$slug")', '$(sqlq "$vibe")', 'score-v1', $technical, ${editorial:-null}, $total, $accepted, 'below_target', 0, null, null, '$(sqlq "$config_json")')
        returning id;")

    image_n=$(jq '.image.candidates | length' "$meta")
    image_cid=""
    if [ "$image_n" -gt 0 ]; then
        i=1
        while [ "$i" -le "$image_n" ]; do
            seed=$(jq -r --argjson i "$i" '.image.candidates[$i - 1].seed // "null"' "$meta")
            cid=$(insert_candidate "$run_id" image "$i" "$seed")
            if [ "$i" -eq "$image_sel" ]; then image_cid=$cid; fi
            i=$((i + 1))
        done
    else
        image_cid=$(insert_candidate "$run_id" image 1 null)
    fi

    audio_n=$(jq '.audio.draft.candidates | length' "$meta")
    audio_cid=""
    if [ "$audio_n" -gt 0 ]; then
        i=1
        while [ "$i" -le "$audio_n" ]; do
            seed=$(jq -r --argjson i "$i" '.audio.draft.candidates[$i - 1].seed // "null"' "$meta")
            cid=$(insert_candidate "$run_id" audio "$i" "$seed")
            if [ "$i" -eq "$audio_sel" ]; then audio_cid=$cid; fi
            i=$((i + 1))
        done
    else
        seed=$(jq -r '.seed // "null"' "$meta")
        audio_cid=$(insert_candidate "$run_id" audio 1 "$seed")
    fi

    [ -n "$image_cid" ] || image_cid=null
    [ -n "$audio_cid" ] || audio_cid=null
    insert_findings "$run_id" "$image_cid" image
    insert_findings "$run_id" "$audio_cid" audio

    if [ "$gates_ok" = "0" ]; then
        sqlite3 "$db" "insert into findings (run_id, candidate_id, code, source, severity, metric, value, threshold, delta, message)
            values ($run_id, $image_cid, 'GATE_TYPE', 'deterministic', 'high', 'aspect', $IMG_ASPECT, 1.7778, -20, 'thumbnail is not 16:9 at 1280x720 or more');"
    fi

    printf '%-42s %7.1f %7s %7.1f %s\n' "$slug" "$technical" "${editorial:--}" "$total" "$accepted"
    seen=$((seen + 1))
done

echo
sqlite3 -header -column "$db" \
    "select code, count(*) as runs, round(sum(delta), 1) as points
     from findings group by code order by sum(delta);"

echo
printf 'runs: %s, candidates: %s, findings: %s\n' \
    "$(sqlite3 "$db" 'select count(*) from runs;')" \
    "$(sqlite3 "$db" 'select count(*) from candidates;')" \
    "$(sqlite3 "$db" 'select count(*) from findings;')"

if [ "$seen" -eq 0 ]; then
    echo "nothing to backfill: every run is already in the database (use --replace)" >&2
fi
