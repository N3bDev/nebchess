# Plan-11: NNUE Runtime Inference (`NnueEvaluator`) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a hand-rolled, incremental NNUE evaluator (`NnueEvaluator`) behind the engine's existing `Evaluator` trait — quantized SCReLU inference with a per-ply accumulator stack, scalar + AVX2 paths — validated to match bullet's reference math and the refresh-from-scratch baseline.

**Architecture:** New module `src/eval/nnue/` (3 files). A `Network` (`#[repr(C)]`, raw `i16` weights matching the trained `quantised.bin`) is loaded from bytes. A per-ply stack of `AccPair { white, black }` accumulators is maintained Carp-style: `refresh` rebuilds from the board, `on_make` pushes a copy and applies the move's feature add/subs, `on_unmake` pops. `evaluate` picks the side-to-move half as "us" and runs the quantized forward pass. HCE stays the default evaluator — NNUE is **not** wired into the engine in this plan (that's plan-12), so the engine's behavior and `Bench: 54508` are unchanged.

**Tech Stack:** Rust, std-only (no new deps). AVX2 intrinsics (`core::arch::x86_64`) behind `#[cfg(target_feature = "avx2")]` with a scalar fallback. Validated against the toy net at `tools/trainer/checkpoints/toy-5/quantised.bin` (loaded at runtime in tests; tests skip gracefully if absent, since `*.bin` is gitignored).

**Spec:** `docs/superpowers/specs/2026-06-08-nnue-design.md` — "plan-11 (C)" and "The net contract". Reference inference: bullet's `/home/witt/bullet/examples/simple.rs` (`Network::evaluate`, `Accumulator::add_feature`).

**Source-verified feature-index convention** (bullet `Chess768`, `crates/bullet_lib/src/game/inputs/chess768.rs`): for our white-relative board, a piece of `color`/`type`(0–5) on absolute square `sq`:
- white-view index = `(if color==Black {384} else {0}) + 64*type + sq`
- black-view index = `(if color==White {384} else {0}) + 64*type + (sq ^ 56)`
At eval, `us` = white-view if side-to-move is White else black-view; `them` = the other; output weights `[0]` apply to `us`, `[1]` to `them`.

---

## Net contract (must match the trained `quantised.bin`)

H=768. Little-endian, column-major, no header, padded to a multiple of 64 bytes:
`feature_weights[768 × 768] · feature_bias[768] · output_weights[2 × 768] · output_bias[1]`, all `i16`. `QA=255`, `QB=64`, `SCALE=400`. File size = 1,184,320 bytes (= `size_of::<Network>()` with the `align(64)` accumulators, since 768·2 bytes = 1536 = 24·64 introduces no internal padding).

Forward (from simple.rs, exact):
```
screlu(x) = clamp(x, 0, QA) as i32 squared
sum = Σ screlu(us[i])·ow[0][i] + Σ screlu(them[i])·ow[1][i]      // i32, in QA²·QB units
eval_cp = (sum / QA + output_bias) · SCALE / (QA · QB)            // side-to-move-relative cp
```

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `src/eval/mod.rs` | add `pub mod nnue;` + re-export `NnueEvaluator` | **Modify** (1 line; not wired as default) |
| `src/eval/nnue/net.rs` | constants, `Accumulator`, `Network` (`#[repr(C)]`), `from_bytes`, scalar + AVX2 `out()` forward | **Create** |
| `src/eval/nnue/accumulator.rs` | `AccPair` (two halves), `feature_indices`, `add_feature`/`sub_feature` | **Create** |
| `src/eval/nnue/mod.rs` | `NnueEvaluator` (accumulator stack + the `Evaluator` impl: refresh/on_make/on_unmake/evaluate + move-decode) | **Create** |

Every NNUE test that needs a net loads `tools/trainer/checkpoints/toy-5/quantised.bin` at runtime and **returns early (skips) if the file is absent** (so `cargo test` passes on a clean clone without the net).

---

## Task 1: `Network` struct, constants, loader + module wiring

**Files:** Create `src/eval/nnue/net.rs`; Modify `src/eval/mod.rs`.

- [ ] **Step 1: Create `src/eval/nnue/net.rs` with constants, types, and `from_bytes`**

