# Plan-9: NNUE Datagen Binary — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `datagen` binary that uses NebChess's own search to play self-play games and emit labeled training positions (`FEN | cp_white | wdl_white`) for NNUE training.

**Architecture:** A soft-node search limit (stop at the next depth boundary once a node budget is crossed) is added to the existing iterative-deepening loop. A new dev-only bin (`src/bin/datagen.rs`) plays games from random openings at ~5k soft nodes/move, records quiet non-saturated positions with the engine's white-relative score, labels them with the white-relative game result (natural ending, resign/draw adjudication, or Syzygy TB adjudication), and writes per-worker text shards in parallel with a seeded RNG for reproducibility.

**Tech Stack:** Rust (std-only + pyrrhic-rs), `std::thread::scope`, the engine lib (`nebchess::{board, eval, search, tb}`). No new crate dependencies — the engine stays std-only; conversion/shuffle to bullet's binary format happens later in the offline trainer toolchain (plan-10), not here.

**Spec:** `docs/superpowers/specs/2026-06-08-nnue-design.md` — section "plan-9 (A) — datagen".

**Dependency note:** This sub-project is GPU-free and does **not** depend on the Step-0 bullet/CUDA outcome. It is identical whether the trainer ends up being bullet or pytorch.

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `src/search/limits.rs` | The `Limits` struct | **Modify** — add `pub soft_nodes: Option<u64>` |
| `src/search/mod.rs` | Iterative-deepening loop (`iterate`) | **Modify** — break at the depth boundary once `soft_nodes` is exceeded |
| `src/bin/datagen.rs` | The whole self-play data generator (RNG, opening, game loop, filters, labeling, parallel workers, CLI, a `stats` verify mode) | **Create** |

All datagen logic lives in the bin with inline `#[cfg(test)] mod tests` (run via `cargo test --bin datagen`) — keeping it out of the engine library entirely. Only the soft-node limit touches the library.

**Reference working bin** (import paths, arg-parsing idiom): `src/bin/solve.rs`. If any import path below is off, the canonical re-exports are in `src/board/mod.rs`; the concrete module paths (`nebchess::board::moves`, `::types`, `::movegen`, `::position`) always resolve.

---

## Task 1: Soft-node search limit

**Files:**
- Modify: `src/search/limits.rs` (the `Limits` struct, ~lines 19-30)
- Modify: `src/search/mod.rs` (the `iterate` deepening loop, ~lines 1140-1225)
- Test: inline `#[cfg(test)]` in `src/search/mod.rs` (or `limits.rs`)

