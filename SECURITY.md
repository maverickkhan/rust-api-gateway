# Security policy

## Scope

The gateway is an edge component. It authenticates and rate-limits traffic before
it reaches upstreams, but it is one layer in a defense-in-depth posture.

## What it provides

- **Authentication:** per-route API-key and HS256 JWT validation (signature +
  expiry + required claims). Unauthorized requests get `401` before any upstream
  call.
- **Rate limiting:** per-IP / per-key / per-route, in-memory or Redis-backed, to
  blunt abuse and protect upstreams. Standard `X-RateLimit-*` / `Retry-After`
  headers.
- **Request hardening:** body-size limit (`413` over the cap), hop-by-hop header
  stripping, and `X-Forwarded-*` injection so upstreams see the real client.
- **Safe errors:** upstream internals are not leaked; errors are small JSON
  bodies with a correlation id.

## Secrets

- Auth secrets (API keys, JWT secret) live in the YAML config. **Keep config with
  real secrets out of Git**; inject via your secret manager / mounted file.
- `.env` is git-ignored; only `.env.example` (no secrets) is committed.
- The committed example config uses obvious placeholders (`change-me-in-prod`).

## Honest limitations

- **TLS:** the gateway currently terminates plain HTTP. Run it behind a TLS
  terminator (LB / mesh) for now; native Rustls termination is on the roadmap.
- **JWT:** HS256 (shared secret) only; RS256/JWKS rotation is not yet supported.
- **Rate limiting:** fixed-window (a burst at a window boundary can exceed the
  nominal rate briefly); the in-memory limiter is per instance.
- **Not audited:** no independent security audit has been performed.
- A misconfigured route (e.g. `auth: none` on a sensitive upstream) is only as
  safe as its config — validate your config in review.

## Reporting a vulnerability

Open a private GitHub security advisory or email the maintainer. Please do not
file public issues for security problems.
