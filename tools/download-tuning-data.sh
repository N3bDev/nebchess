#!/usr/bin/env bash
# zurichess quiet-labeled.epd: quiet positions with game results (MIT).
# (nominally "725k" upstream; the mirrored file contains 648,351 lines)
# Mirror: github.com/KierenP/ChessTrainingSets (spec §6.3).
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p data
if [ ! -s data/quiet-labeled.epd ]; then
  curl -sSfL -o data/quiet-labeled.epd \
    "https://raw.githubusercontent.com/KierenP/ChessTrainingSets/master/quiet-labeled.epd"
fi
wc -l data/quiet-labeled.epd