**Why bench-safe:** the new field defaults to `None`; the only new code path is `if let Some(sn) = limits.soft_nodes`, which no existing caller sets (UCI `parse_go` doesn't set it, `bench` uses `search_to_depth` not `iterate`). So all existing behavior is bit-identical and **bench stays `54508`**. No SPRT required (no behavior change for any existing path).

- [ ] **Step 1: Add the field to `Limits`**

In `src/search/limits.rs`, add `soft_nodes` to the struct (keep `#[derive(Default, Clone, Debug)]`):

```rust
#[derive(Default, Clone, Debug)]
pub struct Limits {
    pub depth: Option<i32>,
    pub nodes: Option<u64>,
    /// Stop at the NEXT depth-iteration boundary once `self.nodes` exceeds this.
    /// Unlike `nodes` (a hard mid-search cutoff), this lets the current iteration finish.
    pub soft_nodes: Option<u64>,
    pub movetime: Option<u64>,
    pub wtime: Option<u64>,
    pub btime: Option<u64>,
    pub winc: Option<u64>,
    pub binc: Option<u64>,
    pub movestogo: Option<u32>,
    pub infinite: bool,
}
```

- [ ] **Step 2: Write the failing test**

Add to a `#[cfg(test)] mod tests` in `src/search/mod.rs`:

```rust
#[test]
fn soft_node_limit_shortens_search() {
    use crate::board::Position;
    use crate::eval::Hce;
    use crate::search::limits::Limits;

    let small = {
        let mut st = SearchThread::new(Position::startpos(), Hce::new());
        st.iterate(&Limits { soft_nodes: Some(1), ..Limits::default() }, |_| {});
        st.nodes
    };
    let big = {
        let mut st = SearchThread::new(Position::startpos(), Hce::new());
        st.iterate(&Limits { soft_nodes: Some(2_000_000), ..Limits::default() }, |_| {});
        st.nodes
    };
    assert!(small >= 1, "did at least one iteration");
    assert!(small < big, "soft_nodes=1 ({small}) must search far less than soft_nodes=2M ({big})");
}
```

- [ ] **Step 3: Run it — verify it fails**

Run: `cargo test --lib soft_node_limit_shortens_search`
Expected: FAIL — `small < big` is false (both run to the same default max depth because `soft_nodes` is ignored).

- [ ] **Step 4: Implement the soft-node break**

In `src/search/mod.rs`, inside `iterate`'s iterative-deepening loop (`for depth in 1..=max_depth`, ~line 1192), find the existing post-iteration stop check (`tm.past_soft()`, ~line 1221) and add the soft-node break right after it:

```rust
        // ... existing: if tm.past_soft() { break; } (or equivalent stability/time stop)

        // Soft node limit: finish this iteration, then stop before starting a deeper one.
        if let Some(sn) = limits.soft_nodes {
            if self.nodes >= sn {
                break;
            }
        }
```

(Place it so it runs after a completed iteration, alongside the existing soft-time check — not inside `should_stop`, which is the hard, mid-iteration cutoff.)

- [ ] **Step 5: Run it — verify it passes**

Run: `cargo test --lib soft_node_limit_shortens_search`
Expected: PASS.

- [ ] **Step 6: Verify bench is unchanged**

Run: `cargo build --release && ./target/release/nebchess bench | tail -1`
Expected: `Bench: 54508` (the soft-node path is inert for `search_to_depth`/bench).

- [ ] **Step 7: Run the full test battery**

Run: `cargo test`
Expected: all green.

- [ ] **Step 8: Commit**

```bash
git add src/search/limits.rs src/search/mod.rs
git commit -m "feat(search): soft-node limit (datagen support)" \
  -m "Adds Limits.soft_nodes: stop at the next depth boundary once the node budget is crossed (lets the current iteration finish, unlike the hard nodes cutoff). Inert for all existing callers (defaults None; UCI/bench never set it), so behavior and bench are unchanged." \
  -m "Bench: 54508" \
  -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: `datagen` bin scaffold + seeded RNG

**Files:**
- Create: `src/bin/datagen.rs`

- [ ] **Step 1: Create the file with imports, RNG, and a usage stub**

```rust
//! plan-9: self-play training-data generator for NNUE. Dev-only binary.
//! Emits `FEN | cp_white | wdl_white` text shards from engine self-play.
//! Reproducible given (--seed, --threads, --games). No GPU, no new deps.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use nebchess::board::Position;
use nebchess::board::moves::{Move, MoveList};
use nebchess::board::movegen::{find_first_legal, generate_moves};
use nebchess::board::types::Color;
use nebchess::eval::Hce;
use nebchess::search::limits::Limits;
use nebchess::search::SearchThread;
use nebchess::tb::{Tb, Wdl};

/// Scores at or above this magnitude are mate/saturated and are not recorded.
const MATE_THRESHOLD: i32 = 29_000; // mirrors search::MATE_BOUND

/// Seeded SplitMix64 (adapted from src/bin/find_magics.rs).
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    /// Uniform in `[0, n)`. `n` must be > 0.
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

fn main() {
    eprintln!("datagen: see plan-9; run with --help");
}

#[cfg(test)]
mod tests {
    use super::*;
}
```

- [ ] **Step 2: Write the failing RNG determinism test**

Add to `mod tests`:

```rust
#[test]
fn rng_is_deterministic_and_bounded() {
    let a: Vec<u64> = { let mut r = Rng::new(42); (0..5).map(|_| r.next_u64()).collect() };
    let b: Vec<u64> = { let mut r = Rng::new(42); (0..5).map(|_| r.next_u64()).collect() };
    let c: Vec<u64> = { let mut r = Rng::new(43); (0..5).map(|_| r.next_u64()).collect() };
    assert_eq!(a, b, "same seed -> same stream");
    assert_ne!(a, c, "different seed -> different stream");

    let mut r = Rng::new(7);
    for _ in 0..1000 {
        assert!(r.below(13) < 13);
    }
}
```

- [ ] **Step 3: Run it**

Run: `cargo test --bin datagen rng_is_deterministic_and_bounded`
Expected: PASS (the RNG is already implemented in Step 1).

- [ ] **Step 4: Confirm the bin builds**

Run: `cargo build --release --bin datagen`
Expected: builds; `ls target/release/datagen` exists.

- [ ] **Step 5: Commit**

```bash
git add src/bin/datagen.rs
git commit -m "feat(datagen): bin scaffold + seeded SplitMix64 RNG" \
  -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: Random legal move + random opening

**Files:**
- Modify: `src/bin/datagen.rs`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests`:

```rust
#[test]
fn random_legal_move_is_legal_and_deterministic() {
    let pick = |seed: u64| {
        let mut pos = Position::startpos();
        let mut rng = Rng::new(seed);
        random_legal_move(&mut pos, &mut rng)
    };
    let mv = pick(1).expect("startpos has legal moves");
    // Legal: applying it succeeds (make returns true), then restore.
    let mut pos = Position::startpos();
    assert!(pos.make(mv), "returned move must be legal");
    pos.unmake();
    assert_eq!(pick(1), pick(1), "same seed -> same pick");
}

#[test]
fn random_opening_applies_requested_plies() {
    let mut rng = Rng::new(99);
    let pos = play_random_opening(&mut rng, 8).expect("8-ply opening from startpos");
    // 8 half-moves from startpos -> fullmove 5, side to move White.
    assert_eq!(pos.stm(), Color::White);
    assert!(pos.to_fen().split_whitespace().count() >= 6, "valid FEN");

    let mut rng_a = Rng::new(5);
    let mut rng_b = Rng::new(5);
    assert_eq!(
        play_random_opening(&mut rng_a, 8).map(|p| p.to_fen()),
        play_random_opening(&mut rng_b, 8).map(|p| p.to_fen()),
        "same seed -> same opening"
    );
}
```

- [ ] **Step 2: Run them — verify they fail**

Run: `cargo test --bin datagen random_`
Expected: FAIL — `random_legal_move` / `play_random_opening` not defined.

- [ ] **Step 3: Implement**

Add to `src/bin/datagen.rs` (above `main`):

```rust
/// Pick a uniformly-random LEGAL move (generate_moves is pseudo-legal; filter via make/unmake).
fn random_legal_move(pos: &mut Position, rng: &mut Rng) -> Option<Move> {
    let mut pseudo = MoveList::new();
    generate_moves(pos, &mut pseudo);
    let mut legal: Vec<Move> = Vec::with_capacity(pseudo.len());
    for &mv in pseudo.iter() {
        if pos.make(mv) {
            pos.unmake();
            legal.push(mv);
        }
    }
    if legal.is_empty() {
        None
    } else {
        Some(legal[rng.below(legal.len())])
    }
}

/// Play `plies` random legal half-moves from the start position. Returns None if a
/// terminal position (mate/stalemate) is hit during the opening (caller skips the game).
fn play_random_opening(rng: &mut Rng, plies: usize) -> Option<Position> {
    let mut pos = Position::startpos();
    for _ in 0..plies {
        let mv = random_legal_move(&mut pos, rng)?;
        pos.make(mv);
    }
    Some(pos)
}
```

- [ ] **Step 4: Run them — verify they pass**

Run: `cargo test --bin datagen random_`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/bin/datagen.rs
git commit -m "feat(datagen): random legal move + random opening" \
  -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: Game-outcome detection (natural + TB mapping)

**Files:**
- Modify: `src/bin/datagen.rs`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests`:

```rust
#[test]
fn terminal_outcome_detects_endings() {
    // Fool's mate: White to move, checkmated -> Black wins.
    let mut mate = Position::from_fen("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3").unwrap();
    assert_eq!(terminal_outcome(&mut mate), Some(Outcome::BlackWin));

    // Stalemate: Black to move, not in check, no legal moves -> Draw.
    let mut stale = Position::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").unwrap();
    assert_eq!(terminal_outcome(&mut stale), Some(Outcome::Draw));

    // KvK insufficient material -> Draw.
    let mut kvk = Position::from_fen("8/8/4k3/8/8/3K4/8/8 w - - 0 1").unwrap();
    assert_eq!(terminal_outcome(&mut kvk), Some(Outcome::Draw));

    // Start position is ongoing.
    let mut start = Position::startpos();
    assert_eq!(terminal_outcome(&mut start), None);
}

#[test]
fn wdl_maps_to_white_relative_outcome() {
    assert_eq!(wdl_to_outcome(Color::White, Wdl::Win), Outcome::WhiteWin);
    assert_eq!(wdl_to_outcome(Color::Black, Wdl::Win), Outcome::BlackWin);
    assert_eq!(wdl_to_outcome(Color::White, Wdl::Loss), Outcome::BlackWin);
    assert_eq!(wdl_to_outcome(Color::Black, Wdl::Loss), Outcome::WhiteWin);
    assert_eq!(wdl_to_outcome(Color::White, Wdl::Draw), Outcome::Draw);
}
```

- [ ] **Step 2: Run them — verify they fail**

Run: `cargo test --bin datagen _outcome`
Expected: FAIL — `Outcome` / `terminal_outcome` / `wdl_to_outcome` not defined.

- [ ] **Step 3: Implement**

Add to `src/bin/datagen.rs`:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    WhiteWin,
    Draw,
    BlackWin,
}

