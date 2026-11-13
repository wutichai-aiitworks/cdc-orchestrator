//! OAuth 2.1 bridge for remote MCP clients that cannot send a static header — Claude.ai custom
//! connectors, ChatGPT connectors, and others (ADR 0017, amends ADR 0008).
//!
//! The identity stays the per-client key of ADR 0008: on the consent page an operator pastes
//! one of `MCP_CLIENT_KEYS`, and the issued tokens act as that client with that scope (or less,
//! if the app asked for read only). Removing or downgrading a key takes effect on the next
//! request. Supported: RFC 9728 resource metadata, RFC 8414 server metadata, RFC 7591 dynamic
//! client registration, authorization code + PKCE (S256 only), rotating refresh tokens.
//! Tokens are random 256-bit values stored only as SHA-256 hashes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use axum::extract::{Form, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{AppState, ClientCtx, Scope};

const MAX_CLIENTS: usize = 200;
const CODE_TTL: Duration = Duration::from_secs(300);
const MAX_FAILURES: u32 = 10;
const LOCKOUT: Duration = Duration::from_secs(900);

#[derive(Clone, Debug)]
pub struct OAuthConfig {
    pub enabled: bool,
    /// Public base URL (`MCP_PUBLIC_URL`), e.g. https://cdc-mcp.example.com. Without it the base
    /// is taken from X-Forwarded-Proto / X-Forwarded-Host / Host.
    pub public_url: Option<String>,
    pub store_path: Option<PathBuf>,
    pub access_ttl: Duration,
    pub refresh_ttl: Duration,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RegisteredClient {
    client_id: String,
    client_name: String,
    redirect_uris: Vec<String>,
    created_at: i64,
}

#[derive(Clone, Debug)]
struct CodeGrant {
    client_id: String,
    redirect_uri: String,
    challenge: String,
    key_name: String,
    write: bool,
    expires: Instant,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Access,
    Refresh,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TokenGrant {
    kind: Kind,
    client_id: String,
    key_name: String,
    /// The app asked for (and the key allowed) write.
    write: bool,
    expires_at: i64,
}

#[derive(Default, Serialize, Deserialize)]
pub struct OAuthStore {
    clients: HashMap<String, RegisteredClient>,
    /// SHA-256 (hex) of the token → grant.
    tokens: HashMap<String, TokenGrant>,
    #[serde(skip)]
    codes: HashMap<String, CodeGrant>,
    #[serde(skip)]
    failures: HashMap<String, (u32, Instant)>,
}

impl OAuthStore {
    pub fn load(path: Option<&PathBuf>) -> Self {
        let Some(p) = path else {
            return Self::default();
        };
        match std::fs::read(p) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!(path = %p.display(), error = %e, "oauth store unreadable; starting empty");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    fn save(&mut self, path: Option<&PathBuf>) {
        let now = now_unix();
        self.tokens.retain(|_, g| g.expires_at > now);
        let Some(p) = path else { return };
        match serde_json::to_vec(self) {
            Ok(bytes) => {
                let tmp = p.with_extension("tmp");
                if std::fs::write(&tmp, bytes)
                    .and_then(|_| std::fs::rename(&tmp, p))
                    .is_err()
                {
                    tracing::warn!(path = %p.display(), "oauth store write failed");
                }
            }
            Err(e) => tracing::warn!(error = %e, "oauth store encode failed"),
        }
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn random_token() -> String {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    B64URL.encode(b)
}

fn hash(token: &str) -> String {
    hex_lower(&Sha256::digest(token.as_bytes()))
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn pkce_s256(verifier: &str) -> String {
    B64URL.encode(Sha256::digest(verifier.as_bytes()))
}

/// Base URL of this server as clients reach it.
pub fn base_url(state: &AppState, headers: &HeaderMap) -> String {
    if let Some(u) = &state.cfg.oauth.public_url {
        return u.clone();
    }
    let h = |n: &str| {
        headers
            .get(n)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.split(',').next().unwrap_or("").trim().to_string())
    };
    let proto = h("x-forwarded-proto")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "http".into());
    let host = h("x-forwarded-host")
        .filter(|s| !s.is_empty())
        .or_else(|| h("host"))
        .unwrap_or_else(|| state.cfg.bind_addr.clone());
    format!("{proto}://{host}")
}

/// 401 that tells MCP clients where to start OAuth (RFC 9728 §5.1).
pub fn unauthorized(state: &AppState, headers: &HeaderMap) -> Response {
    let mut res = (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error":"unauthorized"})),
    )
        .into_response();
    if state.cfg.oauth.enabled {
        let v = format!(
            "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\"",
            base_url(state, headers)
        );
        if let Ok(hv) = HeaderValue::from_str(&v) {
            res.headers_mut().insert(header::WWW_AUTHENTICATE, hv);
        }
    }
    res
}

/// An OAuth access token → the client it acts as. The key must still exist; scope is the
/// lower of what was granted and what the key allows now.
pub fn authenticate_token(state: &AppState, token: &str) -> Option<ClientCtx> {
    if !state.cfg.oauth.enabled {
        return None;
    }
    let grant = {
        let store = state.oauth.lock().unwrap();
        store.tokens.get(&hash(token)).cloned()?
    };
    if grant.kind != Kind::Access || grant.expires_at <= now_unix() {
        return None;
    }
    let key = state
        .cfg
        .clients
        .iter()
        .find(|c| c.name == grant.key_name)?;
    let scope = if grant.write && key.scope == Scope::Write {
        Scope::Write
    } else {
        Scope::Read
    };
    Some(ClientCtx {
        name: key.name.clone(),
        scope,
    })
}

fn disabled() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error":"oauth is disabled (MCP_OAUTH=false)"})),
    )
        .into_response()
}

fn no_store(mut res: Response) -> Response {
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

// ── Metadata ────────────────────────────────────────────────────────────────

pub async fn protected_resource(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !state.cfg.oauth.enabled {
        return disabled();
    }
    let base = base_url(&state, &headers);
    Json(json!({
        "resource": format!("{base}/mcp"),
        "authorization_servers": [base],
        "bearer_methods_supported": ["header"],
        "scopes_supported": ["cdc:read", "cdc:write"],
        "resource_name": "Hyper CDC Orchestrator",
    }))
    .into_response()
}

pub async fn authorization_server(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !state.cfg.oauth.enabled {
        return disabled();
    }
    let base = base_url(&state, &headers);
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none", "client_secret_post"],
        "scopes_supported": ["cdc:read", "cdc:write"],
    }))
    .into_response()
}

