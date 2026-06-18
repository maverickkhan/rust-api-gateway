//! End-to-end gateway tests: routing, header forwarding, load balancing,
//! retries, circuit breaking, rate limiting, auth, timeouts, body limits,
//! caching and metrics — all through the real proxy against mock upstreams.

use gw_integration_tests::{cfg, start_gateway, start_upstream};
use serde_json::Value;
use std::time::Duration;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

#[tokio::test]
async fn path_routing_strips_prefix() {
    let up = start_upstream("u1").await;
    let config = cfg(&format!(
        r#"
routes:
  - name: api
    matches: {{ path_prefix: "/api" }}
    strip_path_prefix: true
    upstreams: [{{ url: "{}" }}]
"#,
        up.url()
    ));
    let gw = start_gateway(config).await;

    let resp = client().get(gw.url("/api/widgets/7")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["id"], "u1");
    assert_eq!(body["path"], "/widgets/7", "prefix should be stripped");
}

#[tokio::test]
async fn host_routing_selects_upstream() {
    let a = start_upstream("hostA").await;
    let b = start_upstream("hostB").await;
    let config = cfg(&format!(
        r#"
routes:
  - name: a
    matches: {{ host: "a.example.com", path_prefix: "/" }}
    upstreams: [{{ url: "{}" }}]
  - name: b
    matches: {{ host: "b.example.com", path_prefix: "/" }}
    upstreams: [{{ url: "{}" }}]
"#,
        a.url(),
        b.url()
    ));
    let gw = start_gateway(config).await;

    let resp = client()
        .get(gw.url("/anything"))
        .header("host", "b.example.com")
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["id"], "hostB");
}

#[tokio::test]
async fn forwards_headers_and_adds_x_forwarded_for() {
    let up = start_upstream("u1").await;
    let config = cfg(&format!(
        r#"
routes:
  - name: r
    matches: {{ path_prefix: "/" }}
    upstreams: [{{ url: "{}" }}]
    request_headers:
      add: {{ x-gateway: "rust-gw" }}
"#,
        up.url()
    ));
    let gw = start_gateway(config).await;

    let resp = client()
        .get(gw.url("/x"))
        .header("x-test", "hello")
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let headers = &body["headers"];
    assert_eq!(headers["x-test"], "hello", "client header forwarded");
    assert_eq!(headers["x-gateway"], "rust-gw", "configured header added");
    assert!(headers.get("x-forwarded-for").is_some(), "XFF injected");
}

