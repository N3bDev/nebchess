#!/usr/bin/env bash
# Download and verify the anchored-gauntlet engine pool.
# Engines are fixed public versions with known CCRL Blitz ratings.
# PROTOCOL: Do not change engine versions without bumping this header and
# re-running an anchored gauntlet — anchor ratings are version-specific.
#
# M5 POOL (the five rungs the gauntlet consumes via ratings.txt):
#   Stash15       2140   https://gitlab.com/mhouppin/stash-bot (tag v15)
#   Stash17       2298   https://gitlab.com/mhouppin/stash-bot (tag v17.0)
#   Stash19       2473   https://gitlab.com/mhouppin/stash-bot (tag v19.2)
#   Stash20       2509   https://gitlab.com/mhouppin/stash-bot (tag v20.0.1)
#   Stash21       2714   https://gitlab.com/mhouppin/stash-bot (tag v21.2)
# Ratings are CCRL Blitz reference points (v15/v17/v19 from the 2025-10 list;
# v20=2509, v21=2714 are CCRL Blitz reference points). Re-pull before any
# public claim — anchor ratings are version-specific.
#
# FETCHED BUT NOT IN THE M5 POOL (sub-2000 rungs are blowout-only vs an M5
# engine; downloaded + UCI-verified for archival/optional use, but kept OUT of
# ratings.txt so the gauntlet never plays them):
#   RusticAlpha2  ~1815   https://codeberg.org/mvanthoor/rustic (alpha-3.0.0 CCRL zip)
#   Stash13       ~1972   https://gitlab.com/mhouppin/stash-bot (tag v13)
set -uo pipefail
cd "$(dirname "$0")"

ANCHOR_DIR="bin/anchors"
mkdir -p "$ANCHOR_DIR"

# ratings.txt = the POOL the gauntlet consumes. Only pool members are written
# here; fetched-but-excluded engines (Rustic, Stash13) are verified but omitted.
RATINGS_FILE="$ANCHOR_DIR/ratings.txt"
> "$RATINGS_FILE"

VERIFIED=0          # total binaries that passed UCI handshake (pool + archival)
POOL_VERIFIED=0     # pool members verified (this is what the gauntlet needs)
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
# 1. Rustic Alpha 2  (~1815 CCRL Blitz) — ARCHIVAL, NOT in the M5 pool.
#    Source: codeberg.org/mvanthoor/rustic releases/tag/alpha-3.0.0
#    The alpha-3.0.0 "CCRL" zip contains the exact alpha-2 binary used by CCRL.
#    Fetched + UCI-verified for archival use, but deliberately NOT written to
#    ratings.txt (sub-2000 rung = blowout-only against an M5 engine).
# ---------------------------------------------------------------------------
NAME="RusticAlpha2"
DEST="$ANCHOR_DIR/rustic-alpha-2-linux64"
ZIP_URL="https://codeberg.org/mvanthoor/rustic/releases/download/alpha-3.0.0/rustic-alpha-ccrl.zip"
TMPZIP="/tmp/rustic-alpha-ccrl-$$.zip"

