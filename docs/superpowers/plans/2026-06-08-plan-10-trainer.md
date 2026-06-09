# Plan-10: NNUE Trainer Crate + First Net — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. NOTE: this plan is integration/operational glue around the `bullet` trainer, not unit-testable logic — "tests" are operational validations (does it build? does conversion produce the right byte count? does training export a contract-correct net?).

**Goal:** Stand up an offline `bullet`-based trainer (isolated from the engine build) that converts our self-play shards to bullet's format, trains the v1 NNUE `(768→H)×2→1` SCReLU net on the GPU, and exports a `quantised.bin` that honors the net contract.

**Architecture:** A standalone cargo project at `tools/trainer/` (its own `[workspace]` so the engine build never pulls `bullet`/CUDA) depends on `bullet_lib` (git) with the `cuda` feature. A small shell script drives `bullet-utils` (built from the cloned bullet repo, no CUDA) to convert→shuffle our `FEN | cp_white | wdl_white` shards into a bulletformat `.bin`. `tools/trainer/src/main.rs` mirrors bullet's `examples/simple.rs` with our constants (QA=255, QB=64, SCALE=400, H=768) and takes the data path / net-id / superbatch count as args, so the same binary trains both a toy validation net (on the gate sample) and the real first net (on the full ~100M run).

**Tech Stack:** Rust (edition 2024, the trainer crate only), `bullet_lib` (CUDA), `bullet-utils`, the RTX 5080 (`sm_120`, `CUDA_PATH=/usr/local/cuda`). The engine itself is untouched (stays std-only + pyrrhic-rs).

**Spec:** `docs/superpowers/specs/2026-06-08-nnue-design.md` — "plan-10 (B)", "Training recipe", "The net contract".

**Inputs from plan-9:** `FEN | cp_white | wdl_white` text shards. Gate sample (pipeline validation): `tools/data/selfplay-gate/` (~169k positions). Full first-net set (generating in the background now): `tools/data/selfplay-net1/` (~100M positions).

**bullet reference:** cloned at `/home/witt/bullet`; CUDA confirmed working via `cargo run -r -p bullet_lib --features cuda --example test1` (from `/home/witt/bullet`). `examples/simple.rs` IS the v1 architecture + the runtime-inference reference (plan-11 will adapt the latter). Run all bullet commands from `/home/witt/bullet` with `CUDA_PATH=/usr/local/cuda`.

---

## A nuance to carry (spec reconciliation)

The spec says "single epoch + weight decay." That guidance is for *billion-scale* data. Our first-net set is ~100M, so we train **multiple epochs** (the bullet `DirectSequentialDataLoader` loops the file): a "superbatch" of `batches_per_superbatch × batch_size` ≈ one pass over ~100M, and we run ~20–25 superbatches. Weight decay (AdamW default clipping) still regularizes. The exact superbatch count / LR / WDL-lambda are **tunables** to iterate; the values below are sane starting points, not sacred.

WDL convention (important, easy to get backwards): bullet's `wdl_scheduler` value is the weight on the **game result**: `target = wdl·result + (1−wdl)·sigmoid(cp/SCALE)`. So **eval-dominant** (the spec's "lambda≈0.8 on eval") = `wdl ≈ 0.2`.

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `tools/trainer/Cargo.toml` | Standalone crate manifest: empty `[workspace]` (decouple from engine) + `bullet_lib` git dep (cuda) | **Create** |
| `tools/trainer/.gitignore` | Ignore `target/`, `checkpoints/` | **Create** |
| `tools/trainer/src/main.rs` | The training program (v1 arch + recipe + arg parsing) | **Create** |
| `tools/trainer/prepare-data.sh` | Drive `bullet-utils`: cat shards → `convert --from text` → `shuffle` → one `.bin` | **Create** |
| `tools/trainer/README.md` | How to build/run the trainer + the data pipeline (brief) | **Create** |

Converted `.bin` data lands in `tools/data/` (already gitignored). Net checkpoints land in `tools/trainer/checkpoints/` (gitignored). The engine workspace (`Cargo.toml` at repo root) is **not modified**.

---

## Task 1: Trainer crate scaffold (CPU)

**Files:** Create `tools/trainer/Cargo.toml`, `tools/trainer/.gitignore`, `tools/trainer/src/main.rs` (stub).

- [ ] **Step 1: Pin the validated bullet revision**

Run: `git -C /home/witt/bullet rev-parse HEAD`
Record the SHA (call it `<BULLET_REV>`); use it in the Cargo.toml below so the trainer build is reproducible against the bullet version we validated in Step 0.

