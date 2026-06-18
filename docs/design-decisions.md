# Design decisions

## 1. Config-driven routing table, single fallback handler

**Decision.** One Axum fallback handler proxies all methods/paths; routes are
data in validated YAML.

**Why.** Routing as data (not code) is easy to reason about, validate, and later
hot-reload. Adding a route is a config edit, not a redeploy of handler code.

## 2. `reqwest` as the upstream client

**Decision.** Use `reqwest` (async, rustls, pooled) for upstream calls.

**Alternatives.** Raw `hyper-util` client with manual body plumbing.

**Why.** `reqwest` gives connection pooling, per-request timeouts, rustls and
response streaming out of the box, which is most of what a proxy needs. The cost
is that we buffer request bodies (bounded) rather than streaming them upstream —
an accepted, documented trade-off; response bodies still stream. A raw hyper
client would allow bidirectional streaming at significantly more code.

## 3. Circuit breaker + retries as the resilience core

**Decision.** Per-upstream circuit breaker with passive (5xx + errors) detection,
plus bounded retries over distinct upstreams for eligible requests.

**Why.** These two primitives cover the common failure modes: a single bad
instance (retry around it) and a sustained outage (breaker sheds load fast
instead of piling requests onto a dying upstream).

## 4. `RateLimiter` trait, in-memory + Redis

**Decision.** Abstract rate limiting behind a trait; pick in-memory or Redis by
config (`server.redis_url`).

**Why.** In-memory is perfect for a single instance and for tests (no
infrastructure). Redis (`INCR`/`EXPIRE` fixed window) makes limits **global**
across many gateway instances. The proxy code is identical either way. Redis
failures fail **open** (allow) and log, so a Redis blip can't take down traffic.

## 5. Separate admin port for health/metrics

**Decision.** Serve `/healthz`, `/readyz`, `/metrics` on a second port.

**Why.** The proxy's fallback handler matches *everything*, so health/metrics
can't live on the proxy port without colliding with a `/` route. A separate admin
listener also lets you firewall it off from public traffic.

## 6. Longest-prefix route matching

**Decision.** Among host-matching routes, the longest matching path prefix wins;
host-specific beats host-agnostic on ties.

**Why.** Predictable regardless of config order: `/api/users` beats `/api` beats
`/`, and matching is at segment boundaries so `/apixyz` doesn't match `/api`.

## 7. What's deliberately deferred (and documented, not stubbed)

- **WebSocket proxying** — meaningful extra machinery (upgrade handling, a WS
  client); HTTP is the 95% case. On the roadmap.
- **Hot reload** — the config model supports it (validate then swap), but doing
  it safely (draining, atomic swap) is its own feature. On the roadmap; restart
  for now.
- **TLS termination** — expected from an upstream LB today; native Rustls
  termination is on the roadmap.

These are called out honestly in the README and ROADMAP rather than left as
half-working code.