// ── Dynamic client registration (RFC 7591) ──────────────────────────────────

/// https anywhere; http only for loopback (desktop apps, MCP Inspector).
pub fn redirect_allowed(uri: &str) -> bool {
    let Ok(u) = url_parse(uri) else { return false };
    if u.fragment {
        return false;
    }
    match u.scheme.as_str() {
        "https" => !u.host.is_empty(),
        "http" => matches!(u.host.as_str(), "localhost" | "127.0.0.1" | "[::1]"),
        _ => false,
    }
}

struct MiniUrl {
    scheme: String,
    host: String,
    fragment: bool,
}

/// Enough URL parsing for redirect checks without another dependency.
fn url_parse(s: &str) -> Result<MiniUrl, ()> {
    let (scheme, rest) = s.split_once("://").ok_or(())?;
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
    {
        return Err(());
    }
    if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(());
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = if host_port.starts_with('[') {
        host_port
            .split(']')
            .next()
            .map(|h| format!("{h}]"))
            .unwrap_or_default()
    } else {
        host_port.split(':').next().unwrap_or("").to_string()
    };
    Ok(MiniUrl {
        scheme: scheme.to_ascii_lowercase(),
        host: host.to_ascii_lowercase(),
        fragment: s.contains('#'),
    })
}

pub async fn register(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    if !state.cfg.oauth.enabled {
        return disabled();
    }
    let uris: Vec<String> = body
        .get("redirect_uris")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if uris.is_empty() || uris.len() > 10 || !uris.iter().all(|u| redirect_allowed(u)) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid_redirect_uri","error_description":"redirect_uris must be https (or http on localhost), without fragments"})),
        )
            .into_response();
    }
    let name: String = body
        .get("client_name")
        .and_then(Value::as_str)
        .unwrap_or("MCP client")
        .chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect();
    let client = RegisteredClient {
        client_id: format!("mcp_{}", random_token()),
        client_name: name,
        redirect_uris: uris,
        created_at: now_unix(),
    };
    {
        let mut store = state.oauth.lock().unwrap();
        if store.clients.len() >= MAX_CLIENTS {
            if let Some(oldest) = store
                .clients
                .values()
                .min_by_key(|c| c.created_at)
                .map(|c| c.client_id.clone())
            {
                store.clients.remove(&oldest);
            }
        }
        store
            .clients
            .insert(client.client_id.clone(), client.clone());
        store.save(state.cfg.oauth.store_path.as_ref());
    }
    tracing::info!(client = %client.client_name, "oauth client registered");
    no_store(
        (
            StatusCode::CREATED,
            Json(json!({
                "client_id": client.client_id,
                "client_id_issued_at": client.created_at,
                "client_name": client.client_name,
                "redirect_uris": client.redirect_uris,
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none",
            })),
        )
            .into_response(),
    )
}