#[tokio::test]
async fn round_robin_distributes_across_upstreams() {
    let a = start_upstream("A").await;
    let b = start_upstream("B").await;
    let config = cfg(&format!(
        r#"
routes:
  - name: r
    strategy: round_robin
    matches: {{ path_prefix: "/" }}
    upstreams:
      - {{ url: "{}" }}
      - {{ url: "{}" }}
"#,
        a.url(),
        b.url()
    ));
    let gw = start_gateway(config).await;

    let mut seen_a = 0;
    let mut seen_b = 0;
    for _ in 0..4 {
        let body: Value = client()
            .get(gw.url("/p"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        match body["id"].as_str().unwrap() {
            "A" => seen_a += 1,
            "B" => seen_b += 1,
            other => panic!("unexpected id {other}"),
        }
    }
    assert_eq!(seen_a, 2);
    assert_eq!(seen_b, 2);
}

#[tokio::test]
async fn retries_past_a_dead_upstream() {
    let good = start_upstream("good").await;
    // First upstream refuses connections (nothing listens on port 1).
    let config = cfg(&format!(
        r#"
routes:
  - name: r
    strategy: round_robin
    retries: 2
    matches: {{ path_prefix: "/" }}
    upstreams:
      - {{ url: "http://127.0.0.1:1" }}
      - {{ url: "{}" }}
"#,
        good.url()
    ));
    let gw = start_gateway(config).await;

    for _ in 0..3 {
        let resp = client().get(gw.url("/p")).send().await.unwrap();
        assert_eq!(
            resp.status(),
            200,
            "retry should reach the healthy upstream"
        );
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["id"], "good");
    }
}

#[tokio::test]
async fn circuit_breaker_opens_after_failures() {
    let up = start_upstream("u").await;
    let config = cfg(&format!(
        r#"
routes:
  - name: r
    retries: 0
    matches: {{ path_prefix: "/" }}
    upstreams: [{{ url: "{}" }}]
    circuit_breaker: {{ failure_threshold: 3, open_secs: 10 }}
"#,
        up.url()
    ));
    let gw = start_gateway(config).await;

    let mut statuses = Vec::new();
    for _ in 0..5 {
        // Upstream returns 500 for this path.
        let s = client()
            .get(gw.url("/status/500"))
            .send()
            .await
            .unwrap()
            .status();
        statuses.push(s.as_u16());
    }
    // First three are upstream 500s; once the breaker opens, fast 503s.
    assert_eq!(&statuses[0..3], &[500, 500, 500]);
    assert_eq!(statuses[4], 503, "breaker should be open: {statuses:?}");
}

#[tokio::test]
async fn rate_limit_blocks_excess() {
    let up = start_upstream("u").await;
    let config = cfg(&format!(
        r#"
routes:
  - name: r
    matches: {{ path_prefix: "/" }}
    upstreams: [{{ url: "{}" }}]
    rate_limit: {{ scope: ip, limit: 2, window_secs: 60 }}
"#,
        up.url()
    ));
    let gw = start_gateway(config).await;
    let c = client();

    assert_eq!(c.get(gw.url("/a")).send().await.unwrap().status(), 200);
    assert_eq!(c.get(gw.url("/a")).send().await.unwrap().status(), 200);
    let third = c.get(gw.url("/a")).send().await.unwrap();
    assert_eq!(third.status(), 429);
    assert!(third.headers().get("x-ratelimit-limit").is_some());
}

#[tokio::test]
async fn api_key_auth_is_enforced() {
    let up = start_upstream("u").await;
    let config = cfg(&format!(
        r#"
routes:
  - name: r
    matches: {{ path_prefix: "/" }}
    auth: {{ type: api_key, keys: ["secret"] }}
    upstreams: [{{ url: "{}" }}]
"#,
        up.url()
    ));
    let gw = start_gateway(config).await;
    let c = client();

    assert_eq!(c.get(gw.url("/x")).send().await.unwrap().status(), 401);
    let ok = c
        .get(gw.url("/x"))
        .header("x-api-key", "secret")
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
}

#[tokio::test]
async fn upstream_timeout_returns_502() {
    let up = start_upstream("u").await;
    let config = cfg(&format!(
        r#"
routes:
  - name: r
    retries: 0
    timeout_ms: 200
    matches: {{ path_prefix: "/" }}
    upstreams: [{{ url: "{}" }}]
"#,
        up.url()
    ));
    let gw = start_gateway(config).await;

    // Upstream sleeps 1500ms; the 200ms gateway timeout fires first.
    let resp = client().get(gw.url("/slow?ms=1500")).send().await.unwrap();
    assert_eq!(resp.status(), 502);
}

#[tokio::test]
async fn oversized_body_is_rejected() {
    let up = start_upstream("u").await;
    let config = cfg(&format!(
        r#"
server: {{ max_body_bytes: 100 }}
routes:
  - name: r
    matches: {{ path_prefix: "/" }}
    upstreams: [{{ url: "{}" }}]
"#,
        up.url()
    ));
    let gw = start_gateway(config).await;

    let big = vec![b'x'; 1000];
    let resp = client().post(gw.url("/x")).body(big).send().await.unwrap();
    assert_eq!(resp.status(), 413);
}

#[tokio::test]
async fn get_responses_are_cached() {
    let up = start_upstream("u").await;
    let config = cfg(&format!(
        r#"
routes:
  - name: r
    matches: {{ path_prefix: "/" }}
    upstreams: [{{ url: "{}" }}]
    cache: {{ ttl_secs: 30 }}
"#,
        up.url()
    ));
    let gw = start_gateway(config).await;
    let c = client();

    let first = c.get(gw.url("/cacheable")).send().await.unwrap();
    assert_eq!(first.headers().get("x-cache").unwrap(), "MISS");
    let second = c.get(gw.url("/cacheable")).send().await.unwrap();
    assert_eq!(second.headers().get("x-cache").unwrap(), "HIT");

    // The upstream was only hit once.
    assert_eq!(up.hits(), 1, "second request should be served from cache");
}

#[tokio::test]
async fn metrics_are_recorded() {
    let up = start_upstream("u").await;
    let config = cfg(&format!(
        r#"
routes:
  - name: r
    matches: {{ path_prefix: "/" }}
    upstreams: [{{ url: "{}" }}]
"#,
        up.url()
    ));
    let gw = start_gateway(config).await;
    let _ = client().get(gw.url("/x")).send().await.unwrap();

    let rendered = gw.state.metrics.render();
    assert!(rendered.contains("gw_requests_total"));
    assert!(rendered.contains("gw_gateway_latency_seconds"));
}
