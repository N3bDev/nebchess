# NebChess

A from-scratch UCI chess engine in Rust with a self-play-trained **NNUE** evaluation. Measured **3192 ± 18** (10+0.1, anchored vs a 5-family pool spanning 2713–3458, engine-default config — book/Syzygy are live-deploy multipliers on top; see docs/strength-log.md for caveats). **Two turns of the self-play data flywheel have delivered +203 anchored on an unchanged 600k-parameter net** — and M10's capacity ladder showed width is not yet affordable at this engine's NPS (data > parameters, measured).

Design spec: [docs/superpowers/specs/2026-06-04-nebchess-engine-design.md](docs/superpowers/specs/2026-06-04-nebchess-engine-design.md)

## Status

- [x] M0: scaffolding, CI, test harness
- [x] M1: board representation + perft-verified move generation
- [x] M2: minimal playing engine (search, eval, UCI)
- [x] M3: transposition table + move ordering + PVS
- [x] M4: search pruning (null move, LMR, aspiration) + PST tuning pipeline
- [x] M5: full HCE evaluation + Texel tuning at scale
- [x] M6a: bracketed measurement + search polish (SEE, conthist; LMR/extensions/futility-v2 honestly H0'd)
- [x] M6b: TimeBrain + bot readiness (book, Syzygy, pondering, Lichess hardening)
- [x] M7: desktop migration (RTX 5080); TimeBrain-v2 attempted → de-scoped (NebChess is time-elastic — banking time costs strength)
- [x] M8: **NNUE evaluation** — self-play-trained `(768→768)x2→1` SCReLU net, **+165 anchored (2827→2993)**, replaces HCE
- [x] M9: **the data flywheel** — net2 = same architecture retrained on data self-played by the NNUE engine, **+142 anchored (2989→3131)**; anchor ladder extended to 3458 (Carp, Midnight)
- [x] M10: **capacity ladder** — flywheel turn 2 **+61 anchored (3131→3192)**; width honestly H0'd (1024 = −30 at fixed time — the NPS tax beats the judgment gain; re-gate after eval-efficiency work), buckets +10 isolated
- [ ] M11: eval efficiency (lazy accumulator updates, fused SIMD kernels) + search selectivity round + book audit → lower the width tax, then re-gate capacity

## Play against it

Build (`cargo build --release`), then point any UCI GUI (CuteChess, Arena,
En Croissant) at `target/release/nebchess`.

## Development

Engine-affecting commits carry a `Bench: <nodes>` line (get it via
`./target/release/nebchess bench | tail -1`); CI re-runs the bench and
fails on mismatch. Docs/tooling commits omit the line and are skipped.

## Build

```sh
cargo build --release
```
