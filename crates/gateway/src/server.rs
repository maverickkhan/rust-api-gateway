//! Server assembly: the proxy app (fallback → [`proxy::handle`]) and a separate
//! admin app for health and metrics.

use crate::proxy;
use crate::state::GatewayState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use std::net::SocketAddr;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

/// The proxy router — a single fallback handler over all methods/paths.
pub fn proxy_router(state: GatewayState) -> Router {
    let max_body = state.config.server.max_body_bytes;
    Router::new()
        .fallback(proxy::handle)
        .layer(RequestBodyLimitLayer::new(max_body))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// The admin router — health, readiness and Prometheus metrics.
pub fn admin_router(state: GatewayState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ready" }))
        .route("/metrics", get(metrics))
        .with_state(state)
}

async fn metrics(State(state): State<GatewayState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        state.metrics.render(),
    )
}

/// Serve the proxy app on `addr` with client-connect-info (for per-IP limits).
pub async fn serve_proxy(
    state: GatewayState,
    addr: SocketAddr,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let app = proxy_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "proxy listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await?;
    Ok(())
}

/// Serve the admin app on `addr`.
pub async fn serve_admin(
    state: GatewayState,
    addr: SocketAddr,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let app = admin_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "admin (health/metrics) listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}