- [ ] **Step 2: Create `tools/trainer/Cargo.toml`**

```toml
# Standalone offline trainer — EXCLUDED from the engine build.
# The empty [workspace] makes this its own workspace root, so `cargo build`
# in the repo root (the nebchess engine) never descends here or pulls bullet/CUDA.
[workspace]

[package]
name = "nebchess-trainer"
version = "0.1.0"
edition = "2024"

[dependencies]
# Pinned to the rev validated in Step 0. cuda feature => GPU training (needs CUDA_PATH).
bullet = { git = "https://github.com/jw1912/bullet", package = "bullet_lib", rev = "<BULLET_REV>", features = ["cuda"] }
```

- [ ] **Step 3: Create `tools/trainer/.gitignore`**

```gitignore
/target
/checkpoints
```

- [ ] **Step 4: Create `tools/trainer/src/main.rs` (stub)**

```rust
//! plan-10: offline NNUE trainer for NebChess (bullet, CUDA). Not part of the engine build.
fn main() {
    eprintln!("nebchess-trainer: see plan-10; real config lands in Task 3");
}
```

- [ ] **Step 5: Verify the ENGINE build is untouched**

Run (from the repo root):
```bash
cd /home/witt/claude-workspace/NebChess
cargo build --release 2>&1 | tail -3
./target/release/nebchess bench | tail -1
```
Expected: the engine builds as before, prints `Bench: 54508`, and cargo does NOT mention `bullet`/`nebchess-trainer` (the trainer crate is invisible to the engine build).

- [ ] **Step 6: Build the trainer crate with CUDA (confirms bullet resolves + compiles on this box)**

Run:
```bash
cd /home/witt/claude-workspace/NebChess/tools/trainer
CUDA_PATH=/usr/local/cuda cargo build --release 2>&1 | tail -15
```
Expected: it fetches the pinned bullet rev, compiles (this is slow the first time — bullet + CUDA bindings), and finishes. `./target/release/nebchess-trainer` runs and prints the stub line. If the CUDA link fails, confirm `CUDA_PATH` and that `ls /usr/local/cuda/lib64/libnvrtc.so` exists (Step 0 toolchain).

- [ ] **Step 7: Commit** (trainer crate; engine unaffected, so NO `Bench:` line)

```bash
cd /home/witt/claude-workspace/NebChess
git add tools/trainer/Cargo.toml tools/trainer/.gitignore tools/trainer/src/main.rs
git commit -m "feat(trainer): standalone bullet trainer crate scaffold (offline, CUDA)" \
  -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```
(Note: `tools/trainer/Cargo.lock` will be generated — add it too if you want a reproducible trainer build; it does not affect the engine. Do NOT commit `tools/trainer/target/`.)

---

## Task 2: Data conversion pipeline (CPU)

**Files:** Create `tools/trainer/prepare-data.sh`.

- [ ] **Step 1: Build `bullet-utils` (no CUDA needed)**

Run:
```bash
cd /home/witt/bullet
cargo build --release --package bullet-utils 2>&1 | tail -5
./target/release/bullet-utils help 2>&1 | head -20   # confirm: convert / shuffle / interleave subcommands
```
Expected: builds; `help` lists `convert`, `interleave`, `shuffle`, `validate`, etc.

- [ ] **Step 2: Create `tools/trainer/prepare-data.sh`**

```bash
#!/usr/bin/env bash
# Convert + shuffle plan-9 self-play shards into one bulletformat .bin for training.
# Usage: prepare-data.sh <shard-dir> <out.bin> [mem_mb=16384]
# Our shards are already `FEN | cp_white | wdl_white` — bullet's `convert --from text`
# ingests this directly (white-relative cp + 1.0/0.5/0.0 result).
set -euo pipefail

SHARD_DIR="${1:?usage: prepare-data.sh <shard-dir> <out.bin> [mem_mb]}"
OUT="${2:?usage: prepare-data.sh <shard-dir> <out.bin> [mem_mb]}"
MEM_MB="${3:-16384}"
UTILS="${BULLET_UTILS:-/home/witt/bullet/target/release/bullet-utils}"

tmp_txt="$(mktemp --suffix=.txt)"
tmp_bin="$(mktemp --suffix=.bin)"
trap 'rm -f "$tmp_txt" "$tmp_bin"' EXIT

echo "[prepare-data] concatenating shards from $SHARD_DIR"
cat "$SHARD_DIR"/shard_*.txt > "$tmp_txt"
echo "[prepare-data] lines: $(wc -l < "$tmp_txt")"

echo "[prepare-data] convert text -> bulletformat"
"$UTILS" convert --from text --input "$tmp_txt" --output "$tmp_bin" --threads 8

echo "[prepare-data] shuffle -> $OUT (mem ${MEM_MB} MB)"
"$UTILS" shuffle --input "$tmp_bin" --output "$OUT" --mem-used-mb "$MEM_MB"

bytes=$(stat -c%s "$OUT")
echo "[prepare-data] done: $OUT  ($bytes bytes = $((bytes / 32)) positions @ 32 B/record)"
```
Make it executable: `chmod +x tools/trainer/prepare-data.sh`.

