#!/usr/bin/env bash
# Snapshot the current release build as the live Lichess engine.
# A deliberate copy step: rebuilds of target/release (e.g. a net swap mid-
# milestone) must never yank the binary out from under a live game. Redeploying
# a new version = run this again while the bot is stopped.
set -euo pipefail
cd "$(dirname "$0")"

SRC="../../target/release/nebchess"
[ -x "$SRC" ] || { echo "ERROR: $SRC not built (cargo build --release)" >&2; exit 1; }

if pgrep -f "lichess-bot.py" >/dev/null 2>&1; then
    echo "ERROR: lichess-bot is running — stop it before redeploying" >&2
    exit 1
fi

cp "$SRC" nebchess-live
chmod +x nebchess-live
echo "deployed nebchess-live: $(printf 'uci\nquit\n' | ./nebchess-live | grep 'id name')"
echo "bench: $(./nebchess-live bench 2>/dev/null | tail -1)"