echo "Downloading $NAME (archival, not in pool) ..."
http_code=$(curl -fsSL -o "$TMPZIP" -w "%{http_code}" "$ZIP_URL" 2>/dev/null || echo "000")
if [[ "$http_code" == "200" ]]; then
    unzip -p "$TMPZIP" "rustic-alpha-ccrl/rustic-alpha-2-linux64" > "$DEST" 2>/dev/null
    chmod +x "$DEST"
    rm -f "$TMPZIP"
    if verify_uci "$DEST"; then
        echo "OK  $NAME -> $DEST  (archival; omitted from ratings.txt)"
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
# 2-7. Stash versions: v13, v15, v17, v19, v20, v21
#    Assets live at:  https://gitlab.com/mhouppin/stash-bot/uploads/<hash>/<file>
#    (upload links discovered via the GitLab releases API release descriptions).
#    Tags: v13="v13", v15="v15", v17="v17.0", v19="v19.2" (binary v19.1),
#          v20="v20.0.1", v21="v21.2". We pin the MAJOR version CCRL tested and
#          use the latest minor of that major.
#    M5 POOL = v15, v17, v19, v20, v21 (written to ratings.txt).
#    Stash13 is ARCHIVAL only (sub-2000; fetched + verified, NOT in the pool).
# ---------------------------------------------------------------------------
declare -A STASH_ENTRIES
# format: "label=upload_path rating"
# Using x86_64 (generic, not bmi2/modern) for widest compatibility.
# Ratings: v15/v17/v19 CCRL Blitz (2025-10 list); v20=2509, v21=2714 are CCRL
# Blitz reference points (per the M5 wrap pool definition).
STASH_ENTRIES["Stash13"]="4ff97bc58d4b3801d525bf723e0574e7/stash-13.0-linux-x86_64 1972"
STASH_ENTRIES["Stash15"]="56cba735a1572e7b665b5571d0abb486/stash-15.0-linux-x86_64 2140"
STASH_ENTRIES["Stash17"]="058f6a6706656223502f0222d861471c/stash-17.0-linux-x86_64 2298"
STASH_ENTRIES["Stash19"]="5c37aa29f0c5e25ab7e935013d5cfb8d/stash-19.1-linux-x86_64 2473"
STASH_ENTRIES["Stash20"]="bb23ef7457a5e9e18a87078008b6ee97/stash-20.0.1-linux-x86_64 2509"
STASH_ENTRIES["Stash21"]="4881a30b90418fab74b5c745826c94af/stash-21.2-linux-x86_64 2714"

# Pool membership: only these enter ratings.txt (Stash13 is archival-only).
declare -A IN_POOL=( [Stash15]=1 [Stash17]=1 [Stash19]=1 [Stash20]=1 [Stash21]=1 )

GITLAB_UPLOAD_BASE="https://gitlab.com/mhouppin/stash-bot/uploads"

for ENTRY_NAME in Stash13 Stash15 Stash17 Stash19 Stash20 Stash21; do
    ENTRY_VAL="${STASH_ENTRIES[$ENTRY_NAME]}"
    UPLOAD_PATH="${ENTRY_VAL% *}"
    RATING="${ENTRY_VAL##* }"
    FILENAME="${UPLOAD_PATH##*/}"
    URL="$GITLAB_UPLOAD_BASE/$UPLOAD_PATH"
    DEST="$ANCHOR_DIR/$FILENAME"
    POOL_TAG=""
    [[ -z "${IN_POOL[$ENTRY_NAME]:-}" ]] && POOL_TAG=" (archival, not in pool)"

    echo "Downloading $ENTRY_NAME$POOL_TAG ..."
    if download_engine "$ENTRY_NAME" "$URL" "$DEST"; then
        if verify_uci "$DEST"; then
            VERIFIED=$(( VERIFIED + 1 ))
            if [[ -n "${IN_POOL[$ENTRY_NAME]:-}" ]]; then
                echo "OK  $ENTRY_NAME -> $DEST"
                echo "$ENTRY_NAME $RATING" >> "$RATINGS_FILE"
                POOL_VERIFIED=$(( POOL_VERIFIED + 1 ))
            else
                echo "OK  $ENTRY_NAME -> $DEST  (archival; omitted from ratings.txt)"
            fi
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
echo "=== M5 gauntlet pool ($POOL_VERIFIED anchors in ratings.txt) ==="
if [[ -s "$RATINGS_FILE" ]]; then
    while IFS=' ' read -r eng rat; do
        echo "  $eng  $rat"
    done < "$RATINGS_FILE"
fi
echo "(total binaries verified incl. archival: $VERIFIED)"

# The gauntlet needs the FULL pool; archival binaries don't count toward the
# guard. A partial pool is blocked outright: silently dropping a rung —
# especially v21, the 2714 ceiling — shifts the Ordo anchor math and any
# absolute-rating claim bracketed by it (review finding on b04d2ee).
if [[ $POOL_VERIFIED -lt ${#IN_POOL[@]} ]]; then
    echo "" >&2
    echo "BLOCKED: only $POOL_VERIFIED of ${#IN_POOL[@]} pool anchors verified; the gauntlet requires the full pool." >&2
    echo "Failed engines:" >&2
    printf "%b\n" "$ERRORS" >&2
    exit 1
fi

echo ""
echo "get-anchors.sh complete: $POOL_VERIFIED pool anchor(s) ready in $ANCHOR_DIR"
