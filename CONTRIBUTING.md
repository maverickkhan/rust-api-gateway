# Contributing

## Development setup

```bash
rustup toolchain install stable
cargo run -p gw-gateway -- config/gateway.yaml   # run locally
make ci                                          # fmt-check + clippy + test
```

Integration tests use in-process mock upstreams, so no external services are
needed to run the suite.

## Before opening a PR

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Guidelines:
- Keep `gw-core` pure (config + matching only; no I/O, no async runtime).
- Never hold a `std::Mutex` across an `.await`.
- New proxy behavior needs an integration test driving the real proxy.
- Don't replay non-idempotent request bodies on retry.

## Commit style

Conventional Commits, one logical change per commit. Review `git diff` and
staged files; confirm no secrets (config secrets, keys) are committed.

## Adding a feature flag to a route

Extend `gw_core::config` with a serde-defaulted field, wire it into
`RouteRuntime`/`proxy`, and add both a config-validation check and a test.
