#!/usr/bin/env python3
"""Random-walk perft cross-validation vs Stockfish (spec section 10.1).

Walks random legal games; at every position compares our divide(depth=2)
move-by-move against stockfish `go perft 2`. First mismatch prints a repro
command and exits 1.

Usage: tools/perft_compare.py [games=25] [max_plies=40]
Requires: stockfish on PATH, `cargo build --release` already run.

Stockfish runs as ONE persistent process: SF18 loads its NNUE nets on
startup (~1s), so spawn-per-query turns a 2-minute run into a 40-minute one.
"""
import random
import shutil
import subprocess
import sys

NEB = "./target/release/perft"
# PATH first, then the project-local install (tools/bin is gitignored)
STOCKFISH = shutil.which("stockfish") or "./tools/bin/stockfish"
START = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"


class Stockfish:
    """Minimal persistent UCI pipe."""

    def __init__(self) -> None:
        self.p = subprocess.Popen(
            [STOCKFISH],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        self._sync()

    def _send(self, cmd: str) -> None:
        assert self.p.stdin is not None
        self.p.stdin.write(cmd + "\n")

    def _sync(self) -> None:
        self._send("isready")
        assert self.p.stdout is not None
        for line in self.p.stdout:
            if line.strip() == "readyok":
                return
        raise RuntimeError("stockfish died")

    def perft(self, fen: str, depth: int) -> dict[str, int]:
        self._send(f"position fen {fen}")
        self._send(f"go perft {depth}")
        moves: dict[str, int] = {}
        assert self.p.stdout is not None
        for line in self.p.stdout:
            line = line.strip()
            if line.startswith("Nodes searched"):
                break
            parts = line.split(": ")
            if len(parts) == 2 and 4 <= len(parts[0]) <= 5 and parts[1].isdigit():
                moves[parts[0]] = int(parts[1])
        return moves

    def fen_after(self, fen: str, move: str) -> str:
        self._send(f"position fen {fen} moves {move}")
        self._send("d")
        fen_out = None
        assert self.p.stdout is not None
        for line in self.p.stdout:
            line = line.strip()
            if line.startswith("Fen: "):
                fen_out = line[5:]
            if line.startswith("Checkers"):  # last line of `d` output
                break
        self._sync()
        if fen_out is None:
            raise RuntimeError("stockfish gave no Fen line")
        return fen_out


def neb_perft(fen: str, depth: int) -> dict[str, int]:
    out = subprocess.run(
        [NEB, fen, str(depth)], capture_output=True, text=True, timeout=60
    ).stdout
    moves = {}
    for line in out.splitlines():
        k, v = line.split(": ")
        if k != "total":
            moves[k] = int(v)
    return moves


def main() -> int:
    games = int(sys.argv[1]) if len(sys.argv) > 1 else 25
    max_plies = int(sys.argv[2]) if len(sys.argv) > 2 else 40
    rng = random.Random(0x4E45)  # fixed seed, reproducible
    sf = Stockfish()
    positions = 0
    for g in range(games):
        fen = START
        for _ in range(max_plies):
            ours, theirs = neb_perft(fen, 2), sf.perft(fen, 2)
            positions += 1
            if ours != theirs:
                print(f"MISMATCH at: {fen}")
                for k in sorted(set(ours) | set(theirs)):
                    a, b = ours.get(k), theirs.get(k)
                    if a != b:
                        print(f"  {k}: ours={a} stockfish={b}")
                print(f'repro: {NEB} "{fen}" 2')
                return 1
            if not theirs:
                break  # mate/stalemate
            fen = sf.fen_after(fen, rng.choice(sorted(theirs)))
        print(f"game {g + 1}/{games} ok ({positions} positions so far)", flush=True)
    print(f"PASS: {positions} positions cross-validated")
    return 0


if __name__ == "__main__":
    sys.exit(main())