// ── Authorization (consent page) ────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct AuthorizeParams {
    #[serde(default)]
    response_type: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    redirect_uri: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    code_challenge: String,
    #[serde(default)]
    code_challenge_method: String,
    // Consent form only.
    #[serde(default)]
    key: String,
    #[serde(default)]
    decision: String,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn page(title: &str, body: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{title}</title><style>\
         :root{{color-scheme:light dark;--bg:#f7f7f8;--card:#fff;--fg:#111;--muted:#666;--border:#ddd;--accent:#e29749}}\
         @media (prefers-color-scheme:dark){{:root{{--bg:#111;--card:#1b1b1d;--fg:#eee;--muted:#9a9a9a;--border:#333}}}}\
         body{{margin:0;background:var(--bg);color:var(--fg);font:15px/1.5 -apple-system,Segoe UI,sans-serif;display:flex;min-height:100vh;align-items:center;justify-content:center;padding:16px}}\
         .card{{background:var(--card);border:1px solid var(--border);border-radius:14px;padding:28px;max-width:440px;width:100%}}\
         h1{{font-size:18px;margin:0 0 6px}}p{{color:var(--muted);margin:0 0 16px}}label{{display:block;font-size:13px;color:var(--muted);margin-bottom:6px}}\
         input[type=password]{{width:100%;box-sizing:border-box;height:40px;border:1px solid var(--border);border-radius:8px;padding:0 12px;background:transparent;color:var(--fg);font:inherit}}\
         .row{{display:flex;gap:8px;justify-content:flex-end;margin-top:18px}}button{{height:38px;border-radius:8px;padding:0 16px;font:inherit;cursor:pointer;border:1px solid var(--border);background:transparent;color:var(--fg)}}\
         button.primary{{background:var(--accent);border-color:var(--accent);color:#111;font-weight:600}}.err{{color:#dc2626;font-size:13px;margin-top:10px}}code{{font-size:13px}}\
         </style></head><body><main class=\"card\">{body}</main></body></html>"
    );
    let mut res = Html(html).into_response();
    let h = res.headers_mut();
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; form-action 'self' https: http://localhost:* http://127.0.0.1:*; frame-ancestors 'none'"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    res
}

fn error_page(msg: &str) -> Response {
    let mut r = page(
        "Authorization error",
        &format!("<h1>Can't authorize</h1><p>{}</p>", esc(msg)),
    );
    *r.status_mut() = StatusCode::BAD_REQUEST;
    r
}

/// Validates the request against the registered client; on success returns its name and the
/// redirect URI to use. Errors are shown on a page, never sent to an unverified redirect.
fn check_request(state: &AppState, p: &AuthorizeParams) -> Result<(String, String), String> {
    if p.response_type != "code" {
        return Err("response_type must be code".into());
    }
    if p.code_challenge.len() < 43 || p.code_challenge_method != "S256" {
        return Err("PKCE with code_challenge_method=S256 is required".into());
    }
    let store = state.oauth.lock().unwrap();
    let client = store
        .clients
        .get(&p.client_id)
        .ok_or("unknown client_id; register the client first")?;
    let redirect = if p.redirect_uri.is_empty() && client.redirect_uris.len() == 1 {
        client.redirect_uris[0].clone()
    } else if client.redirect_uris.iter().any(|u| u == &p.redirect_uri) {
        p.redirect_uri.clone()
    } else {
        return Err("redirect_uri is not registered for this client".into());
    };
    Ok((client.client_name.clone(), redirect))
}

fn wants_write(scope: &str) -> bool {
    // Default (no scope) asks for whatever the key allows.
    scope.trim().is_empty() || scope.split_whitespace().any(|s| s == "cdc:write")
}

fn consent_form(
    p: &AuthorizeParams,
    client_name: &str,
    redirect: &str,
    error: Option<&str>,
) -> Response {
    let hidden =
        |n: &str, v: &str| format!("<input type=\"hidden\" name=\"{n}\" value=\"{}\">", esc(v));
    let access = if wants_write(&p.scope) {
        "read and, if the key allows, change"
    } else {
        "read"
    };
    let host = url_parse(redirect).map(|u| u.host).unwrap_or_default();
    page(
        "Authorize access to CDC Orchestrator",
        &format!(
            "<h1>Connect {name} to CDC Orchestrator</h1>\
             <p><strong>{name}</strong> ({host}) wants to {access} your CDC pipelines, connectors and alerts. \
             Paste an MCP client key to approve; the app acts as that client and with its scope.</p>\
             <form method=\"post\" action=\"/oauth/authorize\" autocomplete=\"off\">{h}\
             <label for=\"key\">MCP client key</label>\
             <input id=\"key\" name=\"key\" type=\"password\" required autofocus>\
             {err}<div class=\"row\"><button name=\"decision\" value=\"deny\" formnovalidate>Deny</button>\
             <button class=\"primary\" name=\"decision\" value=\"approve\">Approve</button></div></form>",
            name = esc(client_name),
            host = esc(&host),
            h = [
                hidden("response_type", "code"),
                hidden("client_id", &p.client_id),
                hidden("redirect_uri", redirect),
                hidden("state", &p.state),
                hidden("scope", &p.scope),
                hidden("code_challenge", &p.code_challenge),
                hidden("code_challenge_method", &p.code_challenge_method),
            ]
            .join(""),
            err = error.map(|e| format!("<div class=\"err\">{}</div>", esc(e))).unwrap_or_default(),
        ),
    )
}

pub async fn authorize_get(
    State(state): State<AppState>,
    Query(p): Query<AuthorizeParams>,
) -> Response {
    if !state.cfg.oauth.enabled {
        return disabled();
    }
    match check_request(&state, &p) {
        Ok((name, redirect)) => consent_form(&p, &name, &redirect, None),
        Err(e) => error_page(&e),
    }
}

fn redirect_with(redirect: &str, params: &[(&str, &str)]) -> Response {
    let q: Vec<String> = params
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}={}", crate::urlencoding_simple(v)))
        .collect();
    let sep = if redirect.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{redirect}{sep}{}", q.join("&"))).into_response()
}

fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "direct".into())
}

pub async fn authorize_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(p): Form<AuthorizeParams>,
) -> Response {
    if !state.cfg.oauth.enabled {
        return disabled();
    }
    let (name, redirect) = match check_request(&state, &p) {
        Ok(v) => v,
        Err(e) => return error_page(&e),
    };
    let issuer = base_url(&state, &headers);
    if p.decision != "approve" {
        return redirect_with(
            &redirect,
            &[
                ("error", "access_denied"),
                ("state", &p.state),
                ("iss", &issuer),
            ],
        );
    }
    let ip = client_ip(&headers);
    {
        let mut store = state.oauth.lock().unwrap();
        if let Some((n, since)) = store.failures.get(&ip).copied() {
            if n >= MAX_FAILURES && since.elapsed() < LOCKOUT {
                return consent_form(
                    &p,
                    &name,
                    &redirect,
                    Some("Too many wrong keys. Try again in 15 minutes."),
                );
            }
            if since.elapsed() >= LOCKOUT {
                store.failures.remove(&ip);
            }
        }
    }
    let key = p.key.trim();
    let mut matched: Option<(String, Scope)> = None;
    for c in &state.cfg.clients {
        if bool::from(key.as_bytes().ct_eq(c.token.as_bytes())) {
            matched = Some((c.name.clone(), c.scope));
        }
    }
    let Some((key_name, key_scope)) = matched else {
        let mut store = state.oauth.lock().unwrap();
        let e = store.failures.entry(ip).or_insert((0, Instant::now()));
        e.0 += 1;
        tracing::warn!(client = %name, "oauth consent: wrong key");
        return consent_form(&p, &name, &redirect, Some("That key is not valid."));
    };
    let code = random_token();
    {
        let mut store = state.oauth.lock().unwrap();
        store.failures.remove(&ip);
        store.codes.retain(|_, c| c.expires > Instant::now());
        store.codes.insert(
            hash(&code),
            CodeGrant {
                client_id: p.client_id.clone(),
                redirect_uri: redirect.clone(),
                challenge: p.code_challenge.clone(),
                key_name: key_name.clone(),
                write: wants_write(&p.scope) && key_scope == Scope::Write,
                expires: Instant::now() + CODE_TTL,
            },
        );
    }
    tracing::info!(client = %name, key = %key_name, "oauth consent approved");
    redirect_with(
        &redirect,
        &[("code", &code), ("state", &p.state), ("iss", &issuer)],
    )
}

// ── Token endpoint ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct TokenParams {
    #[serde(default)]
    grant_type: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    redirect_uri: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    code_verifier: String,
    #[serde(default)]
    refresh_token: String,
}

fn token_error(code: &str, desc: &str) -> Response {
    no_store(
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": code, "error_description": desc})),
        )
            .into_response(),
    )
}

fn issue(
    state: &AppState,
    store: &mut OAuthStore,
    client_id: &str,
    key_name: &str,
    write: bool,
) -> Value {
    let access = random_token();
    let refresh = random_token();
    let now = now_unix();
    let a = state.cfg.oauth.access_ttl.as_secs() as i64;
    let r = state.cfg.oauth.refresh_ttl.as_secs() as i64;
    store.tokens.insert(
        hash(&access),
        TokenGrant {
            kind: Kind::Access,
            client_id: client_id.into(),
            key_name: key_name.into(),
            write,
            expires_at: now + a,
        },
    );
    store.tokens.insert(
        hash(&refresh),
        TokenGrant {
            kind: Kind::Refresh,
            client_id: client_id.into(),
            key_name: key_name.into(),
            write,
            expires_at: now + r,
        },
    );
    store.save(state.cfg.oauth.store_path.as_ref());
    json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": a,
        "refresh_token": refresh,
        "scope": if write { "cdc:read cdc:write" } else { "cdc:read" },
    })
}

