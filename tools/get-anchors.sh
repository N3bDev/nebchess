#!/usr/bin/env bash
# Download and verify the anchored-gauntlet engine pool.
# Engines are fixed public versions with known CCRL Blitz ratings.
# PROTOCOL: Do not change engine versions without bumping this header and
# re-running an anchored gauntlet — anchor ratings are version-specific.
#
# Pool (CCRL Blitz ratings as of 2025-10 list — re-pull before any public claim):
#   RusticAlpha2  ~1815   https://codeberg.org/mvanthoor/rustic (alpha-3.0.0 CCRL zip)
#   Stash13       ~1972   https://gitlab.com/mhouppin/stash-bot (tag v13)
#   Stash15       ~2140   https://gitlab.com/mhouppin/stash-bot (tag v15)
#   Stash17       ~2298   https://gitlab.com/mhouppin/stash-bot (tag v17)
#   Stash19       ~2473   https://gitlab.com/mhouppin/stash-bot (tag v19)
set -uo pipefail
cd "$(dirname "$0")"

ANCHOR_DIR="bin/anchors"
mkdir -p "$ANCHOR_DIR"

RATINGS_FILE="$ANCHOR_DIR/ratings.txt"
> "$RATINGS_FILE"

VERIFIED=0
ERRORS=""

# ---------------------------------------------------------------------------
# Helper: UCI handshake — returns 0 if uciok received within 10s
# ---------------------------------------------------------------------------
verify_uci() {
    local bin="$1"
    local output
    output=$(printf 'uci\nquit\n' | timeout 10 "$bin" 2>&1 || true)
    if echo "$output" | grep -q "uciok"; then
        return 0
    else
        return 1
    fi
}

# ---------------------------------------------------------------------------
# Helper: download with resilience — skip on 404/error
# ---------------------------------------------------------------------------
download_engine() {
    local name="$1"
    local url="$2"
    local dest="$3"
    local http_code
    http_code=$(curl -fsSL -o "$dest" -w "%{http_code}" "$url" 2>/dev/null || echo "000")
    if [[ "$http_code" != "200" ]]; then
        echo "SKIP [$name]: download failed (HTTP $http_code) URL: $url" >&2
        rm -f "$dest"
        return 1
    fi
    chmod +x "$dest"
    return 0
}

# ---------------------------------------------------------------------------
# 1. Rustic Alpha 2  (~1815 CCRL Blitz)
#    Source: codeberg.org/mvanthoor/rustic releases/tag/alpha-3.0.0
#    The alpha-3.0.0 "CCRL" zip contains the exact alpha-2 binary used by CCRL.
# ---------------------------------------------------------------------------
NAME="RusticAlpha2"
DEST="$ANCHOR_DIR/rustic-alpha-2-linux64"
ZIP_URL="https://codeberg.org/mvanthoor/rustic/releases/download/alpha-3.0.0/rustic-alpha-ccrl.zip"
TMPZIP="/tmp/rustic-alpha-ccrl-$$.zip"

echo "Downloading $NAME ..."
http_code=$(curl -fsSL -o "$TMPZIP" -w "%{http_code}" "$ZIP_URL" 2>/dev/null || echo "000")
if [[ "$http_code" == "200" ]]; then
    unzip -p "$TMPZIP" "rustic-alpha-ccrl/rustic-alpha-2-linux64" > "$DEST" 2>/dev/null
    chmod +x "$DEST"
    rm -f "$TMPZIP"
    if verify_uci "$DEST"; then
        echo "OK  $NAME -> $DEST"
        echo "RusticAlpha2 1815" >> "$RATINGS_FILE"
        VERIFIED=$(( VERIFIED + 1 ))
    else
        echo "SKIP [$NAME]: UCI handshake failed (binary ran but no uciok)" >&2
        ERRORS="$ERRORS\n  $NAME: UCI handshake failed"
        rm -f "$DEST"
    fi
else
    echo "SKIP [$NAME]: zip download failed (HTTP $http_code) URL: $ZIP_URL" >&2
    ERRORS="$ERRORS\n  $NAME: HTTP $http_code from $ZIP_URL"
    rm -f "$TMPZIP"
fi

# ---------------------------------------------------------------------------
# 2-5. Stash versions: v13 (~1972), v15 (~2140), v17 (~2298), v19 (~2473)
#    Assets live at:  https://gitlab.com/mhouppin/stash-bot/uploads/<hash>/<file>
#    Tag v15 is "v15" (not v15.0), v13 is "v13", v17 is "v17.0", v19 is "v19.2"
#    We pin the MAJOR version that CCRL tested; use the latest minor of that major.
# ---------------------------------------------------------------------------
declare -A STASH_ENTRIES
# format: "label=upload_path rating"
# Using x86_64 (generic, not bmi2) for widest compatibility
STASH_ENTRIES["Stash13"]="4ff97bc58d4b3801d525bf723e0574e7/stash-13.0-linux-x86_64 1972"
STASH_ENTRIES["Stash15"]="56cba735a1572e7b665b5571d0abb486/stash-15.0-linux-x86_64 2140"
STASH_ENTRIES["Stash17"]="058f6a6706656223502f0222d861471c/stash-17.0-linux-x86_64 2298"
STASH_ENTRIES["Stash19"]="5c37aa29f0c5e25ab7e935013d5cfb8d/stash-19.1-linux-x86_64 2473"

GITLAB_UPLOAD_BASE="https://gitlab.com/mhouppin/stash-bot/uploads"

for ENTRY_NAME in Stash13 Stash15 Stash17 Stash19; do
    ENTRY_VAL="${STASH_ENTRIES[$ENTRY_NAME]}"
    UPLOAD_PATH="${ENTRY_VAL% *}"
    RATING="${ENTRY_VAL##* }"
    FILENAME="${UPLOAD_PATH##*/}"
    URL="$GITLAB_UPLOAD_BASE/$UPLOAD_PATH"
    DEST="$ANCHOR_DIR/$FILENAME"

    echo "Downloading $ENTRY_NAME ..."
    if download_engine "$ENTRY_NAME" "$URL" "$DEST"; then
        if verify_uci "$DEST"; then
            echo "OK  $ENTRY_NAME -> $DEST"
            echo "$ENTRY_NAME $RATING" >> "$RATINGS_FILE"
            VERIFIED=$(( VERIFIED + 1 ))
        else
            echo "SKIP [$ENTRY_NAME]: UCI handshake failed" >&2
            ERRORS="$ERRORS\n  $ENTRY_NAME: UCI handshake failed (binary ran but no uciok)"
            rm -f "$DEST"
        fi
    else
        ERRORS="$ERRORS\n  $ENTRY_NAME: download failed from $URL"
    fi
done

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo "=== Verified pool ($VERIFIED anchors) ==="
if [[ -s "$RATINGS_FILE" ]]; then
    while IFS=' ' read -r eng rat; do
        echo "  $eng  $rat"
    done < "$RATINGS_FILE"
fi

if [[ $VERIFIED -lt 3 ]]; then
    echo "" >&2
    echo "BLOCKED: only $VERIFIED anchor(s) verified; minimum 3 required." >&2
    echo "Failed engines:" >&2
    printf "%b\n" "$ERRORS" >&2
    exit 1
fi

echo ""
echo "get-anchors.sh complete: $VERIFIED anchor(s) ready in $ANCHOR_DIR"
