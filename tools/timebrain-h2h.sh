#!/usr/bin/env bash
# Off-self-play TimeBrain head-to-head: NEW-TM vs OLD-TM with IDENTICAL search/eval,
# so the ONLY difference is the TimeManager. This isolates the spending-profile
# difference that same-TM self-play (the 8+0.08 strength SPRT) structurally cannot
# see (both sides share one TimeManager and starve in lockstep).
#
# Usage: tools/timebrain-h2h.sh <new-binary> <v1-binary> [tc] [elo1]
#   tc defaults to 8+0.08 — the fast arbiter. The 5x->2x hard-cap leak is
#   TC-PROPORTIONAL (one move can eat 1/3 of the clock at any TC), so a faster TC
#   still detects it but runs ~22x more games/hour than 180+2.
#   Real-blitz confirmation (slower): ROUNDS=200 tools/timebrain-h2h.sh new v1 180+2
#
# DELIBERATE deviation from the frozen strength SPRT (sprt.sh): NO draw
# adjudication. The field losses happen AFTER move 40 in long endgames; the
# protocol's `-draw movenumber=40 ... score=10` would adjudicate those equal
# positions as draws BEFORE the time-pressure loss manifests, masking the very
# effect we are measuring. Resign (clearly lost) is kept; everything else plays
# to the finish so clock survival decides the result.
set -euo pipefail
cd "$(dirname "$0")"
NEW="$(realpath "$1")"; OLD="$(realpath "$2")"; TC="${3:-8+0.08}"; ELO1="${4:-10}"
CONCURRENCY=$(( $(nproc) - 1 ))
bin/fastchess \
  -engine cmd="$NEW" name=tb-new -engine cmd="$OLD" name=tb-v1 \
  -each tc="$TC" option.Hash=16 option.Threads=1 \
  -openings file=books/8moves_v3.pgn format=pgn order=random \
  -repeat -rounds "${ROUNDS:-30000}" -recover \
  -resign movecount=3 score=600 \
  -concurrency "$CONCURRENCY" -report penta=true -ratinginterval 50 \
  -sprt elo0=0 elo1="$ELO1" alpha=0.05 beta=0.05 model=normalized
