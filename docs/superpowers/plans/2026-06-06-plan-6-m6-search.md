# NebChess Plan 6 (M6.0 + M6.1): Bracketed Measurement + Search Polish → v0.6.0

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the measurement (mixed ≥3-family anchor pool, bracketed re-measurement, data-driven target), then bank the remaining big search gains (SEE, conthist, history-LMR, singular+check extensions, futility v2, margin sweep) and ship v0.6.0.

**Architecture:** M6.0 is tooling + measurement (no engine changes). M6.1 modifies `src/search/` only — eval is untouched (no retunes this plan; K stays frozen; eval_params.rs must not change). Every search feature lands behind the established gate train: implement+tests → review → canary alone → SPRT alone → ledger rows → `tools/baseline.sh <name>`.

**Tech Stack:** Rust std-only. fastchess 1.8.1, Ordo, WAC canary, frozen SPRT protocol v1 (tools/sprt.sh), probe harness (tools/probe.sh).

**Spec:** `docs/superpowers/specs/2026-06-06-m6-design.md`. **Current state:** HEAD = ca63e34, v0.5.0, Bench 77211, 146 tests, WAC reference 268/299, baseline chain head `tools/bin/baseline-hybrid`.

**Gate protocol per search task (T4-T9):** implement + tests → review; bench ×2 → commit with `Bench:` line; CONTROLLER: canary alone (`./target/release/solve tools/suites/wac.epd 1000`, floor = previous entry −10, halt-and-attribute); CONTROLLER: SPRT alone vs the previous baseline binary; log rows (sprt-log + tactics-log); `tools/baseline.sh <name>`. NO eval retunes in this plan — if a task seems to need one, STOP and report.

---

## File structure (end state)

```
src/search/
  mod.rs        # negamax/qsearch/iterate (modified T4-T9)
  see.rs        # NEW (T4): static exchange evaluation
  tt.rs         # unchanged
tools/get-anchors.sh       # T2: mixed pool (Stash spine + 2-3 other families)
tools/anchored-gauntlet.sh # T2: new case-mappings
docs/{sprt,tactics,strength}-log.md  # T1 restructure + rows per gate
README.md      # T1 target text; T10 milestone tick
Cargo.toml     # T10: 0.6.0
```

---

### Task 1: Ledger restructure + README target + retroactive v0.5.0 tag

**Files:** Modify `docs/sprt-log.md`, `docs/tactics-log.md`, `docs/strength-log.md`, `README.md`.

- [x] **Step 1.1:** In `docs/sprt-log.md`: move the `**M3 cumulative: ... M4 cumulative: ...**` paragraph from below the table to directly UNDER the intro paragraph (above the table header). Append to it: `**M5 cumulative: +379.1 self-play (gates T1-T6) → +369 anchored (2414→2783).**` Verify the table renders (header row + separator immediately precede the first data row).
- [x] **Step 1.2:** Same restructure in `docs/tactics-log.md`: the `**M5 canary trend...**` paragraph moves above the table, under the intro. In `docs/strength-log.md`: confirm every row is a well-formed table row (the 2026-06-06 row was heredoc-appended — if the table header/alignment row is missing or prose interleaves rows, normalize so the file is: intro paragraph(s), summary line, single table).
- [x] **Step 1.3:** `README.md`: replace any "~2400" / "2400 ELO" target text with: `Measured 2783 ± 22 (10+0.1, anchored vs a Stash ladder — see docs/strength-log.md for caveats). M6 target: set after bracketed re-measurement (2900 path).` Keep the milestone checklist untouched (T10 owns it).
- [x] **Step 1.4:** Tag the release commit: `git tag -a v0.5.0 b04d2ee -m "M5: full HCE + hybrid Texel tune; anchored 2783±22"` then `git push origin v0.5.0`.
- [x] **Step 1.5:** `cargo test --quiet` (docs-only, must stay green), commit `docs: ledger restructure (summaries above tables) + README target + v0.5.0 tag`.

### Task 2: Mixed anchor pool

**Files:** Modify `tools/get-anchors.sh`, `tools/anchored-gauntlet.sh`.

