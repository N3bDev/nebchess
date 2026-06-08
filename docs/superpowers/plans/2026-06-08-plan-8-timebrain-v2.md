# TimeBrain-V2 Implementation Plan (Plan 8 / M7)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop NebChess from burning its live-blitz clock too fast — so it reaches the deciding phase of a game with time to think instead of on the increment.

**Architecture:** Two surgical changes to `TimeManager` (`src/search/limits.rs`), each independently gated: (1) tighten the per-move hard cap from `5×soft` to `2×soft` so one complex move can't eat ~38 s; (2) enrich the soft-deadline scale with a **score-volatility** signal (think longer when the eval is swinging, settle when it's flat) on top of the existing best-move-stability signal. The search tree is untouched — only *when we stop between iterations* and *the single-move ceiling* move. Validated **off self-play** via a new TimeBrain-v2-vs-v1 head-to-head (identical search/eval, only the TM differs), because same-TM self-play structurally cannot measure a spending-profile difference.

**Tech Stack:** Rust (engine), `fastchess 1.8.1` + `bin/fastchess` (matches), bash harnesses in `tools/`. Field evidence: `docs/field-analysis-071.md` (8 games). Peer reference: cutecassia (`github.com/taracutie/cassia`).

---

## Scope

**In scope:** hard-cap tighten; score-volatility soft scaling; the off-self-play head-to-head harness; gates (head-to-head SPRT + real-blitz confirmation + forfeit battery); docs + version bump.

**Deliberately deferred (YAGNI — do NOT build in this plan):**
- **Node-effort scaling** (best-move node fraction, Stockfish-style). Requires new per-root-move node instrumentation in the root search (does not exist today — `self.nodes` is a total only). Only worth the invasiveness if Tasks 2–3 leave Elo on the table; revisit then. Documented in the wrap task, not implemented here.
- **Low-time / endgame gear** (Task 5) is **contingent**: implement ONLY if the real-blitz confirmation run after Tasks 2–3 still shows endgame starvation (median clock at move 40 < 45 s at 180+2). If the hard-cap fix already resolves it, skip Task 5 entirely.

**Non-goals:** the allocation divisor (`movestogo.unwrap_or(30)`) stays — it is conventional and the peer engine uses the identical formula; it is not the problem. The `go movetime N` EXACT exemption stays untouched (UCI contract).

---

## File Structure

- `src/search/limits.rs` — **MODIFY.** `TimeManager`: add `HARD_CAP_MULT` const + change the hard formula (Task 2); add `prev_score` field + score-volatility in `report_iteration` (Task 3). Inline `#[cfg(test)] mod tests` updated alongside each change.
- `src/search/mod.rs` — **NO CODE CHANGE.** `report_iteration(depth, score, best.raw())` (line 1220) already passes the score; Task 3 only consumes it inside `TimeManager`. Listed so the engineer knows the call site is already correct.
- `tools/timebrain-h2h.sh` — **CREATE** (Task 1). Off-self-play head-to-head harness.
- `docs/strength-log.md`, `docs/field-analysis-071.md`, `docs/lichess-deploy.md` — **MODIFY** (Task 6 wrap).
- `Cargo.toml` — **MODIFY** (Task 6 wrap, version bump).

**Convention note:** this project commits directly to `main` (see prior plans). Build the v1 reference binary from the current clean HEAD *before* any change (Task 1).

---

### Task 1: Off-self-play head-to-head harness + v1 reference binary

**Files:**
- Create: `tools/timebrain-h2h.sh`
- Reference binary: `/tmp/nebchess-tb-v1` (built from current HEAD)

- [ ] **Step 1: Build the v1 reference binary from the current (pre-change) HEAD**

Run:
```bash
cd /home/witt/claude-workspace/NebChess
git status --porcelain   # expect clean (only the new plan doc untracked)
cargo build --release
cp target/release/nebchess /tmp/nebchess-tb-v1
/tmp/nebchess-tb-v1 bench 2>&1 | tail -1   # record the bench fingerprint
```
Expected: builds clean; bench prints the current `Bench 77211`-style fingerprint. Record it — every TM-only change below must leave it **identical** (TM never touches the fixed-depth bench path).

