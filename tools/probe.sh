#!/usr/bin/env bash
# Fixed-games probe — the CHEAP filter for candidate evaluation experiments
# (plan-5 step 6.5). Same engine protocol as the frozen SPRT (tc/hash/book/
# adjudication identical) but a fixed game count and NO sequential test:
# this produces an elo ESTIMATE for ranking candidates, never a gate verdict.
# Only a full tools/sprt.sh run can admit a change to the baseline.
#   tools/probe.sh <candidate-binary> <reference-binary> [games=400]
set -euo pipefail
cd "$(dirname "$0")"
NEW="$(realpath "$1")"; OLD="$(realpath "$2")"; GAMES="${3:-400}"
CONCURRENCY=$(( $(nproc) - 1 ))
bin/fastchess \
  -engine cmd="$NEW" name=cand -engine cmd="$OLD" name=ref \
  -each tc=8+0.08 option.Hash=16 option.Threads=1 \
  -openings file=books/8moves_v3.pgn format=pgn order=random \
  -repeat -rounds $(( GAMES / 2 )) -recover \
  -resign movecount=3 score=600 -draw movenumber=40 movecount=8 score=10 \
  -concurrency "$CONCURRENCY" -report penta=true -ratinginterval 50
