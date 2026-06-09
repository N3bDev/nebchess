# M8 Design: NNUE Evaluation (supervised self-play distillation, lean Rust runtime)

**Approved:** 2026-06-08 (brainstormed with N3bDev on the desktop post-migration; research-grounded across peer from-scratch engines, the `bullet`/Blackwell toolchain, the data recipe, and Rust runtime inference — see "Research grounding" footnotes).
**Context:** v0.7.1 at **2827.8 ± 15.7** anchored (10+0.1, 3-family CCRL pool) and ~2400 live Lichess blitz. The hand-crafted eval (HCE) is near its ceiling; M6 anchored deltas had shrunk to tens. TimeBrain-V2 was de-scoped after the hard-cap gate came back **−55.6 ± 23.3** at 8+0.08 — NebChess is *time-elastic* (more search time genuinely produces stronger moves), so banking time trades away strength, and the clock leak is partly inherent to a time-elastic HCE engine. **NNUE is the ceiling-raiser for the 3000+ goal AND the time-elasticity fix** (a strong net reaches good moves at lower depth, less dependent on deep search). Both peer cutecassia and the ~3000 Patricia-5 are NN-eval; from-scratch HCE realistically tops out below 3000.

## Goal & scope (bounded milestone)

M8 = build the full NNUE pipeline and land **ONE SPRT-validated first net** at a simple, proven architecture, shipped as a version. The climb to the ceiling — bigger hidden layer, output buckets, king buckets, deeper output layers, data regeneration — is **M9+**, each a gated version bump matching the existing cadence. The ceiling is still the goal; this milestone reaches it *safely* by proving the whole pipeline end-to-end on the simplest net first.

**Honest Elo expectation:** a first NNUE over a *tuned* HCE adds roughly **+100–250 Elo** — the lower-middle of the field's range (the gain is baseline-dependent: akimbo got +388 over a minimal HCE, Stockfish only ~+80 over its elite HCE; NebChess's 2827 HCE is strong, so plan for the middle, not the top). That likely clears **~2950–3050** out of the gate. The path past 3000 is the M9+ iteration loop plus the real ceiling levers — **deeper output net + output buckets + king buckets, not raw width** (width gains fade hard past 1024 and shrink further at long time control).

## Decisions locked in brainstorming

1. **Data strategy — own self-play datagen.** Shipped nets are trained *only* on NebChess self-play, labeled by NebChess's own search. This is supervised distillation of our *own* eval — NOT the AlphaZero-style reinforcement learning deferred earlier. Chosen over SF18 distillation for two reasons aimed at the ceiling: (a) self-play data compounds through a datagen→train→promote→regenerate loop and is uncapped by any teacher; (b) provenance — a net trained on Stockfish labels is widely treated as non-original (CCRL/community), and whether GPLv3 reaches NN weights is legally unsettled. Every peer "zero" engine (Viridithas, Stormphrax, Smallbrain, Altair) trains exclusively on its own self-play and refuses third-party data. **SF18 stays out of all shipped nets**; it remains available only as an optional cold-start accelerant for a *throwaway* net-0 if the first self-play net ever disappoints.
2. **Architecture v1 — `(768 → H)×2 → 1`, SCReLU, perspective, no buckets**, int16, `QA=255 / QB=64 / SCALE=400`. The Carp/akimbo-proven first-net config. **H is an empirical knob** (sweep {512, 768, 1024}, SPRT the winner; default penciled at **768**). HalfKA is explicitly rejected — the field moved *away* from it to 768+buckets; nobody starts a new engine on HalfKA.
3. **Trainer — `bullet`** (jw1912, Rust/CUDA), used as an **offline-only** dependency. Toolchain de-risk resolved favorably at the source level: bullet compiles kernels at runtime via NVRTC with dynamic `sm_{major}{minor}` detection, so it targets the 5080's native **`sm_120` automatically, no hardcoded arch**; it only needs CUDA toolkit **≥12.8** (the box reports 13.1) and WSL2 is explicitly handled in its `build.rs`. nnue-pytorch is the fallback only if Step 0 surprises us.
4. **Runtime — lean.** Only `NnueEvaluator` + the embedded net ship; datagen is a dev bin, the trainer is a separate excluded crate. Net = `quantised.bin` via `include_bytes!` + hand-rolled inference. Engine stays std-only + the one pinned dep (pyrrhic-rs).
5. **HCE disposition — kept in-tree**, compilable via the instantiation site (a build-time choice, not a runtime UCI toggle): it's the SPRT baseline (we need it to validate NNUE), a fallback, and bench-history continuity. NNUE becomes the *default* only after it SPRTs positive. Pure NNUE eval, **not** an HCE+NNUE blend.

## Structure: a de-risk spike + 4 plans, one release

