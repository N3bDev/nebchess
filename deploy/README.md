# Deploying NebChess as a Lichess bot

This directory turns the NebChess UCI engine into a live [Lichess](https://lichess.org)
bot using the official [lichess-bot](https://github.com/lichess-bot-devs/lichess-bot)
Python bridge, packaged as a Docker image and deployed to [Railway](https://railway.app).

The whole flow is doable **from a phone** — no local machine, terminal, or
`docker`/`curl` required. Railway builds the image from this repo and runs it
always-on; the engine itself needs no changes.

## What's here

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
