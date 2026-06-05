#!/usr/bin/env bash
# WAC (Win At Chess): the standard 300-position tactical canary.
# NOTE: spec URL https://raw.githubusercontent.com/fsmosca/STS-Rating/master/epd/wacnew.epd
# returned HTTP 404 on 2026-06-05. Using jdart1/arasan-chess mirror instead.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p suites
if [ ! -s suites/wac.epd ]; then
  curl -sSfL -o suites/wac.epd \
    "https://raw.githubusercontent.com/jdart1/arasan-chess/master/tests/wacnew.epd"
fi
wc -l suites/wac.epd
