#!/usr/bin/env bash
# CI gate (spec §10.2): when HEAD's commit message carries "Bench: N",
# the built binary must reproduce exactly N. Commits without a Bench line
# (docs, tools) are skipped.
set -euo pipefail
cd "$(dirname "$0")/.."
expected=$(git log -1 --pretty=%B | grep -oP '^Bench: \K[0-9]+' | head -1 || true)
if [ -z "$expected" ]; then
  echo "no Bench: line in HEAD commit message; skipping"
  exit 0
fi
actual=$(./target/release/nebchess bench | grep -oP '^Bench: \K[0-9]+')
if [ "$expected" != "$actual" ]; then
  echo "BENCH MISMATCH: commit says $expected, binary produces $actual"
  exit 1
fi
echo "bench ok: $actual"
