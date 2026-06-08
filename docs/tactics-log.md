# Tactics Log

WAC (300 positions; 299 scored — WAC.274 has a malformed EPD line) at
1000ms/position, single thread, default Hash.
Informational regression canary — self-play SPRT shares blind spots
between both engines; this metric does not. A drop >= 10 positions vs
the previous entry is a stop-and-investigate signal.

**M5 canary trend (1s/position):** 267 (T1 tapered) → 261 (T2 pawns, K-incident recovered) → 262 (T3 mobility) → 257 (T4 KS, NPS tax) → 260 (fused, attribution probe) → 267 (T5 threats) → 268 (hybrid ship). Two genuine catches this milestone: the K-refit scale degeneracy (fired at the −10 floor, attributed, frozen) and the "tactically sharper ≠ stronger" big3 dissociation (269 canary / ±0 SPRT — caught by the gate pairing, not the canary alone). The floor discipline held: no build shipped below reference −10.

| date | binary | WAC | notes |
|------|--------|-----|-------|
| 2026-06-05 | 0.3.x @ 38947c3 (post-aspiration) | 267/299 | 1 position skipped (WAC.274 bad fen); spec URL 404 — jdart1/arasan-chess mirror used |
| 2026-06-05 | futility d<=4 + RFP (pre-fix T5) | 257/299 | **CANARY FIRED (−11 vs paired 268 rerun)**: worktree diff = 14 broken / 3 rescued; A/B attribution: futility d3-4 quiet-skips break sacrificial combinations; RFP innocent (269 with futility off) |
| 2026-06-05 | futility d<=2 + RFP @ c41d9c6 | 266/299 | fix verified — kept ~97% of node savings (bench 96150→97369); deeper futility returns with gives_check/SEE margins (M6) |
| 2026-06-05 | qsearch TT @ 8554de1 | 266/299 | clean — cache, not a prune |
| 2026-06-05 | Texel-tuned PST+material @ 78d2fc6 | 258/299 | −8 (below the 10 floor): tuned Q=1049 likely dims sacrifice appetite; SPRT is the arbiter — if H0, tables revert per gate semantics |
| 2026-06-06 | Tapered mg/eg + retune @ 1d0bdfe | 267/299 | **+9 — recovers the flat-tune dip** (queen mg/eg split frees middlegame sacrifice judgment); project high |
| 2026-06-06 | pawn structure @ 82eab57 (degenerate K) | 257/299 | **CANARY FIRED (−10; paired rerun 258 vs 267)**: per-run K refit (1.520→1.377) slid the params down the K·eval degeneracy — pieces inflated ×1.09 vs fixed-cp margins + P_mg anchor; 11 broken / 2 rescued, breaks all sacrifice motifs (Rxg2+, Nf7, Qe6+, Rxh2+, Nf6+, Qh8+); NPS exonerated (−3.6%) |
| 2026-06-06 | pawn structure, K frozen 1.520 @ c58d05d | 261/299 | fix verified — identical MSE at the validated scale (degeneracy bought zero fit); recovers the 6-position sacrifice cluster; −6 residual is pawn-term tax + threshold flips (5 new misses the *worse* build solved), within floor; SPRT arbitrates |
| 2026-06-06 | safe mobility @ 33c9b7a | 262/299 | +1 over pawns build — mobility knowledge offsets the slider-attack NPS cost; trapped-piece cells deeply negative (N: −75eg at 0 mobility) |
| 2026-06-06 | king safety @ 689f1cf | 257/299 | −5, within floor, but plan expected improvement — diff is CHURN (12 broken / 7 rescued, incl. 167/210/290 king-attack rescues), not motif blindness; NPS −13% (KS recomputes slider attacks mobility already computed) pushes deep-sac finds past 1000ms; **T5 must add a shared attack-table pass** (mobility+KS+threats compute attacks once); SPRT arbitrates |
| 2026-06-06 | *(infrastructure)* fused attack pass @ 9b6adfa | 260/299 | **attribution confirmed**: eval bit-identical to 689f1cf (257), +3 from NPS alone (recovers WAC.131/200/265/291 deep-sac finds; −1 threshold churn) — the T4 dip was time-tax, not eval-shaping |
| 2026-06-06 | threats/coordination/tempo @ 021646d | 267/299 | **+7 — ties project high**: hanging/threat terms are tactical primitives (recovers king-attack + sacrifice motifs); val MSE −4.0%, biggest single-term drop since tapering |
| 2026-06-06 | full joint tune on big3 7.15M @ b030465 | 269/299 | **+2 — NEW PROJECT HIGH**: review flagged deflated material vs fixed-cp margins as the risk (rook/queen sac motifs) — did not materialize; the bigger corpus sharpened tactical judgment |
| 2026-06-06 | hybrid zurichess+big3 tune @ HEAD | 268/299 | top of band — half-distortion costs nothing tactically; hybrid ships after H1 |
| 2026-06-07 | SEE + qsearch pruning @ 08cfb32 | 271/299 | **+3 — NEW PROJECT HIGH**: ~26% node cut buys depth; sound sacs unaffected (checks bypass the SEE gate); WAC.288 verified in review |
| 2026-06-07 | continuation history @ bd81973 | 273/299 | **+2 — project high again**: ordering gains compound with SEE depth; note: tables persist per-go only (SearchThread recreated per search — cross-move persistence is the Plan 7 conthist refactor) |
| 2026-06-07 | log-formula LMR + history adj @ 033c6b7 | 270/299 | −3, within floor — deeper cold-quiet reductions trade threshold tactics for tree size; SPRT arbitrates |
| 2026-06-07 | singular + check extensions @ dd9143c | 276/299 | **+3 — PROJECT HIGH**: forcing-line extensions are pure WAC fuel; prediction verified (unlike the M5 king-safety expectation) |
| 2026-06-07 | futility v2 (d<=4 + gives_check) @ 384143f | 272/299 | **THE IOU PAYS: −1 vs the unguarded M4 variant's −11** — the gives_check guard was the missing piece exactly as the incident attribution said; watch gate passed |
| 2026-06-07 | **v0.6.0 ship state** @ 650bf84 | 273/299 | M6a trend: 268→271 (SEE)→273 (conthist)→276 (ext, reverted)→272 (fut-v2, reverted)→**273 ship** (+5 over the 0.5.0 ship state; two all-time-high features reverted on SPRT evidence — the suite measures tactics, the gates measure chess) |
| 2026-06-07 | TimeBrain gate 1 @ ba544b8 | 272/299 | ±1 expected (TM is inert at fixed movetime — canary just confirms no collateral search damage); the real test is SPRT at 8+0.08 |
| 2026-06-07 | TimeBrain gate 2 + movetime-exact fix @ 9f954ab | 272/299 | **no regression** — back-to-back A/B vs the pre-TimeBrain build (same load window) both solve 272; the earlier 266 readings were NPS jitter on a busier box (the "273" reference was itself a high reading). Bench-identical tree confirmed by measurement, not assumption |
| 2026-06-07 | insufficient-material + Syzygy (M6.3 T5) @ ea8a477 | 272/299 | unchanged — insufficient-material never fires on WAC midgame; Syzygy inert (TB off in solve); both fixes are endgame/correctness, invisible to a midgame tactics suite |
| 2026-06-08 | persistent histories + pondering (M6.3 T7) @ feb24aa | 272/299 | unchanged — persistence warm-start is cross-MOVE (solve runs per-position, no bleed); pondering inert in solve; both verified by review not canary |
