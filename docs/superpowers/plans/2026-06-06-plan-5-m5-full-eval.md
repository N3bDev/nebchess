# NebChess Plan 5 (M5): The Evaluation Awakening — Tapered Full HCE + Texel at Scale

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Teach the engine that activity, king danger, threats, and structure matter: tapered mg/eg foundation, then pawn structure, mobility, king safety, and threats/coordination — every term Texel-tuned through a trace architecture, gated by canary+SPRT, closed by an upshifted anchored gauntlet (re-aimed target: 2600).

**Architecture:** The single-phase eval becomes tapered (mg,eg) pairs blended by a 24-point phase. All parameters move to a generated `eval_params.rs` with a **manifest** (single source of truth for parameter layout) and a **Tracer** seam: the engine path uses a zero-cost `NullTracer`; the tuner runs the SAME eval with a `CollectingTracer`, so features are extracted by the exact code that plays (no tuner/engine drift — the Ethereal architecture). Each new term = eval code + manifest entry; the tuner picks it up automatically.

**Tech Stack:** Rust std-only (std::thread for the parallel tuner). Datasets: zurichess quiet-labeled (in hand) + lichess-big3-resolved upgrade attempt. Established gates: tactics canary (WAC), SPRT via tools/sprt.sh, anchored gauntlet via tools/anchored-gauntlet.sh.

**User-layer mapping (the seven asks → where they land):**
- *Mobility* → T3 explicit per-piece safe-mobility tables. *Trapped pieces* land here free: a 0-mobility bishop reads the most negative table cell — that IS the trapped-piece term.
- *King safety* → T4 explicit (zone attackers by type, pawn shield, open/semi files near king).
- *Pawn structure* → T2 explicit (passed by rank, connected passers, isolated, doubled; shields live in T4).
- *Threats* → T5 explicit (pawn threats on pieces, minor-on-major, hanging pieces).
- *Coordination* → T5 explicit (bishop pair, rook open/semi files) + batteries emerge from mobility×threats.
- *Development* → EMERGENT: tapered-retuned mg PSTs (T1) + mobility (T3) + tempo (T5) express it; modern HCEs carry no literal "development" term. If post-M5 play still shuffles in the opening, an explicit term becomes an M6 experiment.
- *Initiative/tempo* → tempo term (T5) + emergent from threat/mobility weights; "don't cash out into passivity" is exactly what tuned threat+mobility scores encode.
- *Weak squares* → deferred (largely emergent via PST+mobility; explicit outpost terms are M6 polish).

**Gate protocol per eval task (T1-T6), the M4-evolved routine:**
1. implement + tests → two-stage review;
2. **retune**: `cargo run --release --bin tune -- tools/data/quiet-labeled.epd > src/eval/eval_params.rs && cargo test` (the manifest makes new params tune automatically; ~1-4 min);
3. bench (twice, identical) → commit with Bench line;
4. CONTROLLER: **tactics canary alone** (drop ≥10 vs the previous entry = halt-and-attribute, the futility precedent);
5. CONTROLLER: **SPRT alone** vs the previous baseline — [0,10] for T1-T4 (big expected gains), [0,5] for T5/T6. H0 = stop-and-debug (no H0-tolerated gates this plan — every term is expected to gain).
6. log rows (sprt-log + tactics-log), `tools/baseline.sh <name>`.

**Plan deviations from spec (recorded deliberately):**
- §6.2's pawn hash lands in T2 as specified (thread-local, owned by Hce). The TUNER path bypasses it (a cache would swallow trace records) — the Tracer seam gates it.
- Backward pawns, outposts/weak squares, rook-on-7th, king tropism: M6 polish candidates, not M5 (YAGNI until the tune plateaus).
- PV-instability time extensions + history persistence across moves: still deferred (bot-polish phase alongside book/Syzygy per the user's roadmap).
- The spec's "Contempt" option: still deferred.

**Environment facts:** v0.4.0 @ edabfa4, bench 138119, 121 tests, baseline-texel-pst is the SPRT chain head (= the anchored 2414 binary). WAC reference: 258/299 (the tuned-eval entry). 16-core WSL2; run canaries/SPRTs alone. Prefix cargo with `source "$HOME/.cargo/env" && `.

---

## File structure (end state)

```
src/eval/
  mod.rs           # Evaluator trait (unchanged seam) + re-exports
  eval_params.rs   # GENERATED (replaces psqt.rs): all (mg,eg) parameter pairs
  manifest.rs      # NEW: parameter registry — names, lengths, flat offsets
  trace.rs         # NEW: Tracer trait, NullTracer (engine), CollectingTracer (tuner)
  hce.rs           # REWRITTEN: traced term functions, phase blend, pawn hash
src/board/position.rs  # + incremental pawn_key (pawn-only zobrist)
src/bin/tune.rs    # v2: manifest-driven, trace-fed, phase-weighted, parallel (T6)
tools/get-anchors.sh        # + Stash v20/v21 rungs (T7)
docs/{sprt,tactics,strength}-log.md  # rows per gate
```

`psqt.rs` is deleted in T1 (its values seed `eval_params.rs`). MovePicker's `MATERIAL` import moves to a search-local const (ordering shouldn't churn when eval retunes — pin it).

---

### Task 1: Tapered foundation + trace architecture + tuner v2 + first mg/eg retune — SPRT GATE #1

The structural long-pole, done as ONE coordinated change (M4 final-review directive). After this task, every later term is a small add.

**Files:**
- Create: `src/eval/manifest.rs`, `src/eval/trace.rs`, `src/eval/eval_params.rs`
- Rewrite: `src/eval/hce.rs`, `src/bin/tune.rs`
- Modify: `src/eval/mod.rs`, `src/search/mod.rs` (MovePicker const), delete `src/eval/psqt.rs`

- [ ] **Step 1.1: The manifest** (`src/eval/manifest.rs`) — the single source of truth both the eval and the tuner read:

```rust
//! Parameter registry: every tunable term declares (name, length) here.
//! Flat pair-index space: offsets are cumulative. The tuner sizes its
//! vector from TOTAL_PAIRS; eval_params.rs is emitted in manifest order.

pub struct TermDef {
    pub name: &'static str,
    pub len: usize,
}

/// ORDER IS ABI: eval_params.rs and the tuner both index by these offsets.
/// Append-only within a task; never reorder existing entries mid-plan.
pub const TERMS: &[TermDef] = &[
    TermDef { name: "MATERIAL", len: 6 },   // P N B R Q K (K pair stays 0; P mg pinned 100)
    TermDef { name: "PST_PAWN", len: 64 },
    TermDef { name: "PST_KNIGHT", len: 64 },
    TermDef { name: "PST_BISHOP", len: 64 },
    TermDef { name: "PST_ROOK", len: 64 },
    TermDef { name: "PST_QUEEN", len: 64 },
    TermDef { name: "PST_KING", len: 64 },
    // T2 appends: PASSED(6) PASSED_CONNECTED(1) ISOLATED(1) DOUBLED(1)
    // T3 appends: MOB_KNIGHT(9) MOB_BISHOP(14) MOB_ROOK(15) MOB_QUEEN(28)
    // T4 appends: KS_ATTACKER(4) KS_SHIELD(3) KS_OPEN_FILE(1) KS_SEMI_FILE(1)
    // T5 appends: THREAT_BY_PAWN(4) THREAT_BY_MINOR(4) HANGING(1)
    //             BISHOP_PAIR(1) ROOK_OPEN(1) ROOK_SEMI(1) TEMPO(1)
];

pub const fn offset_of(name: &str) -> usize {
    let mut off = 0;
    let mut i = 0;
    while i < TERMS.len() {
        // const-compatible string compare
        if str_eq(TERMS[i].name, name) {
            return off;
        }
        off += TERMS[i].len;
        i += 1;
    }
    panic!("unknown term");
}

const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

pub const fn total_pairs() -> usize {
    let mut off = 0;
    let mut i = 0;
    while i < TERMS.len() {
        off += TERMS[i].len;
        i += 1;
    }
    off
}

// Named offsets (computed once, used by hce term code as const indices)
pub const MATERIAL: usize = offset_of("MATERIAL");
pub const PST_PAWN: usize = offset_of("PST_PAWN");
pub const PST_KNIGHT: usize = offset_of("PST_KNIGHT");
pub const PST_BISHOP: usize = offset_of("PST_BISHOP");
pub const PST_ROOK: usize = offset_of("PST_ROOK");
pub const PST_QUEEN: usize = offset_of("PST_QUEEN");
pub const PST_KING: usize = offset_of("PST_KING");
pub const TOTAL_PAIRS: usize = total_pairs();
```

- [ ] **Step 1.2: The Tracer seam** (`src/eval/trace.rs`):

```rust
//! The tuner/engine seam: eval terms call `trace.record(pair_idx, sign)`
//! alongside every parameter use. NullTracer compiles to nothing (engine
//! hot path); CollectingTracer captures the feature vector (tuner) from
//! the EXACT code that plays — no extraction drift, ever.

pub trait Tracer {
    fn record(&mut self, pair_idx: usize, sign: i8);
}

pub struct NullTracer;
impl Tracer for NullTracer {
    #[inline(always)]
    fn record(&mut self, _pair_idx: usize, _sign: i8) {}
}

#[derive(Default)]
pub struct CollectingTracer {
    pub features: Vec<(u16, i8)>,
}
impl Tracer for CollectingTracer {
    fn record(&mut self, pair_idx: usize, sign: i8) {
        self.features.push((pair_idx as u16, sign));
    }
}
```