- [ ] **Step 2: Write the head-to-head harness**

Create `tools/timebrain-h2h.sh`:
```bash
#!/usr/bin/env bash
# Off-self-play TimeBrain head-to-head: NEW-TM vs OLD-TM with IDENTICAL search/eval,
# so the ONLY difference is the TimeManager. This isolates the spending-profile
# difference that same-TM self-play (the 8+0.08 strength SPRT) structurally cannot
# see (both sides share one TimeManager and starve in lockstep).
#
# Usage: tools/timebrain-h2h.sh <new-binary> <v1-binary> [tc] [elo1]
#   tc defaults to 8+0.08 — the fast arbiter. The 5x->2x hard-cap leak is
#   TC-PROPORTIONAL (one move can eat 1/3 of the clock at any TC), so a faster TC
#   still detects it but runs ~22x more games/hour than 180+2.
#   Real-blitz confirmation (slower): ROUNDS=200 tools/timebrain-h2h.sh new v1 180+2
#
# DELIBERATE deviation from the frozen strength SPRT (sprt.sh): NO draw
# adjudication. The field losses happen AFTER move 40 in long endgames; the
# protocol's `-draw movenumber=40 ... score=10` would adjudicate those equal
# positions as draws BEFORE the time-pressure loss manifests, masking the very
# effect we are measuring. Resign (clearly lost) is kept; everything else plays
# to the finish so clock survival decides the result.
set -euo pipefail
cd "$(dirname "$0")"
NEW="$(realpath "$1")"; OLD="$(realpath "$2")"; TC="${3:-8+0.08}"; ELO1="${4:-10}"
CONCURRENCY=$(( $(nproc) - 1 ))
bin/fastchess \
  -engine cmd="$NEW" name=tb-new -engine cmd="$OLD" name=tb-v1 \
  -each tc="$TC" option.Hash=16 option.Threads=1 \
  -openings file=books/8moves_v3.pgn format=pgn order=random \
  -repeat -rounds "${ROUNDS:-30000}" -recover \
  -resign movecount=3 score=600 \
  -concurrency "$CONCURRENCY" -report penta=true -ratinginterval 50 \
  -sprt elo0=0 elo1="$ELO1" alpha=0.05 beta=0.05 model=normalized
```

- [ ] **Step 3: Make it executable and smoke it (sanity: identical binaries → ~0 Elo)**

