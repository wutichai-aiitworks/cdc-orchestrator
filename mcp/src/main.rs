//! MCP HTTP gateway for hyper-cdc-orchestrator (`/cdc/*`).
//!
//! Streamable HTTP transport (POST JSON-RPC, no server-initiated stream). Clients authenticate
//! with a per-client bearer key that carries a scope (read | write); see ADR 0008.
#![recursion_limit = "256"]

use std::{
    collections::HashMap,
    env,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{bail, Result};
use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};
use uuid::Uuid;

mod copy_tools;
mod redact;
mod tools;

/// Newest first. The server answers with the client's version when supported, else the newest.
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    Read,
    Write,
}

#[derive(Clone, Debug)]
pub(crate) struct ClientKey {
    name: String,
    scope: Scope,
    token: String,
}

/// The authenticated caller of the current request (task-local, so tools and `orch` see it
/// without threading it through every signature).
#[derive(Clone, Debug)]
pub(crate) struct ClientCtx {
    pub(crate) name: String,
    pub(crate) scope: Scope,
}

tokio::task_local! {
    pub(crate) static CLIENT: ClientCtx;
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) cfg: Arc<Config>,
    http: Client,
    sessions: Arc<Mutex<Sessions>>,
    rate_limit: Arc<Mutex<RateLimiter>>,
}

#[derive(Clone)]
pub(crate) struct Config {
    bind_addr: String,
    clients: Vec<ClientKey>,
    /// Global kill switch: no client can write while this is false, whatever its scope.
    pub(crate) allow_write: bool,
    orchestrator_url: String,
    orchestrator_token: Option<String>,
    cors_origins: Vec<String>,
    rate_limit_enabled: bool,
    rate_limit_rpm: u32,
    session_ttl: Duration,
    max_sessions: usize,
}

impl Config {
    fn from_env() -> Result<Self> {
        let clients = load_clients(
            env::var("MCP_CLIENT_KEYS").ok().as_deref(),
            env::var("MCP_BEARER_TOKEN").ok().as_deref(),
        )?;
        let allow_weak = env_or("MCP_ALLOW_WEAK_TOKENS", "false").eq_ignore_ascii_case("true");
        let weak: Vec<&str> = clients
            .iter()
            .filter(|c| is_weak_secret(&c.token))
            .map(|c| c.name.as_str())
            .collect();
        if !weak.is_empty() {
            if allow_weak {
                tracing::warn!(clients = ?weak, "weak MCP client key(s) allowed by MCP_ALLOW_WEAK_TOKENS — local development only");
            } else {
                bail!(
                    "weak MCP client key(s) for {weak:?}: use at least 24 random characters \
                     (e.g. `openssl rand -hex 24`), not a placeholder. \
                     Set MCP_ALLOW_WEAK_TOKENS=true only for local development."
                );
            }
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
            bind_addr: env_or("MCP_BIND_ADDR", "127.0.0.1:8410"),
            clients,
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
            session_ttl: Duration::from_secs(
                env_or("MCP_SESSION_TTL_SECS", "86400")
                    .parse()
                    .unwrap_or(86_400),
            ),
            max_sessions: env_or("MCP_MAX_SESSIONS", "1000").parse().unwrap_or(1000),
        })
    }
}

