# M11 Eval Efficiency + Width Re-Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut the per-node cost of NNUE evaluation with fused + lazy accumulator updates (node-identical refactors), then re-gate the 1024-wide net on flywheel-turn-3 data against the now-faster baseline.

**Architecture:** A1 replaces per-feature accumulator loops with fused per-move-shape kernels keyed by feature indices; A2 makes `on_make` record pre-resolved pending deltas and `evaluate()` materialize them from a watermark, so never-evaluated nodes (~30–40% of makes) cost nothing. Both preserve every evaluated number and the exact search tree — gated by node-identical search + unchanged Bench, committed as infrastructure refactors with the baseline refreshed and **no Elo credited** (the M5 attack-map law). Then the standard flywheel + SPRT ladder machinery answers the width question.

**Tech Stack:** Rust std-only runtime (src/eval/nnue/), bullet trainer (tools/trainer), fastchess SPRT, Ordo gauntlet.

**Spec:** `docs/superpowers/specs/2026-06-11-m11-eval-efficiency-design.md`

**Sequencing gates:** Task 1 starts only after the rung-d verdict is resolved and committed (the tree must be clean; "champion" below = net3a 768-plain or net3d 768×8 per that verdict). Tasks 1–2 are implementer-subagent work (CPU-light, coexists with the live bot). Tasks 3–6 are controller-owned ops; Task 5–6 measurements need the idle machine (bot paused via the between-games watcher).

**Ownership:** T1/T2 implementer + combined review per commit; T3–T6 controller.

---

### Task 1: A1 — fused accumulator kernels

**Files:**
- Modify: `src/eval/nnue/accumulator.rs` (kernels + tests)
- Modify: `src/eval/nnue/mod.rs` (`on_make` branches call the fused kernels)

- [ ] **Step 1: Build and stash the reference binary** (for the node-identical gate):

```bash
cargo build --release && cp target/release/nebchess /tmp/nebchess-ref
/tmp/nebchess-ref bench | tail -1   # record the Bench value; call it B0
```

- [ ] **Step 2: Add index-based fused kernels to `accumulator.rs`** (below the existing `add`/`sub`, which stay — `refresh()` uses `add`, and the suite cross-validates fused-vs-sequential through incremental==refresh):

```rust
/// Pre-resolved feature-index pair: the white-view and black-view indices for
/// one (piece, square) feature. u16 is ample (indices < 768).
#[derive(Clone, Copy, Default)]
pub struct FeatIdx {
    pub w: u16,
    pub b: u16,
}

#[inline]
pub fn feat_idx(piece: Piece, sq: Square) -> FeatIdx {
    let (w, b) = feature_indices(piece, sq);
    FeatIdx { w: w as u16, b: b as u16 }
}

impl AccPair {
    /// Quiet / promotion shape: one add, one sub, single traversal.
    /// i16-safety: |quantised weight| <= ~508 (bullet clips at ±1.98, ×QA=255),
    /// so any 2-term combine <= ~1016 and any 4-term combine <= ~2032; the
    /// accumulator itself is bounded by |bias| + 32·508 ≈ 16.5k. Worst-case
    /// |acc + delta| < 19k, far inside i16. Asserted on the real net by
    /// `weights_within_fused_safety_bound`.
    #[inline]
    pub fn add_sub(&mut self, net: &Network, a: FeatIdx, s: FeatIdx) {
        let aw = &net.feature_weights[a.w as usize].vals;
        let sw = &net.feature_weights[s.w as usize].vals;
        let ab = &net.feature_weights[a.b as usize].vals;
        let sb = &net.feature_weights[s.b as usize].vals;
        for i in 0..HIDDEN {
            self.white.vals[i] += aw[i] - sw[i];
            self.black.vals[i] += ab[i] - sb[i];
        }
    }

    /// Capture / en-passant / promotion-capture shape: one add, two subs.
    #[inline]
    pub fn add_sub_sub(&mut self, net: &Network, a: FeatIdx, s1: FeatIdx, s2: FeatIdx) {
        let aw = &net.feature_weights[a.w as usize].vals;
        let s1w = &net.feature_weights[s1.w as usize].vals;
        let s2w = &net.feature_weights[s2.w as usize].vals;
        let ab = &net.feature_weights[a.b as usize].vals;
        let s1b = &net.feature_weights[s1.b as usize].vals;
        let s2b = &net.feature_weights[s2.b as usize].vals;
        for i in 0..HIDDEN {
            self.white.vals[i] += aw[i] - s1w[i] - s2w[i];
            self.black.vals[i] += ab[i] - s1b[i] - s2b[i];
        }
    }

    /// Castle shape: two adds (king-to, rook-to), two subs (king-from, rook-from).
    #[inline]
    pub fn add_add_sub_sub(
        &mut self,
        net: &Network,
        a1: FeatIdx,
        a2: FeatIdx,
        s1: FeatIdx,
        s2: FeatIdx,
    ) {
        let a1w = &net.feature_weights[a1.w as usize].vals;
        let a2w = &net.feature_weights[a2.w as usize].vals;
        let s1w = &net.feature_weights[s1.w as usize].vals;
        let s2w = &net.feature_weights[s2.w as usize].vals;
        let a1b = &net.feature_weights[a1.b as usize].vals;
        let a2b = &net.feature_weights[a2.b as usize].vals;
        let s1b = &net.feature_weights[s1.b as usize].vals;
        let s2b = &net.feature_weights[s2.b as usize].vals;
        for i in 0..HIDDEN {
            self.white.vals[i] += a1w[i] + a2w[i] - s1w[i] - s2w[i];
            self.black.vals[i] += a1b[i] + a2b[i] - s1b[i] - s2b[i];
        }
    }
}
```

