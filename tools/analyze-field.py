#!/usr/bin/env python3
"""Field-corpus analysis driver (NebChess Plan 7, Task 1).

Replays the lichess corpus through the engine library (via the
`pgn_replay` example, which emits a flat TSV of every replayed position) and
drives `./target/release/nebchess` over a *single persistent UCI pipe* to
evaluate NebChess-to-move positions. Outputs the data behind
`docs/field-analysis-050.md` and `tools/suites/sac-entrance.epd`.

Stdlib only. The persistent-pipe rule is project law: the engine is spawned
ONCE and reused for every query; never spawn-per-position.

Usage:
  tools/analyze-field.py draws    # step 1.1/1.2: replay+eval the 14 draws
  tools/analyze-field.py leaks    # step 1.3: leak-moment EPD + 10s recheck (reads draws cache)
  tools/analyze-field.py sacs     # step 1.4: sacrifice-entrance scan over losses+draws

All three reuse a JSON cache under tools/data/field-050/ so reruns are cheap.
"""

import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PGN = os.path.join(ROOT, "db", "lichess_nebchessbot_0.5.0.pgn")
ENGINE = os.path.join(ROOT, "target", "release", "nebchess")
CACHE_DIR = os.path.join(ROOT, "tools", "data", "field-050")

# ---------------------------------------------------------------------------
# Replay (delegates SAN resolution + terminal detection to the Rust library)
# ---------------------------------------------------------------------------


