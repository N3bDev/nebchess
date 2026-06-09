# NebChess

A from-scratch UCI chess engine in Rust with a self-play-trained **NNUE** evaluation. Measured **2993 ± 17** (10+0.1, anchored vs a 3-family pool, engine-default config — book/Syzygy are live-deploy multipliers on top; see docs/strength-log.md for caveats). **M8 (NNUE) cleared the 2900 target — the 3000 stretch goal sits inside the error band; M9 net-scaling closes it.**

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
- [ ] M9: NNUE scaling (larger hidden layer, output/king buckets, datagen→train→promote loop) → push past 3000

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
