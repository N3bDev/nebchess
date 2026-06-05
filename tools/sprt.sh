#!/usr/bin/env bash
# Frozen SPRT protocol (spec §10.3). Usage:
#   tools/sprt.sh <new-binary> <old-binary> [elo1]
# elo1 defaults to 10 (early project); tighten to 5 once gains shrink.
# PROTOCOL CHANGES INVALIDATE CROSS-VERSION COMPARISONS - bump this header if changed.
# Protocol v1: STC 8+0.08, hash 16, threads 1, 8moves_v3 book, reversed pairs,
#   resign 3x600cp, draw mv40 8x10cp, SPRT alpha=beta=0.05 model=normalized.
#   (flag syntax fixed for fastchess 1.8.1: option.Threads in -each, no -randomseed;
#    protocol parameters unchanged)
set -euo pipefail
cd "$(dirname "$0")"
NEW="$(realpath "$1")"; OLD="$(realpath "$2")"; ELO1="${3:-10}"
CONCURRENCY=$(( $(nproc) - 1 ))
bin/fastchess \
  -engine cmd="$NEW" name=new -engine cmd="$OLD" name=old \
  -each tc=8+0.08 option.Hash=16 option.Threads=1 \
  -openings file=books/8moves_v3.pgn format=pgn order=random \
  -repeat -rounds 30000 -recover \
  -resign movecount=3 score=600 -draw movenumber=40 movecount=8 score=10 \
  -concurrency "$CONCURRENCY" -report penta=true -ratinginterval 50 \
  -sprt elo0=0 elo1="$ELO1" alpha=0.05 beta=0.05 model=normalized
