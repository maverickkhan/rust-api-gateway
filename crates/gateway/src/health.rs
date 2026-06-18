//! Active health checks: periodically probe each configured upstream and flip
//! its health flag, which the balancer consults when selecting.

use crate::state::GatewayState;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Spawn health-check loops for every route that configures one. Returns when
/// `shutdown` is triggered.
pub async fn run(state: GatewayState, shutdown: CancellationToken) {
    let mut handles = Vec::new();

    for runtime in state.routes.values() {
        let Some(hc) = runtime.config.health_check.clone() else {
            continue;
        };
        for upstream in runtime.balancer.upstreams() {
            let upstream = upstream.clone();
            let client = state.client.clone();
            let shutdown = shutdown.clone();
            let url = format!("{}{}", upstream.url, hc.path);
            let interval = Duration::from_secs(hc.interval_secs.max(1));
            let timeout = Duration::from_millis(hc.timeout_ms.max(100));

            handles.push(tokio::spawn(async move {
                let mut ticker = tokio::time::interval(interval);
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        _ = shutdown.cancelled() => break,
                        _ = ticker.tick() => {
                            let healthy = match client.get(&url).timeout(timeout).send().await {
                                Ok(r) => r.status().is_success(),
                                Err(_) => false,
                            };
                            if healthy != upstream.is_healthy() {
                                tracing::info!(upstream = %upstream.url, healthy, "upstream health changed");
                            }
                            upstream.set_healthy(healthy);
                        }
                    }
                }
            }));
        }
    }

    if handles.is_empty() {
        // Nothing to check; just wait for shutdown.
        shutdown.cancelled().await;
        return;
    }

    shutdown.cancelled().await;
    for h in handles {
        let _ = h.await;
    }
}
