# NebChess

A from-scratch UCI chess engine in Rust, targeting 2400 CCRL Blitz.

Design spec: [docs/superpowers/specs/2026-06-04-nebchess-engine-design.md](docs/superpowers/specs/2026-06-04-nebchess-engine-design.md)

## Status

- [x] M0: scaffolding, CI, test harness
- [x] M1: board representation + perft-verified move generation
- [ ] M2: minimal playing engine (search, eval, UCI)

## Build

```sh
cargo build --release
```
