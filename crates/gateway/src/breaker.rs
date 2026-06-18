//! Per-upstream circuit breaker.
//!
//! Closed → (failures reach threshold) → Open → (after `open` elapses) →
//! HalfOpen → (success) → Closed, or (failure) → Open again. While Open, calls
//! are rejected fast so a struggling upstream isn't hammered.

use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    Closed,
    Open,
    HalfOpen,
}

impl BreakerState {
    /// Numeric encoding for the Prometheus gauge (0=closed,1=half,2=open).
    pub fn code(&self) -> i64 {
        match self {
            BreakerState::Closed => 0,
            BreakerState::HalfOpen => 1,
            BreakerState::Open => 2,
        }
    }
}

#[derive(Debug)]
struct Inner {
    state: BreakerState,
    consecutive_failures: u32,
    open_until: Option<Instant>,
}

#[derive(Debug)]
pub struct CircuitBreaker {
    threshold: u32,
    open_for: Duration,
    inner: Mutex<Inner>,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, open_secs: u64) -> Self {
        Self {
            threshold: failure_threshold.max(1),
            open_for: Duration::from_secs(open_secs.max(1)),
            inner: Mutex::new(Inner {
                state: BreakerState::Closed,
                consecutive_failures: 0,
                open_until: None,
            }),
        }
    }

    /// Whether a request may proceed. Transitions Open→HalfOpen when the
    /// cooldown has elapsed (allowing a single trial).
    pub fn allow(&self) -> bool {
        self.allow_at(Instant::now())
    }

    fn allow_at(&self, now: Instant) -> bool {
        let mut inner = self.inner.lock().expect("breaker mutex");
        match inner.state {
            BreakerState::Closed | BreakerState::HalfOpen => true,
            BreakerState::Open => {
                if inner.open_until.map(|u| now >= u).unwrap_or(true) {
                    inner.state = BreakerState::HalfOpen;
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn record_success(&self) {
        let mut inner = self.inner.lock().expect("breaker mutex");
        inner.state = BreakerState::Closed;
        inner.consecutive_failures = 0;
        inner.open_until = None;
    }

    pub fn record_failure(&self) {
        self.record_failure_at(Instant::now());
    }

    fn record_failure_at(&self, now: Instant) {
        let mut inner = self.inner.lock().expect("breaker mutex");
        inner.consecutive_failures += 1;
        // A failed half-open trial, or crossing the threshold, opens the breaker.
        if inner.state == BreakerState::HalfOpen || inner.consecutive_failures >= self.threshold {
            inner.state = BreakerState::Open;
            inner.open_until = Some(now + self.open_for);
        }
    }

    pub fn state(&self) -> BreakerState {
        self.inner.lock().expect("breaker mutex").state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_after_threshold() {
        let cb = CircuitBreaker::new(3, 10);
        assert!(cb.allow());
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), BreakerState::Closed);
        cb.record_failure(); // 3rd → open
        assert_eq!(cb.state(), BreakerState::Open);
        assert!(!cb.allow());
    }

    #[test]
    fn half_open_after_cooldown_then_closes_on_success() {
        let cb = CircuitBreaker::new(1, 10);
        cb.record_failure_at(Instant::now());
        assert_eq!(cb.state(), BreakerState::Open);
        // Before cooldown: rejected.
        let t0 = Instant::now();
        assert!(!cb.allow_at(t0));
        // After cooldown: half-open trial allowed.
        let later = t0 + Duration::from_secs(11);
        assert!(cb.allow_at(later));
        assert_eq!(cb.state(), BreakerState::HalfOpen);
        cb.record_success();
        assert_eq!(cb.state(), BreakerState::Closed);
    }

    #[test]
    fn half_open_failure_reopens() {
        let cb = CircuitBreaker::new(1, 10);
        cb.record_failure_at(Instant::now());
        let later = Instant::now() + Duration::from_secs(11);
        assert!(cb.allow_at(later));
        cb.record_failure_at(later);
        assert_eq!(cb.state(), BreakerState::Open);
    }
}