fn outcome_to_wdl(o: Outcome) -> f32 {
    match o {
        Outcome::WhiteWin => 1.0,
        Outcome::Draw => 0.5,
        Outcome::BlackWin => 0.0,
    }
}

/// Side-to-move-relative TB result -> white-relative outcome.
fn wdl_to_outcome(stm: Color, w: Wdl) -> Outcome {
    match w {
        Wdl::Draw => Outcome::Draw,
        Wdl::Win => if stm == Color::White { Outcome::WhiteWin } else { Outcome::BlackWin },
        Wdl::Loss => if stm == Color::White { Outcome::BlackWin } else { Outcome::WhiteWin },
    }
}

/// Natural game end for the side to move. Mate/stalemate first (decisive/terminal),
/// then the draw rules. Returns None if the game is ongoing.
fn terminal_outcome(pos: &mut Position) -> Option<Outcome> {
    if find_first_legal(pos).is_none() {
        return Some(if pos.in_check(pos.stm()) {
            if pos.stm() == Color::White { Outcome::BlackWin } else { Outcome::WhiteWin }
        } else {
            Outcome::Draw // stalemate
        });
    }
    if pos.is_fifty_move_draw() || pos.is_repetition() || pos.is_insufficient_material() {
        return Some(Outcome::Draw);
    }
    None
}
```

- [ ] **Step 4: Run them — verify they pass**

Run: `cargo test --bin datagen _outcome`
Expected: PASS. (If a FEN assertion fails, re-verify the FEN with `printf 'position fen <FEN>\ngo depth 1\n' | ./target/release/nebchess` — the engine is the source of truth — but these three are standard textbook positions.)

- [ ] **Step 5: Commit**

```bash
git add src/bin/datagen.rs
git commit -m "feat(datagen): game-outcome detection + TB->white-relative mapping" \
  -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5: Position filter + white-relative score