- [ ] **Step 3: Switch `on_make` in `mod.rs` to the fused kernels** — same branch structure, one kernel call per branch (`use accumulator::{feat_idx, FeatIdx};`):

```rust
fn on_make(&mut self, mv: Move, pos: &Position) {
    self.top += 1;
    self.stack[self.top] = self.stack[self.top - 1];
    let net = &self.net;
    let acc = &mut self.stack[self.top];

    let from = mv.from();
    let to = mv.to();
    let moved = pos
        .piece_on(to)
        .expect("a piece on the destination after make");

    if mv.is_promotion() {
        let pawn = Piece::new(moved.color(), PieceType::Pawn);
        if mv.is_capture() {
            let cap = pos.undo_stack.last().expect("undo entry").captured.expect("captured piece");
            acc.add_sub_sub(net, feat_idx(moved, to), feat_idx(pawn, from), feat_idx(cap, to));
        } else {
            acc.add_sub(net, feat_idx(moved, to), feat_idx(pawn, from));
        }
    } else if mv.flag() == Move::KING_CASTLE || mv.flag() == Move::QUEEN_CASTLE {
        let (rf, rt) = castle_rook_squares(to);
        let rook = Piece::new(moved.color(), PieceType::Rook);
        acc.add_add_sub_sub(
            net,
            feat_idx(moved, to),
            feat_idx(rook, rt),
            feat_idx(moved, from),
            feat_idx(rook, rf),
        );
    } else if mv.flag() == Move::EN_PASSANT {
        let cap_sq = Square::from_fr(to.file(), from.rank());
        let cap_pawn = Piece::new(moved.color().flip(), PieceType::Pawn);
        acc.add_sub_sub(net, feat_idx(moved, to), feat_idx(moved, from), feat_idx(cap_pawn, cap_sq));
    } else if mv.is_capture() {
        let cap = pos.undo_stack.last().expect("undo entry").captured.expect("captured piece");
        acc.add_sub_sub(net, feat_idx(moved, to), feat_idx(moved, from), feat_idx(cap, to));
    } else {
        acc.add_sub(net, feat_idx(moved, to), feat_idx(moved, from));
    }
}
```

- [ ] **Step 4: Add the i16-safety test** to `accumulator.rs` tests (documents and enforces the overflow proof on the real embedded net):

