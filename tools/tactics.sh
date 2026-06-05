#!/usr/bin/env bash
# Tactical regression runner (informational metric; see docs/tactics-log.md).
# Usage: tactics.sh [movetime_ms=1000]
set -euo pipefail
cd "$(dirname "$0")/.."
MT="${1:-1000}"
cargo build --release 2>/dev/null
./target/release/solve tools/suites/wac.epd "$MT" | tail -3
