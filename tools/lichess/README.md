# NebChess Lichess bot service

A standing live-play service: the official [lichess-bot](https://github.com/lichess-bot-devs/lichess-bot)
client plays the games (UCI bridge, clocks, ponder); our `matchmaker.py` paces
them — challenging online bots within a rating band of our **current** rating,
never exceeding a rolling-24h game budget (default 96, margin under Lichess's
100/day). Incoming bot challenges are accepted and count against the budget
(the Lichess API is the source of truth for the count).

## One-time setup
1. Repo-root `.env` (gitignored) with `LICHESS_KEY=<bot OAuth token, bot:play scope>`.
2. `git clone --depth 1 https://github.com/lichess-bot-devs/lichess-bot.git lichess-bot`
3. `python3 -m venv venv && venv/bin/pip install -r lichess-bot/requirements.txt`
4. Book + tablebases present: `tools/books/nebbook.bin` (tools/download-book.sh),
   `tools/tb/` (Syzygy 3-4-5).

## Deploy + run
```sh
./deploy.sh    # snapshot target/release/nebchess -> nebchess-live (bot must be stopped)
./run.sh       # starts both processes; Ctrl-C stops both
```

Deploy config: engine book (BookFile/BookDepth 16), Syzygy, ponder ON,
Hash 256, blitz 3+2 / 5+3 rated outgoing; incoming bots-only, blitz/rapid
with increment >= 1 (no bullet — WSL2 timing).

Tune via env before `./run.sh`:
`MM_BUDGET_PER_DAY=96 MM_RATING_BAND=300 MM_TCS="180+2,300+3"`.

**Do not run during measurements** (SPRT/gauntlet/datagen need an idle,
uncontended machine — and the engine plays weaker under load).
