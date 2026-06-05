#!/usr/bin/env bash
# Deferred M0 gate: a real 2-engine match runs end-to-end (10 games self-play).
set -euo pipefail
cd "$(dirname "$0")"
ENGINE="$(realpath ../target/release/nebchess)"
bin/fastchess \
  -engine cmd="$ENGINE" name=neb-a -engine cmd="$ENGINE" name=neb-b \
  -each tc=8+0.08 option.Hash=16 option.Threads=1 \
  -openings file=books/8moves_v3.pgn format=pgn order=random \
  -rounds 5 -repeat -concurrency 5 -recover \
  -pgnout file=smoke.pgn 2>&1 | tee smoke.log
echo "--- failure scan ---"
if grep -Ei "disconnect|illegal|loses on time|timeout|stall|crash" smoke.log; then
  echo "SMOKE FAILURE DETECTED"
  exit 1
fi
echo "smoke ok: 10 games completed cleanly"