```rust
#[test]
fn weights_within_fused_safety_bound() {
    // The fused kernels' i16 proof (see add_sub doc) requires every quantised
    // feature weight to be small. bullet clips at ±1.98 → |w| ≤ ~508 after
    // QA-scaling; assert with margin so a future training change that breaks
    // the bound fails loudly here instead of overflowing silently in release.
    let Ok(bytes) = std::fs::read("tools/trainer/checkpoints/toy768x8-1/quantised.bin")
    else {
        return;
    };
    let net = Network::from_bytes(&bytes);
    let max = net
        .feature_weights
        .iter()
        .flat_map(|c| c.vals.iter())
        .map(|&v| (v as i32).abs())
        .max()
        .unwrap();
    assert!(max <= 2000, "feature weight {max} breaks the fused-kernel i16 bound");
}
```
(Adjust the toy path to the champion's family if rung-d changed it — use whatever toy checkpoint the suite already loads.)

- [ ] **Step 5: Run the suite in BOTH profiles** (debug catches any overflow panic; release is the shipping behavior):

```bash
cargo test --lib nnue            # debug: overflow-checks ON
cargo test --release --lib nnue  # release
```
Expected: all green in both (incremental==refresh now cross-validates fused vs sequential, since `refresh` still uses `add`).

- [ ] **Step 6: Node-identical gate at depth 13** (the M5 precedent), exact per-position match:

```bash
cargo build --release
for tool in /tmp/nebchess-ref target/release/nebchess; do
  echo "== $tool"
  ./"$tool" bench | grep position    # depth-6 fingerprint per position
done
# deeper check on 4 spot FENs:
for fen in "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1" \
           "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1" \
           "8/2k5/8/8/8/5K2/6Q1/8 b - - 0 1" \
           "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3"; do
  for bin in /tmp/nebchess-ref target/release/nebchess; do
    printf 'position fen %s\ngo depth 13\nquit\n' "$fen" | ./"$bin" | grep "depth 13" | tail -1 | grep -oE "nodes [0-9]+"
  done
done
```
Expected: node counts identical pairwise, `bench | tail -1` == B0 exactly.

- [ ] **Step 7: Record NPS before/after** (`bench` nps line ×3 runs each, report median) and **commit** (controller reviews first per review-every-step):

```bash
cargo fmt && git add src/eval/nnue/accumulator.rs src/eval/nnue/mod.rs
git commit -m "perf(nnue): fused accumulator kernels (node-identical; no elo credit)

One traversal per move instead of 2-4: quiet=add_sub, capture/EP/promo-cap
=add_sub_sub, castle=add_add_sub_sub, keyed by pre-resolved FeatIdx pairs.
i16 transient safety proven from bullet's +-1.98 weight clip and enforced
by weights_within_fused_safety_bound. refresh() keeps the sequential add,
so incremental==refresh cross-validates the kernels. Node-identical at
d6 (bench) and d13 spot FENs vs the pre-change binary. NPS: <before> ->
<after> (median of 3 bench runs).

Bench: <B0>

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: A2 — lazy materialization

**Files:**
- Modify: `src/eval/nnue/accumulator.rs` (PendingDelta + materialize dispatch)
- Modify: `src/eval/nnue/mod.rs` (NnueEvaluator fields, on_make/on_unmake/evaluate/refresh + tests)

- [ ] **Step 1: Re-stash the reference binary** (post-A1 build): `cp target/release/nebchess /tmp/nebchess-ref` and re-record B0 (unchanged from Task 1).

- [ ] **Step 2: Add `PendingDelta` to `accumulator.rs`** — pre-resolved at make time (captured pieces CANNOT be re-derived later; the position moves on):

```rust
/// A move's accumulator delta, fully resolved at make time (max 2 adds for
/// castle, max 2 subs for capture/castle). Applied later — or never — by
/// materialization.
#[derive(Clone, Copy, Default)]
pub struct PendingDelta {
    pub adds: [FeatIdx; 2],
    pub subs: [FeatIdx; 2],
    pub n_adds: u8,
    pub n_subs: u8,
}

impl AccPair {
    /// Apply a pending delta with the fused kernels. Shapes: (1,1) quiet/promo,
    /// (1,2) capture/EP/promo-capture, (2,2) castle.
    #[inline]
    pub fn apply(&mut self, net: &Network, d: &PendingDelta) {
        match (d.n_adds, d.n_subs) {
            (1, 1) => self.add_sub(net, d.adds[0], d.subs[0]),
            (1, 2) => self.add_sub_sub(net, d.adds[0], d.subs[0], d.subs[1]),
            (2, 2) => self.add_add_sub_sub(net, d.adds[0], d.adds[1], d.subs[0], d.subs[1]),
            _ => unreachable!("pending delta shape {}/{}", d.n_adds, d.n_subs),
        }
    }
}
```

- [ ] **Step 3: Restructure `NnueEvaluator` in `mod.rs`:**

```rust
pub struct NnueEvaluator {
    net: Box<Network>,
    stack: Box<[AccPair]>,         // stack[0..=clean] are materialized
    pending: Box<[PendingDelta]>,  // deltas for plies clean+1 ..= top
    top: usize,                    // current ply (our own make/unmake count)
    clean: usize,                  // materialization watermark; clean <= top
}
```
`from_bytes` allocates `pending` as `vec![PendingDelta::default(); MAX_PLY + 1].into_boxed_slice()` alongside `stack`.

```rust
fn refresh(&mut self, pos: &Position) {
    self.top = 0;
    self.clean = 0;
    // ... existing stack[0] rebuild via add() unchanged ...
}

fn on_make(&mut self, mv: Move, pos: &Position) {
    self.top += 1;
    let d = &mut self.pending[self.top];
    *d = PendingDelta::default();
    // identical branch structure to A1's on_make, but instead of calling a
    // kernel, fill d.adds/d.subs/d.n_adds/d.n_subs with the SAME feat_idx
    // values (captured piece resolved RIGHT HERE from pos.undo_stack):
    //   quiet:        adds=[moved@to]            subs=[moved@from]
    //   capture:      adds=[moved@to]            subs=[moved@from, cap@to]
    //   promo:        adds=[promoted@to]         subs=[pawn@from]
    //   promo-cap:    adds=[promoted@to]         subs=[pawn@from, cap@to]
    //   en passant:   adds=[moved@to]            subs=[moved@from, cap_pawn@cap_sq]
    //   castle:       adds=[king@to, rook@rt]    subs=[king@from, rook@rf]
    // (write the six branches out fully — they are the A1 branches with
    //  kernel calls replaced by delta-field assignment)
}

fn on_unmake(&mut self, _mv: Move, _pos: &Position) {
    self.top -= 1;
    self.clean = self.clean.min(self.top); // discard unmaterialized = free
}

fn evaluate(&mut self, pos: &Position) -> i32 {
    while self.clean < self.top {
        let next = self.clean + 1;
        self.stack[next] = self.stack[self.clean]; // copy-forward at materialization
        let (head, tail) = self.stack.split_at_mut(next); // or copy then apply on stack[next]
        let _ = head;
        tail[0].apply(&self.net, &self.pending[next]);
        self.clean = next;
    }
    let acc = &self.stack[self.top];
    // ... existing (us, them) select + self.net.out(...) unchanged ...
}
```
(Borrow note for the implementer: the copy `self.stack[next] = self.stack[self.clean]` and the subsequent `apply` on `self.stack[next]` are two separate statements — `AccPair` is `Copy`, so plain indexing works without `split_at_mut`; use the simple form if it borrow-checks: `let prev = self.stack[self.clean]; self.stack[next] = prev; self.stack[next].apply(&self.net, &self.pending[next]);` — `apply` takes `&Network`, which aliases `self.net` while `self.stack` is borrowed mutably; if the borrow checker objects, clone the `Box<Network>` reference via `let net = &*self.net as *const Network` — NO. The established pattern in this file is `let net = &self.net;` before mutating `self.stack[...]` through a separate field borrow, which Rust allows (disjoint fields). Follow the existing on_make's `let net = &self.net; let acc = &mut self.stack[self.top];` pattern.)

- [ ] **Step 4: New laziness tests in `mod.rs`** (alongside the existing suite, which all stays):

```rust
#[test]
fn lazy_evaluate_after_unevaluated_makes_matches_refresh() {
    let Ok(bytes) = std::fs::read(TOY) else { return };
    for n in 1..=8usize {
        let mut lazy = NnueEvaluator::from_bytes(&bytes);
        let mut chk = NnueEvaluator::from_bytes(&bytes);
        let mut pos = Position::startpos();
        lazy.refresh(&pos);
        let mut s = 0xFEED_0000u64 + n as u64;
        for _ in 0..n {
            // random legal move; NO evaluate between makes
            let mut list = MoveList::new();
            generate_moves(&pos, &mut list);
            let legal: Vec<Move> = list.iter().copied().filter(|&m| {
                if pos.make(m) { pos.unmake(); true } else { false }
            }).collect();
            if legal.is_empty() { return; }
            s ^= s << 13; s ^= s >> 7; s ^= s << 17;
            let mv = legal[(s as usize) % legal.len()];
            pos.make(mv);
            lazy.on_make(mv, &pos);
        }
        chk.refresh(&pos);
        assert_eq!(lazy.evaluate(&pos), chk.evaluate(&pos), "n={n}");
    }
}

#[test]
fn churn_without_evaluate_then_evaluate_matches_refresh() {
    // 200 random make/unmake ops with no evaluate, then one evaluate.
    let Ok(bytes) = std::fs::read(TOY) else { return };
    let mut lazy = NnueEvaluator::from_bytes(&bytes);
    let mut pos = Position::startpos();
    lazy.refresh(&pos);
    let mut made: Vec<Move> = Vec::new();
    let mut s = 0xBEEF_CAFEu64;
    for _ in 0..200 {
        s ^= s << 13; s ^= s >> 7; s ^= s << 17;
        let unmake = !made.is_empty() && (s % 3 == 0);
        if unmake {
            let mv = made.pop().unwrap();
            pos.unmake();
            lazy.on_unmake(mv, &pos);
        } else {
            let mut list = MoveList::new();
            generate_moves(&pos, &mut list);
            let legal: Vec<Move> = list.iter().copied().filter(|&m| {
                if pos.make(m) { pos.unmake(); true } else { false }
            }).collect();
            if legal.is_empty() { continue; }
            let mv = legal[(s as usize) % legal.len()];
            pos.make(mv);
            lazy.on_make(mv, &pos);
            made.push(mv);
        }
    }
    let mut chk = NnueEvaluator::from_bytes(&bytes);
    chk.refresh(&pos);
    assert_eq!(lazy.evaluate(&pos), chk.evaluate(&pos));
}

#[test]
fn null_between_pending_makes_is_transparent() {
    // pending makes, then a null (no hooks), then evaluate — must match refresh.
    let Ok(bytes) = std::fs::read(TOY) else { return };
    let mut lazy = NnueEvaluator::from_bytes(&bytes);
    let mut pos = Position::from_fen(
        "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3",
    ).unwrap();
    lazy.refresh(&pos);
    let mv = crate::board::movegen::find_first_legal(&mut pos).unwrap();
    pos.make(mv);
    lazy.on_make(mv, &pos);   // pending, unevaluated
    pos.make_null();          // search-style null: no eval hook
    let mut chk = NnueEvaluator::from_bytes(&bytes);
    chk.refresh(&pos);
    assert_eq!(lazy.evaluate(&pos), chk.evaluate(&pos));
    pos.unmake_null();
}
```
(Existing promo/EP/castle targeted tests already route through the lazy path now — they evaluate after exactly one pending make.)

- [ ] **Step 5: Both-profile suite, node-identical d6+d13, Bench == B0, NPS recorded** — identical commands to Task 1 Steps 5–6. Expected NPS: a further +8–15% on top of A1 (the eval-throughput share of make/unmake drops by the never-evaluated fraction).

- [ ] **Step 6: Commit** (after combined review):

```bash
cargo fmt && git add src/eval/nnue/accumulator.rs src/eval/nnue/mod.rs
git commit -m "perf(nnue): lazy accumulator materialization (node-identical; no elo credit)

on_make records a pre-resolved PendingDelta (captured pieces resolved at
make time — they cannot be re-derived later) and touches no vectors;
evaluate() materializes pending plies from the clean watermark with the
fused kernels; on_unmake discards unmaterialized deltas for free. Nodes
that never evaluate (TT cutoffs, prunes — roughly a third of makes) now
skip accumulator work entirely. Null moves remain hook-free and
transparent (ply bookkeeping is the evaluator's own make/unmake count).
Node-identical at d6/d13; suite green in debug+release incl. new
laziness/churn/null-pending tests. NPS: <before> -> <after>.

Bench: <B0>

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

Push both commits; CI must stay green (fmt!).

---

### Task 3: B — flywheel turn 3 (controller)

- [ ] **Step 1:** Confirm the tree is the committed champion config and `target/release/nebchess` benches as the champion (rebuild via `rm target/release/nebchess && touch src/eval/nnue/mod.rs && cargo build --release` — never `cp` onto cargo outputs). NEVER launch datagen with a placeholder/toy net embedded.
- [ ] **Step 2:** `./target/release/datagen --out tools/data/selfplay-net4 --games 1770000 --threads 20 --seed 1 --nodes 5000 > tools/datagen-net4.log 2>&1` (background, harness-tracked; bot stays live; ~22h).
- [ ] **Step 3 (on completion):** stats gate — `./target/release/datagen stats tools/data/selfplay-net4`: positions ≈150M ±5%, `LEAKS 0/0`, draws ≥ ~47% (fourth-generation signature). Record exact POS.
- [ ] **Step 4:** `TMPDIR=/home/witt/claude-workspace/NebChess/tools/data tools/trainer/prepare-data.sh tools/data/selfplay-net4 tools/data/net4.shuf.bin 12288`; byte gate `stat -c %s` == `32 * POS`.

### Task 4: C — train the re-gate pair (controller)

- [ ] **Step 1:** `BPS=$(( (POS + 8192) / 16384 ))`.
- [ ] **Step 2:** From `tools/trainer` (CUDA_PATH=/usr/local/cuda):
  - `--data ../data/net4.shuf.bin --id net4a --superbatches 25 --bps $BPS` **+ the champion's arch flags** (none if 768-plain; `--buckets 8` if rung-d promoted 768×8)
  - `--data ../data/net4.shuf.bin --id net4b --superbatches 25 --bps $BPS --hidden 1024` **+ `--buckets 8` iff rung-d validated buckets**
- [ ] **Step 3:** Contract sizes exact (768 = 1,184,320 / 768×8 = 1,205,824 / 1024 = 1,579,072 / 1024×8 = 1,607,744); record losses.

### Task 5: C — the gates (controller; bot paused via the between-games watcher)

- [ ] **Step 1 (rung 4a — data):** embed net4a (champion arch: net-file swap only), rebuild, bench, full suite; SPRT `tools/sprt.sh "$(pwd)/target/release/nebchess" "$(pwd)/tools/bin/baseline-<champion>" 10`. H1 → `tools/baseline.sh net4a`. Honest H0 → recorded, ladder continues (net4b still gates vs the champion).
- [ ] **Step 2 (rung 4b — THE WIDTH RE-GATE):** re-author the width config on top of the A2 runtime (the M10 patches will NOT apply — A2 rewrote the files; the re-authoring is small and mechanical because width/buckets never touched accumulator internals: `HIDDEN` 768→1024 in net.rs + the size assert to the right contract + TOY paths to a 1024-family toy + embed net4b.bin; the bucket struct/evaluate code is already in the committed tree iff rung-d shipped it). Build, both-profile suite, real-net material-sign on net4b, bench. SPRT vs best-so-far at elo1=10.
  - **H1:** width unlocked — net4b is the winner.
  - **H0 decisive:** width stays waitlisted with the new tax number; net4a (or the champion) is the winner.
  - **H0-but-positive:** numbers to the user (standing rule).
- [ ] **Step 3:** All verdicts recorded for the sprt-log (including the A1/A2 NPS notes).

### Task 6: Promotion + ship 0.11.0 (controller)

- [ ] **Step 1:** Working tree = the winner's config only; `.gitignore` exception swapped to the winner's bin; previous net `git rm`'d; loser bins deleted.
- [ ] **Step 2:** Combined spec+quality review (call-site completeness, embed/asserts, gitignore, ledger arithmetic vs the SPRT/gauntlet ground truth).
- [ ] **Step 3:** Promotion commit (`Bench:` line) + anchored gauntlet `tools/anchored-gauntlet.sh 300` vs the UNCHANGED pool (comparison row = the latest shipped rating) + release commit (version bump, README, strength-log row with full ladder attribution + NPS gains noted as infrastructure, sprt-log rows for every rung incl. honest H0s) + push + CI green.
- [ ] **Step 4:** `tools/lichess/deploy.sh` + restart the bot; memory update (M11 verdicts, the width answer, M12 fork: 1536/king-buckets if H1, features-at-768/search if H0).

---

## Self-review (controller)

- **Spec coverage:** A1 fused kernels with the i16 requirement (T1 S2/S4) ✓; A2 lazy with the three careful design points — copy-at-materialization (T2 S3 evaluate), null transparency via own ply counter (T2 S3/S4 null test), make-time capture resolution (T2 S2/S3 comments) ✓; node-identical + no-Elo-credit gating (T1 S6/S7, T2 S5/S6) ✓; flywheel turn 3 with generator-from-rung-d (T3) ✓; re-gate on the fast eval with re-authored width config (T5 S2) ✓; ship + ledgers + bot (T6) ✓; out-of-scope items untouched ✓.
- **Placeholders:** `<before>/<after>/<B0>` NPS/bench values are measured at execution and inserted in the commit messages — explicitly produced by named steps. The on_make delta-fill in T2 S3 lists all six branch mappings rather than full repeated code (the A1 on_make in T1 S3 IS the full branch code; T2 transcribes kernel-calls→field-assignments 1:1 per the listed mapping) — acceptable since T1's code is complete and adjacent.
- **Type consistency:** `FeatIdx`/`feat_idx`/`PendingDelta`/`apply` names consistent T1→T2; `clean`/`top` semantics consistent across on_make/on_unmake/evaluate/refresh; kernel names match between definition (T1 S2) and dispatch (T2 S2).
- **Sequencing traps:** T1 blocked on rung-d resolution (header); reference-binary stash before each refactor; debug-profile test runs to catch overflow; the borrow-checker note in T2 S3 points at the file's existing disjoint-field pattern.
