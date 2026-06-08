#!/usr/bin/env bash
# UCI torture battery (Plan 7 T6.2): feed the engine malformed / hostile UCI and
# assert it never PANICS and never HANGS. This is an operational robustness gate,
# not an elo gate — Lichess is a messy adversary (truncated FENs, illegal moves
# mid-list, zero/negative clocks, stop storms, mid-search GUI disconnects).
#
# Each case is piped to a FRESH engine process under a `timeout` wall so a hang
# is bounded and observable. A case PASSES iff:
#   - the process exits within the timeout (no HANG), AND
#   - it exits cleanly (exit 0 / quit) or via normal EOF teardown — never via a
#     panic/abort/segfault (exit 101/134/139/etc.), AND
#   - stderr contains no `panicked` line (a worker panic that stop_and_join
#     recovers from still exits 0 with a fallback move — robust by design, but
#     a panic on these inputs is a defect the operator must see, so we fail it).
#
# Exit 0 iff every case passes; exit 1 (with a per-case table) on any failure.
#
# Usage: tools/uci-torture.sh [path-to-binary]   (default ../target/release/nebchess)
set -uo pipefail
cd "$(dirname "$0")"

ENGINE="$(realpath "${1:-../target/release/nebchess}")"
if [ ! -x "$ENGINE" ]; then
  echo "engine not found / not executable: $ENGINE" >&2
  echo "build it first: cargo build --release" >&2
  exit 2
fi

# Per-case wall. Generous: every legitimate case here completes in well under a
# second; only a genuine hang reaches the limit. 124 = timeout fired (= HANG).
CASE_TIMEOUT=10

PASS=0
FAIL=0
declare -a ROWS=()

# run_case <name> <input-as-printf-string>
# The input is fed verbatim (printf interpretation) on stdin. Cases that want to
# simulate a GUI disconnect mid-search simply omit a trailing `stop`/`quit`: the
# pipe closes (EOF) once the input is consumed, which must trigger a clean
# stop-and-join teardown rather than a hang.
run_case() {
  local name="$1" input="$2"
  local err out status verdict
  err="$(mktemp)"
  # Capture stderr; discard stdout (we only care about liveness, not moves).
  printf '%b' "$input" | timeout "$CASE_TIMEOUT" "$ENGINE" >/dev/null 2>"$err"
  status=$?
  out="$(cat "$err")"
  rm -f "$err"

  if [ "$status" -eq 124 ] || [ "$status" -eq 137 ]; then
    verdict="FAIL (HANG: timeout ${CASE_TIMEOUT}s)"
  elif printf '%s' "$out" | grep -qi "panicked"; then
    verdict="FAIL (PANIC: $(printf '%s' "$out" | grep -i panicked | head -1 | cut -c1-60))"
  elif [ "$status" -ne 0 ]; then
    # 101 = Rust panic/abort, 134 = SIGABRT, 139 = SIGSEGV, etc.
    verdict="FAIL (CRASH: exit $status)"
  else
    verdict="PASS"
  fi

  if [ "${verdict:0:4}" = "PASS" ]; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
  fi
  ROWS+=("$(printf '%-34s %s' "$name" "$verdict")")
}

# ---------------------------------------------------------------------------
# Position parsing: empty / truncated / illegal.
# ---------------------------------------------------------------------------
run_case "position-no-args"            'position\nisready\n'
run_case "startpos-no-moves-then-go"   'position startpos\ngo depth 1\n'
# en-prise-king class: a legal-looking FEN whose side-to-move can be mated by
# capturing the bare enemy king. The search must score it, not deref a None king.
run_case "fen-enprise-king-then-go"    'position fen 4k3/8/8/8/8/8/8/4K2R w K - 0 1\ngo depth 6\n'
# Garbage / truncated FEN fields (too few ranks, junk side-to-move).
run_case "fen-truncated"               'position fen 4k3/8/8 x\ngo depth 4\n'
run_case "fen-empty"                   'position fen\ngo depth 2\n'
# Illegal move in the middle of an otherwise-legal move list.
run_case "moves-illegal-midlist"       'position startpos moves e2e4 zzzz g1f3\ngo depth 5\n'

# ---------------------------------------------------------------------------
# go: no position, zero / degenerate limits.
# ---------------------------------------------------------------------------
# `go` with no prior `position`: the engine defaults to startpos at construction.
run_case "go-no-position-set"          'go depth 3\n'
run_case "go-wtime0-btime0"            'position startpos\ngo wtime 0 btime 0\n'
run_case "go-movetime0"                'position startpos\ngo movetime 0\n'
run_case "go-depth0"                   'position startpos\ngo depth 0\n'
run_case "go-nodes0"                   'position startpos\ngo nodes 0\n'

# ---------------------------------------------------------------------------
# stop / lifecycle races.
# ---------------------------------------------------------------------------
run_case "stop-no-search"              'stop\nisready\n'
# Stop-storm: 50 rapid go/stop cycles — exercises the spawn/join discipline and
# the stop-flag-clear-before-spawn race in cmd_go.
STORM='position startpos\n'
for _ in $(seq 1 50); do STORM+='go depth 20\nstop\n'; done
run_case "stop-storm-50x"              "$STORM"
run_case "go-infinite-then-stop"       'position startpos\ngo infinite\nstop\n'
# ucinewgame arriving mid-search must abort the running search first.
run_case "ucinewgame-mid-search"       'position startpos\ngo infinite\nucinewgame\nstop\n'
# EOF mid-search (GUI disconnect): no trailing stop — the pipe just closes.
run_case "eof-mid-infinite-search"     'position startpos\ngo infinite\n'

# ---------------------------------------------------------------------------
# setoption: unknown / out-of-range / bad paths.
# ---------------------------------------------------------------------------
run_case "setoption-unknown"           'setoption name Frobnicate value 7\nisready\n'
run_case "setoption-hash-negative"     'setoption name Hash value -5\nisready\n'
run_case "setoption-syzygy-missing"    'setoption name SyzygyPath value /nonexistent\nisready\n'
run_case "setoption-book-missing"      'setoption name BookFile value /nonexistent/book.bin\nisready\n'

# ---------------------------------------------------------------------------
# Report.
# ---------------------------------------------------------------------------
echo "=== UCI torture battery ($((PASS + FAIL)) cases, timeout ${CASE_TIMEOUT}s each) ==="
echo "engine: $ENGINE"
echo "----------------------------------------------------------------"
for row in "${ROWS[@]}"; do echo "$row"; done
echo "----------------------------------------------------------------"
echo "PASS=$PASS  FAIL=$FAIL"
if [ "$FAIL" -ne 0 ]; then
  echo "TORTURE FAILED: $FAIL case(s) panicked or hung."
  exit 1
fi
echo "torture ok: all cases survived (no panic, no hang)."
