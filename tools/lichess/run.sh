#!/usr/bin/env bash
# Run the NebChess Lichess bot: lichess-bot (plays) + matchmaker.py (paces).
# Token comes from the repo-root .env (LICHESS_KEY=...), exported as
# LICHESS_BOT_TOKEN. Ctrl-C stops both processes.
set -euo pipefail
cd "$(dirname "$0")"

# --- secrets (never echo) ---
[ -f ../../.env ] || { echo "ERROR: .env not found at repo root" >&2; exit 1; }
set -a; source ../../.env; set +a
export LICHESS_BOT_TOKEN="${LICHESS_BOT_TOKEN:-${LICHESS_KEY:?LICHESS_KEY missing from .env}}"

[ -x nebchess-live ] || { echo "ERROR: no nebchess-live — run ./deploy.sh first" >&2; exit 1; }
[ -d venv ] || { echo "ERROR: no venv — python3 -m venv venv && venv/bin/pip install -r lichess-bot/requirements.txt" >&2; exit 1; }

PY="$(pwd)/venv/bin/python"

( cd lichess-bot && exec "$PY" lichess-bot.py --config ../config.yml --logfile ../lichess-bot.log ) &
BOT=$!
sleep 5  # let the client connect before the matchmaker starts issuing challenges

"$PY" matchmaker.py >> matchmaker.log 2>&1 &
MM=$!

trap 'echo "stopping..."; kill $MM $BOT 2>/dev/null; wait; exit 0' INT TERM
echo "lichess-bot pid $BOT (log: lichess-bot.log), matchmaker pid $MM (log: matchmaker.log)"
echo "Ctrl-C stops both."
# If either process dies, tear down the other (a matchmaker challenging with
# no player produces no-show aborts; a player with no matchmaker idles).
# `|| true`: under set -e, a child's non-zero exit would otherwise abort the
# script HERE and orphan the survivor — the exact case this guards against.
wait -n || true
echo "a bot process exited — stopping the other"
kill $MM $BOT 2>/dev/null
wait