/// `MCP_CLIENT_KEYS`: entries `name:scope:token` separated by commas or newlines
/// (scope = read | write). `MCP_BEARER_TOKEN` (legacy) adds client `default` with write scope;
/// writes still need `ALLOW_WRITE_TOOLS=true`.
fn load_clients(keys: Option<&str>, legacy: Option<&str>) -> Result<Vec<ClientKey>> {
    let mut clients = Vec::new();
    for entry in keys
        .unwrap_or_default()
        .split([',', '\n'])
        .map(str::trim)
        .filter(|e| !e.is_empty())
    {
        let mut parts = entry.splitn(3, ':');
        let (Some(name), Some(scope), Some(token)) = (parts.next(), parts.next(), parts.next())
        else {
            bail!(
                "MCP_CLIENT_KEYS entry must be name:scope:token (got an entry with {} part(s))",
                entry.split(':').count()
            );
        };
        let name = name.trim();
        if name.is_empty()
            || name.len() > 64
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        {
            bail!("MCP_CLIENT_KEYS client name `{name}`: use 1-64 of [A-Za-z0-9-_.]");
        }
        let scope = match scope.trim().to_ascii_lowercase().as_str() {
            "read" => Scope::Read,
            "write" => Scope::Write,
            other => {
                bail!("MCP_CLIENT_KEYS client `{name}`: scope must be read or write, got `{other}`")
            }
        };
        clients.push(ClientKey {
            name: name.to_string(),
            scope,
            token: token.trim().to_string(),
        });
    }
    if let Some(tok) = legacy.map(str::trim).filter(|t| !t.is_empty()) {
        clients.push(ClientKey {
            name: "default".into(),
            scope: Scope::Write,
            token: tok.to_string(),
        });
    }
    if clients.is_empty() {
        bail!("no MCP client keys: set MCP_CLIENT_KEYS (name:scope:token,...) or MCP_BEARER_TOKEN");
    }
    for (i, a) in clients.iter().enumerate() {
        for b in &clients[i + 1..] {
            if a.name == b.name {
                bail!("duplicate MCP client name `{}`", a.name);
            }
            if a.token == b.token {
                bail!(
                    "MCP clients `{}` and `{}` share a key; give each client its own",
                    a.name,
                    b.name
                );
            }
        }
    }
    Ok(clients)
}

fn env_or(k: &str, d: &str) -> String {
    env::var(k).unwrap_or_else(|_| d.to_string())
}

fn is_weak_secret(v: &str) -> bool {
    let t = v.trim().to_ascii_lowercase();
    t.len() < 24 || t.contains("change-me") || t.contains("changeme") || t.contains("placeholder")
}

/// Sliding one-minute window per client.
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
        let window = self.window;
        if self.buckets.len() > 256 {
            self.buckets
                .retain(|_, s| s.last().is_some_and(|t| now.duration_since(*t) < window));
        }
        let stamps = self.buckets.entry(key.to_string()).or_default();
        stamps.retain(|t| now.duration_since(*t) < window);
        if stamps.len() as u32 >= self.limit {
            return Err(window.saturating_sub(now.duration_since(stamps[0])));
        }
        stamps.push(now);
        Ok(())
    }
}

/// Sessions are bound to the client that created them, expire when idle, and are capped
/// (oldest evicted) so `initialize` without `DELETE` cannot grow memory without bound.
struct Sessions {
    ttl: Duration,
    max: usize,
    by_id: HashMap<String, (String, Instant)>,
}

impl Sessions {
    fn new(ttl: Duration, max: usize) -> Self {
        Self {
            ttl,
            max: max.max(1),
            by_id: HashMap::new(),
        }
    }

    fn prune(&mut self, now: Instant) {
        let ttl = self.ttl;
        self.by_id
            .retain(|_, (_, seen)| now.duration_since(*seen) < ttl);
    }

    fn create(&mut self, client: &str) -> String {
        let now = Instant::now();
        self.prune(now);
        while self.by_id.len() >= self.max {
            let oldest = self
                .by_id
                .iter()
                .min_by_key(|(_, (_, seen))| *seen)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => self.by_id.remove(&k),
                None => break,
            };
        }
        let sid = Uuid::new_v4().to_string();
        self.by_id.insert(sid.clone(), (client.to_string(), now));
        sid
    }

    /// True when `sid` is live and belongs to `client`; refreshes its idle timer.
    fn touch(&mut self, sid: &str, client: &str) -> bool {
        let now = Instant::now();
        match self.by_id.get_mut(sid) {
            Some((owner, seen)) if owner == client && now.duration_since(*seen) < self.ttl => {
                *seen = now;
                true
            }
            _ => false,
        }
    }

    fn remove(&mut self, sid: &str, client: &str) -> bool {
        if self
            .by_id
            .get(sid)
            .is_some_and(|(owner, _)| owner == client)
        {
            self.by_id.remove(sid);
            true
        } else {
            false
        }
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
            ApiError::Message(m) => (
                StatusCode::BAD_REQUEST,
                json!({"error": redact::redact_text(m)}),
            ),
            ApiError::RateLimited(secs) => (
                StatusCode::TOO_MANY_REQUESTS,
                json!({"error":"rate_limit_exceeded","retry_after_seconds":secs}),
            ),
        };
        let mut res = (status, Json(body)).into_response();
        if let ApiError::Unauthorized = self {
            res.headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        if let ApiError::RateLimited(secs) = self {
            if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
                res.headers_mut().insert(header::RETRY_AFTER, v);
            }
        }
        res
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