- [ ] **Step 3: Run it on the gate sample (validates the whole conversion path on real data)**

Run:
```bash
cd /home/witt/claude-workspace/NebChess
tools/trainer/prepare-data.sh tools/data/selfplay-gate tools/data/gate.shuf.bin 4096
```
Expected: prints the line count (~169k), the convert summary (`Wins/Draws/Losses`, and crucially **no `error parsing:` lines** — that would mean our text format doesn't match bullet's parser), and a final positions count. Verify: the reported positions ≈ the `datagen stats tools/data/selfplay-gate` count, and `bytes % 32 == 0`.

- [ ] **Step 4: Commit** (the script; the `.bin` data is gitignored)

```bash
git add tools/trainer/prepare-data.sh
git commit -m "feat(trainer): prepare-data.sh (shards -> bulletformat via bullet-utils)" \
  -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: Trainer program + toy net (GPU)

**Files:** Modify `tools/trainer/src/main.rs`; Create `tools/trainer/README.md`.

- [ ] **Step 1: Write `tools/trainer/src/main.rs`** (adapted from bullet's `examples/simple.rs`)

```rust
//! plan-10: offline NNUE trainer for NebChess (bullet, CUDA). Not part of the engine build.
//! Arch: (768 -> HIDDEN)x2 -> 1, SCReLU, perspective. Quantization QA/QB/SCALE per the net contract.
//! Usage: nebchess-trainer --data <shuffled.bin> --id <net-id> --superbatches <N> [--bps <batches_per_superbatch>]
use std::env;

use bullet::{
    game::inputs::Chess768,
    nn::optimiser::AdamW,
    trainer::{
        save::SavedFormat,
        schedule::{TrainingSchedule, TrainingSteps, lr, wdl},
        settings::LocalSettings,
    },
    value::{ValueTrainerBuilder, loader::DirectSequentialDataLoader},
};

const HIDDEN: usize = 768; // v1 hidden width (sweepable: 512/768/1024)
const QA: i16 = 255;       // input/accumulator weight quantization
const QB: i16 = 64;        // output weight quantization
const SCALE: i32 = 400;    // eval scale (cp <-> win-prob sigmoid)

fn main() {
    let mut data = String::new();
    let mut id = "net".to_string();
    let mut superbatches = 25usize;
    let mut bps = 6104usize; // ~one pass over ~100M positions at batch 16384
    let argv: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let flag = argv[i].clone();
        match flag.as_str() {
            "--data" => { i += 1; data = argv.get(i).cloned().unwrap_or_default(); }
            "--id" => { i += 1; if let Some(v) = argv.get(i) { id = v.clone(); } }
            "--superbatches" => { i += 1; superbatches = argv.get(i).and_then(|s| s.parse().ok()).unwrap_or(superbatches); }
            "--bps" => { i += 1; bps = argv.get(i).and_then(|s| s.parse().ok()).unwrap_or(bps); }
            other => eprintln!("trainer: ignoring {other}"),
        }
        i += 1;
    }
    assert!(!data.is_empty(), "pass --data <path-to-shuffled.bin>");

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
            let l0 = builder.new_affine("l0", 768, HIDDEN);
            let l1 = builder.new_affine("l1", 2 * HIDDEN, 1);
            let stm_h = l0.forward(stm).screlu();
            let ntm_h = l0.forward(ntm).screlu();
            l1.forward(stm_h.concat(ntm_h))
        });

    let schedule = TrainingSchedule {
        net_id: id,
        eval_scale: SCALE as f32,
        steps: TrainingSteps {
            batch_size: 16_384,
            batches_per_superbatch: bps,
            start_superbatch: 1,
            end_superbatch: superbatches,
        },
        // 0.2 weight on game-result => eval-dominant (~lambda 0.8 on eval). Tunable.
        wdl_scheduler: wdl::ConstantWDL { value: 0.2 },
        lr_scheduler: lr::StepLR { start: 0.001, gamma: 0.3, step: (superbatches * 2 / 3).max(1) },
        save_rate: superbatches.max(1),
    };

    let settings = LocalSettings { threads: 4, test_set: None, output_directory: "checkpoints", batch_queue_size: 64 };
    let data_loader = DirectSequentialDataLoader::new(&[data.as_str()]);

    trainer.run(&schedule, &settings, &data_loader);
    println!("done -> checkpoints/{}-{}/quantised.bin", schedule.net_id, superbatches);
}
```
**If a `bullet::` import path or builder method differs** from this (it's adapted from `examples/simple.rs` — read that file to confirm `ValueTrainerBuilder`, `.dual_perspective()`, `.build(|builder, stm, ntm| ...)`, `DirectSequentialDataLoader::new`, the `wdl`/`lr` scheduler types, and `TrainingSteps` field names), adapt to the real API. The example is the source of truth.

- [ ] **Step 2: Build the trainer**

Run:
```bash
cd /home/witt/claude-workspace/NebChess/tools/trainer
CUDA_PATH=/usr/local/cuda cargo build --release 2>&1 | tail -15
```
Expected: compiles cleanly.

- [ ] **Step 3: Toy-train on the gate sample (proves convert→train→export end-to-end on the GPU)**

Run (small schedule — this is pipeline validation, NOT a strong net; ~169k positions):
```bash
cd /home/witt/claude-workspace/NebChess/tools/trainer
CUDA_PATH=/usr/local/cuda ./target/release/nebchess-trainer \
  --data ../data/gate.shuf.bin --id toy --superbatches 5 --bps 10