- [x] **Step 2.1 (research, no code):** Verify on the live CCRL Blitz list (record the list date in script comments): (a) Stash v22 and v23 ratings — take whichever land in 2750–2950 (expect roughly v22≈2790, v23≈2880 — VERIFY, do not trust these numbers); (b) pick 2-3 other-family rungs in 2700–2950 from this candidate matrix (verify each rating + an official Linux x86-64 binary or a clean `make`-from-source release): Ethereal (8.61/9.30 era), Weiss (1.0/2.0 era), Halogen (8/9/10 era), Koivisto (4.x era), Marvin (4.x era), Igel (2.x era). Selection rule: official binary preferred; source build acceptable if `make` completes with no deps beyond a C/C++ toolchain; NO NNUE-download-at-runtime engines unless the net ships in the release archive.
- [x] **Step 2.2:** Extend `tools/get-anchors.sh`: new entries follow the existing per-engine pattern (download → chmod → UCI handshake verify → `IN_POOL` gating). New pool (replaces the old `IN_POOL` set): `Stash19 Stash20 Stash21 <Stash22|23> <Family2> <Family3> [<Family4>]` — ≥6 rungs, ≥3 families, ≥2 rungs ≥2800. v15/v17 leave `IN_POOL` (stay archival). `ratings.txt` entries carry the verified CCRL pins. The full-pool guard (`POOL_VERIFIED -lt ${#IN_POOL[@]}` → BLOCKED) already exists — do not weaken it. Non-Stash engines: if from source, build into `tools/bin/anchors/` with the binary name embedded in the script (deterministic, matching gauntlet case-mappings).
- [x] **Step 2.3:** Extend `tools/anchored-gauntlet.sh` case-mappings for each new pool engine (exact binary filename).
- [x] **Step 2.4:** Run `tools/get-anchors.sh` — all pool engines download/build + UCI-verify; paste each new engine's `id name` line into the commit message body. Run `bash -n` on both scripts.
- [x] **Step 2.5:** Commit `feat(tools): mixed anchor pool — Stash spine + <families> (CCRL <list-date> pins)`.

### Task 3 (CONTROLLER): Bracketed re-measurement of v0.5.0 + target decision

- [x] **Step 3.1:** Idle system. `tools/anchored-gauntlet.sh 300` (~2h at 6-7 rungs). The binary measured must be the v0.5.0 build (`git stash` any WIP; verify `./target/release/nebchess bench` = 77211 before starting).
- [x] **Step 3.2:** strength-log row: bracketed rating ± error, per-rung scores, family list, CCRL list date, TC caveat. Compare with the extrapolated 2783: agreement within ~±30 validates the Stash-only measurement; larger deviation gets a sentence of analysis (style intransitivity quantified).
- [x] **Step 3.3:** Target decision per spec: bracketed ≥2750 → README target line becomes `M6 target: 2900 (stretch 3000)`; 2720-2750 → `2875`; ≤2720 → `2850`. Commit `docs: bracketed 0.5.0 measurement + M6 target`.

### Task 4: SEE + qsearch SEE pruning — SPRT GATE #1 ([0,5])

**Files:** Create `src/search/see.rs`; modify `src/search/mod.rs` (module decl + qsearch loop).

- [x] **Step 4.1: Failing tests first** (in `see.rs` `#[cfg(test)]`):

