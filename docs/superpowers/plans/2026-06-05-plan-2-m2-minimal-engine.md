# NebChess Plan 2 (M2): Minimal Playing Engine — Search, Eval, UCI, Time Management

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A complete, weak-but-correct UCI chess engine: iterative-deepening alpha-beta with quiescence, material+PST evaluation behind the NNUE-ready seam, full UCI protocol with robust time management — playing real games with zero time forfeits (expected ~1500-1700 CCRL).

**Architecture:** Search state lives in a `SearchThread` that owns a cloned `Position` (spec §5.1 — only shared items are the stop flag); evaluation hides behind the four-hook `Evaluator` trait (spec §6.1) so NNUE can slot in later; the UCI loop runs on the main thread with search on a worker thread signaled via `AtomicBool`. Repetition/50-move plumbing lands in `Position` (spec §3).

**Tech Stack:** Rust 1.96.0 (pinned), std-only (std::thread, std::sync::atomic — no dependencies). fastchess + Stockfish 18 (both in `tools/bin/`) for the gates.

**Spec:** `docs/superpowers/specs/2026-06-04-nebchess-engine-design.md` §3 (repetition), §5.1-5.2 (search), §5.4 (time), §6 (eval seam), §7 (UCI), §10.2 (bench), §12 M2 row. Read the relevant section before each task.

**Plan 1 carry-ins (final review):** (a) lift the UCI move resolver out of `src/bin/perft.rs` into the lib; (b) key-history + repetition/50-move node-prologue plumbing; (c) the deferred M0 "2-engine smoke match" gate runs here, first thing after the engine binary exists.

**Executed-review amendments (code as landed differs from the blocks below in these ways):**
- T4: negamax counts only interior nodes (qsearch owns the horizon — fixes 2x node inflation); no-king guard returns -(MATE-ply) (see synced code blocks).
- T5/T6 (CRITICAL race fix): the stop-flag clear moved OUT of `iterate()` (worker thread) INTO `cmd_go` on the main thread before spawn — a worker-side clear races with an instant GUI `stop` and deadlocked `go infinite` (reproduced 40/40, fixed 200/200). `iterate`'s doc now states the caller owns flag hygiene.
- T6: `print_info` emits one buffered println (line-tearing window); `stop_and_join` prints a fallback bestmove if the worker panicked (GUI is owed one per go); `find_first_legal` lifted to `board::movegen` (shared by search + the fallback); the go-stop gate is zero-delay plus a 25-round `zero_delay_stop_never_hangs` watchdog test.

