# NebChess — Engine Design Spec

**Date:** 2026-06-04
**Status:** Approved
**Repo:** https://github.com/N3bDev/nebchess

## 1. Overview

NebChess is a from-scratch UCI chess engine written in Rust.

**Target: 2400 CCRL Blitz** (stretch goal: 2500) with a hand-crafted evaluation (HCE).
The list is named deliberately: CCRL Blitz runs ~100-150 Elo higher than CCRL 40/15 for
the same engine, so "2400" without a list is meaningless. Calibration points from real
engine progressions: Rustic Alpha 1 (equivalent to our M2 feature set) measured 1675
CCRL; Leorik reached 2112-2566 across HCE versions. 2400 HCE is ambitious-but-reachable
with deep tuning. A phase-2 NNUE evaluation (out of scope for v1.0, designed-for in the
architecture) would push toward 2700+.

### Decisions locked with the user

- **Language:** Rust.
- **Move generation:** from scratch (bitboards + magic bitboards). No cozy-chess/shakmaty.
- **Evaluation:** HCE now, with a clean seam so NNUE can slot in as phase 2.
- **Interface:** UCI only. No built-in terminal/web UI. (lichess-bot bridge works with any
  UCI engine and needs no engine changes; out of scope.)
- **Opening book:** Polyglot `.bin` reader in the engine AND a `bookgen` tool that builds
  books from PGN databases.
- **Extras in scope:** Syzygy tablebase probing, Lazy SMP multithreading, pondering.
- **Methodology:** engine-first — reach a weak-but-complete playing engine quickly, then
  add features one at a time, each validated by SPRT self-play testing.

### Non-goals (v1.0)

- NNUE training/inference (phase 2; the eval seam is built for it, the network is not).
- Chess960 support (Polyglot castling decode handles the standard-chess case only).
- MultiPV > 1 *quality* (the root loop supports MultiPV structurally; no effort is spent
  tuning multi-line search).
- OpenBench deployment (single-machine fastchess is sufficient for a solo dev; OpenBench
  can be adopted later without design changes).
- 6/7-man tablebases (3-4-5 piece set only, ~939 MiB; 6-man is 149 GiB).

## 2. Crate layout

Single cargo package: library + two binaries.

```
nebchess/
  Cargo.toml
  src/
    lib.rs
    board/        # bitboards, position, make/unmake, zobrist, FEN, movegen, magics, perft
    search/       # iterative deepening, alpha-beta/PVS, qsearch, TT, ordering, time mgmt
    eval/         # Evaluator trait (NNUE seam), HCE implementation, PSQTs, pawn hash
    book/         # Polyglot reader: format, Polyglot-zobrist (separate from engine zobrist)
    syzygy/       # pyrrhic-rs adapter + probe gating
    uci/          # protocol parse/dispatch, option handling, info output
    main.rs       # UCI binary entry
    bin/
      bookgen.rs  # PGN -> Polyglot .bin builder
  tools/          # SPRT runner scripts, gauntlet configs, tuning pipeline
  tests/          # perft suite, UCI integration tests
  docs/
```

Rationale: one package keeps a single version/test surface; the binaries are thin shells
over the library so integration tests can drive the engine in-process.

## 3. Board representation

- **Bitboards:** one `u64` per piece type per color (12), plus per-color and global
  occupancy. Redundant 8x8 mailbox array (`[Option<Piece>; 64]`) for O(1) piece-on-square
  lookup.
- **Make/unmake** with an undo stack — NOT copy-make. Chosen because (a) it is the
  conventional layout for the search features below, and (b) the NNUE accumulator stack
  in phase 2 pushes/pops in the same places. Note: make/unmake does not by itself make
  NNUE fast — the per-ply accumulator stack does; make/unmake just gives it natural hook
  points.
