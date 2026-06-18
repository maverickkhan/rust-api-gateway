# Roadmap

## Implemented

- [x] Host + path-prefix routing (longest-prefix wins), config validation
- [x] Reverse proxy: method/header/query/body forwarding, hop-by-hop stripping, `X-Forwarded-*`, response streaming, body-size limit, upstream timeouts
- [x] Load balancing: weighted round-robin and least-connections, active-request tracking
- [x] Resilience: active health checks, passive failure detection, circuit breaker, retries, timeouts, graceful shutdown
- [x] Auth: API keys and HS256 JWT, per-route policy
- [x] Rate limiting: per-IP / per-key / per-route, in-memory and Redis-backed, standard headers
- [x] Header transforms (add/remove) and correlation IDs
- [x] Response caching: TTL, capacity bound, `Cache-Control` bypass, hit/miss metrics
- [x] Observability: request/latency/error/cache/breaker metrics, structured logs
- [x] Docker, Compose (gateway + echo upstreams + Redis + Prometheus), CI, benchmark script

## Near-term

- [ ] **WebSocket proxying** (HTTP upgrade passthrough)
- [ ] **Config hot reload** (validate-then-atomic-swap, draining)
- [ ] Native **TLS termination** with Rustls (today: terminate upstream)
- [ ] Sliding-window rate limiting (current: fixed window)

## Medium-term

- [ ] Streaming request bodies upstream (today: bounded buffering)
- [ ] OpenTelemetry trace export across gateway → upstream
- [ ] Per-route concurrency limits / bulkheads
- [ ] Response compression / decompression negotiation

## Longer-term

- [ ] gRPC / HTTP2 upstream support
- [ ] A control-plane API + dashboard for live config
- [ ] Pluggable middleware (WASM filters)