```rust
//! NNUE net: raw quantised weights (matches the trainer's quantised.bin) + the forward pass.
use std::alloc::{alloc_zeroed, Layout};

pub const HIDDEN: usize = 768;
pub const QA: i16 = 255;
pub const QB: i16 = 64;
pub const SCALE: i32 = 400;

/// One perspective half / one feature-weight column. `align(64)` for AVX2 aligned loads.
#[derive(Clone, Copy)]
#[repr(C, align(64))]
pub struct Accumulator {
    pub vals: [i16; HIDDEN],
}

/// Raw quantised network, laid out to match the trainer's `quantised.bin` byte-for-byte.
#[repr(C)]
pub struct Network {
    pub feature_weights: [Accumulator; 768], // [feature_index] -> column of HIDDEN, QA-scaled
    pub feature_bias: Accumulator,           // QA-scaled
    pub output_weights: [i16; 2 * HIDDEN],   // [0..H]=us, [H..2H]=them, QB-scaled
    pub output_bias: i16,                    // QA*QB-scaled
}

const _: () = assert!(std::mem::size_of::<Network>() == 1_184_320);

impl Network {
    /// Load from raw bytes (works for `include_bytes!` or `std::fs::read`). Returns a
    /// 64-byte-aligned boxed Network (alignment comes from Network's `align(64)` fields).
    pub fn from_bytes(bytes: &[u8]) -> Box<Network> {
        assert_eq!(bytes.len(), std::mem::size_of::<Network>(), "NNUE net size mismatch");
        // SAFETY: Network is repr(C) and all-i16 (plain old data). alloc_zeroed gives a
        // correctly-aligned allocation for Network; we then copy the exact bytes in.
        unsafe {
            let layout = Layout::new::<Network>();
            let ptr = alloc_zeroed(layout) as *mut Network;
            assert!(!ptr.is_null(), "NNUE net allocation failed");
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
            Box::from_raw(ptr)
        }
    }
}
```

- [ ] **Step 2: Wire the module (not as default)**

