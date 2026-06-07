# v0.5.0 Lichess field analysis — draw classification + sacrifice-entrance suite

**Plan 7, Task 1.** Corpus: `db/lichess_nebchessbot_0.5.0.pgn` — 38 games,
17W–14D–7L (63.2%) vs a 2000–2500+ bot field, TCs 180+2 / 300+3.

**Method.** Each game is replayed through the engine library
(`Position` + `find_san_move`) by `examples/pgn_replay.rs`, which also reports
the terminal mechanism straight from the library (`is_repetition`,
`is_fifty_move_draw`, an insufficient-material check). The driver
`tools/analyze-field.py` runs `./target/release/nebchess` over a single
persistent UCI pipe (`position fen … / go movetime …`, reading the last
`info … score …` before `bestmove`). The root score is side-to-move-relative
(negamax), and every probed position is NebChess-to-move, so the reported `cp`
is already **NebChess-relative** (positive = good for NebChess). Mate scores
are kept as signed mate distances.

For each of the 14 draws, every NebChess-to-move position from move 20 on was
searched for 2 s (1020 positions total). The headline metric is the **maximum
eval NebChess held in the final 30 plies** (the spec's leak window).

---

## Step 1.2 — Draw classification

Classification rule: **LEAKED** = held ≥ +200 cp (or a mate score) within the
final 30 plies and drew anyway; **FAIR** = never better than +100 cp; the
100–200 cp gray band is a documented judgment call.

| Game | NebColor | Opponent (Elo)        | Max eval (final 30) | Terminal mechanism | Class  | Note |
|-----:|:--------:|:----------------------|:-------------------:|:-------------------|:-------|:-----|
| 2    | W        | ratsu-bot (2435)      | −199                | insufficient (KN-v-K) | FAIR  | Never ahead; ended worse, K+N-v-K shuffle. |
| 3    | B        | halcyonbot (2216)     | **+422**            | insufficient (KB-v-K) | **LEAKED** | Eval blindspot — see §1.3. Held +4.2 into a *dead-drawn* K+B-v-K. |
| 4    | W        | SoloBot (2537)        | +44 (peak +179)     | 50-move            | FAIR   | Peaked +179 pre-final-30, dissipated through normal play; balanced ending. |
| 8    | B        | bot_adario (2518)     | +1                  | repetition         | FAIR   | Dead-equal Q+R-v-Q+R; repetition. |
| 12   | B        | ratsu-bot (2435)      | +18                 | 50-move            | FAIR   | 596-ply grind from near-equality; 50-move in K+B+P-v-K+B. |
| 14   | W        | ImranMelikovBot (2196)| **+462**            | repetition         | **LEAKED (perpetual/no-progress)** | R+P-v-R held +4.6 for 14 moves, shuffled into threefold — see §1.3. |
| 16   | W        | simbelmyne-bot (2599) | +37                 | repetition         | FAIR   | Roughly level minor-piece ending. |
| 18   | W        | bot_adario (2518)     | +13                 | repetition         | FAIR   | Equal middlegame, Rc1–c2–c1 repetition. |
| 19   | B        | PeachFruit (2396)     | **+425**            | insufficient (KB-v-K) | **LEAKED** | Same eval blindspot as g3 — held +4.25 into K+B-v-K. |
| 29   | W        | ImranMelikovBot (2196)| +12 (peak +57)      | insufficient       | FAIR   | Never decisively ahead. |
| 33   | W        | cutecassia (2336)     | +121                | repetition         | GRAY → FAIR | Held +1.2 only transiently (move 32); simplified to a near-equal fortress, forced repetition. |
| 34   | W        | Casanchess-NNUE (2379)| **+237**            | repetition         | **LEAKED (perpetual)** | Held +4.8 (move 25), drifted to +0.01 and shuffled — see §1.3. |
| 35   | B        | s3gfau1t (2243)       | **#21 (mate)**      | 50-move            | **LEAKED** | **Forced mate up a rook (R-v-K), drawn on the 50-move counter while blitzing at depth 1** — see §1.3. |
| 38   | B        | bot_adario (2521)     | +5                  | repetition         | FAIR   | Equal rook-and-minor ending. |

### Summary

- **LEAKED: 5** (g3, g14, g19, g34, g35) · **GRAY→FAIR: 1** (g33) · **FAIR: 8**.
- **Leaked half-points: 2.5** (five drawn games that were objectively won —
  worth a full point each — scored ½; 5 × 0.5 = 2.5 points left on the table).
  Counting the g33 gray as leaked would make it 3.0.
- **Terminal-mechanism breakdown of all 14 draws** (from the library):
  repetition 7 (g8, g14, g16, g18, g33, g34, g38), 50-move 3 (g4, g12, g35),
  insufficient-material 4 (g2, g3, g19, g29). (Lichess tags every game
  `Termination "Normal"`; the mechanism here is computed, not parsed.)
- **Of the 5 leaks, the terminal mechanisms split: 2 perpetual/repetition
  (g14, g34), 2 insufficient-material (g3, g19), 1 fifty-move (g35).** The
  perpetual-check pattern the spec anticipated as the *main* draw pattern is
  real but is only 2 of 5 leaks; the other 3 are endgame-conversion failures
  (insufficient-material eval blindspot ×2, R-v-K 50-move clock collapse ×1).

---

## Step 1.3 — Leak moments + depth-vs-knowledge verdict

For each leaked game, the table gives the **leak-moment EPD** (the peak
NebChess-to-move position just before the eval slid toward the draw — where the
winning continuation still existed), the engine's preferred move at 2 s vs the
move actually played, and a **10 s recheck** at that position. The recheck is
the discriminator: if depth keeps (or finds) the win, the leak is a **time/depth
leak (TimeBrain-fixable)**; if depth confirms the misjudgment, it is a
**knowledge leak** (eval / tablebase work, M7+).

### LEAKED-perpetual (step 1.3's explicit scope)

**g34 — vs Casanchess-NNUE — TimeBrain-fixable.**
Leak at move 25 (held +481): played **g1h1** (passive king tuck), eval dropped
241 cp; engine preferred **g1f2**. 10 s recheck: **+472, best still g1f2,
depth 17** — depth does not budge the eval, the engine knows the winning move.
The game move was a blitz-time error; with time the engine plays g1f2.
EPD: `2r3r1/kpB4p/p4p1Q/1q1N4/n3P3/7P/P4bP1/2RR2K1 w - - 0 25 bm Kxf2; id "LEAK.g34";`
(engine-preferred g1f2 = Kxf2; played g1h1).

**g14 — vs ImranMelikovBot — TimeBrain-fixable (no-progress / conversion).**
No single ≥150 cp drop: NebChess held **+462 for 14 consecutive moves**
(moves 68–81) in a won R+P-v-R, and on every move the engine's best was a
*progress* move while the game played a *shuffle*, ending in threefold.
First clear divergence (move 46, +435): engine best **f3e3**, played **g3g4**.
Peak/representative EPD (move 47, +486):
`R7/P6k/8/8/r5p1/5K1P/8/8 w - - 0 47 bm hxg4; id "LEAK.g14";`. 10 s recheck:
**+469, best h3g4 = hxg4, depth 35** — the eval is rock-stable; the engine evaluates
the win correctly but, under blitz, never searched deep enough to choose the
converting line over a safe shuffle. (Caveat: the engine's *own* best moves in
this position are also slow king maneuvers; some endgame-technique weakness
rides alongside the clock factor, but depth — i.e. time — is the dominant lever:
nothing in the eval is *wrong*.)

**g33 — vs cutecassia — borderline (GRAY), TimeBrain-adjacent.**
Held only +139 (move 32) and slid to a forced repetition from a near-equal
fortress. 10 s recheck at the peak: **+134, best e3d2, depth 16** — eval stable
but modest. Classified GRAY→FAIR (transient edge, not a clearly won game); the
repetition is closer to a fair result than a squandered win.

### Worst-3 verdict (the three largest squandered advantages)

The three worst leaks by held magnitude are **g35 (forced mate), g14 (+486),
and g3/g19 (+425/+422)**. Their verdicts split cleanly:

**g35 — vs s3gfau1t — TimeBrain + Syzygy (clock collapse).**
NebChess (Black) reached a **forced mate** (eval #21–#26, R-v-K) and **drew by
the 50-move rule**. The move-by-move depths tell the story: moves 106–116 were
searched at **depth 1–12** (clock collapsed), playing mate-distance-*increasing*
shuffles (e.g. best `d6d4` #26 vs played `d6c6`), burning the 50-move counter.
By move 102 the position (halfmove 67/100) is still a clean #11 at depth 14, but
the counter was already spent. 10 s recheck at move 102:
`8/8/8/4k3/3r4/8/2K5/8 b - - 67 102 bm Rd5; id "LEAK.g35";` → **#11, depth 14**.
The engine *finds* the mate with time; the loss was the depth-1 blitz phase
exhausting the 50-move rule. **Primarily TimeBrain-fixable; Syzygy (KRvK DTZ)
would also convert directly.**

**g3 & g19 — vs halcyonbot / PeachFruit — KNOWLEDGE GAP (eval blindspot).**
Both games NebChess held ≈ **+4.2** and drew by **insufficient material**, both
ending **K+B vs K** — a dead draw. One ply before the end NebChess (with a
passed pawn one step from queening and a bishop) evaluates the position +4.2;
the opponent gives up the bishop for the pawn (or the pawn falls), leaving a
bare bishop. 10 s rechecks **increase** confidence in the wrong eval:
- g3, move 96: `8/6P1/8/4b3/5k2/8/8/4K3 b - - 0 96` → +427 at depth 38.
- g19, move 102: `8/8/6P1/5k1b/8/6K1/8/8 b - - 0 102` → +427 at depth 39.
Depth does **not** fix these — they are an **insufficient-material / wrong-bishop
eval blindspot**. **Not TimeBrain. Syzygy 3-4-5 (KBvK = draw) resolves them
directly**; an eval-side insufficient-material clamp (M7) is the non-TB fix.

### Verdict tally

| Game | Leak class | Verdict |
|-----:|:-----------|:--------|
| g34 | perpetual | **TimeBrain-fixable** (engine knows g1f2 with time) |
| g14 | perpetual / no-progress | **TimeBrain-fixable** (eval stable, blitz conversion failure; minor technique factor) |
| g35 | 50-move from a forced mate | **TimeBrain-fixable + Syzygy** (depth-1 collapse spent the counter) |
| g3  | insufficient-material | **Knowledge gap** (eval blindspot; Syzygy / M7 eval) |
| g19 | insufficient-material | **Knowledge gap** (eval blindspot; Syzygy / M7 eval) |

**Three of five leaks are time/depth (TimeBrain-fixable), corroborating the
spec's "clock collapse is the dominant leak" hypothesis. Two are a genuine
knowledge gap (K+B-v-K over-valuation) that TimeBrain cannot touch — Syzygy
3-4-5 fixes both directly.**

---

## Step 1.4 — Sacrifice-entrance suite

`tools/suites/sac-entrance.epd` — **11 lines** (INFORMATIONAL; not a gate).

**Method.** Over every NebChess-to-move position in the losses + draws (moves
8–50), the engine's screening-move (1.5 s) was compared to the move played; when
they differed, the engine move was classified by a **full SEE swap** ported into
`pgn_replay.rs` (`see_swing`, bit-identical to the engine's private `see` — a
sacrifice is SEE ≤ −90) and the candidate confirmed at 10 s. A move counts as a
sacrifice entrance when the engine's 10 s choice gives up material by SEE, keeps
a favorable eval, and the game move declined it.

**Corpus yield: 3 genuine sacrifice entrances** (SAC.001 g7, SAC.002 g23,
SAC.003 g29). An earlier 2-ply-material heuristic flagged 11 "candidates", but a
true SEE pass rejected 8 as false positives (rook lifts to defended-but-safe
squares, queen *trades* mis-read as sacs, already-winning technical moves). The
corpus is simply thin on declined-sacrifice losses — NebChess's draws/losses are
dominated by clock-driven drift and endgame-conversion failure, **not** missed
tactical entrances. (The spec's user-reported Greek-gift miss is not cleanly
reproducible at the scanned NebChess-to-move positions in these 38 games; it is
represented in the suite by the WAC Greek-gift line, SAC.007.)

**Padding: 7 WAC sacrifice-motif lines NebChess misses at 2 s** (per the spec's
explicit fallback). Selected from the 26 SEE ≤ −200 WAC positions the engine
fails at 2 s, prioritizing king-attack / Greek-gift motifs and cases where the
engine's non-sac eval is poor (SAC.008 Qxg7+ engine −256; SAC.009 Rxh7+ engine
−654). Provenance is noted per line in the `c0` comment.

**Suite composition (provenance split): 3 corpus + 7 WAC + 1 WAC positive
control** (SAC.000 = WAC.001, which the engine *solves* — a sanity anchor).

| id | provenance | bm | motif | engine @2s |
|:---|:-----------|:---|:------|:-----------|
| SAC.000 | WAC.001 (control) | Qg6 | queen sac / mate | **solves** |
| SAC.001 | corpus g7 m32     | h5  | pawn-break sac | solves (game missed under clock) |
| SAC.002 | corpus g23 m16    | e4  | central pawn sac | misses (finds at 10s) |
| SAC.003 | corpus g29 m18    | Nxd5| piece-for-pawn | solves (game declined) |
| SAC.004 | WAC.002           | Rxb2| exchange sac | misses |
| SAC.005 | WAC.055           | Qxg7+ | queen sac | misses |
| SAC.006 | WAC.163           | Qg2+ | queen sac / mate | misses |
| SAC.007 | WAC.185           | Qxh7+ | **Greek gift** | misses |
| SAC.008 | WAC.207           | Qxg7+ | queen sac | misses (−256) |
| SAC.009 | WAC.213           | Rxh7+ | rook sac | misses (−654) |
| SAC.010 | WAC.297           | Bxg2 / Bxh2+ | bishop sac | misses |

Current engine solve rate on the suite: **3/11 at 2 s** (a depth probe for the
M7+ gated-check-extension work).

---

## Reproducing

```sh
cargo build --release --example pgn_replay     # SAN replay + SEE annotate helper
python3 tools/analyze-field.py draws           # step 1.1/1.2 (~34 min, 1020 positions @2s)
python3 tools/analyze-field.py leaks           # step 1.3 (leak moments + 10s recheck)
python3 tools/analyze-field.py sacs            # step 1.4 screening (~45 min) + 10s confirm
./target/release/solve tools/suites/sac-entrance.epd 2000   # validate suite + solve rate
```

Caches land in `tools/data/field-050/` (gitignored). The driver spawns the
engine ONCE per phase and reuses it over a persistent UCI pipe (project law);
the system must be otherwise idle (movetime results are timing-sensitive).
