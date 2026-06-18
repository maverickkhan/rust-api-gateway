//! Pure host + path-prefix route matching.
//!
//! Matching picks, among routes whose host constraint is satisfied, the one
//! with the **longest** matching path prefix. This makes specific routes
//! (`/api/v1/users`) win over general ones (`/`) regardless of config order.

use crate::config::{GatewayConfig, RouteConfig};

/// Index-based matcher over a config's routes.
pub struct RouteMatcher<'a> {
    config: &'a GatewayConfig,
}

impl<'a> RouteMatcher<'a> {
    pub fn new(config: &'a GatewayConfig) -> Self {
        Self { config }
    }

    /// Return the best-matching route for a host + path, if any.
    pub fn match_route(&self, host: Option<&str>, path: &str) -> Option<&'a RouteConfig> {
        let mut best: Option<&RouteConfig> = None;
        let mut best_len = 0usize;
        for route in &self.config.routes {
            // Host constraint (exact, case-insensitive, port-insensitive).
            if let Some(want) = &route.matches.host {
                let got = host.map(strip_port).unwrap_or("");
                if !got.eq_ignore_ascii_case(strip_port(want)) {
                    continue;
                }
            }
            let prefix = &route.matches.path_prefix;
            if path_matches(path, prefix) {
                // Prefer the longest prefix; on ties prefer a host-specific route.
                let len = prefix.len();
                let more_specific = len > best_len
                    || (len == best_len
                        && route.matches.host.is_some()
                        && best.is_some_and(|b| b.matches.host.is_none()));
                if best.is_none() || more_specific {
                    best = Some(route);
                    best_len = len;
                }
            }
        }
        best
    }
}

fn strip_port(host: &str) -> &str {
    host.split(':').next().unwrap_or(host)
}

/// A path matches a prefix when it is the prefix, or continues at a segment
/// boundary. `/` matches everything.
fn path_matches(path: &str, prefix: &str) -> bool {
    if prefix == "/" {
        return true;
    }
    let prefix = prefix.trim_end_matches('/');
    if path == prefix {
        return true;
    }
    path.starts_with(prefix) && path.as_bytes().get(prefix.len()) == Some(&b'/')
}

/// Compute the forwarded path after optionally stripping the matched prefix.
pub fn forward_path(path: &str, route: &RouteConfig) -> String {
    if !route.strip_path_prefix || route.matches.path_prefix == "/" {
        return path.to_string();
    }
    let prefix = route.matches.path_prefix.trim_end_matches('/');
    let stripped = path.strip_prefix(prefix).unwrap_or(path);
    if stripped.is_empty() {
        "/".to_string()
    } else {
        stripped.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GatewayConfig;

    fn cfg(yaml: &str) -> GatewayConfig {
        GatewayConfig::from_yaml(yaml).unwrap()
    }

    const YAML: &str = r#"
routes:
  - name: root
    matches: { path_prefix: "/" }
    upstreams: [{ url: "http://a" }]
  - name: api
    matches: { path_prefix: "/api" }
    upstreams: [{ url: "http://b" }]
  - name: users
    matches: { path_prefix: "/api/users" }
    upstreams: [{ url: "http://c" }]
  - name: hosted
    matches: { host: "admin.example.com", path_prefix: "/" }
    upstreams: [{ url: "http://d" }]
"#;

    #[test]
    fn longest_prefix_wins() {
        let c = cfg(YAML);
        let m = RouteMatcher::new(&c);
        assert_eq!(m.match_route(None, "/api/users/42").unwrap().name, "users");
        assert_eq!(m.match_route(None, "/api/orders").unwrap().name, "api");
        assert_eq!(m.match_route(None, "/other").unwrap().name, "root");
    }

    #[test]
    fn segment_boundary_required() {
        let c = cfg(YAML);
        let m = RouteMatcher::new(&c);
        // "/apixyz" must NOT match the "/api" prefix.
        assert_eq!(m.match_route(None, "/apixyz").unwrap().name, "root");
    }

    #[test]
    fn host_routing() {
        let c = cfg(YAML);
        let m = RouteMatcher::new(&c);
        assert_eq!(
            m.match_route(Some("admin.example.com"), "/anything")
                .unwrap()
                .name,
            "hosted"
        );
        assert_eq!(
            m.match_route(Some("admin.example.com:8080"), "/x")
                .unwrap()
                .name,
            "hosted"
        );
    }

    #[test]
    fn strip_prefix_rewrites_path() {
        let c = cfg(YAML);
        let route = c.routes.iter().find(|r| r.name == "api").unwrap();
        let mut r = route.clone();
        r.strip_path_prefix = true;
        assert_eq!(forward_path("/api/orders", &r), "/orders");
        assert_eq!(forward_path("/api", &r), "/");
    }
}