**Files:**
- Modify: `src/bin/datagen.rs`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` (note the extra imports for move flags):

```rust
#[test]
fn cp_white_flips_for_black() {
    assert_eq!(cp_white(Color::White, 30), 30);
    assert_eq!(cp_white(Color::Black, 30), -30);
}

#[test]
fn filter_skips_check_capture_and_saturated() {
    use nebchess::board::moves::{QUIET, CAPTURE};
    use nebchess::board::types::Square;

    let e2 = Square::from_name("e2").unwrap();
    let e4 = Square::from_name("e4").unwrap();
    let quiet = Move::new(e2, e4, QUIET);
    let capture = Move::new(e2, e4, CAPTURE);

    let start = Position::startpos();
    assert!(should_record(&start, quiet, 25), "quiet, in-bounds score -> record");
    assert!(!should_record(&start, capture, 25), "best move is a capture -> skip");
    assert!(!should_record(&start, quiet, 30_000), "saturated/mate score -> skip");

    // Side to move in check -> skip.
    let in_check = Position::from_fen("4k3/8/8/8/7q/8/8/4K3 w - - 0 1").unwrap();
    assert!(!should_record(&in_check, quiet, 25), "stm in check -> skip");
}
```

- [ ] **Step 2: Run them — verify they fail**

Run: `cargo test --bin datagen "cp_white\|filter_"`
Expected: FAIL — `cp_white` / `should_record` not defined.

- [ ] **Step 3: Implement**

Add to `src/bin/datagen.rs`:

```rust
/// Convert a side-to-move-relative centipawn score to white-relative.
fn cp_white(stm: Color, score_cp: i32) -> i32 {
    if stm == Color::White { score_cp } else { -score_cp }
}

