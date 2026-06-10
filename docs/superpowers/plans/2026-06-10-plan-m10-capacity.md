# M10 Capacity Ladder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Grow the NNUE 768→1024 hidden and add 8 output buckets, each lever isolated behind its own SPRT on flywheel-turn-2 data; ship only the final winner as 0.10.0.

**Architecture:** Three rungs (net3a=768/new-data, net3b=1024, net3c=1024+8-buckets) built **sequentially in the working tree** — the runtime supports one arch at a time via constants; SPRT baselines are snapshot binaries (`tools/baseline.sh`); **only the final winning configuration is committed** (the M8/M9 single-promotion pattern). The trainer gains `--hidden`/`--buckets` args (committed early — tooling, default behavior unchanged).

**Tech Stack:** bullet (pinned rev d835890, CUDA sm_120), Rust std-only runtime, fastchess SPRT, Ordo gauntlet.

**Spec:** `docs/superpowers/specs/2026-06-10-m10-capacity-design.md`

**Source-verified facts (bullet @ d835890, from `examples/progression/2_output_buckets.rs` and `crates/bullet_lib/src/game/outputs.rs:24-31`):**
- Builder: `.output_buckets(MaterialCount::<8>)` before `.build(...)`; build closure gains a 4th arg: `|builder, stm, ntm, output_buckets|`; output layer becomes `new_affine("l1", 2*H, 8)` and the forward ends `.select(output_buckets)`.
- Bucket convention: `divisor = 32usize.div_ceil(N)` (=4 for N=8); `bucket = (occ.count_ones() - 2) / 4` → 0..7.
- The bucketed example saves `l1w` with `.transpose()` appended: `SavedFormat::id("l1w").round().quantise::<i16>(64).transpose()`. Save order on disk stays l0w, l0b, l1w, l1b.
- Expected contract sizes (toy-MEASURED values are authoritative; if a toy differs, STOP and re-derive): H=1024 B=1 → **1,579,072 B**; H=1024 B=8 → **1,607,744 B** (the `.transpose()` layout question is settled empirically in Task 1 Step 4).

**Ownership:** Tasks 1–3 = implementer subagents + combined spec/quality review per commit. Tasks 4–7 = controller-owned measurement/ops (no subagents during measurements; **the Lichess bot must be paused for Task 6–7's measurements**).

---

### Task 1: Trainer `--hidden` / `--buckets` + toy nets (tooling commit)

**Files:**
- Modify: `tools/trainer/src/main.rs`
- Test: toy trains (commands below; no Rust unit tests — the toys ARE the test)

- [ ] **Step 1: Parameterize the trainer.** In `tools/trainer/src/main.rs`: parse two new args following the existing `--superbatches`/`--bps` pattern: `--hidden N` (usize, default `768`) and `--buckets B` (usize, default `1`). Because bullet's builder uses a different `.build()` closure arity with buckets, branch on `buckets == 1` and construct one of two trainers (duplicate the builder call — DRY yields to the API's arity; keep both branches adjacent):

