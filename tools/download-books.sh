#!/usr/bin/env bash
# Downloads the frozen test opening books (spec §10.3) into tools/books/.
# 8moves_v3.pgn  - balanced, used while the engine is weak (M2+)
# UHO_Lichess_4852_v1.epd - unbalanced, used once the engine is strong
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p books && cd books
BASE="https://github.com/official-stockfish/books/raw/master"
for f in 8moves_v3.pgn.zip UHO_Lichess_4852_v1.epd.zip; do
  if [ ! -f "${f%.zip}" ]; then
    curl -sSfLO "$BASE/$f"
    unzip -o "$f" && rm "$f"
  fi
done
ls -l
