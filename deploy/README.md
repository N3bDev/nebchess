# Deploying NebChess as a Lichess bot

This directory turns the NebChess UCI engine into a live [Lichess](https://lichess.org)
bot using the official [lichess-bot](https://github.com/lichess-bot-devs/lichess-bot)
Python bridge, packaged as a Docker image and deployed to [Railway](https://railway.app).

The whole flow is doable **from a phone** — no local machine, terminal, or
`docker`/`curl` required. Railway builds the image from this repo and runs it
always-on; the engine itself needs no changes.

There are two ways to run it:
- **[Run locally (WSL Ubuntu)](#run-locally-wsl-ubuntu)** — recommended for a strong
  desktop; full control, big hash, book + Syzygy, pondering.
- **Cloud (Railway)** — always-on, phone-deployable; see the rest of this doc.

## Run locally (WSL Ubuntu)

A native run (no Docker) on a beefy box, with every 0.7.0 feature on. Replace
`nebchessbot` references with your bot account.

```bash
# 0. Prereqs (once)
sudo apt update && sudo apt install -y curl git unzip python3 python3-venv build-essential
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y   # Rust
source "$HOME/.cargo/env"

# 1. NebChess: clone this branch, build, fetch the book + Syzygy tables
git clone -b claude/lichess-bot-deployment-3lfRU https://github.com/N3bDev/nebchess.git ~/nebchess
cd ~/nebchess
cargo build --release                 # -> target/release/nebchess
tools/download-book.sh                 # -> tools/books/nebbook.bin (~5MB, from the v0.7.0 release)
tools/download-syzygy.sh              # -> tools/tb/ (~1GB, 3-4-5 tables)
printf 'uci\nquit\n' | ./target/release/nebchess | grep -E 'id name|uciok'   # sanity

# 2. lichess-bot: clone, install deps, drop the engine + artifacts in engines/
git clone https://github.com/lichess-bot-devs/lichess-bot.git ~/lichess-bot
cd ~/lichess-bot
pip install -r requirements.txt        # add --break-system-packages if PEP-668 blocks it
mkdir -p engines/tb
cp ~/nebchess/target/release/nebchess engines/nebchess
cp ~/nebchess/tools/books/nebbook.bin engines/nebbook.bin
cp ~/nebchess/tools/tb/* engines/tb/

# 3. Config + token, then run
cp ~/nebchess/deploy/config.yml config.yml      # already set for book/Syzygy/ponder/Hash 4096
export LICHESS_BOT_TOKEN=lip_xxxxxxxxxxxx        # your bot token (bot:play [+ challenge:write])
python3 lichess-bot.py -u                        # -u upgrades the account on first run
```

The shipped `config.yml` already points `BookFile`/`SyzygyPath` at `./engines/...`,
which is where step 2 copies them. If the engine logs that it can't find them, use
absolute paths (e.g. `/home/<you>/lichess-bot/engines/nebbook.bin`).

**Note on "cranking it up":** the engine is **single-threaded** (`Threads` maxes at
1; Lazy SMP is a later milestone), so a 24-core CPU can't be parallelised yet — but
its fast single-core throughput plus `Hash 4096`, the book, Syzygy, and pondering
make it as strong as 0.7.0 gets. Longer time controls (rapid/classical) let it
search deeper and play better, too.

## What's here (cloud / Railway)

| File | What it does |
|------|--------------|
| `deploy/Dockerfile` | Multi-stage build: compiles `nebchess` (Rust 1.96), then runs lichess-bot (pinned master commit) on Python 3.12 with the engine binary in `engines/nebchess`. |
| `deploy/config.yml` | lichess-bot config (UCI, hash/overhead, which challenges to accept). Token is **blank** — injected at runtime. |
| `railway.json` (repo root) | Tells Railway to build `deploy/Dockerfile` and keep the service running (`restartPolicyType: ALWAYS`). |
| `.dockerignore` (repo root) | Keeps the build context small/fast. |

## How it works

- **Token** comes from the `LICHESS_BOT_TOKEN` environment variable, which
  lichess-bot reads natively and uses instead of the blank token in `config.yml`.
  The token is **never committed** — you set it as a Railway variable.
- **Account upgrade** is automatic: the container command includes `-u`, which
  upgrades the account to a BOT account on first boot (and is a harmless no-op
  afterward). No manual API call needed.
- The bot is an **outbound-only worker** — it opens an HTTPS event stream to
  Lichess and waits for challenges. No ports are exposed, so it must stay
  always-on (Railway's hobby worker does not sleep).

## Setup (all from your phone)

### 1. Create a bot Lichess account
Sign up for a **brand-new** Lichess account for the bot (e.g. `NebChessBot`).
> ⚠️ It must have **zero games ever played** — Lichess only allows upgrading a
> gameless account to a BOT account. Don't play any games on it.

### 2. Generate an API token
On lichess.org: **Preferences → API access tokens → +** (new token).
- Check the scope **"Play games with the bot API"** (`bot:play`).
- Generate, then **copy the token** (`lip_...`) — it's shown only once.

### 3. Deploy on Railway
On [railway.app](https://railway.app) in your mobile browser:
1. **New Project → Deploy from GitHub repo** → select `N3bDev/nebchess`.
2. Choose the branch **`claude/lichess-bot-deployment-3lfRU`**.
3. Railway reads `railway.json` and builds `deploy/Dockerfile` automatically.
4. Open the service → **Variables** → add:
   - `LICHESS_BOT_TOKEN` = *(the token from step 2)*
5. Deploy (Railway redeploys when the variable is added).

On first boot the bot upgrades the account to a BOT account, connects to Lichess,
and waits for challenges.

### 4. Play it
From your normal Lichess account, open `https://lichess.org/@/NebChessBot`
→ **Challenge to a game** → pick a time control (blitz/rapid work great) → play.

## Verifying it's live
In Railway's **Deploy logs** you should see lichess-bot authenticate, report the
account is (now) a BOT, and "accept" challenges as they arrive. When you challenge
it, the logs show the game start and the engine's moves.

Try a **Chess960** challenge to confirm it's declined — NebChess plays standard
chess only (`variants: [standard]`).

## Tuning
- **Time trouble in fast games?** Raise `Move Overhead` / `move_overhead` in
  `config.yml`, or raise `min_base`.
- **Cut cost?** Drop `uci_options.Hash` to `128` so it fits a 256 MB instance.
- **Seek games actively?** Set `matchmaking.allow_matchmaking: true` (also add the
  `challenge:write` scope to the token).
- Full reference: lichess-bot's [`config.yml.default`](https://github.com/lichess-bot-devs/lichess-bot/blob/master/config.yml.default).

## Running elsewhere
The image is portable. On any Docker host:
```bash
docker build -f deploy/Dockerfile -t nebchess-bot .
docker run -e LICHESS_BOT_TOKEN=lip_xxx nebchess-bot
```
