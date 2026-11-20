#!/usr/bin/env bash
# Fetches pixel-art references from Pixabay into a concept library.
#
# The library is only a folder. It can live on a mounted share or in a local
# directory, and it is never copied into the repository. Each concept is a
# subfolder that matches a vibe, for example battle/, forest/ or castle/.
#
# The API key is read from PIXABAY_API_KEY, or from
# $XDG_CONFIG_HOME/melodias-studio/pixabay.key, or from pixabay_api_key in
# config.toml, in that order. It is never written to disk here and never
# belongs in the repository.
#
# Usage:
#   scripts/fetch-pixabay.sh [options]
#
# Options:
#   --out DIR          library root; default $IMAGE_REFERENCES, else
#                      image_references from config.toml, else
#                      ~/.local/share/melodias-studio/references
#   --concept NAME     concept to fill, repeatable; default battle forest
#                      castle jrpg
#   --query TEXT       Pixabay query; only valid with a single --concept,
#                      and required for a concept without a default query
#   --count N          images per concept, default 60, capped at 500
#   --min-width N      skip images narrower than N pixels, default 400
#   --tag TEXT         keep only images whose tags contain TEXT (repeatable,
#                      case-insensitive); --tag pixel drops most stock photos
#   --dry-run          list what would be downloaded, write nothing
#
# Every accepted image is recorded in $out/sources.jsonl with its id, page,
# author, tags and the Pixabay Content License. Duplicates are skipped by
# sha256, across concepts and across runs.
#
# Requires: curl, jq and sha256sum (or shasum).
#
# Pixabay Content License: free use and adaptation, no attribution required.
# Do not redistribute the source images on their own, and avoid logos, brands
# and recognizable people.

set -euo pipefail
export LC_ALL=C

out=""
custom_query=""
count=60
min_width=400
dry_run=0
concepts=()
tags_filter=()

declare -A default_query=(
    [battle]="pixel art knight"
    [forest]="pixel art forest background"
    [castle]="pixel art castle"
    [jrpg]="pixel art game"
)

usage() {
    sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed -e 's/^# \{0,1\}//' -e '/^set -euo/d'
}

die() {
    echo "$*" >&2
    exit 2
}

warn() {
    echo "warning: $*" >&2
}

while [ $# -gt 0 ]; do
    case "$1" in
        --out) out=${2:-}; shift 2 ;;
        --concept) concepts+=("${2:-}"); shift 2 ;;
        --query) custom_query=${2:-}; shift 2 ;;
        --count) count=${2:-}; shift 2 ;;
        --min-width) min_width=${2:-}; shift 2 ;;
        --tag) tags_filter+=("${2:-}"); shift 2 ;;
        --dry-run) dry_run=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

[ -n "$count" ] || die "missing value for --count"
[ -n "$min_width" ] || die "missing value for --min-width"
[[ "$count" =~ ^[0-9]+$ ]] || die "--count must be a number"
[[ "$min_width" =~ ^[0-9]+$ ]] || die "--min-width must be a number"
for tag in "${tags_filter[@]}"; do
    [ -n "$tag" ] || die "empty --tag"
done

for bin in curl jq; do
    command -v "$bin" >/dev/null || die "missing $bin on PATH"
done

sha256() {
    if command -v sha256sum >/dev/null; then
        sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        die "missing sha256sum or shasum on PATH"
    fi
}

file_size() {
    stat -c %s "$1" 2>/dev/null || stat -f %z "$1" 2>/dev/null || echo 0
}

urlencode() {
    jq -rn --arg value "$1" '$value | @uri'
}

config_value() {
    local name=$1
    local cfg=${MELODIAS_CONFIG:-${XDG_CONFIG_HOME:-$HOME/.config}/melodias-studio/config.toml}
    [ -f "$cfg" ] || return 0
    sed -n "s/^[[:space:]]*${name}[[:space:]]*=[[:space:]]*\"\\(.*\\)\"[[:space:]]*$/\\1/p" "$cfg" | tail -1
}

if [ -z "$out" ]; then
    out=${IMAGE_REFERENCES:-}
