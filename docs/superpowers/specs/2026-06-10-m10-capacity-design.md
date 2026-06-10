# M10 Design — NNUE Capacity Ladder (768 → 1024 → +8 Output Buckets)

**Status:** Design approved 2026-06-10 (brainstorming). Next: implementation plan.

**Goal:** The first architecture step beyond the v1 net: grow the hidden layer 768→1024 and add 8 output buckets, trained on flywheel-turn-2 data — with each lever isolated behind its own SPRT so we learn exactly what each is worth.

**Context:** 0.9.0 = 3131.1 ± 17.4 anchored (M9: the data flywheel, net2, +141.7 on the unchanged 768 arch). Two capacity signals motivate M10: training loss *rose* on the better data (0.00374→0.00417 — a 600k-param net straining to fit a richer target), and every engine in our strength class runs 1024+ hidden with output buckets. **Flywheel turn 2 is running** (~150M positions self-played by net2, 20 threads, seed 1, nodes 5000 → `tools/data/selfplay-net3`); all M10 nets train on that data.

---

## 1. The ladder (three isolated levers, sequential builds)

| Rung | Net | Isolates | SPRT vs | Bounds |
|---|---|---|---|---|
| **net3a** | 768, turn-2 data | flywheel turn 2 | `tools/bin/baseline-net2` (snapshotted, Bench 51395) | 8+0.08 [0,10] |
| **net3b** | 1024, same data | capacity | best-so-far | 8+0.08 [0,10] |
| **net3c** | 1024 + 8 output buckets, same data | output buckets | best-so-far | 8+0.08 [0,5] (refinement-class) |

- Each rung is a **sequential build**: constants change, the rung's net embeds via `include_bytes!`, the previous winner is snapshotted with `tools/baseline.sh`. The runtime supports ONE architecture at a time — no multi-arch machinery (YAGNI).
- "Best-so-far" chaining: net3b gates against whichever of {net2, net3a} won; net3c against whichever of {…, net3b} won. A flat/H0 rung is documented honestly in sprt-log and the ladder continues — each lever is judged independently. H0-but-positive → bring the numbers to the user and decide (the standing M9 contingency rule).
- **Only the final winner ships** (version 0.10.0): one anchored gauntlet (`tools/anchored-gauntlet.sh`, 300/pairing) against the **unchanged M9 pool** — the 0.9.0 = 3131.1 row stands as the comparison; no re-baseline needed.
- Expectation setting (not gates): turn 2 ≈ +20–50, capacity ≈ +20–40 net of NPS tax, buckets ≈ +10–25 self-play; anchored compression applies.

## 2. Net contracts (empirically verified with toy nets BEFORE engine code — the M8 pattern)

All quantization unchanged: **SCReLU, QA=255, QB=64, SCALE=400, i16 weights, LE, column-major, padded to 64 B** (bullet `quantised.bin`).