The whole milestone hangs off the **`Evaluator` seam designed in the master spec (2026-06-04, §6.1)** — `refresh`/`on_make`/`on_unmake`/`evaluate`, already wired unconditionally from M2 and no-op'd by HCE. Search is generic over `<E: Evaluator>`, so swapping in NNUE is **zero search refactoring**. Plans numbered plan-9+ per convention; each is subagent-driven (implementer → combined spec+quality review per commit → controller-owned gate).

| # | Sub-project | Gate | Depends on |
|---|---|---|---|
| **0** | **bullet smoke-test on the 5080** (spike, ~hours) | trains a superbatch on `sm_120` end-to-end → **go/no-go on bullet** (else pytorch fallback); yields a toy net + frozen arch/format | — |
| **plan-9 (A)** | **`datagen`** binary | produces clean labeled self-play data; score/WDL distribution + legality sanity-checked | 0 |
| **plan-10 (B)** | **trainer crate + first real net** | training loss converges; `quantised.bin` exports; matches the net contract | A |
| **plan-11 (C)** | **`NnueEvaluator`** runtime | numeric parity ≡ bullet reference (exact), incremental == refresh property test | 0 (toy net) |
| **plan-12 (D)** | **integrate + validate + ship** | **SPRT 8+0.08 vs HCE passes** → anchored gauntlet → version bump | B, C |

**Critical path: `0 → A → B → D`.** C is the wildcard: it needs only the frozen net contract + the Step-0 toy net, so the highest-risk hand-rolled code (SIMD/quantization/accumulator) can be **built and parity-proven early, in parallel with A/B**, keeping it off the critical path and making D a clean "embed + validate." Authoring order for C is flexible; everything else is sequential.

## Invariants (unchanged project law)

Frozen SPRT protocol v1 (`tools/sprt.sh`; 8+0.08, Hash 16, 8moves_v3, α=β=0.05; **the NNUE gate is the big-change [0,10]**). WAC canary alone before the SPRT (movetime-noisy — only the −10 floor is signal; the SPRT is the arbiter). Review-every-step (combined spec+quality review per commit, including provably-safe refactors and the offline trainer crate). Probes (`tools/probe.sh`) rank candidates; only full SPRT admits to baseline. Wandering SPRTs stopped honestly. Idle-system measurement discipline — no datagen/training/builds during a canary/SPRT/gauntlet. The dev loop optimizes self-play only; the anchor pool is a measurement instrument and never enters the feedback loop.

**NNUE-specific additions to project law:**
- **Provenance rule:** shipped nets are trained ONLY on NebChess self-play (optionally + NebChess-rescored human positions in M9+); never on another engine's eval. Preserves originality for CCRL/community listing.
- **Bench is net-tied:** the bench fingerprint reflects the shipped evaluator + embedded net, so *every new net changes `Bench:`*. The commit that swaps the net carries the new `Bench:` line; CI (`check-bench.sh`) enforces it. Not a regression.
- **The net contract** (below) is a single shared spec between the trainer and the runtime; both ends are guarded (compile-time size assert + numeric parity test).

## Step 0 — Toolchain de-risk spike

Before building anything, run bullet's `simple` example (`cargo r -r --features cuda --example simple`) on a tiny dataset in WSL2 and confirm: it detects the 5080, NVRTC compiles `sm_120` without error, and it trains ≥1 superbatch. Prereqs: latest NVIDIA *Windows* driver (have it — reports CUDA 13.1), CUDA toolkit ≥12.8 in WSL (`wsl-ubuntu`, no bundled driver), Rust 1.87+/edition 2024 (1.96.0 covers it). **Gate: trains a superbatch → commit to bullet.** If it fails, pivot to nnue-pytorch (cu128+ wheels ship sm_120) before proceeding. Output: a throwaway toy net in the agreed format that unblocks plan-11's parity work.

## plan-9 (A) — `datagen` binary

New bin in the `nebchess` crate (alongside `tune`/`solve`/`perft`), using the engine's own search and board. Dev-only; does not enter the shipped runtime.

