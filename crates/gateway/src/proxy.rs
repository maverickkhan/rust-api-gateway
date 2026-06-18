//! The core reverse-proxy request handler.

use crate::state::{GatewayState, RouteRuntime};
use crate::{auth, transform};
use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::response::{IntoResponse, Response};
use gw_core::{forward_path, RateScope, RouteMatcher};
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

const CORRELATION_HEADER: &str = "x-request-id";

/// Axum fallback handler: every request flows through here.
pub async fn handle(
    State(state): State<GatewayState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: axum::extract::Request,
) -> Response {
    state.metrics.active_connections.inc();
    let started = Instant::now();
    let response = route_and_proxy(&state, peer, req).await;
    state.metrics.active_connections.dec();
    let _ = started; // gateway latency is recorded per-route inside
    response
}

async fn route_and_proxy(
    state: &GatewayState,
    peer: SocketAddr,
    req: axum::extract::Request,
) -> Response {
    let started = Instant::now();
    let (parts, body) = req.into_parts();
    let method = parts.method.clone();
    let uri = parts.uri.clone();
    let headers = parts.headers;

    let host = headers
        .get(http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let client_ip = client_ip(&headers, peer);
    let correlation = headers
        .get(CORRELATION_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Match a route.
    let matcher = RouteMatcher::new(&state.config);
    let route_cfg = match matcher.match_route(Some(&host), uri.path()) {
        Some(r) => r,
        None => {
            state
                .metrics
                .requests
                .with_label_values(&["none", method.as_str(), "404"])
                .inc();
            return error(StatusCode::NOT_FOUND, "no matching route", &correlation);
        }
    };
    let route_name = route_cfg.name.clone();
    let runtime = match state.route(&route_name) {
        Some(r) => r,
        None => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "route runtime missing",
                &correlation,
            )
        }
    };

    // Authentication.
    if let Err(reason) = auth::authorize(&runtime.config.auth, &headers) {
        state
            .metrics
            .auth_rejections
            .with_label_values(&[&route_name])
            .inc();
        record(
            state,
            &route_name,
            &method,
            StatusCode::UNAUTHORIZED,
            started,
        );
        return error(StatusCode::UNAUTHORIZED, reason, &correlation);
    }

    // Rate limiting.
    if let Some(rl) = &runtime.config.rate_limit {
        let key = rate_key(rl.scope, &route_name, &client_ip, &headers);
        let decision = state
            .limiter
            .check(&key, rl.limit, Duration::from_secs(rl.window_secs))
            .await;
        if !decision.allowed {
            state
                .metrics
                .rate_limit_rejections
                .with_label_values(&[&route_name])
                .inc();
            record(
                state,
                &route_name,
                &method,
                StatusCode::TOO_MANY_REQUESTS,
                started,
            );
            let mut resp = error(
                StatusCode::TOO_MANY_REQUESTS,
                "rate limit exceeded",
                &correlation,
            );
            add_ratelimit_headers(resp.headers_mut(), decision.limit, 0, decision.reset_secs);
            return resp;
        }
    }

    // Cache lookup (GET only, when configured and not bypassed).
    let cache_enabled = runtime.cache.is_some() && method == Method::GET && !cache_bypass(&headers);
    let cache_key = crate::cache::ResponseCache::key(
        method.as_str(),
        &host,
        uri.path_and_query().map(|p| p.as_str()).unwrap_or("/"),
    );
    if cache_enabled {
        if let Some(cache) = &runtime.cache {
            if let Some(hit) = cache.get(&cache_key) {
                state
                    .metrics
                    .cache_hits
                    .with_label_values(&[&route_name])
                    .inc();
                record(
                    state,
                    &route_name,
                    &method,
                    StatusCode::from_u16(hit.status).unwrap_or(StatusCode::OK),
                    started,
                );
                return cached_response(hit, &runtime, &correlation);
            }
            state
                .metrics
                .cache_misses
                .with_label_values(&[&route_name])
                .inc();
        }
    }

    // Buffer the request body (bounded).
    let body_bytes = match axum::body::to_bytes(body, state.config.server.max_body_bytes).await {
        Ok(b) => b,
        Err(_) => {
            record(
                state,
                &route_name,
                &method,
                StatusCode::PAYLOAD_TOO_LARGE,
                started,
            );
            return error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request body too large",
                &correlation,
            );
        }
    };

    let fwd_path = forward_path(uri.path(), &runtime.config);
    let path_and_query = match uri.query() {
        Some(q) => format!("{fwd_path}?{q}"),
        None => fwd_path,
    };
    let timeout = Duration::from_millis(
        runtime
            .config
            .timeout_ms
            .unwrap_or(state.config.server.default_timeout_ms),
    );
    let mut fwd_headers = transform::forwarded_request_headers(&headers, &client_ip, &host, "http");
    transform::apply_transforms(&mut fwd_headers, &runtime.config.request_headers);
    set_header(&mut fwd_headers, CORRELATION_HEADER, &correlation);

    // Attempt with retries over distinct upstreams.
    let max_attempts = runtime.config.retries.saturating_add(1).max(1);
    let idempotent = is_idempotent(&method);
    let mut excluded: Vec<usize> = Vec::new();

    for attempt in 0..max_attempts {
        let Some((idx, upstream, _guard)) = runtime.balancer.pick(&excluded) else {
            record(
                state,
                &route_name,
                &method,
                StatusCode::SERVICE_UNAVAILABLE,
                started,
            );
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "no healthy upstream",
                &correlation,
            );
        };
        let url = format!("{}{}", upstream.url, path_and_query);

        let upstream_start = Instant::now();
        let result = state
            .client
            .request(method.clone(), &url)
            .headers(fwd_headers.clone())
            .body(body_bytes.clone())
            .timeout(timeout)
            .send()
            .await;
        state
            .metrics
            .upstream_latency
            .with_label_values(&[&route_name])
            .observe(upstream_start.elapsed().as_secs_f64());

        // Reflect breaker state in the gauge.
        state
            .metrics
            .circuit_state
            .with_label_values(&[&route_name, &upstream.url])
            .set(upstream.breaker.state().code());

        match result {
            Ok(resp) => {
                let status = resp.status();
                // Passive failure detection: 5xx counts against the breaker.
                if status.is_server_error() {
                    upstream.breaker.record_failure();
                    state
                        .metrics
                        .upstream_errors
                        .with_label_values(&[&route_name])
                        .inc();
                    let can_retry = idempotent && attempt + 1 < max_attempts;
                    if can_retry {
                        excluded.push(idx);
                        continue;
                    }
                } else {
                    upstream.breaker.record_success();
                }
                return finish_response(
                    state,
                    &runtime,
                    &route_name,
                    &method,
                    status,
                    resp,
                    cache_enabled,
                    cache_key,
                    &correlation,
                    started,
                )
                .await;
            }
            Err(e) => {
                upstream.breaker.record_failure();
                state
                    .metrics
                    .upstream_errors
                    .with_label_values(&[&route_name])
                    .inc();
                tracing::warn!(route = %route_name, upstream = %upstream.url, error = %e, "upstream call failed");
                let can_retry = (idempotent || e.is_connect()) && attempt + 1 < max_attempts;
                if can_retry {
                    excluded.push(idx);
                    continue;
                }
                record(
                    state,
                    &route_name,
                    &method,
                    StatusCode::BAD_GATEWAY,
                    started,
                );
                return error(
                    StatusCode::BAD_GATEWAY,
                    "upstream unavailable",
                    &correlation,
                );
            }
        }
    }

    record(
        state,
        &route_name,
        &method,
        StatusCode::BAD_GATEWAY,
        started,
    );
    error(
        StatusCode::BAD_GATEWAY,
        "all upstream attempts failed",
        &correlation,
    )
}

