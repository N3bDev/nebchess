#!/usr/bin/env bash
# Zero-time-forfeit acceptance gauntlet (spec §5.4, §12 M2 gate).
# A single time loss is a BLOCKER bug, not noise. Usage: forfeit-gauntlet.sh [rounds=100]
set -euo pipefail
cd "$(dirname "$0")"
ROUNDS="${1:-100}"
ENGINE="$(realpath ../target/release/nebchess)"
rm -f gauntlet.pgn
bin/fastchess \
  -engine cmd="$ENGINE" name=new -engine cmd="$ENGINE" name=old \
  -each tc=8+0.08 option.Hash=16 option.Threads=1 \
  -openings file=books/8moves_v3.pgn format=pgn order=random \
  -rounds "$ROUNDS" -repeat -concurrency "$(( $(nproc) - 1 ))" -recover \
  -draw movenumber=40 movecount=8 score=10 \
  -pgnout file=gauntlet.pgn 2>&1 | tee gauntlet.log
echo "--- forfeit scan (console) ---"
if grep -Ei "loses on time|timeout|disconnect|illegal move|crash" gauntlet.log; then
  echo "FORFEIT/FAILURE DETECTED"
  exit 1
fi
echo "--- forfeit scan (pgn terminations) ---"
forfeits=$(grep -ci "time forfeit" gauntlet.pgn || true)
echo "time forfeits in pgn: $forfeits"
[ "$forfeits" = "0" ] && echo "gauntlet ok: $((ROUNDS * 2)) games, zero forfeits"
