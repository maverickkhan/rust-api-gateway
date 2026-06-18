//! `gw-gateway` — the gateway runtime.
//!
//! Modules:
//! * [`balancer`] — load balancing (round-robin / least-connections) with
//!   health, weights and circuit breakers.
//! * [`breaker`] — per-upstream circuit breaker.
//! * [`cache`] — in-memory TTL response cache.
//! * [`ratelimit`] — in-memory + Redis rate limiters.
//! * [`auth`] — API-key and JWT authentication.
//! * [`transform`] — header forwarding/stripping/transforms.
//! * [`proxy`] — the request handler.
//! * [`health`] — active upstream health checks.
//! * [`server`] — proxy + admin app assembly.

pub mod auth;
pub mod balancer;
pub mod breaker;
pub mod cache;
pub mod health;
pub mod proxy;
pub mod ratelimit;
pub mod server;
pub mod state;
pub mod transform;

pub use state::GatewayState;