```rust
// buckets == 1 (existing shape, hidden now a variable):
let mut trainer = ValueTrainerBuilder::default()
    .dual_perspective()
    .optimiser(AdamW)
    .inputs(Chess768)
    .save_format(&[
        SavedFormat::id("l0w").round().quantise::<i16>(QA),
        SavedFormat::id("l0b").round().quantise::<i16>(QA),
        SavedFormat::id("l1w").round().quantise::<i16>(QB),
        SavedFormat::id("l1b").round().quantise::<i16>(QA * QB),
    ])
    .loss_fn(|output, target| output.sigmoid().squared_error(target))
    .build(|builder, stm, ntm| {
        let l0 = builder.new_affine("l0", 768, hidden);
        let l1 = builder.new_affine("l1", 2 * hidden, 1);
        let stm_h = l0.forward(stm).screlu();
        let ntm_h = l0.forward(ntm).screlu();
        l1.forward(stm_h.concat(ntm_h))
    });

// buckets == 8 (the only bucketed config we ship; assert_eq!(buckets, 8) otherwise):
use bullet_lib::game::outputs::MaterialCount;
let mut trainer = ValueTrainerBuilder::default()
    .dual_perspective()
    .optimiser(AdamW)
    .inputs(Chess768)
    .output_buckets(MaterialCount::<8>)
    .save_format(&[
        SavedFormat::id("l0w").round().quantise::<i16>(QA),
        SavedFormat::id("l0b").round().quantise::<i16>(QA),
        SavedFormat::id("l1w").round().quantise::<i16>(QB).transpose(),
        SavedFormat::id("l1b").round().quantise::<i16>(QA * QB),
    ])
    .loss_fn(|output, target| output.sigmoid().squared_error(target))
    .build(|builder, stm, ntm, output_buckets| {
        let l0 = builder.new_affine("l0", 768, hidden);
        let l1 = builder.new_affine("l1", 2 * hidden, 8);
        let stm_h = l0.forward(stm).screlu();
        let ntm_h = l0.forward(ntm).screlu();
        l1.forward(stm_h.concat(ntm_h)).select(output_buckets)
    });
```