impl RpcResponse {
    fn ok(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }
    fn err(id: Option<Value>, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(json!({"code": code, "message": redact::redact_text(&message.into())})),
        }
    }
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
        sessions: Arc::new(Mutex::new(Sessions::new(cfg.session_ttl, cfg.max_sessions))),
        rate_limit: Arc::new(Mutex::new(RateLimiter::new(cfg.rate_limit_rpm))),
    };

    let app = router(state);
    let addr: SocketAddr = cfg.bind_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let names: Vec<String> = cfg
        .clients
        .iter()
        .map(|c| format!("{}({:?})", c.name, c.scope).to_lowercase())
        .collect();
    tracing::info!(%addr, orch = %cfg.orchestrator_url, writes = cfg.allow_write, clients = ?names, "mcp-cdc listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn router(state: AppState) -> Router {
    let cors = build_cors(&state.cfg.cors_origins);
    Router::new()
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
        .layer(TraceLayer::new_for_http())
}

fn build_cors(origins: &[String]) -> CorsLayer {
    let base = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
            HeaderName::from_static("mcp-session-id"),
            HeaderName::from_static("mcp-protocol-version"),
        ])
        .expose_headers([HeaderName::from_static("mcp-session-id")]);
    if origins.is_empty() {
        return base;
    }
    if origins.iter().any(|o| o == "*") {
        return CorsLayer::permissive();
    }
    let parsed: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    base.allow_origin(AllowOrigin::list(parsed))
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
}

/// Constant-time match against every key (no early exit, so timing does not reveal which
/// entry, if any, matched).
fn authenticate(headers: &HeaderMap, clients: &[ClientKey]) -> Option<ClientCtx> {
    let tok = bearer(headers)?;
    let mut found = None;
    for c in clients {
        if bool::from(tok.as_bytes().ct_eq(c.token.as_bytes())) {
            found = Some(ClientCtx {
                name: c.name.clone(),
                scope: c.scope,
            });
        }
    }
    found
}

fn session_header(headers: &HeaderMap) -> Option<&str> {
    headers.get("mcp-session-id").and_then(|v| v.to_str().ok())
}

/// No server-initiated stream is offered; the spec allows 405 for GET.
async fn mcp_get() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [(header::ALLOW, "POST, DELETE")],
    )
        .into_response()
}

async fn mcp_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let client = authenticate(&headers, &state.cfg.clients).ok_or(ApiError::Unauthorized)?;
    let Some(sid) = session_header(&headers) else {
        return Ok(StatusCode::BAD_REQUEST);
    };
    let removed = state.sessions.lock().unwrap().remove(sid, &client.name);
    Ok(if removed {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}

async fn mcp_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let client = authenticate(&headers, &state.cfg.clients).ok_or(ApiError::Unauthorized)?;
    if state.cfg.rate_limit_enabled {
        // Keyed by client, not IP: behind a proxy every caller shares one address.
        if let Err(retry) = state.rate_limit.lock().unwrap().check(&client.name) {
            return Err(ApiError::RateLimited(retry.as_secs().max(1)));
        }
    }

    let req: RpcRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            let resp = RpcResponse::err(None, -32700, format!("parse error: {e}"));
            return Ok((StatusCode::BAD_REQUEST, Json(resp)).into_response());
        }
    };

    // A session id the server does not know (expired, evicted, restarted, or another client's)
    // gets 404 so the client re-initializes. Requests without one are accepted.
    if req.method != "initialize" {
        if let Some(sid) = session_header(&headers) {
            if !state.sessions.lock().unwrap().touch(sid, &client.name) {
                let resp =
                    RpcResponse::err(req.id, -32001, "session not found; send initialize again");
                return Ok((StatusCode::NOT_FOUND, Json(resp)).into_response());
            }
        }
    }

    if req.id.is_none() {
        // Notifications and responses carry no id and get no body.
        return Ok(StatusCode::ACCEPTED.into_response());
    }

    let (resp, new_sid) = CLIENT
        .scope(client.clone(), handle_rpc(&state, &client, req))
        .await;
    let mut response = Json(resp).into_response();
    if let Some(sid) = new_sid.and_then(|s| HeaderValue::from_str(&s).ok()) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("mcp-session-id"), sid);
    }
    Ok(response)
}

