#!/usr/bin/env bash
# Anchored gauntlet: measure nebchess absolute rating on CCRL Blitz scale.
# Usage: tools/anchored-gauntlet.sh [games_per_pairing=300]
#
# Runs sequential fastchess matches (nebchess vs each anchor), then uses Ordo
# with all anchors pinned to their published CCRL Blitz ratings to compute
# nebchess's absolute rating estimate + error bars.
#
# PROTOCOL: TC=10+0.1, Hash=16, Threads=1, 8moves_v3 book (random order).
# Do NOT change TC or book without re-running the full gauntlet and updating
# docs/strength-log.md — anchor ratings are specific to their CCRL conditions;
# this protocol is intentionally close to CCRL Blitz (10+0.1).
set -euo pipefail
cd "$(dirname "$0")"

GAMES="${1:-300}"
ROUNDS=$(( GAMES / 2 ))
CONCURRENCY=$(( $(nproc) - 1 ))
[[ $CONCURRENCY -lt 1 ]] && CONCURRENCY=1

NEBCHESS="$(realpath ../target/release/nebchess)"
FASTCHESS="$(realpath bin/fastchess)"
ORDO="$(realpath bin/ordo)"
BOOK="$(realpath books/8moves_v3.pgn)"
RATINGS_FILE="$(realpath bin/anchors/ratings.txt)"
ANCHOR_DIR="$(realpath bin/anchors)"

# Sanity checks
for f in "$NEBCHESS" "$FASTCHESS" "$ORDO" "$BOOK" "$RATINGS_FILE"; do
    if [[ ! -f "$f" ]]; then
        echo "ERROR: required file not found: $f" >&2
        echo "  Run: cargo build --release  (for nebchess)" >&2
        echo "       tools/get-anchors.sh   (for anchors + ratings.txt)" >&2
        exit 1
    fi
done

if [[ ! -x "$NEBCHESS" ]]; then
    echo "ERROR: $NEBCHESS is not executable" >&2; exit 1
fi

echo "=== Anchored Gauntlet ==="
echo "nebchess : $NEBCHESS"
echo "games/pairing: $GAMES  (rounds: $ROUNDS, -repeat => $((ROUNDS * 2)) games total per anchor)"
echo "concurrency  : $CONCURRENCY"
echo ""

# ---------------------------------------------------------------------------
# Phase 1: Run one fastchess match per anchor
# ---------------------------------------------------------------------------
PGN_FILES=()

while IFS=' ' read -r anchor_name _rating; do
    [[ -z "$anchor_name" || "$anchor_name" == \#* ]] && continue

    # Find the binary for this anchor
    anchor_bin=""
    case "$anchor_name" in
        RusticAlpha2) anchor_bin="$ANCHOR_DIR/rustic-alpha-2-linux64" ;;
        Stash13)      anchor_bin="$ANCHOR_DIR/stash-13.0-linux-x86_64" ;;
        Stash15)      anchor_bin="$ANCHOR_DIR/stash-15.0-linux-x86_64" ;;
        Stash17)      anchor_bin="$ANCHOR_DIR/stash-17.0-linux-x86_64" ;;
        Stash19)      anchor_bin="$ANCHOR_DIR/stash-19.1-linux-x86_64" ;;
        *)
            # Generic fallback: try a lowercase name match in anchor dir
            anchor_bin=$(find "$ANCHOR_DIR" -maxdepth 1 -type f -iname "*${anchor_name,,}*" | head -1 || true)
            ;;
    esac

    if [[ -z "$anchor_bin" || ! -x "$anchor_bin" ]]; then
        echo "SKIP [$anchor_name]: binary not found or not executable (expected: $anchor_bin)" >&2
        continue
    fi

    PGN_OUT="gauntlet-anchored-${anchor_name}.pgn"
    LOG_OUT="gauntlet-anchored-${anchor_name}.log"
    rm -f "$PGN_OUT" "$LOG_OUT"

    echo "--- Match: nebchess vs $anchor_name ---"
    echo "    binary : $anchor_bin"
    echo "    pgn    : tools/$PGN_OUT"

    "$FASTCHESS" \
        -engine cmd="$NEBCHESS" name=nebchess \
        -engine cmd="$anchor_bin" name="$anchor_name" \
        -each tc=10+0.1 option.Hash=16 option.Threads=1 \
        -openings file="$BOOK" format=pgn order=random \
        -rounds "$ROUNDS" -repeat -recover \
        -concurrency "$CONCURRENCY" \
        -ratinginterval 10 \
        -pgnout file="$PGN_OUT" \
        2>&1 | tee "$LOG_OUT"

    echo ""
    PGN_FILES+=( "$PGN_OUT" )
