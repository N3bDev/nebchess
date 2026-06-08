# v0.7.1 Lichess field analysis — the clock-starvation pattern

**M7 input (field data, no plan/gate yet).** Corpus: `db/nebchessbot0.7.1games.pgn`
(batch 1, 3 games) + `db/nebchessbot0.7.1games2.pgn` (batch 2, 5 games) — 8
rated games, **both colors**, `180+2` and `300+3`, vs a 1736–2642 BOT field.
Combined result **2W–3D–3L**. The PGNs carry per-move `[%clk]` (lichess) **and**
NebChess's own depth-tagged `[%eval cp,depth]`, so both the clock and the
engine's self-assessment are recoverable move-by-move.

This is the asymmetric-clock weakness the prior turn predicted: **self-play SPRT
structurally cannot see it** (both sides share one TimeManager, so they starve in
lockstep). Against a time-efficient opponent it is decisive — and batch 2 shows
the worst case directly: cutecassia/vala-bot played **near-instant** (banking
their entire clock — vala-bot *finished* on 3:53 of a 3:00 base) while NebChess
burned its clock down to the increment by move ~30.

---

## Batch 1 — the one pattern, three games (all Black, vs 2549–2642)

NebChess spends its base clock ~2× too fast and plays the entire critical phase
of every game on the +2 s increment, while the opponent sits on minutes.