fn negotiate_version(params: &Value) -> &'static str {
    let asked = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or("");
    PROTOCOL_VERSIONS
        .iter()
        .copied()
        .find(|v| *v == asked)
        .unwrap_or(PROTOCOL_VERSIONS[0])
}

async fn handle_rpc(
    state: &AppState,
    client: &ClientCtx,
    req: RpcRequest,
) -> (RpcResponse, Option<String>) {
    if req.jsonrpc != "2.0" {
        return (
            RpcResponse::err(req.id, -32600, "invalid jsonrpc version"),
            None,
        );
    }
    let id = req.id;
    match req.method.as_str() {
        "initialize" => {
            let sid = state.sessions.lock().unwrap().create(&client.name);
            let writes = can_write(state);
            let result = json!({
                "protocolVersion": negotiate_version(&req.params),
                "serverInfo": {"name":"mcp-cdc","version":env!("CARGO_PKG_VERSION")},
                "capabilities": {"tools":{"listChanged":false},"prompts":{"listChanged":false}},
                "instructions": format!(
                    "Operate Hyper CDC without the web UI. Start with cdc_help or cdc_overview. \
                     Pipelines: cdc_source_validate / cdc_target_validate, then cdc_pipeline_create. \
                     Copy & Sync: cdc_copy_test_connection, cdc_pg_copy_create (saves only), cdc_copy_run (starts; only when asked). \
                     Write tools for this client: {}.",
                    if writes { "enabled" } else { "disabled" }
                ),
            });
            (RpcResponse::ok(id, result), Some(sid))
        }
        "ping" => (RpcResponse::ok(id, json!({})), None),
        "tools/list" => (
            RpcResponse::ok(id, json!({ "tools": tools::definitions() })),
            None,
        ),
        "tools/call" => {
            let call: ToolCall = match serde_json::from_value(req.params) {
                Ok(c) => c,
                Err(e) => {
                    return (
                        RpcResponse::err(id, -32602, format!("invalid params: {e}")),
                        None,
                    )
                }
            };
            if !tools::definitions().iter().any(|d| d.name == call.name) {
                return (
                    RpcResponse::err(id, -32602, format!("unknown tool: {}", call.name)),
                    None,
                );
            }
            let name = call.name.clone();
            // Tool failures are results with isError so the model can read them and retry;
            // only protocol problems are JSON-RPC errors.
            let result = match tools::call(state, call).await {
                Ok(v) => {
                    let text = serde_json::to_string_pretty(&redact::redact(v)).unwrap_or_default();
                    json!({"content":[{"type":"text","text": text}], "isError": false})
                }
                Err(e) => {
                    tracing::info!(client = %client.name, tool = %name, "tool error");
                    json!({"content":[{"type":"text","text": redact::redact_text(&e.to_string())}], "isError": true})
                }
            };
            (RpcResponse::ok(id, result), None)
        }
        "resources/list" => (RpcResponse::ok(id, json!({ "resources": [] })), None),
        "prompts/list" => (
            RpcResponse::ok(id, json!({ "prompts": tools::prompts() })),
            None,
        ),
        "prompts/get" => {
            let name = req.params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = req
                .params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match tools::prompt_messages(name, &arguments) {
                Ok(messages) => (
                    RpcResponse::ok(
                        id,
                        json!({"description": tools::prompt_description(name), "messages": messages}),
                    ),
                    None,
                ),
                Err(e) => (RpcResponse::err(id, -32602, e.to_string()), None),
            }
        }
        _ => (RpcResponse::err(id, -32601, "method not found"), None),
    }
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

/// Writes need both the global switch and a write-scoped client key.
pub(crate) fn can_write(state: &AppState) -> bool {
    state.cfg.allow_write
        && CLIENT
            .try_with(|c| c.scope == Scope::Write)
            .unwrap_or(false)
}

pub(crate) fn guard_write(state: &AppState) -> Result<(), ApiError> {
    if !state.cfg.allow_write {
        return Err(ApiError::Message(
            "write tools disabled; set ALLOW_WRITE_TOOLS=true".into(),
        ));
    }
    match CLIENT.try_with(|c| c.clone()) {
        Ok(c) if c.scope == Scope::Write => Ok(()),
        Ok(c) => Err(ApiError::Message(format!(
            "write tools disabled for client `{}` (read scope); ask an operator for a write-scoped key",
            c.name
        ))),
        Err(_) => Err(ApiError::Message("write tools disabled: no authenticated client".into())),
    }
}

pub(crate) fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ApiError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::Message(format!("missing string arg: {key}")))
}

