//! Load balancing over a route's upstreams, with health, circuit breakers and
//! active-request tracking.

use crate::breaker::CircuitBreaker;
use gw_core::{RouteConfig, Strategy};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;

/// Runtime state for a single upstream.
pub struct Upstream {
    pub url: String,
    pub weight: u32,
    /// Active in-flight requests (for least-connections).
    pub active: AtomicI64,
    /// Active-health-check status.
    pub healthy: AtomicBool,
    pub breaker: CircuitBreaker,
}

impl Upstream {
    pub fn active(&self) -> i64 {
        self.active.load(Ordering::Relaxed)
    }
    pub fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }
    pub fn set_healthy(&self, v: bool) {
        self.healthy.store(v, Ordering::Relaxed);
    }
    /// Eligible = passive health (breaker closed/half-open) AND active health.
    pub fn eligible(&self) -> bool {
        self.is_healthy() && self.breaker.allow()
    }
}

/// A RAII guard that decrements an upstream's active count on drop.
pub struct ActiveGuard {
    upstream: Arc<Upstream>,
}

impl ActiveGuard {
    fn new(upstream: Arc<Upstream>) -> Self {
        upstream.active.fetch_add(1, Ordering::Relaxed);
        Self { upstream }
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.upstream.active.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Per-route balancer.
pub struct Balancer {
    upstreams: Vec<Arc<Upstream>>,
    strategy: Strategy,
    /// Weighted round-robin selection order (indices expanded by weight).
    rr_order: Vec<usize>,
    rr_cursor: AtomicUsize,
}

impl Balancer {
    pub fn from_route(route: &RouteConfig) -> Self {
        let upstreams: Vec<Arc<Upstream>> = route
            .upstreams
            .iter()
            .map(|u| {
                Arc::new(Upstream {
                    url: u.url.trim_end_matches('/').to_string(),
                    weight: u.weight.max(1),
                    active: AtomicI64::new(0),
                    healthy: AtomicBool::new(true),
                    breaker: CircuitBreaker::new(
                        route.circuit_breaker.failure_threshold,
                        route.circuit_breaker.open_secs,
                    ),
                })
            })
            .collect();

        // Expand indices by weight for weighted round-robin.
        let mut rr_order = Vec::new();
        for (i, u) in upstreams.iter().enumerate() {
            for _ in 0..u.weight {
                rr_order.push(i);
            }
        }
        if rr_order.is_empty() {
            rr_order.push(0);
        }

        Self {
            upstreams,
            strategy: route.strategy,
            rr_order,
            rr_cursor: AtomicUsize::new(0),
        }
    }

    pub fn upstreams(&self) -> &[Arc<Upstream>] {
        &self.upstreams
    }

    /// Pick an eligible upstream by the configured strategy. `exclude` lets a
    /// retry avoid an upstream that just failed. Returns the upstream and an
    /// active-request guard.
    pub fn pick(&self, exclude: &[usize]) -> Option<(usize, Arc<Upstream>, ActiveGuard)> {
        let idx = match self.strategy {
            Strategy::RoundRobin => self.pick_round_robin(exclude),
            Strategy::LeastConnections => self.pick_least_conn(exclude),
        }?;
        let up = self.upstreams[idx].clone();
        let guard = ActiveGuard::new(up.clone());
        Some((idx, up, guard))
    }

    fn pick_round_robin(&self, exclude: &[usize]) -> Option<usize> {
        let n = self.rr_order.len();
        for _ in 0..n {
            let cursor = self.rr_cursor.fetch_add(1, Ordering::Relaxed);
            let idx = self.rr_order[cursor % n];
            if !exclude.contains(&idx) && self.upstreams[idx].eligible() {
                return Some(idx);
            }
        }
        // Fall back to any eligible upstream not excluded.
        self.first_eligible(exclude)
    }

    fn pick_least_conn(&self, exclude: &[usize]) -> Option<usize> {
        self.upstreams
            .iter()
            .enumerate()
            .filter(|(i, u)| !exclude.contains(i) && u.eligible())
            .min_by_key(|(_, u)| u.active())
            .map(|(i, _)| i)
    }

    fn first_eligible(&self, exclude: &[usize]) -> Option<usize> {
        self.upstreams
            .iter()
            .enumerate()
            .find(|(i, u)| !exclude.contains(i) && u.eligible())
            .map(|(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gw_core::GatewayConfig;

    fn route(yaml: &str) -> RouteConfig {
        GatewayConfig::from_yaml(yaml).unwrap().routes.remove(0)
    }

    const RR: &str = r#"
routes:
  - name: r
    strategy: round_robin
    upstreams:
      - { url: "http://a", weight: 1 }
      - { url: "http://b", weight: 1 }
"#;

    #[test]
    fn round_robin_alternates() {
        let b = Balancer::from_route(&route(RR));
        let (i0, _, g0) = b.pick(&[]).unwrap();
        let (i1, _, _g1) = b.pick(&[]).unwrap();
        drop(g0);
        assert_ne!(i0, i1, "should alternate between the two upstreams");
    }

    #[test]
    fn weighted_round_robin_respects_weight() {
        let yaml = r#"
routes:
  - name: r
    strategy: round_robin
    upstreams:
      - { url: "http://a", weight: 3 }
      - { url: "http://b", weight: 1 }
"#;
        let b = Balancer::from_route(&route(yaml));
        let mut counts = [0, 0];
        let mut guards = Vec::new();
        for _ in 0..8 {
            let (i, _, g) = b.pick(&[]).unwrap();
            counts[i] += 1;
            guards.push(g);
        }
        // 8 picks over weights 3:1 → 6:2.
        assert_eq!(counts[0], 6);
        assert_eq!(counts[1], 2);
    }

    #[test]
    fn least_connections_prefers_idle() {
        let yaml = r#"
routes:
  - name: r
    strategy: least_connections
    upstreams:
      - { url: "http://a" }
      - { url: "http://b" }
"#;
        let b = Balancer::from_route(&route(yaml));
        // Hold a connection on whichever is picked first; next pick must differ.
        let (i0, _, _g0) = b.pick(&[]).unwrap();
        let (i1, _, _g1) = b.pick(&[]).unwrap();
        assert_ne!(i0, i1);
    }

    #[test]
    fn unhealthy_upstreams_are_skipped() {
        let b = Balancer::from_route(&route(RR));
        b.upstreams()[0].set_healthy(false);
        for _ in 0..5 {
            let (i, _, _g) = b.pick(&[]).unwrap();
            assert_eq!(i, 1, "only the healthy upstream should be picked");
        }
    }
}
