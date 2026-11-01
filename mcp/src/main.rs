//! MCP HTTP gateway for hyper-cdc-orchestrator (`/cdc/*`).
#![recursion_limit = "256"]

use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    env,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Result;
use axum::{
    body::Bytes,
    extract::{ConnectInfo, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::get,
    Json, Router,
};
use futures_util::stream::{self, Stream};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::sync::RwLock;
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};
use uuid::Uuid;

mod tools;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) cfg: Arc<Config>,
    http: Client,
    sessions: Arc<RwLock<HashSet<String>>>,
    rate_limit: Arc<Mutex<RateLimiter>>,
}

#[derive(Clone)]
pub(crate) struct Config {
    bind_addr: String,
    bearer_token: String,
    pub(crate) allow_write: bool,
    orchestrator_url: String,
    orchestrator_token: Option<String>,
    cors_origins: Vec<String>,
    rate_limit_enabled: bool,
    rate_limit_rpm: u32,
}

impl Config {
    fn from_env() -> Result<Self> {
        let bearer_token = env_or("MCP_BEARER_TOKEN", "change-me-mcp-cdc-token");
        if is_weak_secret(&bearer_token) {
            tracing::warn!("MCP_BEARER_TOKEN is weak — do not expose publicly");
        }
        let cors_origins = env::var("MCP_CORS_ORIGINS")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            bind_addr: env_or("MCP_BIND_ADDR", "0.0.0.0:8410"),
            bearer_token,
            allow_write: env_or("ALLOW_WRITE_TOOLS", "false").eq_ignore_ascii_case("true"),
            orchestrator_url: env_or("CDC_ORCHESTRATOR_URL", "http://127.0.0.1:9788")
                .trim_end_matches('/')
                .to_string(),
            orchestrator_token: env::var("CDC_ORCHESTRATOR_TOKEN")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            cors_origins,
            rate_limit_enabled: env_or("MCP_RATE_LIMIT_ENABLED", "true")
                .eq_ignore_ascii_case("true"),
            rate_limit_rpm: env_or("MCP_RATE_LIMIT_RPM", "300").parse().unwrap_or(300),
        })
    }
}

fn env_or(k: &str, d: &str) -> String {
    env::var(k).unwrap_or_else(|_| d.to_string())
}

fn is_weak_secret(v: &str) -> bool {
    let t = v.trim();
    t.is_empty()
        || t.len() < 24
        || t.eq_ignore_ascii_case("change-me-mcp-cdc-token")
        || t.eq_ignore_ascii_case("changeme")
}

struct RateLimiter {
    window: Duration,
    limit: u32,
    buckets: HashMap<String, Vec<Instant>>,
}

impl RateLimiter {
    fn new(limit: u32) -> Self {
        Self {
            window: Duration::from_secs(60),
            limit,
            buckets: HashMap::new(),
        }
    }

    fn check(&mut self, key: &str) -> Result<(), Duration> {
        let now = Instant::now();
        let stamps = self.buckets.entry(key.to_string()).or_default();
        stamps.retain(|t| now.duration_since(*t) < self.window);
        if stamps.len() as u32 >= self.limit {
            let retry = self.window.saturating_sub(now.duration_since(stamps[0]));
            return Err(retry);
        }
        stamps.push(now);
        Ok(())
    }
}

#[derive(Debug, Error)]
pub(crate) enum ApiError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("{0}")]
    Message(String),
    #[error("rate limited")]
    RateLimited(u64),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match &self {
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, json!({"error":"unauthorized"})),
            ApiError::Message(m) => (StatusCode::BAD_REQUEST, json!({"error":m})),
            ApiError::RateLimited(secs) => (
                StatusCode::TOO_MANY_REQUESTS,
                json!({"error":"rate_limit_exceeded","retry_after_seconds":secs}),
            ),
        };
        (status, Json(body)).into_response()
    }
}

#[derive(Deserialize)]
struct RpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

#[derive(Serialize)]
pub(crate) struct ToolDef {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
    annotations: Value,
}