/// Smart-fen-skipping: record only QUIET, non-saturated positions where the
/// side to move is not in check (the net learns quiet eval; search handles tactics).
fn should_record(pos: &Position, best: Move, score_cp: i32) -> bool {
    !pos.in_check(pos.stm()) && !best.is_capture() && score_cp.abs() < MATE_THRESHOLD
}
```

- [ ] **Step 4: Run them — verify they pass**

Run: `cargo test --bin datagen "cp_white\|filter_"`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/bin/datagen.rs
git commit -m "feat(datagen): smart-fen-skipping filter + white-relative score" \
  -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 6: Self-play game loop

**Files:**
- Modify: `src/bin/datagen.rs`

- [ ] **Step 1: Add the `Config` struct and `play_game`**

Add to `src/bin/datagen.rs`:

```rust
#[derive(Clone)]
struct Config {
    soft_nodes: u64,
    opening_plies: usize,
    max_plies: usize,
    resign_cp: i32,
    resign_plies: i32,
}

impl Default for Config {
    fn default() -> Config {
        Config { soft_nodes: 5_000, opening_plies: 8, max_plies: 400, resign_cp: 1_000, resign_plies: 8 }
    }
}

/// Play one self-play game; push `(fen, cp_white, wdl_white)` for each kept position.
/// Reuses the caller's SearchThread (and its TT) across games for throughput; this is
/// deterministic per worker (single-threaded) and benign at ~5k nodes.
fn play_game(st: &mut SearchThread<Hce>, rng: &mut Rng, cfg: &Config,
             tb: Option<&Tb>, out: &mut Vec<(String, i32, f32)>) {
    // 1. Random opening (skip the game if it dead-ends during the opening).
    let Some(opening) = play_random_opening(rng, cfg.opening_plies) else { return };
    st.pos = opening;

    // 2. Self-play.
    let mut records: Vec<(String, i32)> = Vec::new();
    let mut outcome: Option<Outcome> = None;
    let mut resign_run = 0i32;

    for _ in 0..cfg.max_plies {
        if let Some(o) = terminal_outcome(&mut st.pos) {
            outcome = Some(o);
            break;
        }
        if let Some(tb) = tb {
            if let Some(w) = tb.probe_wdl(&st.pos) {
                outcome = Some(wdl_to_outcome(st.pos.stm(), w));
                break;
            }
        }

        let limits = Limits {
            soft_nodes: Some(cfg.soft_nodes),
            nodes: Some(cfg.soft_nodes.saturating_mul(8)), // hard safety ceiling
            ..Limits::default()
        };
        let mut score = 0i32;
        let best = st.iterate(&limits, |info| score = info.score);
        let Some(mv) = best else { break };

        if should_record(&st.pos, mv, score) {
            records.push((st.pos.to_fen(), cp_white(st.pos.stm(), score)));
        }

        // Resign adjudication: a sustained large white-relative edge ends the game.
        let wcp = cp_white(st.pos.stm(), score);
        resign_run = if wcp.abs() >= cfg.resign_cp { resign_run + 1 } else { 0 };
        if resign_run >= cfg.resign_plies {
            outcome = Some(if wcp > 0 { Outcome::WhiteWin } else { Outcome::BlackWin });
            break;
        }

        st.pos.make(mv);
    }

    // 3. Label every recorded position with the game result.
    let wdl = outcome_to_wdl(outcome.unwrap_or(Outcome::Draw));
    for (fen, cp) in records {
        out.push((fen, cp, wdl));
    }
}
```

- [ ] **Step 2: Write the test**

Add to `mod tests`:

```rust
#[test]
fn play_game_is_deterministic_and_consistent() {
    let cfg = Config { soft_nodes: 400, opening_plies: 4, max_plies: 60, ..Config::default() };

    let run = |seed: u64| {
        let mut st = SearchThread::new(Position::startpos(), Hce::new());
        let mut rng = Rng::new(seed);
        let mut out = Vec::new();
        play_game(&mut st, &mut rng, &cfg, None, &mut out);
        out
    };

    let a = run(123);
    let b = run(123);
    assert_eq!(a, b, "same seed -> identical game/records");

    // Every record: valid FEN (parses), single shared WDL in {0.0,0.5,1.0}, bounded cp.
    if let Some((_, _, wdl0)) = a.first() {
        for (fen, cp, wdl) in &a {
            assert!(Position::from_fen(fen).is_ok(), "recorded FEN must parse: {fen}");
            assert!(*wdl == 0.0 || *wdl == 0.5 || *wdl == 1.0, "wdl in set");
            assert_eq!(wdl, wdl0, "one game -> one result label");
            assert!(cp.abs() < MATE_THRESHOLD, "no saturated scores recorded");
        }
    }
}
```

- [ ] **Step 3: Run it — verify it passes**

Run: `cargo test --bin datagen play_game_is_deterministic_and_consistent`
Expected: PASS. (If it fails to compile because `SearchThread::new` needs a type annotation, write `SearchThread::<Hce>::new(...)`.)

- [ ] **Step 4: Run the full bin test suite**

Run: `cargo test --bin datagen`
Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add src/bin/datagen.rs
git commit -m "feat(datagen): self-play game loop with resign/TB adjudication" \
  -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 7: Parallel workers, CLI, shard output

**Files:**
- Modify: `src/bin/datagen.rs`

- [ ] **Step 1: Replace `main` with arg parsing + scoped parallel workers**

Replace the stub `main` in `src/bin/datagen.rs` with:

```rust
struct Args {
    out_dir: String,
    games: u64,     // total games across all workers
    threads: usize,
    seed: u64,
    tb_path: Option<String>,
    cfg: Config,
}

