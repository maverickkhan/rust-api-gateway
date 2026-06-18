//! Test harness: a configurable mock upstream and a helper that spins up the
//! real gateway proxy on an ephemeral port. Tests drive the gateway with a
//! normal HTTP client.

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Json;
use axum::Router;
use gw_core::GatewayConfig;
use gw_gateway::GatewayState;
use gw_telemetry::Metrics;
use http::{HeaderMap, Method, StatusCode, Uri};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// A running mock upstream that echoes the request and counts hits.
pub struct Upstream {
    pub addr: SocketAddr,
    pub id: String,
    hits: Arc<AtomicUsize>,
    _token: CancellationToken,
}

impl Upstream {
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
struct UpState {
    id: String,
    hits: Arc<AtomicUsize>,
}

/// Start a mock upstream on `127.0.0.1:0`. Routes:
/// * `/slow?ms=N` — sleep N ms then 200
/// * `/status/{code}` — return that status
/// * everything else — echo `{id, method, path, query, headers}` with a
///   `Cache-Control: max-age=...` header so responses are cacheable.
pub async fn start_upstream(id: &str) -> Upstream {
    let hits = Arc::new(AtomicUsize::new(0));
    let state = UpState {
        id: id.to_string(),
        hits: hits.clone(),
    };
    let app = Router::new()
        .route("/slow", get(slow))
        .route("/status/{code}", get(status_code))
        .fallback(echo)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = CancellationToken::new();
    let tc = token.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move { tc.cancelled().await })
            .await;
    });
    Upstream {
        addr,
        id: id.to_string(),
        hits,
        _token: token,
    }
}

async fn echo(
    State(s): State<UpState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> impl IntoResponse {
    s.hits.fetch_add(1, Ordering::Relaxed);
    let header_map: serde_json::Map<String, serde_json::Value> = headers
        .iter()
        .filter_map(|(n, v)| {
            v.to_str().ok().map(|v| {
                (
                    n.as_str().to_string(),
                    serde_json::Value::String(v.to_string()),
                )
            })
        })
        .collect();
    let body = Json(serde_json::json!({
        "id": s.id,
        "method": method.as_str(),
        "path": uri.path(),
        "query": uri.query(),
        "headers": header_map,
    }));
    ([("cache-control", "max-age=60")], body)
}

async fn slow(Query(q): Query<HashMap<String, String>>) -> impl IntoResponse {
    let ms: u64 = q.get("ms").and_then(|v| v.parse().ok()).unwrap_or(1000);
    tokio::time::sleep(Duration::from_millis(ms)).await;
    (StatusCode::OK, "slow ok")
}

async fn status_code(Path(code): Path<u16>) -> impl IntoResponse {
    StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

/// A running gateway proxy on an ephemeral port.
pub struct Gateway {
    pub addr: SocketAddr,
    pub state: GatewayState,
    _token: CancellationToken,
}

impl Gateway {
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
}

/// Build the gateway runtime from a config and serve it on an ephemeral port.
pub async fn start_gateway(config: GatewayConfig) -> Gateway {
    let state = GatewayState::build(config, Metrics::new()).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = gw_gateway::server::proxy_router(state.clone());
    let token = CancellationToken::new();
    let tc = token.clone();
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move { tc.cancelled().await })
        .await;
    });
    tokio::spawn(gw_gateway::health::run(state.clone(), token.clone()));
    Gateway {
        addr,
        state,
        _token: token,
    }
}

/// Parse a YAML config (panics on error — test convenience).
pub fn cfg(yaml: &str) -> GatewayConfig {
    GatewayConfig::from_yaml(yaml).expect("valid config")
}