```
Expected: prints `Training on NVIDIA GeForce RTX 5080 (sm_120)`, the per-superbatch **loss decreases** across the 5 superbatches, and it saves checkpoints. Confirm `checkpoints/toy-5/quantised.bin` exists.

- [ ] **Step 4: Validate the exported net honors the contract (byte size)**

For `HIDDEN=768`, `quantised.bin` (i16 = 2 bytes each), padded up to a multiple of 64 bytes, must hold:
`feature_weights 768×768` + `feature_bias 768` + `output_weights 2×768` + `output_bias 1` = `589824 + 768 + 1536 + 1 = 592129` i16 = `1184258` bytes, padded up to the next multiple of 64 = **1184320 bytes**.
Run:
```bash
stat -c%s tools/trainer/checkpoints/toy-5/quantised.bin
```
Expected: **1184320** (or, if bullet pads differently, `ceil(1184258/64)*64`). If the raw element count doesn't match `592129`, the architecture/quantisation config is wrong — STOP and reconcile against `examples/simple.rs`.

- [ ] **Step 5: Write `tools/trainer/README.md`** (brief: build, prepare-data, train, where the net lands; the net contract numbers above)

```markdown
# nebchess-trainer (offline, plan-10)

Standalone bullet-based NNUE trainer. NOT part of the engine build (own `[workspace]`).
Requires `CUDA_PATH=/usr/local/cuda` and the RTX 5080 (sm_120).

## Pipeline
1. `bullet-utils`: `cd /home/witt/bullet && cargo build --release --package bullet-utils`
2. Convert+shuffle shards:  `tools/trainer/prepare-data.sh <shard-dir> <out.bin> [mem_mb]`
3. Train:  `cd tools/trainer && CUDA_PATH=/usr/local/cuda ./target/release/nebchess-trainer --data <out.bin> --id <name> --superbatches <N> [--bps <B>]`
4. Net: `tools/trainer/checkpoints/<name>-<N>/quantised.bin` (raw i16, column-major, LE, padded /64; the net contract for plan-11).

Arch: `(768 -> 768)x2 -> 1` SCReLU, QA=255, QB=64, SCALE=400, WDL=0.2 (eval-dominant). Tunables: HIDDEN, superbatches, --bps, wdl, lr.
```

- [ ] **Step 6: Commit**

```bash
cd /home/witt/claude-workspace/NebChess
git add tools/trainer/src/main.rs tools/trainer/README.md
git commit -m "feat(trainer): v1 (768->768)x2->1 SCReLU trainer + toy-net pipeline validation" \
  -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: Train the first net on the full ~100M (GPU, gated on datagen)

**Gated:** only start when the background datagen run (`tools/data/selfplay-net1/`) has finished. Confirm: the run's task reported done, and `datagen stats tools/data/selfplay-net1` shows ~100M positions with `in-check: 0  bad-fen: 0`.

- [ ] **Step 1: Prepare the full dataset**