#[derive(Deserialize)]
pub(crate) struct ToolCall {
    name: String,
    #[serde(default)]
    arguments: Value,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cfg = Arc::new(Config::from_env()?);
    let http = Client::builder()
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(5))
        .user_agent(concat!("mcp-cdc/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let state = AppState {
        cfg: cfg.clone(),
        http,
        sessions: Arc::new(RwLock::new(HashSet::new())),
        rate_limit: Arc::new(Mutex::new(RateLimiter::new(cfg.rate_limit_rpm))),
    };

    let cors = build_cors(&cfg.cors_origins);
    let app = Router::new()
        .route(
            "/healthz",
            get(|| async { Json(json!({"ok":true,"service":"mcp-cdc"})) }),
        )
        .route("/mcp", get(mcp_get).post(mcp_post).delete(mcp_delete))
        .with_state(state)
        .layer(cors)
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = cfg.bind_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, orch = %cfg.orchestrator_url, "mcp-cdc listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

fn build_cors(origins: &[String]) -> CorsLayer {
    if origins.is_empty() {
        return CorsLayer::new()
            .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
            .allow_headers([
                header::AUTHORIZATION,
                header::CONTENT_TYPE,
                header::ACCEPT,
                HeaderName::from_static("mcp-session-id"),
            ]);
    }
    if origins.iter().any(|o| o == "*") {
        return CorsLayer::permissive();
    }
    let parsed: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(parsed))
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
            HeaderName::from_static("mcp-session-id"),
        ])
}

fn bearer_ok(headers: &HeaderMap, expected: &str) -> bool {
    let Some(v) = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
    else {
        return false;
    };
    let Some(tok) = v.strip_prefix("Bearer ") else {
        return false;
    };
    bool::from(tok.trim().as_bytes().ct_eq(expected.as_bytes()))
}

async fn mcp_get(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let _ = state;
    let stream = stream::once(async { Ok(Event::default().comment("mcp-cdc")) });
    Sse::new(stream).keep_alive(KeepAlive::new())
}

async fn mcp_delete(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(sid) = headers.get("mcp-session-id").and_then(|v| v.to_str().ok()) {
        state.sessions.write().await.remove(sid);
    }
    StatusCode::NO_CONTENT
}

async fn mcp_post(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    if !bearer_ok(&headers, &state.cfg.bearer_token) {
        return Err(ApiError::Unauthorized);
    }
    if state.cfg.rate_limit_enabled {
        let mut rl = state.rate_limit.lock().unwrap();
        if let Err(retry) = rl.check(&addr.ip().to_string()) {
            return Err(ApiError::RateLimited(retry.as_secs().max(1)));
        }
    }

    let req: RpcRequest = serde_json::from_slice(&body)
        .map_err(|e| ApiError::Message(format!("invalid json: {e}")))?;

    if req.id.is_none() && req.method.starts_with("notifications/") {
        return Ok(StatusCode::ACCEPTED.into_response());
    }

    let (resp, new_sid) = handle_rpc(&state, req).await?;
    let mut response = Json(resp).into_response();
    if let Some(sid) = new_sid {
        if let Ok(v) = HeaderValue::from_str(&sid) {
            response
                .headers_mut()
                .insert(HeaderName::from_static("mcp-session-id"), v);
        }
    }
    Ok(response)
}