```rust
// Values: pawn 100, knight 320, bishop 330, rook 500, queen 900 (SEE-local,
// decoupled from eval — same rationale as the pinned ordering VICTIM_VALS).
#[test]
fn pawn_takes_defended_pawn_is_zero() {
    // e4xd5, d5 defended by c6 pawn: PxP, PxP -> 100 - 100 = 0
    let pos = Position::from_fen("4k3/8/2p5/3p4/4P3/8/8/4K3 w - - 0 1").unwrap();
    let mv = find_uci_move(&pos, "e4d5").unwrap();
    assert_eq!(see(&pos, mv), 0);
}
#[test]
fn rook_takes_defended_pawn_loses_exchange() {
    // Rxd5 (pawn), d5 defended by e6 pawn: +100 - 500 = -400
    let pos = Position::from_fen("4k3/8/4p3/3p4/8/8/8/3RK3 w - - 0 1").unwrap();
    let mv = find_uci_move(&pos, "d1d5").unwrap();
    assert_eq!(see(&pos, mv), -400);
}
#[test]
fn xray_battery_wins_pawn() {
    // Rxd5 with rook battery (Rd1,Rd2... use Rd1 behind Rd3) vs lone defender:
    // RxP, pxR reveals second rook: +100 -500 +100 = -300? Construct instead a
    // WINNING battery: attackers R+R vs defender N on d5-pawn:
    // Rxd5 (+100), Nxd5 (-500... net -400), Rxd5 (+320) -> swap = max(0 stand)
    // Final: 100 - 500 + 320 = -80 -> SEE = -80? No: defender chooses to stop.
    // Negamax-swap handles this; assert the exact value the swap yields: 0
    // (defender declines recapture when it loses material: 100, then Nxd5
    // gives -400 for us only if we started it... we stop after +100? No -
    // attacker initiated; defender N recaptures only if profitable for THEM:
    // taking the rook (+500-ish for them) is profitable, then our second rook
    // takes the knight (+320). Net: 100 - 500 + 320 = -80.)
    let pos = Position::from_fen("4k3/8/4n3/3p4/8/3R4/3R4/4K3 w - - 0 1").unwrap();
    let mv = find_uci_move(&pos, "d3d5").unwrap();
    assert_eq!(see(&pos, mv), -80);
}
#[test]
fn en_passant_capture_sees_pawn() {
    // ep: captured pawn is NOT on the to-square; undefended -> +100
    let pos = Position::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1").unwrap();
    let mv = find_uci_move(&pos, "e5d6").unwrap();
    assert_eq!(see(&pos, mv), 100);
}
#[test]
fn queen_grabs_poisoned_pawn() {
    // Qxd5 defended by knight: +100 - 900 + ... defender takes: -800 region
    let pos = Position::from_fen("4k3/8/4n3/3p4/8/8/8/3QK3 w - - 0 1").unwrap();
    let mv = find_uci_move(&pos, "d1d5").unwrap();
    assert_eq!(see(&pos, mv), -800);
}
```

- [x] **Step 4.2:** Run `cargo test see` — all fail (module/function missing).
- [x] **Step 4.3: Implement** `src/search/see.rs`:

