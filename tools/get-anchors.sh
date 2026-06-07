#!/usr/bin/env bash
# Download and verify the anchored-gauntlet engine pool.
# Engines are fixed public versions with known CCRL Blitz ratings.
# PROTOCOL: Do not change engine versions without bumping this header and
# re-running an anchored gauntlet — anchor ratings are version-specific.
#
# M6 MIXED POOL (the six rungs the gauntlet consumes via ratings.txt) — a
# >=3-family pool replacing the M5 Stash-only ladder (style-monoculture fix):
#   Stash19         2471   Stash    https://gitlab.com/mhouppin/stash-bot  (tag v19.2, bin v19.1)
#   Stash20         2508   Stash    https://gitlab.com/mhouppin/stash-bot  (tag v20.0.1)
#   Stash21         2713   Stash    https://gitlab.com/mhouppin/stash-bot  (tag v21.2)  -- longitudinal spine
#   Stash25         2934   Stash    https://gitlab.com/mhouppin/stash-bot  (tag v25.0)  -- same-family ceiling
#   Weiss10         2898   Weiss    https://github.com/TerjeKir/weiss      (tag v1.0, SOURCE-BUILT)
#   Koivisto20      2907   Koivisto https://github.com/Luecx/Koivisto      (tag v2.0, SOURCE-BUILT)
# Pool shape: 6 rungs, 3 families (Stash/Weiss/Koivisto), 3 rungs >=2800
# (Stash25 2934, Koivisto 2907, Weiss 2898). The Stash v19/v20/v21 spine stays
# so deltas vs v0.5.0 remain interpretable; Stash25 brackets from above.
#
# RATINGS ARE CCRL BLITZ (40/2) POINTS (1-CPU list — matches the gauntlet's
# Threads=1), list COMPUTED 2025-12-20. Source: the
# live list at computerchess.org.uk/ccrl/404/ is bot-protected (403/curl); the
# 40/2 complete-list snapshot was read from web.archive.org (label "Back to CCRL
# 40/2", "Computed on December 20, 2025 with Bayeselo"). Spine pins refreshed
# from this list (were 2473/2509/2714 in M5; CCRL re-tuning is normal).
# NOTE: the plan's guessed Stash v22~=2790 / v23~=2880 are UNVERIFIED and WRONG
# as Blitz pins -- CCRL never tested Stash 22/23/24 on the Blitz list (it jumps
# 21.0=2713 -> 25.0=2934). Stash 25.0 (2934, the next CCRL-rated same-family
# rung) is used as the Stash ceiling instead. Re-pull before any public claim.
#
# FETCHED BUT NOT IN THE M6 POOL (archival: downloaded/built + UCI-verified for
# optional use, but kept OUT of ratings.txt so the gauntlet never plays them):
#   RusticAlpha2  ~1815   https://codeberg.org/mvanthoor/rustic (alpha-3.0.0 CCRL zip)
#   Stash13       ~1962   https://gitlab.com/mhouppin/stash-bot (tag v13)
#   Stash15        2168   https://gitlab.com/mhouppin/stash-bot (tag v15) -- blowout rung, archival per M6
#   Stash17        2294   https://gitlab.com/mhouppin/stash-bot (tag v17.0) -- blowout rung, archival per M6
set -uo pipefail
cd "$(dirname "$0")"

