# M9 Plan A — Anchor-Ladder Extension Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the anchored-gauntlet pool upward so NebChess's absolute rating stays bracketed from *above* as it climbs past the current 2934 ceiling, then re-baseline 0.8.0 against the new pool.

**Architecture:** This is **measurement-infrastructure / ops work, not TDD code.** Each task is a concrete operation with an explicit verification gate (expected command output), not a unit test. We acquire three stronger, distinct-family open-source NNUE engines, drop the two saturated bottom rungs, re-pin every anchor to one current CCRL Blitz list date, update the two committed scripts (`get-anchors.sh` is the source of truth; `anchored-gauntlet.sh` maps names→binaries), and run a re-baseline gauntlet of 0.8.0 against the new 7-engine pool.

**Tech Stack:** bash, fastchess (`tools/bin/fastchess`), Ordo (`tools/bin/ordo` v1.2.6), the anchor binaries in `tools/bin/anchors/` (gitignored).

**Key facts (verified):**
- `tools/bin/` is **gitignored in full** (`.gitignore:2 = /tools/bin/`). So anchor binaries AND `tools/bin/anchors/ratings.txt` are **local artifacts**, not committed. The committed source of truth that *produces* them is `tools/get-anchors.sh`. The gauntlet reads `ratings.txt` (space-separated `<Name> <rating>`, one per line).
- Current pool (to be changed): Stash19 2471, Stash20 2508, Stash21 2713, Stash25 2934, Weiss10 2898, Koivisto20 2907.
- Target pool (7): **drop** Stash19 + Stash20; **keep** Stash21, Stash25, Weiss10, Koivisto20; **add** three engines spread ~3050–3300.
- `tools/anchored-gauntlet.sh` resolves names→binaries via a `case` block at **lines 62–77**; reads `ratings.txt` at **line 57**; `GAMES` default 300, `ROUNDS=GAMES/2`, `CONCURRENCY=nproc-1` (lines 16–18).
- `tools/get-anchors.sh`: `IN_POOL` assoc-array (line 145 `[Stash19]=1 [Stash20]=1 [Stash21]=1 [Stash25]=1`, plus `Weiss10`/`Koivisto20` at lines 181–182); pool guard at lines 330–336 (`if (( POOL_VERIFIED < ${#IN_POOL[@]} ))` → exit 1); writes `ratings.txt` via `echo "$ENTRY_NAME $RATING" >> "$RATINGS_FILE"`.
- 0.8.0 = commit `72c124e`, Bench 60525, anchored 2993.2 ± 16.9 (below-only-bracketed against the old pool).
- **No passwordless sudo** (user runs sudo). Anchor binaries are x86-64 Linux; engines may be **downloaded** (GitHub release assets) or **source-built** (g++/gcc/cargo), mirroring how Weiss/Koivisto are source-built in `get-anchors.sh`.

**Engine candidates (distinct families, open-source, CCRL-Blitz-rated, Linux x86-64):**
- **Viridithas** (Rust) — `https://github.com/cosmobobak/viridithas` — release binaries under `/releases`.
- **Stormphrax** (C++) — `https://github.com/Ciekce/Stormphrax` — release binaries under `/releases`.
- **Berserk** (C) — `https://github.com/jhonnold/berserk` — release binaries under `/releases`.
- Versions are chosen at execution time so their **current** CCRL Blitz ratings spread across ~3050 / ~3175 / ~3300 (bracketing net2 from both sides). If a top engine's current release is too strong/clustered, pick an older tagged release to hit the lower rung — all three have version histories spanning 2900–3400.

---

### Task 1: Acquire the three new anchor binaries

**Files:**
- Create: `tools/bin/anchors/viridithas-<ver>-linux-x86_64` (gitignored)
- Create: `tools/bin/anchors/stormphrax-<ver>-linux-x86_64` (gitignored)
- Create: `tools/bin/anchors/berserk-<ver>-linux-x86_64` (gitignored)

- [ ] **Step 1: Pick versions to span ~3050 / ~3175 / ~3300**

