# SPRT Log

Per-feature self-play results (protocol: tools/sprt.sh — STC 8+0.08, Hash 16,
8moves_v3 book, reversed pairs, alpha=beta=0.05, model=normalized).
H1 accepted = gain >= elo1 confirmed; the feature binary becomes the next baseline.

**M3 cumulative: +384.4 measured self-play elo over M2. M4 cumulative: +553.2 over M3** (self-play deltas overstate rating-list deltas ~1.5x per spec §10.4 — absolute claims come from anchored gauntlets only). Informational anchors: M3 closed 8-2 vs SF@1800; M4 closed 10-0 vs SF@2000. **M5 cumulative: +379.1 self-play (gates T1-T6) → +369 anchored (2414→2783).**

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
| 2026-06-06 | **Pawn structure + pawn hash, K frozen** (M5 T2) | tapered | [0,10] | 470 | 197-110-163 | **H1: +65.1 ±24.9** (0 timeouts; canary 261 = −6 accepted trade after K-degeneracy incident — see tactics-log) | 102691 |
| 2026-06-06 | **Safe mobility (4 tables)** (M5 T3) | pawns | [0,10] | 406 | 168-83-155 | **H1: +73.8 ±26.7** (0 timeouts; canary 262 = +1; trapped-piece signal lands) | 107109 |
| 2026-06-06 | **King safety (zone/shield/files)** (M5 T4) | mobility | [0,10] | 4396 | 1374-1249-1773 | **H1: +9.9 ±7.7** (0 timeouts; smallest effect yet — knowledge ≈ −13% NPS tax, canary churn 257; shared attack-map refactor mandated before T5, see plan step 5.0 decision tree) | 99185 |
| 2026-06-06 | *(infrastructure)* shared attack-map pass @ 9b6adfa | kingsafety | — | — | — | bench-identical refactor (99185, node-identical d13); NPS +21-23%; baseline refreshed to fused build — **not credited as elo**, T5 gate isolates knowledge | 99185 |
| 2026-06-06 | **Threats + coordination + tempo** (M5 T5) | ks-fused | [0,5] | 580 | 274-90-216 | **H1: +114.2 ±23.7** (0 timeouts; largest M5 gain — knowledge nearly free on the AttackMaps pass; canary 267 ties project high; val MSE −4.0%; tuner moved threat ladders off-monotone, accepted by verdict) | 71571 |
| 2026-06-06 | big3 7.15M joint tune (M5 T6 values) | threats | [0,5] | 1788 | — | **STOPPED UNRESOLVED at user call: −0.0 ±13.0, LLR −0.18 — NOT baselined** (walked 0→−27→−14→0; canary 269 = project high yet plays dead even: "tactically sharper ≠ stronger engine"). Values reverted to T5; big3 = useful failed evidence — needs scale/phase controls (plan step 6.5 investigation). Tuner infra (9× parallel, big3 loader) RETAINED | 70048 |
| 2026-06-06 | *(probe, not a gate)* exp1 fit-K / exp2 pin-material | threats | — | — | — | step 6.5 experiments 1-2 FALSIFIED analytically: K_big3=1.305 bought zero fit, distortion persisted; pinning material re-routed it into MOB_QUEEN — intrinsic phase-conditional label noise in human blitz | 64698/68308 |
| 2026-06-06 | **Hybrid zurichess+big3 1:1 tune** (M5 T6 final) | threats | [0,5] | 3558 | 1076-904-1578 | **H1: 52.42%, LOS 100% (~+17)** — dilution carries half the distortion, coverage outweighs it; canary 268; 3 timeouts (old side, host noise); probe 400g was +13.9 ±25.6 | 77211 |
| 2026-06-07 | **SEE + qsearch losing-capture pruning** (M6.1 T4) | hybrid | [0,5] | 1670 | 516-358-796 | **H1: +33.0 ±12.1** (0 timeouts; canary 271 = project high; ~26% qsearch node cut) | 57181 |
| 2026-06-07 | **Continuation history 1+2-ply w/ malus** (M6.1 T5) | see | [0,10] | 3332 | 926-818-1588 | **H1: +11.3 ±8.4** (1 timeout old-side; canary 273 = project high; per-go table lifetime — cross-move persistence is Plan 7) | 54728 |
| 2026-06-07 | log-formula LMR + history adjustment (M6.1 T6) | conthist | [0,5] | 8366 | 2097-2197-4072 | **H0: −4.2 ±5.1 — REVERTED** (log formula max r=8 too aggressive vs the additive ladder max r=3; ±2 hist adjustment pure variance; quiet_history refactor kept, bench-identical; canary was 270) | 54755→54728 |
| 2026-06-07 | singular + check extensions (M6.1 T7) | conthist | [0,5] | 4726 | 1163-1285-2278 | **H0: −9.0 ±7.0 — REVERTED** (canary 276 = all-time high yet plays −9: unconditional check ext doubled the tree, node cost > forcing-line gains, 4 new-side timeouts; "tactically sharper ≠ stronger" rides again; singular-only probe queued) | 100320→54728 |
| 2026-06-07 | *(probe, not a gate)* singular-only carve | conthist | — | 400 | 105-99-196 | probe +5.2 ±23.9, inside the ±10 bar — NO SPRT spent; attribution complete: check ext owned both the 276 canary high AND the −9 play loss (carve is bench-identical 54728, singular never fires at bench depths); singular retry queued for deeper-search era, gated check ext for M7+ | 54728 |