async fn handle_rpc(
    state: &AppState,
    req: RpcRequest,
) -> Result<(RpcResponse, Option<String>), ApiError> {
    if req.jsonrpc != "2.0" {
        return Ok((
            RpcResponse {
                jsonrpc: "2.0",
                id: req.id,
                result: None,
                error: Some(json!({"code":-32600,"message":"invalid jsonrpc version"})),
            },
            None,
        ));
    }

    let mut new_session = None;
    let response = match req.method.as_str() {
        "initialize" => {
            let sid = Uuid::new_v4().to_string();
            state.sessions.write().await.insert(sid.clone());
            new_session = Some(sid);
            RpcResponse {
                jsonrpc: "2.0",
                id: req.id,
                result: Some(json!({
                    "protocolVersion":"2025-03-26",
                    "serverInfo":{"name":"mcp-cdc","version":env!("CARGO_PKG_VERSION")},
                    "capabilities":{
                        "tools":{"listChanged":false},
                        "prompts":{"listChanged":false}
                    },
                    "instructions":"Operate Hyper CDC without the web UI. Prefer cdc_overview, cdc_help, cdc_tables, cdc_topic_messages, and cdc_pipeline_create. Use cdc_source_validate / cdc_target_validate before creating a pipeline. Writes require ALLOW_WRITE_TOOLS=true."
                })),
                error: None,
            }
        }
        "ping" => RpcResponse {
            jsonrpc: "2.0",
            id: req.id,
            result: Some(json!({})),
            error: None,
        },
        "tools/list" => RpcResponse {
            jsonrpc: "2.0",
            id: req.id,
            result: Some(json!({ "tools": tools::definitions() })),
            error: None,
        },
        "tools/call" => {
            let call: ToolCall = serde_json::from_value(req.params)
                .map_err(|e| ApiError::Message(format!("invalid params: {e}")))?;
            let result = tools::call(state, call).await?;
            RpcResponse {
                jsonrpc: "2.0",
                id: req.id,
                result: Some(json!({
                    "content":[{"type":"text","text": serde_json::to_string_pretty(&result).unwrap()}]
                })),
                error: None,
            }
        }
        "resources/list" => RpcResponse {
            jsonrpc: "2.0",
            id: req.id,
            result: Some(json!({ "resources": [] })),
            error: None,
        },
        "prompts/list" => RpcResponse {
            jsonrpc: "2.0",
            id: req.id,
            result: Some(json!({ "prompts": tools::prompts() })),
            error: None,
        },
        "prompts/get" => {
            let name = req.params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = req
                .params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match tools::prompt_messages(name, &arguments) {
                Ok(messages) => RpcResponse {
                    jsonrpc: "2.0",
                    id: req.id,
                    result: Some(json!({
                        "description": tools::prompt_description(name),
                        "messages": messages
                    })),
                    error: None,
                },
                Err(e) => RpcResponse {
                    jsonrpc: "2.0",
                    id: req.id,
                    result: None,
                    error: Some(json!({"code":-32602,"message": e.to_string()})),
                },
            }
        }
        _ => RpcResponse {
            jsonrpc: "2.0",
            id: req.id,
            result: None,
            error: Some(json!({"code":-32601,"message":"method not found"})),
        },
    };
    Ok((response, new_session))
}

pub(crate) fn td(
    name: &str,
    description: &str,
    mut input_schema: Value,
    read_only: bool,
) -> ToolDef {
    if let Value::Object(map) = &mut input_schema {
        if !map.contains_key("type") {
            map.insert("type".into(), json!("object"));
        }
        map.entry("properties".to_string())
            .or_insert_with(|| json!({}));
    } else {
        input_schema = json!({"type":"object","properties":{}});
    }
    ToolDef {
        name: name.into(),
        description: description.into(),
        input_schema,
        annotations: json!({"readOnlyHint": read_only}),
    }
}

pub(crate) fn guard_write(state: &AppState) -> Result<(), ApiError> {
    if state.cfg.allow_write {
        Ok(())
    } else {
        Err(ApiError::Message(
            "write tools disabled; set ALLOW_WRITE_TOOLS=true".into(),
        ))
    }
}

pub(crate) fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ApiError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::Message(format!("missing string arg: {key}")))
}