```rust
//! Static Exchange Evaluation (swap algorithm). Returns the material outcome
//! in centipawns (SEE-local values, decoupled from the tuned eval) of the
//! capture sequence on `mv.to()`, assuming both sides capture with their
//! least valuable attacker and stop when continuing loses material.
//! Approximations (documented, standard at this level): promotions during
//! the exchange are not modeled (the moved piece keeps its value); pins are
//! ignored (a pinned defender still "defends").

use crate::board::attacks;
use crate::board::position::Position;
use crate::core::{Bitboard, Color, Move, PieceType, Square};

const SEE_VALS: [i32; 6] = [100, 320, 330, 500, 900, 20_000];

/// All pieces of BOTH colors attacking `sq` under occupancy `occ`.
fn attackers_to(pos: &Position, sq: Square, occ: Bitboard) -> Bitboard {
    // Pawn attackers: white pawns attack sq if they sit on black-pawn-attack
    // squares from sq, and vice versa.
    (attacks::pawn_attacks(Color::Black, sq) & pos.piece_bb(Color::White, PieceType::Pawn))
        | (attacks::pawn_attacks(Color::White, sq) & pos.piece_bb(Color::Black, PieceType::Pawn))
        | (attacks::knight_attacks(sq)
            & (pos.piece_bb(Color::White, PieceType::Knight)
                | pos.piece_bb(Color::Black, PieceType::Knight)))
        | (attacks::king_attacks(sq)
            & (pos.piece_bb(Color::White, PieceType::King)
                | pos.piece_bb(Color::Black, PieceType::King)))
        | (attacks::bishop_attacks(sq, occ)
            & (pos.piece_bb(Color::White, PieceType::Bishop)
                | pos.piece_bb(Color::Black, PieceType::Bishop)
                | pos.piece_bb(Color::White, PieceType::Queen)
                | pos.piece_bb(Color::Black, PieceType::Queen)))
        | (attacks::rook_attacks(sq, occ)
            & (pos.piece_bb(Color::White, PieceType::Rook)
                | pos.piece_bb(Color::Black, PieceType::Rook)
                | pos.piece_bb(Color::White, PieceType::Queen)
                | pos.piece_bb(Color::Black, PieceType::Queen)))
}

pub fn see(pos: &Position, mv: Move) -> i32 {
    let to = mv.to();
    let from = mv.from();
    let mover = pos
        .piece_on(from)
        .expect("see: no piece on from-square")
        .piece_type();

    // Victim value (ep: the pawn is not on `to`).
    let mut gain = [0i32; 32];
    gain[0] = if mv.is_en_passant() {
        SEE_VALS[PieceType::Pawn.index()]
    } else {
        match pos.piece_on(to) {
            Some(p) => SEE_VALS[p.piece_type().index()],
            None => 0, // quiet move: exchange starts with our piece exposed
        }
    };

    let mut occ = pos.occ_all() ^ from.bb(); // mover leaves its square
    if mv.is_en_passant() {
        // remove the captured pawn (one rank behind `to` from mover's view)
        let cap_sq = Square::from_index((to.index() as i8
            - if pos.stm() == Color::White { 8 } else { -8 })
            as usize);
        occ ^= cap_sq.bb();
    }
    let mut attackers = attackers_to(pos, to, occ) & occ;
    let mut stm = pos.stm().flip();
    let mut victim_val = SEE_VALS[mover.index()]; // next piece to be captured
    let mut depth = 0usize;

    loop {
        let my_attackers = attackers
            & (pos.occ(stm) & occ);
        if my_attackers.is_empty() {
            break;
        }
        // least valuable attacker
        let mut lva_sq = None;
        for pt in [
            PieceType::Pawn,
            PieceType::Knight,
            PieceType::Bishop,
            PieceType::Rook,
            PieceType::Queen,
            PieceType::King,
        ] {
            let s = my_attackers & pos.piece_bb(stm, pt);
            if Bitboard::any(s) {
                lva_sq = Some((s.lsb(), pt));
                break;
            }
        }
        let (sq, pt) = lva_sq.unwrap();
        depth += 1;
        gain[depth] = victim_val - gain[depth - 1];
        // prune: if even capturing for free can't help, both gain entries say stop
        if gain[depth].max(-gain[depth - 1]) < 0 {
            break;
        }
        victim_val = SEE_VALS[pt.index()];
        occ ^= sq.bb();
        // slider x-rays revealed by the departure
        attackers |= attackers_to(pos, to, occ) & occ;
        attackers &= occ;
        stm = stm.flip();
    }
    while depth > 0 {
        gain[depth - 1] = -((-gain[depth - 1]).max(gain[depth]));
        depth -= 1;
    }
    gain[0]
}
```

Adjust to the REAL APIs: `Move::is_en_passant`, `Square::from_index`, `Bitboard::lsb`, `PieceType::index`, `Color::flip`, `attacks::pawn_attacks(color, sq)` — verify each against the codebase and adapt (e.g., if pawn attack tables are `pawn_attacks(sq, color)` or per-color arrays). The UFCS `Bitboard::any(s)` avoids the Iterator::any shadowing footgun. Declare `mod see;` + `pub use see::see;` in `src/search/mod.rs` (or keep crate-internal — match module style).

- [x] **Step 4.4:** `cargo test see` — all pass. Recheck the x-ray test value by hand against your implementation; if your swap yields a different defensible number for the battery case, STOP and re-derive on paper (the test pins the algorithm, not vibes).
- [x] **Step 4.5: qsearch pruning.** In the qsearch move loop, BEFORE `self.pos.make(mv)`:

```rust
// SEE pruning: skip captures that lose material outright (not while in
// check — evasions must all be tried; not promotions — value swing too big
// for the no-promo SEE approximation).
if !in_check && mv.is_capture() && !mv.is_promotion() && see(&self.pos, mv) < 0 {
    continue;
}
```

