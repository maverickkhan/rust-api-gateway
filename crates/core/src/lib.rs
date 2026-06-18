//! `gw-core` — configuration model and pure routing for the API gateway.
//!
//! Free of I/O and async: the config types (YAML-loadable, validated) and the
//! host/path [`router::RouteMatcher`] live here so they can be unit-tested
//! without a running server.

pub mod config;
pub mod error;
pub mod router;

pub use config::{
    AuthPolicy, CacheConfig, CircuitBreakerConfig, GatewayConfig, HeaderTransforms,
    HealthCheckConfig, MatchConfig, RateLimitConfig, RateScope, RouteConfig, ServerConfig,
    Strategy, UpstreamConfig,
};
pub use error::ConfigError;
pub use router::{forward_path, RouteMatcher};
