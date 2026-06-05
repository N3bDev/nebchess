# Tactics Log

WAC (300 positions) at 1000ms/position, single thread, default Hash.
Informational regression canary — self-play SPRT shares blind spots
between both engines; this metric does not. A drop >= 10 positions vs
the previous entry is a stop-and-investigate signal.

| date | binary | WAC | notes |
|------|--------|-----|-------|
| 2026-06-05 | 0.3.x @ 38947c3 (post-aspiration) | 267/299 | 1 position skipped (WAC.274 bad fen); spec URL 404 — jdart1/arasan-chess mirror used |