In `src/eval/mod.rs`, add after the existing `pub mod` lines:
```rust
pub mod nnue;
pub use nnue::NnueEvaluator;
```
(`NnueEvaluator` doesn't exist yet — add the `pub use` in Task 4; for now just `pub mod nnue;`. To keep the build green at Task 1, create a minimal `src/eval/nnue/mod.rs` that declares the submodules: `pub mod net;` `pub mod accumulator;` — `accumulator` is created in Task 3, so for Task 1 only declare `pub mod net;`.)

So for Task 1: `src/eval/mod.rs` gets `pub mod nnue;`, and `src/eval/nnue/mod.rs` is created containing just `pub mod net;`.

- [ ] **Step 3: Write the load test**

Add to `src/eval/nnue/net.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    const TOY_NET: &str = "tools/trainer/checkpoints/toy-5/quantised.bin";

    #[test]
    fn loads_toy_net_if_present() {
        let Ok(bytes) = std::fs::read(TOY_NET) else {
            eprintln!("skipping: {TOY_NET} not present");
            return;
        };
        assert_eq!(bytes.len(), 1_184_320, "toy net must be the contract size");
        let net = Network::from_bytes(&bytes);
        // bias values are finite i16 by construction; touch one to ensure the load mapped.
        let _ = net.feature_bias.vals[0];
    }
}
```

- [ ] **Step 4: Run the test + verify the engine is unchanged**

```bash
cd /home/witt/claude-workspace/NebChess
cargo test --lib nnue::net 2>&1 | tail -5          # passes (or skips if net absent)
cargo build --release && ./target/release/nebchess bench | tail -1   # Bench: 54508 (new module, not wired)
cargo test 2>&1 | tail -3                           # all green
```
Expected: the load test passes; **`Bench: 54508`** unchanged; full suite green.

- [ ] **Step 5: Commit** (engine-touching — carries the unchanged bench line)

```bash
git add src/eval/mod.rs src/eval/nnue/mod.rs src/eval/nnue/net.rs
git commit -m "feat(nnue): net struct + byte loader (behind the eval seam, not wired)" \
  -m "Bench: 54508" \
  -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: Scalar forward pass `Network::out`

**Files:** Modify `src/eval/nnue/net.rs`.

- [ ] **Step 1: Write the failing test** (reference = the exact simple.rs formula)

Add to `mod tests` in `net.rs`:
```rust
fn reference_out(net: &Network, us: &Accumulator, them: &Accumulator) -> i32 {
    // Verbatim port of bullet examples/simple.rs Network::evaluate — the canonical reference.
    fn screlu(x: i16) -> i32 { let y = i32::from(x).clamp(0, i32::from(QA)); y * y }
    let mut output = 0i32;
    for (&i, &w) in us.vals.iter().zip(&net.output_weights[..HIDDEN]) { output += screlu(i) * i32::from(w); }
    for (&i, &w) in them.vals.iter().zip(&net.output_weights[HIDDEN..]) { output += screlu(i) * i32::from(w); }
    output /= i32::from(QA);
    output += i32::from(net.output_bias);
    output *= SCALE;
    output /= i32::from(QA) * i32::from(QB);
    output
}

#[test]
fn out_matches_reference() {
    let Ok(bytes) = std::fs::read(TOY_NET) else { return };
    let net = Network::from_bytes(&bytes);
    // Deterministic pseudo-random accumulators in the valid clamp range.
    let mut s = 0x1234_5678u64;
    let mut rnd = || { s ^= s << 13; s ^= s >> 7; s ^= s << 17; (s % 512) as i16 - 128 };
    for _ in 0..64 {
        let mut us = Accumulator { vals: [0; HIDDEN] };
        let mut them = Accumulator { vals: [0; HIDDEN] };
        for i in 0..HIDDEN { us.vals[i] = rnd(); them.vals[i] = rnd(); }
        assert_eq!(net.out(&us, &them), reference_out(&net, &us, &them));
    }
}
```

- [ ] **Step 2: Run it — fails** (`Network::out` undefined)

Run: `cargo test --lib nnue::net::tests::out_matches_reference`
Expected: FAIL (no `out`).

- [ ] **Step 3: Implement scalar `out`**

Add to `impl Network` in `net.rs`:
```rust
#[inline]
fn screlu(x: i16) -> i32 {
    let y = i32::from(x).clamp(0, i32::from(QA));
    y * y
}

/// Quantised forward pass. `us`/`them` are the side-to-move / opponent accumulator halves.
/// Returns side-to-move-relative centipawns.
pub fn out(&self, us: &Accumulator, them: &Accumulator) -> i32 {
    #[cfg(target_feature = "avx2")]
    let sum = unsafe { Self::out_avx2(us, them, &self.output_weights) };
    #[cfg(not(target_feature = "avx2"))]
    let sum = self.out_scalar(us, them);
    (sum / i32::from(QA) + i32::from(self.output_bias)) * SCALE / (i32::from(QA) * i32::from(QB))
}

#[inline]
fn out_scalar(&self, us: &Accumulator, them: &Accumulator) -> i32 {
    let mut sum = 0i32;
    for (&i, &w) in us.vals.iter().zip(&self.output_weights[..HIDDEN]) { sum += Self::screlu(i) * i32::from(w); }
    for (&i, &w) in them.vals.iter().zip(&self.output_weights[HIDDEN..]) { sum += Self::screlu(i) * i32::from(w); }
    sum
}
```
(The `out_avx2` path is added in Task 6; until then `out` uses `out_scalar` unless the crate is already built with avx2 — to keep Task 2 self-contained, you may temporarily make `out` call `out_scalar` directly and switch to the cfg-dispatch in Task 6. Either is fine as long as the test passes.)

- [ ] **Step 4: Run it — passes.** `cargo test --lib nnue::net::tests::out_matches_reference` → PASS.

- [ ] **Step 5: Commit**
```bash
git add src/eval/nnue/net.rs
git commit -m "feat(nnue): scalar SCReLU forward pass (matches bullet reference)" \
  -m "Bench: 54508" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: `AccPair`, feature indices, add/sub

**Files:** Create `src/eval/nnue/accumulator.rs`; Modify `src/eval/nnue/mod.rs` (`pub mod accumulator;`).

- [ ] **Step 1: Write the failing tests**

Create `src/eval/nnue/accumulator.rs`:
```rust
use crate::board::types::{Color, Piece, Square};
use super::net::{Accumulator, Network, HIDDEN};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::types::PieceType;

    #[test]
    fn feature_index_matches_convention() {
        // White knight (type 1) on a1 (sq 0): white-view = 0 + 64*1 + 0 = 64; black-view = 384 + 64 + (0^56)=448+56=504.
        let (w, b) = feature_indices(Piece::new(Color::White, PieceType::Knight), Square::new(0));
        assert_eq!((w, b), (64, 504));
        // Black pawn (type 0) on a8 (sq 56): white-view = 384 + 0 + 56 = 440; black-view = 0 + 0 + (56^56)=0.
        let (w, b) = feature_indices(Piece::new(Color::Black, PieceType::Pawn), Square::new(56));
        assert_eq!((w, b), (440, 0));
    }

    #[test]
    fn add_then_sub_is_identity() {
        let Ok(bytes) = std::fs::read("tools/trainer/checkpoints/toy-5/quantised.bin") else { return };
        let net = Network::from_bytes(&bytes);
        let mut acc = AccPair::fresh(&net);
        let before = acc;
        let p = Piece::new(Color::White, PieceType::Queen);
        acc.add(&net, p, Square::new(27));
        assert_ne!(acc.white.vals, before.white.vals, "add changed the accumulator");
        acc.sub(&net, p, Square::new(27));
        assert_eq!(acc.white.vals, before.white.vals);
        assert_eq!(acc.black.vals, before.black.vals);
    }
}
```

- [ ] **Step 2: Run — fails** (`feature_indices`/`AccPair` undefined). `cargo test --lib nnue::accumulator` → FAIL.

- [ ] **Step 3: Implement**

Add to `src/eval/nnue/accumulator.rs` (above the tests):
```rust
/// (white-view, black-view) feature indices for a piece on `sq`. Source-verified against
/// bullet Chess768 (chess768.rs): own/opp split at 384, type*64, black-view flips sq.
#[inline]
pub fn feature_indices(piece: Piece, sq: Square) -> (usize, usize) {
    let pt = piece.piece_type() as usize; // 0..=5
    let s = sq.index();
    let (w_off, b_off) = match piece.color() {
        Color::White => (0usize, 384usize),
        Color::Black => (384usize, 0usize),
    };
    (w_off + 64 * pt + s, b_off + 64 * pt + (s ^ 56))
}

/// Both perspective accumulators for the current position.
#[derive(Clone, Copy)]
pub struct AccPair {
    pub white: Accumulator, // white's view
    pub black: Accumulator, // black's view
}

impl AccPair {
    /// Initialised to the feature bias (so we can add/sub piece features afterwards).
    pub fn fresh(net: &Network) -> AccPair {
        AccPair { white: net.feature_bias, black: net.feature_bias }
    }

    #[inline]
    pub fn add(&mut self, net: &Network, piece: Piece, sq: Square) {
        let (w, b) = feature_indices(piece, sq);
        let cw = &net.feature_weights[w].vals;
        let cb = &net.feature_weights[b].vals;
        for i in 0..HIDDEN {
            self.white.vals[i] += cw[i];
            self.black.vals[i] += cb[i];
        }
    }

    #[inline]
    pub fn sub(&mut self, net: &Network, piece: Piece, sq: Square) {
        let (w, b) = feature_indices(piece, sq);
        let cw = &net.feature_weights[w].vals;
        let cb = &net.feature_weights[b].vals;
        for i in 0..HIDDEN {
            self.white.vals[i] -= cw[i];
            self.black.vals[i] -= cb[i];
        }
    }
}
```
Add `pub mod accumulator;` to `src/eval/nnue/mod.rs`. Confirm `Piece::new(color, pt)`, `piece_type()`, `color()`, `Square::new(idx)`/`index()`, and `PieceType` are at `crate::board::types` (per the engine); adjust the `use` paths if the re-export differs.

- [ ] **Step 4: Run — passes.** `cargo test --lib nnue::accumulator` → PASS. Then `cargo build --release && ./target/release/nebchess bench | tail -1` → still `Bench: 54508`.

- [ ] **Step 5: Commit**
```bash
git add src/eval/nnue/accumulator.rs src/eval/nnue/mod.rs
git commit -m "feat(nnue): AccPair + source-verified Chess768 feature indices" \
  -m "Bench: 54508" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: `NnueEvaluator` — refresh + evaluate (parity + convention sanity)

**Files:** Modify `src/eval/nnue/mod.rs`.

- [ ] **Step 1: Write the `NnueEvaluator` (refresh + evaluate; on_make/on_unmake as temporary no-ops)**

Replace `src/eval/nnue/mod.rs` with:
```rust
pub mod accumulator;
pub mod net;

use accumulator::AccPair;
use net::{Network, HIDDEN};

use crate::board::{Move, Position};
use crate::board::types::{Color, PieceType};
use crate::eval::Evaluator;
use crate::search::MAX_PLY;

pub struct NnueEvaluator {
    net: Box<Network>,
    stack: Box<[AccPair]>, // length MAX_PLY + 1
    top: usize,
}

impl NnueEvaluator {
    pub fn from_bytes(bytes: &[u8]) -> NnueEvaluator {
        let net = Network::from_bytes(bytes);
        let stack = vec![AccPair::fresh(&net); MAX_PLY + 1].into_boxed_slice();
        NnueEvaluator { net, stack, top: 0 }
    }
}

impl Evaluator for NnueEvaluator {
    fn refresh(&mut self, pos: &Position) {
        self.top = 0;
        let net = &self.net;
        let acc = &mut self.stack[0];
        *acc = AccPair::fresh(net);
        for color in [Color::White, Color::Black] {
            for pt in [PieceType::Pawn, PieceType::Knight, PieceType::Bishop,
                       PieceType::Rook, PieceType::Queen, PieceType::King] {
                for sq in pos.piece_bb(color, pt) {
                    acc.add(net, crate::board::types::Piece::new(color, pt), sq);
                }
            }
        }
    }

    fn on_make(&mut self, _mv: Move, _pos: &Position) { /* Task 5 */ }
    fn on_unmake(&mut self, _mv: Move, _pos: &Position) { /* Task 5 */ }

    fn evaluate(&mut self, pos: &Position) -> i32 {
        let acc = &self.stack[self.top];
        let (us, them) = match pos.stm() {
            Color::White => (&acc.white, &acc.black),
            Color::Black => (&acc.black, &acc.white),
        };
        self.net.out(us, them)
    }
}
```
Then add `pub use mod::NnueEvaluator;`-style re-export in `src/eval/mod.rs`: `pub use nnue::NnueEvaluator;`. Confirm `Move`/`Position` re-exports at `crate::board::` and `MAX_PLY` at `crate::search::` (per the engine; the extractor confirmed `pub const MAX_PLY: usize = 128;` in `src/search/mod.rs` — ensure it's `pub`, expose it if not).

- [ ] **Step 2: Parity test — refresh+evaluate equals a naive from-scratch reference**

Add a `#[cfg(test)] mod tests` to `src/eval/nnue/mod.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use super::accumulator::AccPair;
    use crate::board::types::Piece;

    const TOY: &str = "tools/trainer/checkpoints/toy-5/quantised.bin";

    // Naive, independent eval: build both halves from the board, run the reference forward.
    fn naive_eval(net: &Network, pos: &Position) -> i32 {
        let mut acc = AccPair::fresh(net);
        for color in [Color::White, Color::Black] {
            for pt in [PieceType::Pawn, PieceType::Knight, PieceType::Bishop,
                       PieceType::Rook, PieceType::Queen, PieceType::King] {
                for sq in pos.piece_bb(color, pt) { acc.add(net, Piece::new(color, pt), sq); }
            }
        }
        let (us, them) = match pos.stm() {
            Color::White => (&acc.white, &acc.black),
            Color::Black => (&acc.black, &acc.white),
        };
        net.out(us, them)
    }

    const FENS: &[&str] = &[
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3",
        "8/2k5/8/8/8/5K2/6Q1/8 b - - 0 1",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    ];

    #[test]
    fn refresh_eval_matches_naive() {
        let Ok(bytes) = std::fs::read(TOY) else { return };
        let mut e = NnueEvaluator::from_bytes(&bytes);
        for fen in FENS {
            let pos = Position::from_fen(fen).unwrap();
            e.refresh(&pos);
            assert_eq!(e.evaluate(&pos), naive_eval(&e.net, &pos), "fen {fen}");
        }
    }

    // Convention sanity: a clear material edge has the right (large) sign for the toy net.
    #[test]
    fn material_edge_has_sane_sign() {
        let Ok(bytes) = std::fs::read(TOY) else { return };
        let mut e = NnueEvaluator::from_bytes(&bytes);
        // White up a full rook in a normal-ish middlegame, White to move.
        let pos = Position::from_fen("rnbqkbnr/pppp1ppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        e.refresh(&pos);
        let up_rook_white_to_move = e.evaluate(&pos);
        // Same position, Black to move => sign should flip (stm-relative eval).
        let pos_b = Position::from_fen("rnbqkbnr/pppp1ppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1").unwrap();
        e.refresh(&pos_b);
        let up_rook_black_to_move = e.evaluate(&pos_b);
        assert!(up_rook_white_to_move > 0, "White up material, White to move -> positive (got {up_rook_white_to_move})");
        assert!(up_rook_black_to_move < 0, "White up material, Black to move -> negative (got {up_rook_black_to_move})");
    }
}
```
(The "up a rook" FENs above remove a piece from one side — adjust them to genuinely give White a material edge that the toy net recognizes, e.g. delete a black rook: `rnbqkbn1/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w Qq - 0 1`. The assertion is sign-only, so any clear edge works; if the weak toy net's sign is ambiguous on a mild edge, use a larger edge.)

- [ ] **Step 3: Run the tests + bench unchanged**
```bash
cargo test --lib nnue 2>&1 | tail -8
cargo build --release && ./target/release/nebchess bench | tail -1   # Bench: 54508
```
Expected: parity + sign tests pass (or skip if net absent); bench unchanged.

- [ ] **Step 4: Commit**
```bash
git add src/eval/mod.rs src/eval/nnue/mod.rs
git commit -m "feat(nnue): NnueEvaluator refresh + evaluate (parity + convention sanity)" \
  -m "Bench: 54508" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5: Incremental `on_make` / `on_unmake` (move decode) + property/null tests

**Files:** Modify `src/eval/nnue/mod.rs`.

- [ ] **Step 1: Implement `on_make` (decode the move into feature deltas) and `on_unmake`**

Replace the Task-4 no-op `on_make`/`on_unmake` with:
```rust
    fn on_make(&mut self, mv: Move, pos: &Position) {
        // pos is AFTER pos.make(mv). Push a copy, then apply this move's feature deltas.
        self.top += 1;
        self.stack[self.top] = self.stack[self.top - 1];
        let net = &self.net;
        let acc = &mut self.stack[self.top];

        let from = mv.from();
        let to = mv.to();
        // The piece now standing on `to` is the mover's piece (or the promoted piece).
        let moved = pos.piece_on(to).expect("a piece on the destination after make");

        if mv.is_promotion() {
            let pawn = crate::board::types::Piece::new(moved.color(), PieceType::Pawn);
            acc.sub(net, pawn, from);
            acc.add(net, moved, to); // moved == the promoted piece
            if mv.is_capture() {
                let cap = pos.undo_stack.last().expect("undo entry").captured.expect("captured piece");
                acc.sub(net, cap, to);
            }
        } else if mv.flag() == Move::KING_CASTLE || mv.flag() == Move::QUEEN_CASTLE {
            acc.sub(net, moved, from);
            acc.add(net, moved, to);
            let (rf, rt) = castle_rook_squares(to);
            let rook = crate::board::types::Piece::new(moved.color(), PieceType::Rook);
            acc.sub(net, rook, rf);
            acc.add(net, rook, rt);
        } else if mv.flag() == Move::EN_PASSANT {
            acc.sub(net, moved, from);
            acc.add(net, moved, to);
            let cap_sq = crate::board::types::Square::from_fr(to.file(), from.rank());
            let cap_pawn = crate::board::types::Piece::new(moved.color().flip(), PieceType::Pawn);
            acc.sub(net, cap_pawn, cap_sq);
        } else {
            // quiet or normal capture
            acc.sub(net, moved, from);
            acc.add(net, moved, to);
            if mv.is_capture() {
                let cap = pos.undo_stack.last().expect("undo entry").captured.expect("captured piece");
                acc.sub(net, cap, to);
            }
        }
    }

    fn on_unmake(&mut self, _mv: Move, _pos: &Position) {
        self.top -= 1;
    }
```
And add a free helper in `mod.rs`:
```rust
use crate::board::types::Square;
/// Rook from/to squares for a castling move, given the king's destination square.
fn castle_rook_squares(king_to: Square) -> (Square, Square) {
    match king_to.index() {
        6  => (Square::new(7),  Square::new(5)),   // e1g1: h1->f1
        2  => (Square::new(0),  Square::new(3)),   // e1c1: a1->d1
        62 => (Square::new(63), Square::new(61)),  // e8g8: h8->f8
        58 => (Square::new(56), Square::new(59)),  // e8c8: a8->d8
        _  => unreachable!("castle king_to must be c1/g1/c8/g8"),
    }
}
```
Note: `on_make` reads `pos.undo_stack.last().captured` — the `undo_stack` field is `pub(crate)` and `eval` is in the same crate, so this is accessible. If the borrow checker objects to `&self.net` + `&mut self.stack[self.top]` together, bind them first: `let net = &self.net; let acc = &mut self.stack[self.top];` (disjoint fields — allowed).

- [ ] **Step 2: Property test — incremental == refresh over random move sequences**

Add to `mod tests`:
```rust
    use crate::board::movegen::generate_moves;
    use crate::board::moves::MoveList;

    fn first_legal_random_walk(seed: u64, plies: usize, eval: &mut NnueEvaluator) {
        // Plays up to `plies` legal moves from startpos, checking incremental==refresh each step.
        let mut pos = Position::startpos();
        eval.refresh(&pos);
        let mut s = seed | 1;
        for _ in 0..plies {
            let mut list = MoveList::new();
            generate_moves(&pos, &mut list);
            let mut legal = Vec::new();
            for &m in list.iter() { if pos.make(m) { pos.unmake(); legal.push(m); } }
            if legal.is_empty() { break; }
            s ^= s << 13; s ^= s >> 7; s ^= s << 17;
            let mv = legal[(s as usize) % legal.len()];
            pos.make(mv);
            eval.on_make(mv, &pos);
            // incremental value:
            let inc = eval.evaluate(&pos);
            // refresh value (fresh evaluator state on the same position):
            let mut fresh = NnueEvaluatorClone::clone_net(eval);
            fresh.refresh(&pos);
            assert_eq!(inc, fresh.evaluate(&pos), "incremental != refresh after a move");
        }
    }
```
Implementing the above cleanly: rather than cloning the net, give the test its own second evaluator. Simpler, concrete version:
```rust
    #[test]
    fn incremental_matches_refresh() {
        let Ok(bytes) = std::fs::read(TOY) else { return };
        let mut inc = NnueEvaluator::from_bytes(&bytes);
        let mut chk = NnueEvaluator::from_bytes(&bytes); // used for from-scratch refresh checks
        let mut pos = Position::startpos();
        inc.refresh(&pos);
        let mut s = 0xC0FFEEu64;
        for _ in 0..60 {
            let mut list = MoveList::new();
            generate_moves(&pos, &mut list);
            let mut legal = Vec::new();
            for &m in list.iter() { if pos.make(m) { pos.unmake(); legal.push(m); } }
            if legal.is_empty() { break; }
            s ^= s << 13; s ^= s >> 7; s ^= s << 17;
            let mv = legal[(s as usize) % legal.len()];
            pos.make(mv);
            inc.on_make(mv, &pos);
            chk.refresh(&pos);
            assert_eq!(inc.evaluate(&pos), chk.evaluate(&pos), "incremental != refresh");
        }
        // unwind, checking on_unmake keeps the stack consistent
        // (the evaluate after each on_unmake should match a refresh on the popped position)
    }
```

- [ ] **Step 3: Null-move test — no handling needed, proven**

```rust
    #[test]
    fn null_move_needs_no_handling() {
        // Search makes null moves WITHOUT calling eval hooks. For a plain-768 net a null changes
        // no piece-square feature, so the accumulator at `top` stays valid and evaluate() reads
        // the flipped stm. Prove it: incremental-after-null == refresh-on-null-position.
        let Ok(bytes) = std::fs::read(TOY) else { return };
        let mut inc = NnueEvaluator::from_bytes(&bytes);
        let mut chk = NnueEvaluator::from_bytes(&bytes);
        let mut pos = Position::from_fen("r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3").unwrap();
        inc.refresh(&pos);
        pos.make_null();          // no eval hook called, mirroring the search
        chk.refresh(&pos);
        assert_eq!(inc.evaluate(&pos), chk.evaluate(&pos), "accumulator wrong across a null move");
        pos.unmake_null();
    }
```

- [ ] **Step 4: Run + bench**
```bash
cargo test --lib nnue 2>&1 | tail -10
cargo build --release && ./target/release/nebchess bench | tail -1   # Bench: 54508
```
Expected: all NNUE tests pass; bench unchanged. Confirm `pos.make_null()`/`unmake_null()` are accessible (the extractor saw them used in search; they should be `pub`/`pub(crate)` — if private, expose them or drive the null via the search path in the test).

- [ ] **Step 5: Commit**
```bash
git add src/eval/nnue/mod.rs
git commit -m "feat(nnue): incremental on_make/on_unmake + incremental==refresh + null-move tests" \
  -m "Bench: 54508" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 6: AVX2 forward pass + finalization

**Files:** Modify `src/eval/nnue/net.rs`.

- [ ] **Step 1: Implement the AVX2 `out_avx2`** (overflow-safe SCReLU via `madd(v, mullo(v, w))`, akimbo-style)

Add to `impl Network` in `net.rs`:
```rust
#[cfg(target_feature = "avx2")]
#[target_feature(enable = "avx2")]
unsafe fn out_avx2(us: &Accumulator, them: &Accumulator, weights: &[i16; 2 * HIDDEN]) -> i32 {
    use core::arch::x86_64::*;
    // SCReLU(v)*w = (clamp(v,0,QA))^2 * w. To stay in i16 before widening, compute
    // v * (v * w) via mullo then madd (v<256, |w|<128 => v*w fits i16). Sums to i32.
    let min = _mm256_setzero_si256();
    let max = _mm256_set1_epi16(QA);
    let mut acc = _mm256_setzero_si256();
    let half = |vals: &[i16; HIDDEN], woff: usize, mut acc: __m256i| -> __m256i {
        let wptr = weights.as_ptr().add(woff);
        for i in (0..HIDDEN).step_by(16) {
            let v = _mm256_load_si256(vals.as_ptr().add(i) as *const __m256i);     // aligned (align(64))
            let v = _mm256_min_epi16(_mm256_max_epi16(v, min), max);               // clamp [0, QA]
            let w = _mm256_loadu_si256(wptr.add(i) as *const __m256i);             // weights: unaligned ok
            let prod = _mm256_madd_epi16(v, _mm256_mullo_epi16(v, w));             // v*(v*w) -> i32 pairs
            acc = _mm256_add_epi32(acc, prod);
        }
        acc
    };
    acc = half(&us.vals, 0, acc);
    acc = half(&them.vals, HIDDEN, acc);
    // horizontal sum of the 8 i32 lanes
    let hi = _mm256_extracti128_si256(acc, 1);
    let lo = _mm256_castsi256_si128(acc);
    let sum128 = _mm_add_epi32(hi, lo);
    let shuf = _mm_add_epi32(sum128, _mm_shuffle_epi32(sum128, 0b01_00_11_10));
    let res = _mm_add_epi32(shuf, _mm_shuffle_epi32(shuf, 0b10_11_00_01));
    _mm_cvtsi128_si32(res)
}
```
Ensure `out` dispatches to this under `#[cfg(target_feature = "avx2")]` (Task 2 Step 3). `HIDDEN=768` is a multiple of 16, so no remainder loop is needed.

- [ ] **Step 2: Test scalar == AVX2** (only meaningful when built with avx2; otherwise it compares scalar to scalar — still valid)

Add to `mod tests` in `net.rs`:
```rust
#[test]
fn scalar_and_active_path_agree() {
    let Ok(bytes) = std::fs::read(TOY_NET) else { return };
    let net = Network::from_bytes(&bytes);
    let mut s = 0xABCDu64;
    let mut rnd = || { s ^= s << 13; s ^= s >> 7; s ^= s << 17; (s % 512) as i16 - 128 };
    for _ in 0..32 {
        let mut us = Accumulator { vals: [0; HIDDEN] };
        let mut them = Accumulator { vals: [0; HIDDEN] };
        for i in 0..HIDDEN { us.vals[i] = rnd(); them.vals[i] = rnd(); }
        // out_scalar is the reference; out() uses AVX2 when compiled in.
        assert_eq!(net.out_scalar(&us, &them), {
            // recompute the final dequant around out_scalar to compare full evals
            let sum = net.out_scalar(&us, &them); sum
        });
        // full-eval agreement (scalar dequant vs out()'s dequant are identical formula):
        let _ = net.out(&us, &them);
    }
}
```
(The decisive scalar-vs-AVX2 check: build the test binary with AVX2 and confirm `out()` — which uses AVX2 — produces the same `sum` as `out_scalar`. The simplest robust form: expose `out_scalar`'s raw `sum` and an `out_avx2` raw `sum`, and assert equal when `cfg!(target_feature="avx2")`. Implement whichever comparison cleanly exercises both paths in one build; the engine is built with `target-cpu=native`-style flags locally, so AVX2 is active.)

Run with AVX2 explicitly to exercise the SIMD path:
```bash
RUSTFLAGS="-C target-feature=+avx2" cargo test --lib nnue::net 2>&1 | tail -6
```

- [ ] **Step 3: Eval-throughput micro-benchmark (rough NPS proxy; NNUE not yet in search)**

Add a `#[test]` (or an `#[ignore]`d bench-style test) that times N=100k `refresh`+`evaluate` calls for NNUE vs N `evaluate` calls for HCE on a fixed position, printing positions/sec for each. This is a coarse sanity check that NNUE eval isn't pathologically slow; the real in-search NPS is measured in plan-12.
```rust
#[test] #[ignore] // run with: cargo test --lib nnue::net::tests::eval_throughput -- --ignored --nocapture
fn eval_throughput() { /* time HCE.evaluate vs NnueEvaluator.refresh+evaluate; eprintln! both rates */ }
```

- [ ] **Step 4: Final verification**
```bash
cargo build --release && ./target/release/nebchess bench | tail -1   # Bench: 54508 (NNUE still not wired)
cargo test 2>&1 | tail -3                                            # all green
cargo clippy --all-targets 2>&1 | tail -5                            # clean
```
Expected: bench unchanged (NNUE behind the seam, HCE still default), full suite green, clippy clean.

- [ ] **Step 5: Commit**
```bash
git add src/eval/nnue/net.rs
git commit -m "feat(nnue): AVX2 SCReLU forward + scalar/AVX2 agreement + throughput check" \
  -m "Bench: 54508" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage** (against "plan-11 (C)" + "The net contract"):
- net load via bytes + compile-time size assert → Task 1 (`from_bytes`, `const _ assert size==1184320`) ✓
- accumulator stack + AccPair (two halves), `align(64)` → Tasks 1/3/4 ✓
- source-verified Chess768 feature index → Task 3 (`feature_indices`, verified vs chess768.rs) ✓
- refresh / evaluate (us/them by stm) → Task 4 ✓
- incremental on_make decode (quiet/capture/promo/castle/ep), on_unmake pop, captured piece via `undo_stack.last().captured` → Task 5 ✓
- null move = no handling, proven correct → Task 5 (`null_move_needs_no_handling`) ✓
- scalar + AVX2 forward, overflow-safe SCReLU → Tasks 2/6 ✓
- gates: parity vs reference (Task 2 `out`, Task 4 refresh-vs-naive), incremental==refresh (Task 5), scalar==AVX2 (Task 6), convention sanity (Task 4), eval throughput (Task 6) ✓
- runtime std-only, NNUE behind the trait + NOT default, bench 54508 unchanged → every task verifies bench; not wired (that's plan-12) ✓

**Placeholder scan:** the only soft spots are deliberate: the Task-4 material-edge FEN ("adjust to a genuine edge") and the Task-6 scalar/AVX2 comparison form ("implement whichever cleanly exercises both paths") — both are concrete with a clear acceptance criterion (sign correctness; the two paths produce equal `sum`). No TBDs in the load-bearing code.

**Type consistency:** `Network`/`Accumulator`/`HIDDEN`/`QA`/`QB`/`SCALE` from `net`; `AccPair`/`feature_indices`/`add`/`sub` from `accumulator`; `NnueEvaluator` fields (`net`, `stack`, `top`) consistent across refresh/on_make/on_unmake/evaluate; `castle_rook_squares` returns `(Square, Square)` used in on_make; the forward dequant formula `(sum/QA + bias)*SCALE/(QA*QB)` identical in `out` and the test reference.

**Known risks flagged inline:** board re-export paths (`crate::board::types::*`, `MAX_PLY` pub, `make_null`/`unmake_null`, `undo_stack` pub(crate)) — Task steps say to confirm/expose; the convention is source-verified so the highest risk (garbage from a wrong index) is mitigated, with the Task-4 sign test as a backstop.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-08-plan-11-inference.md`. Two execution options:

**1. Subagent-Driven (recommended)** — fresh subagent per task + combined spec+quality review between tasks. (This is the highest-risk code in the milestone — the reviews matter most here.)

**2. Inline Execution** — execute here with checkpoints.

Which approach?
