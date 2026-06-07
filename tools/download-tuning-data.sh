#!/usr/bin/env bash
# Texel tuning corpora.
#
# 1) zurichess quiet-labeled.epd: quiet positions with game results (MIT).
#    (nominally "725k" upstream; the mirrored file contains 648,351 lines)
#    Mirror: github.com/KierenP/ChessTrainingSets (spec §6.3).
#    Line format: `<placement> <stm> <castling> <ep> c9 "1-0";` (no counters).
#
# 2) lichess-big3-resolved (M5 T6 upgrade attempt, ~7.15M resolved positions):
#    a 94.8MB .7z from archive.org expanding to ~419MB `.book`.
#    Line format: `<full fen> [<0.0|0.5|1.0>]` (bracketed white-relative score).
#    Extraction needs a 7z reader; we try py7zr (pure-python) in a throwaway
#    venv so it never touches the system Python. If the download or extraction
#    fails (no network, PEP-668 lockout, no python venv, ...) this is NON-FATAL:
#    we REPORT and leave the zurichess set as the working corpus (the tuner
#    falls back to it with epochs=600 — a legitimate, recorded outcome).
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p data

# --- 1) zurichess quiet-labeled (required) -----------------------------------
if [ ! -s data/quiet-labeled.epd ]; then
  curl -sSfL -o data/quiet-labeled.epd \
    "https://raw.githubusercontent.com/KierenP/ChessTrainingSets/master/quiet-labeled.epd"
fi
wc -l data/quiet-labeled.epd

# --- 2) lichess-big3-resolved (best-effort upgrade) --------------------------
BIG3_7Z="data/lichess-big3-resolved.7z"
BIG3_BOOK="data/lichess-big3-resolved.book"
BIG3_URL="https://archive.org/download/lichess-big3-resolved.7z/lichess-big3-resolved.7z"

big3_ok() { [ -s "$BIG3_BOOK" ]; }

if big3_ok; then
  echo "big3: already extracted ($(wc -l < "$BIG3_BOOK") lines)"
else
  # Download (non-fatal).
  if [ ! -s "$BIG3_7Z" ]; then
    if ! curl -sSfL --max-time 600 -o "$BIG3_7Z" "$BIG3_URL"; then
      echo "big3: download FAILED (network?) — falling back to quiet-labeled." >&2
      rm -f "$BIG3_7Z"
    fi
  fi
  # Extract via py7zr in a disposable venv (non-fatal).
  if [ -s "$BIG3_7Z" ]; then
    VENV="$(mktemp -d)/py7zr-venv"
    if python3 -m venv "$VENV" \
      && "$VENV/bin/pip" install --quiet py7zr \
      && "$VENV/bin/python" -c "import py7zr; py7zr.SevenZipFile('$BIG3_7Z','r').extractall('data/')"; then
      echo "big3: extracted ($(wc -l < "$BIG3_BOOK") lines)"
    else
      echo "big3: py7zr extraction FAILED — falling back to quiet-labeled." >&2
    fi
    rm -rf "$VENV"
  fi
fi
