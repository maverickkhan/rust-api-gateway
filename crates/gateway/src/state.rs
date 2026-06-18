//! Shared gateway runtime state assembled from configuration.

use crate::balancer::Balancer;
use crate::cache::ResponseCache;
use crate::ratelimit::{InMemoryLimiter, RateLimiter, RedisLimiter};
use gw_core::{GatewayConfig, RouteConfig};
use gw_telemetry::Metrics;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Per-route runtime: its config, load balancer and optional response cache.
pub struct RouteRuntime {
    pub config: RouteConfig,
    pub balancer: Balancer,
    pub cache: Option<ResponseCache>,
}

impl RouteRuntime {
    fn from_config(route: &RouteConfig) -> Self {
        let cache = route
            .cache
            .as_ref()
            .map(|c| ResponseCache::new(Duration::from_secs(c.ttl_secs), c.max_entries));
        Self {
            config: route.clone(),
            balancer: Balancer::from_route(route),
            cache,
        }
    }
}

/// Cloneable shared state for handlers (everything behind `Arc`).
#[derive(Clone)]
pub struct GatewayState {
    pub config: Arc<GatewayConfig>,
    pub routes: Arc<HashMap<String, Arc<RouteRuntime>>>,
    pub client: reqwest::Client,
    pub limiter: Arc<dyn RateLimiter>,
    pub metrics: Metrics,
}

impl GatewayState {
    /// Build runtime state. If a Redis URL is configured it is used for
    /// distributed rate limiting; otherwise (or on connection failure) an
    /// in-memory limiter is used.
    pub async fn build(config: GatewayConfig, metrics: Metrics) -> anyhow::Result<Self> {
        let mut routes = HashMap::new();
        for r in &config.routes {
            routes.insert(r.name.clone(), Arc::new(RouteRuntime::from_config(r)));
        }

        let client = reqwest::Client::builder()
            .pool_max_idle_per_host(32)
            .build()?;

        let limiter: Arc<dyn RateLimiter> = match &config.server.redis_url {
            Some(url) => match RedisLimiter::connect(url).await {
                Ok(l) => {
                    tracing::info!(%url, "using Redis-backed rate limiting");
                    Arc::new(l)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Redis unavailable; falling back to in-memory rate limiting");
                    Arc::new(InMemoryLimiter::new())
                }
            },
            None => Arc::new(InMemoryLimiter::new()),
        };

        Ok(Self {
            config: Arc::new(config),
            routes: Arc::new(routes),
            client,
            limiter,
            metrics,
        })
    }

    pub fn route(&self, name: &str) -> Option<Arc<RouteRuntime>> {
        self.routes.get(name).cloned()
    }
}