- [x] **Step 4.6:** Full `cargo test`; `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`; bench ×2 (new number — record); commit `feat(search): SEE + qsearch losing-capture pruning` with `Bench:` line.
- [x] **Step 4.7 (CONTROLLER):** canary (ref 268) → SPRT #1 [0,5] vs `baseline-hybrid` → rows → `tools/baseline.sh see`. H1: +33.0 ±12.1 (bench 57181 = 08cfb32; WAC 271 project high).

### Task 5: Continuation history — SPRT GATE #2 ([0,10])

**Files:** Modify `src/search/mod.rs`.

- [x] **Step 5.1:** StackEntry gains the conthist key of the move made at that ply:

```rust
struct StackEntry {
    static_eval: i32,
    current_move: Move,
    moved_piece: PieceType, // piece that made current_move (conthist key)
    killers: [Move; 2],
    excluded_move: Move,
}
```

Set `self.stack[ply].moved_piece` right where `current_move` is set (both negamax and qsearch; null move stores `PieceType::Pawn` + `Move::NULL` — the NULL move check gates usage). Default `PieceType::Pawn`.

- [x] **Step 5.2:** Tables on `Search` (alongside `history`):

```rust
/// Continuation history: indexed [prev_piece][prev_to][piece][to], one table
/// for 1-ply-ago and one for 2-ply-ago. i16 saturating, depth^2 bonus.
type ContHist = [[[[i16; 64]; 6]; 64]; 6];
// fields:
cont_hist1: Box<ContHist>,
cont_hist2: Box<ContHist>,
```

(`Box::new` via `vec![...].try_into()` or `unsafe`-free zeroed boxing: `let ch: Box<ContHist> = vec![[[[0i16; 64]; 6]; 64]; 6].into_boxed_slice().try_into().unwrap_or_else(|_| unreachable!());` — if that fights the type system, use `Box::new([[[[0; 64]; 6]; 64]; 6])` and accept the stack-then-move; it is 6*64*6*64*2 = 294_912 bytes per table, fine for a one-time init but verify no stack overflow in debug — if debug overflows, the vec route is required.) Clear both at `new()`; do NOT clear between moves (persistence is the feature).

- [x] **Step 5.3:** Ordering: MovePicker::new gains the two conthist scores. Pass precomputed per-parent references — at the negamax call site:

```rust
let ch1 = (ply >= 1 && self.stack[ply - 1].current_move != Move::NULL).then(|| {
    let e = &self.stack[ply - 1];
    (e.moved_piece, e.current_move.to())
});
let ch2 = (ply >= 2 && self.stack[ply - 2].current_move != Move::NULL).then(|| {
    let e = &self.stack[ply - 2];
    (e.moved_piece, e.current_move.to())
});
```

In MovePicker scoring for quiets (replacing the bare butterfly read):

```rust
let mut s = history[stm.index()][mv.from().index()][mv.to().index()];
let piece = pos.piece_on(mv.from()).unwrap().piece_type();
if let Some((pp, pto)) = ch1 {
    s += i32::from(cont_hist1[pp.index()][pto.index()][piece.index()][mv.to().index()]) * 2;
}
if let Some((pp, pto)) = ch2 {
    s += i32::from(cont_hist2[pp.index()][pto.index()][piece.index()][mv.to().index()]);
}
s
```

(Threading the refs through MovePicker::new — extend its signature; qsearch passes `None, None`.)

- [x] **Step 5.4:** Update on quiet beta cutoff (next to the existing butterfly bump):

```rust
let bonus = (depth * depth).min(400) as i16;
let piece = /* piece_type that just moved — capture it before unmake or read from stack[ply].moved_piece */;
if ply >= 1 { /* bump cont_hist1[parent key][piece][to] saturating at ±16000 */ }
if ply >= 2 { /* bump cont_hist2[grandparent key][piece][to] */ }
```