- **Prereq:** add a **soft-node limit per move** to `limits.rs`/search (abort at the next depth boundary past a node budget — uniform per-move cost, the modern datagen preference over fixed depth). Small, isolated.
- **Per game:** 8 random legal plies from startpos (no book, for diversity) → self-play with both sides at **~5,000 soft nodes/move** → record each kept position with `cp` = that move's white-relative search score; after game end, attach `wdl` = white-relative result.
- **Adjudication:** resign on large sustained eval, draw on long balanced runs, **and Syzygy TB adjudication** (synergy with existing pyrrhic-rs support — exact WDL the moment a position reaches ≤5 men, instead of noisy played-out endings).
- **Filters at record time** (smart-fen-skipping — the net learns *quiet* eval; search handles tactics): skip if side-to-move in check; skip if best move is a capture/non-quiet; skip saturated/mate scores; drop the random-opening plies.
- **Output:** plain-text `FEN | cp_white | wdl_white` shards, one per worker. Conversion → shuffle → interleave into bullet's binary format is done offline by `bullet-utils` — keeps the **engine crate dependency-free** (no `bulletformat` dep) and the data trivially debuggable. Switch to direct binary output only if disk I/O bites at the 1B+ scale (M9+).
- **Parallelism:** ~22 of 24 cores, independent **seeded** self-play per worker → reproducible runs (determinism discipline). Throughput puts the **v1 ~100M-position target at a few hours**.
- **Gate:** generate a sample; sanity-check position counts, score/WDL distribution, no illegal/in-check leaks, filter rates. (Not an SPRT — datagen produces data, not Elo.)

## plan-10 (B) — trainer crate + first real net

A **standalone cargo project at `tools/trainer/`, excluded from the engine workspace** (so `cargo build --release` of the engine never links CUDA/bullet). Depends on `bullet = { git = ".../bullet", package = "bullet_lib" }`.

- **Data prep (no GPU):** `bullet-utils` converts the datagen text shards → bulletformat binary, then **shuffles + interleaves** (the primary overfitting defense).
- **Training config** (bullet `ValueTrainerBuilder`): `Chess768` + `dual_perspective`; `l0 = affine(768, H).screlu()`; `l1 = affine(2H, 1)`; output `l1(stm_h.concat(ntm_h))`; loss `sigmoid().squared_error(target)`; **AdamW**.
- **Recipe** (see "Training recipe" below): WDL target, lambda ≈ 0.8, `eval_scale ≈ 400`, single epoch + weight decay.
- **Export:** `quantised.bin` per the net contract.
- **Gate:** loss converges (held-out test set), net exports, sanity eval on a few FENs is plausible. Train on the real ~100M-position datagen output; sweep H ∈ {512, 768, 1024} (cheap on the 5080) so plan-12 SPRTs the best.

## plan-11 (C) — `NnueEvaluator` runtime (`src/eval/nnue/`)

The only new code in the shipped binary. Implements the `Evaluator` trait. Modeled on Carp's per-ply accumulator stack (maps 1:1 to the seam) + akimbo's SIMD forward pass.

- **Net loading:** `#[repr(C)] struct Network { feature_weights: [[i16; H]; 768], feature_bias: [i16; H], output_weights: [[i16; H]; 2], output_bias: i16 }`; `static NET = transmute(*include_bytes!(...))`; `const _: () = assert!(size_of::<Network>() == <bytes>.len())`.
- **Accumulator + stack:** `#[repr(C, align(64))] Accumulator { white: [i16; H], black: [i16; H] }`; `stack: [Accumulator; MAX_PLY+1]` + `top`. `refresh` rebuilds `stack[0]` from all pieces; `on_make` = `top+=1`, copy `stack[top-1]→stack[top]`, apply the move's feature deltas; `on_unmake` = `top-=1` (pointer decrement, no recompute).
- **Feature index** for piece (type 0–5, color) on `sq`: white half `(color==B)*384 + type*64 + sq`; black half `(color==W)*384 + type*64 + (sq^56)`.
- **Incremental deltas (Position stays eval-agnostic):** `on_make` decodes the `Move` into add/sub toggles — sub `from`, add `to`; capture also subs the captured square (captured piece read from the undo info `pos` already keeps for `unmake`); promotion swaps pawn→piece; castling also moves the rook. Same piece add/remove points Zobrist already hooks.
- **Forward pass:** `us = stm half, them = other`; `sum = Σ screlu(usᵢ)·ow[0]ᵢ + screlu(themᵢ)·ow[1]ᵢ`; eval `= (sum/QA + output_bias)·SCALE/(QA·QB)` → side-to-move-relative cp. `screlu(x) = clamp(x,0,QA)²`. Scalar path: clamp + i32 accumulate. AVX2 path: `madd(v, mullo(v,w))` (computes `v·(v·w)`, dodging i16 overflow); `#[cfg(target_feature="avx2")]` with the scalar fallback as the correctness reference. (Arrow Lake = AVX2, no AVX-512.)
- **Null move:** for a plain-768 net a null changes no piece-square feature (only stm + ep, neither an input), so the accumulator is unchanged and the perspective flip is read from `pos` at eval time. **Verify in code** whether search invokes the eval hooks around null moves; if it does, guard them to no-op; if not, nothing to do.
- **Insufficient material:** decide whether to keep HCE's short-circuit-to-0 (KvK etc.) or let the net predict; default = keep the short-circuit (cheap, exact).
- **Gates:** (1) compile-time size assert; (2) **numeric parity** — Rust `evaluate()` matches bullet's reference integer eval exactly on a FEN set (proves inference ≡ trainer); (3) **incremental == refresh** property test over random move sequences; (4) NPS measurement vs HCE.

