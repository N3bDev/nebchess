# NebChess Plan 7 (M6.2 + M6.3): TimeBrain + Bot Readiness + Pondering → v0.7.0

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert lab strength into live strength: TimeBrain v1 (the field-telemetry headliner — clock collapse is the dominant leak), opening book, Syzygy, Lichess hardening, and pondering on persistent search state; ship v0.7.0.

**Architecture:** TimeBrain rebuilds `src/search/limits.rs` around a stateful per-search controller consulted between iterations (stability/score-trend aware) — search itself unchanged. Book and Syzygy are root-level move sources gated by UCI options, untouched search semantics. Pondering converts the per-`go` SearchThread into a persistent SearchState owned by the UCI loop (which also delivers conthist cross-move persistence — the Plan-6 review note).

**Tech Stack:** Rust std-only EXCEPT pyrrhic-rs for Syzygy (the spec's M8 decision — the project's single external dependency, version-pinned; document the policy exception in Cargo.toml comments). fastchess 1.8.1, probe/SPRT/canary harnesses as established.

**Spec:** `docs/superpowers/specs/2026-06-06-m6-design.md` (M6.2/M6.3 sections + the v0.5.0 telemetry addendum). **Current state:** HEAD = v0.6.0 (650bf84-era, Bench 54728, 154 tests, WAC ship 273, baseline chain head `tools/bin/baseline-conthist`, anchored 2811.4 ±15.9, target 2900). Field corpus: `db/lichess_nebchessbot_0.5.0.pgn` (38 games, 37% draws).

**Gate protocol:** unchanged per-feature train (implement+tests → review → canary alone → SPRT alone → ledgers → baseline). **TM-specific addition (spec):** every TimeBrain gate ALSO runs the idle forfeit gauntlet (0/200 required); the phase ends with a sudden-death stress (60+0) and an LTC probe (60+0.6). Pondering's gate: ponder-enabled fastchess SPRT if the runner supports it cleanly, else documented soak + field telemetry (spec-approved fallback).

---

## File structure (end state)

```
src/search/limits.rs   # REWRITTEN: TimeBrain (stateful controller)
src/search/mod.rs      # iterate(): stability/score reporting into TimeBrain; persistent-state refactor (T7)
src/uci/mod.rs         # BookFile/BookDepth/SyzygyPath/Ponder options; ponder/ponderhit; SearchState ownership
src/book/mod.rs        # NEW: Polyglot reader (key computation + lookup + weighted pick)
src/book/polyglot_random.rs  # NEW: the 781 standard PolyGlot u64s (vendored, public domain)
src/bin/bookgen.rs     # NEW: PGN -> .bin builder (filters per spec)
src/tb.rs              # NEW: pyrrhic-rs wrapper (WDL/DTZ probes, piece-count/path gating)
tools/download-syzygy.sh     # NEW: 3-4-5-men tables (~1GB)
tools/suites/sac-entrance.epd # NEW (T1): sacrifice-entrance suite from the field corpus
docs/field-analysis-050.md   # NEW (T1): draw classification report
```

---

### Task 1: Field-corpus analysis — draw classification + sacrifice-entrance suite

**Files:** Create `docs/field-analysis-050.md`, `tools/suites/sac-entrance.epd`. No engine changes.

