#!/usr/bin/env bash
# Builds fastchess (the SPRT match runner; what Stockfish's Fishtest uses)
# into tools/bin/fastchess. Idempotent.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p bin
if [ -x bin/fastchess ]; then
  echo "fastchess already built: $(bin/fastchess --version | head -1)"
  exit 0
fi
rm -rf fastchess-src
git clone --depth 1 https://github.com/Disservin/fastchess.git fastchess-src
make -C fastchess-src -j"$(nproc)"
cp fastchess-src/fastchess bin/
rm -rf fastchess-src
bin/fastchess --version
