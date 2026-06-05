# SPRT Log

Per-feature self-play results (protocol: tools/sprt.sh — STC 8+0.08, Hash 16,
8moves_v3 book, reversed pairs, alpha=beta=0.05, model=normalized).
H1 accepted = gain >= elo1 confirmed; the feature binary becomes the next baseline.

| date | feature | vs baseline | bounds | games | W-L-D | result | bench |
|------|---------|-------------|--------|-------|-------|--------|-------|
| 2026-06-05 | TT cutoffs | m2 | [0,10] | 568 | 241-154-173 | **H1: +53.6 ±22.2** (2 timeouts = host load; controlled gauntlet 0/200) | 3027664 |
| 2026-06-05 | TT-move ordering + king LVA fix | tt-cuts | [0,10] | 236 | 142-36-58 | **H1: +168.0 ±41.0** (1 timeout, same load pattern) | 2945582 |
| 2026-06-05 | Killer moves | tt-ordering | [0,10] | 438 | 194-99-145 | **H1: +76.6 ±27.0** (0 timeouts) | 1983350 |