## plan-12 (D) — integration, validation, ship

- Snapshot the HCE binary as baseline (`tools/baseline.sh nnue-base`), flip `uci/mod.rs:374` from `Hce::new()` to `NnueEvaluator::new()`, embed the plan-10 net. HCE stays compilable. No runtime eval-switch option (SPRT is binary-vs-binary; the HCE baseline is the pre-flip snapshot — fits `baseline.sh`).
- **Bench refresh:** capture the new fingerprint; the swap commit carries `Bench: <n>`.
- **Gates, in order:** WAC canary (floor only) → **SPRT 8+0.08 vs the HCE snapshot, bounds [0,10]** (the arbiter; a +100–250 change resolves fast) → **anchored gauntlet** (`tools/anchored-gauntlet.sh`) for absolute rating → log to `strength-log.md` → robustness (`uci-torture`, `krk-stress`; NNUE + Syzygy must not regress mating/endgames).
- Version bump, README M8 line, `sprt-log.md` row, memory update, tag, push, CI, bot-redeploy suggestion.

## Training recipe (shared reference)

Grounded in current peer practice; lambda/scale are tuning knobs, not first-principles constants.
- **Target:** WDL-space (0/0.5/1), **lambda ≈ 0.8** (eval-dominant blend of search-cp and game-result), **`eval_scale ≈ 400`** (fit to NebChess's eval→winrate curve once games exist). Keep this distinct from any output-display normalization constant.
- **Volume:** v1 **~100M** positions; M9+ strong net ~1B; ceiling push 2–4B. **Single epoch + weight decay**, shuffle/interleave hard (strength tracks fresh data volume, not epochs).
- **Source:** 100% standard self-play for v1. M9+ adds a DFRC slice (~20%, needs FRC-castling support first) and optionally a minority of NebChess-*rescored* human positions for opening diversity.

## The net contract (trainer ↔ runtime interface)

Little-endian, column-major, raw quantised weights, no header, padded to 64 bytes (bullet's `quantised.bin`). Field order = byte order:
`feature_weights [768 columns × H, i16 @ QA] · feature_bias [H, i16 @ QA] · output_weights [2 × H, i16 @ QB; index 0 = side-to-move/us, 1 = opponent/them] · output_bias [i16 @ QA·QB]`. Constants `QA=255, QB=64, SCALE=400`. Guarded by the compile-time size assertion (plan-11) and the numeric parity test against bullet's reference eval. **Any architecture change (H, buckets, layers) is a contract change** — both ends update together, in lockstep, as a single reviewed unit.

## Success criteria

- **Step 0:** bullet trains a superbatch on the 5080 (`sm_120`) — toolchain committed.
- **plan-9:** datagen produces legality-clean, well-distributed labeled data at the v1 volume.
- **plan-10:** a trained `quantised.bin` that converges and honors the contract.
- **plan-11:** exact numeric parity with bullet's reference; incremental == refresh; NPS hit quantified.
- **plan-12:** **SPRT 8+0.08 H1** (NNUE > HCE), anchored gauntlet logs a material jump (target band ~2950–3050), zero new forfeits, version shipped.
- Live Lichess field telemetry corroborates the gauntlet trend after redeploy.

## Out of scope (M9+, the climb to the ceiling)

Bigger H; **output buckets** (8, by piece count) + **deeper output net** (`→16→32→1`) — the field's biggest ceiling lever; **king buckets** (4→16, horizontal-mirrored, merged king planes) + finny-table refresh cache; DFRC datagen slice; the **iteration loop** (datagen with the stronger net → retrain → promote → regenerate the full dataset on major jumps); lazy/dirty accumulator updates; threat input features and frontier activations (Swish/SwiGLU) as research items. Each is its own gated milestone/version.

---
*Research grounding (2026-06-08, four parallel research passes, sources in the brainstorming transcript): peer architectures + Elo-per-choice (Carp/akimbo/Stormphrax/Viridithas/Obsidian/Stockfish); `bullet` source-level Blackwell/sm_120/WSL2 verification; distillation/self-play data recipe (nnue-pytorch, Smallbrain, Viridithas, TalkChess, arXiv 2412.17948); Rust runtime inference patterns (akimbo/Carp/Viridithas/Stormphrax source). Two items to re-verify live before relying on them: current Stockfish-master arch drift, and the late-Jan-2026 Swish>SCReLU result — neither affects the v1 plan.*
