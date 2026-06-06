# Tactics Log

WAC (300 positions; 299 scored — WAC.274 has a malformed EPD line) at
1000ms/position, single thread, default Hash.
Informational regression canary — self-play SPRT shares blind spots
between both engines; this metric does not. A drop >= 10 positions vs
the previous entry is a stop-and-investigate signal.

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
