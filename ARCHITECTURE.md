# Architecture

## Overview

A Tokio/Axum reverse proxy. A single fallback handler receives every request and
runs it through a fixed pipeline driven by validated YAML config. Pure routing
and the config model live in `gw-core`; all runtime behavior lives in
`gw-gateway`.

## Request pipeline

```
request → [route match] → [auth] → [rate limit] → [cache lookup]
        → [pick upstream: LB + circuit breaker] → [proxy w/ timeout + retries]
        → [response transforms + cache store] → response
```

Each stage can short-circuit:
- **match** miss → `404`
- **auth** fail → `401`
- **rate limit** exceeded → `429` (+ `X-RateLimit-*`)
- **cache** hit → cached response (`X-Cache: HIT`)
- **no eligible upstream** → `503`
- **upstream error/timeout** → retry (if eligible) else `502`

## Components

| Module | Role |
|--------|------|
| `gw-core::router` | Longest-prefix host/path matching (pure, tested). |
| `balancer` | Round-robin (weighted) & least-connections over `Upstream`s, each with a health flag, active-request counter and circuit breaker. |
| `breaker` | Per-upstream circuit breaker (Closed/Open/HalfOpen). |
| `cache` | Capacity-bounded TTL response cache. |
| `ratelimit` | `RateLimiter` trait + in-memory fixed-window + Redis (`INCR`/`EXPIRE`). |
| `auth` | API-key and HS256 JWT validation. |
| `transform` | Hop-by-hop stripping, `X-Forwarded-*`, configured add/remove. |
| `proxy` | The handler tying it together; records metrics. |
| `health` | Background active health checks per upstream. |
| `server` | Proxy app (fallback) + admin app (health/metrics). |

## Load balancing

`Balancer` holds `Arc<Upstream>`s. Weighted round-robin expands indices by weight
and advances an atomic cursor; least-connections picks the eligible upstream with
the smallest active count. "Eligible" = active-health flag set AND the circuit
breaker allows. A returned `ActiveGuard` (RAII) increments the active counter and
decrements on drop, so least-connections reflects true in-flight load.

## Circuit breaker

Closed → (consecutive failures ≥ threshold) → Open → (after `open_secs`) →
HalfOpen → success ⇒ Closed, failure ⇒ Open. While Open, `allow()` returns false
so the struggling upstream is skipped fast. 5xx responses and connection/timeout
errors both count as failures (passive detection). The `gw_circuit_breaker_state`
gauge exposes the state.

## Retries

Bounded by `retries + 1` attempts. A failed attempt excludes that upstream and
re-picks. Retries apply to idempotent methods (GET/HEAD/PUT/DELETE/OPTIONS/TRACE)
or pure connection errors — never replaying a non-idempotent body against a new
upstream after a server error.

## Concurrency & blocking

Everything is async on Tokio; the upstream client (`reqwest`) is fully async with
pooled connections. No blocking calls on the request path. Shared state
(balancers, breakers, caches, limiters) uses atomics and short-held `std::Mutex`
sections — never held across `.await`.

## Caching & streaming

For cacheable GETs (route has a cache, `200`, response not `no-store/private`),
the body is buffered so it can be both stored and returned. All other responses
**stream** from upstream to client via `Body::from_stream`. Request bodies are
buffered up to `max_body_bytes` (a `413` otherwise).

## Servers & shutdown

Two Axum apps: the proxy (served with `ConnectInfo` for per-IP limiting) and a
separate admin app for health/metrics, so probes never collide with proxied
paths. Both use `with_graceful_shutdown`; a `CancellationToken` also stops the
health-check tasks on SIGINT/SIGTERM.
