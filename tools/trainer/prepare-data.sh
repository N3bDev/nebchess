#!/usr/bin/env bash
# Convert + shuffle plan-9 self-play shards into one bulletformat .bin for training.
# Usage: prepare-data.sh <shard-dir> <out.bin> [mem_mb=16384]
# Our shards are already `FEN | cp_white | wdl_white` — bullet's `convert --from text`
# ingests this directly (white-relative cp + 1.0/0.5/0.0 result).
set -euo pipefail

SHARD_DIR="${1:?usage: prepare-data.sh <shard-dir> <out.bin> [mem_mb]}"
OUT="${2:?usage: prepare-data.sh <shard-dir> <out.bin> [mem_mb]}"
MEM_MB="${3:-16384}"
UTILS="${BULLET_UTILS:-/home/witt/bullet/target/release/bullet-utils}"

tmp_txt="$(mktemp --suffix=.txt)"
tmp_bin="$(mktemp --suffix=.bin)"
trap 'rm -f "$tmp_txt" "$tmp_bin"' EXIT

echo "[prepare-data] concatenating shards from $SHARD_DIR"
cat "$SHARD_DIR"/shard_*.txt > "$tmp_txt"
echo "[prepare-data] lines: $(wc -l < "$tmp_txt")"

echo "[prepare-data] convert text -> bulletformat"
"$UTILS" convert --from text --input "$tmp_txt" --output "$tmp_bin" --threads 8

echo "[prepare-data] shuffle -> $OUT (mem ${MEM_MB} MB)"
"$UTILS" shuffle --input "$tmp_bin" --output "$OUT" --mem-used-mb "$MEM_MB"

bytes=$(stat -c%s "$OUT")
echo "[prepare-data] done: $OUT  ($bytes bytes = $((bytes / 32)) positions @ 32 B/record)"
