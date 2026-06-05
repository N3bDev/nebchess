#!/usr/bin/env bash
# Snapshot the current release binary as an SPRT baseline.
# Usage: tools/baseline.sh <name>   ->  tools/bin/baseline-<name>
set -euo pipefail
cd "$(dirname "$0")"
[ $# -eq 1 ] || { echo "usage: baseline.sh <name>" >&2; exit 2; }
cp ../target/release/nebchess "bin/baseline-$1"
echo "saved baseline-$1: $(bin/baseline-$1 bench 2>/dev/null | tail -1)"