ANCHOR_DIR="bin/anchors"
mkdir -p "$ANCHOR_DIR"
# Absolute form: source builds `cd` into a /tmp build tree, so their `-o`
# output path must be absolute (a relative bin/anchors would resolve under /tmp).
ANCHOR_ABS="$(cd "$ANCHOR_DIR" && pwd)"

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
# 2-8. Stash versions: v13, v15, v17, v19, v20, v21, v25
#    Assets live at:  https://gitlab.com/mhouppin/stash-bot/uploads/<hash>/<file>
#    (upload links discovered via the GitLab releases API release descriptions).
#    Tags: v13="v13", v15="v15", v17="v17.0", v19="v19.2" (binary v19.1),
#          v20="v20.0.1", v21="v21.2", v25="v25.0". We pin the MAJOR version
#          CCRL tested and use the latest minor of that major.
#    M6 STASH POOL = v19, v20, v21 (spine) + v25 (ceiling), written to ratings.txt.
#    Stash13/15/17 are ARCHIVAL only (blowout rungs at current strength; fetched
#    + verified, NOT in the pool). v15/v17 left their M5 pool slot per M6 design.
# ---------------------------------------------------------------------------
declare -A STASH_ENTRIES
# format: "label=upload_path rating"
# Using x86_64 (generic, not bmi2/modern) for widest compatibility.
# Ratings: CCRL Blitz (40/2), list computed 2025-12-20 (see header note).
STASH_ENTRIES["Stash13"]="4ff97bc58d4b3801d525bf723e0574e7/stash-13.0-linux-x86_64 1962"
STASH_ENTRIES["Stash15"]="56cba735a1572e7b665b5571d0abb486/stash-15.0-linux-x86_64 2168"
STASH_ENTRIES["Stash17"]="058f6a6706656223502f0222d861471c/stash-17.0-linux-x86_64 2294"
STASH_ENTRIES["Stash19"]="5c37aa29f0c5e25ab7e935013d5cfb8d/stash-19.1-linux-x86_64 2471"
STASH_ENTRIES["Stash20"]="bb23ef7457a5e9e18a87078008b6ee97/stash-20.0.1-linux-x86_64 2508"
STASH_ENTRIES["Stash21"]="4881a30b90418fab74b5c745826c94af/stash-21.2-linux-x86_64 2713"
STASH_ENTRIES["Stash25"]="3ba23a4c6069e234aef12babbef2cb57/stash-25.0-linux-x86_64 2934"

# Pool membership: only these enter ratings.txt. The M6 Stash rungs are the
# v19/v20/v21 spine + the v25 ceiling; v13/v15/v17 stay archival (fetch logic
# kept, but they are not pool anchors). The non-Stash families (Weiss, Koivisto)
# are added to IN_POOL further below, after their own source-build steps.
declare -A IN_POOL=( [Stash19]=1 [Stash20]=1 [Stash21]=1 [Stash25]=1 )

GITLAB_UPLOAD_BASE="https://gitlab.com/mhouppin/stash-bot/uploads"

for ENTRY_NAME in Stash13 Stash15 Stash17 Stash19 Stash20 Stash21 Stash25; do
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

# Register the non-Stash pool members now so the full-pool guard counts them.
IN_POOL["Weiss10"]=1
IN_POOL["Koivisto20"]=1

# ---------------------------------------------------------------------------
# Helper: record a verified pool engine into ratings.txt (or note archival).
# Mirrors the Stash-loop accounting so VERIFIED / POOL_VERIFIED stay correct.
# ---------------------------------------------------------------------------
record_pool_engine() {
    local name="$1" rating="$2" dest="$3"
    VERIFIED=$(( VERIFIED + 1 ))
    if [[ -n "${IN_POOL[$name]:-}" ]]; then
        echo "OK  $name -> $dest"
        echo "$name $rating" >> "$RATINGS_FILE"
        POOL_VERIFIED=$(( POOL_VERIFIED + 1 ))
    else
        echo "OK  $name -> $dest  (archival; omitted from ratings.txt)"
    fi
}

