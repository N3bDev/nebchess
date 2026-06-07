# NebChess

A from-scratch UCI chess engine in Rust, targeting 2400 CCRL Blitz.

Design spec: [docs/superpowers/specs/2026-06-04-nebchess-engine-design.md](docs/superpowers/specs/2026-06-04-nebchess-engine-design.md)

## Status

- [x] M0: scaffolding, CI, test harness
- [x] M1: board representation + perft-verified move generation
- [x] M2: minimal playing engine (search, eval, UCI)
- [x] M3: transposition table + move ordering + PVS
- [x] M4: search pruning (null move, LMR, aspiration) + PST tuning pipeline
- [x] M5: full HCE evaluation + Texel tuning at scale
- [ ] M6: search & eval polish + bot readiness (book, Syzygy, time management, Lichess hardening)

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