Open the current CCRL Blitz list (`https://computerchess.org.uk/ccrl/404/` — "Blitz" / 40/2). Note the exact version of each engine and its rating so the three land roughly at ~3050 (just above net2's likely landing), ~3175, ~3300. Record the chosen `engine version → rating` triples and the **list-access date** in a scratch note for Task 4. Requirement: **at least one rung must be rated above ~3050** (so it stays above net2).

- [ ] **Step 2: Download the Linux x86-64 release of each (preferred path)**

For each engine, fetch its chosen release asset for Linux x86-64 (commonly an `*-x86-64-v3` or `*-linux-*` artifact on the GitHub `/releases` page) into `tools/bin/anchors/`, rename to the canonical `<engine>-<ver>-linux-x86_64`, and mark executable. Example shape (substitute the real release URLs found in Step 1):

```bash
cd /home/witt/claude-workspace/NebChess/tools/bin/anchors
curl -L -o viridithas-VER-linux-x86_64   "<viridithas release asset url>"
curl -L -o stormphrax-VER-linux-x86_64    "<stormphrax release asset url>"
curl -L -o berserk-VER-linux-x86_64       "<berserk release asset url>"
chmod +x viridithas-VER-linux-x86_64 stormphrax-VER-linux-x86_64 berserk-VER-linux-x86_64
```

- [ ] **Step 3: Fallback — source-build any engine without a usable release binary**

If a release binary is missing or fails to run on this glibc (Step 4), build from source (mirrors how `get-anchors.sh` builds Weiss/Koivisto). Viridithas/Stormphrax are Rust/C++ with `make`/`cargo build --release` targets; Berserk has a `make` target. Build, then copy the produced binary to `tools/bin/anchors/<engine>-<ver>-linux-x86_64` and `chmod +x`. (Source-build of an external repo requires the user's authorization, like the Ordo build — surface it if blocked.)

- [ ] **Step 4: Verify each binary speaks UCI (the acquisition gate)**

Run, for each new binary:

```bash
printf 'uci\nisready\nquit\n' | tools/bin/anchors/<engine>-<ver>-linux-x86_64
```

Expected: each prints an `id name ...` line, `option` lines, `uciok`, then `readyok`. If any binary fails to launch (missing shared lib, illegal instruction), use the Step 3 source-build fallback or pick a different tagged version. **Gate: all three respond with `uciok` + `readyok`.**

- [ ] **Step 5: No commit** (binaries are gitignored under `/tools/bin/`). Proceed to Task 2.

---

### Task 2: Update `tools/anchored-gauntlet.sh` name→binary map

**Files:**
- Modify: `tools/anchored-gauntlet.sh:62-77` (the `case "$anchor_name"` block)

- [ ] **Step 1: Add three `case` arms before the `*)` fallback**

Insert (using the real version strings from Task 1), immediately after the `Koivisto20)` line (line 72) and before `*)`:

```bash
        Viridithas)   anchor_bin="$ANCHOR_DIR/viridithas-VER-linux-x86_64" ;;
        Stormphrax)   anchor_bin="$ANCHOR_DIR/stormphrax-VER-linux-x86_64" ;;
        Berserk)      anchor_bin="$ANCHOR_DIR/berserk-VER-linux-x86_64" ;;
```

The `Stash19`/`Stash20` arms can remain (harmless dead arms — they are simply absent from `ratings.txt`, so the gauntlet never requests them).

- [ ] **Step 2: Verify the script still parses**

Run: `bash -n tools/anchored-gauntlet.sh`
Expected: no output, exit 0 (syntax OK).

- [ ] **Step 3: Commit** (this file is tracked — it lives in `tools/`, not `tools/bin/`)

```bash
git add tools/anchored-gauntlet.sh
git commit -m "feat(gauntlet): map Viridithas/Stormphrax/Berserk anchors (M9 ladder)

Adds three distinct-family ~3050-3300 NNUE engines to the
anchored-gauntlet name->binary case map for the M9 extended pool.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Re-pin the pool in `tools/get-anchors.sh` (drop 2, add 3, current CCRL)

**Files:**
- Modify: `tools/get-anchors.sh` (the `IN_POOL` array + the three new acquisitions + the ratings each anchor is written with)

- [ ] **Step 1: Update `IN_POOL` to the new 7-engine set**

At line 145, change the initial array so Stash19/Stash20 are NOT pool members, and the four kept Stash/family engines + three new ones are. Concretely, set the kept Stash rungs at line 145 and append the rest near lines 181–182:

```bash
# line 145 — kept Stash rungs only (Stash19/Stash20 removed from the pool)
declare -A IN_POOL=( [Stash21]=1 [Stash25]=1 )
# ... near 181-182, after Weiss/Koivisto acquisition, keep:
IN_POOL["Weiss10"]=1
IN_POOL["Koivisto20"]=1
# ... and add after the new-engine acquisitions (Step 2):
IN_POOL["Viridithas"]=1
IN_POOL["Stormphrax"]=1
IN_POOL["Berserk"]=1
```

The guard at lines 330–336 (`POOL_VERIFIED < ${#IN_POOL[@]}`) then requires all **7** to verify.

- [ ] **Step 2: Add acquisition + `ratings.txt` writes for the three new engines**

After the Koivisto block (~line 310), add an acquisition section for each new engine mirroring the existing pattern: obtain the binary (download release asset, or source-build like Weiss/Koivisto), verify UCI, then write its rating line and bump `POOL_VERIFIED`:

```bash
# --- Viridithas (M9) ---
ENTRY_NAME="Viridithas"; RATING=<current CCRL Blitz>
# (download or build into "$ANCHOR_DIR/viridithas-VER-linux-x86_64"; chmod +x)
if printf 'uci\nquit\n' | "$ANCHOR_DIR/viridithas-VER-linux-x86_64" | grep -q uciok; then
    echo "$ENTRY_NAME $RATING" >> "$RATINGS_FILE"
    [[ -n "${IN_POOL[$ENTRY_NAME]:-}" ]] && POOL_VERIFIED=$((POOL_VERIFIED+1))
else
    ERRORS+="\n  $ENTRY_NAME: UCI check failed"
fi
# --- repeat for Stormphrax and Berserk with their ratings/binaries ---
```

Also stop writing the Stash19/Stash20 rating lines (remove or guard their `echo ... >> ratings.txt` so they no longer enter the pool file).

- [ ] **Step 3: Re-pin the kept anchors to the SAME current CCRL list date**

Update the `RATING` values written for Stash21, Stash25, Weiss10, Koivisto20 to their **current** CCRL Blitz ratings from the same list date used in Task 1 Step 1 (ratings drift; the whole pool must share one list date). Add a comment line at the top of the ratings-writing section recording the list date, e.g. `# CCRL Blitz list YYYY-MM-DD`.

- [ ] **Step 4: Regenerate the pool and verify the guard passes**

Run: `tools/get-anchors.sh`
Expected: it acquires/verifies all 7, prints the final ratings table, and does **not** hit the `BLOCKED: only N of 7 pool anchors verified` guard. Confirm:

```bash
cat tools/bin/anchors/ratings.txt
```
Expected: exactly 7 lines — Stash21, Stash25, Weiss10, Koivisto20, Viridithas, Stormphrax, Berserk — each with its current rating; no Stash19/Stash20.

- [ ] **Step 5: Commit** (`get-anchors.sh` is tracked; `ratings.txt`/binaries are gitignored and not staged)

```bash
git add tools/get-anchors.sh
git commit -m "feat(anchors): M9 extended pool — drop Stash19/20, add Viridithas/Stormphrax/Berserk

Re-pins the anchored-gauntlet pool to 7 engines (~2713-3300) on one
current CCRL Blitz list date. Drops the two saturated bottom rungs
(Stash19/20, ~94% score = near-zero info) and adds three distinct-family
NNUE engines above our 2993 estimate so the rating stays bracketed from
above. Pool guard now requires 7.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Re-baseline 0.8.0 against the extended pool (the measurement gate)

**Files:**
- Read: `tools/anchored-gauntlet.sh` (run it)
- Modify: `docs/strength-log.md` (add the re-baseline row)

- [ ] **Step 1: Confirm the 0.8.0 build is current**

Run: `cargo build --release && ./target/release/nebchess bench | tail -1`
Expected: `Bench: 60525` (the shipped 0.8.0/net1 build).

- [ ] **Step 2: Run the re-baseline gauntlet on an idle system**

This is a measurement — **no other agents/builds running.** ~2–3h.

```bash
tools/anchored-gauntlet.sh 300 > tools/gauntlet-08x-rebaseline.log 2>&1
```
(Run via the harness background mechanism so completion auto-notifies; do NOT double-detach with `nohup &`.)

- [ ] **Step 3: Verify the re-baseline gate**

When complete, read the Ordo table and forfeit scan from the log. **Gate (all must hold):**
- The Ordo table lists `nebchess` and all 7 anchors; nebchess rating ≈ 2993 ± ~17 (re-bracketed; the absolute value may shift a little now that it's bracketed from above — that is expected and *more* trustworthy).
- **At least one anchor is rated ABOVE the nebchess estimate** (bracketed from above — the whole point).
- Forfeit scan: **NebChess 0 forfeits / 1800**; any anomalies are anchor-side (note them).

If NebChess shows time-forfeits, halt and reduce gauntlet concurrency (the WSL2 timing check), then re-run before trusting the number.

- [ ] **Step 4: Record the re-baseline in `docs/strength-log.md`**

Append one row after the most recent entry, matching the existing column format `| date | nebchess | pool (games/pairing) | ordo estimate | notes |`. Fill the real numbers from Step 3; example shape:

```markdown
| 2026-06-09 | 0.8.0 @ 72c124e (M9 re-baseline vs extended pool) | **extended 7-engine pool**: Stash21/25 + Weiss10 + Koivisto20 + Viridithas/Stormphrax/Berserk (~2713–3300 CCRL Blitz, list <DATE>; 300 ea, 10+0.1; engine-DEFAULT book/TB OFF) | **<RATING> ± <ERR>** | <N> games; the M9 ladder extension — now bracketed from ABOVE (top rung <NAME> <rating> > our estimate), replacing the below-only-bracketed 2993.2 vs the old pool. Per-rung nebchess score: <fill>. NebChess 0/<N> forfeits (<anchor-side anomalies>). This is the clean reference net2 (Plan B) is measured against |
```

- [ ] **Step 5: Commit + push**

```bash
git add docs/strength-log.md
git commit -m "docs(strength): M9 re-baseline 0.8.0 vs extended 7-engine pool

0.8.0 re-measured against the M9 ladder (Stash21/25 + Weiss + Koivisto +
Viridithas/Stormphrax/Berserk, ~2713-3300). Now bracketed from above;
<RATING> +/- <ERR>. Clean reference for the net2 gauntlet. 0 NebChess
forfeits.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
git push origin main
```

---

## Self-review (controller)

- **Spec coverage (§3):** acquire 3 engines (T1) ✓; gauntlet case map (T2) ✓; re-pin all anchors to current CCRL, drop 2 / add 3, pool guard → 7 (T3) ✓; re-baseline 0.8.0 from above + strength-log (T4) ✓.
- **Gitignore reality:** binaries + `ratings.txt` are gitignored artifacts; only `anchored-gauntlet.sh`, `get-anchors.sh`, `strength-log.md` are committed. Reflected in every commit step.
- **Gate is explicit:** T4 Step 3 — a rung above our estimate + 0 NebChess forfeits.
- **No placeholders:** the only `<...>` are execution-time values (release URLs, current CCRL ratings, the measured rating) — deliberately pinned during execution, with the *method* fully specified. Engine version strings (`VER`) are filled in Task 1 and reused consistently in Tasks 2–3.
