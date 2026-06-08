# NebChess — Lichess bot deployment

How to run the engine as a Lichess bot via [lichess-bot](https://github.com/lichess-bot-devs/lichess-bot).
The engine is UCI; lichess-bot drives it directly.

## Build

```bash
cargo build --release   # → target/release/nebchess
```

Ship `target/release/nebchess` plus the two data artifacts below to the bot host.

## UCI options (what to set in lichess-bot's config)

| Option | Type | Default | Recommended on Lichess | Why |
|--------|------|---------|------------------------|-----|
| `Hash` | spin MB | 16 | **256** (or as RAM allows) | bigger TT = stronger at the bot's TCs |
| `Move Overhead` | spin ms | 50 | **100–300** | network + lichess-bot round-trip; raise if you ever see time forfeits |
| `BookFile` | string | _(off)_ | **path to `nebbook.bin`** | +51.6 elo self-play; instant sound openings live (see below) |
| `BookDepth` | spin plies | 16 | 16 | book answers the first 16 plies, then searches |
| `SyzygyPath` | string | _(off)_ | **path to the 3-4-5 tables dir** | perfect endgame play ≤5 men; closes the KBvK/KR-K field leaks |
| `Threads` / `MultiPV` | spin | 1 | 1 | single-threaded engine (Lazy SMP is a later milestone) |

`Ponder` arrives with the pondering task (Plan 7 T7); enable it in lichess-bot once advertised.

### Data artifacts (gitignored — build/fetch on the host)
- **Opening book**: `cargo run --release --bin bookgen tools/books/nebbook.bin db/export_ELO2400.pgn` (or copy a prebuilt `.bin`). PolyGlot format.
- **Syzygy 3-4-5**: `tools/download-syzygy.sh` (~939 MB into `tools/tb/`), point `SyzygyPath` at that directory.

## Time management

TimeBrain v1 (shipped) handles the clock from the `go wtime/btime/winc/binc` lichess-bot sends — emergency reserve (never flags), best-move-stability early-stop, and a hard cap. **No movetime override needed**; let the engine manage its own clock. Validated: KR-vs-K converts 20/20 at 5+0.1 (the field clock-collapse leak is fixed). Forfeit-tested at 180+2, 300+3, and 60+0 (see the forfeit battery in the strength log).

## TC ranges to accept

The engine is sound across blitz and rapid (180+2 / 300+3 are the tested field TCs; 60+0 sudden-death is forfeit-clean). Bullet (60+0 and faster) works but is where a weak network/overhead bites first — keep `Move Overhead` ≥ 100 there. No known instability; the UCI torture battery (`tools/uci-torture.sh`, 20/20) covers truncated FENs, illegal moves, zero/negative clocks, stop-storms, and mid-search disconnects.

## Operational checklist

1. `cargo build --release`; confirm `printf 'uci\nquit\n' | ./target/release/nebchess` prints `id name NebChess <version>` + `uciok`.
2. Place `nebbook.bin` and the Syzygy dir on the host; set `BookFile` / `SyzygyPath` in lichess-bot's `config.yml` engine options.
3. Set `Hash 256`, `Move Overhead 100`.
4. Smoke a game offline: `tools/krk-stress.sh` (KR-K conversion) + `tools/uci-torture.sh` (robustness) both green.
5. Start lichess-bot; watch the first few games for time usage (the `info string time soft=.. hard=.. used=..` line reports per-move allocation).

## Forfeit battery (T6, 2026-06-07)

TimeBrain v1 forfeit-clean across the live TC spectrum — **zero NebChess time forfeits**:
`60+0` SD (120 games, the no-increment reserve stress) · `180+2` (60) · `300+3` (60).
Plus the always-on `8+0.08` SPRT/gauntlet history (0 forfeits over thousands of games)
and the UCI torture battery (`tools/uci-torture.sh`, 20/20: hostile FENs, illegal
moves, zero/negative clocks, stop-storms, mid-search disconnects — no panic/hang).
