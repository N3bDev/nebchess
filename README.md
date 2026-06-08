# NebChess

A from-scratch UCI chess engine in Rust. Measured **2827 ± 16** (10+0.1, anchored vs a 3-family pool, engine-default config — book/Syzygy are live-deploy multipliers on top; see docs/strength-log.md for caveats). **M6 target: 2900 (stretch 3000).**

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
- [ ] M7: eval round 2 (outposts, king-attack rework, gated check extensions) + deeper-search retries (singular, futility v2) + TimeBrain-v2 (Lichess-tuned) + desktop migration

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
