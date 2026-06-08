#!/usr/bin/env bash
# Fetch the complete Syzygy 3-4-5-men tablebase set (WDL .rtbw + DTZ .rtbz,
# ~1 GB, 290 files) into tools/tb/ (gitignored). NON-FATAL: an unreachable
# mirror prints a warning and exits 0 so CI / the build chain is never blocked.
#
# Point the engine at the result with: setoption name SyzygyPath value tools/tb
#
# Sources (tried in order):
#   1. sesse.net  — combined WDL+DTZ in one directory (preferred)
#   2. lichess    — split 3-4-5-wdl / 3-4-5-dtz directories (fallback)
set -u

DEST="$(cd "$(dirname "$0")/.." && pwd)/tools/tb"
mkdir -p "$DEST"

SESSE="http://tablebase.sesse.net/syzygy/3-4-5"
LICHESS_WDL="https://tablebase.lichess.ovh/tables/standard/3-4-5-wdl"
LICHESS_DTZ="https://tablebase.lichess.ovh/tables/standard/3-4-5-dtz"

reachable() { curl -fsI --max-time 20 "$1" >/dev/null 2>&1; }

# List the *.rtbw / *.rtbz hrefs from an Apache/nginx directory index.
list_files() {
  curl -fs --max-time 60 "$1/" 2>/dev/null \
    | grep -oE 'href="[^"]*\.(rtbw|rtbz)"' \
    | sed 's/href="//;s/"//' \
    | sort -u
}

# Download every file in $2.. from base $1 into $DEST, skipping ones already
# present with a non-zero size. Returns the count fetched via the FETCHED global.
FETCHED=0
fetch_all() {
  local base="$1"; shift
  local f
  for f in "$@"; do
    local out="$DEST/$f"
    if [[ -s "$out" ]]; then continue; fi
    if curl -fs --max-time 600 -o "$out.part" "$base/$f" 2>/dev/null && [[ -s "$out.part" ]]; then
      mv "$out.part" "$out"
      FETCHED=$((FETCHED + 1))
    else
      rm -f "$out.part"
      echo "  WARN: failed to fetch $f from $base" >&2
    fi
  done
}

echo "Syzygy 3-4-5 downloader -> $DEST"

if reachable "$SESSE/KRvK.rtbw"; then
  echo "Source: sesse.net (combined WDL+DTZ)"
  mapfile -t FILES < <(list_files "$SESSE")
  if [[ ${#FILES[@]} -eq 0 ]]; then
    echo "WARN: sesse.net index empty; nothing to do." >&2
    exit 0
  fi
  echo "Index lists ${#FILES[@]} files; downloading (this is ~1 GB)..."
  fetch_all "$SESSE" "${FILES[@]}"
elif reachable "$LICHESS_WDL/"; then
  echo "Source: lichess.ovh (split WDL / DTZ dirs)"
  mapfile -t WDL < <(list_files "$LICHESS_WDL")
  mapfile -t DTZ < <(list_files "$LICHESS_DTZ")
  if [[ ${#WDL[@]} -eq 0 && ${#DTZ[@]} -eq 0 ]]; then
    echo "WARN: lichess index empty; nothing to do." >&2
    exit 0
  fi
  echo "Index lists ${#WDL[@]} WDL + ${#DTZ[@]} DTZ files; downloading (~1 GB)..."
  fetch_all "$LICHESS_WDL" "${WDL[@]}"
  fetch_all "$LICHESS_DTZ" "${DTZ[@]}"
else
  echo "WARN: no Syzygy mirror reachable (sesse.net / lichess.ovh). Skipping." >&2
  echo "      The engine runs fine without tables (SyzygyPath stays off)." >&2
  exit 0
fi

TOTAL=$(find "$DEST" -type f \( -name '*.rtbw' -o -name '*.rtbz' \) | wc -l)
SIZE=$(du -sh "$DEST" 2>/dev/null | cut -f1)
echo "Done. Fetched $FETCHED new file(s); $TOTAL tables present in $DEST ($SIZE)."