fi
if [ -z "$out" ]; then
    out=$(config_value image_references)
fi
if [ -z "$out" ]; then
    out="$HOME/.local/share/melodias-studio/references"
fi
case "$out" in
    "~") out=$HOME ;;
    "~/"*) out=$HOME/${out#\~/} ;;
esac
out=${out%/}

if [ ${#concepts[@]} -eq 0 ]; then
    concepts=(battle forest castle jrpg)
fi
if [ -n "$custom_query" ] && [ ${#concepts[@]} -ne 1 ]; then
    die "--query needs exactly one --concept"
fi

for concept in "${concepts[@]}"; do
    [ -n "$concept" ] || die "empty --concept"
    [[ "$concept" =~ ^[A-Za-z0-9._-]+$ ]] || die "invalid concept name: $concept"
    [ "$concept" != "." ] && [ "$concept" != ".." ] || die "invalid concept name: $concept"
    if [ -z "$custom_query" ] && [ -z "${default_query[$concept]:-}" ]; then
        die "no default query for concept '$concept', pass --query"
    fi
done

key=${PIXABAY_API_KEY:-}
key_file=${PIXABAY_KEY_FILE:-${XDG_CONFIG_HOME:-$HOME/.config}/melodias-studio/pixabay.key}
if [ -z "$key" ] && [ -r "$key_file" ]; then
    key=$(tr -d '[:space:]' < "$key_file")
    mode=$(stat -c %a "$key_file" 2>/dev/null || true)
    if [ -n "$mode" ] && [ "${mode: -2}" != "00" ]; then
        warn "$key_file is readable by other users; run: chmod 600 $key_file"
    fi
fi
if [ -z "$key" ]; then
    key=$(config_value pixabay_api_key)
fi
[ -n "$key" ] || die "no Pixabay key: set PIXABAY_API_KEY, write $key_file (chmod 600) or set pixabay_api_key in config.toml"

if [ "$count" -gt 500 ]; then
    warn "--count $count exceeds the Pixabay cap of 500 per query; using 500"
    count=500
fi

per_page=100
max_pages=$(( (count + per_page - 1) / per_page ))
[ "$max_pages" -le 5 ] || max_pages=5

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
hashes="$work/hashes"
: > "$hashes"
manifest="$out/sources.jsonl"
if [ -f "$manifest" ]; then
    jq -r 'select(.sha256 != null) | .sha256' "$manifest" >> "$hashes" 2>/dev/null || true
fi

api_get() {
    local url=$1 body=$2 attempt code
    for attempt in 1 2 3; do
        code=$(curl -sS -o "$body" -w '%{http_code}' "$url" 2>/dev/null || echo 000)
        case "$code" in
            200) return 0 ;;
            429|5*) warn "pixabay answered $code, retrying in $((attempt * 5))s"; sleep $(( attempt * 5 )) ;;
            *) warn "pixabay answered $code"; head -c 300 "$body" >&2 2>/dev/null || true; echo >&2; return 1 ;;
        esac
    done
    warn "pixabay did not answer after 3 attempts"
    return 1
}

declare -A added_by_concept=()
declare -A skipped_by_concept=()
declare -A failed_by_concept=()
declare -A filtered_by_concept=()
total_added=0
total_skipped=0
total_planned=0

for concept in "${concepts[@]}"; do
    query=${custom_query:-${default_query[$concept]}}
    echo "== $concept: \"$query\" (up to $count images)"
    added=0
    skipped=0
    failed=0
    filtered=0
    for (( page = 1; page <= max_pages; page++ )); do
        if [ "$added" -ge "$count" ]; then
            break
        fi
        body="$work/page.json"
        url="https://pixabay.com/api/?key=${key}&q=$(urlencode "$query")&image_type=illustration&safesearch=true&per_page=${per_page}&page=${page}&order=popular"
        if ! api_get "$url" "$body"; then
            failed=$((failed + 1))
            break
        fi
        hits=$(jq '.hits | length' "$body" 2>/dev/null || echo 0)
        if [ "$hits" -eq 0 ]; then
            break
        fi
        while IFS= read -r hit; do
            if [ "$added" -ge "$count" ]; then
                break
            fi
            IFS=$'\t' read -r id web large page_url author width _height tags <<< "$hit"
            if [ -z "$id" ] || { [ -z "$web" ] && [ -z "$large" ]; }; then
                continue
            fi
            [[ "${width:-}" =~ ^[0-9]+$ ]] || width=0
            if [ "${width:-0}" -lt "$min_width" ]; then
                continue
            fi
            if [ ${#tags_filter[@]} -gt 0 ]; then
                tags_lc=${tags,,}
                matched=0
                for tag in "${tags_filter[@]}"; do
                    if [[ "$tags_lc" == *"${tag,,}"* ]]; then
                        matched=1
                        break
                    fi
                done
                if [ "$matched" -eq 0 ]; then
                    filtered=$((filtered + 1))
                    continue
                fi
            fi
            image_url=$web
            [ -n "$image_url" ] || image_url=$large
            ext=${image_url%%\?*}
            ext=${ext##*.}
            ext=${ext,,}
            case "$ext" in
                png|jpg|jpeg|webp) ;;
                *) ext=jpg ;;
            esac
            target="$out/$concept/pixabay-$id.$ext"
            if [ "$dry_run" -eq 1 ]; then
                echo "   would fetch $concept/pixabay-$id.$ext  $page_url"
                added=$((added + 1))
                total_planned=$((total_planned + 1))
                continue
            fi
            if [ -f "$target" ]; then
                skipped=$((skipped + 1))
                continue
            fi
            tmp="$work/$id.$ext"
            if ! curl -sS -f -L --retry 2 -o "$tmp" "$image_url" 2>/dev/null; then
                warn "could not download $page_url"
                failed=$((failed + 1))
                continue
            fi
            size=$(file_size "$tmp")
            if [ "$size" -lt 10000 ]; then
                failed=$((failed + 1))
                continue
            fi
            hash=$(sha256 "$tmp")
            if grep -qxF "$hash" "$hashes"; then
                skipped=$((skipped + 1))
                continue
            fi
            mkdir -p "$out/$concept"
            mv -f "$tmp" "$target"
            printf '%s\n' "$hash" >> "$hashes"
            jq -cn \
                --arg concept "$concept" \
                --arg file "$concept/pixabay-$id.$ext" \
                --arg id "$id" \
                --arg query "$query" \
                --arg page_url "$page_url" \
                --arg author "$author" \
                --arg license "pixabay-content-license" \
                --arg sha256 "$hash" \
                --arg tags "$tags" \
                --arg fetched_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
                '{
                    concept: $concept,
                    file: $file,
                    source: "pixabay",
                    id: $id,
                    query: $query,
                    page_url: $page_url,
                    author: $author,
                    license: $license,
                    sha256: $sha256,
                    tags: $tags,
                    fetched_at: $fetched_at
                }' >> "$manifest"
            added=$((added + 1))
            total_added=$((total_added + 1))
        done < <(jq -r '.hits[] | [.id, .webformatURL, .largeImageURL, .pageURL, .user, .imageWidth, .imageHeight, (.tags // "")] | @tsv' "$body")
        sleep 1
    done
    added_by_concept[$concept]=$added
    skipped_by_concept[$concept]=$skipped
    failed_by_concept[$concept]=$failed
    filtered_by_concept[$concept]=$filtered
    total_skipped=$((total_skipped + skipped))
done

echo "== summary"
for concept in "${concepts[@]}"; do
    printf '   %-12s %3s new %3s skipped %3s filtered %3s failed\n' \
        "$concept" "${added_by_concept[$concept]:-0}" "${skipped_by_concept[$concept]:-0}" \
        "${filtered_by_concept[$concept]:-0}" "${failed_by_concept[$concept]:-0}"
done
if [ "$dry_run" -eq 1 ]; then
    echo "   dry run: nothing written to $out"
else
    echo "   library: $out"
    echo "   manifest: $manifest"
fi
if [ "$total_added" -eq 0 ] && [ "$total_skipped" -eq 0 ] && [ "$total_planned" -eq 0 ]; then
    echo "   no images matched; try another --query" >&2
fi
