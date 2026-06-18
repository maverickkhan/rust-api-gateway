//! Gateway binary: load config, build state, run proxy + admin servers with
//! active health checks and graceful shutdown.

use std::net::SocketAddr;

use anyhow::Context;
use gw_core::GatewayConfig;
use gw_gateway::{health, server, GatewayState};
use gw_telemetry::Metrics;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let json_logs = std::env::var("LOG_JSON")
        .map(|v| v == "true")
        .unwrap_or(false);
    gw_telemetry::init_tracing("gw-gateway", json_logs);

    let config_path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("GATEWAY_CONFIG").ok())
        .unwrap_or_else(|| "config/gateway.yaml".to_string());

    let config = GatewayConfig::from_file(&config_path)
        .with_context(|| format!("loading config from {config_path}"))?;
    tracing::info!(
        routes = config.routes.len(),
        config = %config_path,
        "configuration loaded and validated"
    );

    let proxy_addr: SocketAddr = config.server.bind_addr.parse().context("bind_addr")?;
    let admin_addr: SocketAddr = config
        .server
        .admin_bind_addr
        .parse()
        .context("admin_bind_addr")?;

    let metrics = Metrics::new();
    let state = GatewayState::build(config, metrics).await?;

    let shutdown = CancellationToken::new();

    // Active health checks.
    let health_task = tokio::spawn(health::run(state.clone(), shutdown.clone()));

    // Trigger shutdown on signal.
    let sig_token = shutdown.clone();
    tokio::spawn(async move {
        wait_for_signal().await;
        tracing::info!("shutdown signal received");
        sig_token.cancel();
    });

    // Run both servers; each completes on shutdown.
    let admin_state = state.clone();
    let admin_shutdown = shutdown.clone();
    let admin = tokio::spawn(async move {
        server::serve_admin(admin_state, admin_addr, async move {
            admin_shutdown.cancelled().await
        })
        .await
    });

    let proxy_shutdown = shutdown.clone();
    server::serve_proxy(state, proxy_addr, async move {
        proxy_shutdown.cancelled().await
    })
    .await?;

    let _ = admin.await;
    let _ = health_task.await;
    tracing::info!("gateway stopped");
    Ok(())
}

#[cfg(unix)]
async fn wait_for_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("SIGINT handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}

#[cfg(not(unix))]
async fn wait_for_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
