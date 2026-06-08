#!/usr/bin/env bash
# KR-vs-K conversion under a low clock — the field g35 leak rematch (a forced
# win NebChess drew on the 50-move counter while blitzing). Every game from the
# KRK start MUST be decisive; a draw = clock-collapse conversion bug reproduced.
set -uo pipefail
cd "$(dirname "$0")"
ENGINE="$(realpath "${1:-../target/release/nebchess}")"
GAMES="${2:-20}"
bin/fastchess \
  -engine cmd="$ENGINE" name=A -engine cmd="$ENGINE" name=B \
  -each tc=5+0.1 option.Hash=16 option.Threads=1 \
  -openings file=/tmp/krk.epd format=epd order=random \
  -rounds $(( GAMES / 2 )) -games 2 -repeat -concurrency 4 \
  -pgnout file=/tmp/krk-games.pgn 2>&1 | tail -6
echo "--- draws (must be 0):"
grep -c "1/2-1/2" /tmp/krk-games.pgn 2>/dev/null || echo 0
