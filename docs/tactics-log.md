# Tactics Log

WAC (300 positions) at 1000ms/position, single thread, default Hash.
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
