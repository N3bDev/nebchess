#!/usr/bin/env bash
# Book SPRT — measures an opening book vs no-book. Frozen protocol v1 EXCEPT:
#   - openings truncated to 4 plies (plies=4): gives cross-game variety while
#     leaving plies 5..BookDepth for the book to actually act on. (Our book's
#     pick is deterministic per position, so a 0-ply/startpos-only set would
#     replay one identical game — useless. A full 8-move set would exhaust the
#     book before it's consulted. 4 plies threads the needle.)
#   - BookFile/BookDepth set on the NEW engine ONLY; OLD plays book-off.
# DEVIATION from sprt.sh is intentional and logged in the ledger row.
#   tools/sprt-book.sh <new-binary> <old-binary> <book.bin> [elo1=5]
set -euo pipefail
cd "$(dirname "$0")"
NEW="$(realpath "$1")"; OLD="$(realpath "$2")"; BOOK="$(realpath "$3")"; ELO1="${4:-5}"
CONCURRENCY=$(( $(nproc) - 1 ))
bin/fastchess \
  -engine cmd="$NEW" name=new option.BookFile="$BOOK" option.BookDepth=16 \
  -engine cmd="$OLD" name=old \
  -each tc=8+0.08 option.Hash=16 option.Threads=1 \
  -openings file=books/8moves_v3.pgn format=pgn order=random plies=4 \
  -repeat -rounds 30000 -recover \
  -resign movecount=3 score=600 -draw movenumber=40 movecount=8 score=10 \
  -concurrency "$CONCURRENCY" -report penta=true -ratinginterval 50 \
  -sprt elo0=0 elo1="$ELO1" alpha=0.05 beta=0.05 model=normalized
