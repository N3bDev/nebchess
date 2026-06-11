#!/usr/bin/env bash
# Run the NebChess Lichess bot: lichess-bot (plays) + matchmaker.py (paces).
# Token comes from the repo-root .env (LICHESS_KEY=...), exported as
# LICHESS_BOT_TOKEN. Ctrl-C stops both processes AND all their children.
set -euo pipefail
cd "$(dirname "$0")"

VENV_PY_PATTERN="tools/lichess/venv/bin/python"
STAMP=".last-start"

# Kill every bot-owned python (the main processes AND lichess-bot's
# multiprocessing children, whose cmdline is just "python -c from
# multiprocessing..." — plain parent-kills miss them; orphaned stream-watcher
# children then hammer /api/stream/event forever and pin the account in
# Lichess's rate-limit penalty box. Incident: 2026-06-11, 26 orphans from 5
# dead instances kept the limit alive for hours).
sweep_bot_processes() {
    local pid
    for pid in $(pgrep -f "$VENV_PY_PATTERN" 2>/dev/null); do
        case "$(ps -o comm= -p "$pid" 2>/dev/null)" in
            python*) kill -9 "$pid" 2>/dev/null || true ;;
        esac
    done
    pkill -x nebchess-live 2>/dev/null || true
}

# --- restart guard: rapid restart cycles burn stream-opens and escalate the
# --- rate-limit penalty. Refuse to start within 5 minutes of the last start.
if [ -f "$STAMP" ]; then
    last=$(cat "$STAMP" 2>/dev/null || echo 0)
    [[ "$last" =~ ^[0-9]+$ ]] || last=0
    now=$(date +%s)
    age=$(( now - last ))
    if [ "$age" -lt 300 ]; then
        echo "ERROR: last start was ${age}s ago (<300s). Rapid restarts escalate" >&2
        echo "Lichess's rate-limit penalty. Wait $(( 300 - age ))s, or rm $STAMP to override." >&2
        exit 1
    fi
fi

# --- secrets (never echo) ---
[ -f ../../.env ] || { echo "ERROR: .env not found at repo root" >&2; exit 1; }
set -a; source ../../.env; set +a
export LICHESS_BOT_TOKEN="${LICHESS_BOT_TOKEN:-${LICHESS_KEY:?LICHESS_KEY missing from .env}}"

[ -x nebchess-live ] || { echo "ERROR: no nebchess-live — run ./deploy.sh first" >&2; exit 1; }
[ -d venv ] || { echo "ERROR: no venv — python3 -m venv venv && venv/bin/pip install -r lichess-bot/requirements.txt" >&2; exit 1; }

# --- pre-flight: no ghosts may precede us (idempotent hygiene) ---
sweep_bot_processes
date +%s > "$STAMP"

PY="$(pwd)/venv/bin/python"

# Baseline the (append-mode) log so the connect gate only reads THIS session's
# lines — a previous session that died right after connecting leaves a stale
# "now connected" inside any fixed tail window (live false positive, reviewed).
baseline=0
[ -f lichess-bot.log ] && baseline=$(wc -l < lichess-bot.log)

( cd lichess-bot && exec "$PY" lichess-bot.py --config ../config.yml --logfile ../lichess-bot.log ) &
BOT=$!

# Gate the matchmaker on the client actually connecting: a rate-limited or
# dead client must never have a challenger booking games it cannot hear
# (incident: 2026-06-11, a deaf client lost a game on time). Watch the log
# for the connect line; give up and tear down after 90s.
connected=""
for _ in $(seq 1 45); do
    sleep 2
    if ! kill -0 "$BOT" 2>/dev/null; then break; fi
    if tail -n +"$(( baseline + 1 ))" lichess-bot.log 2>/dev/null | grep -q "now connected"; then
        connected=yes
        break
    fi
done
if [ -z "$connected" ]; then
    echo "ERROR: client did not connect within 90s (rate-limited?). Tearing down." >&2
    kill "$BOT" 2>/dev/null || true
    sweep_bot_processes
    exit 1
fi

"$PY" matchmaker.py >> matchmaker.log 2>&1 &
MM=$!

trap 'echo "stopping..."; kill $MM $BOT 2>/dev/null || true; sweep_bot_processes; wait; exit 0' INT TERM
echo "lichess-bot pid $BOT (log: lichess-bot.log), matchmaker pid $MM (log: matchmaker.log)"
echo "Ctrl-C stops both (and sweeps all children)."
# If either process dies, tear down the other (a matchmaker challenging with
# no player produces no-show aborts; a player with no matchmaker idles).
# `|| true`: under set -e, a child's non-zero exit would otherwise abort the
# script HERE and orphan the survivor — the exact case this guards against.
wait -n || true
echo "a bot process exited — stopping the other (and sweeping children)"
kill $MM $BOT 2>/dev/null || true
sweep_bot_processes
wait
