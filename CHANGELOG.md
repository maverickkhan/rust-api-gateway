# Changelog

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-06-18

### Added
- Cargo workspace: `gw-core`, `gw-telemetry`, `gw-gateway`,
  `gw-integration-tests`.
- YAML config model with validation and pure longest-prefix host/path routing.
- Reverse proxy: method/header/query/body forwarding, hop-by-hop stripping,
  `X-Forwarded-*`, response streaming, body-size limit, upstream timeouts.
- Load balancing (weighted round-robin, least-connections) with active-request
  tracking and per-upstream circuit breakers.
- Resilience: active health checks, passive failure detection, circuit breaker,
  bounded retries, graceful shutdown.
- Authentication: API keys and HS256 JWT, per-route policy.
- Rate limiting: per-IP / per-key / per-route; in-memory and Redis-backed;
  standard rate-limit headers.
- Header transforms, correlation IDs, and a TTL response cache with bypass and
  hit/miss metrics.
- Prometheus metrics, structured logs, separate admin (health/metrics) port.
- Docker, Docker Compose (gateway + echo upstreams + Redis + Prometheus), GitHub
  Actions CI (fmt, clippy, tests, release build, Docker build, cargo audit),
  Makefile, `deny.toml`, and a reproducible benchmark script.
- 25 unit tests and 12 integration tests (37 total), all passing, driving the
  real proxy against mock upstreams.