pub async fn token(State(state): State<AppState>, Form(p): Form<TokenParams>) -> Response {
    if !state.cfg.oauth.enabled {
        return disabled();
    }
    match p.grant_type.as_str() {
        "authorization_code" => exchange_code(&state, &p),
        "refresh_token" => exchange_refresh(&state, &p),
        _ => token_error(
            "unsupported_grant_type",
            "use authorization_code or refresh_token",
        ),
    }
}

fn exchange_code(state: &AppState, p: &TokenParams) -> Response {
    let mut store = state.oauth.lock().unwrap();
    // Single use: removed whatever happens next.
    let Some(grant) = store.codes.remove(&hash(&p.code)) else {
        return token_error("invalid_grant", "unknown or used authorization code");
    };
    if grant.expires < Instant::now() {
        return token_error("invalid_grant", "authorization code expired");
    }
    let redirect_mismatch = !p.redirect_uri.is_empty() && grant.redirect_uri != p.redirect_uri;
    if grant.client_id != p.client_id || redirect_mismatch {
        return token_error(
            "invalid_grant",
            "client_id or redirect_uri does not match the authorization request",
        );
    }
    let verifier_ok = bool::from(
        pkce_s256(&p.code_verifier)
            .as_bytes()
            .ct_eq(grant.challenge.as_bytes()),
    );
    if !verifier_ok {
        return token_error(
            "invalid_grant",
            "code_verifier does not match code_challenge",
        );
    }
    let body = issue(
        state,
        &mut store,
        &grant.client_id,
        &grant.key_name,
        grant.write,
    );
    no_store(Json(body).into_response())
}

