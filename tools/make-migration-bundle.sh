#!/usr/bin/env bash
# Package the MUST-COPY, not-in-git artifacts for moving NebChess dev to a new
# machine. `git clone` brings the repo (source/tests/docs/specs/plans/tools-scripts);
# this bundles what clone won't: the private db/ game corpora and the Claude Code
# project memory (~/.claude/.../memory). Everything else (Syzygy, books, anchors,
# fastchess) re-downloads/rebuilds via the tools/ provisioner scripts on the desktop.
#
# Usage: tools/make-migration-bundle.sh [out-basename] [--with-stockfish] [--with-anchors] [--with-tb]
#   default bundle  = db/*.pgn (Zone.Identifier excluded) + the memory dir
#   --with-stockfish = also fold in tools/bin/stockfish (113 MB; else re-download)
#   --with-anchors   = also fold in tools/bin/anchors/   (5.5 MB; else tools/get-anchors.sh)
#   --with-tb        = also fold in tools/tb/ (~939 MB; normally re-download via download-syzygy.sh)
# Output: <out>.tar.zst (or .tar.gz), plus a printed manifest + sha256.
# See ONBOARDING.md §4a / §5 for how to unpack on the desktop.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd -P)"
OUT="nebchess-migration"
WITH_SF=0 WITH_ANCHORS=0 WITH_TB=0
for a in "$@"; do
  case "$a" in
    --with-stockfish) WITH_SF=1 ;;
    --with-anchors)   WITH_ANCHORS=1 ;;
    --with-tb)        WITH_TB=1 ;;
    --*) echo "unknown flag: $a" >&2; exit 2 ;;
    *) OUT="$a" ;;
  esac
done

# Memory lives at ~/.claude/projects/<abs-repo-path with / -> ->/memory
ENC="$(printf '%s' "$REPO" | sed 's#/#-#g')"
MEMDIR="$HOME/.claude/projects/$ENC/memory"

# Compression: prefer zstd (fast, strong on text PGN), else gzip.
if command -v zstd >/dev/null 2>&1; then CFLAG=(--zstd); EXT="tar.zst"; else CFLAG=(-z); EXT="tar.gz"; fi
ARCHIVE="$REPO/$OUT.$EXT"

# Assemble the -C/path include list.
INCLUDES=()
if [ -d "$REPO/db" ]; then INCLUDES+=( -C "$REPO" db ); else echo "WARN: no db/ at $REPO/db" >&2; fi
if [ -d "$MEMDIR" ]; then INCLUDES+=( -C "$(dirname "$MEMDIR")" memory ); else
  echo "WARN: memory not found at $MEMDIR — bundle will omit it (its content is also summarized in ONBOARDING.md)" >&2
fi
[ "$WITH_SF" = 1 ]      && [ -f "$REPO/tools/bin/stockfish" ] && INCLUDES+=( -C "$REPO" tools/bin/stockfish )
[ "$WITH_ANCHORS" = 1 ] && [ -d "$REPO/tools/bin/anchors" ]   && INCLUDES+=( -C "$REPO" tools/bin/anchors )
[ "$WITH_TB" = 1 ]      && [ -d "$REPO/tools/tb" ]            && INCLUDES+=( -C "$REPO" tools/tb )

if [ "${#INCLUDES[@]}" -eq 0 ]; then echo "nothing to bundle" >&2; exit 1; fi

echo "Bundling -> $ARCHIVE"
echo "  repo:   $REPO"
echo "  memory: $MEMDIR"
echo "  extras: stockfish=$WITH_SF anchors=$WITH_ANCHORS tb=$WITH_TB"
echo "Creating archive (Windows ADS *Zone.Identifier excluded)..."
tar "${CFLAG[@]}" --exclude='*Zone.Identifier' -cf "$ARCHIVE" "${INCLUDES[@]}"

echo
echo "=== bundle manifest (top-level) ==="
tar -tf "$ARCHIVE" | sed 's#/.*##' | sort -u
echo "=== size + checksum ==="
ls -lh "$ARCHIVE" | awk '{print $5"  "$9}'
sha256sum "$ARCHIVE"
echo
echo "Next: copy this file to the desktop, clone the repo there, then inside the clone:"
echo "  tar --zstd -xvf $(basename "$ARCHIVE")    # (use -xzvf for .tar.gz)"
echo "  # this restores db/ ; place the unpacked memory/ at:"
echo "  #   \$HOME/.claude/projects/<encoded-clone-path>/memory/   (see ONBOARDING.md §5)"
