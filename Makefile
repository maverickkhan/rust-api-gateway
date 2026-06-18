.DEFAULT_GOAL := help
SHELL := /bin/bash

.PHONY: help
help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS=":.*?## "}; {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'

.PHONY: fmt
fmt: ## Format
	cargo fmt --all

.PHONY: fmt-check
fmt-check: ## Check formatting
	cargo fmt --all -- --check

.PHONY: lint
lint: ## Clippy with warnings denied
	cargo clippy --workspace --all-targets --all-features -- -D warnings

.PHONY: build
build: ## Debug build
	cargo build --workspace

.PHONY: release
release: ## Release build of the gateway
	cargo build --release -p gw-gateway

.PHONY: test
test: ## Run all tests (no external services required)
	cargo test --workspace --all-features

.PHONY: audit
audit: ## Dependency security audit (requires cargo-audit)
	cargo audit

.PHONY: deny
deny: ## License/advisory checks (requires cargo-deny)
	cargo deny check

.PHONY: run
run: ## Run the gateway with the example config
	cargo run -p gw-gateway -- config/gateway.yaml

.PHONY: up
up: ## Bring up gateway + echo upstreams + Redis + Prometheus
	docker compose up --build

.PHONY: down
down: ## Tear down the compose stack
	docker compose down -v

.PHONY: bench
bench: ## Direct-vs-gateway throughput comparison (needs `oha`; see scripts/bench.sh)
	./scripts/bench.sh

.PHONY: ci
ci: fmt-check lint test ## What CI runs (minus Docker)
