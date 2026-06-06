# SPRT Log

Per-feature self-play results (protocol: tools/sprt.sh — STC 8+0.08, Hash 16,
8moves_v3 book, reversed pairs, alpha=beta=0.05, model=normalized).
H1 accepted = gain >= elo1 confirmed; the feature binary becomes the next baseline.

| date | feature | vs baseline | bounds | games | W-L-D | result | bench |
|------|---------|-------------|--------|-------|-------|--------|-------|
| 2026-06-05 | TT cutoffs | m2 | [0,10] | 568 | 241-154-173 | **H1: +53.6 ±22.2** (2 timeouts = host load; controlled gauntlet 0/200) | 3027664 |
| 2026-06-05 | TT-move ordering + king LVA fix | tt-cuts | [0,10] | 236 | 142-36-58 | **H1: +168.0 ±41.0** (1 timeout, same load pattern) | 2945582 |
| 2026-06-05 | Killer moves | tt-ordering | [0,10] | 438 | 194-99-145 | **H1: +76.6 ±27.0** (0 timeouts) | 1983350 |
| 2026-06-05 | Butterfly history | killers | [0,10] | 624 | 246-164-214 | **H1: +45.9 ±20.0** (2 timeouts EACH side — symmetric load noise) | 1820387 |
| 2026-06-05 | PVS | history | [0,10] | 744 | 284-198-262 | **H1: +40.3 ±18.7** (1 timeout) | 1312955 |

| 2026-06-05 | Null-move pruning | pvs | [0,10] | 330 | 155-63-112 | **H1: +99.5 ±31.0** (0 timeouts) | 689295 |
| 2026-06-05 | LMR (depth>=3, killers exempt) | nullmove | [0,10] | 934 | 301-217-416 | **H1: +31.3 ±16.1** (1 timeout, old side) | 212534 |
| 2026-06-05 | Aspiration windows | lmr | [0,10] | 402 | 149-66-187 | **H1: +72.8 ±25.0** (0 timeouts; LMR synergy) | 212534 |
| 2026-06-05 | RFP + futility (d<=2 after canary fix) | aspiration | [0,10] | 300 | 122-36-142 | **H1: +102.5 ±28.4** (tactics canary: 266/299 ✓ — see tactics-log incident) | 97369 |
| 2026-06-05 | Quiescence TT | futility | [0,5] | 1488 | 421-274-793 | **H1: +34.4 ±11.8** (canary 266 ✓) | 85636 |
| 2026-06-05 | **Texel-tuned PST+material** (first learned eval) | qsearch-tt | [0,5] | 416 | 286-59-71 | **H1: +212.7 ±35.4** — largest gain in project history; WAC −8 accepted as the documented trade | 138119 |
| 2026-06-06 | **Tapered mg/eg + trace arch + retune** (M5 T1) | texel-pst | [0,10] | 342 | 176-81-85 | **H1: +99.1 ±32.0** (canary 267 = project high; val MSE −3.3%) | 90242 |

**M3 cumulative: +384.4 measured self-play elo over M2. M4 cumulative: +553.2 over M3** (self-play deltas overstate rating-list deltas ~1.5x per spec §10.4 — absolute claims come from anchored gauntlets only). Informational anchors: M3 closed 8-2 vs SF@1800; M4 closed 10-0 vs SF@2000.