**Plan deviations from spec (recorded deliberately):**
- §5.2 lists SEE-based qsearch pruning and delta pruning among "core" — M2 ships qsearch with MVV-LVA ordering only; SEE and delta pruning are M3/M4 SPRT-gated features (per §12's per-feature methodology).
- §5.4's PV-instability/fail-low time extensions are deferred to M4 (individually SPRT-testable); M2 ships soft/hard limits + node-cadence polling + Move Overhead.
- §7 options: `Hash` and `Threads` are accepted-but-inert (TT is M3, SMP is M9b); `MultiPV` accepted and clamped to 1 (root loop is structured for it; multi-line output lands when first needed); `Contempt` deferred to M5 (eval maturity); `OwnBook`/`BookFile`/`SyzygyPath`/`Ponder` not yet advertised (M7/M8/M9a).
- §6.2's tapered eval is M5; M2 PST is single-phase (Michniewski "Simplified Evaluation Function" tables, the canonical starter).

**Environment facts:** 16-core WSL2; `tools/bin/stockfish` (SF18) and `tools/bin/fastchess` exist (NOT on PATH — scripts must use the path or the PATH-fallback pattern); books in `tools/books/`; no sudo. Prefix all cargo commands with `source "$HOME/.cargo/env" && `.

---

## File structure (end state of this plan)

```
src/
  lib.rs                  # + pub mod eval; pub mod search; pub mod uci;
  main.rs                 # NEW: nebchess binary — UCI loop; "bench" subcommand
  board/
    movegen.rs            # + find_uci_move (lifted from bin), generate_moves unchanged
    position.rs           # + key_history, is_repetition(), Clone, halfmove>=100 helpers
  eval/
    mod.rs                # Evaluator trait (the NNUE seam, spec §6.1) + re-exports
    psqt.rs               # Michniewski PST tables (single-phase)
    hce.rs                # Hce: material + PST, no-op hooks
  search/
    mod.rs                # SearchThread, negamax, qsearch, MovePicker, PV, mate scores
    limits.rs             # Limits (go params), TimeManager (soft/hard, overhead)
    bench.rs              # fixed-position bench (spec §10.2)
  uci/
    mod.rs                # protocol loop, options, position replay, go dispatch
  bin/perft.rs            # MODIFIED: uses board::movegen::find_uci_move
tests/
  search.rs               # mate-finding, draw, limit-respecting tests
  uci.rs                  # spec §7 edge-case gates (drives the real binary)
tools/
  check-bench.sh          # CI bench-vs-commit-message assertion
  forfeit-gauntlet.sh     # 200-game self-play, zero time losses gate
  uci_replay_check.py     # random-game position-replay diff vs Stockfish
.github/workflows/ci.yml  # + bench check step
```

Responsibilities: `search/` never parses UCI; `uci/` never computes chess; `eval/` is reached only through the trait. `Position` owns all repetition state (it owns the history the rules are defined over).

**Domain glossary for this plan:**
- *Negamax:* minimax where both sides maximize `score(stm)`; child score = `-negamax(-beta, -alpha)`. *Alpha-beta:* prune when a move proves ≥ beta (opponent won't allow this node).
- *Quiescence (qsearch):* at depth 0, keep searching captures only until the position is "quiet", so the eval is never taken mid-exchange. *Stand pat:* the option to stop capturing.
- *MVV-LVA:* order captures by Most Valuable Victim, then Least Valuable Attacker — QxP last, PxQ first.
- *Mate scores:* `MATE - ply` (we mate) / `-(MATE - ply)` (we're mated), so nearer mates score higher. Any |score| > MATE_BOUND is a mate score.
- *Soft/hard time limits:* soft = "don't start another iteration past this"; hard = "abort the search NOW" (checked every 2048 nodes).
- *PV (principal variation):* the engine's expected best line, collected via a triangular table.

---

### Task 1: Lift the UCI move resolver into the library

**Files:**
- Modify: `src/board/movegen.rs`
- Modify: `src/bin/perft.rs`

- [ ] **Step 1.1: Write failing tests** (append inside `movegen.rs` `mod tests`)

```rust
    #[test]
    fn find_uci_move_resolves_and_rejects() {
        let pos = Position::startpos();
        let mv = find_uci_move(&pos, "e2e4").expect("e2e4 exists");
        assert_eq!(mv.flag(), Move::DOUBLE_PUSH);
        assert!(find_uci_move(&pos, "e2e5").is_none(), "not a legal move shape");
        assert!(find_uci_move(&pos, "zzzz").is_none());
        // promotion suffix resolves to the right flag
        let pos = Position::from_fen("1n2k3/2P5/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let q = find_uci_move(&pos, "c7c8q").unwrap();
        assert!(q.is_promotion() && !q.is_capture());
        let n = find_uci_move(&pos, "c7b8n").unwrap();
        assert!(n.is_promotion() && n.is_capture());
        assert_eq!(n.promotion_piece_type(), crate::board::PieceType::Knight);
        // castle resolves to the castle flag, not a quiet king move
        let pos = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        assert_eq!(
            find_uci_move(&pos, "e1g1").unwrap().flag(),
            Move::KING_CASTLE
        );
    }
```

- [ ] **Step 1.2: Implement** (append to `movegen.rs` above the tests)

```rust
/// Resolves a UCI long-algebraic string ("e2e4", "e7e8q", castling as "e1g1")
/// against the pseudo-legal move list. Returns None for unknown strings.
/// NOTE: pseudo-legal resolution — the caller still validates via make().
pub fn find_uci_move(pos: &Position, uci: &str) -> Option<Move> {
    let mut list = MoveList::new();
    generate_moves(pos, &mut list);
    list.iter().copied().find(|mv| mv.to_string() == uci)
}
```

- [ ] **Step 1.3: Rewire the perft bin.** In `src/bin/perft.rs`, delete the local `apply_uci_move` fn and replace its call site:

```rust
use nebchess::board::movegen::find_uci_move;
use nebchess::board::Position;
```

(drop the now-unused `generate_moves`/`MoveList` imports), and in `main`:

```rust
    for uci in &args[2..] {
        match find_uci_move(&pos, uci) {
            Some(mv) if pos.make(mv) => {}
            Some(_) => {
                eprintln!("illegal move: {uci}");
                std::process::exit(2);
            }
            None => {
                eprintln!("unknown move: {uci}");
                std::process::exit(2);
            }
        }
    }
```

- [ ] **Step 1.4: Run tests + CLI sanity**

```bash
cargo test board::movegen && cargo build --release
./target/release/perft startpos 1 e2e4 e7e5 | tail -1
./target/release/perft startpos 2 e9x9; echo "exit: $?"
```

Expected: tests pass (8 in movegen), `total: 29`, then `unknown move: e9x9` + `exit: 2`.

- [ ] **Step 1.5: Lint + commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/board/movegen.rs src/bin/perft.rs
git commit -m "refactor(board): lift UCI move resolver into the library"
```

---

### Task 2: Repetition + 50-move plumbing in Position

**Files:**
- Modify: `src/board/position.rs`

Spec §3: key history spans the game prefix (UCI replay) AND the in-tree search path — both flow through `make()`, so the history lives in `Position`. `make` pushes the pre-move key; `unmake` pops. Scan backward stepping 2 (same side to move), capped by the halfmove clock (irreversible moves cut repetition scope). Also: `Position` becomes `Clone` (SearchThread owns a copy, spec §5.1).

- [ ] **Step 2.1: Write failing tests** (append inside `position.rs` `mod tests`)

```rust
    #[test]
    fn repetition_detected_via_history() {
        let mut pos = Position::startpos();
        assert!(!pos.is_repetition());
        // Ng1f3 Ng8f6 Nf3g1 Nf6g8 -> startpos repeated (one fold)
        for (f, t) in [("g1", "f3"), ("g8", "f6"), ("f3", "g1"), ("f6", "g8")] {
            assert!(pos.make(mv(&pos, f, t, Move::QUIET)));
        }
        assert!(pos.is_repetition(), "back at startpos: repetition");
        pos.unmake();
        assert!(!pos.is_repetition());
    }

    #[test]
    fn pawn_move_cuts_repetition_scope() {
        let mut pos = Position::startpos();
        assert!(pos.make(mv(&pos, "e2", "e4", Move::DOUBLE_PUSH)));
        assert!(pos.make(mv(&pos, "e7", "e5", Move::DOUBLE_PUSH)));
        // shuffle knights back to the post-e4e5 position
        for (f, t) in [("g1", "f3"), ("g8", "f6"), ("f3", "g1"), ("f6", "g8")] {
            assert!(pos.make(mv(&pos, f, t, Move::QUIET)));
        }
        assert!(pos.is_repetition(), "post-e4e5 position repeated");
        // but startpos itself is NOT reachable as a repetition (pawn moves reset)
        // halfmove clock is 4 here; history scan must not cross the e7e5 boundary
        assert_eq!(pos.halfmove(), 4);
    }

    #[test]
    fn history_survives_clone_and_unmake_restores_len() {
        let mut pos = Position::startpos();
        assert!(pos.make(mv(&pos, "g1", "f3", Move::QUIET)));
        let snapshot = pos.clone();
        assert_eq!(snapshot.key(), pos.key());
        assert!(pos.make(mv(&pos, "g8", "f6", Move::QUIET)));
        pos.unmake();
        assert_eq!(pos.key(), snapshot.key());
        assert!(!pos.is_repetition());
    }

    #[test]
    fn fifty_move_counter_draw_helper() {
        // artificial position with halfmove=99: one quiet move crosses 100
        let mut pos =
            Position::from_fen("4k3/8/8/8/8/8/8/R3K3 w Q - 99 80").unwrap();
        assert!(!pos.is_fifty_move_draw());
        assert!(pos.make(mv(&pos, "a1", "a2", Move::QUIET)));
        assert_eq!(pos.halfmove(), 100);
        assert!(pos.is_fifty_move_draw());
    }
```

- [ ] **Step 2.2: Implement.** In `position.rs`:

(a) Derive Clone on Position and Undo:

```rust
#[derive(Clone)]
pub(crate) struct Undo {
```

```rust
#[derive(Clone)]
pub struct Position {
```

(b) Add the field after `undo_stack`:

```rust
    pub(crate) key_history: Vec<u64>,
```

and initialize in `new_empty()` after the `undo_stack` line:

```rust
            key_history: Vec::with_capacity(256),
```

(c) In `make()`, immediately after the `self.undo_stack.push(Undo {...});` statement, add:

```rust
        self.key_history.push(self.key);
```

(d) In `unmake()`, as the FIRST line of the function body (before popping the undo stack):

```rust
        self.key_history.pop();
```

(e) Append accessors/predicates to the `impl Position` block:

```rust
    /// Has the current position occurred before within the reversible-move
    /// window? (Twofold; search treats this as a draw, spec §3.)
    pub fn is_repetition(&self) -> bool {
        let n = self.key_history.len();
        let lookback = (self.halfmove as usize).min(n);
        // same side to move only: ancestors at distance 2, 4, ...
        let mut d = 2;
        while d <= lookback {
            if self.key_history[n - d] == self.key {
                return true;
            }
            d += 2;
        }
        false
    }

    /// 50-move rule (100 halfmoves). Mate-precedence is the caller's job:
    /// a mated side at halfmove >= 100 is still mated (search checks moves).
    #[inline]
    pub fn is_fifty_move_draw(&self) -> bool {
        self.halfmove >= 100
    }
```

- [ ] **Step 2.3: Run the full suite** — repetition tests pass AND nothing else regressed (make/unmake round-trip tests + perft exercise the push/pop balance):

```bash
cargo test && cargo test --test perft
```

Expected: all green (the perft fast+edge tiers re-verify make/unmake with the new history bookkeeping at every node).

- [ ] **Step 2.4: Lint + commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/board/position.rs
git commit -m "feat(board): key history, repetition and 50-move detection, Clone"
```

---

### Task 3: Evaluation — the Evaluator trait (NNUE seam) + material/PST HCE

**Files:**
- Create: `src/eval/mod.rs`, `src/eval/psqt.rs`, `src/eval/hce.rs`
- Modify: `src/lib.rs`

The trait shape is spec §6.1 verbatim — four hooks, wired through the search from day one even though HCE no-ops three of them. That seam is the whole reason NNUE later drops in without a search rewrite. PST tables: Michniewski's "Simplified Evaluation Function" (CPW, public domain) — single-phase for M2 (tapered is M5).

- [ ] **Step 3.1: Write failing tests** (bottom of new `src/eval/hce.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Position;

    #[test]
    fn startpos_is_balanced() {
        let mut e = Hce::new();
        let pos = Position::startpos();
        assert_eq!(e.evaluate(&pos), 0, "symmetric position must be 0");
    }

    #[test]
    fn eval_is_stm_relative() {
        // same physical position, both side-to-move variants: scores negate
        let w = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 1";
        let b = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
        let mut e = Hce::new();
        let sw = e.evaluate(&Position::from_fen(w).unwrap());
        let sb = e.evaluate(&Position::from_fen(b).unwrap());
        assert_eq!(sw, -sb);
        // e2->e4 is a PST improvement for White
        assert!(sw > 0, "White improved by e4, White to move: positive");
    }

    #[test]
    fn material_dominates_pst() {
        // White is a clean knight up; score from White's view >> 200cp
        let fen = "rnbqkb1r/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let mut e = Hce::new();
        let s = e.evaluate(&Position::from_fen(fen).unwrap());
        assert!(s > 200, "knight-up should exceed 200cp, got {s}");
        assert!(s < 450, "but not exceed knight+max-pst, got {s}");
    }

    #[test]
    fn hooks_are_callable_noops() {
        // the seam contract: search calls these unconditionally from M2 on
        let mut e = Hce::new();
        let mut pos = Position::startpos();
        e.refresh(&pos);
        let before = e.evaluate(&pos);
        let mv = crate::board::movegen::find_uci_move(&pos, "e2e4").unwrap();
        assert!(pos.make(mv));
        e.on_make(mv, &pos);
        pos.unmake();
        e.on_unmake(mv, &pos);
        assert_eq!(e.evaluate(&pos), before, "no-op hooks don't corrupt eval");
    }

    #[test]
    fn mirrored_position_negates() {
        // asymmetric position and its color-flipped mirror: stm-relative
        // scores must be equal (White's edge becomes Black's edge).
        let orig = "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 0 1";
        let flip = "rnbqk2r/pppp1ppp/5n2/2b1p3/4P3/2N5/PPPP1PPP/R1BQKBNR b KQkq - 0 1";
        let mut e = Hce::new();
        let a = e.evaluate(&Position::from_fen(orig).unwrap());
        let b = e.evaluate(&Position::from_fen(flip).unwrap());
        assert_eq!(a, b, "color-flip symmetry violated: {a} vs {b}");
    }
}
```

- [ ] **Step 3.2: `src/eval/mod.rs`** — the seam:

```rust
//! Evaluation behind the NNUE-ready seam (spec §6.1). The search calls
//! refresh/on_make/on_unmake unconditionally from M2 onward; HCE no-ops
//! them, a future NNUE updates its accumulator there.

pub mod hce;
pub mod psqt;

use crate::board::{Move, Position};

pub trait Evaluator {
    /// Full rebuild from the position (search root, ucinewgame).
    fn refresh(&mut self, pos: &Position);
    /// Incremental update; called immediately AFTER pos.make(mv).
    fn on_make(&mut self, mv: Move, pos: &Position);
    /// Incremental downdate; called immediately AFTER pos.unmake().
    fn on_unmake(&mut self, mv: Move, pos: &Position);
    /// Static evaluation in centipawns, side-to-move relative.
    /// (&mut: the HCE pawn hash (M5) and NNUE both mutate caches.)
    fn evaluate(&mut self, pos: &Position) -> i32;
}

pub use hce::Hce;
```

- [ ] **Step 3.3: `src/eval/psqt.rs`** — tables written rank-8-row-first (visually like a board from White's side). Access: white pieces `TABLE[sq ^ 56]`, black pieces `TABLE[sq]`.

```rust
//! Michniewski "Simplified Evaluation Function" piece-square tables
//! (chessprogramming.org, public domain). Single-phase (tapered eval is M5).
//! Layout: rank-8 row first (a8..h8, a7..h7, ...). For a white piece on
//! square s (LERF) use PST[s ^ 56]; for black use PST[s].

pub const MATERIAL: [i32; 6] = [100, 320, 330, 500, 900, 0]; // P N B R Q K

#[rustfmt::skip]
pub const PAWN: [i32; 64] = [
     0,  0,  0,  0,  0,  0,  0,  0,
    50, 50, 50, 50, 50, 50, 50, 50,
    10, 10, 20, 30, 30, 20, 10, 10,
     5,  5, 10, 25, 25, 10,  5,  5,
     0,  0,  0, 20, 20,  0,  0,  0,
     5, -5,-10,  0,  0,-10, -5,  5,
     5, 10, 10,-20,-20, 10, 10,  5,
     0,  0,  0,  0,  0,  0,  0,  0,
];

#[rustfmt::skip]
pub const KNIGHT: [i32; 64] = [
   -50,-40,-30,-30,-30,-30,-40,-50,
   -40,-20,  0,  0,  0,  0,-20,-40,
   -30,  0, 10, 15, 15, 10,  0,-30,
   -30,  5, 15, 20, 20, 15,  5,-30,
   -30,  0, 15, 20, 20, 15,  0,-30,
   -30,  5, 10, 15, 15, 10,  5,-30,
   -40,-20,  0,  5,  5,  0,-20,-40,
   -50,-40,-30,-30,-30,-30,-40,-50,
];

#[rustfmt::skip]
pub const BISHOP: [i32; 64] = [
   -20,-10,-10,-10,-10,-10,-10,-20,
   -10,  0,  0,  0,  0,  0,  0,-10,
   -10,  0,  5, 10, 10,  5,  0,-10,
   -10,  5,  5, 10, 10,  5,  5,-10,
   -10,  0, 10, 10, 10, 10,  0,-10,
   -10, 10, 10, 10, 10, 10, 10,-10,
   -10,  5,  0,  0,  0,  0,  5,-10,
   -20,-10,-10,-10,-10,-10,-10,-20,
];

#[rustfmt::skip]
pub const ROOK: [i32; 64] = [
     0,  0,  0,  0,  0,  0,  0,  0,
     5, 10, 10, 10, 10, 10, 10,  5,
    -5,  0,  0,  0,  0,  0,  0, -5,
    -5,  0,  0,  0,  0,  0,  0, -5,
    -5,  0,  0,  0,  0,  0,  0, -5,
    -5,  0,  0,  0,  0,  0,  0, -5,
    -5,  0,  0,  0,  0,  0,  0, -5,
     0,  0,  0,  5,  5,  0,  0,  0,
];

#[rustfmt::skip]
pub const QUEEN: [i32; 64] = [
   -20,-10,-10, -5, -5,-10,-10,-20,
   -10,  0,  0,  0,  0,  0,  0,-10,
   -10,  0,  5,  5,  5,  5,  0,-10,
    -5,  0,  5,  5,  5,  5,  0, -5,
     0,  0,  5,  5,  5,  5,  0, -5,
   -10,  5,  5,  5,  5,  5,  0,-10,
   -10,  0,  5,  0,  0,  0,  0,-10,
   -20,-10,-10, -5, -5,-10,-10,-20,
];

#[rustfmt::skip]
pub const KING: [i32; 64] = [
   -30,-40,-40,-50,-50,-40,-40,-30,
   -30,-40,-40,-50,-50,-40,-40,-30,
   -30,-40,-40,-50,-50,-40,-40,-30,
   -30,-40,-40,-50,-50,-40,-40,-30,
   -20,-30,-30,-40,-40,-30,-30,-20,
   -10,-20,-20,-20,-20,-20,-20,-10,
    20, 20,  0,  0,  0,  0, 20, 20,
    20, 30, 10,  0,  0, 10, 30, 20,
];

pub const TABLES: [&[i32; 64]; 6] = [&PAWN, &KNIGHT, &BISHOP, &ROOK, &QUEEN, &KING];
```

- [ ] **Step 3.4: `src/eval/hce.rs`** (above the tests from Step 3.1):

```rust
//! M2 hand-crafted eval: material + single-phase PSTs. Full-scan evaluate;
//! the trait hooks are no-ops (NNUE will use them; incremental PST tracking
//! is a possible M3+ optimization, deliberately not done yet — YAGNI).

use crate::board::{Color, Move, PieceType, Position};
use crate::eval::psqt::{MATERIAL, TABLES};
use crate::eval::Evaluator;

#[derive(Default)]
pub struct Hce;

impl Hce {
    pub fn new() -> Hce {
        Hce
    }
}

impl Evaluator for Hce {
    fn refresh(&mut self, _pos: &Position) {}
    fn on_make(&mut self, _mv: Move, _pos: &Position) {}
    fn on_unmake(&mut self, _mv: Move, _pos: &Position) {}

    fn evaluate(&mut self, pos: &Position) -> i32 {
        let mut score = 0i32; // White-relative accumulation
        for pt in PieceType::ALL {
            let val = MATERIAL[pt.index()];
            let table = TABLES[pt.index()];
            for sq in pos.piece_bb(Color::White, pt) {
                score += val + table[sq.index() ^ 56];
            }
            for sq in pos.piece_bb(Color::Black, pt) {
                score -= val + table[sq.index()];
            }
        }
        if pos.stm() == Color::White {
            score
        } else {
            -score
        }
    }
}
```

- [ ] **Step 3.5: Wire the module.** `src/lib.rs` becomes:

```rust
pub mod board;
pub mod eval;
```

- [ ] **Step 3.6: Run tests**

```bash
cargo test eval::
```

Expected: 5 passed.

- [ ] **Step 3.7: Lint + commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/lib.rs src/eval/
git commit -m "feat(eval): Evaluator trait seam and material+PST HCE"
```

---

### Task 4: Search core — negamax + quiescence + MVV-LVA + PV

**Files:**
- Modify: `src/board/moves.rs` (add `as_mut_slice`)
- Create: `src/search/mod.rs`
- Modify: `src/lib.rs`
- Test: `tests/search.rs`

Fixed-depth search only in this task (time management is Task 5). All search state lives in `SearchThread` (spec §5.1); the evaluator hooks are called at every make/unmake site (spec §6.1).

- [ ] **Step 4.1: `MoveList::as_mut_slice`** — append inside `impl MoveList` in `src/board/moves.rs`:

```rust
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [Move] {
        &mut self.moves[..self.len]
    }
```

and a test inside its `mod tests`:

```rust
    #[test]
    fn as_mut_slice_allows_reordering() {
        let mut list = MoveList::new();
        let a = Move::new(Square::E1, Square::G1, Move::KING_CASTLE);
        let b = Move::new(Square::A1, Square::H8, Move::QUIET);
        list.push(a);
        list.push(b);
        list.as_mut_slice().swap(0, 1);
        assert_eq!(list.as_slice()[0], b);
        assert_eq!(list.as_slice()[1], a);
    }
```

Run: `cargo test board::moves` → 5 passed. Commit:

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/board/moves.rs
git commit -m "feat(board): MoveList::as_mut_slice for move ordering"
```

- [ ] **Step 4.2: Write failing integration tests** (`tests/search.rs`)

```rust
//! Search behavior tests: mate finding, draw handling, limit respect.

use nebchess::board::{movegen::find_uci_move, Position};
use nebchess::eval::Hce;
use nebchess::search::{SearchThread, MATE};

fn searcher(fen: &str) -> SearchThread<Hce> {
    SearchThread::new(Position::from_fen(fen).unwrap(), Hce::new())
}

#[test]
fn finds_mate_in_one() {
    // back-rank: 1.Ra8#
    let mut st = searcher("6k1/5ppp/8/8/8/8/8/R3K3 w - - 0 1");
    let (best, score) = st.search_to_depth(2);
    assert_eq!(best.unwrap().to_string(), "a1a8");
    assert_eq!(score, MATE - 1, "mate at ply 1");
}

#[test]
fn finds_mate_in_two() {
    // KR vs K: 1.Kb6! Kb8 2.Rh8# (1.Rh8+? Ka7 escapes; 1.Rh7 Kb8 2.Rh8+ Ka7 escapes)
    let mut st = searcher("k7/8/2K5/8/8/8/8/7R w - - 0 1");
    let (best, score) = st.search_to_depth(4);
    assert_eq!(score, MATE - 3, "mate at ply 3");
    assert_eq!(best.unwrap().to_string(), "c6b6");
}

#[test]
fn stalemate_scores_draw() {
    // black to move, Kh8 has no moves, not in check
    let mut st = searcher("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1");
    let (best, score) = st.search_to_depth(3);
    assert!(best.is_none(), "no legal moves");
    assert!(score.abs() <= 1, "draw jitter only, got {score}");
}

#[test]
fn qsearch_resolves_hanging_queen() {
    // Qd1xd8 wins a queen outright; depth 1 + qsearch must see it
    let mut st = searcher("3q1k2/8/8/8/8/8/8/3Q1K2 w - - 0 1");
    let (best, score) = st.search_to_depth(1);
    assert_eq!(best.unwrap().to_string(), "d1d8");
    assert!(score > 700, "won a queen, got {score}");
}

#[test]
fn depth_one_returns_a_legal_move() {
    let mut st = searcher("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    let (best, _) = st.search_to_depth(1);
    let pos = Position::startpos();
    let mv = best.expect("must produce a move");
    assert!(
        find_uci_move(&pos, &mv.to_string()).is_some(),
        "bestmove must be legal"
    );
}

#[test]
fn node_limit_stops_search() {
    let mut st = searcher("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    st.set_node_limit(Some(10_000));
    let (_best, _) = st.search_to_depth(99);
    assert!(
        st.nodes < 10_000 + 5_000,
        "polling cadence overshoot bounded, got {}",
        st.nodes
    );
}

#[test]
fn fifty_move_draw_scored_in_search() {
    // halfmove already at 100: any deeper node should resolve as draw-ish.
    // KQ vs K would otherwise be a huge score; the rule caps it.
    // (Black king on a8: NOT attacked by Qf6 — the position must be legal.)
    let mut st = searcher("k7/8/5Q2/8/8/8/8/K7 w - - 100 90");
    let (_best, score) = st.search_to_depth(3);
    // root itself is exempt (ply 0); children all return draw — score ~0,
    // far below the +900-ish a live queen would give
    assert!(score.abs() <= 1, "fifty-move children cap score, got {score}");
}
```

- [ ] **Step 4.3: Implement `src/search/mod.rs`**

```rust
//! M2 search: iterative-deepening driver lives in Task 5; this module is
//! fixed-depth negamax + alpha-beta + quiescence with MVV-LVA ordering.
//! All mutable search state lives in SearchThread (spec §5.1).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::board::{generate_moves, Move, MoveList, PieceType, Position};
use crate::eval::psqt::MATERIAL;
use crate::eval::Evaluator;

pub const MATE: i32 = 30_000;
/// |score| above this is a mate score (UCI "score mate" conversion).
pub const MATE_BOUND: i32 = 29_000;
pub const INF: i32 = 32_000;
pub const MAX_PLY: usize = 128;

/// Triangular PV table: row[ply] holds the best line found at that ply.
struct PvTable {
    moves: Vec<[Move; MAX_PLY]>,
    len: [usize; MAX_PLY],
}

impl PvTable {
    fn new() -> PvTable {
        PvTable {
            moves: vec![[Move::NULL; MAX_PLY]; MAX_PLY],
            len: [0; MAX_PLY],
        }
    }
    #[inline]
    fn clear_ply(&mut self, ply: usize) {
        self.len[ply] = 0;
    }
    fn update(&mut self, ply: usize, mv: Move) {
        let child_len = if ply + 1 < MAX_PLY { self.len[ply + 1] } else { 0 };
        let (head, tail) = self.moves.split_at_mut(ply + 1);
        let row = &mut head[ply];
        row[0] = mv;
        if child_len > 0 {
            // guard: at ply == MAX_PLY-1 `tail` is empty — indexing it panics
            row[1..=child_len].copy_from_slice(&tail[0][..child_len]);
        }
        self.len[ply] = child_len + 1;
    }
    fn line(&self) -> &[Move] {
        &self.moves[0][..self.len[0]]
    }
}

/// Scores generated moves once, then yields them best-first by selection.
/// M2 ordering: captures by MVV-LVA (above all quiets), quiets unordered.
struct MovePicker {
    moves: MoveList,
    scores: [i32; 256],
    cur: usize,
}

impl MovePicker {
    fn new(pos: &Position) -> MovePicker {
        let mut moves = MoveList::new();
        generate_moves(pos, &mut moves);
        let mut scores = [0i32; 256];
        for (i, &mv) in moves.iter().enumerate() {
            if mv.is_capture() {
                let victim = if mv.flag() == Move::EN_PASSANT {
                    PieceType::Pawn
                } else {
                    pos.piece_on(mv.to()).expect("capture target").piece_type()
                };
                let attacker = pos.piece_on(mv.from()).expect("mover").piece_type();
                scores[i] =
                    1_000_000 + 10 * MATERIAL[victim.index()] - MATERIAL[attacker.index()];
            }
        }
        MovePicker {
            moves,
            scores,
            cur: 0,
        }
    }

    fn next(&mut self) -> Option<Move> {
        let len = self.moves.len();
        if self.cur >= len {
            return None;
        }
        // selection: swap the best remaining move into position `cur`
        let mut best = self.cur;
        for i in (self.cur + 1)..len {
            if self.scores[i] > self.scores[best] {
                best = i;
            }
        }
        self.moves.as_mut_slice().swap(self.cur, best);
        self.scores.swap(self.cur, best);
        let mv = self.moves.as_slice()[self.cur];
        self.cur += 1;
        Some(mv)
    }
}

pub struct SearchThread<E: Evaluator> {
    pub pos: Position,
    pub eval: E,
    pub nodes: u64,
    stop: Arc<AtomicBool>,
    node_limit: Option<u64>,
    stopped: bool,
    pv: PvTable,
}

impl<E: Evaluator> SearchThread<E> {
    pub fn new(pos: Position, eval: E) -> SearchThread<E> {
        SearchThread {
            pos,
            eval,
            nodes: 0,
            stop: Arc::new(AtomicBool::new(false)),
            node_limit: None,
            stopped: false,
            pv: PvTable::new(),
        }
    }

    /// Share this flag with the UCI thread; setting it aborts the search.
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop)
    }
    pub fn set_stop_flag(&mut self, flag: Arc<AtomicBool>) {
        self.stop = flag;
    }
    pub fn set_node_limit(&mut self, limit: Option<u64>) {
        self.node_limit = limit;
    }

    /// Best line from the last completed search call.
    pub fn pv_line(&self) -> &[Move] {
        self.pv.line()
    }
    pub fn was_stopped(&self) -> bool {
        self.stopped
    }

    /// Fixed-depth, full-window search. Returns (best move, score).
    /// Task 5's iterative deepening calls this once per depth.
    pub fn search_to_depth(&mut self, depth: i32) -> (Option<Move>, i32) {
        self.eval.refresh(&self.pos);
        self.stopped = false;
        let score = self.negamax(depth, -INF, INF, 0);
        (self.pv.line().first().copied(), score)
    }

    /// Polled every 2048 nodes (spec §5.4): external stop or node budget.
    /// Task 5 extends this with the hard time deadline.
    #[inline]
    fn should_stop(&mut self) -> bool {
        if self.stopped {
            return true;
        }
        if self.nodes & 2047 == 0 {
            if self.stop.load(Ordering::Relaxed) {
                self.stopped = true;
            }
            if let Some(limit) = self.node_limit {
                if self.nodes >= limit {
                    self.stopped = true;
                }
            }
        }
        self.stopped
    }

    /// Small jitter (±1cp) instead of flat 0: avoids threefold blindness in
    /// self-play pools (spec §3).
    #[inline]
    fn draw_score(&self) -> i32 {
        1 - (self.nodes as i32 & 2)
    }

    /// 50-move rule with mate precedence: a mated side at halfmove >= 100
    /// is still mated.
    fn fifty_move_score(&mut self, ply: usize) -> i32 {
        let mut list = MoveList::new();
        generate_moves(&self.pos, &mut list);
        let mut any_legal = false;
        for &mv in list.iter() {
            if self.pos.make(mv) {
                self.pos.unmake();
                any_legal = true;
                break;
            }
        }
        if !any_legal && self.pos.in_check(self.pos.stm()) {
            -(MATE - ply as i32)
        } else {
            self.draw_score()
        }
    }

    fn negamax(&mut self, depth: i32, mut alpha: i32, beta: i32, ply: usize) -> i32 {
        self.pv.clear_ply(ply);
        self.nodes += 1;
        if self.should_stop() {
            return 0;
        }
        // stm has no king: unreachable through legal make() flows, but GUI
        // FENs can be illegal (enemy king en prise). Score as already-mated
        // (stm-relative) so the capturer prefers it — and never crash.
        if self.pos.piece_bb(self.pos.stm(), PieceType::King).is_empty() {
            return -(MATE - ply as i32);
        }
        if ply > 0 {
            if self.pos.is_repetition() {
                return self.draw_score();
            }
            if self.pos.is_fifty_move_draw() {
                return self.fifty_move_score(ply);
            }
        }
        if depth <= 0 {
            return self.qsearch(alpha, beta, ply);
        }
        if ply >= MAX_PLY - 1 {
            return self.eval.evaluate(&self.pos);
        }

        let mut picker = MovePicker::new(&self.pos);
        let mut legal = 0u32;
        let mut best = -INF;
        while let Some(mv) = picker.next() {
            if !self.pos.make(mv) {
                continue;
            }
            self.eval.on_make(mv, &self.pos);
            legal += 1;
            let score = -self.negamax(depth - 1, -beta, -alpha, ply + 1);
            self.pos.unmake();
            self.eval.on_unmake(mv, &self.pos);
            if self.stopped {
                return 0;
            }
            if score > best {
                best = score;
                if score > alpha {
                    alpha = score;
                    self.pv.update(ply, mv);
                    if alpha >= beta {
                        break; // beta cutoff
                    }
                }
            }
        }

        if legal == 0 {
            return if self.pos.in_check(self.pos.stm()) {
                -(MATE - ply as i32) // checkmated at this ply
            } else {
                self.draw_score() // stalemate
            };
        }
        best
    }

    fn qsearch(&mut self, mut alpha: i32, beta: i32, ply: usize) -> i32 {
        self.pv.clear_ply(ply);
        self.nodes += 1;
        if self.should_stop() {
            return 0;
        }
        // see negamax: never-crash guard for illegal (en-prise-king) inputs
        if self.pos.piece_bb(self.pos.stm(), PieceType::King).is_empty() {
            return -(MATE - ply as i32);
        }
        if ply >= MAX_PLY - 1 {
            return self.eval.evaluate(&self.pos);
        }
        let in_check = self.pos.in_check(self.pos.stm());
        let mut best = if in_check {
            -INF // no stand-pat while in check: must find an evasion
        } else {
            let stand_pat = self.eval.evaluate(&self.pos);
            if stand_pat >= beta {
                return stand_pat;
            }
            if stand_pat > alpha {
                alpha = stand_pat;
            }
            stand_pat
        };

        let mut picker = MovePicker::new(&self.pos);
        let mut legal = 0u32;
        while let Some(mv) = picker.next() {
            // quiet moves only matter when evading check
            if !in_check && !mv.is_capture() {
                continue;
            }
            if !self.pos.make(mv) {
                continue;
            }
            self.eval.on_make(mv, &self.pos);
            legal += 1;
            let score = -self.qsearch(-beta, -alpha, ply + 1);
            self.pos.unmake();
            self.eval.on_unmake(mv, &self.pos);
            if self.stopped {
                return 0;
            }
            if score > best {
                best = score;
                if score > alpha {
                    alpha = score;
                    self.pv.update(ply, mv);
                    if alpha >= beta {
                        break;
                    }
                }
            }
        }

        if in_check && legal == 0 {
            return -(MATE - ply as i32); // mate found inside qsearch
        }
        best
    }
}
```

- [ ] **Step 4.4: Wire the module.** `src/lib.rs` becomes:

```rust
pub mod board;
pub mod eval;
pub mod search;
```

- [ ] **Step 4.5: Run the tests**

```bash
cargo test --test search
```

Expected: 7 passed. If `finds_mate_in_two` returns a different move with the same score, that's a real bug (the position has a unique mate-in-2 keymove); if scores are off-by-one on mate distances, check the `-(MATE - ply)` convention and the negation at each level.

- [ ] **Step 4.6: Full suite + lint + commit**

```bash
cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/search/ src/lib.rs tests/search.rs
git commit -m "feat(search): negamax + quiescence + MVV-LVA ordering + PV"
```

---

### Task 5: Iterative deepening + time management

**Files:**
- Create: `src/search/limits.rs`
- Modify: `src/search/mod.rs`
- Modify: `tests/search.rs`

Spec §5.4 essentials: soft limit (don't start another iteration), hard limit (abort now, polled at the 2048-node cadence), `Move Overhead` subtracted for GUI/network lag, `movestogo`/`movetime`/`infinite` handled, and a **legal bestmove guaranteed under any stop timing** (first-legal fallback + depth-1 completes before the first poll).

- [ ] **Step 5.1: Write failing unit tests** (bottom of new `src/search/limits.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Color;

    fn lim() -> Limits {
        Limits::default()
    }

    #[test]
    fn movetime_sets_equal_soft_and_hard() {
        let mut l = lim();
        l.movetime = Some(500);
        let tm = TimeManager::new(&l, Color::White, 10);
        let (soft, hard) = tm.budgets_ms();
        assert_eq!(soft, Some(490));
        assert_eq!(hard, Some(490));
    }

    #[test]
    fn clock_allocation_is_sane() {
        let mut l = lim();
        l.wtime = Some(60_000); // one minute, no increment
        let tm = TimeManager::new(&l, Color::White, 10);
        let (soft, hard) = tm.budgets_ms();
        let (soft, hard) = (soft.unwrap(), hard.unwrap());
        assert!(soft >= 1_500 && soft <= 2_500, "soft ~ time/30, got {soft}");
        assert_eq!(hard, soft * 4);
        // black's clock must be read for black
        let mut l = lim();
        l.btime = Some(30_000);
        let tm = TimeManager::new(&l, Color::Black, 10);
        assert!(tm.budgets_ms().0.unwrap() <= 1_100);
    }

    #[test]
    fn movestogo_and_increment_raise_budget() {
        let mut l = lim();
        l.wtime = Some(60_000);
        l.movestogo = Some(10);
        l.winc = Some(2_000);
        let tm = TimeManager::new(&l, Color::White, 10);
        let soft = tm.budgets_ms().0.unwrap();
        assert!(soft >= 6_500 && soft <= 7_500, "time/10 + inc/2, got {soft}");
    }

    #[test]
    fn low_time_never_overspends() {
        let mut l = lim();
        l.wtime = Some(50); // 50ms on the clock!
        let tm = TimeManager::new(&l, Color::White, 10);
        let (soft, hard) = tm.budgets_ms();
        let hard = hard.unwrap();
        assert!(hard <= 40, "hard must stay under remaining-overhead, got {hard}");
        assert!(hard >= 1);
        assert!(soft.unwrap() <= hard);
    }

    #[test]
    fn infinite_and_depth_only_have_no_deadlines() {
        let mut l = lim();
        l.infinite = true;
        let tm = TimeManager::new(&l, Color::White, 10);
        assert_eq!(tm.budgets_ms(), (None, None));
        let mut l = lim();
        l.depth = Some(6);
        let tm = TimeManager::new(&l, Color::White, 10);
        assert_eq!(tm.budgets_ms(), (None, None));
    }
}
```

- [ ] **Step 5.2: Implement `src/search/limits.rs`** (above the tests)

```rust
//! `go` parameters and time allocation (spec §5.4).

use std::time::{Duration, Instant};

use crate::board::Color;

#[derive(Default, Clone, Debug)]
pub struct Limits {
    pub depth: Option<i32>,
    pub nodes: Option<u64>,
    pub movetime: Option<u64>, // ms
    pub wtime: Option<u64>,
    pub btime: Option<u64>,
    pub winc: Option<u64>,
    pub binc: Option<u64>,
    pub movestogo: Option<u32>,
    pub infinite: bool,
}

pub struct TimeManager {
    start: Instant,
    soft: Option<Duration>,
    hard: Option<Duration>,
}

impl TimeManager {
    pub fn new(limits: &Limits, stm: Color, overhead_ms: u64) -> TimeManager {
        let start = Instant::now();
        let (soft, hard) = if limits.infinite {
            (None, None)
        } else if let Some(mt) = limits.movetime {
            let t = mt.saturating_sub(overhead_ms).max(1);
            (Some(t), Some(t))
        } else {
            let (time, inc) = match stm {
                Color::White => (limits.wtime, limits.winc.unwrap_or(0)),
                Color::Black => (limits.btime, limits.binc.unwrap_or(0)),
            };
            match time {
                None => (None, None), // depth/nodes-only searches
                Some(time) => {
                    let avail = time.saturating_sub(overhead_ms).max(1);
                    let mtg = u64::from(limits.movestogo.unwrap_or(30).clamp(1, 30));
                    let soft = (avail / mtg + inc / 2).clamp(1, avail);
                    let hard = (soft * 4).min(avail);
                    (Some(soft), Some(hard))
                }
            }
        };
        TimeManager {
            start,
            soft: soft.map(Duration::from_millis),
            hard: hard.map(Duration::from_millis),
        }
    }

    /// Absolute instant for the in-search abort poll (None = no time control).
    pub fn hard_deadline(&self) -> Option<Instant> {
        self.hard.map(|d| self.start + d)
    }

    /// Checked between iterations: don't start another depth past soft.
    pub fn past_soft(&self) -> bool {
        match self.soft {
            Some(s) => self.start.elapsed() >= s,
            None => false,
        }
    }

    pub fn elapsed_ms(&self) -> u128 {
        self.start.elapsed().as_millis()
    }

    /// (soft, hard) in ms — for tests and debugging.
    pub fn budgets_ms(&self) -> (Option<u64>, Option<u64>) {
        (
            self.soft.map(|d| d.as_millis() as u64),
            self.hard.map(|d| d.as_millis() as u64),
        )
    }
}
```

- [ ] **Step 5.3: Extend `src/search/mod.rs`.** Add at the top:

```rust
pub mod limits;

use std::time::Instant;

use crate::search::limits::{Limits, TimeManager};
```

Add fields to `SearchThread` (after `stopped: bool,`):

```rust
    deadline: Option<Instant>,
    overhead_ms: u64,
```

initialize in `new()` (after `stopped: false,`):

```rust
            deadline: None,
            overhead_ms: 10,
```

add a setter alongside the others:

```rust
    pub fn set_overhead_ms(&mut self, ms: u64) {
        self.overhead_ms = ms;
    }
```

extend `should_stop`'s 2048-cadence block (after the node-limit check, inside the same `if self.nodes & 2047 == 0`):

```rust
            if let Some(d) = self.deadline {
                if Instant::now() >= d {
                    self.stopped = true;
                }
            }
```

Add the per-iteration report type and the driver to the module (outside `impl`):

```rust
/// Per-iteration report for UCI `info` lines.
pub struct IterInfo<'a> {
    pub depth: i32,
    pub score: i32,
    pub nodes: u64,
    pub elapsed_ms: u128,
    pub pv: &'a [Move],
}
```

and inside `impl<E: Evaluator> SearchThread<E>`:

```rust
    /// First root move that survives the legality filter (bestmove fallback).
    fn first_legal(&mut self) -> Option<Move> {
        let mut list = MoveList::new();
        generate_moves(&self.pos, &mut list);
        for &mv in list.iter() {
            if self.pos.make(mv) {
                self.pos.unmake();
                return Some(mv);
            }
        }
        None
    }

    /// Iterative deepening driver. Returns None only when the root has no
    /// legal moves (mate/stalemate already on the board).
    /// `info` is called after every COMPLETED iteration.
    pub fn iterate(
        &mut self,
        limits: &Limits,
        mut info: impl FnMut(IterInfo),
    ) -> Option<Move> {
        let tm = TimeManager::new(limits, self.pos.stm(), self.overhead_ms);
        self.deadline = tm.hard_deadline();
        self.node_limit = limits.nodes;
        self.nodes = 0;
        self.stop.store(false, std::sync::atomic::Ordering::Relaxed);

        let mut best = self.first_legal()?;
        let max_depth = limits.depth.unwrap_or(MAX_PLY as i32 - 1).clamp(1, MAX_PLY as i32 - 1);

        for depth in 1..=max_depth {
            let (mv, score) = self.search_to_depth(depth);
            if self.was_stopped() {
                // partial iteration: only trust it at depth 1 (first full
                // root move beats the arbitrary fallback)
                if depth == 1 {
                    if let Some(mv) = mv {
                        best = mv;
                    }
                }
                break;
            }
            best = mv.expect("completed iteration always has a move");
            info(IterInfo {
                depth,
                score,
                nodes: self.nodes,
                elapsed_ms: tm.elapsed_ms(),
                pv: self.pv.line(),
            });
            if tm.past_soft() {
                break;
            }
            if score.abs() >= MATE_BOUND {
                break; // forced mate found; deeper search can't change it
            }
        }
        Some(best)
    }
```

NOTE: `iterate` clears the external stop flag at entry — the UCI layer sets it to abort and the next `go` must start clean.

- [ ] **Step 5.4: Add driver tests** (append to `tests/search.rs`)

```rust
use nebchess::search::limits::Limits;
use std::time::Instant;

#[test]
fn movetime_is_respected() {
    let mut st = searcher("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    let mut limits = Limits::default();
    limits.movetime = Some(100);
    let t0 = Instant::now();
    let best = st.iterate(&limits, |_| {});
    let elapsed = t0.elapsed().as_millis();
    assert!(best.is_some());
    assert!(elapsed < 600, "movetime 100 took {elapsed}ms");
}

#[test]
fn depth_limit_caps_iterations() {
    let mut st = searcher("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    let mut limits = Limits::default();
    limits.depth = Some(3);
    let mut depths = Vec::new();
    st.iterate(&limits, |i| depths.push(i.depth));
    assert_eq!(depths, vec![1, 2, 3]);
}

#[test]
fn tiny_node_budget_still_returns_legal_move() {
    let mut st = searcher("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1");
    // a 1-node budget forces the earliest possible abort path; the
    // first-legal fallback must still produce a legal bestmove
    let mut limits = Limits::default();
    limits.nodes = Some(1);
    let best = st.iterate(&limits, |_| {}).expect("legal moves exist");
    let pos = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
    assert!(find_uci_move(&pos, &best.to_string()).is_some());
}

#[test]
fn clock_allocation_returns_promptly() {
    let mut st = searcher("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    let mut limits = Limits::default();
    limits.wtime = Some(1_000); // soft ~33ms, hard ~132ms
    let t0 = Instant::now();
    st.iterate(&limits, |_| {});
    assert!(t0.elapsed().as_millis() < 700);
}

#[test]
fn mate_found_exits_early() {
    let mut st = searcher("6k1/5ppp/8/8/8/8/8/R3K3 w - - 0 1");
    let limits = Limits::default(); // no limits at all
    let t0 = Instant::now();
    let best = st.iterate(&limits, |_| {});
    assert_eq!(best.unwrap().to_string(), "a1a8");
    assert!(t0.elapsed().as_secs() < 5, "mate-bound early exit");
}

#[test]
fn no_legal_moves_returns_none() {
    // stalemate on the board
    let mut st = searcher("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1");
    let best = st.iterate(&Limits::default(), |_| {});
    assert!(best.is_none());
}
```

- [ ] **Step 5.5: Run + lint + commit**

```bash
cargo test --test search && cargo test
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/search/ tests/search.rs
git commit -m "feat(search): iterative deepening with soft/hard time management"
```

Expected: 13 search tests green, no regressions.

---

### Task 6: UCI protocol + engine binary + edge-case gates

**Files:**
- Create: `src/uci/mod.rs`, `src/main.rs`
- Modify: `src/lib.rs`
- Test: `tests/uci.rs`

Architecture: main thread owns stdin and the master `Position` (whose `key_history` IS the game history — `position ... moves ...` replays through `make()`). Search runs on a worker thread with a cloned Position; the only shared state is the `AtomicBool` stop flag. `isready` answers immediately even mid-search because the main thread never blocks on the search (spec §7).

- [ ] **Step 6.1: Implement `src/uci/mod.rs`**

```rust
//! UCI protocol (spec §7). Main thread: stdin + master position.
//! Worker thread: one search at a time, aborted via the shared stop flag.

use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::board::movegen::find_uci_move;
use crate::board::Position;
use crate::eval::Hce;
use crate::search::limits::Limits;
use crate::search::{IterInfo, SearchThread, MATE, MATE_BOUND};

pub const NAME: &str = concat!("NebChess ", env!("CARGO_PKG_VERSION"));

pub fn run() {
    Uci::new().main_loop();
}

struct Uci {
    pos: Position,
    stop: Arc<AtomicBool>,
    search: Option<JoinHandle<()>>,
    overhead_ms: u64,
}

impl Uci {
    fn new() -> Uci {
        Uci {
            pos: Position::startpos(),
            stop: Arc::new(AtomicBool::new(false)),
            search: None,
            overhead_ms: 10,
        }
    }

    fn main_loop(&mut self) {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            let cmd = line.split_whitespace().next().unwrap_or("");
            match cmd {
                "uci" => self.cmd_uci(),
                "isready" => println!("readyok"),
                "ucinewgame" => {
                    self.stop_and_join();
                    self.pos = Position::startpos();
                    // M3: clear the transposition table here
                }
                "position" => {
                    self.stop_and_join();
                    self.cmd_position(&line);
                }
                "go" => {
                    self.stop_and_join();
                    self.cmd_go(&line);
                }
                "stop" => self.stop_and_join(),
                "setoption" => self.cmd_setoption(&line),
                // debug extension (not UCI): print the current FEN
                "fen" => println!("{}", self.pos.to_fen()),
                "quit" => {
                    self.stop_and_join();
                    return;
                }
                _ => {} // unknown commands are ignored per UCI custom
            }
            io::stdout().flush().ok();
        }
        self.stop_and_join(); // EOF
    }

    /// Abort any running search and wait for its bestmove to be printed.
    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.search.take() {
            h.join().ok();
        }
    }

    fn cmd_uci(&self) {
        println!("id name {NAME}");
        println!("id author N3bDev");
        // Hash/Threads/MultiPV: accepted but inert in M2 (TT=M3, SMP=M9b);
        // advertised so GUIs can set them without erroring.
        println!("option name Hash type spin default 16 min 1 max 4096");
        println!("option name Threads type spin default 1 min 1 max 1");
        println!("option name MultiPV type spin default 1 min 1 max 1");
        println!("option name Move Overhead type spin default 10 min 0 max 5000");
        println!("uciok");
    }

    fn cmd_setoption(&mut self, line: &str) {
        // setoption name <name words...> value <v>
        let mut name = Vec::new();
        let mut value = None;
        let mut tok = line.split_whitespace().skip(1); // skip "setoption"
        if tok.next() != Some("name") {
            return;
        }
        let mut in_value = false;
        for t in tok {
            if t == "value" {
                in_value = true;
            } else if in_value {
                value = Some(t.to_string());
                break;
            } else {
                name.push(t);
            }
        }
        let name = name.join(" ");
        match (name.as_str(), value) {
            ("Move Overhead", Some(v)) => {
                if let Ok(ms) = v.parse::<u64>() {
                    self.overhead_ms = ms.min(5000);
                }
            }
            // Hash / Threads / MultiPV: accepted, inert until M3/M9b
            _ => {}
        }
    }

    fn cmd_position(&mut self, line: &str) {
        let mut tok = line.split_whitespace().skip(1); // skip "position"
        let mut saw_moves = false;
        match tok.next() {
            Some("startpos") => {
                self.pos = Position::startpos();
                saw_moves = tok.next() == Some("moves");
            }
            Some("fen") => {
                let mut fen_parts = Vec::new();
                for t in tok.by_ref() {
                    if t == "moves" {
                        saw_moves = true;
                        break;
                    }
                    fen_parts.push(t);
                }
                match Position::from_fen(&fen_parts.join(" ")) {
                    Ok(p) => self.pos = p,
                    Err(e) => {
                        println!("info string {e}");
                        return;
                    }
                }
            }
            _ => return,
        }
        if saw_moves {
            for uci in tok {
                match find_uci_move(&self.pos, uci) {
                    Some(mv) if self.pos.make(mv) => {}
                    _ => {
                        println!("info string ignoring illegal move {uci}");
                        return;
                    }
                }
            }
        }
    }

    fn cmd_go(&mut self, line: &str) {
        let limits = parse_go(line);
        let mut st = SearchThread::new(self.pos.clone(), Hce::new());
        st.set_stop_flag(Arc::clone(&self.stop));
        st.set_overhead_ms(self.overhead_ms);
        self.search = Some(std::thread::spawn(move || {
            let best = st.iterate(&limits, print_info);
            match best {
                Some(mv) => println!("bestmove {mv}"),
                None => println!("bestmove 0000"), // no legal moves on board
            }
            io::stdout().flush().ok();
        }));
    }
}

fn parse_go(line: &str) -> Limits {
    let mut limits = Limits::default();
    let mut tok = line.split_whitespace().skip(1).peekable();
    while let Some(t) = tok.next() {
        let mut num = |dst: &mut Option<u64>| {
            if let Some(v) = tok.peek().and_then(|s| s.parse::<u64>().ok()) {
                *dst = Some(v);
                tok.next();
            }
        };
        match t {
            "wtime" => num(&mut limits.wtime),
            "btime" => num(&mut limits.btime),
            "winc" => num(&mut limits.winc),
            "binc" => num(&mut limits.binc),
            "movetime" => num(&mut limits.movetime),
            "nodes" => num(&mut limits.nodes),
            "depth" => {
                if let Some(v) = tok.peek().and_then(|s| s.parse::<i32>().ok()) {
                    limits.depth = Some(v);
                    tok.next();
                }
            }
            "movestogo" => {
                if let Some(v) = tok.peek().and_then(|s| s.parse::<u32>().ok()) {
                    limits.movestogo = Some(v);
                    tok.next();
                }
            }
            // ponder isn't advertised; if a GUI sends it anyway, treating the
            // search as infinite is the safe interpretation (stop/quit ends it)
            "infinite" | "ponder" => limits.infinite = true,
            _ => {} // searchmoves etc: ignored in M2
        }
    }
    limits
}

fn print_info(i: IterInfo) {
    let nps = if i.elapsed_ms > 0 {
        (i.nodes as u128 * 1000 / i.elapsed_ms) as u64
    } else {
        0
    };
    let score = if i.score.abs() >= MATE_BOUND {
        let plies = MATE - i.score.abs();
        let moves = (plies + 1) / 2;
        if i.score > 0 {
            format!("mate {moves}")
        } else {
            format!("mate -{moves}")
        }
    } else {
        format!("cp {}", i.score)
    };
    print!(
        "info depth {} score {} nodes {} nps {} time {} pv",
        i.depth, score, i.nodes, nps, i.elapsed_ms
    );
    for mv in i.pv {
        print!(" {mv}");
    }
    println!();
    io::stdout().flush().ok();
}
```

A borrow subtlety in `parse_go`: the `num` closure mutably borrows `tok`, so the `depth`/`movestogo` arms (which use `tok` directly) can't coexist with a long-lived closure. As written, `num` is re-created per loop iteration and the direct-use arms don't call it — this compiles. If the borrow checker objects on the implementer's toolchain, inline the closure body per arm and report the deviation.

- [ ] **Step 6.2: The binary.** `src/main.rs`:

```rust
fn main() {
    nebchess::uci::run();
}
```

`src/lib.rs` becomes:

```rust
pub mod board;
pub mod eval;
pub mod search;
pub mod uci;
```

Build check: `cargo build --release` — produces `target/release/nebchess`.

- [ ] **Step 6.3: Write the UCI gate tests** (`tests/uci.rs`) — these are the spec §7 acceptance gates, driving the real binary over pipes:

```rust
//! UCI edge-case gates (spec §7): these failures forfeit real games.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use nebchess::board::{movegen::find_uci_move, Position};

const T: Duration = Duration::from_secs(5);

struct Engine {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<String>,
}

impl Engine {
    fn start() -> Engine {
        let mut child = Command::new(env!("CARGO_BIN_EXE_nebchess"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn engine");
        let stdout = child.stdout.take().unwrap();
        let stdin = child.stdin.take().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Engine { child, stdin, rx }
    }

    fn send(&mut self, s: &str) {
        writeln!(self.stdin, "{s}").expect("engine stdin");
    }

    /// Lines until (and including) the first one matching `stop`.
    fn collect_until(&mut self, stop: impl Fn(&str) -> bool) -> Vec<String> {
        let deadline = Instant::now() + T;
        let mut out = Vec::new();
        loop {
            let remain = deadline
                .checked_duration_since(Instant::now())
                .expect("timeout waiting for engine output");
            let line = self.rx.recv_timeout(remain).expect("engine output timeout");
            let done = stop(&line);
            out.push(line);
            if done {
                return out;
            }
        }
    }

    fn expect_line(&mut self, pred: impl Fn(&str) -> bool) -> String {
        self.collect_until(pred).pop().unwrap()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "quit");
        thread::sleep(Duration::from_millis(100));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// bestmove must be legal in the given position (resolved via the lib).
fn assert_legal_bestmove(line: &str, pos: &Position) {
    let mv = line
        .strip_prefix("bestmove ")
        .expect("bestmove line")
        .split_whitespace()
        .next()
        .unwrap();
    assert!(
        find_uci_move(pos, mv).is_some(),
        "illegal bestmove {mv} in {}",
        pos.to_fen()
    );
}

#[test]
fn uci_handshake_lists_identity_and_options() {
    let mut e = Engine::start();
    e.send("uci");
    let lines = e.collect_until(|l| l == "uciok");
    assert!(lines.iter().any(|l| l.starts_with("id name NebChess")));
    assert!(lines.iter().any(|l| l.contains("option name Move Overhead")));
    assert!(lines.iter().any(|l| l.contains("option name Hash")));
}

#[test]
fn isready_answers_readyok() {
    let mut e = Engine::start();
    e.send("isready");
    e.expect_line(|l| l == "readyok");
}

#[test]
fn position_replay_matches_library_fen() {
    // full-game replay equivalence: castles both sides + pawn captures
    let moves = "e2e4 e7e5 g1f3 g8f6 f1c4 f8c5 e1g1 e8g8 d2d4 e5d4 c2c3 d4c3 b1c3";
    let mut e = Engine::start();
    e.send(&format!("position startpos moves {moves}"));
    e.send("fen");
    let got = e.expect_line(|l| l.contains(' ') && l.split(' ').count() == 6);
    // compute the same thing through the library
    let mut pos = Position::startpos();
    for m in moves.split(' ') {
        let mv = find_uci_move(&pos, m).expect("test moves are legal");
        assert!(pos.make(mv));
    }
    assert_eq!(got, pos.to_fen(), "UCI replay diverged from library");
}

#[test]
fn go_then_immediate_stop_still_gives_legal_bestmove() {
    let mut e = Engine::start();
    e.send("position startpos moves e2e4 e7e5");
    e.send("go infinite");
    thread::sleep(Duration::from_millis(50));
    e.send("stop");
    let line = e.expect_line(|l| l.starts_with("bestmove"));
    let mut pos = Position::startpos();
    for m in ["e2e4", "e7e5"] {
        let mv = find_uci_move(&pos, m).unwrap();
        pos.make(mv);
    }
    assert_legal_bestmove(&line, &pos);
}

#[test]
fn isready_during_search_answers_before_bestmove() {
    let mut e = Engine::start();
    e.send("position startpos");
    e.send("go movetime 500");
    e.send("isready");
    let line = e.expect_line(|l| l == "readyok" || l.starts_with("bestmove"));
    assert_eq!(line, "readyok", "isready must not wait for the search");
    e.expect_line(|l| l.starts_with("bestmove"));
}

#[test]
fn ucinewgame_resets_cleanly_between_games() {
    let mut e = Engine::start();
    e.send("ucinewgame");
    e.send("position startpos moves e2e4");
    e.send("go depth 3");
    e.expect_line(|l| l.starts_with("bestmove"));
    e.send("ucinewgame");
    e.send("position startpos");
    e.send("go depth 3");
    let line = e.expect_line(|l| l.starts_with("bestmove"));
    assert_legal_bestmove(&line, &Position::startpos());
}

#[test]
fn go_depth_emits_info_lines_with_pv() {
    let mut e = Engine::start();
    e.send("position startpos");
    e.send("go depth 4");
    let lines = e.collect_until(|l| l.starts_with("bestmove"));
    let infos: Vec<&String> = lines.iter().filter(|l| l.starts_with("info depth")).collect();
    assert!(infos.len() >= 4, "one info per completed depth");
    assert!(infos.iter().all(|l| l.contains(" pv ")), "info lines carry pv");
    assert!(infos.iter().all(|l| l.contains(" score cp ") || l.contains(" score mate ")));
}

#[test]
fn illegal_replay_move_is_reported_not_fatal() {
    let mut e = Engine::start();
    e.send("position startpos moves e2e4 e2e4");
    let line = e.expect_line(|l| l.starts_with("info string"));
    assert!(line.contains("illegal"));
    // engine must still be responsive afterwards
    e.send("isready");
    e.expect_line(|l| l == "readyok");
}

#[test]
fn checkmated_position_answers_null_bestmove() {
    // fool's mate delivered: white is mated, no legal moves
    let mut e = Engine::start();
    e.send("position startpos moves f2f3 e7e5 g2g4 d8h4");
    e.send("go depth 3");
    let line = e.expect_line(|l| l.starts_with("bestmove"));
    assert_eq!(line, "bestmove 0000");
}
```

- [ ] **Step 6.4: Run the gates**

```bash
cargo test --test uci
```

Expected: 9 passed (each spawns a fresh engine process; total < 30s).

- [ ] **Step 6.5: Manual smoke** (optional but satisfying — your first game!)

```bash
printf 'uci\nposition startpos\ngo movetime 1000\nquit\n' | ./target/release/nebchess
```

Expected: id/options/uciok, info lines climbing in depth, a sane opening bestmove (e2e4/d2d4/g1f3-class).

- [ ] **Step 6.6: Full suite + lint + commit**

```bash
cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/uci/ src/main.rs src/lib.rs tests/uci.rs
git commit -m "feat(uci): full protocol, search thread, edge-case gates"
```

---

### Task 7: Deterministic bench + commit-message convention + CI assertion

**Files:**
- Create: `src/search/bench.rs`, `tools/check-bench.sh`
- Modify: `src/main.rs`, `src/search/mod.rs`, `.github/workflows/ci.yml`, `README.md`

Spec §10.2: fixed positions, fixed depth, threads=1 → bit-reproducible node count, recorded in commit messages (Stockfish convention), asserted by CI.

- [ ] **Step 7.1: `src/search/bench.rs`**

```rust
//! Deterministic bench (spec §10.2): fixed positions, fixed depth, no time
//! control. The total node count fingerprints search behavior — it goes in
//! every engine-affecting commit message as "Bench: N" and CI re-verifies it.

use std::time::Instant;

use crate::board::Position;
use crate::eval::Hce;
use crate::search::SearchThread;

pub const BENCH_DEPTH: i32 = 6;

pub const BENCH_FENS: [&str; 12] = [
    "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
    "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
    "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
    "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
    "r3k2r/1b4bq/8/8/8/8/7B/R3K2R w KQkq - 0 1",
    "2K2r2/4P3/8/8/8/8/8/3k4 w - - 0 1",
    "8/8/1P2K3/8/2n5/1q6/8/5k2 b - - 0 1",
    "8/8/1k6/2b5/2pP4/8/5K2/8 b - d3 0 1",
    "4k3/8/8/8/8/8/8/R3K3 w Q - 0 1",
    "7k/5Q2/6K1/8/8/8/8/8 w - - 0 1",
];

pub fn run() {
    let start = Instant::now();
    let mut total: u64 = 0;
    for (i, fen) in BENCH_FENS.iter().enumerate() {
        let pos = Position::from_fen(fen).expect("bench FEN");
        let mut st = SearchThread::new(pos, Hce::new());
        let (_best, _score) = st.search_to_depth(BENCH_DEPTH);
        println!("position {:>2}: {:>10} nodes", i + 1, st.nodes);
        total += st.nodes;
    }
    let ms = start.elapsed().as_millis().max(1);
    println!("nps: {}", total as u128 * 1000 / ms);
    println!("Bench: {total}");
}
```

NOTE: `search_to_depth` searches one fixed depth (not iterative) — deterministic by construction (no time, no TT, no threads; the draw-jitter uses the node counter which is itself deterministic).

- [ ] **Step 7.2: Wire it.** `src/search/mod.rs` top: add `pub mod bench;`. `src/main.rs` becomes:

```rust
fn main() {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("bench") => nebchess::search::bench::run(),
        _ => nebchess::uci::run(),
    }
}
```

- [ ] **Step 7.3: Determinism check**

```bash
cargo build --release
./target/release/nebchess bench | tail -1
./target/release/nebchess bench | tail -1
```

Expected: two identical `Bench: N` lines. **Record N — the commit in Step 7.6 embeds it.**

- [ ] **Step 7.4: `tools/check-bench.sh`**

```bash
#!/usr/bin/env bash
# CI gate (spec §10.2): when HEAD's commit message carries "Bench: N",
# the built binary must reproduce exactly N. Commits without a Bench line
# (docs, tools) are skipped.
set -euo pipefail
cd "$(dirname "$0")/.."
expected=$(git log -1 --pretty=%B | grep -oP '^Bench: \K[0-9]+' | head -1 || true)
if [ -z "$expected" ]; then
  echo "no Bench: line in HEAD commit message; skipping"
  exit 0
fi
actual=$(./target/release/nebchess bench | grep -oP '^Bench: \K[0-9]+')
if [ "$expected" != "$actual" ]; then
  echo "BENCH MISMATCH: commit says $expected, binary produces $actual"
  exit 1
fi
echo "bench ok: $actual"
```

```bash
chmod +x tools/check-bench.sh
```

- [ ] **Step 7.5: CI step.** In `.github/workflows/ci.yml`, append after the `Build (release)` step (before `Tests`):

```yaml
      - name: Bench check
        run: tools/check-bench.sh
```

And add the convention to `README.md` (new section before `## Build`):

```markdown
## Development

Engine-affecting commits carry a `Bench: <nodes>` line (get it via
`./target/release/nebchess bench | tail -1`); CI re-runs the bench and
fails on mismatch. Docs/tooling commits omit the line and are skipped.
```

- [ ] **Step 7.6: Run everything + commit WITH the bench line**

```bash
tools/check-bench.sh   # will skip (current HEAD has no Bench line) — checks the script runs
cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/search/bench.rs src/search/mod.rs src/main.rs tools/check-bench.sh .github/workflows/ci.yml README.md
git commit -m "feat(search): deterministic bench with CI assertion" -m "Bench: <N from Step 7.3>"
```

(Substitute the real number. From this commit on, every commit that changes search/eval/board behavior carries its bench.)

---

### Task 8: The game-playing gates — smoke match, zero-forfeit gauntlet, replay cross-check

**Files:**
- Create: `tools/smoke-match.sh`, `tools/forfeit-gauntlet.sh`, `tools/uci_replay_check.py`

Three gates: (1) the deferred M0 smoke match — fastchess runs NebChess end-to-end; (2) spec §5.4/§12's zero-time-forfeit acceptance (a single forfeit is a blocker, not noise); (3) UCI replay equivalence vs Stockfish on random games (the spec §7 "GUIs resend the whole game" hazard).

- [ ] **Step 8.1: `tools/smoke-match.sh`**

```bash
#!/usr/bin/env bash
# Deferred M0 gate: a real 2-engine match runs end-to-end (10 games self-play).
set -euo pipefail
cd "$(dirname "$0")"
ENGINE="$(realpath ../target/release/nebchess)"
bin/fastchess \
  -engine cmd="$ENGINE" name=neb-a -engine cmd="$ENGINE" name=neb-b \
  -each tc=8+0.08 option.Hash=16 -threads 1 \
  -openings file=books/8moves_v3.pgn format=pgn order=random \
  -rounds 5 -repeat -concurrency 5 -recover \
  -pgnout file=smoke.pgn 2>&1 | tee smoke.log
echo "--- failure scan ---"
if grep -Ei "disconnect|illegal|loses on time|timeout|stall|crash" smoke.log; then
  echo "SMOKE FAILURE DETECTED"
  exit 1
fi
echo "smoke ok: 10 games completed cleanly"
```

- [ ] **Step 8.2: `tools/forfeit-gauntlet.sh`**

```bash
#!/usr/bin/env bash
# Zero-time-forfeit acceptance gauntlet (spec §5.4, §12 M2 gate).
# A single time loss is a BLOCKER bug, not noise. Usage: forfeit-gauntlet.sh [rounds=100]
set -euo pipefail
cd "$(dirname "$0")"
ROUNDS="${1:-100}"
ENGINE="$(realpath ../target/release/nebchess)"
rm -f gauntlet.pgn
bin/fastchess \
  -engine cmd="$ENGINE" name=new -engine cmd="$ENGINE" name=old \
  -each tc=8+0.08 option.Hash=16 -threads 1 \
  -openings file=books/8moves_v3.pgn format=pgn order=random \
  -rounds "$ROUNDS" -repeat -concurrency "$(( $(nproc) - 1 ))" -recover -randomseed \
  -draw movenumber=40 movecount=8 score=10 \
  -pgnout file=gauntlet.pgn 2>&1 | tee gauntlet.log
echo "--- forfeit scan (console) ---"
if grep -Ei "loses on time|timeout|disconnect|illegal move|crash" gauntlet.log; then
  echo "FORFEIT/FAILURE DETECTED"
  exit 1
fi
echo "--- forfeit scan (pgn terminations) ---"
forfeits=$(grep -ci "time forfeit" gauntlet.pgn || true)
echo "time forfeits in pgn: $forfeits"
[ "$forfeits" = "0" ] && echo "gauntlet ok: $((ROUNDS * 2)) games, zero forfeits"
```

- [ ] **Step 8.3: `tools/uci_replay_check.py`**

```python
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
```

- [ ] **Step 8.4: Run all three gates**

```bash
chmod +x tools/smoke-match.sh tools/forfeit-gauntlet.sh tools/uci_replay_check.py
cargo build --release
tools/smoke-match.sh
python3 tools/uci_replay_check.py 20 60
tools/forfeit-gauntlet.sh 100
```

Expected: smoke `10 games completed cleanly` (~1 min); replay `PASS: ~900-1200 replay positions matched` (~1-3 min); gauntlet `200 games, zero forfeits` (~5-8 min at concurrency 15). **A time forfeit or replay mismatch is a blocker: report it with the log excerpt, do NOT proceed.** Also eyeball `tools/smoke.log`'s result table and report the W/L/D (self-play should be roughly balanced, draws common).

- [ ] **Step 8.5: Commit (scripts only — logs/pgns are local; they match existing gitignore? `smoke.pgn`/`gauntlet.pgn`/`*.log` are new)** — add to `.gitignore` first:

```
/tools/*.pgn
/tools/*.log
```

```bash
git add .gitignore tools/smoke-match.sh tools/forfeit-gauntlet.sh tools/uci_replay_check.py
git commit -m "tools: smoke match, zero-forfeit gauntlet, UCI replay cross-check"
```

---

### Task 9: M2 wrap-up

**Files:**
- Modify: `README.md`, `Cargo.toml`

- [ ] **Step 9.1: Version + status.** `Cargo.toml`: `version = "0.2.0"`. README status list:

```markdown
- [x] M0: scaffolding, CI, test harness
- [x] M1: board representation + perft-verified move generation
- [x] M2: minimal playing engine (search, eval, UCI)
- [ ] M3: transposition table + move ordering + PVS
```

Also add a play section after Status:

```markdown
## Play against it

Build (`cargo build --release`), then point any UCI GUI (CuteChess, Arena,
En Croissant) at `target/release/nebchess`.
```

- [ ] **Step 9.2: Full local gate** (board mutation paths changed in T2 — the deep perft tier re-verifies them):

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo test --release --test perft -- --ignored
```

Expected: all green including deep perft (~25s).

- [ ] **Step 9.3: Commit, push, CI**

```bash
git add README.md Cargo.toml Cargo.lock
git commit -m "docs: mark M2 complete, bump to 0.2.0"
git push
gh run watch --exit-status $(gh run list --limit 1 --json databaseId --jq '.[0].databaseId')
```

Expected: CI success (now including the bench-check step — this commit has no Bench line, so that step skips; the T7 commit exercised the match path locally).

---

## Plan self-review notes

- **Spec coverage (M2 scope):** §3 repetition/50-move ✓ (T2 + search prologue T4), §5.1 SearchThread ✓ (T4; killers/history/excluded_move slots arrive with their features in M3 — the struct is private to the crate, extending it is non-breaking), §5.2 M2 subset ✓ (T4; deviations header lists deferred items), §5.4 ✓ (T5 + T8 gauntlet; instability extensions deferred per header), §6.1 seam ✓ (T3, hooks wired in T4's make/unmake sites), §6.2 single-phase subset ✓ (T3), §7 ✓ (T6 + gates; option deviations in header), §10.2 bench ✓ (T7), §12 M2 gates ✓ (T6 UCI tests, T8 forfeit gauntlet + smoke).
- **Carry-ins:** resolver ✓ T1, key-history ✓ T2, smoke match ✓ T8.
- **Type consistency verified:** `Evaluator` signature (T3) matches all T4 call sites; `Limits`/`TimeManager` (T5) match T6's `parse_go`; `IterInfo` fields match `print_info`; `find_uci_move` (T1) used in T6 + tests; `search_to_depth`/`iterate` signatures consistent across T4/T5/T7.
- **Known risks for the executor:** the `parse_go` closure-borrow note (T6 Step 6.1); fastchess log phrasing in the T8 greps may vary by version — if a grep misfires, inspect the log/pgn manually and adjust the pattern minimally, reporting the deviation.

## Execution Handoff

Plan complete. Execute with superpowers:subagent-driven-development (fresh subagent per task, two-stage review) or superpowers:executing-plans (inline with checkpoints).





