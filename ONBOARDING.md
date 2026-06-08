# NebChess — Onboarding & Desktop-Migration Handoff

**Purpose of this file:** everything a fresh Claude Code session (on the new
desktop) needs to (1) understand the project, (2) reconstruct the full dev
environment, and (3) pick up the **NNUE milestone (M8)** cleanly. Read this
top-to-bottom first. Written 2026-06-08 at repo HEAD `57ec2a4`.

---

## 0. Mission (why we're migrating)

NebChess is a from-scratch Rust UCI chess engine (`github.com/N3bDev/nebchess`).
It plays at **~2827 anchored** (10+0.1 vs a CCRL-pinned pool) and **~2400 live
Lichess blitz**. The hand-crafted eval (HCE) is near its ceiling. **The goal is a
3000-rated bot, and the lever is NNUE** (a trained neural-net evaluation) —
both our peer cutecassia (2355, from-scratch Rust NNUE) and the ~3000 Patricia-5
are NN-eval; from-scratch HCE realistically tops out below 3000.

We are moving development to the desktop because **NNUE training wants a GPU**,
and the desktop has an **RTX 5080**. The laptop (16-core WSL2, no usable GPU)
could not train efficiently. The NNUE design + implementation happens in the new
session on the desktop, starting from §8 below.

---

## 1. Current state (as of HEAD `57ec2a4`)

- **Version:** `0.7.1` (`Cargo.toml`). Live Lichess bot is on 0.7.1.
- **Bench fingerprint invariant:** `Bench: 54508` (`./target/release/nebchess bench | tail -1`). Engine-affecting commits carry a `Bench:` line; CI re-runs it and fails on mismatch. **Memorize 54508** — it's the current baseline.
- **Milestones:** M0–M6b complete (board/movegen, search+TT+ordering+PVS, pruning, full HCE + Texel tuning, SEE/conthist, TimeBrain v1, opening book, Syzygy, pondering, Lichess hardening). See `README.md` status list.
- **Just happened (this session):** **TimeBrain-V2 was attempted and DE-SCOPED.** The first gate (tighten the per-move hard cap 5×→2×) came back **H0: −55.6 ±23.3 at 8+0.08** and was reverted (`6f02bea`). It revealed two things: (a) the hard cap is the iteration-completion allowance, not the clock-leak lever; (b) **NebChess is "time-elastic"** — more search time per move genuinely produces stronger moves, so "bank time by spending less" trades away real strength. The clock over-spend is partly *inherent* to a time-elastic HCE engine. Full write-up: `docs/field-analysis-071.md`, `docs/sprt-log.md` (2026-06-08 row), and the de-scope banner atop `docs/superpowers/plans/2026-06-08-plan-8-timebrain-v2.md`.
- **Why that matters for NNUE:** a strong net reaches good moves at *lower* depth, which directly reduces the time-elasticity that hurt us live. NNUE is both the ceiling-raiser AND the robustness fix.
- **Tree is clean** at `57ec2a4` (only the stray Windows `docs/*.lnk` shortcut is untracked — ignore it).

---

## 2. Target hardware (the desktop) — and what it unlocks

