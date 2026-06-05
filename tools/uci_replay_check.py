#!/usr/bin/env python3
"""UCI replay equivalence vs Stockfish (spec section 7 gate).

Random games: after every ply, send 'position startpos moves ...' to OUR
engine and compare its `fen` against Stockfish's `d` Fen.

EP-field note: both engines canonicalize the FEN ep square, but with
slightly different rules (SF: fully-legal-capture-aware; ours:
capturer-existence). EP correctness is perft-proven; the ep FIELD is
normalized to '-' on both sides before comparison. Everything else
(placement, stm, castling, halfmove, fullmove) compares exactly.

Usage: tools/uci_replay_check.py [games=20] [max_plies=60]
"""
import random
import shutil
import subprocess
import sys

STOCKFISH = shutil.which("stockfish") or "./tools/bin/stockfish"
NEB = "./target/release/nebchess"


class Pipe:
    def __init__(self, cmd: str) -> None:
        self.p = subprocess.Popen(
            [cmd], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1
        )

    def send(self, s: str) -> None:
        assert self.p.stdin is not None
        self.p.stdin.write(s + "\n")

    def read_until(self, pred):
        assert self.p.stdout is not None
        for line in self.p.stdout:
            line = line.strip()
            if pred(line):
                return line
        raise RuntimeError("engine died")


def norm_ep(fen: str) -> str:
    f = fen.split()
    f[3] = "-"
    return " ".join(f)


def main() -> int:
    games = int(sys.argv[1]) if len(sys.argv) > 1 else 20
    max_plies = int(sys.argv[2]) if len(sys.argv) > 2 else 60
    rng = random.Random(0x4E45)
    sf = Pipe(STOCKFISH)
    sf.send("isready")
    sf.read_until(lambda l: l == "readyok")
    neb = Pipe(NEB)
    checked = 0
    for g in range(games):
        moves: list[str] = []
        for _ in range(max_plies):
            pos_cmd = (
                f"position startpos moves {' '.join(moves)}" if moves else "position startpos"
            )
            sf.send(pos_cmd)
            sf.send("d")
            sfen = sf.read_until(lambda l: l.startswith("Fen: "))[5:]
            sf.read_until(lambda l: l.startswith("Checkers"))
            sf.send("isready")
            sf.read_until(lambda l: l == "readyok")

            neb.send(pos_cmd)
            neb.send("fen")
            nfen = neb.read_until(lambda l: l.count(" ") == 5)
            checked += 1
            if norm_ep(nfen) != norm_ep(sfen):
                print(f"MISMATCH after: {' '.join(moves)}")
                print(f"  neb: {nfen}")
                print(f"  sf : {sfen}")
                return 1

            sf.send(pos_cmd)
            sf.send("go perft 1")
            legal = []
            while True:
                line = sf.read_until(lambda l: True)
                if line.startswith("Nodes searched"):
                    break
                parts = line.split(": ")
                if len(parts) == 2 and 4 <= len(parts[0]) <= 5 and parts[1].isdigit():
                    legal.append(parts[0])
            if not legal:
                break  # mate/stalemate
            moves.append(rng.choice(sorted(legal)))
        print(f"game {g + 1}/{games} ok ({checked} positions)", flush=True)
    print(f"PASS: {checked} replay positions matched")
    return 0


if __name__ == "__main__":
    sys.exit(main())
