//! A small in-memory TTL response cache for eligible GET responses.

use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A cached upstream response.
#[derive(Clone)]
pub struct CachedResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    expires_at: Instant,
}

/// Per-route response cache (capacity-bounded, TTL-expired).
pub struct ResponseCache {
    ttl: Duration,
    capacity: usize,
    map: Mutex<HashMap<String, CachedResponse>>,
}

impl ResponseCache {
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            ttl,
            capacity: capacity.max(1),
            map: Mutex::new(HashMap::new()),
        }
    }

    /// Build a cache key from the request's method, host and full path+query.
    pub fn key(method: &str, host: &str, path_and_query: &str) -> String {
        format!("{method} {host} {path_and_query}")
    }

    pub fn get(&self, key: &str) -> Option<CachedResponse> {
        let mut map = self.map.lock().expect("cache mutex");
        match map.get(key) {
            Some(entry) if entry.expires_at > Instant::now() => Some(entry.clone()),
            Some(_) => {
                map.remove(key);
                None
            }
            None => None,
        }
    }

    pub fn put(&self, key: String, status: u16, headers: Vec<(String, String)>, body: Bytes) {
        let mut map = self.map.lock().expect("cache mutex");
        if map.len() >= self.capacity && !map.contains_key(&key) {
            // Evict one expired entry, else an arbitrary one.
            let now = Instant::now();
            let victim = map
                .iter()
                .find(|(_, v)| v.expires_at <= now)
                .map(|(k, _)| k.clone())
                .or_else(|| map.keys().next().cloned());
            if let Some(v) = victim {
                map.remove(&v);
            }
        }
        map.insert(
            key,
            CachedResponse {
                status,
                headers,
                body,
                expires_at: Instant::now() + self.ttl,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_returns_within_ttl() {
        let c = ResponseCache::new(Duration::from_secs(60), 10);
        c.put("k".into(), 200, vec![], Bytes::from_static(b"hi"));
        let got = c.get("k").unwrap();
        assert_eq!(got.status, 200);
        assert_eq!(&got.body[..], b"hi");
    }

    #[test]
    fn expired_entries_are_dropped() {
        let c = ResponseCache::new(Duration::from_millis(0), 10);
        c.put("k".into(), 200, vec![], Bytes::from_static(b"hi"));
        std::thread::sleep(Duration::from_millis(5));
        assert!(c.get("k").is_none());
    }

    #[test]
    fn capacity_is_bounded() {
        let c = ResponseCache::new(Duration::from_secs(60), 2);
        c.put("a".into(), 200, vec![], Bytes::new());
        c.put("b".into(), 200, vec![], Bytes::new());
        c.put("c".into(), 200, vec![], Bytes::new());
        let present = ["a", "b", "c"]
            .iter()
            .filter(|k| c.get(k).is_some())
            .count();
        assert!(present <= 2, "cache must not exceed capacity");
    }
}
