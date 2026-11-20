#!/usr/bin/env bash
# Imports reference images you downloaded by hand into a concept library.
#
# Use it for CC0 packs from OpenGameArt, your own art, or anything you have a
# license for. It copies, normalizes and deduplicates; it never downloads.
# The library is only a folder, local or mounted, and is never copied into the
# repository.
#
# Usage:
#   scripts/import-references.sh --concept NAME --license TEXT [options] SOURCE...
#
# Options:
#   --concept NAME     destination concept folder (required)
#   --license TEXT     license of this batch, for example "CC0-1.0" (required)
#   --author TEXT      author or pack name
#   --source-url URL   page where the pack came from
#   --out DIR          library root; default $IMAGE_REFERENCES, else
#                      image_references from config.toml, else
#                      ~/.local/share/melodias-studio/references
#   --size N           longest side after resizing, default 768; never upscales
#   --prefix NAME      file prefix, default "import"
#   --dry-run          list what would be imported, write nothing
#
# SOURCE can be an image, a folder (searched recursively) or a .zip pack
# (needs unzip). Every image becomes a PNG with metadata stripped, named
# import-<sha8>.png, so importing the same folder twice is harmless.
#
# Each import is recorded in $out/sources.jsonl with its license, author and
# source URL.
#
# Requires: ImageMagick 7 (magick), jq and sha256sum (or shasum).

set -euo pipefail
export LC_ALL=C

out=""
concept=""
license=""
author=""
source_url=""
size=768
prefix=import
dry_run=0
sources=()

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
        --concept) concept=${2:-}; shift 2 ;;
        --license) license=${2:-}; shift 2 ;;
        --author) author=${2:-}; shift 2 ;;
        --source-url) source_url=${2:-}; shift 2 ;;
        --size) size=${2:-}; shift 2 ;;
        --prefix) prefix=${2:-}; shift 2 ;;
        --dry-run) dry_run=1; shift ;;
        -h|--help) usage; exit 0 ;;
        -*) die "unknown option: $1" ;;
        *) sources+=("$1"); shift ;;
    esac
done

[ -n "$concept" ] || die "missing --concept"
[ -n "$license" ] || die "missing --license (for example --license CC0-1.0)"
[[ "$concept" =~ ^[A-Za-z0-9._-]+$ ]] || die "invalid concept name: $concept"
[ "$concept" != "." ] && [ "$concept" != ".." ] || die "invalid concept name: $concept"
[ -n "$size" ] || die "missing value for --size"
[[ "$size" =~ ^[0-9]+$ ]] || die "--size must be a number"
[ "$size" -ge 64 ] || die "--size must be at least 64"
[[ "$prefix" =~ ^[A-Za-z0-9._-]+$ ]] || die "invalid prefix: $prefix"
[ ${#sources[@]} -gt 0 ] || die "missing SOURCE (a file, a folder or a .zip)"

for bin in jq; do
    command -v "$bin" >/dev/null || die "missing $bin on PATH"
done
command -v magick >/dev/null || command -v convert >/dev/null || die "missing ImageMagick (magick) on PATH"

magick_bin=magick
command -v magick >/dev/null || magick_bin=convert

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

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
hashes="$work/hashes"
: > "$hashes"
manifest="$out/sources.jsonl"
if [ -f "$manifest" ]; then
    jq -r 'select(.sha256 != null) | .sha256' "$manifest" >> "$hashes" 2>/dev/null || true
fi

is_image() {
    case "${1,,}" in
        *.png|*.jpg|*.jpeg|*.webp|*.gif|*.bmp) return 0 ;;
        *) return 1 ;;
    esac
}

is_zip() {
    case "${1,,}" in
        *.zip) return 0 ;;
        *) return 1 ;;
    esac
}

pack=0
collect() {
    local path=$1
    if [ -d "$path" ]; then
        find "$path" -type f \( -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' \
            -o -iname '*.webp' -o -iname '*.gif' -o -iname '*.bmp' \) -print0
    elif is_zip "$path"; then
        command -v unzip >/dev/null || die "missing unzip for $path"
        pack=$((pack + 1))
        local target="$work/pack-$pack"
        mkdir -p "$target"
        unzip -q -o "$path" -d "$target" \
            -x '__MACOSX/*' '*/._*' '._*' >/dev/null
        find "$target" -type f \( -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' \
            -o -iname '*.webp' -o -iname '*.gif' -o -iname '*.bmp' \) -print0
    elif is_image "$path"; then
        printf '%s\0' "$path"
    else
        warn "not an image, folder or zip: $path" >&2
    fi
}

imported=0
skipped=0
failed=0
seen=0

for source in "${sources[@]}"; do
    if [ ! -e "$source" ]; then
        warn "no such path: $source"
        failed=$((failed + 1))
        continue
    fi
    while IFS= read -r -d '' src; do
        [ -n "$src" ] || continue
        is_image "$src" || continue
        seen=$((seen + 1))
        tmp="$work/$concept-$imported.png"
        if ! "$magick_bin" "$src[0]" -auto-orient -resize "${size}x${size}>" -strip \
            -define png:exclude-chunks=date,time "PNG32:$tmp" 2>/dev/null; then
            warn "could not read $src"
            failed=$((failed + 1))
            continue
        fi
        tmp_size=$(file_size "$tmp")
        if [ "$tmp_size" -lt 200 ]; then
            failed=$((failed + 1))
            continue
        fi
        hash=$(sha256 "$tmp")
        short=${hash:0:8}
        if grep -qxF "$hash" "$hashes"; then
            skipped=$((skipped + 1))
            continue
        fi
        target="$out/$concept/$prefix-$short.png"
        if [ -f "$target" ]; then
            skipped=$((skipped + 1))
            printf '%s\n' "$hash" >> "$hashes"
            continue
        fi
        if [ "$dry_run" -eq 1 ]; then
            echo "would import $src -> $concept/$prefix-$short.png"
            imported=$((imported + 1))
            continue
        fi
        mkdir -p "$out/$concept"
        mv -f "$tmp" "$target"
        printf '%s\n' "$hash" >> "$hashes"
        jq -cn \
            --arg concept "$concept" \
            --arg file "$concept/$prefix-$short.png" \
            --arg license "$license" \
            --arg author "${author:-unknown}" \
            --arg source_url "$source_url" \
            --arg original "$(basename "$src")" \
            --arg sha256 "$hash" \
            --arg imported_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
            '{
                concept: $concept,
                file: $file,
                source: "import",
                license: $license,
                author: $author,
                source_url: $source_url,
                original: $original,
                sha256: $sha256,
                imported_at: $imported_at
            }' >> "$manifest"
        imported=$((imported + 1))
    done < <(collect "$source")
done

if [ "$dry_run" -eq 1 ]; then
    echo "dry run: $imported would be imported, $failed failed, nothing written"
    exit 0
fi

echo "imported $imported, skipped $skipped duplicates, $failed failed"
echo "library: $out/$concept"
echo "manifest: $manifest"
if [ "$imported" -eq 0 ] && [ "$skipped" -eq 0 ] && [ "$seen" -eq 0 ]; then
    echo "no images found in the sources" >&2
fi