# ---------------------------------------------------------------------------
# 9. Weiss 1.0  (2898 CCRL Blitz 40/2, 2025-12-20) — Weiss family, SOURCE-BUILT.
#    Source: github.com/TerjeKir/weiss tag v1.0. The v1.0 release ships only a
#    Windows binary collection, so we build from source. Weiss 1.0 predates
#    Weiss's NNUE era: pure-C hand-crafted eval (evaluate.c/psqt.c), NO network
#    file, NO runtime download (the optional online-DB probe in noobprobe is
#    OFF unless a UCI option turns it on). Clean `gcc` build, deps: -pthread -lm.
#    Build is idempotent: skip if the binary already exists. We compile a
#    portable static binary (-O3 -flto -static, generic x86-64 + popcnt; NO
#    -march=native) for deterministic, machine-independent strength.
# ---------------------------------------------------------------------------
NAME="Weiss10"
RATING="2898"
DEST="$ANCHOR_ABS/weiss-1.0-linux-x86_64"  # absolute: build subshell cd's to /tmp
WEISS_SRC_URL="https://github.com/TerjeKir/weiss/archive/refs/tags/v1.0.tar.gz"
WEISS_BUILD="/tmp/nebchess-weiss-build-$$"

echo "Building $NAME (Weiss 1.0, from source) ..."
if [[ -x "$DEST" ]]; then
    echo "  (binary already present, skipping build)"
    if verify_uci "$DEST"; then
        record_pool_engine "$NAME" "$RATING" "$DEST"
    else
        echo "SKIP [$NAME]: existing binary failed UCI handshake" >&2
        ERRORS="$ERRORS\n  $NAME: existing binary failed UCI handshake"
        rm -f "$DEST"
    fi
else
    rm -rf "$WEISS_BUILD"
    mkdir -p "$WEISS_BUILD"
    wcode=$(curl -fsSL -o "$WEISS_BUILD/weiss.tar.gz" -w "%{http_code}" "$WEISS_SRC_URL" 2>/dev/null || echo "000")
    if [[ "$wcode" == "200" ]] && tar xzf "$WEISS_BUILD/weiss.tar.gz" -C "$WEISS_BUILD" 2>/dev/null; then
        WSRC=$(find "$WEISS_BUILD" -maxdepth 3 -type f -name makefile -printf '%h\n' | head -1)
        if [[ -n "$WSRC" ]] && ( cd "$WSRC" && gcc -std=gnu11 -O3 -flto -static -msse3 -mpopcnt \
                *.c fathom/tbprobe.c noobprobe/noobprobe.c -pthread -lm -o "$DEST" ) 2>/dev/null; then
            chmod +x "$DEST"
            if verify_uci "$DEST"; then
                record_pool_engine "$NAME" "$RATING" "$DEST"
            else
                echo "SKIP [$NAME]: UCI handshake failed (built but no uciok)" >&2
                ERRORS="$ERRORS\n  $NAME: UCI handshake failed after build"
                rm -f "$DEST"
            fi
        else
            echo "SKIP [$NAME]: source build failed (gcc/make)" >&2
            ERRORS="$ERRORS\n  $NAME: source build failed"
        fi
    else
        echo "SKIP [$NAME]: source download failed (HTTP $wcode) URL: $WEISS_SRC_URL" >&2
        ERRORS="$ERRORS\n  $NAME: source download failed (HTTP $wcode)"
    fi
    rm -rf "$WEISS_BUILD"
fi

# ---------------------------------------------------------------------------
# 10. Koivisto 2.0  (2907 CCRL Blitz 40/2, 2025-12-20) — Koivisto family,
#    SOURCE-BUILT. Source: github.com/Luecx/Koivisto tag v2.0. The v2.0 release
#    ships no Linux binary (Koivisto published binaries only from v3.0 on), so
#    we build from source. Koivisto 2.0 predates Koivisto's NNUE era (NNUE came
#    at v5.0): its eval is the "real-men-evaluation" + a tiny compiled-in eval
#    unit (eun/data/Weight.cpp, ~1KB) — NO external network file, NO runtime
#    download. The repo carries a CMakeLists, but it merely enumerates the same
#    sources its makefile globs; we invoke g++ directly (no cmake dependency) to
#    match the makefile `release` target. The 2020 C++ omits some transitive
#    includes that GCC 13 now requires, so we force-include <cstring>/<cstdint>
#    via flags (NO source edits). Portable static build, no -march=native.
#    Idempotent: skip if the binary already exists.
# ---------------------------------------------------------------------------
NAME="Koivisto20"
RATING="2907"
DEST="$ANCHOR_ABS/koivisto-2.0-linux-x86_64"  # absolute: build subshell cd's to /tmp
KOI_SRC_URL="https://github.com/Luecx/Koivisto/archive/refs/tags/v2.0.tar.gz"
KOI_BUILD="/tmp/nebchess-koivisto-build-$$"

