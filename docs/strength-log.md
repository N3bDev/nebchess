# Strength Log (anchored)

Absolute-scale measurements: gauntlets vs fixed-version engines with
published CCRL Blitz ratings, rated by Ordo with all anchors pinned.
Self-play ELO (sprt-log) measures feature deltas; THIS ledger is the
project's only source of absolute claims. Re-pull live CCRL numbers
before any public claim (anchor ratings drift).

| date | nebchess | pool (games/pairing) | ordo estimate | notes |
|------|----------|----------------------|---------------|-------|
| 2026-06-05 | 0.4.0 @ 11c407b | RusticA2/Stash13/15/17/19 (300 ea, 10+0.1) | **2414.2 ± 21.7** | 76% overall; per-rung 98.5/91.0/81.7/62.5/45.8%; lost the v19 (2473) pairing on points — honest top rung; all 4 time forfeits were ANCHORS, NebChess 0 in 1500 games. Caveat: 10+0.1 ≠ CCRL Blitz 2'+1" — TC-transfer error applies; M10's official claim uses longer TC + ≥1000/pairing |
| 2026-06-06 | 0.5.0 @ b04d2ee (M5 full HCE + hybrid tune) | Stash v15/17/19/20/21 (2140–2714 CCRL Blitz pins; 300 ea, 10+0.1) | **2783.3 ± 21.7** | 1500 games, 83.2% overall; 61.8% vs the 2714 ceiling — estimate EXTRAPOLATED above the top anchor (M6 pool needs ~2850+ rung); same TC-transfer + generic-anchor-build caveats as 0.4.0, so the +369 delta vs 2414 is the robust claim; forfeits 1/1500 own (18ms WSL2 noise), 6 anchor-side |
| 2026-06-07 | 0.5.0 @ b04d2ee (bracketed re-measurement, M6.0) | **3-family pool**: Stash 19/20/21/25 + Weiss 1.0 + Koivisto 2.0 (2471–2934 CCRL 40/2 1-CPU pins, list 2025-12-20; 300 ea, 10+0.1) | **2773.7 ± 15.8** | 1800 games, 53.2%; per-rung implied: Stash 2787/2766/2780/2788 (internally consistent INCLUDING the 2934 above-bracket), Koivisto ~2799, Weiss ~2723 — real ±40 style spread across families, joint estimate moved only −10 vs the Stash-only extrapolation: **monoculture concern quantified, M5 number validated**. Stash20 crashed 10/300 (anchor-side; ≤5 elo inflation of that rung). NebChess 0 crashes/forfeits. **M6 TARGET SET: 2900 (stretch 3000)** per spec branch ≥2750 |
| 2026-06-07 | 0.6.0 @ 650bf84 (M6a search polish) | same 3-family pool (2471–2934 CCRL 40/2 1-CPU, list 2025-12-20; 300 ea, 10+0.1) | **2811.4 ± 15.9** | 1800 games, 57.3%; **+37.7 vs 0.5.0** (banked +44 self-play SEE+conthist, ~0.85 compression); per-rung: Stash 86.0/85.2/66.3/36.5, Weiss 30.0 (implied ~2751, still the harsh judge), Koivisto 39.7; rung-implied band 2751–2838; 14 forfeits ALL anchor-side (13 Koivisto ~100ms vintage-TM overruns — mild rung inflation), NebChess 0/1800; **target 2900: 89 to go — M6b TimeBrain is the field-telemetry play** |
