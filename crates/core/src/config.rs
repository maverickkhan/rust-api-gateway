//! Strongly typed gateway configuration, loadable from YAML with sensible
//! defaults and validated before use.

use crate::error::ConfigError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level gateway configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Address the proxy listens on.
    #[serde(default = "default_bind")]
    pub bind_addr: String,
    /// Address for health/metrics (kept off the proxy port).
    #[serde(default = "default_admin_bind")]
    pub admin_bind_addr: String,
    /// Max request body the gateway will buffer/forward.
    #[serde(default = "default_max_body")]
    pub max_body_bytes: usize,
    /// Default upstream timeout when a route doesn't set one.
    #[serde(default = "default_timeout_ms")]
    pub default_timeout_ms: u64,
    /// Optional Redis URL enabling distributed rate limiting.
    #[serde(default)]
    pub redis_url: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: default_bind(),
            admin_bind_addr: default_admin_bind(),
            max_body_bytes: default_max_body(),
            default_timeout_ms: default_timeout_ms(),
            redis_url: None,
        }
    }
}

fn default_bind() -> String {
    "0.0.0.0:8080".into()
}
fn default_admin_bind() -> String {
    "0.0.0.0:9090".into()
}
fn default_max_body() -> usize {
    8 * 1024 * 1024
}
fn default_timeout_ms() -> u64 {
    30_000
}

/// A routing rule and its behaviour.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteConfig {
    pub name: String,
    #[serde(default)]
    pub matches: MatchConfig,
    pub upstreams: Vec<UpstreamConfig>,
    #[serde(default)]
    pub strategy: Strategy,
    /// Strip the matched `path_prefix` before forwarding.
    #[serde(default)]
    pub strip_path_prefix: bool,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Retries on eligible failures (idempotent methods / connection errors).
    #[serde(default)]
    pub retries: u32,
    #[serde(default)]
    pub auth: AuthPolicy,
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,
    #[serde(default)]
    pub cache: Option<CacheConfig>,
    #[serde(default)]
    pub circuit_breaker: CircuitBreakerConfig,
    #[serde(default)]
    pub health_check: Option<HealthCheckConfig>,
    #[serde(default)]
    pub request_headers: HeaderTransforms,
    #[serde(default)]
    pub response_headers: HeaderTransforms,
}

/// What a request must look like to match this route.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchConfig {
    /// Required `Host` header (exact). `None` matches any host.
    #[serde(default)]
    pub host: Option<String>,
    /// Path prefix that must match. Defaults to `/`.
    #[serde(default = "default_prefix")]
    pub path_prefix: String,
}

impl Default for MatchConfig {
    fn default() -> Self {
        Self {
            host: None,
            path_prefix: default_prefix(),
        }
    }
}

fn default_prefix() -> String {
    "/".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamConfig {
    /// Base URL, e.g. `http://127.0.0.1:9001`.
    pub url: String,
    /// Relative weight for weighted round-robin (default 1).
    #[serde(default = "default_weight")]
    pub weight: u32,
}

fn default_weight() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    #[default]
    RoundRobin,
    LeastConnections,
}

/// Per-route authentication policy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthPolicy {
    #[default]
    None,
    /// Accept a static set of API keys (sent via `X-API-Key` / `Authorization`).
    ApiKey { keys: Vec<String> },
    /// Validate an HS256 JWT bearer token.
    Jwt {
        secret: String,
        #[serde(default)]
        required_claims: HashMap<String, String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    #[serde(default)]
    pub scope: RateScope,
    /// Requests allowed per `window_secs`.
    pub limit: u32,
    #[serde(default = "default_window")]
    pub window_secs: u64,
}

fn default_window() -> u64 {
    60
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateScope {
    #[default]
    Ip,
    ApiKey,
    Route,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    pub ttl_secs: u64,
    /// Max cached entries for this route (LRU-ish eviction by capacity).
    #[serde(default = "default_cache_cap")]
    pub max_entries: usize,
}

fn default_cache_cap() -> usize {
    1024
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// Consecutive failures before the breaker opens.
    #[serde(default = "default_cb_threshold")]
    pub failure_threshold: u32,
    /// How long the breaker stays open before a half-open trial.
    #[serde(default = "default_cb_open_secs")]
    pub open_secs: u64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: default_cb_threshold(),
            open_secs: default_cb_open_secs(),
        }
    }
}

fn default_cb_threshold() -> u32 {
    5
}
fn default_cb_open_secs() -> u64 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckConfig {
    pub path: String,
    #[serde(default = "default_hc_interval")]
    pub interval_secs: u64,
    #[serde(default = "default_hc_timeout")]
    pub timeout_ms: u64,
}

fn default_hc_interval() -> u64 {
    5
}
fn default_hc_timeout() -> u64 {
    2000
}

/// Header add/remove transforms.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HeaderTransforms {
    /// Headers to set (overwriting).
    #[serde(default)]
    pub add: HashMap<String, String>,
    /// Header names to remove.
    #[serde(default)]
    pub remove: Vec<String>,
}

impl GatewayConfig {
    /// Parse from a YAML string and validate.
    pub fn from_yaml(yaml: &str) -> Result<Self, ConfigError> {
        let cfg: GatewayConfig =
            serde_yaml::from_str(yaml).map_err(|e| ConfigError::Parse(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Load and validate from a YAML file path.
    pub fn from_file(path: &str) -> Result<Self, ConfigError> {
        let text =
            std::fs::read_to_string(path).map_err(|e| ConfigError::Io(format!("{path}: {e}")))?;
        Self::from_yaml(&text)
    }

    /// Validate structural invariants.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.routes.is_empty() {
            return Err(ConfigError::Invalid("no routes configured".into()));
        }
        let mut names = std::collections::HashSet::new();
        for r in &self.routes {
            if !names.insert(&r.name) {
                return Err(ConfigError::Invalid(format!(
                    "duplicate route name: {}",
                    r.name
                )));
            }
            if r.upstreams.is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "route '{}' has no upstreams",
                    r.name
                )));
            }
            for u in &r.upstreams {
                if !(u.url.starts_with("http://") || u.url.starts_with("https://")) {
                    return Err(ConfigError::Invalid(format!(
                        "route '{}' upstream url must start with http:// or https://: {}",
                        r.name, u.url
                    )));
                }
                if u.weight == 0 {
                    return Err(ConfigError::Invalid(format!(
                        "route '{}' upstream weight must be >= 1",
                        r.name
                    )));
                }
            }
            if !r.matches.path_prefix.starts_with('/') {
                return Err(ConfigError::Invalid(format!(
                    "route '{}' path_prefix must start with '/'",
                    r.name
                )));
            }
            if let AuthPolicy::ApiKey { keys } = &r.auth {
                if keys.is_empty() {
                    return Err(ConfigError::Invalid(format!(
                        "route '{}' api_key auth has no keys",
                        r.name
                    )));
                }
            }
        }
        Ok(())
    }
}