| Component | Spec | Relevance to NNUE |
|---|---|---|
| CPU | **Intel Ultra 9 285K** (Arrow Lake, ~24 cores: 8 P + 16 E) | Fast self-play data generation + many concurrent fastchess games for SPRT. |
| RAM | **32 GB** | Comfortable for data-gen and CPU-side training; fine for GPU training data pipelines. |
| GPU | **RTX 5080** (Blackwell, 16 GB VRAM, CUDA) | **The reason for the move.** Standard GPU NNUE training (the `bullet` trainer or PyTorch-CUDA) is fast here. Blackwell is `sm_120` — needs a **current CUDA 12.x toolkit + latest NVIDIA driver**; PyTorch needs a recent cu12x build (nightly if a stable wheel doesn't yet ship Blackwell kernels). Confirm `nvidia-smi` + a tiny CUDA tensor op early. |

**OS environment (CONFIRMED 2026-06-08): WSL2 Ubuntu** on the desktop, matching
the laptop, with **CUDA-on-WSL2** for the RTX 5080. One environment for the Rust
engine, fastchess/SPRT, and GPU training. Setup order that matters:
1. Install the latest **NVIDIA *Windows* driver** (the build with WSL2 CUDA
   support). Do **NOT** install a Linux GPU driver inside WSL — the Windows
   driver projects the GPU into WSL automatically.
2. In WSL2 Ubuntu install the **CUDA 12.x toolkit** (the `wsl-ubuntu` variant,
   *no* bundled driver). Verify: `nvidia-smi` lists the 5080, and a tiny CUDA op
   runs (e.g. `python -c "import torch; print(torch.cuda.get_device_name())"`).
3. **Blackwell caveat (`sm_120`, new):** use a current CUDA 12.x; for PyTorch use
   a recent **cu12x / nightly** wheel that ships Blackwell kernels; for the Rust
   `bullet` trainer, build against the installed CUDA toolkit. Confirm a GPU
   op works *before* investing in a long training run.

---

## 3. What travels via `git clone` vs. what does NOT

`git clone https://github.com/N3bDev/nebchess.git` brings **all source, tests,
docs, specs, plans, and `tools/*.sh|*.py` scripts** — that's the engine and the
whole workflow toolkit. It does **NOT** bring the gitignored artifacts below
(`.gitignore`: `/target`, `/tools/bin/`, `/tools/books/`, `/tools/suites/`,
`/tools/data/`, `/tools/tb/`, `*.bin`, `/db/`, `.claude/`).

**Action table — reconstruct these on the desktop:**

| Path | Size | What it is | How to get it on the desktop |
|---|---|---|---|
| `target/` | 1.7 G | Rust build output | **REBUILD** — `cargo build --release` |
| `db/*.pgn` | ~1.5 G | **Private game corpora** (`export_ELO2400.pgn` 784 MB OTB ≥2400; `LumbrasGigaBase_Online_2025.pgn` 803 MB Lichess Elite; the small `nebchessbot*.pgn` field games) | **COPY** (user-provided, not re-downloadable) — use the bundle in §4. **These are the raw material for NNUE training-data generation.** |
| `~/.claude/.../memory/` | tiny | Project memory (5 files) — see §5 | **COPY** (bundle includes it) — critical context |
| `tools/bin/stockfish` | 113 M | **Stockfish 18** (the eval *teacher* for distillation + a strong sparring/label engine) | COPY, or re-download from the Stockfish releases page |
| `tools/bin/anchors/` | 5.5 M | CCRL anchor pool (Stash 13–25, Weiss 1.0, Koivisto 2.0, Rustic-α2, `ratings.txt`) | **RE-DOWNLOAD** — `tools/get-anchors.sh` (or copy) |
| `tools/bin/fastchess` | 2.4 M | SPRT/gauntlet match runner (Fishtest's) | **REBUILD** — `tools/setup-fastchess.sh` |
| `tools/bin/ordo` | 226 K | Rating computation (Ordo) for gauntlets | Build from `github.com/michiguel/Ordo` (or copy) |
| `tools/bin/baseline-*` | ~13 M | Historical SPRT baseline snapshots | **SKIP** — not needed for NNUE; regenerate via `tools/baseline.sh <name>` from the matching commit only if re-running an old gate |
| `tools/books/8moves_v3.pgn` + `UHO_*.epd` | 183 M | SPRT/gauntlet opening books | **RE-DOWNLOAD** — `tools/download-books.sh` |
| `tools/books/nebbook.bin` | 5.2 M | Polyglot opening book (deploy artifact) | **RE-DOWNLOAD** — `tools/download-book.sh` (GitHub release asset) |
| `tools/tb/` | 939 M | Syzygy 3-4-5 tablebases (290 files) | **RE-DOWNLOAD** — `tools/download-syzygy.sh` |
| `tools/data/` | ~? | Texel corpora (`quiet-labeled.epd`, `lichess-big3-resolved.*`, `field-050`) | RE-DOWNLOAD — `tools/download-tuning-data.sh` (HCE-era; NNUE may not need it) |
| `tools/suites/` | small | Test suites (`wac.epd` canary, `sac-entrance.epd`) | `tools/download-testsuites.sh` for WAC; copy `sac-entrance.epd` (generated in Plan 7) |

**Bottom line:** only **`db/` (~1.5 G)** and the **memory** truly *must* be
copied; almost everything else re-downloads/rebuilds via the `tools/` scripts.
Copying `stockfish` and `anchors/` too just saves bandwidth.

---

## 4. Desktop setup — step by step

```bash
# 0. Prereqs (WSL2 Ubuntu assumed)
#    - Rust via rustup (the repo pins 1.96.0 via rust-toolchain.toml — rustup auto-installs it)
#    - build-essential, git, curl, p7zip-full (for tools/data), zstd
#    - For GPU training later: NVIDIA Windows driver w/ WSL CUDA + CUDA 12.x toolkit; verify `nvidia-smi`

# 1. Clone the repo
cd ~/   # or wherever; see the memory-path note in §5 if you keep ~/claude-workspace/NebChess
git clone https://github.com/N3bDev/nebchess.git
cd nebchess

# 2. Build + verify the engine BEFORE anything else
cargo build --release
./target/release/nebchess bench | tail -1     # MUST print: Bench: 54508
printf 'uci\nquit\n' | ./target/release/nebchess | grep -E 'id name|uciok'   # NebChess 0.7.1 + uciok
cargo test                                     # all green

# 3. Unpack the migration bundle (the db corpora + memory — see §4a)
#    -> places db/*.pgn and the ~/.claude memory files

# 4. Rebuild/re-download the rest as needed
tools/setup-fastchess.sh        # fastchess
tools/get-anchors.sh            # anchor engines (if not copied)
tools/download-books.sh         # 8moves_v3 + UHO
tools/download-book.sh          # nebbook.bin
tools/download-syzygy.sh        # Syzygy 3-4-5 (~1 GB; non-fatal if skipped)
# Stockfish 18 -> tools/bin/stockfish (copy, or download from stockfishchess.org)

# 5. Smoke the match harness (proves fastchess + a baseline binary work)
tools/baseline.sh head                                  # snapshots current build -> tools/bin/baseline-head
tools/sprt.sh ./target/release/nebchess tools/bin/baseline-head 5   # identical -> ~0 Elo, Ctrl-C after one report
```

### 4a. Making the migration bundle (run THIS on the laptop, before moving)

`tools/make-migration-bundle.sh` packages the must-copy, not-in-git artifacts
(the private `db/` corpora + the `~/.claude` project memory) into one compressed
tarball. Copy that tarball + clone the repo on the desktop, then unpack.

```bash
# On the laptop:
tools/make-migration-bundle.sh                 # -> nebchess-migration.tar.zst (+ prints manifest/sha256)
#   add --with-stockfish --with-anchors to fold those in too (saves re-downloading)
# Move the .tar.zst to the desktop (scp / USB / shared drive), then on the desktop in the cloned repo:
tar --zstd -xvf nebchess-migration.tar.zst     # restores db/ and a memory/ dir
# Place the memory files (see §5 for the exact path).
```

---

## 5. The project memory (context that lives OUTSIDE the repo)

Claude Code's persistent memory is at
`~/.claude/projects/<encoded-project-path>/memory/` — **not** in the repo, so it
won't clone. The encoded path is the absolute project dir with `/`→`-`, e.g.
`/home/witt/claude-workspace/NebChess` → `-home-witt-claude-workspace-NebChess`.
**If you clone to the same path under the same username, the encoded dir matches
and you can drop the memory files straight in.** Otherwise put them under the new
machine's corresponding encoded dir. The bundle (§4a) carries them under `memory/`.

The 5 memory files and what they hold:
- `MEMORY.md` — the index (loaded each session).
- `nebchess-project-state.md` — milestone state, the settled **workflow** (plan→subagent-driven→gates), the **tuning law** (frozen Texel K=1.520, safe two-step tuner invocation), and environment facts.
- `review-every-step.md` — **mandate: two-stage code review (spec + quality) on EVERY commit, no exceptions**, even provably-safe refactors.
- `canary-is-movetime-noisy.md` — the WAC canary is movetime-noisy; SPRT is the arbiter; run measurements on an idle system.
- `timebrain-v2-field-finding.md` — the de-scope + time-elasticity lesson + "NNUE is the lever."

**Even without the memory copy, the load-bearing facts are captured in this file
(§1, §7) so the new session is not blind.**

---

## 6. Repo map — "what everything is"

**Engine source (`src/`):**
- `board/` — bitboards, magics, movegen, make/unmake, Zobrist, perft, FEN. **(NNUE accumulator updates hook into make/unmake here.)**
- `eval/` — `mod.rs` (the `Evaluator` entry), `hce.rs` (hand-crafted eval), `manifest.rs` + `trace.rs` (the tapered HCE/Tracer architecture), `eval_params.rs` (tuned weights). **(NNUE slots in behind the eval entry — replace or hybrid; this is a design fork in §8.)**
- `search/` — `mod.rs` (alpha-beta/PVS/iterative deepening), `limits.rs` (TimeManager), `see.rs`, `tt.rs`, `bench.rs`.
- `book/` — Polyglot reader. `tb.rs` — Syzygy wrapper (the precedent for loading a data file + an external dep).
- `uci/mod.rs` — UCI protocol. `lib.rs`/`main.rs`. `bin/` — `tune` (Texel), `bookgen`, `perft`, `find_magics`, `solve`.

**Workflow toolkit (`tools/*.sh`, `*.py`):** `sprt.sh` (frozen SPRT arbiter, 8+0.08), `probe.sh` (cheap 400-game filter), `baseline.sh` (snapshot a baseline binary), `anchored-gauntlet.sh` (absolute rating vs anchors, Ordo), `forfeit-gauntlet.sh`, `timebrain-h2h.sh` (off-self-play A/B), `check-bench.sh` (CI bench gate), the `download-*`/`get-anchors`/`setup-fastchess` provisioners, `krk-stress.sh`/`uci-torture.sh`/`tactics.sh` (robustness/regression), `analyze-field.py` (replay live PGNs through a UCI pipe).

**Docs (`docs/`):** `strength-log.md` (anchored ratings ledger), `sprt-log.md` (every gate, chronological), `field-analysis-050.md` + `field-analysis-071.md` (live-game analyses), `lichess-deploy.md`, `tactics-log.md`. **Specs** in `docs/superpowers/specs/` (the master design + m6-design), **plans** in `docs/superpowers/plans/` (plan-1…plan-8, one per milestone).

---

## 7. Conventions you MUST follow (non-negotiable)

1. **Plan-per-milestone → subagent-driven execution → controller-owned gates.** Brainstorm a design (spec) → `writing-plans` → execute task-by-task with a fresh implementer subagent + **two-stage review (spec compliance, then code quality) on every commit**. Controller (you) owns the gates.
2. **The gate is the arbiter — "correct ≠ stronger."** The frozen **8+0.08 SPRT** (`tools/sprt.sh`) decides whether a change ships; the **WAC canary is noisy** (movetime-based) and only its −10 floor is signal. Multiple plausible, code-correct changes have been honestly H0'd and reverted (log-LMR, extensions, warm-start histories, TimeBrain-V2). Never rationalize past a gate. *(NNUE caveat: NNUE changes the bench fingerprint and need their own validation vs the HCE baseline — see §8.)*
3. **Bench discipline:** engine-affecting commits carry a `Bench: <n>` line; CI enforces it. A pure-refactor must be bench-identical.
4. **Measurements need an idle system** — don't run agents/builds during an SPRT/gauntlet (timing noise).
5. **Memory discipline:** keep `~/.claude/.../memory/` current; one fact per file + the `MEMORY.md` index pointer.
6. **Commits go to `main`** (this project's convention). End commit messages with the `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>` trailer. Commit/push only when asked.

---

## 8. NNUE mission brief (start here in the new session)

**Goal:** add a trained NNUE evaluation to push toward 3000. **Approach the user
endorsed:** supervised **eval distillation** (label positions with a teacher's
eval — Stockfish 18 is in `tools/bin/`), which IS the standard NNUE training
recipe (NOT reinforcement learning; true self-play RL is a *later* possible
evolution). The user's phrasing "compare how we evaluate vs Stockfish and align"
= exactly this distillation objective.

This is a multi-plan milestone. **Do NOT jump to code — run the design first**
(the `superpowers:brainstorming` skill was mid-flight when we migrated). The
heavy design research is best done here on the desktop. Recommended first move:
a research workflow fanning out across the forks below, then brainstorm them with
the user to a spec, then `writing-plans` per sub-project.

**Design forks to resolve (with current leanings):**
1. **Net architecture** — start simple: a perspective net `(768→H)x2 → 1` with clipped-ReLU/SCReLU, single output bucket; `H` ≈ 256–1024 (Elo vs train/infer cost). HalfKP / HalfKAv2-with-buckets is the v2. *Research what comparable from-scratch engines (Viridithas, Carp, Stormphrax, Obsidian, Akimbo) use and the Elo each hidden size buys.*
2. **Trainer + toolchain** — with the RTX 5080, **GPU training via the Rust `bullet` trainer (`github.com/jw1912/bullet`, CUDA)** is the leading candidate (used by many open engines; exports a net the engine loads). PyTorch-CUDA (`nnue-pytorch`) is the alternative. Training toolchain may be a heavy/external dependency; the **runtime must stay lean** (net = data file + hand-rolled Rust inference). Confirm Blackwell/CUDA-12.x support early.
3. **Training data** — distill SF18 evals over positions sourced from `db/` corpora + engine self-play (fixed-nodes), cp or WDL-blended targets; tens-to-hundreds of millions of positions typical. *Resolve volume, depth/nodes-per-label, target format, and the generation pipeline.* Provenance: using SF as a *labeler* is standard and fine; don't ship SF code or an SF-derived net.
4. **Inference in Rust** — incremental "efficiently-updatable" accumulator on make/unmake (`src/board/`), int16/int8 quantization, AVX2 SIMD + scalar fallback, net embedded via `include_bytes!` or loaded as a file. Measure the NPS hit vs the Elo gain.
5. **Integration** — replace HCE vs hybrid; how the net slots behind the `eval/` entry; bench-fingerprint refresh; **validation** (NNUE-vs-HCE SPRT at 8+0.08, then anchored gauntlet for absolute rating).
6. **Milestone decomposition** — likely: (a) data-generation pipeline → (b) trainer setup + first net → (c) Rust inference + accumulator → (d) search integration + SPRT vs HCE → (e) net-quality iteration (more/better data, bigger net) → (f) quantization/SIMD perf. Sequence + critical path to be set in the spec.

**Honest Elo expectation:** a first working NNUE typically adds a large jump over
a tuned HCE (often +100–300+ Elo), and iterating data/net size is where the path
to 3000 actually lives — NNUE quality, search speed, and data quality together.
Validate every step against the HCE baseline; don't assume.

---

## 9. First actions for the new session (checklist)

1. Confirm the OS/GPU environment (`nvidia-smi`, WSL2-vs-native), and that the engine builds with **`Bench: 54508`** and `cargo test` is green.
2. Restore `db/` and the memory (§4a/§5); skim the memory + `docs/field-analysis-071.md` + `docs/sprt-log.md` for context.
3. Re-provision the match harness (`setup-fastchess.sh`, `get-anchors.sh`) and smoke an SPRT (identical binaries ≈ 0 Elo).
4. Resume the **NNUE design** (§8): research the forks → brainstorm to a spec in `docs/superpowers/specs/` → `writing-plans` for the first sub-project (data generation) → subagent-driven execution with two-stage reviews.
5. Update `Cargo.toml` version + the README M7/M8 line + memory as the work lands.