Write the actual saturating-add code (`(*c).saturating_add(bonus).min(16_000)`); ALSO add the symmetric malus (−bonus, floor −16_000) for previously-tried quiets that did not cutoff: collect tried quiets in a small `[Move; 64]` array + count during the loop, apply malus on cutoff. Tests: (a) a cutoff bumps all three tables; (b) ordering prefers a conthist-hot quiet over a cold one (construct directly via the picker like the existing `history_orders_quiets_below_killers` test); (c) malus applied to tried-but-failed quiets.

- [x] **Step 5.5:** Full test/fmt/clippy; bench ×2; commit `feat(search): continuation history (1-ply + 2-ply) with malus` + Bench.
- [x] **Step 5.6 (CONTROLLER):** canary → SPRT #2 [0,10] vs `baseline-see` → rows → `tools/baseline.sh conthist`. H1: +11.3 ±8.4 (bench 54728 = bd81973; WAC 273 project high).

### Task 6: History-driven LMR — SPRT GATE #3 ([0,5])

**Files:** Modify `src/search/mod.rs` (LMR block only).

- [x] **Step 6.1:** Replace the additive ladder with a precomputed log table + history adjustment:

```rust
// once, in Search::new (or a LazyLock-free static built by build.rs? NO —
// just compute in new(), it is 64*64 i32):
// reductions[d][m] = (0.77 + ln(d) * ln(m) / 2.36) as i32, d,m in 1..64
let mut reductions = [[0i32; 64]; 64];
for d in 1..64 {
    for m in 1..64 {
        reductions[d][m] =
            (0.77 + (d as f64).ln() * (m as f64).ln() / 2.36) as i32;
    }
}
```

In the move loop (same guards as today: `!in_check && quiet && depth >= 3 && quiet_count >= 3 && !is_killer`):

```rust
let mut r = self.reductions[(depth.min(63)) as usize][(quiet_count.min(63)) as usize];
// history-driven adjustment: hot moves reduce less, cold reduce more
let hist = /* same combined butterfly+conthist score the picker used; recompute via the helper */;
r -= (hist / 8_000).clamp(-2, 2);
let r = r.clamp(0, depth - 2); // never drop into qsearch from the reduction
```

Factor the combined-history read into a helper `fn quiet_history(&self, ...) -> i32` used by BOTH the picker scoring and this adjustment (single source of truth).

- [x] **Step 6.2:** Tests: reductions table sanity (monotone in both axes; r(3,3)≥1); the killer exemption and check/capture exclusions still hold (existing tests must stay green unchanged — they pin behavior).
- [x] **Step 6.3:** test/fmt/clippy; bench ×2; commit `feat(search): log-formula LMR with history adjustment` + Bench.
- [~] **Step 6.4 (CONTROLLER):** canary → SPRT #3 [0,5] vs `baseline-conthist` → rows → `tools/baseline.sh histlmr`. **H0: −4.2 ±5.1 at 8366 games — REVERTED (033c6b7→506e640). quiet_history refactor retained (eac98bc, bench-identical). Baseline remains conthist.**

### Task 7: Singular extensions + check extensions — SPRT GATE #4 ([0,5])

**Files:** Modify `src/search/mod.rs`.

- [x] **Step 7.1: Check extensions** (small, lands first inside the same commit): in the negamax move loop after `make` succeeds, the child inherits +1 depth when the move gives check:

```rust
let gives_check = self.pos.in_check(self.pos.stm()); // post-make: new stm in check
let ext = i32::from(gives_check);
// every child call in the PVS block uses depth - 1 + ext instead of depth - 1
```

(LMR reduction and extension compose: reduced scout = `depth - 1 + ext - r`.)

- [x] **Step 7.2: Singular extensions.** At the top of negamax after the TT probe (only when ALL hold: `ply > 0`, `depth >= 8`, `self.stack[ply].excluded_move == Move::NULL`, tt_hit with `h.depth >= depth - 3`, bound Lower or Exact, `h.score.abs() < MATE_BOUND`, `h.mv != Move::NULL`):

```rust
let mut singular_ext = 0;
if /* conditions above */ {
    let s_beta = (h.score - 2 * depth).max(-MATE_BOUND + 1);
    self.stack[ply].excluded_move = h.mv;
    let s = self.negamax((depth - 1) / 2, s_beta - 1, s_beta, ply);
    self.stack[ply].excluded_move = Move::NULL;
    if s < s_beta {
        singular_ext = 1; // the TT move is singular: extend it
    }
}
```

