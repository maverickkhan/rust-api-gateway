//! Rate limiting: a trait with an in-memory fixed-window implementation and a
//! Redis-backed distributed implementation.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The outcome of a rate-limit check, suitable for response headers.
#[derive(Debug, Clone, Copy)]
pub struct RateDecision {
    pub allowed: bool,
    pub limit: u32,
    pub remaining: u32,
    pub reset_secs: u64,
}

#[async_trait]
pub trait RateLimiter: Send + Sync {
    /// Count one request against `key` and decide if it is allowed.
    async fn check(&self, key: &str, limit: u32, window: Duration) -> RateDecision;
}

/// Fixed-window in-memory limiter (per process).
pub struct InMemoryLimiter {
    windows: Mutex<HashMap<String, (Instant, u32)>>,
}

impl InMemoryLimiter {
    pub fn new() -> Self {
        Self {
            windows: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RateLimiter for InMemoryLimiter {
    async fn check(&self, key: &str, limit: u32, window: Duration) -> RateDecision {
        let now = Instant::now();
        let mut windows = self.windows.lock().expect("ratelimit mutex");
        let entry = windows.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0) >= window {
            *entry = (now, 0);
        }
        entry.1 += 1;
        let used = entry.1;
        let reset = window
            .checked_sub(now.duration_since(entry.0))
            .unwrap_or_default()
            .as_secs();
        RateDecision {
            allowed: used <= limit,
            limit,
            remaining: limit.saturating_sub(used.min(limit)),
            reset_secs: reset,
        }
    }
}

/// Redis-backed fixed-window limiter (`INCR` + `EXPIRE`), shared across many
/// gateway instances.
pub struct RedisLimiter {
    conn: redis::aio::ConnectionManager,
}

impl RedisLimiter {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let client = redis::Client::open(url)?;
        let conn = redis::aio::ConnectionManager::new(client).await?;
        Ok(Self { conn })
    }
}

#[async_trait]
impl RateLimiter for RedisLimiter {
    async fn check(&self, key: &str, limit: u32, window: Duration) -> RateDecision {
        let redis_key = format!("gw:rl:{key}");
        let window_secs = window.as_secs().max(1) as i64;
        let mut conn = self.conn.clone();

        // INCR then, on first hit, set the window expiry.
        let count: i64 = match redis::cmd("INCR")
            .arg(&redis_key)
            .query_async(&mut conn)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "redis INCR failed; failing open");
                return RateDecision {
                    allowed: true,
                    limit,
                    remaining: limit,
                    reset_secs: window_secs as u64,
                };
            }
        };
        if count == 1 {
            let _: Result<(), _> = redis::cmd("EXPIRE")
                .arg(&redis_key)
                .arg(window_secs)
                .query_async(&mut conn)
                .await;
        }
        let ttl: i64 = redis::cmd("TTL")
            .arg(&redis_key)
            .query_async(&mut conn)
            .await
            .unwrap_or(window_secs);

        let used = count.max(0) as u32;
        RateDecision {
            allowed: used <= limit,
            limit,
            remaining: limit.saturating_sub(used.min(limit)),
            reset_secs: ttl.max(0) as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_blocks_over_limit() {
        let rl = InMemoryLimiter::new();
        let w = Duration::from_secs(60);
        for i in 1..=3 {
            let d = rl.check("k", 3, w).await;
            assert!(d.allowed, "request {i} should be allowed");
        }
        let d = rl.check("k", 3, w).await;
        assert!(
            !d.allowed,
            "4th request over a limit of 3 should be blocked"
        );
        assert_eq!(d.remaining, 0);
    }

    #[tokio::test]
    async fn in_memory_separates_keys() {
        let rl = InMemoryLimiter::new();
        let w = Duration::from_secs(60);
        assert!(rl.check("a", 1, w).await.allowed);
        assert!(!rl.check("a", 1, w).await.allowed);
        assert!(
            rl.check("b", 1, w).await.allowed,
            "different key has its own budget"
        );
    }
}