#[allow(clippy::too_many_arguments)]
async fn finish_response(
    state: &GatewayState,
    runtime: &Arc<RouteRuntime>,
    route_name: &str,
    method: &Method,
    status: StatusCode,
    resp: reqwest::Response,
    cache_enabled: bool,
    cache_key: String,
    correlation: &str,
    started: Instant,
) -> Response {
    let mut out_headers = transform::filtered_response_headers(resp.headers());
    transform::apply_transforms(&mut out_headers, &runtime.config.response_headers);
    set_header(&mut out_headers, CORRELATION_HEADER, correlation);

    let store = cache_enabled
        && status == StatusCode::OK
        && runtime.cache.is_some()
        && response_cacheable(resp.headers());

    record(state, route_name, method, status, started);

    if store {
        // Buffer so we can both cache and return.
        let bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(_) => return error(StatusCode::BAD_GATEWAY, "upstream read failed", correlation),
        };
        if let Some(cache) = &runtime.cache {
            let header_pairs: Vec<(String, String)> = out_headers
                .iter()
                .filter_map(|(n, v)| {
                    v.to_str()
                        .ok()
                        .map(|v| (n.as_str().to_string(), v.to_string()))
                })
                .collect();
            cache.put(cache_key, status.as_u16(), header_pairs, bytes.clone());
        }
        set_header(&mut out_headers, "x-cache", "MISS");
        build_response(status, out_headers, Body::from(bytes))
    } else {
        if cache_enabled {
            set_header(&mut out_headers, "x-cache", "MISS");
        }
        let stream = resp.bytes_stream();
        build_response(status, out_headers, Body::from_stream(stream))
    }
}