def run_replay():
    """Build & run the pgn_replay example; return parsed records."""
    subprocess.run(
        ["cargo", "build", "--release", "--example", "pgn_replay"],
        cwd=ROOT,
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    out = subprocess.run(
        [os.path.join(ROOT, "target", "release", "examples", "pgn_replay"), PGN],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    games = {}  # idx -> dict(meta + positions + term)
    for line in out.stdout.splitlines():
        f = line.split("\t")
        if f[0] == "GAME":
            idx = int(f[1])
            games[idx] = {
                "idx": idx,
                "neb_color": f[2],
                "result": f[3],
                "opponent": f[4],
                "opp_elo": f[5],
                "tc": f[6],
                "site": f[7],
                "plies": int(f[8]),
                "positions": [],
                "term": None,
            }
        elif f[0] == "POS":
            idx = int(f[1])
            games[idx]["positions"].append(
                {
                    "ply": int(f[2]),
                    "fullmove": int(f[3]),
                    "neb_to_move": f[4] == "1",
                    "played_uci": f[5],
                    "played_san": f[6],
                    "fen": f[7],
                    "halfmove": int(f[8]),
                    "piececount": int(f[9]),
                }
            )
        elif f[0] == "TERM":
            idx = int(f[1])
            games[idx]["term"] = {
                "final_fen": f[2],
                "rep": f[3] == "1",
                "fifty": f[4] == "1",
                "insuff": f[5] == "1",
                "result": f[6],
            }
    return games


# ---------------------------------------------------------------------------
# Persistent UCI engine (spawned ONCE)
# ---------------------------------------------------------------------------


class Engine:
    def __init__(self, path):
        self.p = subprocess.Popen(
            [path],
            cwd=ROOT,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        self._send("uci")
        self._wait_for("uciok")
        self._send("isready")
        self._wait_for("readyok")

    def _send(self, cmd):
        self.p.stdin.write(cmd + "\n")
        self.p.stdin.flush()

    def _wait_for(self, token):
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("engine EOF waiting for " + token)
            if line.strip() == token:
                return

    def newgame(self):
        self._send("ucinewgame")
        self._send("isready")
        self._wait_for("readyok")

    def eval_fen(self, fen, movetime_ms):
        """Search a FEN; return (best_uci, score_cp, mate, depth).

        score_cp is side-to-move-relative (negamax root convention).
        `mate` is the signed mate distance in moves (None if not a mate).
        """
        self._send("position fen " + fen)
        self._send("go movetime %d" % movetime_ms)
        last_cp = None
        last_mate = None
        last_depth = None
        best = None
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("engine EOF during search")
            line = line.strip()
            if line.startswith("info depth") and " score " in line:
                tok = line.split()
                try:
                    di = tok.index("depth")
                    last_depth = int(tok[di + 1])
                    si = tok.index("score")
                    if tok[si + 1] == "cp":
                        last_cp = int(tok[si + 2])
                        last_mate = None
                    elif tok[si + 1] == "mate":
                        last_mate = int(tok[si + 2])
                        last_cp = None
                except (ValueError, IndexError):
                    pass
            elif line.startswith("bestmove"):
                best = line.split()[1]
                break
        return best, last_cp, last_mate, last_depth

    def quit(self):
        try:
            self._send("quit")
            self.p.wait(timeout=5)
        except Exception:
            self.p.kill()


# Convert (cp, mate) to a single NebChess-relative cp scalar for comparison.
# Positions queried are always NebChess-to-move, so the root score is already
# NebChess-relative. Mate scores map to a large signed magnitude.
MATE_CP = 100000


def to_scalar(cp, mate):
    if mate is not None:
        return MATE_CP - abs(mate) if mate > 0 else -(MATE_CP - abs(mate))
    return cp if cp is not None else 0


def fmt_eval(cp, mate):
    if mate is not None:
        return ("#%d" % mate) if mate > 0 else ("#-%d" % abs(mate))
    return "%+d" % (cp if cp is not None else 0)


# ---------------------------------------------------------------------------
# Step 1.1 / 1.2 — draws: replay, eval every NebChess-to-move pos from move 20,
# track max eval held in the final 30 plies.
# ---------------------------------------------------------------------------

DRAW_RESULT = "1/2-1/2"
MOVETIME = 2000
FINAL_PLIES = 30


def cmd_draws():
    os.makedirs(CACHE_DIR, exist_ok=True)
    games = run_replay()
    draws = [g for g in games.values() if g["result"] == DRAW_RESULT]
    draws.sort(key=lambda g: g["idx"])
    print("Draw games: %d" % len(draws), file=sys.stderr)

    eng = Engine(ENGINE)
    out = []
    try:
        for g in draws:
            eng.newgame()
            total_plies = g["plies"]
            final_start = total_plies - FINAL_PLIES  # ply index where final 30 begin
            evals = []  # (ply, fullmove, scalar, cp, mate, best, played)
            for pos in g["positions"]:
                if not pos["neb_to_move"]:
                    continue
                if pos["fullmove"] < 20:
                    continue
                best, cp, mate, depth = eng.eval_fen(pos["fen"], MOVETIME)
                scalar = to_scalar(cp, mate)
                evals.append(
                    {
                        "ply": pos["ply"],
                        "fullmove": pos["fullmove"],
                        "scalar": scalar,
                        "cp": cp,
                        "mate": mate,
                        "depth": depth,
                        "best": best,
                        "played": pos["played_uci"],
                        "fen": pos["fen"],
                    }
                )
            # max eval held in the final 30 plies (NebChess-to-move positions only)
            final_evals = [e for e in evals if e["ply"] >= final_start]
            max_final = max((e["scalar"] for e in final_evals), default=None)
            max_overall = max((e["scalar"] for e in evals), default=None)
            g_out = {
                "idx": g["idx"],
                "neb_color": g["neb_color"],
                "opponent": g["opponent"],
                "opp_elo": g["opp_elo"],
                "tc": g["tc"],
                "site": g["site"],
                "plies": total_plies,
                "term": g["term"],
                "max_final_30": max_final,
                "max_overall_from20": max_overall,
                "evals": evals,
            }
            out.append(g_out)
            print(
                "g%-3d %s vs %-18s maxfinal30=%s maxall=%s n=%d"
                % (
                    g["idx"],
                    g["neb_color"],
                    g["opponent"],
                    _scal_str(max_final),
                    _scal_str(max_overall),
                    len(evals),
                ),
                file=sys.stderr,
            )
    finally:
        eng.quit()

    with open(os.path.join(CACHE_DIR, "draws.json"), "w") as fh:
        json.dump(out, fh, indent=1)
    print("wrote draws.json (%d games)" % len(out), file=sys.stderr)


def _scal_str(s):
    if s is None:
        return "n/a"
    if abs(s) >= MATE_CP - 1000:
        return "#%d" % (MATE_CP - abs(s)) if s > 0 else "#-%d" % (MATE_CP - abs(s))
    return "%+d" % s


# ---------------------------------------------------------------------------
# Step 1.3 — leak moments for LEAKED-perpetual (repetition) games + 10s recheck
# ---------------------------------------------------------------------------

LEAK_DROP = 150  # eval first drops >=150cp toward the draw
RECHECK_MOVETIME = 10000


def cmd_leaks(held_threshold_cp=200, leaked_ids=None):
    with open(os.path.join(CACHE_DIR, "draws.json")) as fh:
        draws = json.load(fh)

    # LEAKED = held >= +200cp within final 30; perpetual = repetition terminal.
    perp = []
    for g in draws:
        if leaked_ids is not None and g["idx"] not in leaked_ids:
            continue
        if g["term"]["rep"] and (g["max_final_30"] or 0) >= held_threshold_cp:
            perp.append(g)
    print("LEAKED-perpetual games: %s" % [g["idx"] for g in perp], file=sys.stderr)

    # Leak moment = the peak NebChess-to-move position right BEFORE the eval
    # first drops >=150cp toward the draw. That peak position is where the
    # winning continuation still existed (engine-preferred vs played diverge
    # there), so its FEN is the leak-moment EPD per step 1.3.
    results = []
    for g in perp:
        evals = sorted(g["evals"], key=lambda e: e["ply"])
        peak = None          # running high-water scalar
        peak_pos = None      # the eval entry at that high-water mark
        leak = None
        for e in evals:
            s = e["scalar"]
            if peak is None or s > peak:
                peak = s
                peak_pos = e
            if peak is not None and (peak - s) >= LEAK_DROP and leak is None:
                # record the PEAK position (the leak moment) + the drop seen
                leak = {
                    "peak": peak,
                    "drop": peak - s,
                    "drop_at_move": e["fullmove"],
                    "drop_to": s,
                    **peak_pos,  # fen/best/played/ply/fullmove of the peak
                }
        results.append({"idx": g["idx"], "opponent": g["opponent"], "leak": leak, "peak": peak})

    # 10s recheck on the worst 3 (largest held peak -> most blown win).
    perp_sorted = sorted(results, key=lambda r: -(r["peak"] or 0))
    worst3 = [r for r in perp_sorted if r["leak"] is not None][:3]
    eng = Engine(ENGINE)
    try:
        for r in worst3:
            lk = r["leak"]
            eng.newgame()
            best, cp, mate, depth = eng.eval_fen(lk["fen"], RECHECK_MOVETIME)
            r["recheck"] = {
                "best": best,
                "cp": cp,
                "mate": mate,
                "depth": depth,
                "scalar": to_scalar(cp, mate),
            }
    finally:
        eng.quit()

    with open(os.path.join(CACHE_DIR, "leaks.json"), "w") as fh:
        json.dump({"all": results, "worst3": worst3}, fh, indent=1)
    for r in results:
        lk = r["leak"]
        if lk is None:
            print("g%-3d peak=%s NO >=150 drop found" % (r["idx"], _scal_str(r["peak"])), file=sys.stderr)
        else:
            rc = r.get("recheck")
            rcs = (" recheck10s=%s best=%s d=%s" % (fmt_eval(rc["cp"], rc["mate"]), rc["best"], rc["depth"])) if rc else ""
            print(
                "g%-3d leak@move%d ply%d peak=%s drop=%d 2s_best=%s played=%s%s"
                % (r["idx"], lk["fullmove"], lk["ply"], _scal_str(r["peak"]), lk["drop"], lk["best"], lk["played"], rcs),
                file=sys.stderr,
            )


# ---------------------------------------------------------------------------
# Step 1.4 — sacrifice-entrance scan: from losses + draws, positions where a
# 10s search finds a sacrifice that the game move missed.
# ---------------------------------------------------------------------------

# Sacrifice judgment uses the pgn_replay `annotate` mode (real make/unmake on
# the board), NOT a Python board reimplementation. A move is a "sacrifice" when
# its 2-ply material swing (move + opponent's least-valuable recapture) is
# clearly negative for the mover — it gives up material to enter. Captures that
# win material and quiet developing moves are excluded.
SAC_SCREEN_MS = 1500   # cheap screening pass over every position
SAC_CONFIRM_MS = 10000  # spec's 10s confirmation on screened candidates
SAC_SWING_THRESH = -90  # 2-ply swing must be <= this (mover gives up ~a pawn+)
SAC_MIN_EVAL = 30       # confirmed engine eval must still be >= this (entrance pays)
SAC_MOVE_LO = 8         # entrances are middlegame; scan moves 8..50
SAC_MOVE_HI = 50


def annotate_moves(fen, ucis):
    """Call pgn_replay `annotate`; return {uci: (is_cap, gives_check, swing2)}."""
    out = subprocess.run(
        [
            os.path.join(ROOT, "target", "release", "examples", "pgn_replay"),
            "annotate",
            fen,
            *ucis,
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    res = {}
    for line in out.stdout.splitlines():
        f = line.split("\t")
        if f[0] == "ANNOT":
            res[f[1]] = (int(f[2]), int(f[3]), int(f[4]))
    return res


def cmd_sacs():
    os.makedirs(CACHE_DIR, exist_ok=True)
    subprocess.run(
        ["cargo", "build", "--release", "--example", "pgn_replay"],
        cwd=ROOT, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    games = run_replay()
    # Spec scope: from LOSSES and DRAWS. NebChess-to-move positions, middlegame.
    losses_draws = [g for g in games.values() if g["result"] in ("1/2-1/2", "0-1")]
    losses_draws.sort(key=lambda g: g["idx"])

    # --- screening pass (cheap): find positions where the engine's preferred
    # move differs from the game move AND is a material-giving sacrifice. ---
    eng = Engine(ENGINE)
    screened = []
    try:
        for g in losses_draws:
            eng.newgame()
            for pos in g["positions"]:
                if not pos["neb_to_move"]:
                    continue
                if not (SAC_MOVE_LO <= pos["fullmove"] <= SAC_MOVE_HI):
                    continue
                fen = pos["fen"]
                best, cp, mate, depth = eng.eval_fen(fen, SAC_SCREEN_MS)
                if best is None or best == pos["played_uci"]:
                    continue
                ann = annotate_moves(fen, [best, pos["played_uci"]])
                b_cap, b_chk, b_swing = ann.get(best, (0, 0, 0))
                p_ann = ann.get(pos["played_uci"], (0, 0, 0))
                # engine move must be a sacrifice (negative 2-ply swing); the
                # played move must NOT be the same sacrifice (it "missed" it).
                if b_swing > SAC_SWING_THRESH:
                    continue
                if to_scalar(cp, mate) < SAC_MIN_EVAL:
                    continue
                screened.append(
                    {
                        "idx": g["idx"],
                        "opponent": g["opponent"],
                        "neb_color": g["neb_color"],
                        "fullmove": pos["fullmove"],
                        "ply": pos["ply"],
                        "fen": fen,
                        "screen_best": best,
                        "screen_cp": cp,
                        "screen_mate": mate,
                        "screen_swing": b_swing,
                        "screen_gives_check": b_chk,
                        "screen_is_capture": b_cap,
                        "played": pos["played_uci"],
                        "played_san": pos["played_san"],
                        "played_swing": p_ann[2],
                    }
                )
    finally:
        eng.quit()
    print("screened sacrifice candidates: %d" % len(screened), file=sys.stderr)

    # --- confirmation pass (10s) on the screened set: does the sacrifice hold
    # at depth, and is it still the engine's choice? ---
    eng = Engine(ENGINE)
    confirmed = []
    try:
        for c in screened:
            eng.newgame()
            best, cp, mate, depth = eng.eval_fen(c["fen"], SAC_CONFIRM_MS)
            ann = annotate_moves(c["fen"], [best])
            b_cap, b_chk, b_swing = ann.get(best, (0, 0, 0))
            c["confirm_best"] = best
            c["confirm_cp"] = cp
            c["confirm_mate"] = mate
            c["confirm_depth"] = depth
            c["confirm_swing"] = b_swing
            c["confirm_gives_check"] = b_chk
            c["confirm_is_capture"] = b_cap
            c["confirm_scalar"] = to_scalar(cp, mate)
            # keep only if the 10s move is STILL a sacrifice that the game move
            # didn't play, and the eval is favorable.
            if (
                best != c["played"]
                and b_swing <= SAC_SWING_THRESH
                and to_scalar(cp, mate) >= SAC_MIN_EVAL
            ):
                confirmed.append(c)
    finally:
        eng.quit()

    with open(os.path.join(CACHE_DIR, "sacs.json"), "w") as fh:
        json.dump({"screened": screened, "confirmed": confirmed}, fh, indent=1)
    print("CONFIRMED sacrifice entrances (10s): %d" % len(confirmed), file=sys.stderr)
    for c in confirmed:
        print(
            "g%-3d move%d %s plays %s (swing%+d) | engine %s %s d%s swing%+d %s"
            % (
                c["idx"], c["fullmove"], c["opponent"], c["played"], c["played_swing"],
                c["confirm_best"], fmt_eval(c["confirm_cp"], c["confirm_mate"]),
                c["confirm_depth"], c["confirm_swing"],
                "CHECK" if c["confirm_gives_check"] else ("CAP" if c["confirm_is_capture"] else "QUIET"),
            ),
            file=sys.stderr,
        )


# ---------------------------------------------------------------------------

def main():
    if len(sys.argv) < 2 or sys.argv[1] not in ("draws", "leaks", "sacs"):
        print(__doc__)
        sys.exit(2)
    if not os.path.exists(ENGINE):
        print("engine not built: " + ENGINE, file=sys.stderr)
        sys.exit(1)
    cmd = sys.argv[1]
    if cmd == "draws":
        cmd_draws()
    elif cmd == "leaks":
        cmd_leaks()
    elif cmd == "sacs":
        cmd_sacs()


if __name__ == "__main__":
    main()
