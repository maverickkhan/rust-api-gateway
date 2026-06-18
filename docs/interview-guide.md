# Interview guide

## 30-second pitch

"An async API gateway in Rust on Tokio/Hyper/Axum. It routes by host and path,
load-balances (round-robin and least-connections), and adds resilience — active
health checks, a circuit breaker, retries and timeouts — plus API-key/JWT auth,
per-IP/key/route rate limiting (in-memory or Redis), response caching and full
Prometheus metrics. It's config-driven YAML and the proxy path is tested
end-to-end against real upstreams, including the circuit breaker actually
opening."

## Concepts demonstrated

- **Rust/async:** Tokio, Axum fallback handlers, `reqwest` streaming, RAII active
  guards, atomics + short-held mutexes (never across `.await`), graceful shutdown.
- **Networking:** reverse proxying, hop-by-hop vs end-to-end headers,
  `X-Forwarded-*`, request/response streaming.
- **Distributed systems:** load balancing, health checking, circuit breaking,
  retries/idempotency, distributed rate limiting via Redis.
- **Operability:** Prometheus metrics, correlation IDs, config validation.

## Five likely questions (with answers)

1. **How does the circuit breaker work and why bother?**
   Per upstream: consecutive failures (5xx or connection/timeout) open it; while
   open, that upstream is skipped fast; after a cooldown one half-open trial
   decides whether to close or reopen. It stops the gateway from piling requests
   onto a dying upstream (a key cause of cascading failure). Tested by
   `circuit_breaker_opens_after_failures`.

2. **When do you retry, and when is that dangerous?**
   Only for idempotent methods, or pure connection errors before a response.
   Replaying a non-idempotent body (POST) against another upstream after a 500
   could double-execute, so we don't. Retries also exclude the upstream that just
   failed.

3. **Least-connections — how do you track connections?**
   Each upstream has an atomic active counter incremented when picked and
   decremented by an RAII guard when the request finishes (even on error). The
   balancer picks the eligible upstream with the smallest count.

4. **In-memory vs Redis rate limiting?**
   In-memory is per instance — fine for one node and for tests. Redis
   (`INCR`+`EXPIRE`) makes the limit global across instances. Same trait, chosen
   by config. Redis errors fail open so a Redis outage doesn't drop traffic.

5. **What's the performance trade-off you made?**
   Request bodies are buffered (bounded) before forwarding while responses
   stream. Full bidirectional streaming would need a raw hyper client and more
   code; for typical API traffic (small request bodies) buffering is fine, and I
   documented it rather than hiding it.

## What I'd add next

WebSocket proxying, config hot reload (validate-then-swap), native Rustls TLS
termination, and sliding-window rate limits.