Run:
```bash
chmod +x tools/timebrain-h2h.sh
ROUNDS=20 tools/timebrain-h2h.sh /tmp/nebchess-tb-v1 /tmp/nebchess-tb-v1 8+0.08
```
Expected: it launches fastchess, plays ~40 games, prints a penta report with Elo ≈ 0 (a binary vs itself). Confirms the harness runs. (Don't wait for SPRT termination — Ctrl-C after the first rating interval; this is a plumbing smoke only.)

- [ ] **Step 4: Commit**

```bash
git add tools/timebrain-h2h.sh docs/superpowers/plans/2026-06-08-plan-8-timebrain-v2.md
git commit -m "test(timebrain): off-self-play v2-vs-v1 head-to-head harness (no draw adjudication)"
```

Commit message footer (per repo convention):
```
Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
```

---

### Task 2: Tighten the per-move hard cap (5×soft → 2×soft)

**Files:**
- Modify: `src/search/limits.rs` (the `new` constructor's wtime/btime branch, ~line 80-84; doc comment ~line 73; the inline test `clock_allocation_is_sane`, ~line 218-240)

- [ ] **Step 1: Update the failing test first (TDD — encode the new contract)**

In `src/search/limits.rs`, the test `clock_allocation_is_sane` currently asserts `assert_eq!(hard, soft * 5);`. Change that block to:
```rust
        // TimeBrain v2: hard is soft*HARD_CAP_MULT (=2) capped at usable/3 (was
        // soft*5). 60s no-inc => usable/3 ~ 19_330, so the soft*2 term (~4_000)
        // wins and the cap is soft*2.
        assert_eq!(hard, soft * 2);
```

- [ ] **Step 2: Run it to confirm it fails against the current 5× code**

Run: `cargo test -p nebchess --lib limits::tests::clock_allocation_is_sane`
Expected: FAIL — `assertion failed: left: <soft*5> right: <soft*2>`.

- [ ] **Step 3: Add the constant and change the formula**

Near the top of `src/search/limits.rs`, below `RESERVE_MIN_MS` (~line 34), add:
```rust
/// Per-move hard ceiling as a multiple of the base soft target. TimeBrain v2:
/// 2× (was 5×). The wide 5× cap let one complex middlegame move eat ~38 s at
/// 180+2 and drained the live-blitz clock into the increment by move ~30
/// (docs/field-analysis-071.md). Peer engine cutecassia caps at 1.5×; modern
/// engines sit ~2×.
pub const HARD_CAP_MULT: u64 = 2;
```

In the wtime/btime branch, change the hard line (currently `let hard = (soft * 5).min(third).max(soft);`) to:
```rust
                    let hard = (soft * HARD_CAP_MULT).min(third).max(soft);
```

Update the doc comment on that branch (currently `// hard    = the absolute one-move ceiling (a third of usable)`) to:
```rust
                    // hard    = one-move ceiling: HARD_CAP_MULT×soft, capped at a third of usable
```

- [ ] **Step 4: Run the full limits test module**

Run: `cargo test -p nebchess --lib limits::tests`
Expected: PASS — all tests green, including the updated `clock_allocation_is_sane`. (`emergency_reserve_is_never_allocated`, `hard_cap_is_a_third_of_usable`, `low_time_never_overspends` all still hold: a *smaller* hard cap only tightens those bounds.)

- [ ] **Step 5: Confirm bench fingerprint is unchanged**

Run: `cargo build --release && ./target/release/nebchess bench 2>&1 | tail -1`
Expected: IDENTICAL fingerprint to Task 1 Step 1 (TM never touches the fixed-depth bench path). If it changed, STOP — something other than TM was altered.

- [ ] **Step 6: Commit**

```bash
git add src/search/limits.rs
git commit -m "feat(timebrain): tighten per-move hard cap 5x->2x soft (clock-leak fix)"
```
(footer as in Task 1)

- [ ] **Step 7: GATE — two-stage code review, then measure**

1. **Code review** (per the standing two-stage mandate — spec/correctness + quality — on EVERY commit, no exceptions): confirm the change is exactly the cap multiplier, the clamp bounds can't invert, and no other behavior moved.
2. **Head-to-head SPRT (the arbiter), fast TC:**
   ```bash
   cargo build --release
   tools/timebrain-h2h.sh ./target/release/nebchess /tmp/nebchess-tb-v1 8+0.08 10
   ```
   Expected outcome: **H1 (new > old)** or a clear positive Elo. Apply the wanderer protocol — stop honestly at ~1500–2000 games if unresolved; never a false H0/H1.
3. **Real-blitz confirmation (slower, field-faithful):**
   ```bash
   ROUNDS=200 tools/timebrain-h2h.sh ./target/release/nebchess /tmp/nebchess-tb-v1 180+2
   ```
   Expected: non-negative Elo at 180+2, AND (spot-check a handful of the produced PGNs) NebChess's own clock at move 40 is materially higher than v1's. Record the result in the gate log.
4. **Forfeit safety:** `tools/forfeit-gauntlet.sh 60 180+2` and `tools/forfeit-gauntlet.sh 60 60+0` — expect **0 NebChess forfeits** (a tighter cap only reduces forfeit risk).
5. **Canary sanity (expected no-op):** run the WAC movetime canary once; it should be UNCHANGED — `go movetime` is EXACT-exempt from the hard cap, so the canary cannot see this change. A change here means an accidental edit to the movetime path; investigate.

**Decision:** keep the change only if the head-to-head shows ≥0 (ideally positive) and forfeits stay 0. If H0/negative, revert to 5× and try `HARD_CAP_MULT = 3` (re-run the gate); record both.

---

### Task 3: Score-volatility soft scaling

**Files:**
- Modify: `src/search/limits.rs` (`TimeManager` struct + `new` + `report_iteration` + struct/method doc comments; add new tests; update existing `report_iteration`-based tests)

- [ ] **Step 1: Write the failing tests for the new signal**

Add these tests to the inline `#[cfg(test)] mod tests` in `src/search/limits.rs`:
```rust
    #[test]
    fn score_volatility_extends_soft_on_swing() {
        // Same best move (stability neutral at iter 2) but a big eval jump ->
        // volatility raises the effective soft above base.
        let l = Limits { wtime: Some(60_000), ..Limits::default() };
        let mut tm = TimeManager::new(&l, Color::White, 10);
        let base = tm.budgets_ms().0.unwrap();
        tm.report_iteration(1, 0, 1); // first iter: volatility neutral
        tm.report_iteration(2, 150, 1); // |150-0|>=80 -> 130%, stability 100%
        assert!(tm.effective_soft_ms().unwrap() > base, "a swinging eval must extend soft");
    }

    #[test]
    fn score_volatility_shrinks_soft_when_flat() {
        // Same best move, dead-flat eval -> volatility 85% pulls soft below base.
        let l = Limits { wtime: Some(60_000), ..Limits::default() };
        let mut tm = TimeManager::new(&l, Color::White, 10);
        let base = tm.budgets_ms().0.unwrap();
        tm.report_iteration(1, 30, 1);
        tm.report_iteration(2, 32, 1); // |2|<16 -> 85%, stability 100%
        assert!(tm.effective_soft_ms().unwrap() < base, "a flat eval must shrink soft");
    }

    #[test]
    fn first_iteration_volatility_is_neutral() {
        // No previous score on iter 1 -> volatility 100%, stability 100% -> base.
        let l = Limits { wtime: Some(60_000), ..Limits::default() };
        let mut tm = TimeManager::new(&l, Color::White, 10);
        let base = tm.budgets_ms().0.unwrap();
        tm.report_iteration(1, 500, 1);
        assert_eq!(tm.effective_soft_ms().unwrap(), base, "iter 1 is neutral");
    }

    #[test]
    fn mate_score_swing_does_not_overflow_and_clamps() {
        // A mate score after a normal score is a huge delta; bucketing keeps the
        // combined pct within the [50,250] clamp and must not panic.
        let l = Limits { wtime: Some(60_000), ..Limits::default() };
        let mut tm = TimeManager::new(&l, Color::White, 10);
        let base = tm.budgets_ms().0.unwrap();
        tm.report_iteration(1, 10, 1);
        tm.report_iteration(2, 31_000, 2); // mate-ish; best move also changed
        let eff = tm.effective_soft_ms().unwrap();
        assert!(eff <= base * 250 / 100, "combined pct must stay clamped");
    }
```

- [ ] **Step 2: Run them to confirm they fail to compile/assert against current code**

Run: `cargo test -p nebchess --lib limits::tests::score_volatility_extends_soft_on_swing`
Expected: FAIL — current `report_iteration` ignores the score, so `score_volatility_shrinks_soft_when_flat` would not shrink and the swing test would equal base (test asserts strict `>`/`<`). (Compiles, but asserts fail.)

- [ ] **Step 3: Add the `prev_score` field**

In `src/search/limits.rs`, in the `TimeManager` struct, after `soft_scale_pct: u32,` add:
```rust
    prev_score: Option<i32>, // previous completed iteration's score (cp), for volatility
```
In `TimeManager::new`, in the returned struct literal (after `soft_scale_pct: 100,`), add:
```rust
            prev_score: None,
```

- [ ] **Step 4: Consume the score in `report_iteration`**

Replace the body of `report_iteration` (currently it computes only `soft_scale_pct` from stability and has a `_score_cp` param) with:
```rust
    pub fn report_iteration(&mut self, depth: i32, score_cp: i32, best_key: u16) {
        if best_key == self.last_best {
            self.stable_iters += 1;
        } else {
            self.stable_iters = 0;
            self.last_best = best_key;
        }
        // Best-move stability (unchanged): 140 on the iteration right after a
        // change (still discovering), shrinking as the PV settles.
        let stability_pct: u32 = if self.stable_iters == 0 && depth > 1 {
            140
        } else if self.stable_iters >= 3 {
            60
        } else if self.stable_iters == 2 {
            80
        } else {
            100
        };
        // Score volatility (new): a swinging eval between iterations means the
        // search is still finding things — spend longer; a flat eval means
        // settle. Bucketed so a mate-score delta can't blow up the budget.
        // Neutral on the first iteration (no previous score). This is distinct
        // from the reverted Gate-2 ABSOLUTE-score logic — it keys on |Δscore|.
        let volatility_pct: u32 = match self.prev_score {
            None => 100,
            Some(prev) => {
                let delta = score_cp.saturating_sub(prev).unsigned_abs();
                if delta >= 80 {
                    130
                } else if delta >= 40 {
                    115
                } else if delta >= 16 {
                    100
                } else {
                    85
                }
            }
        };
        self.prev_score = Some(score_cp);
        self.soft_scale_pct = (stability_pct * volatility_pct / 100).clamp(50, 250);
    }
```
Also update the method's doc comment (currently states `score_cp` is unused / Gate-2 scaffolding) to describe the volatility signal, and update the struct-level doc if it enumerates fields.

- [ ] **Step 5: Update the existing tests that this interacts with**

Two existing tests still pass but one breaks; fix the breaking one:
- `stability_scales_soft_down` — still passes (stable+flat → 60×85/100=51 < 100).
- `best_move_change_extends_soft` — still passes (changed → 140×85/100=119 > 100).
- `won_position_uses_gate1_stability` — **BREAKS**: it asserts `base * 60 / 100`. With volatility, 4 iterations of a flat winning score give stability 60 × volatility 85 = `base * 51 / 100`. Update that test's assertion and comment:
```rust
        assert_eq!(
            tm.effective_soft_ms().unwrap(),
            base * 51 / 100,
            "won + stable + flat eval = stability(60) × volatility(85); still \
             NOT the reverted won-fast (50)"
        );
```

- [ ] **Step 6: Run the whole module**

Run: `cargo test -p nebchess --lib limits::tests`
Expected: PASS — all tests green (the four new ones + the updated `won_position_uses_gate1_stability` + the unchanged rest).

- [ ] **Step 7: Confirm bench fingerprint unchanged**

Run: `cargo build --release && ./target/release/nebchess bench 2>&1 | tail -1`
Expected: IDENTICAL to Task 1 Step 1.

- [ ] **Step 8: Commit**

```bash
git add src/search/limits.rs
git commit -m "feat(timebrain): score-volatility soft scaling (|Δscore| × stability)"
```
(footer as in Task 1)

- [ ] **Step 9: GATE — two-stage review, then measure (against the binary from Task 2)**

1. **Code review** (two-stage, mandatory).
2. **Head-to-head SPRT** at 8+0.08 vs the **post-Task-2 binary** (isolate the volatility delta on top of the hard-cap fix):
   ```bash
   cp target/release/nebchess /tmp/nebchess-tb-t2   # the Task-2 binary (rebuild from the Task-2 commit if needed)
   # (build the Task-3 binary, then:)
   tools/timebrain-h2h.sh ./target/release/nebchess /tmp/nebchess-tb-t2 8+0.08 5
   ```
   Note elo1=5 (gains are small at this layer). Wanderer protocol applies.
3. **Real-blitz confirmation:** `ROUNDS=200 tools/timebrain-h2h.sh ./target/release/nebchess /tmp/nebchess-tb-t2 180+2`.
4. **Forfeit safety:** `tools/forfeit-gauntlet.sh 60 180+2` — expect 0 forfeits.

**Decision:** keep if ≥0 and forfeit-clean. If H0/negative, this is the "correct ≠ stronger" case — revert the volatility commit, keep only Task 2, and record the honest result. (The score-volatility constants are a first cut; a single retune of the buckets is allowed before abandoning, treated as one more gated attempt.)

---

### Task 4: Real-blitz endgame-clock measurement (decides whether Task 5 is needed)

**Files:** none (measurement only; uses PGNs from Task 3's confirmation run)

- [ ] **Step 1: Measure NebChess's clock at move 40 at 180+2**

From the Task-3 real-blitz confirmation PGNs (the post-cap+volatility binary), extract NebChess's `[%clk]` at move 40 across the games and compute the median. Run:
```bash
# point this at the fastchess PGN output from the 180+2 confirmation run
grep -oE '40\.\.\. [^ ]+ \{ \[%clk [0-9:]+' <pgn> | head    # spot the field, then script the median
```
(Use the same clock-reading approach as `docs/field-analysis-071.md`; a 5-line python over the PGN is fine.)
Expected: record the median clock at move 40.

- [ ] **Step 2: Decide on Task 5**

- If median clock at move 40 **≥ 45 s** → the hard-cap fix resolved the starvation. **SKIP Task 5.** Note this decision in the gate log and proceed to Task 6.
- If **< 45 s** → endgame starvation persists; proceed to Task 5 (low-time gear).

No commit (measurement only). Record the number and the decision in `docs/field-analysis-071.md` under a new "v0.8.0 measured" note.

---

### Task 5 (CONTINGENT — only if Task 4 says < 45 s): low-time / endgame gear

**Files:**
- Modify: `src/search/limits.rs` (the wtime/btime branch in `new`)

- [ ] **Step 1: Write the failing test**

Add to the inline test module:
```rust
    #[test]
    fn low_time_gear_caps_per_move_fraction() {
        // With little time left and no increment, a single move must not be
        // allowed to spend a large fraction of the remaining clock, so a long
        // technical endgame keeps thinking time for many moves.
        let l = Limits { wtime: Some(15_000), ..Limits::default() }; // 15s, deep in a game
        let tm = TimeManager::new(&l, Color::White, 10);
        let (soft, _hard) = tm.budgets_ms();
        // soft must stay modest so >~20 more moves are playable on 15s.
        assert!(soft.unwrap() <= 700, "low-time soft must stay small, got {}", soft.unwrap());
    }
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p nebchess --lib limits::tests::low_time_gear_caps_per_move_fraction`
Expected: FAIL — current soft at 15s no-inc is `15000/30 = 500`... (if it already passes at this threshold, RAISE the stringency or DROP this task — the gear isn't needed). Adjust the asserted bound to be just below the *current* value so the test meaningfully drives a change; document the chosen number.

- [ ] **Step 3: Add the low-time gear**

In the wtime/btime branch, after computing `soft` and before `hard`, add a clamp that shrinks the per-move slice when the remaining clock is low (so the tail of a long game isn't all on the increment):
```rust
                    // Low-time gear: when little clock remains, cap a single
                    // move to a small fraction of it so a long technical endgame
                    // keeps thinking time across many moves (field: 60-move
                    // endings played entirely on the increment).
                    let soft = if avail < 30_000 {
                        soft.min(avail / 25).max(1)
                    } else {
                        soft
                    };
```
(`avail` is already in scope in this branch.)

- [ ] **Step 4: Run the module**

Run: `cargo test -p nebchess --lib limits::tests`
Expected: PASS — the new test plus all prior tests (the gear only tightens low-time bounds, so `low_time_never_overspends` still holds).

- [ ] **Step 5: Confirm bench unchanged, commit**

```bash
cargo build --release && ./target/release/nebchess bench 2>&1 | tail -1   # identical
git add src/search/limits.rs
git commit -m "feat(timebrain): low-time endgame gear (cap per-move fraction when clock is low)"
```
(footer as in Task 1)

- [ ] **Step 6: GATE** — two-stage review; head-to-head vs the Task-3 binary at 180+2 (`ROUNDS=200`), expect ≥0 and median move-40 clock now ≥ 45 s; forfeit battery 0. Keep/revert per result.

---

### Task 6: M7 wrap — pick config, docs, version, memory

**Files:**
- Modify: `Cargo.toml`, `docs/strength-log.md`, `docs/field-analysis-071.md`, `docs/lichess-deploy.md`

- [ ] **Step 1: Freeze the winning config**

Confirm the kept changes (Task 2 always; Task 3 if it gated positive; Task 5 only if it was needed and gated positive). The shipping binary is the last kept commit.

- [ ] **Step 2: Record the result in the strength log**

Add a row to `docs/strength-log.md` for TimeBrain-V2: the head-to-head Elo (v2 vs v0.7.1 TM) at 8+0.08 and at 180+2, with game counts and the forfeit result. Note explicitly: this is a **time-management** gain measured **off self-play**; the anchored gauntlet (imposed openings + shared TM) will barely see it — same caveat as book/TB being live multipliers.

- [ ] **Step 3: Close out the field-analysis doc**

In `docs/field-analysis-071.md`, add a short "v0.8.0 outcome" section: which levers shipped, the measured move-40 clock before/after at 180+2, and whether the node-effort signal is still deferred (it is, unless a later plan needs it).

- [ ] **Step 4: Update the deploy doc**

In `docs/lichess-deploy.md`, update the "Time management" section: TimeBrain-V2 ships a tighter hard cap + score-volatility scaling; still forfeit-clean across 60+0 / 180+2 / 300+3; no movetime override needed.

- [ ] **Step 5: Version bump**

In `Cargo.toml`, bump `version` to `0.8.0` (notable behavior change / milestone). Run `cargo build --release` and confirm `printf 'uci\nquit\n' | ./target/release/nebchess` prints `id name NebChess 0.8.0` + `uciok`.

- [ ] **Step 6: Commit + tag**

```bash
git add Cargo.toml docs/strength-log.md docs/field-analysis-071.md docs/lichess-deploy.md
git commit -m "chore: v0.8.0 — TimeBrain-V2 (hard cap 2x + score-volatility), off-self-play validated"
git tag v0.8.0
```
(footer as in Task 1. Push/tag only when the user asks.)

- [ ] **Step 7: Update memory**

Update `memory/timebrain-v2-field-finding.md`: mark TimeBrain-V2 shipped, record what gated positive/negative, and note node-effort still deferred. Update `memory/nebchess-project-state.md`: 0.8.0 = TimeBrain-V2, next milestone = **NNUE** (the 3000+ ceiling-raiser; training = Stockfish-eval distillation per the user's idea, with self-play data generation as a later RL-flavored evolution).

---

## Self-Review

**1. Spec coverage:**
- Hard cap 5×→2× → Task 2. ✓
- Score-volatility scaling → Task 3. ✓
- Node-effort → explicitly deferred (Scope + Task 6 Step 7), with the instrumentation reason. ✓
- Contingent low-time gear → Task 5, gated on Task 4's measurement. ✓
- Validate OFF self-play → Task 1 harness (no draw adjudication) + gates in Tasks 2/3/5. ✓
- Keep divisor 30 / movetime EXACT → Scope non-goals; no task touches them. ✓
- Forfeit-clean + canary + 8+0.08 strength-neutrality → covered, with the note that canary/bench are TM-insensitive so the head-to-head is the real arbiter. ✓
- Two-stage review every commit → every GATE step + commits in Tasks 1–6. ✓

**2. Placeholder scan:** every code step shows the exact code; commands have expected output. The one judgment point is Task 5 Step 2's asserted bound (must be set just below the observed current value) — flagged inline as a deliberate calibration, not a placeholder, and Task 5 only runs if Task 4 triggers it.

**3. Type consistency:** `HARD_CAP_MULT: u64` used in `soft * HARD_CAP_MULT` (soft is u64 ms). `prev_score: Option<i32>`, `score_cp: i32`, `saturating_sub(prev).unsigned_abs(): u32`, combined `(u32 * u32 / 100).clamp(50,250): u32` assigned to `soft_scale_pct: u32`. `report_iteration(depth: i32, score_cp: i32, best_key: u16)` matches the existing call `tm.report_iteration(depth, score, best.raw())` (score is i32, `best.raw()` is u16). Consistent.