- **net3a** — unchanged v1 contract: `feature_weights[768×768] + feature_bias[768] + output_weights[2×768] + output_bias[1]` = **1,184,320 B**.
- **net3b (H=1024)** — `feature_weights[768×1024] + feature_bias[1024] + output_weights[2×1024] + output_bias[1]`: raw 1,579,010 B → padded ≈ **1,579,072 B**. (Exact padded size CONFIRMED from a toy net before the runtime asserts are written.)
- **net3c (H=1024, B=8)** — output side becomes per-bucket: `output_weights[8][2×1024] + output_bias[8]`: raw 1,607,696 B → padded ≈ **1,607,744 B**. (Same toy-net confirmation; the bucket-major vs hidden-major layout of the output weights is read from bullet's saved format, not assumed.)
- **Bucket function:** `idx = (popcount(occupied) − 2) / 4` → 0..7 (2 pieces→0, 32→7). This is the standard convention and what we EXPECT bullet to use — but the authoritative formula is **source-verified in bullet** (the discipline that source-verified Chess768 in M8). If bullet's differs, bullet's wins (trainer and runtime must agree).
- **i32 accumulation kept** (scalar and AVX2-madd paths): at H=1024 the theoretical worst case grows 33% over the H=768 analysis from the plan-11 review; the same trained-net argument holds (real activations/weights are far below saturation; matches bullet's own reference inference). Re-document the headroom comment at the new H.

## 3. Trainer changes (tools/trainer)

- CLI gains `--hidden N` (default 768) and `--buckets B` (default 1, meaning no bucket layer) so ONE trainer binary covers the whole ladder.
- bullet builder: hidden size from the arg; output buckets via bullet's output-bucket support (exact builder API verified against the pinned rev; if the pinned rev lacks it, bump the pin deliberately and re-validate the M8 toy-net loss as a regression check).
- **Recipe held identical across all rungs** (attribution): batch 16384, 25 superbatches = 25 epochs, `bps = round(positions/16384)` (≈9155 at 150M), StepLR start 0.001 / γ 0.3 / step 16, `ConstantWDL 0.2`, eval-scale 400. Each train ≈ 6 min on the 5080.
- Data prep: `prepare-data.sh` unchanged; invoke with `TMPDIR=tools/data` and shuffle mem ≤ 12288 (the WSL2 tmpfs lesson).

## 4. Runtime changes (src/eval/nnue/)

- `net.rs`: `HIDDEN` becomes 1024 at rung b. Rung c adds the bucketed output arrays + a `BUCKETS` const and the new compile-time/runtime size asserts (per-arch exact bytes from §2).
- `accumulator.rs`: width follows `HIDDEN` — no structural change.
- `mod.rs` `evaluate()`: rung c selects the bucket with one `popcount` of the full occupancy, then indexes that bucket's output weights/bias. Everything else (incremental updates, refresh, perspective) is untouched.
- SIMD: scalar + AVX2 forward loops iterate H/16 chunks — 1024 divides cleanly; verified, not assumed.
- **Tests carried forward per rung:** numeric parity vs a naive reference port (extended to buckets at rung c), incremental==refresh over random walks + targeted promo/ep/castle (arch-independent), scalar==AVX2, net-contract byte-size checks against the toy nets. Bench changes at every rung; every engine commit carries its `Bench:` line.

## 5. Pipeline & sequencing

1. **Now (datagen window, ~23h):** implement trainer + runtime changes, subagent-driven, combined spec+quality review per commit. Engineering is CPU-light and coexists with datagen + the live Lichess bot.
2. **Data lands:** `datagen stats` gate (~150M, 0 leaks, W/D/L + cp consistent with net2-generation expectations) → prepare → train net3a/3b/3c (~20 min total).
3. **Measurement phase (idle machine — bot paused, datagen done):** the SPRT ladder, then the winner's anchored gauntlet.
4. **Ship 0.10.0:** promote the winner (embed, Bench, `.gitignore` exception swap, version bump, strength-log/sprt-log/README), push, redeploy the bot snapshot.

## 6. Constraints (unchanged)

std-only runtime + `include_bytes!` embed; trainer offline; shipped nets trained ONLY on NebChess self-play; review-every-step (no exceptions); measurements on an idle system; commits to main with the `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>` trailer; engine-affecting commits carry `Bench:`; bounded milestone (768→1024 + buckets and NOTHING more — 1536, HalfKA, king buckets, recipe tuning are M11+ candidates informed by this ladder's attributions).

## 7. Risks

- **NPS tax at 1024** (~est. −10–15%) could eat more of the eval gain than expected — the fixed-time SPRT prices it honestly; if 3b is flat that's a *finding* (768 not saturated → M11 goes data/features, not width).
- **Bucket-convention mismatch** between trainer and runtime — mitigated by source-verifying bullet + toy-net parity before engine code.
- **i32 headroom at H=1024** — argued safe (§2); the parity tests would catch a real overflow on the toy/real nets.
- **Shared machine during engineering** — only measurements need idle; the bot gets paused for the SPRT/gauntlet phase.
