#!/usr/bin/env python3
"""Random-walk perft cross-validation vs Stockfish (spec section 10.1).

Walks random legal games; at every position compares our divide(depth=2)
move-by-move against stockfish `go perft 2`. First mismatch prints a repro
command and exits 1.

Usage: tools/perft_compare.py [games=25] [max_plies=40]
Requires: stockfish on PATH, `cargo build --release` already run.
"""
import random
import subprocess
import sys

NEB = "./target/release/perft"
START = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"


def sf(cmds: str) -> str:
    return subprocess.run(
        ["stockfish"], input=cmds, capture_output=True, text=True, timeout=60
    ).stdout


def sf_perft(fen: str, depth: int) -> dict[str, int]:
    out = sf(f"position fen {fen}\ngo perft {depth}\n")
    moves = {}
    for line in out.splitlines():
        parts = line.split(": ")
        if len(parts) == 2 and 4 <= len(parts[0]) <= 5 and parts[1].strip().isdigit():
            if parts[0] != "Nodes searched":
                moves[parts[0]] = int(parts[1])
    return moves


def sf_fen_after(fen: str, move: str) -> str:
    out = sf(f"position fen {fen} moves {move}\nd\n")
    for line in out.splitlines():
        if line.startswith("Fen: "):
            return line[5:].strip()
    raise RuntimeError("stockfish gave no Fen line")


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
    positions = 0
    for g in range(games):
        fen = START
        for _ in range(max_plies):
            ours, theirs = neb_perft(fen, 2), sf_perft(fen, 2)
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
            fen = sf_fen_after(fen, rng.choice(sorted(theirs)))
        print(f"game {g + 1}/{games} ok ({positions} positions so far)")
    print(f"PASS: {positions} positions cross-validated")
    return 0


if __name__ == "__main__":
    sys.exit(main())
