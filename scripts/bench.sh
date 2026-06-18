#!/usr/bin/env bash
#
# Direct-upstream vs through-the-gateway throughput comparison.
#
# Measures the gateway's added overhead on your machine. It prints whatever the
# load tool reports — it ships NO canned numbers. Requires `oha`
# (https://github.com/hatoo/oha) or falls back to `wrk` if present.
#
# Usage:
#   GATEWAY=http://localhost:8080/echo/ UPSTREAM=http://localhost:8081/ \
#   REQUESTS=20000 CONCURRENCY=50 ./scripts/bench.sh
set -euo pipefail

GATEWAY="${GATEWAY:-http://localhost:8080/echo/}"
UPSTREAM="${UPSTREAM:-http://localhost:8081/}"
REQUESTS="${REQUESTS:-20000}"
CONCURRENCY="${CONCURRENCY:-50}"

run() {
  local label="$1" url="$2"
  echo "=================================================="
  echo "$label  ($url)"
  echo "=================================================="
  if command -v oha >/dev/null 2>&1; then
    oha -n "$REQUESTS" -c "$CONCURRENCY" --no-tui "$url"
  elif command -v wrk >/dev/null 2>&1; then
    wrk -t4 -c"$CONCURRENCY" -d15s "$url"
  else
    echo "Neither 'oha' nor 'wrk' is installed."
    echo "Install one, e.g.: cargo install oha"
    exit 1
  fi
  echo
}

echo "Comparing direct upstream vs gateway."
echo "Requests=$REQUESTS Concurrency=$CONCURRENCY"
echo
run "DIRECT UPSTREAM" "$UPSTREAM"
run "THROUGH GATEWAY" "$GATEWAY"

cat <<'NOTE'
Interpretation:
  The gateway adds routing, header rewriting, metrics and (optionally) auth /
  rate-limiting / caching. The difference between the two runs above is the
  gateway's overhead ON YOUR HARDWARE. CPU/memory can be observed via
  `docker stats` (compose) or `/usr/bin/time -v` while the load runs.
  No benchmark numbers are committed to this repo on purpose.
NOTE