| Game | Opp (Elo) | Result | NebChess clock trajectory (Black) | Eval (NebChess-relative) | Loss/draw mechanism |
|-----:|:----------|:------:|:----------------------------------|:-------------------------|:--------------------|
| [vIPivhTo](https://lichess.org/vIPivhTo) | prokopakop (2549) | ½–½ | m9 **3:06** → m37 **0:39** → m43 **0:15** | +2.04 @ m37 (d18) → **−0.01 @ m43 (d24)** | Held a +2 mirage at d18; at d24 it's a drawn Q-ending. Reached the conversion on 15–39 s vs White's 1:54 → perpetual. |
| [6YQXEsod](https://lichess.org/6YQXEsod) | SleepMindEngine (2577) | 0–1 | m8 **3:13** → m27 **0:42** → m33 **0:20** → m40 **0:13** | ≤ +0.5 (White) through m32, then 0.74 → 1.36 → 2.41 → mate | Held ≈level until ~m32; once under 20 s it leaked ~½ pawn every few moves. Slow death, mated m74. |
| [57vN9BIm](https://lichess.org/57vN9BIm) | SykoraBot (2642) | 0–1 | m9 **2:09** → m31 **0:31** → m35 **0:15** → m37 **0:11** | ~0 to +0.4 through m45, then slow climb to lost | A ≈drawn R+N endgame held for 30 moves, then lost over **60 moves played on the 2 s increment**. Mated m98. |

In all three the eval stayed flat (held the position) *while the clock had time*,
then deteriorated *monotonically once on the increment*. The losses are not
tactical blunders or eval blindspots — they are **time-pressure slow declines**.

---

## Batch 2 — same pattern, both colors, wider field (2W–2D–1L)

| Game | Opp (Elo) | Col | Result | Clock story | Read |
|-----:|:----------|:---:|:------:|:------------|:-----|
| [OTHY7Wgu](https://lichess.org/OTHY7Wgu) | cutecassia (2353) | B | ½ | NebChess 3:13→**0:30 by m30**; cutecassia sat on **~2:50 all game** | Held a 145-move R+B-v-R *theoretical draw* on the increment. Drew **only because it was a dead fortress** — same over-spend, survivable position. |
| [z8aKfCJK](https://lichess.org/z8aKfCJK) | MateMakingMachine (1736) | B | **1–0 (win)** | comfortable | Opponent hung material in the opening; NebChess converted, mated m27. Big margin → clock irrelevant. |
| [pRoB5f8T](https://lichess.org/pRoB5f8T) | cutecassia (2354) | W | ½ | NebChess 5:00→**0:46 by m45**; cutecassia again **~4:45 all game** | Slight edge (+0.5) dissolved; Q+R repetition fortress. Couldn't make progress on the increment vs an instant-move defender. |
| [aQ7iCosA](https://lichess.org/aQ7iCosA) | vala-bot (2346) | W | **0–1 (loss)** | NebChess 3:08→**0:57 by m34**→0:09; vala-bot **gained** time, finished on **3:53** | **The clean indictment.** Slightly *better* (+0.4) as White vs a *lower*-rated bot; drifted to lost in a N-vs-passed-a-pawn endgame played entirely on 0:09–0:55. −7 rating. Not "outrated." |
| [nL9lv4Mw](https://lichess.org/nL9lv4Mw) | CCI-5 (**2625**) | B | **1–0 (win)** | comfortable | CCI-5 played the unsound `5.Nxf7`; NebChess won the piece and **mated a 2625** on m34. |

Two things batch 2 adds that batch 1 could not:
- **The over-spend is color-symmetric and not rating-relative.** It loses as White
  to a 2346 the same way it loses as Black to a 2577.
- **The search/eval is genuinely strong when the margin is real** — it beat a
  2625 and a 1736 convincingly. This is *not* a tactical/strength problem; it is a
  clock + endgame-conversion problem. The fix is time, not playing strength.
- **The dangerous opponents bank their whole clock.** cutecassia and vala-bot
  played near-instant; NebChess's over-spend handed them a decisive *practical*
  edge in long endgames even when NebChess was objectively equal or better.

---

## Root cause (quantified, not inferred)

TimeBrain v1 base soft (`src/search/limits.rs:74-83`), Black at 180+2, ~3:06 left,
overhead 100 ms:

```
avail   = 186000 − 100            = 185900
reserve = (avail/16).clamp(50,2000) = 2000
usable  = 183900
mtg     = movestogo.unwrap_or(30).clamp(1,40) = 30   ← Lichess increment games send NO movestogo
soft    = usable/mtg + binc·3/4   = 183900/30 + 1500 = 6130 + 1500 ≈ 7.6 s
hard    = (soft·5).min(usable/3)  ≈ 38 s
```

**The divisor 30 is the bug for blitz.** It assumes ~30 moves remain *on every
move*; these games went 57 / 74 / 98. soft is a fixed fraction of the *current*
clock, so it tapers — but it front-loads ~7.6 s/move through the first ~15 moves
and drains the bank before the position even sharpens. Observed gross spend
matches: g2 burned 3:13→1:57 over moves 9–16 (≈13 s/move incl. 140 % instability
extensions); g3 ≈8.4 s/move over moves 9–14. A sustainable 180+2 rate for a
50–90-move game is **~3–4 s/move**, leaning on the 2 s increment as the floor.

There is also **no endgame/low-time gear** beyond the flat 2 s emergency reserve:
in g3, moves 38–98 (60 moves of a technical R-ending) were each played on ≈2 s.
TimeBrain v1 never *flags* — it is forfeit-clean, that part works — but it arrives
at the part of the game that decides the result with no thinking time left.

---

## Secondary findings (lower priority than the clock)

- **Eval optimism at moderate depth (g1).** +2.04 at d18 collapsed to −0.01 by
  d24 — the engine over-valued a Q-ending it could not actually win. Even with
  infinite time the +2 was a mirage. Candidate for M7 eval round 2.
- **Endgame technique under no time (batch 2 g4).** The vala-bot loss was a
  knight-vs-far-passed-rook-pawn ending — hard to hold, and impossible on 0:09–0:55.
  TimeBrain-v2 (giving it time to think there) is the high-leverage fix; an
  endgame-eval pass is a secondary candidate. Syzygy only covers ≤5 men; these
  decisive endings are 6–12 men.
- **Repertoire shape.** Petrov-as-Black (b1) and the Trompowsky-with-3.Bxf6 as
  White (b2 g4 — concedes the bishop pair, doubled f-pawns) both steer into long,
  quiet maneuvering games that play *to* the time weakness. A more forcing
  repertoire is a possible lever, but secondary to the TM fix.
- **NOT a raw strength gap.** Batch 1 (all losses to 2549–2642) looked like being
  outrated — but batch 2 falsifies that: NebChess **beat a 2625** convincingly and
  **lost to a 2346**. The discriminator is the clock, not the opponent's rating.
  The engine is strong when it has time or a real margin; it bleeds exactly where
  it runs out of clock.

---

## Peer cross-check: cutecassia's source (2355 from-scratch NNUE engine)

cutecassia is open-source (`github.com/taracutie/cassia`) — a zero-dependency,
from-scratch **NNUE** engine + Polyglot book + tablebases, by one person; a true
peer. Reading its `src/time.rs` **corrects an overclaim above**: its base
allocation is *identical* to ours — `available/movestogo.unwrap_or(30) + inc·¾`.
**The divisor 30 is conventional, not "the bug."** The real differentiators are:

- **Hard cap.** cutecassia caps a single move at **1.5× soft**; ours is **5× soft**
  (→ up to ~38 s on one move at 180+2 vs cutecassia's ~11 s). This is the clearest
  transferable win — a wide hard cap is how one complex middlegame move drains the
  clock. Modern engines sit ~2×.
- **Richer soft scaling.** cutecassia multiplies *three* orthogonal signals —
  best-move stability **× score volatility** (|Δscore| between iterations) **× node
  effort** (best-move's node fraction) — clamped to [0.5, 2.5]×. Ours uses only
  stability (4 buckets, floor 0.6×). Score-volatility is distinct from the
  absolute-score logic we reverted (Gate 2) and is worth trying; node-effort is the
  standard Stockfish signal.
- **Not just TM.** Much of cutecassia's "no time used" in these games is **book
  (opening) + tablebase (≤5-man endgames) playing instantly**, plus being
  NNUE-strong enough to *hold the draw without thinking*. TM is only part of it.

Takeaway: the TM fixes below recover Elo we *leak at current strength*; they do not
raise the ceiling. The ceiling-raiser (and what makes cutecassia a peer and Patricia
a 3000) is **NNUE** — a separate, larger milestone, not part of TimeBrain-v2.

---

## How to move forward

1. **M7 #1 = TimeBrain-v2, the spending profile.** Two levers, both SPRT-gated and
   validated off self-play: (a) **tighten the hard cap** from 5× soft toward ~2×
   (the single clearest drain); (b) **add score-volatility + node-effort to the
   soft scaling** (cutecassia-style, retuned for our search — copy the *signals*,
   not the constants). Keep the divisor; it is not the problem. Add a low-time /
   endgame gear so a 60-move technical phase isn't all on the increment. Quantified
   success metric: median NebChess clock at move 40 ≥ 45 s at 180+2.
2. **Validate OFF self-play.** Same-TM self-play cannot measure this. Use a
   **TimeBrain-v2 vs TimeBrain-v1 head-to-head at real 180+2** (identical
   search/eval, only the TM differs → isolates the spending profile), plus an
   anchored blitz gauntlet at 180+2 for absolute score. The 8+0.08 SPRT remains
   the search/eval arbiter but is blind to this axis.
3. Eval round 2 (the g1 +2 mirage) and a Black-repertoire review follow the TM
   fix, gated normally.

> Caveat: n = 8 (3 + 5), both colors, 1736–2625 field, two TCs. The *direction*
> is unambiguous, color-symmetric, and mechanism-grounded; the magnitude of the
> fix should be tuned against more field games as they accrue. Note the one loss
> to a *lower*-rated bot (vala-bot 2346) and both draws to cutecassia (~2353) are
> the soft rating drain — winnable/holdable positions surrendered on the clock —
> while the two wins (vs 1736 and **2625**) confirm the engine is strong when it
> has either time or a real margin.