fn parse_args() -> Args {
    let mut a = Args {
        out_dir: "tools/data/selfplay".to_string(),
        games: 100_000,
        threads: 22,
        seed: 1,
        tb_path: None,
        cfg: Config::default(),
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let mut val = || { i += 1; argv.get(i).cloned().unwrap_or_default() };
        match argv[i].as_str() {
            "--out" => a.out_dir = val(),
            "--games" => a.games = val().parse().unwrap_or(a.games),
            "--threads" => a.threads = val().parse().unwrap_or(a.threads),
            "--seed" => a.seed = val().parse().unwrap_or(a.seed),
            "--nodes" => a.cfg.soft_nodes = val().parse().unwrap_or(a.cfg.soft_nodes),
            "--opening-plies" => a.cfg.opening_plies = val().parse().unwrap_or(a.cfg.opening_plies),
            "--max-plies" => a.cfg.max_plies = val().parse().unwrap_or(a.cfg.max_plies),
            "--resign-cp" => a.cfg.resign_cp = val().parse().unwrap_or(a.cfg.resign_cp),
            "--resign-plies" => a.cfg.resign_plies = val().parse().unwrap_or(a.cfg.resign_plies),
            "--tb" => a.tb_path = Some(val()),
            "--help" | "-h" => {
                eprintln!("usage: datagen [--out DIR] [--games N] [--threads T] [--seed S] \
                           [--nodes SOFT] [--opening-plies P] [--max-plies M] \
                           [--resign-cp CP] [--resign-plies N] [--tb PATH]");
                std::process::exit(0);
            }
            other => eprintln!("datagen: ignoring unknown arg {other}"),
        }
        i += 1;
    }
    a.threads = a.threads.max(1);
    a
}

fn worker(id: usize, seed: u64, games: u64, cfg: &Config, tb: Option<&Tb>,
          out_dir: &str, total: &AtomicU64) {
    let mut rng = Rng::new(seed);
    let mut st = SearchThread::new(Position::startpos(), Hce::new());
    let path = format!("{out_dir}/shard_{id:02}.txt");
    let mut f = BufWriter::new(File::create(&path).expect("create shard"));
    let mut buf: Vec<(String, i32, f32)> = Vec::new();
    let mut written = 0u64;
    for _ in 0..games {
        buf.clear();
        play_game(&mut st, &mut rng, cfg, tb, &mut buf);
        for (fen, cp, wdl) in &buf {
            writeln!(f, "{fen} | {cp} | {wdl:.1}").expect("write shard");
        }
        written += buf.len() as u64;
    }
    f.flush().expect("flush shard");
    total.fetch_add(written, Ordering::Relaxed);
    eprintln!("worker {id}: {written} positions -> {path}");
}