done < "$RATINGS_FILE"

if [[ ${#PGN_FILES[@]} -eq 0 ]]; then
    echo "ERROR: no matches completed — check anchor binaries and ratings.txt" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Phase 2: Concatenate PGNs
# ---------------------------------------------------------------------------
ALL_PGN="gauntlet-anchored-all.pgn"
rm -f "$ALL_PGN"
for f in "${PGN_FILES[@]}"; do
    [[ -f "$f" ]] && cat "$f" >> "$ALL_PGN"
done
echo "Combined PGN: tools/$ALL_PGN  ($(wc -l < "$ALL_PGN") lines)"
echo ""

# ---------------------------------------------------------------------------
# Phase 3: Build Ordo multi-anchor pin file
# Format required by ordo -m: "AnchorName",Rating
# ---------------------------------------------------------------------------
ANCHOR_PIN="/tmp/nebchess-ordo-anchors-$$.csv"
while IFS=' ' read -r anchor_name rating; do
    [[ -z "$anchor_name" || "$anchor_name" == \#* ]] && continue
    echo "\"$anchor_name\",$rating"
done < "$RATINGS_FILE" > "$ANCHOR_PIN"

echo "Ordo anchor pins:"
cat "$ANCHOR_PIN"
echo ""

# ---------------------------------------------------------------------------
# Phase 4: Run Ordo
# -m : multi-anchor pin file (format: "Name",Rating)
# -s : simulations for error bars
# -W : auto-adjust white advantage
# -D : auto-adjust draw rate
# -G : force run even if database has disconnected groups (sparse smoke tests)
# -p : input PGN
# ---------------------------------------------------------------------------
ORDO_OUTPUT="/tmp/nebchess-ordo-results-$$.txt"
"$ORDO" \
    -p "$ALL_PGN" \
    -m "$ANCHOR_PIN" \
    -s 1000 \
    -W \
    -D \
    -G \
    -o "$ORDO_OUTPUT" \
    -q || true   # Ordo may exit non-zero with warnings; capture output regardless

echo "=== Ordo Rating Table ==="
cat "$ORDO_OUTPUT"
echo ""

# ---------------------------------------------------------------------------
# Phase 5: Extract nebchess rating line
# ---------------------------------------------------------------------------
echo "=== nebchess absolute estimate ==="
NEBCHESS_LINE=$(grep -E "nebchess" "$ORDO_OUTPUT" || true)
if [[ -n "$NEBCHESS_LINE" ]]; then
    echo "$NEBCHESS_LINE"
else
    echo "(nebchess not found in Ordo output — check PGN player names)" >&2
fi
echo ""

# ---------------------------------------------------------------------------
# Phase 6: Forfeit / time-loss scan
# ---------------------------------------------------------------------------
echo "=== Forfeit scan ==="
total_forfeits=0
for f in "${PGN_FILES[@]}"; do
    [[ ! -f "$f" ]] && continue
    n=$(grep -ci "time forfeit" "$f" 2>/dev/null || true)
    [[ "$n" -gt 0 ]] && echo "  $f: $n time forfeit(s)"
    total_forfeits=$(( total_forfeits + n ))
done
for logf in gauntlet-anchored-*.log; do
    [[ ! -f "$logf" ]] && continue
    if grep -qEi "loses on time|timeout|illegal move|crash" "$logf" 2>/dev/null; then
        echo "  WARNING: suspicious event in $logf"
        grep -Ei "loses on time|timeout|illegal move|crash" "$logf" | head -5
    fi
done
echo "Total time forfeits across all PGNs: $total_forfeits (informational)"

# Cleanup temp files
rm -f "$ANCHOR_PIN" "$ORDO_OUTPUT"

echo ""
echo "=== Anchored gauntlet complete ==="
