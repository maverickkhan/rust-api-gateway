//! Tracing + Prometheus metrics for the gateway.

use once_cell::sync::OnceCell;
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry,
    TextEncoder,
};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

static TRACING: OnceCell<()> = OnceCell::new();

pub fn init_tracing(service: &str, json: bool) {
    TRACING.get_or_init(|| {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,hyper=warn,reqwest=warn"));
        let registry = tracing_subscriber::registry().with(filter);
        if json {
            registry
                .with(fmt::layer().json().with_current_span(true))
                .init();
        } else {
            registry.with(fmt::layer().compact()).init();
        }
        tracing::info!(service, "tracing initialized");
    });
}

/// All gateway Prometheus collectors.
#[derive(Clone)]
pub struct Metrics {
    pub registry: Registry,
    /// Requests by route, method and final status code.
    pub requests: IntCounterVec,
    /// Upstream call latency (seconds) by route.
    pub upstream_latency: HistogramVec,
    /// Total gateway-handling latency (seconds) by route.
    pub gateway_latency: HistogramVec,
    /// In-flight requests.
    pub active_connections: IntGauge,
    /// Rate-limit rejections by route.
    pub rate_limit_rejections: IntCounterVec,
    /// Auth rejections by route.
    pub auth_rejections: IntCounterVec,
    /// Upstream errors (connection/timeout/5xx) by route.
    pub upstream_errors: IntCounterVec,
    /// Cache hits/misses by route.
    pub cache_hits: IntCounterVec,
    pub cache_misses: IntCounterVec,
    /// Circuit-breaker state per route+upstream (0=closed,1=half,2=open).
    pub circuit_state: IntGaugeVec,
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new();
        let requests = IntCounterVec::new(
            Opts::new("gw_requests_total", "Requests handled"),
            &["route", "method", "status"],
        )
        .unwrap();
        let upstream_latency = HistogramVec::new(
            HistogramOpts::new("gw_upstream_latency_seconds", "Upstream call latency").buckets(
                vec![
                    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
                ],
            ),
            &["route"],
        )
        .unwrap();
        let gateway_latency = HistogramVec::new(
            HistogramOpts::new("gw_gateway_latency_seconds", "Total gateway latency").buckets(
                vec![
                    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
                ],
            ),
            &["route"],
        )
        .unwrap();
        let active_connections =
            IntGauge::with_opts(Opts::new("gw_active_connections", "In-flight requests")).unwrap();
        let rate_limit_rejections = IntCounterVec::new(
            Opts::new("gw_rate_limit_rejections_total", "Rate-limit rejections"),
            &["route"],
        )
        .unwrap();
        let auth_rejections = IntCounterVec::new(
            Opts::new("gw_auth_rejections_total", "Auth rejections"),
            &["route"],
        )
        .unwrap();
        let upstream_errors = IntCounterVec::new(
            Opts::new("gw_upstream_errors_total", "Upstream errors"),
            &["route"],
        )
        .unwrap();
        let cache_hits =
            IntCounterVec::new(Opts::new("gw_cache_hits_total", "Cache hits"), &["route"]).unwrap();
        let cache_misses = IntCounterVec::new(
            Opts::new("gw_cache_misses_total", "Cache misses"),
            &["route"],
        )
        .unwrap();
        let circuit_state = IntGaugeVec::new(
            Opts::new("gw_circuit_breaker_state", "0=closed 1=half 2=open"),
            &["route", "upstream"],
        )
        .unwrap();

        for c in [
            &requests,
            &rate_limit_rejections,
            &auth_rejections,
            &upstream_errors,
            &cache_hits,
            &cache_misses,
        ] {
            registry.register(Box::new(c.clone())).unwrap();
        }
        registry
            .register(Box::new(upstream_latency.clone()))
            .unwrap();
        registry
            .register(Box::new(gateway_latency.clone()))
            .unwrap();
        registry
            .register(Box::new(active_connections.clone()))
            .unwrap();
        registry.register(Box::new(circuit_state.clone())).unwrap();

        Self {
            registry,
            requests,
            upstream_latency,
            gateway_latency,
            active_connections,
            rate_limit_rejections,
            auth_rejections,
            upstream_errors,
            cache_hits,
            cache_misses,
            circuit_state,
        }
    }

    pub fn render(&self) -> String {
        let mut buf = Vec::new();
        let encoder = TextEncoder::new();
        if encoder.encode(&self.registry.gather(), &mut buf).is_err() {
            return String::new();
        }
        String::from_utf8(buf).unwrap_or_default()
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders() {
        let m = Metrics::new();
        m.requests.with_label_values(&["r", "GET", "200"]).inc();
        assert!(m.render().contains("gw_requests_total"));
    }
}
