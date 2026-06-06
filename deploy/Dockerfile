# Multi-stage build: compile the NebChess UCI engine, then run it under the
# official lichess-bot Python bridge. Build context must be the repo ROOT
# (the engine stage needs src/, Cargo.*, rust-toolchain.toml), e.g.:
#   docker build -f deploy/Dockerfile -t nebchess-bot .

# ---- Stage 1: build the engine ----------------------------------------------
# Base image matches rust-toolchain.toml (channel 1.96.0) so no extra rustup
# download is needed.
FROM rust:1.96.0-slim-bookworm AS engine

WORKDIR /build
# Copy manifests first so the dependency/build layer caches across source-only
# edits. (NebChess has zero external deps, so this is mostly future-proofing.)
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
RUN cargo build --release --bin nebchess
# Output: /build/target/release/nebchess

# ---- Stage 2: lichess-bot runtime -------------------------------------------
FROM python:3.12-slim AS runtime

# Pin lichess-bot to a specific master commit for reproducible deploys. The
# tagged releases (latest 1.1.3) predate env-var token support and modern deps;
# master is the maintained line and the recommended install. Override at build
# time with --build-arg LICHESS_BOT_REF=<commit-sha>.
ARG LICHESS_BOT_REF=ad4b56621bb0e6c52925212639fb73e8ce5ae451
ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1

# git is needed only to fetch lichess-bot at a pinned ref; remove it afterward.
RUN apt-get update \
 && apt-get install -y --no-install-recommends git ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Run as a non-root user that owns the working dir (lichess-bot writes logs/PGNs).
RUN useradd --create-home --uid 10001 bot
WORKDIR /home/bot/lichess-bot

# Shallow-fetch the pinned commit (GitHub allows fetching a SHA directly).
RUN git init -q . \
 && git remote add origin https://github.com/lichess-bot-devs/lichess-bot.git \
 && git fetch --depth 1 origin "${LICHESS_BOT_REF}" \
 && git checkout -q FETCH_HEAD \
 && pip install --no-cache-dir -r requirements.txt \
 && apt-get purge -y --auto-remove git

# Drop in the engine binary and our config.
COPY --from=engine /build/target/release/nebchess ./engines/nebchess
COPY deploy/config.yml ./config.yml
RUN chmod +x ./engines/nebchess \
 && chown -R bot:bot /home/bot

USER bot

# Outbound-only worker: lichess-bot opens an HTTPS event stream to lichess.org;
# nothing listens, so no port is exposed.
#
# The real API token is injected at runtime via the LICHESS_BOT_TOKEN env var
# (read natively by lichess-bot, overriding the blank token in config.yml).
# `-u` performs the one-time, idempotent BOT-account upgrade on first boot and
# then runs the bot normally; it is a no-op on later restarts.
CMD ["python3", "lichess-bot.py", "-u", "--config", "config.yml", "--disable_auto_logging"]