Exclusion plumbing (the `excluded_move` field finally earns its keep):
- In the move loop: `if mv == self.stack[ply].excluded_move { continue; }` (before make).
- When `excluded_move != Move::NULL` at node entry: SKIP the TT cutoff (the stored result describes the un-excluded node), SKIP null-move, SKIP RFP, and SKIP the TT store at exit. (Guard each with `let excluded = self.stack[ply].excluded_move != Move::NULL;`.)
- The singular verification search runs at the SAME ply (reuses the stack slot — that is why excluded_move must be reset immediately after).
- The TT move's child depth becomes `depth - 1 + ext + singular_ext` (cap total extension per move at +1: `let ext = (ext + singular_ext).min(1);` — checks on the singular move don't double-extend).

- [x] **Step 7.3:** Tests: (a) mate-in-N suite still exact (existing KRK tests green); (b) a singularity smoke: on a position with one clearly-best TT-primed move (prime via `search_to_depth(8)` then assert the next iteration still returns it and the score is sane — behavioral, not white-box); (c) excluded-move node does not pollute TT: probe a position, run an exclusion search manually if the harness allows, assert the TT entry for the key is unchanged (if not testable without exposing internals, document why and rely on review).
- [x] **Step 7.4:** test/fmt/clippy; bench ×2; commit `feat(search): singular + check extensions` + Bench.
- [~] **Step 7.5 (CONTROLLER):** canary → SPRT #4 [0,5] vs `baseline-histlmr` (run vs `baseline-conthist` after T6 revert) → rows. **H0: −9.0 ±7.0 at 4726 games — REVERTED (dd9143c→228c424). Unconditional check ext doubled the tree; singular-only probe +5.2 ±23.9 (400 games, inside ±10 bar, no SPRT). Baseline remains conthist. Singular queued M7+; check ext gated.**

### Task 8: Futility v2 — the M4 IOU — SPRT GATE #5 ([0,5])

**Files:** Modify `src/search/mod.rs` (futility block).

- [x] **Step 8.1:** Keep the d≤2 pre-make skip exactly as-is. Add the d3-4 POST-make variant (gives_check is free post-make in our legality-by-rollback flow):

```rust
// pre-loop:
let futile_v2 = ply > 0
    && !in_check
    && (3..=4).contains(&depth)
    && alpha.abs() < MATE_BOUND
    && static_eval + 110 * depth + 150 <= alpha;
// in-loop, AFTER make + on_make + legal += 1, before searching:
if futile_v2
    && legal > 1               // never skip the first legal move
    && !mv.is_capture()
    && !mv.is_promotion()
    && !self.pos.in_check(self.pos.stm()) // the move does NOT give check
{
    self.pos.unmake();
    self.eval.on_unmake(mv, &self.pos);
    continue;
}
```

The 2026-06-05 incident comment in the d≤2 block gets a follow-up line: `// d3-4 returns POST-make with a gives_check guard (futility v2, M6 — the c41d9c6 IOU).`