fn cached_response(
    hit: crate::cache::CachedResponse,
    runtime: &Arc<RouteRuntime>,
    correlation: &str,
) -> Response {
    let mut headers = HeaderMap::new();
    for (n, v) in &hit.headers {
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(n.as_bytes()),
            HeaderValue::from_str(v),
        ) {
            headers.insert(name, val);
        }
    }
    transform::apply_transforms(&mut headers, &runtime.config.response_headers);
    set_header(&mut headers, "x-cache", "HIT");
    set_header(&mut headers, CORRELATION_HEADER, correlation);
    build_response(
        StatusCode::from_u16(hit.status).unwrap_or(StatusCode::OK),
        headers,
        Body::from(hit.body),
    )
}

fn build_response(status: StatusCode, headers: HeaderMap, body: Body) -> Response {
    let mut resp = Response::new(body);
    *resp.status_mut() = status;
    *resp.headers_mut() = headers;
    resp
}

fn error(status: StatusCode, message: &str, correlation: &str) -> Response {
    let body = serde_json::json!({ "error": message }).to_string();
    let mut resp = (status, body).into_response();
    set_header(resp.headers_mut(), "content-type", "application/json");
    set_header(resp.headers_mut(), CORRELATION_HEADER, correlation);
    resp
}

fn record(
    state: &GatewayState,
    route: &str,
    method: &Method,
    status: StatusCode,
    started: Instant,
) {
    state
        .metrics
        .requests
        .with_label_values(&[route, method.as_str(), status.as_str()])
        .inc();
    state
        .metrics
        .gateway_latency
        .with_label_values(&[route])
        .observe(started.elapsed().as_secs_f64());
}

fn client_ip(headers: &HeaderMap, peer: SocketAddr) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| peer.ip().to_string())
}

fn rate_key(scope: RateScope, route: &str, client_ip: &str, headers: &HeaderMap) -> String {
    match scope {
        RateScope::Ip => format!("{route}:ip:{client_ip}"),
        RateScope::Route => format!("{route}:route"),
        RateScope::ApiKey => {
            let key = headers
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
                .or_else(|| {
                    headers
                        .get(http::header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.strip_prefix("Bearer ").unwrap_or(s).trim().to_string())
                })
                .unwrap_or_else(|| "anon".to_string());
            format!("{route}:key:{key}")
        }
    }
}

fn is_idempotent(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::PUT | Method::DELETE | Method::OPTIONS | Method::TRACE
    )
}

fn cache_bypass(headers: &HeaderMap) -> bool {
    headers
        .get(http::header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            let v = v.to_ascii_lowercase();
            v.contains("no-cache") || v.contains("no-store")
        })
        .unwrap_or(false)
}

fn response_cacheable(headers: &HeaderMap) -> bool {
    headers
        .get(http::header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            let v = v.to_ascii_lowercase();
            !(v.contains("no-store") || v.contains("private") || v.contains("no-cache"))
        })
        .unwrap_or(true)
}

fn add_ratelimit_headers(headers: &mut HeaderMap, limit: u32, remaining: u32, reset: u64) {
    set_header(headers, "x-ratelimit-limit", &limit.to_string());
    set_header(headers, "x-ratelimit-remaining", &remaining.to_string());
    set_header(headers, "x-ratelimit-reset", &reset.to_string());
    set_header(headers, "retry-after", &reset.to_string());
}

fn set_header(headers: &mut HeaderMap, name: &str, value: &str) {
    if let (Ok(n), Ok(v)) = (
        HeaderName::from_bytes(name.as_bytes()),
        HeaderValue::from_str(value),
    ) {
        headers.insert(n, v);
    }
}