- [ ] **Step 1.1:** Parse `db/lichess_nebchessbot_0.5.0.pgn` (38 games). For each of the 14 draws: replay the game via the library (`Position` + `find_san_move` — the EPD resolver handles SAN), and at every NebChess-to-move position from move 20 on, run a 2-second search (`search_to_depth` is depth-based — use the UCI binary with `go movetime 2000` via a persistent pipe instead; idle system, ~15 min total). Record the max eval NebChess held in the final 30 plies.
- [ ] **Step 1.2:** Classify each draw in `docs/field-analysis-050.md`: **LEAKED** (held ≥ +200cp within the final 30 plies, drawn anyway — subclassify: repetition/perpetual, 50-move, insufficient-material shuffle) vs **FAIR** (never better than +100cp). Table: game id, opponent, max-eval-held, terminal mechanism, class, one-line note. Summary counts + the leaked-half-points total.
- [ ] **Step 1.3:** For the LEAKED-perpetual games: extract the position where the eval FIRST dropped ≥150cp toward the draw (the leak moment) as EPD with the engine's preferred-at-2s move vs the move played; note whether deeper search (10s spot-check on the worst 3) flips it — that distinguishes "depth/time" leaks (TimeBrain-fixable) from "knowledge" leaks (eval work, M7+).
- [ ] **Step 1.4:** Build `tools/suites/sac-entrance.epd`: from the LOSSES and DRAWS, find positions where the engine declined/missed a tactical entrance (the user-reported Greek-gift miss + any others where a 10s search finds a sacrifice the game move missed): EPD lines with `bm` set to the sacrifice, `id "SAC.nnn"`, ≥5 positions (pad from the WAC misses' sacrifice subset if the corpus yields fewer — note provenance per line). This suite is INFORMATIONAL (not a gate) — a depth probe for M7+ extension work.
- [ ] **Step 1.5:** Commit `docs: v0.5.0 field analysis — draw classification + sac-entrance suite`.

### Task 2: TimeBrain Gate 1 — allocation core — SPRT GATE #1 ([0,5]) + forfeit gauntlet

**Files:** Rewrite `src/search/limits.rs`; modify `src/search/mod.rs` (iterate loop only).

- [ ] **Step 2.1: Failing tests first** (limits.rs `#[cfg(test)]` — keep the 5 existing tests, they pin behavior that must survive; add):

```rust
#[test]
fn emergency_reserve_is_never_allocated() {
    // 1000ms left, no inc: hard must leave >= RESERVE_MIN_MS (50) + overhead untouched
    let l = Limits { wtime: Some(1_000), ..Limits::default() };
    let tm = TimeManager::new(&l, Color::White, 50);
    let (_, hard) = tm.budgets_ms();
    assert!(hard.unwrap() <= 1_000 - 50 - 50, "hard {} must respect reserve", hard.unwrap());
}
#[test]
fn stability_scales_soft_down() {
    let l = Limits { wtime: Some(60_000), ..Limits::default() };
    let mut tm = TimeManager::new(&l, Color::White, 10);
    let base = tm.budgets_ms().0.unwrap();
    // 3 iterations, same best move, steady score -> effective soft shrinks
    for d in 1..=3 { tm.report_iteration(d, 25, 1 /*same move key*/); }
    assert!(tm.effective_soft_ms().unwrap() < base, "stable PV must shrink soft");
}
#[test]
fn best_move_change_extends_soft() {
    let l = Limits { wtime: Some(60_000), ..Limits::default() };
    let mut tm = TimeManager::new(&l, Color::White, 10);
    let base = tm.budgets_ms().0.unwrap();
    tm.report_iteration(1, 25, 1);
    tm.report_iteration(2, 25, 2); // best move CHANGED
    assert!(tm.effective_soft_ms().unwrap() > base, "instability must extend soft");
}
#[test]
fn hard_cap_is_a_third_of_usable() {
    let l = Limits { wtime: Some(30_000), ..Limits::default() };
    let tm = TimeManager::new(&l, Color::White, 10);
    let (_, hard) = tm.budgets_ms();
    assert!(hard.unwrap() <= 10_000, "never >1/3 of usable on one move");
}
```

- [ ] **Step 2.2: The controller.** Rewrite TimeManager:

```rust
pub struct TimeManager {
    start: Instant,
    soft: Option<Duration>,      // base soft
    hard: Option<Duration>,
    // Gate-1 state (Gate 2 adds score-trend fields in T3):
    last_best: u16,              // caller-supplied move key (Move's raw bits)
    stable_iters: u32,           // consecutive iterations with the same best
    soft_scale_pct: u32,         // 100 = base; recomputed by report_iteration
}
pub const RESERVE_MIN_MS: u64 = 50;

// allocation (replaces the body of new()'s clock branch):
// avail   = time - overhead
// reserve = (avail / 16).clamp(RESERVE_MIN_MS, 2_000)   // flag protection
// usable  = avail - reserve  (min 1)
// mtg     = movestogo.unwrap_or(30).clamp(1, 40)
// soft    = (usable / mtg + inc * 3 / 4).clamp(1, usable / 3)
// hard    = (soft * 5).min(usable / 3).max(soft)
```

`report_iteration(&mut self, depth: i32, score_cp: i32, best_key: u16)`: Gate 1 uses only best_key — same as last → `stable_iters += 1` else reset to 0 and `last_best = best_key`. `soft_scale_pct` = 60 when `stable_iters >= 3`, 80 when `== 2`, 100 when `< 2`, **140 on the iteration right after a change** (`stable_iters == 0 && depth > 1`), all values then clamped so `effective_soft <= hard`. `effective_soft_ms()` exposes `soft * soft_scale_pct / 100`. `past_soft()` now compares against effective soft (keep the method name — iterate's call site survives).

- [ ] **Step 2.3:** iterate() (mod.rs): after each completed iteration, before the `past_soft` check: `tm.report_iteration(depth, score, best.map_or(0, |m| m.raw()))` (add a `raw()` accessor on Move if absent — or use `.0` per its repr; verify against the Move type). NOTHING else in search changes.
- [ ] **Step 2.4:** All tests green (old 5 + new 4); fmt/clippy; bench ×2 (unchanged 54728 — bench is depth-fixed, no TM involvement; verify); commit `feat(time): TimeBrain gate 1 — reserve, stability scaling, hard cap` + `Bench: 54728`.
- [ ] **Step 2.5 (CONTROLLER):** canary (ref 273 — TM doesn't touch fixed-movetime solves; expect ±1) → SPRT #1 [0,5] vs `baseline-conthist` → **idle forfeit gauntlet 0/200 required** → rows → `tools/baseline.sh timebrain1`.

### Task 3: TimeBrain Gate 2 — adaptivity — SPRT GATE #2 ([0,5]) + phase capstone

**Files:** Modify `src/search/limits.rs`, `src/search/mod.rs` (iterate only).

- [ ] **Step 3.1: Tests first** (same style): (a) score drop ≥50cp between iterations at depth ≥8 → effective soft extends ×150 (pct 150, capped by hard); (b) score ≥ +500 with `stable_iters >= 3` → pct 50; (c) behind-on-clock: `TimeManager::new` taking `opp_time: Option<u64>` (threaded from Limits — wtime/btime both present in `go`) → when `my_time < opp_time * 6 / 10`, base soft × 0.8 at allocation. Write the three tests with exact numbers.
- [ ] **Step 3.2:** Implement: extend `report_iteration` with `prev_score: Option<i32>` tracking; the pct resolution ORDER (document in code): panic-extend (score drop) **beats** won-fast (high score) **beats** stability; catch-up applies at allocation not per-iteration. `effective_soft` always `<= hard`, always `>= soft / 4` (floor — never blitz out on one fluky iteration).
- [ ] **Step 3.3:** Tests green; fmt/clippy; bench ×2 unchanged; commit `feat(time): TimeBrain gate 2 — panic extension, won-fast, clock catch-up` + Bench.
- [ ] **Step 3.4 (CONTROLLER):** canary → SPRT #2 [0,5] vs `baseline-timebrain1` → forfeit gauntlet 0/200 → rows → `tools/baseline.sh timebrain2`.
- [ ] **Step 3.5 (CONTROLLER, phase capstone):** (a) **sudden-death stress**: `tools/probe.sh`-style fastchess run, 200 games at **60+0** self-play — zero NebChess forfeits required; (b) **LTC probe**: 100 games at 60+0.6 vs baseline-conthist (informational elo, sanity that TimeBrain transfers); (c) **the KR-K-on-seconds stress** (the field R-vs-K draw): script `tools/krk-stress.sh` — from FEN `8/8/8/4k3/8/8/4K3/4R3 w - - 0 1`, fastchess 20 games NebChess-vs-NebChess at **5+0.1**, every game must end 1-0/0-1 (mate delivered) — no draws allowed; any draw = the clock-collapse bug reproduced, halt and attribute. Log all three in the ledgers.

### Task 4: Polyglot opening book — reader + builder — SPRT GATE #3 ([0,5])

**Files:** Create `src/book/mod.rs`, `src/book/polyglot_random.rs`, `src/bin/bookgen.rs`; modify `src/uci/mod.rs`, `src/lib.rs` (module decl).

- [ ] **Step 4.1: The key standard.** Vendor the 781 PolyGlot random u64s into `polyglot_random.rs` (`pub static RANDOM: [u64; 781]`) from the canonical public-domain table (hgm.nubati.net/book_format.html or any engine's vendored copy — they are bit-identical everywhere; first value `0x9D39247E33776D41`). Key composition (in `mod.rs`): piece-square (offset 64*kind_of_piece + 8*row + file; kind = bp=0,wp=1,bn=2,wn=3,bb=4,wb=5,br=6,wr=7,bq=8,wq=9,bk=10,wk=11), castling (768..772 = white-short,white-long,black-short,black-long), ep file (772..780, ONLY when a capturing pawn stands adjacent — match the standard exactly), turn (780, white to move). **Anchor test:** `polyglot_key(startpos) == 0x463B96181691FC9C` plus the 4 other reference keys from the spec page (e2e4 position = 0x823C9B50FD114196, etc. — vendor all the published reference FEN/key pairs as tests).
- [ ] **Step 4.2: Reader.** `Book::open(path) -> io::Result<Book>` (reads the whole file — books are MBs; entries are 16 bytes big-endian: key u64, move u16, weight u16, learn u32). `Book::pick(&self, pos: &Position, rng_key: u64) -> Option<Move>`: binary-search the sorted entries for the key, weighted-random among matches using a xorshift seeded from `rng_key` (caller passes `pos.key() ^ game_ply` — deterministic per position per game, varied across games), decode the polyglot move encoding (from/to/promo bit-packed; castling encoded as e1h1-style king-takes-rook — translate to our castling Move via find_uci_move on the four special cases). Tests: a hand-built 3-entry book file (write bytes in the test) round-trips; castling translation verified.
- [ ] **Step 4.3: UCI wiring.** Options `BookFile` (string, default empty = off) + `BookDepth` (spin 1..40, default 16 plies). In `cmd_go`: if book is loaded AND game ply < BookDepth*... (track ply from the `position` command's move count) AND `pick` returns a move → `bestmove` immediately (no search thread). `info string book move` printed. Tests: UCI integration test with the hand-built book.
- [ ] **Step 4.4: Builder** (`src/bin/bookgen.rs`): `bookgen <out.bin> <pgn> [<pgn>...]` with flags `--min-elo 2300 --min-plies 40 --max-book-plies 16 --min-count 2`. Parse PGNs game by game (reuse `find_san_move` for SAN; skip games failing filters — for the Online2025 source also skip `[Event` containing "Bullet", players with "BOT"/"bot" titles... concretely: skip when either `[WhiteTitle "BOT"]`/`[BlackTitle "BOT"]` present or TimeControl base < 180s). Accumulate per (key, move): count, wins-for-mover*2+draws. weight = score (u16-saturated, min-count filtered). Sort by key, write big-endian. Report: games read/kept, positions, entries. **Run it:** `bookgen tools/books/nebbook.bin db/export_ELO2400.pgn` (OTB soundness base per spec; Online2025 weighting deferred to M7 — record the deferral) — expect minutes on 748MB; report counts.
- [ ] **Step 4.5:** fmt/clippy/tests; bench unchanged (no search change); commit `feat(book): Polyglot reader + bookgen builder + UCI options` + Bench line unchanged-note.
- [ ] **Step 4.6 (CONTROLLER):** canary (book OFF in canary — solve uses fixed positions, unaffected; run anyway, expect 273) → **SPRT #3 [0,5]: new = with `option.BookFile=tools/books/nebbook.bin`, old = no book, MATCH OPENINGS REMOVED** (custom sprt invocation: copy sprt.sh to a one-off with `-openings` dropped and the option added to the NEW side only — document the protocol deviation in the ledger row: book gates can't use a book-imposed opening set) → forfeit check → rows → baseline binary unchanged (book is data + flag, the BINARY is the same — note `tools/baseline.sh` NOT run; the chain head stays baseline-conthist with book-off defaults).

### Task 5: Syzygy 3-4-5 via pyrrhic-rs — SPRT GATE #4 ([0,5])

**Files:** Create `src/tb.rs`, `tools/download-syzygy.sh`; modify `Cargo.toml` (the single external dep, pinned + policy comment), `src/uci/mod.rs` (SyzygyPath option), `src/search/mod.rs` (root + interior probe hooks).

- [ ] **Step 5.1:** `tools/download-syzygy.sh`: 3-4-5-men WDL+DTZ from the canonical mirror (tablebase.lichess.ovh syzygy/3-4-5 or sesse mirror — implementer verifies availability; ~1GB total into tools/tb/, gitignored; non-fatal skip with report if unreachable). Run it.
- [ ] **Step 5.2:** `Cargo.toml`: `pyrrhic-rs = "=<latest>"` (exact-pin; comment: "the project's single external dependency — spec M8 decision; std-only otherwise"). `src/tb.rs`: thin wrapper — `Tb::init(path) -> Option<Tb>` (probes max-men), `probe_wdl(pos) -> Option<Wdl>` (gated: piece count ≤ tb_men, halfmove == 0 castling none — match pyrrhic's preconditions), `probe_root(pos) -> Option<(Move, Wdl)>` (DTZ-ranked best at root). The pyrrhic-rs API surface differs across versions — implementer reads its docs (docs.rs) and adapts; the WRAPPER signatures above are ours and fixed.
- [ ] **Step 5.3:** Search integration: in `iterate` at root — if `probe_root` hits, play it immediately when winning/drawing-best (still search when losing — TB move ordering only... keep v1 simple per YAGNI: root hit → return the DTZ move, full stop). Interior: in negamax entry after the draw checks — `if pieces <= men && halfmove == 0 && depth >= 4 { probe_wdl -> return mate-bound-ish scores (win = MATE_BOUND-100-ply, loss = -(...), draw = draw_score) with TT store }` (exact score scheme in code comments; do NOT use real MATE scores — TB wins aren't proven mates).
- [ ] **Step 5.4:** Correctness suite: KQvK white-to-move = Win; KRvK = Win; KvK = Draw; KPvK a2/a7-pawn edge cases (known draw/win pairs); plus `4k3/8/8/8/8/8/4P3/4K3 w` = Win. Skip-if-no-tables guard (`#[ignore]`-by-default + a non-ignored smoke that asserts `Tb::init` works when tools/tb exists — CI has no tables; document).
- [ ] **Step 5.5:** fmt/clippy/tests; bench ×2 (UNCHANGED — bench positions are >5 men; verify); commit `feat(tb): Syzygy 3-4-5 WDL/DTZ via pyrrhic-rs (the single external dep)` + Bench.
- [ ] **Step 5.6 (CONTROLLER):** canary (expect ±1 — WAC is middlegame) → SPRT #4 [0,5] vs baseline-conthist with `option.SyzygyPath` on the NEW side (endgame adjudication OFF for this run — draw/resign adjudication masks TB gains; one-off sprt variant, documented) → rows → note: binary gains a dep — `tools/baseline.sh syzygy` DOES run (the binary changed).

### Task 6: Lichess hardening — multi-TC forfeit battery (no SPRT — operational gate)

**Files:** possibly `src/uci/mod.rs` (only if defects found); `docs/field-analysis-050.md` appended.

- [ ] **Step 6.1 (CONTROLLER):** forfeit battery at the LIVE TCs: 200 games each at **180+2** and **300+3** (the corpus TCs) + 200 at **60+0** SD (re-run post-book/TB) — zero NebChess forfeits required at all three; any forfeit = halt-and-fix-and-rerun.
- [ ] **Step 6.2:** UCI robustness sweep (scripted, `tools/uci-torture.sh` — write it): `position` without `moves`, illegal FENs (the en-prise-king class), `go` with zero/negative times, `stop` storms (50 rapid stop/go cycles), `ucinewgame` mid-search, EOF mid-search — engine must never panic/hang (timeout-guarded script, exit 0 = clean). Fix any defect found (each fix = its own reviewed commit).
- [ ] **Step 6.3:** lichess-bot config review (docs only — `docs/lichess-deploy.md`: recommended move_overhead for their setup, TC ranges to accept, book/TB paths, ponder flag once T7 lands).
- [ ] **Step 6.4:** Commit(s) + ledger note.

### Task 7: Persistent search state + pondering capstone — GATE #5 (SPRT-if-supported, else soak+field)

**Files:** Modify `src/search/mod.rs`, `src/uci/mod.rs`.

- [ ] **Step 7.1: Persistence refactor (bench-identical-per-search, behavior-change-across-searches):** histories (butterfly + conthist1/2) move out of per-`go` SearchThread construction into a `SearchState` owned by the Uci struct, passed `&mut` into the search thread per `go`... threads: the search runs on a spawned thread — ownership: `SearchState` lives in an `Arc<Mutex<...>>`? NO — simpler: the Uci loop already joins the search thread before the next `go` (stop_and_join); move the state into the thread and RETURN it via the JoinHandle (`spawn(move || { ...; state })`, reclaim at join). `ucinewgame` resets it. Tests: a two-`go` UCI integration test asserting a conthist cell survives `go`#1 into `go`#2 and is cleared by `ucinewgame`. This is a CHANGE in cross-move behavior — it gates with pondering as one SPRT (they ship together; a solo probe first: 400 games persistence-only — if it probes ≥+10 alone it earns its own [0,5] SPRT first, implementer reports and controller decides).
- [ ] **Step 7.2: Pondering.** Advertise `option name Ponder type check default false`. `go ponder` → search the given position at infinite (limits.infinite path); store `pondering = true`. `ponderhit` → the search CONTINUES but the TimeManager must now apply: implement by having TimeManager support `arm(limits, elapsed_credit)` — on ponderhit, compute budgets from the REAL limits (sent with `go ponder` per UCI — wtime/btime were in the go command) and credit the already-elapsed ponder time as free (start stays the original Instant — effective: deadlines measured from ponderhit MINUS nothing... concretely: budgets computed normally but `start = Instant::now()` at ponderhit — opponent time was free). Stop semantics: `stop` during ponder → bestmove from current search (the M2 stop discipline holds). A miss = GUI sends `stop` + new `position`/`go` — already handled by stop_and_join. Tests: scripted UCI session (the uci integration test harness) — go ponder → ponderhit → bestmove arrives within budget; go ponder → stop → bestmove arrives immediately; 25-round watchdog like zero_delay_stop_never_hangs.
- [ ] **Step 7.3:** fmt/clippy/tests; bench ×2 unchanged; commit `feat(uci): pondering + persistent search state (conthist cross-move)` + Bench.
- [ ] **Step 7.4 (CONTROLLER):** check fastchess ponder support (`-each ponder=true`?— read fastchess docs/--help). If supported: SPRT #5 [0,5] ponder-on-both-sides vs baseline-conthist... NOTE ponder-on-both at concurrency saturates cores — drop concurrency to nproc/2-1 for this run, document. If NOT supported: the spec fallback — 2h soak (continuous self-play cutechess-free loop via scripted UCI pipes, zero violations) + ship flagged for field telemetry. Forfeit gauntlet either way. Rows + `tools/baseline.sh ponder`.

### Task 8: v0.7.0 wrap

- [ ] **Step 8.1:** Cargo.toml 0.7.0; README: tick M6b, add `- [ ] M7: eval round 2 (outposts, king-attack rework, gated check extensions) + deeper-search retries (singular, futility v2) + desktop migration`; strength line updated post-gauntlet. Commit.
- [ ] **Step 8.2 (CONTROLLER):** anchored gauntlet (mixed pool, 300/rung) — the 2900 verdict; multi-TC forfeit battery rerun (180+2/300+3/60+0); WAC trend row; strength-log row (book+TB OFF for the gauntlet binary settings? NO — gauntlet measures the SHIPPING config: book ON via the one-off args is NOT possible with pinned anchors fairness... DECISION: gauntlet runs engine-default (book off, TB on via SyzygyPath if tables local) — the book is a Lichess-deploy artifact, not an engine-strength claim; document in the row).
- [ ] **Step 8.3:** tag v0.7.0, push main+tags, CI green, final milestone review agent (ledger coherence, plan checkboxes, the Cargo.lock dep audit, loose ends), memory update, user report + redeploy checklist (book path, SyzygyPath, ponder flag, move overhead).

---

## Plan self-review notes

- **Spec coverage:** telemetry classification ✓(T1), sac-entrance suite ✓(T1), TimeBrain 2 gates + SD/LTC capstone + KR-K stress ✓(T2/T3), move-time telemetry — FOLD into T2 (`info string time soft=.. used=..` per move, ungated) — add to step 2.3; book reader+builder+filters ✓(T4), Syzygy ✓(T5), hardening ✓(T6), pondering + conthist persistence ✓(T7), v0.7.0 ✓(T8).
- **Type consistency:** `report_iteration(depth, score_cp, best_key)` defined T2, extended T3 (prev_score is internal state, not a param); `effective_soft_ms()` T2, consumed by `past_soft()`; `Book::pick(pos, rng_key)` T4; `Tb::probe_wdl/probe_root` T5; `SearchState` returned-via-JoinHandle T7.
- **Known decisions recorded:** book gate drops match openings (deviation documented per-row); Syzygy gate disables adjudication (same); gauntlet measures engine-default config; pyrrhic-rs is the single external dep; Online2025 book weighting deferred to M7.
- **Wall-clock:** ~5 SPRTs + 1 probe + 4+ forfeit batteries + 1 gauntlet + T1 analysis ≈ 2 days of compute.