- **Incremental Zobrist hashing** (engine keys — distinct from Polyglot keys, see §8).
- **Key history + halfmove clock** for repetition and 50-move detection. The history
  spans the game prefix (from `position ... moves ...`) plus the in-tree search path.
  Repetition check in the node prologue: scan back to the last irreversible move
  (capture/pawn move/castle), stepping by 2; twofold within the search tree scores as a
  draw. 50-move draw interacts with mate (mate on the boundary still wins). This plumbing
  lands in M2 — it touches the search node prologue and is expensive to retrofit.
- **Draw score:** small randomized jitter around zero (and a `Contempt` UCI option) to
  avoid threefold blindness in self-play pools.

## 4. Move generation

- Precomputed attack tables for knight/king/pawn.
- **Magic bitboards** (plain magics, fixed shift) for sliders. Magic constants are
  precomputed once and committed as `const` arrays — no startup search, deterministic
  builds. The slider attack getter is a single `#[inline]` function so a BMI2/PEXT
  implementation can be added later behind a cargo feature and A/B-benched without
  touching call sites.
- **Pseudo-legal generation + legality filtering** (king-safety check on make). Simple
  and perft-verifiable; can be optimized to fully-legal generation later if NPS profiling
  justifies it.
- **Perft correctness suite** (see §10) gates everything downstream.

## 5. Search

### 5.1 Thread architecture (designed now, used at M9b)

All mutable search state lives in a `SearchThread`:

```
SearchThread {
  board: Board (+ undo stack, key history),
  stack: [SearchStackEntry; MAX_PLY],
  killers, history tables, continuation-history tables,
  pv tables, node counter,
  // phase 2: accumulator_stack: [Accumulator; MAX_PLY]
}
```

Shared between threads: the transposition table, an `AtomicBool` stop flag, and atomic
node counters. Search functions take `&mut SearchThread` — never a global board. Lazy
SMP at M9b is then "spawn N SearchThreads," not a refactor.

`SearchStackEntry` carries at minimum: `static_eval` (enables the "improving" heuristic),
`current_move`, `killers`, and an `excluded_move` slot reserved for singular extensions.

### 5.2 Feature set

Core (M2-M4): iterative deepening; negamax alpha-beta with PVS; quiescence search with
SEE-based pruning; aspiration windows; transposition table cutoffs + TT-move ordering;
move ordering = TT move, MVV-LVA/SEE captures, killers, history heuristic; null-move
pruning; late move reductions; late move pruning; futility + reverse futility pruning
(margins scaled by `improving`); check extensions; mate-distance pruning; delta pruning
in qsearch.

Post-M4 polish (M6) — flagged by review as essential for 2400+: **singular extensions**
(uses the reserved `excluded_move`), **continuation history** (1-ply/2-ply move-pair
history; supersedes the countermove heuristic, which is deliberately omitted),
**main-search SEE pruning**, history-driven LMR adjustments.

### 5.3 Transposition table

- **10-byte entry:** 16-bit key fragment, 16-bit move, 16-bit score, 16-bit static eval,
  8-bit depth, 8-bit packed (2-bit bound | 6-bit generation/age).
- **Cache-line-aligned clusters of 3 entries** + replacement byte padding to 32 bytes;
  depth-preferred + aging replacement within the cluster.
- **Mate scores adjusted by ply** on store and probe (store as distance-from-current-node,
  reconstruct distance-from-root) — classic silent-corruption bug otherwise.
- **Cached static eval** in the entry feeds futility/RFP/improving without re-evaluation.
- **Rust aliasing:** entries are atomics-backed (`AtomicU64`-pairs or equivalent) so the
  shared-table SMP case is not UB. Single-thread path uses relaxed loads/stores —
  benchmark to confirm no measurable overhead pre-SMP.
- **Lockless SMP correctness:** key-XOR-data validation (Hyatt scheme), with the
  generation byte kept OUTSIDE the XOR-validated payload so in-place age refresh does not
  invalidate entries.
- TT moves are **legality-verified before use** (a key collision must never inject an
  illegal move). Probe API shaped to allow prefetching the child entry after computing
  the child key. Resize/clear is parallelized (a multi-GB table cleared single-threaded
  visibly stalls `ucinewgame`).

### 5.4 Time management