(`HIDDEN` const becomes `let hidden` from the arg; keep QA/QB/SCALE consts. The rest of main — schedule, settings, loader — is untouched and shared by both branches; structure with a small generic helper or by running the schedule inside each branch, whichever compiles cleanly against bullet's types. Type inference note: the two trainers have different generic types — running `trainer.run(...)` inside each branch avoids unification.)

- [ ] **Step 2: Build.** `cd tools/trainer && CUDA_PATH=/usr/local/cuda cargo build --release` → compiles clean.

- [ ] **Step 3: Train the two toy nets on existing data** (CPU-light, fine during datagen):

```bash
cd tools/trainer
CUDA_PATH=/usr/local/cuda ./target/release/nebchess-trainer --data ../data/net2.shuf.bin --id toy1024 --superbatches 1 --bps 10 --hidden 1024
CUDA_PATH=/usr/local/cuda ./target/release/nebchess-trainer --data ../data/net2.shuf.bin --id toy1024x8 --superbatches 1 --bps 10 --hidden 1024 --buckets 8
```

- [ ] **Step 4: Record the authoritative contract sizes.**

```bash
stat -c '%n %s' checkpoints/toy1024-1/quantised.bin checkpoints/toy1024x8-1/quantised.bin
```
Expected: `1579072` and `1607744`. **If either differs, STOP** — re-derive the layout from bullet's save path before any runtime code (the spec's bullet-is-authoritative rule). Also sanity: re-train a default toy (`--id toy768check --superbatches 1 --bps 10`) and confirm it is still 1,184,320 B (the default path didn't regress).

- [ ] **Step 5: Commit (tooling; combined review first, per review-every-step).**

```bash
git add tools/trainer/src/main.rs
git commit -m "feat(trainer): --hidden / --buckets args for the M10 capacity ladder

One binary covers 768, 1024, and 1024x8-bucket archs. Bucketed branch
follows bullet's output-bucket example (MaterialCount<8>, .select(),
l1w saved with .transpose()). Defaults unchanged (768, no buckets);
toy nets confirm contracts: 1024 -> 1,579,072 B, 1024x8 -> 1,607,744 B.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Runtime rung b — H=1024 (working tree; NOT committed)

**Files:**
- Modify: `src/eval/nnue/net.rs` (consts + asserts + toy path)
- Modify: `src/eval/nnue/mod.rs` (embed path — placeholder net until Task 5)

- [ ] **Step 1:** In `net.rs`: `pub const HIDDEN: usize = 768;` → `1024`. Update the compile-time assert to the toy-measured size: `const _: () = assert!(std::mem::size_of::<Network>() == 1_579_072);`. Update the i32-headroom comment: the worst-case bound grows 4/3× at H=1024; the trained-net argument (AdamW clipping ±1.98, sparse sign-cancelling activations) is unchanged.
- [ ] **Step 2:** Point the test toy paths at the 1024 toy: in `net.rs` tests `const TOY: &str = "tools/trainer/checkpoints/toy1024-1/quantised.bin";` (and its size assert literal → 1_579_072); same constant swap in `mod.rs` tests (`const TOY`) and the `accumulator.rs` test's inline path.
- [ ] **Step 3:** Verify, don't assume: `accumulator.rs` uses `HIDDEN` throughout (it does — loops are `0..HIDDEN`); the AVX2 loop is `while i < HIDDEN` stepping 16 (1024 % 16 == 0 ✓); `Accumulator` is `[i16; HIDDEN]` align(64) (2048 B at 1024 — still 64-aligned ✓).
- [ ] **Step 4:** Build + test: `cargo build --release && cargo test --release --lib nnue`. Expected: all NNUE tests pass against the 1024 toy (parity, scalar==avx2, incremental==refresh, promo/ep/castle, null, material-sign may be weak on a 10-batch toy — if `material_edge_has_sane_sign` fails on the toy, mark that ONE test to skip-on-toy by checking the net path, as a trained-net-only assertion; do not weaken the others).

Note: `mod.rs` still embeds `net2.bin` (768) — `embedded()` won't compile at HIDDEN=1024 until a real 1024 net exists. Temporary working-tree measure: `cp tools/trainer/checkpoints/toy1024-1/quantised.bin src/eval/nnue/net3b.bin` and point `embedded()` at it (the real net replaces it in Task 5; bench is meaningless until then). NO commit in this task.

---

### Task 3: Runtime rung c — 8 output buckets (working tree; NOT committed)

**Files:**
- Modify: `src/eval/nnue/net.rs` (struct, out(), avx2, tests)
- Modify: `src/eval/nnue/mod.rs` (bucket select in `evaluate()`)

- [ ] **Step 1: Bucketed Network struct** (field order = bullet's save order l0w, l0b, l1w, l1b):

```rust
pub const BUCKETS: usize = 8;

#[repr(C)]
pub struct Network {
    pub feature_weights: [Accumulator; 768],
    pub feature_bias: Accumulator,
    pub output_weights: [[i16; 2 * HIDDEN]; BUCKETS], // per-bucket contiguous [us|them]
    pub output_bias: [i16; BUCKETS],
}

const _: () = assert!(std::mem::size_of::<Network>() == 1_607_744);
```

- [ ] **Step 2: Bucket-aware forward.** `out()`/`sum()`/`out_scalar()`/`out_avx2()` gain a `bucket: usize` arg; they read `&self.output_weights[bucket]` and `self.output_bias[bucket]` — the inner loops are UNCHANGED (each bucket's weights are a contiguous `[i16; 2*HIDDEN]`, exactly the old shape):

```rust
pub fn out(&self, us: &Accumulator, them: &Accumulator, bucket: usize) -> i32 {
    let sum = self.sum(us, them, bucket);
    (sum / i32::from(QA) + i32::from(self.output_bias[bucket])) * SCALE
        / (i32::from(QA) * i32::from(QB))
}
```
(`sum` passes `&self.output_weights[bucket]` to the existing scalar/AVX2 bodies; their signatures already take `weights: &[i16; 2 * HIDDEN]` for AVX2 — make the scalar path take the same slice arg so both share it.)

- [ ] **Step 3: Bucket selection in `mod.rs` `evaluate()`** — bullet's exact convention:

```rust
fn evaluate(&mut self, pos: &Position) -> i32 {
    let acc = &self.stack[self.top];
    let occ_count = pos.occupancy_count(); // total pieces on board — see note
    let bucket = ((occ_count - 2) / (32usize.div_ceil(net::BUCKETS))) as usize;
    let (us, them) = match pos.stm() {
        Color::White => (&acc.white, &acc.black),
        Color::Black => (&acc.black, &acc.white),
    };
    self.net.out(us, them, bucket)
}
```
Note: use the engine's all-pieces occupancy bitboard popcount — locate the accessor with `grep -nE "fn (occupied|occ|all)" src/board/*.rs` (movegen uses it pervasively); if only per-side boards exist, OR them. The formula must be byte-identical to bullet's `(count_ones - 2) / div_ceil(32, N)`.

- [ ] **Step 4: Tests.** Update the reference port (`reference_out` in net.rs tests) to take + index the bucket the same way; toy path constants → `toy1024x8-1/quantised.bin` with the 1_607_744 size; add one new test pinning the bucket boundaries:

```rust
#[test]
fn bucket_index_matches_bullet_convention() {
    let div = 32usize.div_ceil(BUCKETS);
    assert_eq!(div, 4);
    assert_eq!((2usize - 2) / div, 0);   // bare kings
    assert_eq!((9usize - 2) / div, 1);
    assert_eq!((32usize - 2) / div, 7);  // full board
}
```
The parity test now exercises all 8 buckets (loop `for bucket in 0..BUCKETS` inside the random-accumulator loop). `incremental==refresh` and friends are bucket-transparent (they compare `evaluate()` to `evaluate()`).

- [ ] **Step 5:** Build + full NNUE suite vs the bucketed toy: `cargo test --release --lib nnue` → green. NO commit (the winning configuration commits at Task 7).

---

### Task 4: Data pipeline (controller; on the datagen completion notification)

- [ ] **Step 1: Stats gate.** `./target/release/datagen stats tools/data/selfplay-net3` → require: positions ≈ 150M (the run is games-bound; accept ±5%), `LEAKS -> in-check: 0 bad-fen: 0`, draws in a 40–55% band (net2 self-play ≥ net1-gen's 44%), cp mean a modest positive. Record exact `POS=<positions>`.
- [ ] **Step 2: Prepare.** `TMPDIR=/home/witt/claude-workspace/NebChess/tools/data tools/trainer/prepare-data.sh tools/data/selfplay-net3 tools/data/net3.shuf.bin 12288`
- [ ] **Step 3: Byte gate.** `stat -c %s tools/data/net3.shuf.bin` == `32 * POS` exactly.

### Task 5: Train the ladder (controller; ~20 min total on the 5080)

- [ ] **Step 1:** `BPS=$(( (POS + 8192) / 16384 ))` (round-to-nearest; ≈9155 at 150M).
- [ ] **Step 2:** From `tools/trainer`, `CUDA_PATH=/usr/local/cuda`:

```bash
./target/release/nebchess-trainer --data ../data/net3.shuf.bin --id net3a --superbatches 25 --bps $BPS
./target/release/nebchess-trainer --data ../data/net3.shuf.bin --id net3b --superbatches 25 --bps $BPS --hidden 1024
./target/release/nebchess-trainer --data ../data/net3.shuf.bin --id net3c --superbatches 25 --bps $BPS --hidden 1024 --buckets 8
```
- [ ] **Step 3:** Verify contracts: net3a-25 = 1,184,320 B; net3b-25 = 1,579,072 B; net3c-25 = 1,607,744 B. Record final losses (cross-arch loss is informational only).

### Task 6: The SPRT ladder (controller; IDLE machine — bot paused, datagen done)

Per rung: configure → `rm target/release/nebchess && touch src/eval/nnue/mod.rs && cargo build --release` (never `cp` onto the cargo output — the hardlink corrupts the cache) → record `bench` → SPRT with absolute paths → record in sprt-log (honest H0s included; H0-but-positive → surface to the user).

- [ ] **Rung a:** working tree at HIDDEN=768/no-buckets (i.e., rung b/c edits NOT active — execute rung a BEFORE Task 2's constants flip, or stash; sequencing note for the controller: run Task 6a as soon as net3a-25 exists, independent of Tasks 2–3). `cp tools/trainer/checkpoints/net3a-25/quantised.bin src/eval/nnue/net3a.bin`, point `embedded()` at `"net3a.bin"`, build, bench, then:
  `tools/sprt.sh "$(pwd)/target/release/nebchess" "$(pwd)/tools/bin/baseline-net2" 10`
  If H1 → `tools/baseline.sh net3a` (the new best-so-far). Best-so-far binary = `baseline-net3a`, else stays `baseline-net2`.
- [ ] **Rung b:** activate Task 2's constants (HIDDEN=1024), embed `net3b.bin` = `checkpoints/net3b-25/quantised.bin`, build, bench, SPRT vs the best-so-far baseline at elo1=10. If H1 → `tools/baseline.sh net3b`.
- [ ] **Rung c:** activate Task 3's bucketed struct, embed `net3c.bin` = `checkpoints/net3c-25/quantised.bin`, build, bench, **plus the real-net sanity check before the SPRT** (the layout end-to-end proof): `material_edge_has_sane_sign` must pass on net3c, and a 1-position eyeball (`position startpos` eval via bench output sane). SPRT vs best-so-far at **elo1=5**. If H1 → winner is net3c.

### Task 7: Promotion + gauntlet + ship 0.10.0 (controller)

- [ ] **Step 1:** Working tree = the winning configuration ONLY (losing rungs' code reverted if the winner doesn't need it — e.g., if net3b wins, the bucket struct is dropped from the tree; the unused toy/net bins removed). `.gitignore` exception swapped to the winner's bin; `git rm` net2.bin; the winner's net file `git add`-ed.
- [ ] **Step 2:** Combined spec+quality review (the M8-promotion checklist: call-site completeness, embed path/asserts, gitignore, no collateral, numeric accuracy of any doc rows). Fix findings; re-review.
- [ ] **Step 3:** Commit the promotion with `Bench: <winner's bench>`; `cargo fmt` first (CI gate).
- [ ] **Step 4:** Anchored gauntlet vs the unchanged M9 pool: `tools/anchored-gauntlet.sh 300` (bot still paused; ~2.5h). Gate: Ordo table + NebChess 0 forfeits; comparison row = 0.9.0's 3131.1.
- [ ] **Step 5:** Ship: version 0.9.0→0.10.0, strength-log row (winner vs 3131.1, per-rung scores, ladder attribution summary: a/b/c deltas), sprt-log rows (ALL three rungs, including any H0), README strength line + status, push, CI green check, `tools/lichess/deploy.sh` for the bot, memory update.

---

## Self-review (controller)

- **Spec coverage:** ladder + bounds + chaining (T6) ✓; contracts + toy verification (T1) ✓; bullet-authoritative bucket convention (facts + T3 S3/S4) ✓; trainer args + identical recipe (T1, T5) ✓; runtime rungs + tests (T2, T3) ✓; stats/prepare gates with the TMPDIR lesson (T4) ✓; single-promotion commit discipline + only-winner-ships (T3 note, T6, T7) ✓; idle-machine + bot-pause (T6 header) ✓; hardlink + fmt lessons baked into T6/T7 ✓.
- **Placeholders:** none — the one deliberately-deferred value (transpose layout) has an explicit empirical decision procedure with a STOP rule (T1 S4); the occupancy accessor has a concrete location procedure (T3 S3).
- **Type consistency:** `HIDDEN`/`BUCKETS` consts, `out(us, them, bucket)`, toy paths (`toy1024-1`, `toy1024x8-1`), net ids (`net3a/b/c`), baseline names (`baseline-net2/net3a/net3b`) used consistently across tasks.
- **Sequencing trap addressed:** rung a's SPRT needs the 768 configuration — T6 explicitly notes running it before/independent of T2–T3's constant flips.