/// Orchestrator calls that legitimately run longer than the shared 60 s client timeout.
fn orch_timeout(method: &Method, path: &str) -> Option<Duration> {
    let slow = path.starts_with("/cdc/engine/")
        || (*method == Method::POST
            && (path.ends_with("/run") || path.ends_with("/test-connection")));
    slow.then(|| Duration::from_secs(300))
}

pub(crate) async fn orch(
    state: &AppState,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, ApiError> {
    let url = format!("{}{path}", state.cfg.orchestrator_url);
    let timeout = orch_timeout(&method, path);
    let mut req = state.http.request(method, &url);
    if let Some(t) = timeout {
        req = req.timeout(t);
    }
    if let Some(tok) = &state.cfg.orchestrator_token {
        req = req.bearer_auth(tok);
    }
    // Orchestrator audit log records `x-user` as the actor.
    if let Ok(name) = CLIENT.try_with(|c| c.name.clone()) {
        req = req.header("x-user", format!("mcp:{name}"));
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
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const KEY_W: &str = "writer-key-that-is-long-enough-0001";
    const KEY_R: &str = "reader-key-that-is-long-enough-0002";

    fn state(allow_write: bool) -> AppState {
        let cfg = Arc::new(Config {
            bind_addr: "127.0.0.1:0".into(),
            clients: vec![
                ClientKey {
                    name: "ops".into(),
                    scope: Scope::Write,
                    token: KEY_W.into(),
                },
                ClientKey {
                    name: "bi".into(),
                    scope: Scope::Read,
                    token: KEY_R.into(),
                },
            ],
            allow_write,
            // Unroutable: any tool that skips guard_write fails with a connection error instead.
            orchestrator_url: "http://127.0.0.1:1".into(),
            orchestrator_token: None,
            cors_origins: vec![],
            rate_limit_enabled: false,
            rate_limit_rpm: 60,
            session_ttl: Duration::from_secs(60),
            max_sessions: 3,
        });
        AppState {
            cfg: cfg.clone(),
            http: Client::builder()
                .timeout(Duration::from_millis(500))
                .build()
                .unwrap(),
            sessions: Arc::new(Mutex::new(Sessions::new(cfg.session_ttl, cfg.max_sessions))),
            rate_limit: Arc::new(Mutex::new(RateLimiter::new(cfg.rate_limit_rpm))),
        }
    }

    fn auth(key: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            format!("Bearer {key}").parse().unwrap(),
        );
        h
    }

    async fn post(
        st: &AppState,
        headers: HeaderMap,
        body: Value,
    ) -> (StatusCode, HeaderMap, Value) {
        let res = match mcp_post(State(st.clone()), headers, Bytes::from(body.to_string())).await {
            Ok(r) => r,
            Err(e) => e.into_response(),
        };
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            headers,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn ctx(scope: Scope) -> ClientCtx {
        ClientCtx {
            name: "t".into(),
            scope,
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
            assert!(
                d.name.starts_with("cdc_"),
                "{} must start with cdc_",
                d.name
            );
            assert!(seen.insert(d.name.clone()), "duplicate tool {}", d.name);
            assert!(d.description.len() >= 10, "{} needs a description", d.name);
            assert_eq!(d.input_schema["type"], "object", "{} input schema", d.name);
            assert!(
                d.annotations["readOnlyHint"].is_boolean(),
                "{} readOnlyHint",
                d.name
            );
        }
    }

    #[test]
    fn required_args_exist_in_properties() {
        for d in tools::definitions() {
            let props = d.input_schema["properties"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            for req in d.input_schema["required"]
                .as_array()
                .cloned()
                .unwrap_or_default()
            {
                let key = req.as_str().unwrap();
                assert!(
                    props.contains_key(key),
                    "{}: required `{key}` missing from properties",
                    d.name
                );
            }
        }
    }

    #[test]
    fn every_tool_is_dispatched() {
        let src = [include_str!("tools.rs"), include_str!("copy_tools.rs")].concat();
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
            let call = ToolCall {
                name: d.name.clone(),
                arguments: generous_args(),
            };
            match tools::call(&st, call).await {
                Err(ApiError::Message(m)) if m.contains("write tools disabled") => {}
                other => unguarded.push(format!("{} → {:?}", d.name, other.map(|_| "ok"))),
            }
        }
        assert!(
            unguarded.is_empty(),
            "write tools without guard_write:\n{}",
            unguarded.join("\n")
        );
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
        assert!(is_weak_secret("short"));
        assert!(
            is_weak_secret("change-me-mcp-cdc-token-min24chars"),
            "compose placeholder is >= 24 chars"
        );
        assert!(is_weak_secret("CHANGEME-but-padded-to-be-long-enough"));
        assert!(!is_weak_secret("a-perfectly-long-random-token-123456"));
    }

    #[test]
    fn client_keys_parse_scopes_and_legacy_token() {
        let c = load_clients(
            Some("ops:write:aaa, bi:READ:b:c:d\nci:read:zzz"),
            Some("legacy"),
        )
        .unwrap();
        let got: Vec<_> = c
            .iter()
            .map(|k| (k.name.as_str(), k.scope, k.token.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("ops", Scope::Write, "aaa"),
                ("bi", Scope::Read, "b:c:d"),
                ("ci", Scope::Read, "zzz"),
                ("default", Scope::Write, "legacy"),
            ]
        );
        assert!(load_clients(None, None).is_err(), "no keys at all");
        assert!(
            load_clients(Some("ops:admin:x"), None).is_err(),
            "bad scope"
        );
        assert!(load_clients(Some("ops:x"), None).is_err(), "missing token");
        assert!(load_clients(Some("a b:read:x"), None).is_err(), "bad name");
        assert!(
            load_clients(Some("a:read:x,a:write:y"), None).is_err(),
            "duplicate name"
        );
        assert!(
            load_clients(Some("a:read:x,b:write:x"), None).is_err(),
            "shared key"
        );
    }

    #[test]
    fn authenticate_matches_exact_key_and_returns_scope() {
        let st = state(true);
        assert_eq!(
            authenticate(&auth(KEY_W), &st.cfg.clients).unwrap().scope,
            Scope::Write
        );
        let r = authenticate(&auth(KEY_R), &st.cfg.clients).unwrap();
        assert_eq!((r.name.as_str(), r.scope), ("bi", Scope::Read));
        assert!(
            authenticate(&auth("reader-key-that-is-long-enough-000"), &st.cfg.clients).is_none()
        );
        assert!(authenticate(&HeaderMap::new(), &st.cfg.clients).is_none());
        let mut basic = HeaderMap::new();
        basic.insert(
            header::AUTHORIZATION,
            format!("Basic {KEY_W}").parse().unwrap(),
        );
        assert!(authenticate(&basic, &st.cfg.clients).is_none());
    }

    #[tokio::test]
    async fn read_scope_cannot_write_even_when_writes_enabled() {
        let st = state(true);
        let r = CLIENT
            .scope(ctx(Scope::Read), async { guard_write(&st) })
            .await;
        assert!(matches!(r, Err(ApiError::Message(m)) if m.contains("read scope")));
        assert!(CLIENT
            .scope(ctx(Scope::Write), async { guard_write(&st) })
            .await
            .is_ok());
        assert!(guard_write(&st).is_err(), "no client in scope");
        let off = state(false);
        assert!(
            CLIENT
                .scope(ctx(Scope::Write), async { guard_write(&off) })
                .await
                .is_err(),
            "global switch wins"
        );
    }

    #[test]
    fn sessions_are_capped_owned_and_expire() {
        let mut s = Sessions::new(Duration::from_secs(60), 2);
        let a = s.create("ops");
        let b = s.create("ops");
        let c = s.create("bi");
        assert_eq!(s.by_id.len(), 2, "cap evicts oldest");
        assert!(!s.touch(&a, "ops"), "oldest evicted");
        assert!(s.touch(&b, "ops"));
        assert!(!s.touch(&c, "ops"), "another client's session");
        assert!(!s.remove(&c, "ops"));
        assert!(s.remove(&c, "bi"));
        let mut short = Sessions::new(Duration::from_millis(0), 10);
        let x = short.create("ops");
        assert!(!short.touch(&x, "ops"), "expired");
    }

    #[test]
    fn rate_limit_is_per_key() {
        let mut rl = RateLimiter::new(2);
        assert!(rl.check("ops").is_ok());
        assert!(rl.check("ops").is_ok());
        assert!(rl.check("ops").is_err());
        assert!(rl.check("bi").is_ok(), "other client has its own bucket");
    }

    #[test]
    fn version_negotiation_and_slow_paths() {
        assert_eq!(
            negotiate_version(&json!({"protocolVersion":"2025-03-26"})),
            "2025-03-26"
        );
        assert_eq!(
            negotiate_version(&json!({"protocolVersion":"1999-01-01"})),
            PROTOCOL_VERSIONS[0]
        );
        assert_eq!(negotiate_version(&json!({})), PROTOCOL_VERSIONS[0]);
        assert!(orch_timeout(&Method::POST, "/cdc/engine/up").is_some());
        assert!(orch_timeout(&Method::POST, "/cdc/pgcopy/profiles/x/run").is_some());
        assert!(orch_timeout(&Method::GET, "/cdc/profiles").is_none());
    }

    #[tokio::test]
    async fn tool_failures_are_results_with_is_error() {
        let st = state(false);
        // Write tool with writes off: a tool error, not an HTTP 400.
        let (status, _, body) = post(&st, auth(KEY_W), json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cdc_copy_run","arguments":{"tool":"pgcopy","id":"x"}}})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["isError"], true);
        assert!(body["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("write tools disabled"));
        // Unknown tool and bad params: JSON-RPC -32602.
        let (status, _, body) = post(
            &st,
            auth(KEY_W),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"nope"}}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["error"]["code"], -32602);
        let (_, _, body) = post(
            &st,
            auth(KEY_W),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"arguments":{}}}),
        )
        .await;
        assert_eq!(body["error"]["code"], -32602);
        // Parse error: -32700 with HTTP 400.
        let res = mcp_post(
            State(st.clone()),
            auth(KEY_W),
            Bytes::from_static(b"{not json"),
        )
        .await
        .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn http_auth_sessions_and_methods() {
        let st = state(false);
        let (status, headers, _) = post(
            &st,
            HeaderMap::new(),
            json!({"jsonrpc":"2.0","id":1,"method":"ping"}),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(headers.contains_key(header::WWW_AUTHENTICATE));

        let (status, headers, body) = post(&st, auth(KEY_W), json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["protocolVersion"], "2025-03-26");
        let sid = headers
            .get("mcp-session-id")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let mut with_sid = auth(KEY_W);
        with_sid.insert("mcp-session-id", sid.parse().unwrap());
        let (status, _, _) = post(
            &st,
            with_sid.clone(),
            json!({"jsonrpc":"2.0","id":2,"method":"ping"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // Same session id, other client: 404.
        let mut stolen = auth(KEY_R);
        stolen.insert("mcp-session-id", sid.parse().unwrap());
        let (status, _, _) = post(
            &st,
            stolen.clone(),
            json!({"jsonrpc":"2.0","id":3,"method":"ping"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Notification: 202, no body.
        let (status, _, _) = post(
            &st,
            with_sid.clone(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        // DELETE needs auth and only removes the caller's own session.
        let unauth = mcp_delete(State(st.clone()), {
            let mut h = HeaderMap::new();
            h.insert("mcp-session-id", sid.parse().unwrap());
            h
        })
        .await;
        assert!(matches!(unauth, Err(ApiError::Unauthorized)));
        assert_eq!(
            mcp_delete(State(st.clone()), stolen).await.unwrap(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            mcp_delete(State(st.clone()), with_sid.clone())
                .await
                .unwrap(),
            StatusCode::NO_CONTENT
        );
        let (status, _, _) = post(
            &st,
            with_sid,
            json!({"jsonrpc":"2.0","id":4,"method":"ping"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "deleted session");

        assert_eq!(mcp_get().await.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn rate_limit_applies_per_client_over_http() {
        let mut st = state(false);
        let mut cfg = (*st.cfg).clone();
        cfg.rate_limit_enabled = true;
        st.cfg = Arc::new(cfg);
        st.rate_limit = Arc::new(Mutex::new(RateLimiter::new(1)));
        let ping = json!({"jsonrpc":"2.0","id":1,"method":"ping"});
        assert_eq!(post(&st, auth(KEY_W), ping.clone()).await.0, StatusCode::OK);
        let (status, headers, _) = post(&st, auth(KEY_W), ping.clone()).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert!(headers.contains_key(header::RETRY_AFTER));
        assert_eq!(post(&st, auth(KEY_R), ping).await.0, StatusCode::OK);
    }
}