pub(crate) async fn orch(
    state: &AppState,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, ApiError> {
    let url = format!("{}{path}", state.cfg.orchestrator_url);
    let mut req = state.http.request(method, &url);
    if let Some(tok) = &state.cfg.orchestrator_token {
        req = req.bearer_auth(tok);
    }
    if let Some(b) = body {
        req = req.json(&b);
    }
    let res = req
        .send()
        .await
        .map_err(|e| ApiError::Message(format!("orchestrator request failed: {e}")))?;
    let status = res.status();
    let bytes = res
        .bytes()
        .await
        .map_err(|e| ApiError::Message(format!("orchestrator read failed: {e}")))?;
    if !status.is_success() {
        return Err(ApiError::Message(format!(
            "orchestrator {status}: {}",
            String::from_utf8_lossy(&bytes)
        )));
    }
    if bytes.is_empty() {
        return Ok(json!({"status":"ok"}));
    }
    serde_json::from_slice(&bytes).or_else(|_| Ok(json!({"raw": String::from_utf8_lossy(&bytes)})))
}

pub(crate) fn urlencoding_simple(s: &str) -> String {
    // Percent-encode UTF-8 bytes, not chars: `c as u8` truncated non-ASCII names (e.g. Thai).
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn state(allow_write: bool) -> AppState {
        let cfg = Arc::new(Config {
            bind_addr: "127.0.0.1:0".into(),
            bearer_token: "test-token-that-is-long-enough-123".into(),
            allow_write,
            // Unroutable: any tool that skips guard_write fails with a connection error instead.
            orchestrator_url: "http://127.0.0.1:1".into(),
            orchestrator_token: None,
            cors_origins: vec![],
            rate_limit_enabled: false,
            rate_limit_rpm: 60,
        });
        AppState {
            cfg: cfg.clone(),
            http: Client::builder().timeout(Duration::from_millis(500)).build().unwrap(),
            sessions: Arc::new(RwLock::new(HashSet::new())),
            rate_limit: Arc::new(Mutex::new(RateLimiter::new(cfg.rate_limit_rpm))),
        }
    }

    /// Superset of argument names used across tools so arg parsing never masks the guard.
    fn generous_args() -> Value {
        json!({
            "id": "00000000-0000-0000-0000-000000000000",
            "profile_id": "00000000-0000-0000-0000-000000000000",
            "name": "mcp-test", "tool_id": "pgcopy", "job_id": "job-1",
            "topic": "mcp-test-topic", "group": "mcp-test-group", "task": 0,
            "action": "pause", "statement": "SELECT 1", "ksql": "SHOW STREAMS;",
            "tables": ["public.t"], "schemas": ["public"], "value": "{}",
            "enabled": true, "level": "INFO", "logger": "root",
        })
    }

    #[test]
    fn tool_names_are_unique_prefixed_and_documented() {
        let defs = tools::definitions();
        let mut seen = HashSet::new();
        for d in &defs {
            assert!(d.name.starts_with("cdc_"), "{} must start with cdc_", d.name);
            assert!(seen.insert(d.name.clone()), "duplicate tool {}", d.name);
            assert!(d.description.len() >= 10, "{} needs a description", d.name);
            assert_eq!(d.input_schema["type"], "object", "{} input schema", d.name);
            assert!(d.annotations["readOnlyHint"].is_boolean(), "{} readOnlyHint", d.name);
        }
    }

    #[test]
    fn required_args_exist_in_properties() {
        for d in tools::definitions() {
            let props = d.input_schema["properties"].as_object().cloned().unwrap_or_default();
            for req in d.input_schema["required"].as_array().cloned().unwrap_or_default() {
                let key = req.as_str().unwrap();
                assert!(props.contains_key(key), "{}: required `{key}` missing from properties", d.name);
            }
        }
    }

    #[test]
    fn every_tool_is_dispatched() {
        let src = include_str!("tools.rs");
        for d in tools::definitions() {
            assert!(
                src.contains(&format!("\"{}\" =>", d.name)),
                "tool {} is advertised but has no match arm in tools::call",
                d.name
            );
        }
    }

    #[tokio::test]
    async fn write_tools_refuse_when_writes_disabled() {
        let st = state(false);
        let mut unguarded = Vec::new();
        for d in tools::definitions() {
            if d.annotations["readOnlyHint"] == json!(true) {
                continue;
            }
            let call = ToolCall { name: d.name.clone(), arguments: generous_args() };
            match tools::call(&st, call).await {
                Err(ApiError::Message(m)) if m.contains("write tools disabled") => {}
                other => unguarded.push(format!("{} → {:?}", d.name, other.map(|_| "ok"))),
            }
        }
        assert!(unguarded.is_empty(), "write tools without guard_write:\n{}", unguarded.join("\n"));
    }

    #[test]
    fn url_encoding_handles_utf8_and_reserved() {
        assert_eq!(urlencoding_simple("cdc-orders_v1.2~x"), "cdc-orders_v1.2~x");
        assert_eq!(urlencoding_simple("a b/c?d"), "a%20b%2Fc%3Fd");
        assert_eq!(urlencoding_simple("ทด"), "%E0%B8%97%E0%B8%94");
    }

    #[test]
    fn weak_secrets_are_detected() {
        assert!(is_weak_secret(""));
        assert!(is_weak_secret("change-me-mcp-cdc-token"));
        assert!(is_weak_secret("short"));
        assert!(!is_weak_secret("a-perfectly-long-random-token-123456"));
    }

    #[test]
    fn bearer_check_is_exact() {
        let mut h = HeaderMap::new();
        assert!(!bearer_ok(&h, "secret-token"));
        h.insert(header::AUTHORIZATION, "Bearer secret-token".parse().unwrap());
        assert!(bearer_ok(&h, "secret-token"));
        assert!(!bearer_ok(&h, "secret-token-2"));
        h.insert(header::AUTHORIZATION, "Basic secret-token".parse().unwrap());
        assert!(!bearer_ok(&h, "secret-token"));
    }
}