Specified to the failure modes that actually forfeit games:

- Soft limit (don't start a new ID iteration past it) and hard limit (abort search).
- Stop/clock check **every 2048 nodes** via shared `AtomicBool` — never per-node, never
  only at iteration boundaries (one deep iteration must not blow the flag).
- PV-instability and fail-low time extensions, bounded by the hard limit.
- UCI `Move Overhead` option subtracted per move for GUI/network lag.
- Handles `movestogo` (including 0/absent), `movetime`, very low clocks, and
  `go infinite`.
- **Acceptance test: zero time forfeits** over a fastchess smoke gauntlet. A single
  forfeit is a blocker bug, not noise.

## 6. Evaluation

### 6.1 The NNUE seam (the part the adversarial review redesigned)

A one-shot `fn eval(&Board) -> i32` trait is the wrong seam: NNUE is stateful and
incremental (accumulator updated by move deltas). The seam is four hooks:

```rust
trait Evaluator {
    fn refresh(&mut self, board: &Board);              // full rebuild
    fn on_make(&mut self, mv: Move, board: &Board);    // incremental update
    fn on_unmake(&mut self, mv: Move, board: &Board);  // incremental downdate
    fn evaluate(&mut self, board: &Board) -> i32;      // score, side-to-move relative
                                                       // (&mut: HCE writes its pawn hash)
}
```

The search calls `on_make`/`on_unmake` from M2 onward even though HCE implements them as
no-ops — the call sites must already exist when NNUE lands. Per-thread evaluator
instance; the HCE's pawn hash table is owned by the HCE evaluator (thread-local), so it
disappears cleanly when NNUE replaces it.

### 6.2 HCE terms

Tapered (midgame/endgame phase interpolation): material; piece-square tables; pawn
structure (doubled/isolated/passed) with a thread-local pawn hash; mobility; king safety
(pawn shield + attack units); bishop pair; rook on open/semi-open file; threats; tempo.

### 6.3 Texel tuning

- Loss: MSE of `result − sigmoid(K · eval)`; fit scaling constant K once by line search,
  then freeze.
- **Optimizer: analytic gradients + Adam** (lr ~0.1-0.3 with LR-drop scheduling), per the
  Ethereal methodology — not slow coordinate/local search.
- **Datasets:** start with zurichess `quiet-labeled.epd` (725k quiet positions, MIT,
  mirror: github.com/KierenP/ChessTrainingSets) to validate the pipeline; final tune on
  `lichess-big3-resolved` (~9.7M pre-resolved positions,
  archive.org/details/lichess-big3-resolved.7z; no explicit license — attribute, don't
  redistribute).
- Train/validation split to detect overfit. **Tuned weights ship only if they pass SPRT**
  — lower loss does not imply Elo.
- A lightweight PST-only tuning pass runs early (M4) to de-risk the pipeline long before
  the high-stakes M5 tune. Dataset acquisition is a tracked pre-M5 task.

## 7. UCI layer

Full protocol: `uci`, `isready` (answers `readyok` even mid-search), `ucinewgame`
(clears TT), `position startpos|fen ... moves ...` (full replay every time — GUIs resend
the whole game), `go` with `wtime/btime/winc/binc/movestogo/depth/nodes/movetime/
infinite/ponder`, `stop`, `ponderhit`, `setoption`, `quit`.

Options: `Hash`, `Threads`, `Ponder`, `OwnBook`, `BookFile`, `SyzygyPath`, `MultiPV`,
`Move Overhead`, `Contempt`.

Edge cases promoted to explicit M2 acceptance tests (each historically forfeits games):

- Move-list replay correctness, verified by feeding long games and diffing the resulting
  FEN against a reference.
- Long-algebraic parsing round-trip incl. promotions (`e7e8q`), castling, en passant.
- `go` followed by immediate `stop` still emits a **legal** `bestmove` (never
  `bestmove (none)` in a non-terminal position).
- Pondering state machine (M9a): `go ponder` → `ponderhit` continues the search under
  real time management; `go ponder` → `stop` discards and answers for the actual
  position. Dedicated state-machine test asserts both branches return legal moves with
  no time loss.

The root search loop is structured for MultiPV from the start (it constrains root-move
iteration and `info` output), defaulting to 1.

## 8. Opening book

### 8.1 Reader (in-engine)

Polyglot `.bin`: 16-byte big-endian entries `{u64 key, u16 move, u16 weight, u32 learn}`,
sorted by key, binary-searched. Probed when `OwnBook=true`, weighted-random selection by
weight. Interop traps (verified against the canonical spec + python-chess reference,
which we mirror exactly):

- **Polyglot Zobrist ≠ engine Zobrist.** Fixed 781-constant array: [0..768) piece-square,
  [768..772) castling rights, [772..780) en-passant file, [780] side-to-move.
- **En-passant hashing rule:** hash the EP file only if an enemy pawn is adjacent to the
  double-pushed pawn — capturer existence, **ignoring pin legality**. Naive "FEN has an
  EP square" hashing breaks book compatibility.
- **Castling moves are encoded king-to-rook-square** (e1h1/e1a1/e8h8/e8a8) and must be
  remapped to our internal castling representation.
- Move word 0 is a null sentinel — ignore. Promotion code: 0=none, 1=N, 2=B, 3=R, 4=Q.
- Generated keys are validated against a known-good book before the builder is trusted.

### 8.2 Builder (`bookgen`)

PGN → `.bin`, mirroring `polyglot make-book` semantics: `-max-ply` (default 16),
`-min-game` (default ~20-50), weight = 2·wins + draws (from the move's games), scaled
globally to u16. **Source data: Lichess Elite Database** (database.nikonoel.fr — CC0,
maintained monthly, pre-filtered to White 2500+/Black 2300+, bullet excluded; recent
12-24 months ≈ 60-97 MB/month).

The engine's own book is **never** used for SPRT/gauntlet testing (see §10.3).

## 9. Syzygy tablebases

- **Implementation: `pyrrhic-rs`** (pure-Rust Fathom/Pyrrhic port, crates.io). Its
  `EngineAdapter` trait wants six attack functions on raw `u64` bitboards — exactly what
  our movegen exposes. No C toolchain, no board-type conversion. (Fathom-via-FFI is the
  fallback if pyrrhic-rs proves deficient; the probe sits behind our own small trait so
  the backend can swap.)
- **Interior nodes: WDL only**, gated by: piece count ≤ TB max, no castling rights,
  halfmove-clock/probe-depth conditions, fail-soft on probe error.
- **Root: DTZ probing** to select moves that convert wins under the 50-move rule.
- Data: 3-4-5 piece set (~939 MiB; WDL-only ~378 MiB) from tablebase.lichess.ovh or
  tablebase.sesse.net. Expectation set deliberately: low-tens of Elo at our level — this
  is a correctness/completeness feature, not a strength pillar.

## 10. Testing methodology

### 10.1 Perft (gates M1)

Enumerated suite committed to the repo with expected node counts — not "startpos and
Kiwipete etc.": the six standard CPW positions (startpos d6/d7, Kiwipete d5, position 3
EP/promotion, positions 4 + mirror, 5, 6) plus edge-case positions covering EP-into-check
(pinned EP capturer), castling through/into/out of check, promotion while in check,
double check. Plus bulk cross-validation of a few thousand random reachable positions
against a reference engine (catches compensating-error coincidences that node totals
alone miss). Runs in CI on every commit touching movegen.

### 10.2 Bench (non-regression fingerprint)

`nebchess bench`: fixed position list, **fixed depth**, threads=1, fixed TT state →
bit-reproducible total node count + NPS. The node count goes in **every commit message**
(Stockfish convention). CI re-runs bench and fails if a commit's stated count doesn't
match, or if a commit claiming "non-functional" changes the count. Bench stays
single-threaded forever (SMP is nondeterministic by design).

### 10.3 SPRT protocol (frozen; committed to `tools/`)

- Tool: **fastchess** (Fishtest's own runner; dep-free build).
- STC 8+0.08, LTC 60+0.6; fixed Hash; threads=1.
- Bounds: gainers `[0, 5]` (early project, while gains are big: `[0, 10]`);
  simplifications `[-5, 0]`; α=β=0.05; runs terminate at the LLR boundary — never by eye.
- **Any patch touching pruning/reductions/extensions that passes STC is re-verified at
  LTC before its Elo is claimed** (hyperfast TC systematically flatters pruning).
- Openings: balanced `8moves_v3.pgn` while the engine is weak → `UHO_Lichess_4852_v1.epd`
  once strong (both from github.com/official-stockfish/books, CC0); every opening played
  as a reversed-color pair; frozen adjudication: `-resign movecount=3 score=600`,
  `-draw movenumber=40 movecount=8 score=10`; `-recover -repeat -randomseed
  -report penta=true`; concurrency = physical cores − 1.
- Reference command template lives in `tools/sprt.sh`. Changing the test book or TC
  invalidates cross-version comparison and is treated as a protocol version bump.
- Throughput honesty: borderline patches need 10k-40k games — overnight runs are
  expected and planned for, not worked around by truncating tests.

### 10.4 Absolute Elo (the 2400 claim)

Self-play SPRT measures *relative* gain only (and self-play deltas overstate rating-list
deltas, commonly ~1.5×). Absolute claims come **only** from gauntlets:

- Ladder bracketing the target, single source plus cross-author checks:
  Stash v17 (~2298), v18 (~2390), v19 (~2473), v20 (~2509) [gitlab.com/mhouppin/stash-bot
  prebuilt Linux binaries] + Leorik 2.0/2.1 (~2537/2566) + Blunder 8.5.5 (~2695) +
  one low anchor (Rustic Alpha 2 ~1815 or Fairy-Max ~1891).
- fastchess gauntlet: TC 10+0.1, ponder off, 1 thread, fixed hash, neutral book,
  ≥1000 games per pairing.
- Rated with **Ordo**, anchors pinned to published CCRL Blitz ratings (`-a/-A`).
- Every published number carries its 95% CI, opponent list, versions, and TC. Re-pull
  live CCRL ratings at publish time. Stated plainly as a CCRL-Blitz-anchored estimate
  (CCRL ≠ FIDE).

### 10.5 Non-SPRT correctness gates

- **TT validation mode** (debug builds): every TT move legality-checked; TT-on vs TT-off
  search comparison on a position suite; mate-score positions in bench to catch ply
  adjustment bugs.
- **Zero-time-forfeit gauntlet** at M2 and re-run when time management changes.
- **UCI edge-case integration tests** (§7) in CI.
- **SMP validation** at M9b: ThreadSanitizer pass, torn-read stress test on the lockless
  TT, SPRT at equal wall-clock with fixed thread counts.

### 10.6 CI (GitHub Actions)

Pinned toolchain; `cargo build --release`, `cargo test` (unit + perft + UCI integration),
deterministic bench asserted against the commit's stated count, `clippy` + `fmt`, plus a
debug-assertions build for the unsafe magic/TT paths. **SPRT and gauntlets never run in
CI** — they are offline gates on dev hardware; CI gates correctness only.

## 11. Multithreading & pondering

- **M9a Pondering** (independent, lower-risk, ships first): `go ponder` search on the
  predicted move; `ponderhit` → continue under live time management; `stop` → answer
  immediately. Gated by the state-machine test in §7.
- **M9b Lazy SMP:** N `SearchThread`s, shared TT (already lockless by design), shared
  stop flag, per-thread everything else; helper threads search with staggered depths.
  Main thread owns time management and the final move choice. Validated per §10.5.

## 12. Roadmap

Milestone Elo figures are **informational expectations, not gates** (re-based against
measured progressions: Rustic A1 1675, A2 1815; Leorik 1.0 2112). Gates are the perft
suite, the correctness suites, and per-feature SPRT.

| # | Milestone | Gate | CCRL-ish |
|---|-----------|------|----------|
| M0 | Scaffolding, CI, fastchess + opening suites + Ordo installed, SPRT scripts | CI green; harness runs a 2-engine smoke match | — |
| M1 | Board, FEN, Zobrist, magics, movegen, make/unmake | Full perft suite + cross-validation | — |
| M2 | Minimal engine: ID + AB + qsearch, material+PST, repetition/50-move, full UCI, time mgmt | UCI edge-case tests; zero-time-forfeit gauntlet; plays legal chess | ~1500-1700 |
| M3 | TT cuts → TT-move ordering → killers → history → PVS (each its own SPRT) | TT validation mode; per-feature SPRT | ~1800-1900 |
| M4 | Null move → LMR → aspiration → futility/RFP (each SPRT'd); PST-only Texel pass to de-risk pipeline | per-feature SPRT | ~2000-2150 |
| M5 | Full HCE terms; full Texel tune (data acquired beforehand) | SPRT on tuned weights; gauntlet re-anchor | ~2200-2400 |
| M6 | Singular extensions, continuation history, main-search SEE pruning, LMR polish | per-feature SPRT | toward target |
| M7 | Polyglot reader + bookgen (Lichess Elite data) | key validation vs known-good book; SPRT with book on/off documented | — |
| M8 | Syzygy via pyrrhic-rs (WDL interior, DTZ root) | endgame suite; SPRT (small/flat result acceptable) | small |
| M9a | Pondering | state-machine test; no-forfeit gauntlet | practical |
| M9b | Lazy SMP | TSan + torn-read stress; equal-time SPRT at fixed threads | practical |
| M10 | v1.0: official gauntlet vs anchor ladder, Ordo + CI report, README, release binaries | published 2400-claim with 95% CI | 🎯 |

NNUE is phase 2, after v1.0, slotting into the §6.1 seam.

## 13. Risks & mitigations

| Risk | Mitigation |
|------|------------|
| Magic/movegen bugs poisoning everything downstream | Perft suite + random cross-validation before any search work (M1 gate) |
| Zobrist/TT corruption (collisions, mate-score ply, illegal TT moves) | TT validation mode, legality re-check on probe, mate positions in bench |
| Time forfeits at fast TC | Node-cadence clock checks, Move Overhead, zero-forfeit acceptance gauntlet |
| Hyperfast-TC overestimating pruning gains | Mandatory LTC re-verification for pruning patches |
| Texel overfit / loss-vs-Elo divergence | Validation split + SPRT gate on tuned weights |
| Polyglot interop breakage (EP rule, castling encoding) | Mirror python-chess exactly; validate keys vs known-good book |
| Self-play Elo inflation feeding the 2400 claim | Absolute claims only via anchored gauntlets with CIs (§10.4) |
| 2400 misses despite clean process | Re-baselined milestone Elo; M6 search polish + deeper tuning iterations are the lever; NNUE phase 2 is the overflow valve |

## 14. Reference index

- Testing: github.com/Disservin/fastchess · github.com/official-stockfish/books (CC0) ·
  Ordo (Miguel Ballicora) · dannyhammer.github.io/engine-testing-guide
- Anchors: gitlab.com/mhouppin/stash-bot/-/releases · github.com/lithander/Leorik ·
  github.com/deanmchris/blunder · ccrl.chessdom.com/ccrl/404/
- Tuning: github.com/KierenP/ChessTrainingSets (quiet-labeled.epd, MIT) ·
  archive.org/details/lichess-big3-resolved.7z · github.com/GediminasMasaitis/texel-tuner
  (methodology reference) · chessprogramming.org/Texel's_Tuning_Method
- Book: hgm.nubati.net/book_format.html · python-chess `chess/polyglot.py` (reference
  implementation) · database.nikonoel.fr (Lichess Elite, CC0)
- Tablebases: github.com/Algorhythm-sxv/pyrrhic-rs · tablebase.lichess.ovh ·
  chessprogramming.org/Syzygy_Bases
- General: chessprogramming.org (CPW) — perft positions, search/eval technique articles
