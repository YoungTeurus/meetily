use crate::ControlError;
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Never derive Debug/Serialize: credentials must not appear in logs or RPC responses.
#[derive(Clone)]
pub struct Credentials {
    pub enabled: bool,
    pub read_token: String,
    pub control_token: Option<String>,
}
impl Default for Credentials {
    fn default() -> Self {
        Self {
            enabled: false,
            read_token: token(),
            control_token: None,
        }
    }
}
pub fn token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
#[async_trait::async_trait]
pub trait Dispatcher: Send + Sync + 'static {
    async fn dispatch(&self, method: &str, params: Value) -> Result<Value, ControlError>;
}
#[derive(Clone)]
struct Gateway {
    dispatcher: Arc<dyn Dispatcher>,
    credentials: Arc<RwLock<Credentials>>,
    host: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    method: String,
    #[serde(default = "empty")]
    params: Value,
}
fn empty() -> Value {
    json!({})
}
pub fn router(
    dispatcher: Arc<dyn Dispatcher>,
    credentials: Arc<RwLock<Credentials>>,
    port: u16,
) -> Router {
    Router::new()
        .route("/v1/rpc", post(rpc))
        .layer(DefaultBodyLimit::max(65536))
        .with_state(Gateway {
            dispatcher,
            credentials,
            host: format!("127.0.0.1:{port}"),
        })
}
fn matches_secret(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |v, (x, y)| v | (x ^ y)) == 0
}
fn failure(code: &str, message: &str) -> (StatusCode, Json<Value>) {
    error(ControlError::new(code, message))
}
fn error(e: ControlError) -> (StatusCode, Json<Value>) {
    let status = match e.code.as_str() {
        "unauthorized" => StatusCode::UNAUTHORIZED,
        "forbidden" => StatusCode::FORBIDDEN,
        "not_found" => StatusCode::NOT_FOUND,
        "invalid_request" => StatusCode::BAD_REQUEST,
        "conflict" => StatusCode::CONFLICT,
        "unavailable" => StatusCode::SERVICE_UNAVAILABLE,
        "timeout" => StatusCode::REQUEST_TIMEOUT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(json!({"error":e})))
}
async fn rpc(
    State(g): State<Gateway>,
    headers: HeaderMap,
    body: Result<Json<Request>, axum::extract::rejection::JsonRejection>,
) -> (StatusCode, Json<Value>) {
    // An API client never sends browser-origin headers. No CORS headers or OPTIONS route.
    if headers.contains_key("origin")
        || headers.keys().any(|k| k.as_str().starts_with("sec-fetch-"))
    {
        return failure("forbidden", "Browser requests are not permitted");
    }
    if headers.get("host").and_then(|h| h.to_str().ok()) != Some(&g.host) {
        return failure("forbidden", "Invalid loopback Host");
    }
    let credentials = g.credentials.read().await;
    if !credentials.enabled {
        return failure("forbidden", "Local integrations are disabled");
    }
    let supplied = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("");
    let control = credentials
        .control_token
        .as_deref()
        .is_some_and(|t| matches_secret(t, supplied));
    if !control && !matches_secret(&credentials.read_token, supplied) {
        return failure("unauthorized", "Invalid or revoked credential");
    }
    let Json(request) = match body {
        Ok(r) => r,
        Err(_) => {
            return failure(
                "invalid_request",
                "Expected JSON RPC request with method and object params",
            )
        }
    };
    if !request.params.is_object() {
        return failure("invalid_request", "params must be an object");
    }
    let write = matches!(
        request.method.as_str(),
        "recording.start" | "recording.stop" | "recording.pause" | "recording.resume"
    );
    let read = matches!(
        request.method.as_str(),
        "doctor"
            | "status"
            | "devices.list"
            | "meetings.list"
            | "meetings.get"
            | "meetings.export"
            | "transcript.get"
            | "transcripts.search"
            | "events.list"
            | "detection.status"
    );
    if !write && !read {
        return failure("not_found", "Unknown v1 method");
    }
    if write && !control {
        return failure(
            "forbidden",
            "Recording control requires a separate control credential",
        );
    }
    drop(credentials);
    // Accepted operations are backend-owned. Client disconnect or MCP cancellation cannot
    // cancel a transition half-way through creating/persisting a recording.
    match tokio::spawn(async move { g.dispatcher.dispatch(&request.method, request.params).await })
        .await
    {
        Ok(Ok(value)) => (StatusCode::OK, Json(json!({"result":value}))),
        Ok(Err(e)) => error(e),
        Err(_) => failure("internal", "Backend operation failed"),
    }
}
