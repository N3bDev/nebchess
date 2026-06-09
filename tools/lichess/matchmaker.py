#!/usr/bin/env python3
"""NebChess Lichess matchmaker — paced bot-vs-bot challenger.

Runs ALONGSIDE lichess-bot (which actually plays the games; its built-in
matchmaking stays off). This process only decides WHEN to challenge and WHOM:

  1. Tracks a rolling-24h game count via the Lichess API (the API is the
     authority, so games from incoming challenges count against the budget too).
  2. Picks an online bot whose blitz rating is within RATING_BAND of ours
     (re-read every cycle, so the band tracks our rating as it climbs).
  3. Issues one rated challenge at a time, only while under budget and idle.

Budget defaults to 96/day — a safety margin under Lichess's 100 games/day.

Env (required): LICHESS_BOT_TOKEN
Env (optional): MM_BUDGET_PER_DAY=96  MM_RATING_BAND=300
                MM_TCS="180+2,300+3"  MM_POLL_SECONDS=30
"""

import json
import os
import random
import sys
import time

import requests

API = "https://lichess.org"
TOKEN = os.environ.get("LICHESS_BOT_TOKEN") or sys.exit("LICHESS_BOT_TOKEN not set")
HEADERS = {"Authorization": f"Bearer {TOKEN}"}

BUDGET_PER_DAY = int(os.environ.get("MM_BUDGET_PER_DAY", "96"))
RATING_BAND = int(os.environ.get("MM_RATING_BAND", "300"))
TCS = [tc.split("+") for tc in os.environ.get("MM_TCS", "180+2,300+3").split(",")]
POLL_SECONDS = int(os.environ.get("MM_POLL_SECONDS", "30"))
OPPONENT_COOLDOWN_S = 2 * 3600  # don't re-challenge the same bot for 2h
STATE_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "matchmaker-state.json")

DONT_COUNT = {"aborted", "noStart"}  # statuses that don't consume Lichess's daily allowance


def log(msg):
    print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)


def get(path, headers=None, **kw):
    h = {**HEADERS, **(headers or {})}
    r = requests.get(f"{API}{path}", headers=h, timeout=30, **kw)
    if r.status_code == 429:
        log("rate-limited (429) — backing off 90s")
        time.sleep(90)
        r = requests.get(f"{API}{path}", headers=h, timeout=30, **kw)
    r.raise_for_status()
    return r


def ndjson(resp):
    return [json.loads(line) for line in resp.text.splitlines() if line.strip()]


def load_state():
    try:
        with open(STATE_FILE) as f:
            return json.load(f)
    except (OSError, ValueError):
        return {"recent_opponents": {}}


def save_state(state):
    with open(STATE_FILE, "w") as f:
        json.dump(state, f)


def my_profile():
    acct = get("/api/account").json()
    name = acct["username"]
    blitz = acct.get("perfs", {}).get("blitz", {}).get("rating", 2500)
    return name, blitz


def games_in_last_24h(username):
    since = int((time.time() - 24 * 3600) * 1000)
    resp = get(
        f"/api/games/user/{username}",
        params={"since": since, "max": 300, "moves": "false", "ongoing": "true"},
        headers={"Accept": "application/x-ndjson"},
    )
    games = ndjson(resp)
    return sum(1 for g in games if g.get("status") not in DONT_COUNT)


def now_playing():
    return get("/api/account/playing").json().get("nowPlaying", [])


def pick_opponent(my_name, my_rating, state):
    resp = get("/api/bot/online", params={"nb": 200},
               headers={"Accept": "application/x-ndjson"})
    bots = ndjson(resp)
    now = time.time()
    recent = state["recent_opponents"]
    candidates = []
    for b in bots:
        name = b.get("username") or b.get("id", "")
        if not name or name.lower() == my_name.lower():
            continue
        if b.get("tosViolation") or b.get("disabled"):
            continue
        if now - recent.get(name.lower(), 0) < OPPONENT_COOLDOWN_S:
            continue
        rating = b.get("perfs", {}).get("blitz", {}).get("rating")
        if rating is None or abs(rating - my_rating) > RATING_BAND:
            continue
        candidates.append((name, rating))
    return random.choice(candidates) if candidates else None


def send_challenge(opponent):
    limit, inc = random.choice(TCS)
    r = requests.post(
        f"{API}/api/challenge/{opponent}",
        headers=HEADERS, timeout=30,
        data={"rated": "true", "clock.limit": limit, "clock.increment": inc,
              "variant": "standard", "color": "random"},
    )
    if r.status_code == 429:
        log("challenge rate-limited — backing off 90s")
        time.sleep(90)
        return None
    if not r.ok:
        log(f"challenge to {opponent} failed: {r.status_code} {r.text[:120]}")
        return None
    challenge_id = r.json().get("id") or r.json().get("challenge", {}).get("id")
    log(f"challenged {opponent} at {limit}+{inc} (id {challenge_id})")
    return challenge_id


def cancel_challenge(challenge_id):
    requests.post(f"{API}/api/challenge/{challenge_id}/cancel", headers=HEADERS, timeout=30)


def main():
    my_name, _ = my_profile()
    log(f"matchmaker up for {my_name}: budget {BUDGET_PER_DAY}/24h, band +/-{RATING_BAND}, "
        f"TCs {os.environ.get('MM_TCS', '180+2,300+3')}")
    state = load_state()

    while True:
        try:
            if now_playing():
                time.sleep(POLL_SECONDS)
                continue

            played = games_in_last_24h(my_name)
            if played >= BUDGET_PER_DAY:
                log(f"budget reached ({played}/{BUDGET_PER_DAY} in 24h) — sleeping 10 min")
                time.sleep(600)
                continue

            _, my_rating = my_profile()  # re-read: the band tracks our live rating
            pick = pick_opponent(my_name, my_rating, state)
            if pick is None:
                log(f"no online bot within +/-{RATING_BAND} of {my_rating} — retry in 3 min")
                time.sleep(180)
                continue

            opponent, opp_rating = pick
            log(f"budget {played}/{BUDGET_PER_DAY}; we are {my_rating}, "
                f"targeting {opponent} ({opp_rating})")
            challenge_id = send_challenge(opponent)
            state["recent_opponents"][opponent.lower()] = time.time()
            save_state(state)
            if challenge_id is None:
                time.sleep(POLL_SECONDS)
                continue

            # Wait up to 90s for the opponent to accept; otherwise withdraw and move on.
            for _ in range(6):
                time.sleep(15)
                playing = now_playing()
                if playing:
                    # If the game that started is NOT against our challengee (an
                    # incoming game raced us), withdraw the stale challenge.
                    started_vs = playing[0].get("opponent", {}).get("id", "")
                    if started_vs and started_vs != opponent.lower():
                        cancel_challenge(challenge_id)
                        log(f"incoming game vs {started_vs} raced the challenge — withdrew {opponent}")
                    break
            else:
                cancel_challenge(challenge_id)
                log(f"{opponent} did not accept — challenge withdrawn")

        except requests.RequestException as e:
            log(f"API error: {e} — retry in 2 min")
            time.sleep(120)
        except KeyboardInterrupt:
            log("stopped")
            return


if __name__ == "__main__":
    main()
