# syntax=docker/dockerfile:1

# ---- Builder ----------------------------------------------------------------
FROM rust:1-bookworm AS builder
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake clang \
    && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release -p gw-gateway

# ---- Runtime ----------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 10001 -m app
COPY --from=builder /app/target/release/gw-gateway /usr/local/bin/gw-gateway
COPY config /app/config
USER app
WORKDIR /app
EXPOSE 8080 9090
ENV GATEWAY_CONFIG=/app/config/gateway.yaml
CMD ["gw-gateway"]