- [ ] **Step 1.3: Initial `eval_params.rs`.** Mechanically seed pairs from the CURRENT M4-tuned `psqt.rs` values — `(v, v)` for every entry (mg = eg = tuned single-phase value; the retune in Step 1.7 diverges them). Format (the tuner's emitter reproduces exactly this shape):

```rust
//! GENERATED by `cargo run --release --bin tune`. Do not edit.
//! (seeded 2026-06-06 from the M4 single-phase tune; mg==eg until retuned)
//! Layout: manifest order, one `(mg, eg)` pair per parameter.
//! PST layout: rank-8 row first; white reads PST[sq ^ 56], black PST[sq].

pub static PARAMS: [(i32, i32); crate::eval::manifest::TOTAL_PAIRS] = [
    // MATERIAL: P N B R Q K
    (100, 100), (330, 330), (350, 350), (549, 549), (1049, 1049), (0, 0),
    // PST_PAWN (64 pairs, 8 per line) ... copy each current PAWN value v as (v, v)
    // ... all six tables ...
];
```

(The implementer generates this file with a 20-line throwaway script or disciplined editor work from the current psqt.rs — then verifies `PARAMS.len() == TOTAL_PAIRS` via a unit test, and deletes psqt.rs.)

- [ ] **Step 1.4: hce.rs rewrite** — traced terms + phase blend:

```rust
//! Tapered HCE. Every term function takes a Tracer; the engine calls with
//! NullTracer (zero cost), the tuner with CollectingTracer. ALL parameter
//! reads go through PARAMS[idx] + trace.record(idx, sign) IN THE SAME
//! STATEMENT GROUP — that invariant is what keeps the tuner honest.

use crate::board::{Color, Move, PieceType, Position};
use crate::eval::eval_params::PARAMS;
use crate::eval::manifest as m;
use crate::eval::trace::{NullTracer, Tracer};
use crate::eval::Evaluator;

/// Game phase: N/B=1, R=2, Q=4 per piece, capped at 24 (opening) .. 0 (bare kings).
pub fn phase(pos: &Position) -> i32 {
    let mut p = 0;
    for color in [Color::White, Color::Black] {
        p += pos.piece_bb(color, PieceType::Knight).count() as i32;
        p += pos.piece_bb(color, PieceType::Bishop).count() as i32;
        p += 2 * pos.piece_bb(color, PieceType::Rook).count() as i32;
        p += 4 * pos.piece_bb(color, PieceType::Queen).count() as i32;
    }
    p.min(24)
}

/// White-relative (mg, eg) accumulation over all terms.
pub fn eval_terms<T: Tracer>(pos: &Position, t: &mut T) -> (i32, i32) {
    let (mut mg, mut eg) = (0i32, 0i32);
    let mut add = |idx: usize, sign: i32, t: &mut T, mg: &mut i32, eg: &mut i32| {
        let (pmg, peg) = PARAMS[idx];
        *mg += sign * pmg;
        *eg += sign * peg;
        t.record(idx, sign as i8);
    };

    const PST: [usize; 6] = [
        m::PST_PAWN,
        m::PST_KNIGHT,
        m::PST_BISHOP,
        m::PST_ROOK,
        m::PST_QUEEN,
        m::PST_KING,
    ];
    for pt in PieceType::ALL {
        for sq in pos.piece_bb(Color::White, pt) {
            add(m::MATERIAL + pt.index(), 1, t, &mut mg, &mut eg);
            add(PST[pt.index()] + (sq.index() ^ 56), 1, t, &mut mg, &mut eg);
        }
        for sq in pos.piece_bb(Color::Black, pt) {
            add(m::MATERIAL + pt.index(), -1, t, &mut mg, &mut eg);
            add(PST[pt.index()] + sq.index(), -1, t, &mut mg, &mut eg);
        }
    }
    // T2 pawn-structure terms append here; T3 mobility; T4 king safety; T5 threats
    (mg, eg)
}

/// Blend by phase and flip to side-to-move-relative.
pub fn evaluate_white_relative(pos: &Position) -> i32 {
    let (mg, eg) = eval_terms(pos, &mut NullTracer);
    let ph = phase(pos);
    (mg * ph + eg * (24 - ph)) / 24
}

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
        let white = evaluate_white_relative(pos);
        if pos.stm() == Color::White {
            white
        } else {
            -white
        }
    }
}
```

(Existing structural tests — startpos balance, mirror negation, stm negation — survive untouched: symmetric construction is phase-independent. Value-dependent tests rebaseline per the established honest-adjustment convention.)

- [ ] **Step 1.5: MovePicker decoupling.** In `src/search/mod.rs`, MVV-LVA currently imports eval `MATERIAL`. Pin ordering values locally (ordering must not churn under retunes):

```rust
/// MVV victim values for ordering only — deliberately decoupled from the
/// tunable eval material (retunes must not silently reshape move ordering).
const VICTIM_VALS: [i32; 6] = [100, 320, 330, 500, 900, 0];
```

Replace the picker's `MATERIAL[victim.index()]` with `VICTIM_VALS[victim.index()]`; remove the eval import. NOTE: this changes ordering arithmetic only if tuned-material ≠ classic values — it does (549/1049) — so BENCH WILL CHANGE in this step; that's expected and correct (record the post-task value once, after 1.7).

- [ ] **Step 1.6: tune.rs v2.** Rewrite around the trace (keep the EPD loader, K-fit, Adam skeleton from v1):

```rust
//! Texel tuner v2: manifest-driven, trace-fed, phase-weighted.
//!   cargo run --release --bin tune -- tools/data/quiet-labeled.epd > src/eval/eval_params.rs
//! Param vector: [mg_bank | eg_bank], each TOTAL_PAIRS long. A traced
//! feature (idx, sign) at a position with phase ph contributes:
//!   d(eval)/d(mg[idx]) = sign * ph/24,  d(eval)/d(eg[idx]) = sign * (24-ph)/24.

use nebchess::board::Position;
use nebchess::eval::hce::{eval_terms, phase};
use nebchess::eval::manifest::{self, TERMS, TOTAL_PAIRS};
use nebchess::eval::trace::CollectingTracer;

const N: usize = TOTAL_PAIRS;

struct Sample {
    features: Vec<(u16, i8)>,
    phase: i32,
    result: f64,
}

fn extract(pos: &Position, result: f64) -> Sample {
    let mut tr = CollectingTracer::default();
    let _ = eval_terms(pos, &mut tr); // the REAL eval produces the features
    Sample {
        features: tr.features,
        phase: phase(pos),
        result,
    }
}

fn eval_sample(s: &Sample, p: &[f64]) -> f64 {
    // p layout: [0..N) = mg, [N..2N) = eg
    let (wmg, weg) = (s.phase as f64 / 24.0, (24 - s.phase) as f64 / 24.0);
    let mut e = 0.0;
    for &(idx, sign) in &s.features {
        let i = idx as usize;
        e += sign as f64 * (p[i] * wmg + p[N + i] * weg);
    }
    e
}

// sigmoid / mse / K line-search: unchanged from v1 (operate on eval_sample).
// warm_start: read PARAMS pairs -> p[i]=mg, p[N+i]=eg.
// Adam loop: per feature, grad_mg[i] += common * sign * wmg;
//            grad_eg[i] += common * sign * weg.
// AFTER EVERY STEP: re-pin the anchor — p[manifest::MATERIAL + 0] = 100.0 (P mg).
// emit(): walk TERMS in manifest order printing the eval_params.rs shape from
// Step 1.3 (header notes K + train/val MSE + date), values rounded i32 pairs,
// 8 pairs per line for PSTs, term-name comments between blocks.
```

(The plan trusts the v1 file for the unchanged blocks — the implementer ports them; everything novel is specified above. The `// T2 appends here` markers in TERMS/eval_terms are real comments to commit.)

- [ ] **Step 1.7: Tests, retune, bench, commit.**
1. Unit test in hce.rs: `PARAMS.len() == TOTAL_PAIRS` (manifest/params drift guard); phase(startpos)==24; phase of a bare-kings FEN == 0; structural eval tests green.
2. Full retune: `cargo run --release --bin tune -- tools/data/quiet-labeled.epd > src/eval/eval_params.rs` — REPORT K + MSEs (val must improve on the M4 single-phase 0.064445 — tapering adds real capacity); `cargo test` after landing.
3. Bench twice (changes: tapered eval + retune + VICTIM_VALS) → commit `feat(eval): tapered mg/eg foundation with trace architecture; retuned` + Bench line.
4. CONTROLLER: canary (reference 258) then SPRT #1 vs `baseline-texel-pst` [0,10] → log + `tools/baseline.sh tapered`.

(Expected: tapered+retune is historically one of the biggest single HCE gains — Rustic measured ~+250 self-play for tapering+tuning combined; ours arrives on top of an already-tuned single phase, so expect less but still large.)

---

### Task 2: Pawn structure + pawn key + pawn hash — SPRT GATE #2

**Files:**
- Modify: `src/board/position.rs` (incremental pawn_key), `src/eval/manifest.rs`, `src/eval/hce.rs`, `src/eval/mod.rs`

- [ ] **Step 2.1: Incremental pawn key on Position.** Add field `pub(crate) pawn_key: u64` (+ `pawn_key: u64` in `Undo`); in `put_piece`/`remove_piece`, when `p.piece_type() == PieceType::Pawn` also XOR the same zobrist piece key into `self.pawn_key`; `make()` stores it in the Undo, `unmake()` restores wholesale (raw helpers untouched — same architecture as the main key); `make_null` stores/restores it via its Undo too (value unchanged by a null). `from_fen` builds it during placement; add `pub fn pawn_key(&self) -> u64`, `pub fn compute_pawn_key(&self) -> u64` (pawns-only recompute) and a `debug_assert_eq!` beside the existing key assert in `make()`. Tests: pawn_key == compute across the existing make/unmake chain test (extend it); pawn moves change it, knight moves don't.

- [ ] **Step 2.2: Manifest + masks + terms.** Append to `TERMS` (after PST_KING):

```rust
    TermDef { name: "PASSED", len: 6 },           // by relative rank 2..7
    TermDef { name: "PASSED_CONNECTED", len: 1 },
    TermDef { name: "ISOLATED", len: 1 },
    TermDef { name: "DOUBLED", len: 1 },
```

(+ named offsets `PASSED`, `PASSED_CONNECTED`, `ISOLATED`, `DOUBLED`). In hce.rs add const-built masks (same `const fn` + `while` idiom as attacks.rs):

```rust
/// passed_mask[color][sq]: same+adjacent files, all ranks strictly ahead.
/// adjacent_files[file]: the 1-2 neighboring files.
/// forward_file[color][sq]: same file, strictly ahead (doubled detection).
```

and the term function (signature pattern for ALL later terms):

```rust
fn pawn_terms<T: Tracer>(pos: &Position, t: &mut T, mg: &mut i32, eg: &mut i32) {
    for (color, sign) in [(Color::White, 1i32), (Color::Black, -1i32)] {
        let own = pos.piece_bb(color, PieceType::Pawn);
        let enemy = pos.piece_bb(color.flip(), PieceType::Pawn);
        for sq in own {
            let rel_rank = if color == Color::White { sq.rank() } else { 7 - sq.rank() } as usize;
            if (passed_mask(color, sq) & enemy).is_empty() {
                add_term(m::PASSED + rel_rank - 1, sign, t, mg, eg);
                if (adjacent_files(sq.file()) & own).any() {
                    add_term(m::PASSED_CONNECTED, sign, t, mg, eg);
                }
            }
            if (adjacent_files(sq.file()) & own).is_empty() {
                add_term(m::ISOLATED, sign, t, mg, eg);
            }
            if (forward_file(color, sq) & own).any() {
                add_term(m::DOUBLED, sign, t, mg, eg);
            }
        }
    }
}
```

(Promote the Step-1.4 closure to a free `fn add_term<T: Tracer>(idx, sign, t, &mut mg, &mut eg)` — all terms share it. `rel_rank` for a pawn is always 1..=6, so `PASSED + rel_rank - 1` indexes 0..=5.) Call `pawn_terms` from `eval_terms` at the marked spot. Seed the 9 new pairs in eval_params.rs with sane starters: PASSED `(0,10) (5,20) (10,35) (20,60) (40,110) (80,180)`, PASSED_CONNECTED `(5,15)`, ISOLATED `(-10,-15)`, DOUBLED `(-10,-20)` (the retune owns the final values; `PARAMS.len()==TOTAL_PAIRS` test enforces the append).

- [ ] **Step 2.3: Pawn hash in Hce** (spec §6.2; the Tracer seam gates it — caches and traces don't mix):

```rust
const PAWN_HASH_SIZE: usize = 16384; // entries; (u64, i32, i32) = 16B -> 256KB

pub struct Hce {
    pawn_hash: Vec<(u64, i32, i32)>, // (pawn_key, mg, eg); 0-key = empty
}
// evaluate(): pawn terms resolved via the cache (lookup by pos.pawn_key(),
// verify full key, replace-always on miss); all other terms computed fresh.
// eval_terms (the traced/tuner path) computes pawn_terms DIRECTLY — uncached.
```

Restructure: `eval_terms<T>` keeps calling `pawn_terms` (tuner path, uncached); `Hce::evaluate` calls a new `eval_terms_cached(&mut self.pawn_hash, pos)` that computes non-pawn terms via the shared code and pawn terms through the cache. A unit test proves transparency: `Hce::evaluate(pos)` (twice — cold then cached) equals the uncached traced result blended by phase, for 5 varied FENs.

- [ ] **Step 2.4: Tests** — passed/isolated/doubled detection on crafted FENs (e.g. `"4k3/8/8/3P4/8/8/8/4K3 w"` → exactly one PASSED record at rel_rank 4 via CollectingTracer — trace-based unit tests are the pattern for ALL term tasks: assert the FEATURE RECORDS, not tuned values); connected-passer FEN; pawn-key tests from 2.1.

- [ ] **Step 2.5: Retune → bench (twice) → commit** `feat(eval): pawn structure terms with pawn hash` + Bench. **Step 2.6 (CONTROLLER):** canary → SPRT #2 [0,10] vs `baseline-tapered` → logs + `tools/baseline.sh pawns`.

---

### Task 3: Mobility — SPRT GATE #3

**Files:**
- Modify: `src/eval/manifest.rs`, `src/eval/hce.rs`

- [ ] **Step 3.1: Manifest append** `MOB_KNIGHT(9) MOB_BISHOP(14) MOB_ROOK(15) MOB_QUEEN(28)` (+offsets). Seed pairs: a gentle ramp `(-25+8*i...)`-style is fine — concretely seed each table linearly from `(-30,-30)` at 0 mobility up to `(+30,+30)` at max, rounded; the retune owns the curve.

- [ ] **Step 3.2: Whole-set pawn-attack helpers** (bitboard shifts, in hce.rs):

```rust
fn pawn_attack_set(pos: &Position, color: Color) -> Bitboard {
    let p = pos.piece_bb(color, PieceType::Pawn);
    match color {
        Color::White => p.north_east() | p.north_west(),
        Color::Black => p.south_east() | p.south_west(),
    }
}
```

- [ ] **Step 3.3: The term.** Safe-mobility per piece — `safe = !own_occ & !enemy_pawn_attacks`:

```rust
fn mobility_terms<T: Tracer>(pos: &Position, t: &mut T, mg: &mut i32, eg: &mut i32) {
    let occ = pos.occ_all();
    for (color, sign) in [(Color::White, 1i32), (Color::Black, -1i32)] {
        let safe = !pos.occ(color) & !pawn_attack_set(pos, color.flip());
        for sq in pos.piece_bb(color, PieceType::Knight) {
            let n = (attacks::knight_attacks(sq) & safe).count() as usize;
            add_term(m::MOB_KNIGHT + n, sign, t, mg, eg);
        }
        for sq in pos.piece_bb(color, PieceType::Bishop) {
            let n = (attacks::bishop_attacks(sq, occ) & safe).count() as usize;
            add_term(m::MOB_BISHOP + n, sign, t, mg, eg);
        }
        for sq in pos.piece_bb(color, PieceType::Rook) {
            let n = (attacks::rook_attacks(sq, occ) & safe).count() as usize;
            add_term(m::MOB_ROOK + n, sign, t, mg, eg);
        }
        for sq in pos.piece_bb(color, PieceType::Queen) {
            let n = (attacks::queen_attacks(sq, occ) & safe).count() as usize;
            add_term(m::MOB_QUEEN + n, sign, t, mg, eg);
        }
    }
}
```

This is the expensive term (slider attacks per piece per eval) — NPS will drop visibly; the SPRT weighs speed vs knowledge, which is exactly its job. *The trapped-piece ask lands here free: a 0-mobility piece reads the most negative cell.*

- [ ] **Step 3.4: Trace tests** — startpos knight mobility (2 safe squares each → MOB_KNIGHT+2 ×4 records); a trapped-bishop FEN (`"k7/8/8/8/8/p7/P7b/RK6 b"`-style — verify YOUR fen: craft a corner-trapped bishop with 0 safe moves) records MOB_BISHOP+0.

- [ ] **Step 3.5: Retune → bench → commit** `feat(eval): safe mobility tables` + Bench. **Step 3.6 (CONTROLLER):** canary → SPRT #3 [0,10] vs `baseline-pawns` → `tools/baseline.sh mobility`.

---

### Task 4: King safety — SPRT GATE #4

**Files:**
- Modify: `src/eval/manifest.rs`, `src/eval/hce.rs`

- [ ] **Step 4.1: Manifest append** `KS_ATTACKER(4)` (N/B/R/Q touching the zone) `KS_SHIELD(3)` (shield pawn at rel-rank 2 / rel-rank 3 / missing, per file) `KS_OPEN_FILE(1)` `KS_SEMI_FILE(1)` (+offsets). Seeds: ATTACKER `(-15,-5) (-15,-5) (-25,-10) (-40,-15)`, SHIELD `(15,0) (8,0) (-20,-5)`, OPEN `(-30,-5)`, SEMI `(-15,-3)`.

- [ ] **Step 4.2: The term.** SIGN CONVENTION (document in code): all KS features are recorded from the white-relative frame with `sign = +1` when the WHITE king is the subject and `-1` for the black king — the tuner learns whether each parameter is good or bad; penalties come out negative naturally.

```rust
fn king_safety_terms<T: Tracer>(pos: &Position, t: &mut T, mg: &mut i32, eg: &mut i32) {
    let occ = pos.occ_all();
    for (color, sign) in [(Color::White, 1i32), (Color::Black, -1i32)] {
        let ksq = pos.king_sq(color);
        let zone = attacks::king_attacks(ksq) | ksq.bb();
        let enemy = color.flip();
        // attackers touching the zone, by type
        for (pt, slot) in [
            (PieceType::Knight, 0),
            (PieceType::Bishop, 1),
            (PieceType::Rook, 2),
            (PieceType::Queen, 3),
        ] {
            for sq in pos.piece_bb(enemy, pt) {
                let att = match pt {
                    PieceType::Knight => attacks::knight_attacks(sq),
                    PieceType::Bishop => attacks::bishop_attacks(sq, occ),
                    PieceType::Rook => attacks::rook_attacks(sq, occ),
                    _ => attacks::queen_attacks(sq, occ),
                };
                if (att & zone).any() {
                    add_term(m::KS_ATTACKER + slot, sign, t, mg, eg);
                }
            }
        }
        // shield + file state on the king's file and neighbors
        let own_pawns = pos.piece_bb(color, PieceType::Pawn);
        let all_pawns = own_pawns | pos.piece_bb(enemy, PieceType::Pawn);
        let kfile = ksq.file() as i8;
        for f in (kfile - 1).max(0)..=(kfile + 1).min(7) {
            let file_bb = file_mask(f as u8);
            let shield = own_pawns & file_bb & shield_span(color, ksq);
            if (shield & shield_rank(color, ksq, 1)).any() {
                add_term(m::KS_SHIELD, sign, t, mg, eg); // pawn one rank ahead
            } else if (shield & shield_rank(color, ksq, 2)).any() {
                add_term(m::KS_SHIELD + 1, sign, t, mg, eg);
            } else {
                add_term(m::KS_SHIELD + 2, sign, t, mg, eg); // missing
            }
            if (all_pawns & file_bb).is_empty() {
                add_term(m::KS_OPEN_FILE, sign, t, mg, eg);
            } else if (own_pawns & file_bb).is_empty() {
                add_term(m::KS_SEMI_FILE, sign, t, mg, eg);
            }
        }
    }
}
```

with small helpers `file_mask(f) -> Bitboard`, `shield_rank(color, ksq, ahead) -> Bitboard` (the rank `ahead` steps in front of the king, color-relative; empty bitboard when off-board) and `shield_span` = the two ranks ahead (their union — used only to scope `shield`; if simpler, drop `shield_span` and intersect per-rank directly — implementer's choice, report which).

- [ ] **Step 4.3: Trace tests** — castled-with-shield FEN (e.g. white Kg1, pawns f2/g2/h2: three KS_SHIELD(+0) records, no OPEN/SEMI on those files) vs stripped-king FEN (Kg1, no f/g/h pawns at all, enemy queen+rook bearing on g-file: KS_SHIELD+2 ×3, KS_OPEN_FILE records, KS_ATTACKER records present). Assert via CollectingTracer feature counts.

- [ ] **Step 4.4: Retune → bench → commit** `feat(eval): king safety (zone attackers, shield, files)` + Bench. **Step 4.5 (CONTROLLER):** canary (EXPECT IMPROVEMENT here — the WAC misses skewed king-attack motifs) → SPRT #4 [0,10] vs `baseline-mobility` → `tools/baseline.sh kingsafety`.

---

### Task 5: Threats, coordination, tempo — SPRT GATE #5 ([0,5])

**Files:**
- Modify: `src/eval/manifest.rs`, `src/eval/hce.rs`

**T4-outcome decision tree (user-specified 2026-06-06; the refactor is INFRASTRUCTURE in every branch — bench-identical to the KS build, never claimed as elo):**
- *H1*: accept KS despite cost → baseline-kingsafety → step 5.0 refactor (identity vs 99185) → **refresh the baseline to the refactored build** (eval-identical by gate, faster; ledger note "baseline refreshed, infrastructure not credited") so the threats SPRT isolates knowledge and never inherits the NPS reclaim → threats.
- *H0*: reject the current KS implementation (the claim-at-cost, not the terms) → step 5.0 refactor → **re-gate KS itself** (fused build vs baseline-mobility, [0,10]) as the next feature.
- *Still neutral at 1500–2000 games*: do NOT claim H0 — log "no practical gain at current implementation cost" → step 5.0 refactor → re-gate KS vs baseline-mobility.
- *Every branch, post-refactor*: one canary on the refactored build as a free attribution probe — eval is bit-identical, so movement is pure NPS; recovery toward 260+ confirms the T4 dip was time-tax, no movement falsifies that attribution.

- [ ] **Step 5.0 (MANDATORY, decided 2026-06-06 after T4's −13% NPS): shared attack-map pass.** Refactor BEFORE adding any new term: one pass per eval computes each piece's attack bitboard ONCE; mobility counts, king-safety zone-touch tests, and the threat-term unions all consume that single pass. Shape: fuse `mobility_terms` + the zone-attacker half of `king_safety_terms` into one piece loop (per piece: `att = attacks(sq, occ)` → mobility `add_term(MOB_* + (att & safe).count())`, enemy-king zone touch `add_term(KS_ATTACKER + slot, -sign)` — note the sign flip: KS records carry the DEFENDING king owner's sign, which is the negation of the piece owner's), accumulating `pawn_att/minor_att/all_att` unions per color for step 5.2. The king-centric shield/file half of king_safety_terms stays its own loop. **Identity gate: after the refactor and BEFORE the manifest append, `cargo test` green and `nebchess bench` == 99185 (bit-identical eval to 689f1cf) — commit the refactor separately (`refactor(eval): shared attack-map pass, bench-identical`) so the gate isolates the new terms.**

  **Required geometric tests for the fused pass (user-specified 2026-06-06; write BEFORE the refactor, they must pass on the old code too — that's the point):**
  1. White knight attacking the black king ring → trace records (KS_ATTACKER+0, sign −1) (black king is the subject) → white-relative eval contribution positive with the tuned penalty weights; assert the record sign (value-independent), and assert eval directionality by comparing against the same FEN with the knight retreated out of range.
  2. Black knight attacking the white king ring → (KS_ATTACKER+0, sign +1); eval for White strictly worse than the retreated-knight twin.
  3. Color-flipped mirror of an asymmetric king-attack FEN: white-relative evals negate exactly (extend `mirrored_position_negates` with a king-attack-asymmetric position).
  4. Blocked slider does NOT count: rook on the king's file with an interposed pawn → no KS_ATTACKER record for it (occupancy honored in the fused pass).
  5. Same FEN minus the blocker → the rook's KS_ATTACKER record appears.
  6. Mobility unchanged: the existing exact-count trace tests (startpos MOB_KNIGHT+2 ×4, trapped-bishop MOB_BISHOP+0) pass UNCHANGED through the refactor, and the bench-identity gate (99185) pins the rest globally.
  7. Shield/open-file record signs follow the KING OWNER, not any attacker: black king castled short with f7/g7/h7 shield → three (KS_SHIELD+0, sign −1) records; the T4 white-side test keeps passing as-is.

- [ ] **Step 5.1: Manifest append** `THREAT_BY_PAWN(4)` (victim N/B/R/Q) `THREAT_BY_MINOR(4)` `HANGING(1)` `BISHOP_PAIR(1)` `ROOK_OPEN(1)` `ROOK_SEMI(1)` `TEMPO(1)` (+offsets). Seeds: THREAT_BY_PAWN `(30,20)×4 scaled up by victim — (25,15)(30,20)(45,30)(60,35)`, THREAT_BY_MINOR `(15,10)(15,10)(30,20)(40,25)`, HANGING `(25,15)`, BISHOP_PAIR `(25,45)`, ROOK_OPEN `(25,5)`, ROOK_SEMI `(12,3)`, TEMPO `(15,5)`.

- [ ] **Step 5.2: Attack-map helper + the term.**

```rust
/// Every square color attacks (pawn set + per-piece attack union).
fn attack_set(pos: &Position, color: Color) -> Bitboard {
    let occ = pos.occ_all();
    let mut a = pawn_attack_set(pos, color) | attacks::king_attacks(pos.king_sq(color));
    for sq in pos.piece_bb(color, PieceType::Knight) { a |= attacks::knight_attacks(sq); }
    for sq in pos.piece_bb(color, PieceType::Bishop) { a |= attacks::bishop_attacks(sq, occ); }
    for sq in pos.piece_bb(color, PieceType::Rook) { a |= attacks::rook_attacks(sq, occ); }
    for sq in pos.piece_bb(color, PieceType::Queen) { a |= attacks::queen_attacks(sq, occ); }
    a
}

fn threat_terms<T: Tracer>(pos: &Position, t: &mut T, mg: &mut i32, eg: &mut i32) {
    let occ = pos.occ_all();
    for (color, sign) in [(Color::White, 1i32), (Color::Black, -1i32)] {
        let enemy = color.flip();
        // OUR threats against THEIR pieces (sign credits the attacker)
        let our_pawn_att = pawn_attack_set(pos, color);
        let mut our_minor_att = Bitboard::EMPTY;
        for sq in pos.piece_bb(color, PieceType::Knight) { our_minor_att |= attacks::knight_attacks(sq); }
        for sq in pos.piece_bb(color, PieceType::Bishop) { our_minor_att |= attacks::bishop_attacks(sq, occ); }
        for (pt, slot) in [(PieceType::Knight, 0), (PieceType::Bishop, 1), (PieceType::Rook, 2), (PieceType::Queen, 3)] {
            let victims = pos.piece_bb(enemy, pt);
            for _ in our_pawn_att & victims { add_term(m::THREAT_BY_PAWN + slot, sign, t, mg, eg); }
            for _ in our_minor_att & victims { add_term(m::THREAT_BY_MINOR + slot, sign, t, mg, eg); }
        }
        // THEIR hanging pieces (attacked by us, defended by nobody)
        let our_att = attack_set(pos, color);
        let their_att = attack_set(pos, enemy);
        let their_pieces = pos.occ(enemy) & !pos.piece_bb(enemy, PieceType::King);
        for _ in their_pieces & our_att & !their_att { add_term(m::HANGING, sign, t, mg, eg); }
        // coordination
        if pos.piece_bb(color, PieceType::Bishop).count() >= 2 {
            add_term(m::BISHOP_PAIR, sign, t, mg, eg);
        }
        let own_pawns = pos.piece_bb(color, PieceType::Pawn);
        let all_pawns = own_pawns | pos.piece_bb(enemy, PieceType::Pawn);
        for sq in pos.piece_bb(color, PieceType::Rook) {
            let fbb = file_mask(sq.file());
            if (all_pawns & fbb).is_empty() { add_term(m::ROOK_OPEN, sign, t, mg, eg); }
            else if (own_pawns & fbb).is_empty() { add_term(m::ROOK_SEMI, sign, t, mg, eg); }
        }
    }
    // tempo: the side to move has the initiative
    let stm_sign = if pos.stm() == Color::White { 1 } else { -1 };
    add_term(m::TEMPO, stm_sign, t, mg, eg);
}
```

(`for _ in bitboard` iterates per set bit — one record per threatened piece. The plan code above shows the SEMANTICS; per step 5.0 the implementation consumes the shared attack maps instead of calling `attack_set`/recomputing per-piece attacks — no slider attack is computed twice in one eval. Note: TEMPO makes eval stm-sensitive beyond the final negation — the structural `eval_is_stm_relative` negation test STILL holds because the white-relative score itself changes sign sources... verify: the test compares eval(pos,w) vs eval(pos,b) — same position, different stm: white-relative differs by 2×TEMPO, and stm-negation flips — the strict negation equality BREAKS by design. UPDATE that structural test honestly: assert `sw + sb == -2 * blended_tempo_or_just |sw + sb| <= 60` (small, bounded by tempo), with a comment. Report the change.)

- [ ] **Step 5.3: Trace tests** — a FEN with a knight forking... simpler deterministic cases: white pawn e4 attacking black knight d5 → one THREAT_BY_PAWN+0 (+1); an undefended black rook attacked by a white bishop → THREAT_BY_MINOR+2 and HANGING records; startpos → BISHOP_PAIR ×2 (both sides, net 0), no ROOK_OPEN (all files pawned), TEMPO +1.

- [ ] **Step 5.4: Retune → bench → commit** `feat(eval): threats, coordination, tempo` + Bench. **Step 5.5 (CONTROLLER):** canary → SPRT #5 [0,5] vs `baseline-kingsafety` → `tools/baseline.sh threats`.

---

### Task 6: Tuner at scale — parallel gradients + dataset upgrade + the full joint tune — SPRT GATE #6 ([0,5])

**Files:**
- Modify: `src/bin/tune.rs`, `tools/download-tuning-data.sh`

- [ ] **Step 6.1: Parallel gradient accumulation.** Replace the single-threaded epoch loop body with `std::thread::scope`: split `train` into `std::thread::available_parallelism().min(14)` chunks; each worker accumulates a private `Vec<f64>` gradient over its chunk; main sums them and runs the Adam step (Adam state stays single-threaded). Determinism note in code: float summation order changes results slightly across thread counts — acceptable for tuning (each run's output is committed + SPRT-gated; we never diff tuner outputs).

- [ ] **Step 6.2: Dataset upgrade attempt** (lichess-big3-resolved, ~9.7M positions, per spec §6.3 research):
- Extend `tools/download-tuning-data.sh`: download `https://archive.org/download/lichess-big3-resolved.7z/lichess-big3-resolved.7z` to tools/data/ (94.8MB); extraction needs 7z — try `python3 -m pip install --user py7zr` then `python3 -c "import py7zr; py7zr.SevenZipFile('...').extractall('tools/data/')"`. If pip/py7zr fails (no network for pip, etc.): REPORT and fall back to the 648k set with epochs=600 — the fallback is a legitimate outcome, record it.
- Loader: support the big3 line format `<fen> [<0|0.5|1>]` (bracketed white-score) alongside the zurichess `c9 "result"` format (sniff per line).
- [ ] **Step 6.3: The full joint tune.** All manifest params (~1000 pairs), 90/10 split, 300 epochs (early-stop report if val MSE rises 3 checks running): `cargo build --release && ./target/release/tune tools/data/<dataset> 300 0.05 > /tmp/eval_params_new.rs && cp /tmp/eval_params_new.rs src/eval/eval_params.rs` — NEVER redirect `cargo run` straight into eval_params.rs: the shell truncates the file before cargo compiles the library that includes it, destroying the params and failing the build (T4 incident; tune.rs doc-comment carries the same warning). REPORT dataset used, wall time, K, MSE trajectory. `cargo test` (value-dependent rebaselines per convention), bench twice, commit `feat(eval): full-scale joint Texel tune` + Bench.
- [ ] **Step 6.4 (CONTROLLER):** canary → SPRT #6 [0,5] vs `baseline-threats` → `tools/baseline.sh m5-tune`. (If H0: unlike M4's pipeline gate this one is NOT tolerated — a full retune that loses to its seed means something's wrong; stop and investigate, likely overfit or dataset format bug.)

**Step 6.5 (CONTINGENCY, user-specified 2026-06-06 — ran when SPRT #6 went negative with canary 269 = project high; "tactically sharper ≠ stronger engine"):**
On H0: T5 values stay baseline. Surgical revert — restore `eval_params.rs` from `021646d` (the VALUES are the failed claim; the parallel tuner, big3 loader, and download tooling from b030465 are infrastructure and STAY); rebuild, bench must return to 71571, one canary sanity (~267). Big3 is *useful failed evidence*: bigger data is promising, but the tuning setup needs scale/phase controls first. Investigation order (each candidate gets canary + fixed 400-game probe vs baseline-threats; only the best survivor gets the full frozen-protocol [0,5] SPRT):
1. **Refit K on big3, then retune** — the sanctioned *deliberate* re-anchoring act (NOT the banned silent per-run refit): does the mg-deflate/eg-inflate phase distortion collapse when K matches the corpus? Any shipping candidate still requires margin revalidation + fresh canary per the K-freeze law.
2. **Anchor material harder** — freeze or tightly bound P/N/B/R/Q mg/eg ratios (queen/rook especially) during the big3 tune.
3. **Phase-balanced big3 subsample** — equalize opening/middlegame/endgame distribution before tuning.
4. **Hybrid dataset** — curated zurichess + big3 mixed, weighted toward curated quiet positions.
5. **Static search-margin audit** — pruning trigger rates T5-values vs big3-values (RFP fired, futility skipped, null attempted, beta cutoffs, qnodes, fail-highs); counters behind a compile feature so the release path and Bench are untouched.
6. **Eval disagreement set** — sample positions where the two value sets differ >150cp; classify (material sacs, endgames, quiet conversion, king safety, passed pawns). Needs a tiny `eval` UCI debug command (print static eval) — useful permanently.
Diagnostics 5–6 run on existing artifacts and may run alongside 1–4 (serialized with any matches per the idle-system rule).

---

### Task 7: M5 wrap — upshifted anchored gauntlet + release

**Files:**
- Modify: `tools/get-anchors.sh`, `README.md`, `Cargo.toml`

- [ ] **Step 7.1: Upshift the anchor pool.** Extend get-anchors.sh with Stash v20 (~2509) and v21 (~2714) (same GitLab API pattern; UCI-verify). The M5 pool: **Stash v15 (2140), v17 (2298), v19 (2473), v20 (2509), v21 (2714)** — ratings.txt updated; drop Rustic/v13 (sub-2000 rungs are pure blowout against an M5 engine).
- [ ] **Step 7.2 (CONTROLLER): the measurement** — `tools/anchored-gauntlet.sh 300` alone (~90 min), Ordo pinned, row in docs/strength-log.md. The 2600 re-aim verdict lives here.
- [ ] **Step 7.3: Idle forfeit gauntlet** (200 games, zero) + **canary trend note** (tactics-log row for the final M5 binary).
- [ ] **Step 7.4: Docs/version** — README: tick M5, add `- [ ] M6: search & eval polish + bot readiness (book, Syzygy, time management, Lichess hardening)`; Cargo.toml 0.5.0; suggest the user redeploy the Lichess bot on 0.5.0 (their live rating is the field telemetry).
- [ ] **Step 7.5: Full local gate + push + CI green** — commit `docs: mark M5 complete, bump to 0.5.0` (no Bench line).

---

## Plan self-review notes

- **Spec coverage:** §6.2 full HCE list — tapered ✓(T1) pawn structure+pawn hash ✓(T2) mobility ✓(T3) king safety ✓(T4) bishop pair/rook files/threats/tempo ✓(T5); Texel-at-scale per §6.3 ✓(T6: bigger data, parallel, joint). User's seven layers mapped in the header (two explicitly emergent, honestly labeled). Anchored mini-gauntlet ✓(T7).
- **Type consistency:** `add_term(idx, sign, t, &mut mg, &mut eg)` promoted T2 and used by every term; `pawn_attack_set` defined T3 used T5; `file_mask` defined T4 used T5; manifest offsets appended in task order (ABI: append-only, enforced by the `PARAMS.len()==TOTAL_PAIRS` test failing on any drift).
- **Honest test policy carried forward:** trace-based unit tests assert FEATURE RECORDS (deterministic) not tuned values; structural eval tests survive except the stm-negation equality, which T5's tempo deliberately bounds instead (documented in-step).
- **Known costs:** mobility + threats recompute attack sets (eval gets ~2-4x slower; SPRT arbitrates; attack-cache refactor explicitly deferred). Six retunes at 648k are minutes each; T6's big tune is the long one.
- **Wall-clock:** 6 SPRT gates (4×[0,10] + 2×[0,5]) + 6 canaries + the 90-min gauntlet ≈ a full day of compute.

## Execution Handoff

Plan complete. Execute with superpowers:subagent-driven-development — established pipeline; CONTROLLER owns retune verification, canaries, SPRTs, gauntlet, ledgers.


