# Demo script

A ~5-minute walkthrough using the Docker stack.

```bash
docker compose up --build
# gateway proxy :8080, admin :9090, echo1/echo2 upstreams, Redis, Prometheus :9091
```

## 0. Health & metrics (admin port)

```bash
curl -s localhost:9090/healthz   # ok
curl -s localhost:9090/readyz    # ready
curl -s localhost:9090/metrics | grep gw_ | head
```

## 1. Routing + load balancing

```bash
# /echo is round-robin across echo1 and echo2 (prefix stripped).
for i in $(seq 1 4); do
  curl -s localhost:8080/echo/hello | python3 -c 'import sys,json;d=json.load(sys.stdin);print(d.get("hostname") or d.get("os",{}).get("hostname"))'
done   # alternates between the two upstream container hostnames
```

## 2. Header transforms + correlation ID

```bash
curl -is localhost:8080/echo/ | grep -iE 'x-request-id|x-served-by'
# x-request-id is generated if you don't send one; echoed back on the response.
curl -is -H 'x-request-id: my-trace-123' localhost:8080/echo/ | grep -i x-request-id
```

## 3. Caching

```bash
curl -is localhost:8080/echo/cacheme | grep -i x-cache    # MISS
curl -is localhost:8080/echo/cacheme | grep -i x-cache    # HIT (served from cache)
```

## 4. Auth (the /secure route requires an API key)

```bash
curl -s -o /dev/null -w '%{http_code}\n' localhost:8080/secure/x                       # 401
curl -s -o /dev/null -w '%{http_code}\n' -H 'x-api-key: change-me-in-prod' localhost:8080/secure/x  # 200
```

## 5. Rate limiting

```bash
# Hammer a rate-limited route and watch for 429s + headers.
for i in $(seq 1 130); do curl -s -o /dev/null -w '%{http_code} ' localhost:8080/echo/; done; echo
curl -is localhost:8080/echo/ | grep -i x-ratelimit
```

## 6. Resilience (circuit breaker)

```bash
# Stop one upstream and watch traffic shift to the healthy one (health checks +
# breaker). Then stop both to see fast 503s.
docker compose stop echo2
for i in $(seq 1 6); do curl -s localhost:8080/echo/ -o /dev/null -w '%{http_code} '; done; echo
docker compose start echo2
```

## 7. Benchmark (overhead vs direct)

```bash
# Expose an upstream directly for comparison, then:
GATEWAY=http://localhost:8080/echo/ UPSTREAM=http://localhost:8080/echo/ \
REQUESTS=20000 CONCURRENCY=50 ./scripts/bench.sh
# Observe CPU/memory with: docker stats
```

## 8. Prometheus

Open http://localhost:9091 and query `gw_requests_total`,
`gw_circuit_breaker_state`, `gw_cache_hits_total`.