Run (the full set is ~3 GB binary; give shuffle plenty of RAM):
```bash
cd /home/witt/claude-workspace/NebChess
tools/trainer/prepare-data.sh tools/data/selfplay-net1 tools/data/net1.shuf.bin 24576
```
Expected: ~100M positions, no parse errors, `bytes % 32 == 0`.

- [ ] **Step 2: Train the first net**

Run (full schedule; multiple epochs over ~100M):
```bash
cd /home/witt/claude-workspace/NebChess/tools/trainer
CUDA_PATH=/usr/local/cuda ./target/release/nebchess-trainer \
  --data ../data/net1.shuf.bin --id net1 --superbatches 25 --bps 6104 2>&1 | tee net1-train.log
```
Expected: trains on `sm_120`; loss decreases and plateaus; saves `checkpoints/net1-25/quantised.bin`. (Watch for a sane final loss in the ~0.05–0.08 range, comparable to the bullet test refs; the absolute value depends on data/WDL.) This run takes a while — it's the real net.

- [ ] **Step 3: Validate the net file**

Run: `stat -c%s tools/trainer/checkpoints/net1-25/quantised.bin`
Expected: **1184320** bytes (the contract size for H=768). Stage the net at a stable path for plan-11:
```bash
mkdir -p tools/trainer/nets
cp tools/trainer/checkpoints/net1-25/quantised.bin tools/trainer/nets/net1.bin
```
(`tools/trainer/nets/net1.bin` is gitignored via the global `*.bin` rule. How the net is embedded/distributed for the shipped engine — `include_bytes!` from a committed/force-added file vs. a release-asset download like `nebbook.bin` — is decided in **plan-11/12**, not here.)

- [ ] **Step 4: Record the training result** (the net is binary/gitignored; commit the log + a strength-log-style note)

Append a short entry to `tools/trainer/net1-train.log` summary or a `docs/` note: net id, dataset size, schedule, final loss, net path + byte size. Commit the log/doc:
```bash
cd /home/witt/claude-workspace/NebChess
git add tools/trainer/net1-train.log   # or the docs note
git commit -m "chore(trainer): first net (net1) trained on ~100M self-play; final loss <X>" \
  -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

**Plan-10 ends here:** a converged, contract-correct `quantised.bin` exists at `tools/trainer/nets/net1.bin`. Strength (SPRT vs HCE) is plan-12, after plan-11 builds the Rust inference.

---

## Self-Review

**Spec coverage** (against the spec's "plan-10 (B)", "Training recipe", "The net contract"):
- standalone trainer crate excluded from the engine build → Task 1 (empty `[workspace]`, verify engine bench 54508 untouched) ✓
- `bullet` git dep + CUDA → Task 1 (pinned rev, cuda feature) ✓
- convert/shuffle our shards via `bullet-utils` → Task 2 (`prepare-data.sh`, our format matches `--from text`) ✓
- v1 arch `(768→H)×2→1` SCReLU, QA255/QB64/SCALE400, dual_perspective, AdamW, sigmoid+squared-error → Task 3 (`main.rs`) ✓
- WDL eval-dominant + eval_scale 400 → Task 3 (`wdl 0.2`, `eval_scale 400`; convention reconciled above) ✓
- export `quantised.bin` honoring the contract → Task 3 Step 4 + Task 4 Step 3 (byte-size check = 1184320 for H=768) ✓
- toy-validate then real net → Task 3 (gate) + Task 4 (full ~100M) ✓
- engine stays std-only → Task 1 Step 5 (engine build + bench unchanged) ✓

**Placeholder scan:** the `<BULLET_REV>` and `<X>` (final loss) are runtime-discovered values the implementer fills from a command/observation, not vague TODOs; every command and the full `main.rs`/scripts are concrete.

**Consistency:** `HIDDEN=768`, `QA=255`, `QB=64`, `SCALE=400` consistent across `main.rs`, the README, and the byte-size check; `prepare-data.sh` output path (`*.shuf.bin`) feeds `--data` in Tasks 3/4; the 32-byte/record (bulletformat `ChessBoard`) assumption is used consistently in `prepare-data.sh` and matches `bullet-utils` shuffle/interleave (`SIZE=32`).

**Known risks flagged inline:** bullet API drift (Task 3 Step 1 says the example is source-of-truth); the net distribution decision deferred to plan-11/12; Task 4 gated on the background datagen.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-08-plan-10-trainer.md`. Two execution options:

**1. Subagent-Driven (recommended)** — fresh subagent per task + combined spec+quality review between tasks. Note: Tasks 1–3 run now (gate sample); Task 4 is gated on the background ~100M datagen finishing.

**2. Inline Execution** — execute here with checkpoints.

Which approach?
