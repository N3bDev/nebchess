#!/usr/bin/env bash
# Fetch the prebuilt PolyGlot opening book (nebbook.bin) from the GitHub release.
# The book is a build artifact (gitignored in-repo) — 328k entries from 797k
# OTB ≥2400 games, +51.6 elo self-play. Pinned to the release tag for
# reproducibility. Non-fatal: the engine runs bookless if this is skipped.
#
# Usage: tools/download-book.sh [tag]   (default: the version in Cargo.toml)
set -uo pipefail
cd "$(dirname "$0")"
mkdir -p books
DEST="books/nebbook.bin"

if [ -s "$DEST" ]; then
  echo "book: already present ($(wc -c < "$DEST") bytes) — skipping"
  exit 0
fi

# Default to the crate version's tag (so a checkout fetches its matching book).
TAG="${1:-v$(grep -m1 '^version' ../Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')}"
URL="https://github.com/N3bDev/nebchess/releases/download/${TAG}/nebbook.bin"

echo "book: fetching $URL"
if curl -fsSL --max-time 120 -o "$DEST" "$URL"; then
  echo "book: ok ($(wc -c < "$DEST") bytes) -> $DEST"
else
  echo "book: download FAILED (tag $TAG missing the asset, or no network) — engine will run bookless." >&2
  rm -f "$DEST"
  exit 0   # non-fatal, per the bot-readiness convention
fi