- [x] **Step 8.2:** Tests: existing mate suites green (the gives_check guard must keep all checking continuations); a sanity test that a d≤4 search on a sacrificial WAC position (use WAC.288 fen `r1b2rk1/p4ppp/1p1Qp3/4P2N/1P6/8/P3qPPP/3R1RK1 w - -` bm Nf6+) still finds the checking move at depth 6.
- [x] **Step 8.3:** test/fmt/clippy; bench ×2; commit `feat(search): futility v2 — d<=4 with gives_check guard (c41d9c6 IOU)` + Bench.
- [~] **Step 8.4 (CONTROLLER):** canary 272 (IOU vindicated: gives_check guard held, −1 vs unguarded M4 variant's −11) → SPRT #5 [0,5] vs `baseline-conthist`. **STOPPED true-zero: +1.1 ±8.1, LLR −0.14 at 3565 games — REVERTED (384143f→49fa4d1). d3-4 tier only prunes ~1.7% nodes; the IOU's real lesson (gives_check guard works) is preserved in the revert message. Retry M7+ post-TimeBrain.**

### Task 9: Margin sweep — SPRT GATE #6 ([0,5])

**Files:** Modify `src/search/mod.rs` (margin constants only; one named-consts commit, then candidate builds in /tmp).

- [x] **Step 9.1:** Hoist the magic margins into named consts at the top of mod.rs (NO value changes — bench must be unchanged, verify ×2): `RFP_MARGIN_IMPROVING=60, RFP_MARGIN=80, RFP_MAX_DEPTH=6, FUT_D2_SLOPE=90, FUT_D2_BASE=120, FUT_V2_SLOPE=110, FUT_V2_BASE=150, NULL_R=3, ASP_DELTA=25`. Commit `refactor(search): name the margin constants (bench-identical)` (no Bench line; bench verified unchanged — state it in the body). Note: FUT_V2_SLOPE/BASE not present in the shipped tree (T8 reverted) — only FUT_D2 consts appear.
- [~] **Step 9.2 (CONTROLLER, probe-ranked like the 6.5 pattern):** C2/C3/C4 probes run (3×400 games). **All within ±10 bar: tighter −15.7 ±25.0, looser 0.0 ±23.3, adaptive null-R +2.6 ±23.7. No SPRT run. M4-era values survived the M5 eval transition — margins already calibrated. Closed no-gate.**
- [~] **Step 9.3:** On H1: skip (no winner). On no-winner: no commit needed. Closed.

### Task 10: v0.6.0 wrap

**Files:** Modify `README.md`, `Cargo.toml`.

- [x] **Step 10.1:** Cargo.toml → 0.6.0; README milestone list: tick the M6.0/M6.1 line (add `- [x] M6a: bracketed measurement + search polish (SEE, conthist, hist-LMR, singular, futility v2)` matching list style, keep the M6 bot-readiness line unticked for Plan 7); build; bench ×2 unchanged from last gate; `cargo test` green; commit `chore: M6a wrap — bump to 0.6.0` (no Bench line).
- [x] **Step 10.2 (CONTROLLER):** anchored gauntlet on the mixed pool (300/rung) for the 0.6.0 binary → strength-log row (2811.4 ±15.9, +37.7 vs 0.5.0, 57.3% overall); WAC trend row (273/299, +5 over 0.5.0 ship).
- [x] **Step 10.3:** `git tag -a v0.6.0 -m "M6a: search polish; anchored 2811±16"`; push main + tags; CI green.
- [x] **Step 10.4:** Report delivered. M6a closes: SEE+conthist banked (+44 self-play → +38 anchored); T6/T7/T8 honestly H0'd; T9 calibrated-no-gate. Target 2900: 89 elo to go.

---

## Plan self-review notes

- **Spec coverage:** M6.0 ledger restructure ✓(T1), README/target ✓(T1,T3), mixed pool ✓(T2), bracketed re-measure ✓(T3), target decision ✓(T3). M6.1 SEE ✓(T4), conthist ✓(T5), hist-LMR ✓(T6), singular+check ✓(T7), futility v2 IOU ✓(T8), margin sweep last ✓(T9). v0.6.0 ship ✓(T10). No eval retunes anywhere (spec invariant).
- **Type consistency:** `see(&Position, Move) -> i32` used in T4 qsearch and T8 references; `quiet_history` helper introduced T6 and reused; StackEntry.moved_piece introduced T5, consumed T5/T6; excluded_move consumed T7 (existed since M3); margin consts named T9 step 1 and swept step 2.
- **Known approximations (documented in code):** SEE ignores pins and in-exchange promotions; conthist tables are bonus/malus saturating i16; singular verification at same-ply reuses the stack slot.
- **API-verification duty:** T4's code block names (`is_en_passant`, `lsb`, `pawn_attacks` argument order, `Square::from_index`) MUST be verified against the actual core types before use — the plan pins semantics, the implementer pins syntax.
- **Wall-clock:** 2 gauntlets (~2h each) + 5-6 SPRTs + canaries + probes ≈ 1.5-2 days of compute.