fn main() {
    let args = parse_args();
    std::fs::create_dir_all(&args.out_dir).expect("create out dir");
    let tb = args.tb_path.as_deref().and_then(Tb::init);
    if args.tb_path.is_some() {
        eprintln!("datagen: TB adjudication {}", if tb.is_some() { "ENABLED" } else { "DISABLED (init failed)" });
    }
    let total = AtomicU64::new(0);
    let t = args.threads;
    let base = args.games / t as u64;
    let rem = args.games % t as u64;

    std::thread::scope(|s| {
        for w in 0..t {
            // Distinct stream per worker; quota fixed -> reproducible given (seed, threads, games).
            let seed = args.seed.wrapping_add((w as u64).wrapping_mul(0x9E3779B97F4A7C15));
            let games = base + if (w as u64) < rem { 1 } else { 0 };
            let (cfg, tb_ref, out_dir, total_ref) = (&args.cfg, tb.as_ref(), &args.out_dir, &total);
            s.spawn(move || worker(w, seed, games, cfg, tb_ref, out_dir, total_ref));
        }
    });
    println!("datagen done: {} positions across {} shards in {}", total.load(Ordering::Relaxed), t, args.out_dir);
}
```

- [ ] **Step 2: Build**

Run: `cargo build --release --bin datagen`
Expected: builds clean (no warnings beyond any pre-existing). If `Tb: Sync` is required by `thread::scope` and fails to compile, that indicates `tb::Tb` isn't `Sync` — in that case wrap as `std::sync::Arc<Tb>` and clone the `Arc` into each closure instead of sharing `&Tb`.

- [ ] **Step 3: Tiny smoke run (no TB)**

Run: `./target/release/datagen --out /tmp/dg_smoke --games 40 --threads 4 --seed 1 --nodes 2000`
Expected: prints per-worker counts and `datagen done: N positions ...` with N > 0; `ls /tmp/dg_smoke` shows `shard_00.txt`..`shard_03.txt`.

- [ ] **Step 4: Eyeball the output format**

Run: `head -3 /tmp/dg_smoke/shard_00.txt`
Expected: lines like `<fen with 6 fields> | <int cp> | <0.0|0.5|1.0>`.

- [ ] **Step 5: Confirm reproducibility**

Run:
```bash
./target/release/datagen --out /tmp/dg_a --games 40 --threads 4 --seed 7 --nodes 2000
./target/release/datagen --out /tmp/dg_b --games 40 --threads 4 --seed 7 --nodes 2000
diff -r /tmp/dg_a /tmp/dg_b && echo REPRODUCIBLE
```
Expected: `REPRODUCIBLE` (identical shards for the same seed/threads/games).

- [ ] **Step 6: Run the test suite + commit**

Run: `cargo test --bin datagen`
Expected: green.

```bash
git add src/bin/datagen.rs
git commit -m "feat(datagen): parallel scoped workers, CLI, shard output" \
  -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 8: `stats` verify mode + the data-quality gate

**Files:**
- Modify: `src/bin/datagen.rs`

- [ ] **Step 1: Add a `stats` subcommand**

This re-reads emitted shards through the engine to prove the data is clean (no in-check positions leaked, WDL/cp distributions sane). Add to `src/bin/datagen.rs` and dispatch it from `main` when `argv[0] == "stats"`:

```rust
fn run_stats(dir: &str) {
    use std::io::{BufRead, BufReader};
    let (mut n, mut in_check, mut bad_fen, mut wins, mut draws, mut losses) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let (mut cp_min, mut cp_max, mut cp_sum) = (i32::MAX, i32::MIN, 0i64);
    for entry in std::fs::read_dir(dir).expect("read out dir") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("txt") { continue; }
        for line in BufReader::new(File::open(&path).unwrap()).lines() {
            let line = line.unwrap();
            let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
            if parts.len() != 3 { continue; }
            n += 1;
            match Position::from_fen(parts[0]) {
                Ok(pos) => if pos.in_check(pos.stm()) { in_check += 1; },
                Err(_) => { bad_fen += 1; continue; }
            }
            if let Ok(cp) = parts[1].parse::<i32>() {
                cp_min = cp_min.min(cp); cp_max = cp_max.max(cp); cp_sum += cp as i64;
            }
            match parts[2] {
                "1.0" => wins += 1, "0.5" => draws += 1, "0.0" => losses += 1, _ => {}
            }
        }
    }
    let pct = |x: u64| if n > 0 { 100.0 * x as f64 / n as f64 } else { 0.0 };
    println!("positions: {n}");
    println!("white W/D/L: {:.1}% / {:.1}% / {:.1}%", pct(wins), pct(draws), pct(losses));
    println!("cp white: min {cp_min} max {cp_max} mean {:.1}", if n > 0 { cp_sum as f64 / n as f64 } else { 0.0 });
    println!("LEAKS -> in-check: {in_check}  bad-fen: {bad_fen}  (both MUST be 0)");
    assert_eq!(in_check, 0, "in-check positions leaked into the data");
    assert_eq!(bad_fen, 0, "unparseable FENs in the data");
}
```

In `main`, before `parse_args`, add:

```rust
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(|s| s.as_str()) == Some("stats") {
        run_stats(argv.get(1).map(|s| s.as_str()).unwrap_or("tools/data/selfplay"));
        return;
    }
```

- [ ] **Step 2: Build + run stats on the smoke data**

Run:
```bash
cargo build --release --bin datagen
./target/release/datagen stats /tmp/dg_smoke
```
Expected: prints counts; **`in-check: 0  bad-fen: 0`**; W/D/L percentages that sum to ~100; a plausible cp range. The asserts pass (process exits 0).

- [ ] **Step 3: Commit**

```bash
git add src/bin/datagen.rs
git commit -m "feat(datagen): stats verify mode (leak + distribution checks)" \
  -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

- [ ] **Step 4: GATE — generate a real sample and review (controller-owned, idle system)**

This is the plan-9 gate (data, not Elo — no SPRT). On an **idle** machine:

```bash
# ~2000 games -> a few tens of thousands of positions; tune --games for the 100M target after.
./target/release/datagen --out tools/data/selfplay-sample --games 2000 --threads 22 --seed 1 --nodes 5000
./target/release/datagen stats tools/data/selfplay-sample
```

Controller checks:
- **`in-check: 0  bad-fen: 0`** (hard requirement — the asserts enforce it).
- **positions-per-game** = positions / 2000 is in a sane band (~roughly 15-50; this calibrates the `--games` needed for the ~100M v1 target).
- **W/D/L** is plausible for self-play (draw-heavy but not ~100% draws; a gross skew signals an adjudication or labeling bug).
- **cp mean** near 0 (symmetric self-play), range not pinned at the saturation bound.

If anything looks off, halt and attribute before scaling up. The full ~100M-position generation run is **plan-10's** input (kick it off once the gate looks clean), not part of this plan.

---

## Self-Review

**Spec coverage** (against `2026-06-08-nnue-design.md` → "plan-9 (A)"):
- soft-node limit prereq → Task 1 ✓
- new bin in `nebchess` crate alongside tune/solve/perft → Task 2 (`src/bin/datagen.rs`, auto-discovered) ✓
- 8 random opening plies, no book → Task 3 (`play_random_opening`, `opening_plies` default 8) ✓
- ~5,000 soft nodes/move → Task 1 + Task 6 (`Config.soft_nodes` default 5000) ✓
- white-relative cp + WDL label → Tasks 4-6 (`cp_white`, `outcome_to_wdl`) ✓
- resign + draw + Syzygy TB adjudication → Task 6 (`resign_run`, `terminal_outcome`, `tb.probe_wdl`) ✓
- smart-fen-skipping (in-check / capture-bestmove / saturated / drop opening plies) → Task 5 (`should_record`) + Task 6 (opening plies never recorded) ✓
- text `FEN | cp_white | wdl_white`, no `bulletformat` dep → Task 7 (`writeln!`) ✓
- ~22-core parallel, per-worker seeded reproducibility → Task 7 (`thread::scope`, per-worker seed, fixed quota) ✓
- gate = sample + sanity-check (counts, distribution, legality, filter rate), not SPRT → Task 8 ✓
- bench discipline on the search-touching commit → Task 1 Step 6 + commit `Bench: 54508` ✓

**Placeholder scan:** no TBD/TODO; every code step has complete code; the only deferred item (the full 100M run) is explicitly plan-10's, not a gap.

**Type consistency:** `Outcome` {WhiteWin,Draw,BlackWin} used consistently across Tasks 4/6/8; `Config` fields (`soft_nodes`,`opening_plies`,`max_plies`,`resign_cp`,`resign_plies`) defined in Task 6 and consumed identically in Task 7; `cp_white`/`should_record`/`terminal_outcome`/`wdl_to_outcome`/`play_random_opening`/`random_legal_move`/`play_game` signatures match across definition and call sites; record tuple `(String, i32, f32)` consistent in `play_game`/`worker`/tests.

**Two known integration risks flagged inline (not blockers):** (1) `SearchThread::new` may need a turbofish `::<Hce>` in tests; (2) if `tb::Tb` isn't `Sync`, share it as `Arc<Tb>` instead of `&Tb` in the scoped workers (Task 7 Step 2 note).

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-08-plan-9-datagen.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, two-stage review (spec + quality) between tasks, fast iteration. Matches the project's settled workflow.

**2. Inline Execution** — execute the tasks in this session with checkpoints for review.

Which approach?