fn exchange_refresh(state: &AppState, p: &TokenParams) -> Response {
    let mut store = state.oauth.lock().unwrap();
    let h = hash(&p.refresh_token);
    let Some(grant) = store.tokens.remove(&h) else {
        return token_error("invalid_grant", "unknown refresh token");
    }; // removed = rotated
    if grant.kind != Kind::Refresh || grant.expires_at <= now_unix() {
        return token_error("invalid_grant", "refresh token expired");
    }
    if !p.client_id.is_empty() && p.client_id != grant.client_id {
        return token_error("invalid_grant", "refresh token belongs to another client");
    }
    let Some(key) = state.cfg.clients.iter().find(|c| c.name == grant.key_name) else {
        return token_error("invalid_grant", "the MCP key behind this grant was removed");
    };
    let write = grant.write && key.scope == Scope::Write;
    let body = issue(state, &mut store, &grant.client_id, &grant.key_name, write);
    no_store(Json(body).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc7636_example() {
        // RFC 7636 Appendix B.
        assert_eq!(
            pkce_s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn only_https_or_loopback_redirects_are_registered() {
        assert!(redirect_allowed("https://claude.ai/api/mcp/auth_callback"));
        assert!(redirect_allowed(
            "https://chatgpt.com/connector_platform_oauth_redirect"
        ));
        assert!(redirect_allowed("http://localhost:6274/oauth/callback"));
        assert!(redirect_allowed("http://127.0.0.1:33418/callback"));
        for bad in [
            "http://evil.example.com/cb",
            "https://x.example.com/cb#frag",
            "javascript:alert(1)",
            "ftp://x/y",
            "https:///nohost",
            "https://a b/c",
        ] {
            assert!(!redirect_allowed(bad), "{bad}");
        }
    }

    #[test]
    fn scope_defaults_to_what_the_key_allows() {
        assert!(wants_write(""));
        assert!(wants_write("cdc:read cdc:write"));
        assert!(!wants_write("cdc:read"));
    }

    #[test]
    fn html_is_escaped() {
        assert_eq!(esc("<b a=\"x\">&'"), "&lt;b a=&quot;x&quot;&gt;&amp;&#39;");
    }
}