echo "Building $NAME (Koivisto 2.0, from source) ..."
if [[ -x "$DEST" ]]; then
    echo "  (binary already present, skipping build)"
    if verify_uci "$DEST"; then
        record_pool_engine "$NAME" "$RATING" "$DEST"
    else
        echo "SKIP [$NAME]: existing binary failed UCI handshake" >&2
        ERRORS="$ERRORS\n  $NAME: existing binary failed UCI handshake"
        rm -f "$DEST"
    fi
else
    rm -rf "$KOI_BUILD"
    mkdir -p "$KOI_BUILD"
    kcode=$(curl -fsSL -o "$KOI_BUILD/koi.tar.gz" -w "%{http_code}" "$KOI_SRC_URL" 2>/dev/null || echo "000")
    if [[ "$kcode" == "200" ]] && tar xzf "$KOI_BUILD/koi.tar.gz" -C "$KOI_BUILD" 2>/dev/null; then
        KSRC=$(find "$KOI_BUILD" -maxdepth 2 -type d -name src_files | head -1)
        if [[ -n "$KSRC" ]] && ( cd "$KSRC" && g++ -O3 -std=c++17 -DNDEBUG -flto -static \
                -msse3 -mpopcnt -DUSE_POPCNT -include cstring -include cstdint \
                ./*.cpp eun/*.cpp eun/data/*.cpp syzygy/tbprobe.c -lpthread -lm -o "$DEST" ) 2>/dev/null; then
            chmod +x "$DEST"
            if verify_uci "$DEST"; then
                record_pool_engine "$NAME" "$RATING" "$DEST"
            else
                echo "SKIP [$NAME]: UCI handshake failed (built but no uciok)" >&2
                ERRORS="$ERRORS\n  $NAME: UCI handshake failed after build"
                rm -f "$DEST"
            fi
        else
            echo "SKIP [$NAME]: source build failed (g++)" >&2
            ERRORS="$ERRORS\n  $NAME: source build failed"
        fi
    else
        echo "SKIP [$NAME]: source download failed (HTTP $kcode) URL: $KOI_SRC_URL" >&2
        ERRORS="$ERRORS\n  $NAME: source download failed (HTTP $kcode)"
    fi
    rm -rf "$KOI_BUILD"
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo "=== M6 gauntlet pool ($POOL_VERIFIED anchors in ratings.txt) ==="
if [[ -s "$RATINGS_FILE" ]]; then
    while IFS=' ' read -r eng rat; do
        echo "  $eng  $rat"
    done < "$RATINGS_FILE"
fi
echo "(total binaries verified incl. archival: $VERIFIED)"

# The gauntlet needs the FULL pool; archival binaries don't count toward the
# guard. A partial pool is blocked outright: silently dropping a rung —
# especially Stash25, the 2934 ceiling that brackets the estimate from above,
# or any of the cross-family rungs — shifts the Ordo anchor math and any
# absolute-rating claim bracketed by it (review finding on b04d2ee). This guard
# now counts the full M6 mixed pool (4 Stash + Weiss + Koivisto = 6 anchors).
if [[ $POOL_VERIFIED -lt ${#IN_POOL[@]} ]]; then
    echo "" >&2
    echo "BLOCKED: only $POOL_VERIFIED of ${#IN_POOL[@]} pool anchors verified; the gauntlet requires the full pool." >&2
    echo "Failed engines:" >&2
    printf "%b\n" "$ERRORS" >&2
    exit 1
fi

echo ""
echo "get-anchors.sh complete: $POOL_VERIFIED pool anchor(s) ready in $ANCHOR_DIR"
