//! SSH tunnels: "Connection method = SSH tunnel" for pipeline sources/targets and the Copy &
//! Sync tools (ADR 0020).
//!
//! For sites that expose no database port, only SSH (often a Cloudflare Zero Trust SSH app).
//! The `cdc-sshtunnel` gateway keeps one SSH session per tunnel and listens on
//! `cdc-sshtunnel:<listen_port>` for each forward (`ssh -L`).
//!
//! **Forwards are declared here, with a fixed `listen_port`, and stored on the tunnel.** A
//! Debezium connector keeps host:port in its config, so the port must not change when the
//! gateway restarts (WireGuard forwards are allocated on demand and would).
//!
//! **The connection is materialized, not rewritten at every egress.** A connection that uses a
//! tunnel keeps the operator's intent in `ssh_tunnel_id` / `ssh_remote_host` / `ssh_remote_port`
//! and gets `host = cdc-sshtunnel`, `port = <listen_port>`. Kafka Connect, jdbc-admin, the native
//! Postgres/MySQL clients and the TCP probes then all work unchanged.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::error::AppError;
use crate::AppState;
use hyper_cdc_orchestrator::persist::{RqliteClient, KIND_SSH_TUNNEL};

pub const REDACTED: &str = "********";

/// Connection keys that carry the SSH routing intent.
pub const KEY_TUNNEL: &str = "ssh_tunnel_id";
pub const KEY_REMOTE_HOST: &str = "ssh_remote_host";
pub const KEY_REMOTE_PORT: &str = "ssh_remote_port";

// ── model ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProxyConfig {
    /// `none` (direct TCP to host:port) or `cloudflare_access` (`cloudflared access ssh`).
    #[serde(default)]
    pub kind: String,
    /// Cloudflare Access application hostname (defaults to `host`).
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub service_token_id: String,
    /// Secret.
    #[serde(default)]
    pub service_token_secret: String,
}

impl ProxyConfig {
    fn is_cloudflare(&self) -> bool {
        self.kind == "cloudflare_access"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshForward {
    /// Database host as seen from the SSH server (often `127.0.0.1`).
    pub remote_host: String,
    pub remote_port: u16,
    /// Port on `cdc-sshtunnel`; allocated once and never changed.
    pub listen_port: u16,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshTunnel {
    pub id: Uuid,
    pub name: String,
    /// SSH server (or the Cloudflare Access hostname when `proxy.kind = cloudflare_access`).
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    pub username: String,
    /// `key` or `password`.
    #[serde(default)]
    pub auth_method: String,
    /// Secret (OpenSSH / PEM private key).
    #[serde(default)]
    pub private_key: String,
    /// Secret.
    #[serde(default)]
    pub passphrase: String,
    /// Secret.
    #[serde(default)]
    pub password: String,
    /// Pinned server host key, `SHA256:…` (`ssh-keygen -lf`). Required before the tunnel runs.
    #[serde(default)]
    pub host_key_fingerprint: String,
    #[serde(default)]
    pub keepalive: Option<u32>,
    #[serde(default)]
    pub proxy: ProxyConfig,
    #[serde(default)]
    pub forwards: Vec<SshForward>,
    #[serde(default)]
    pub notes: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SshTunnel {
    pub fn redacted(mut self) -> Self {
        for s in [
            &mut self.private_key,
            &mut self.passphrase,
            &mut self.password,
            &mut self.proxy.service_token_secret,
        ] {
            if !s.is_empty() {
                *s = REDACTED.into();
            }
        }
        self
    }

    /// The login the stored secrets belong to. A redacted secret is restored only while this
    /// is unchanged, so editing the host cannot send a saved key/password to another server.
    fn login(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.host.trim().to_ascii_lowercase(),
            self.port.unwrap_or(22),
            self.username.trim(),
            self.proxy.kind,
            self.proxy.hostname.trim().to_ascii_lowercase()
        )
    }

    /// What the gateway needs (plain secrets; never returned by the API).
    fn gateway_spec(&self, cfg: &AppConfig) -> Result<Value, AppError> {
        let mut t = self.clone();
        open_secrets(cfg, &mut t)?;
        Ok(json!({
            "id": t.id.to_string(),
            "name": t.name,
            "version": t.updated_at.to_rfc3339(),
            "host": t.host,
            "port": t.port.unwrap_or(22),
            "username": t.username,
            "private_key": if t.auth_method == "password" { "" } else { t.private_key.as_str() },
            "passphrase": t.passphrase,
            "password": if t.auth_method == "password" { t.password.as_str() } else { "" },
            "host_key_fingerprint": t.host_key_fingerprint,
            "keepalive": t.keepalive.unwrap_or(15),
            "proxy": {
                "kind": if t.proxy.kind.is_empty() { "none" } else { t.proxy.kind.as_str() },
                "hostname": t.proxy.hostname,
                "service_token_id": t.proxy.service_token_id,
                "service_token_secret": t.proxy.service_token_secret,
            },
            "forwards": t.forwards.iter().map(|f| json!({
                "remote_host": f.remote_host,
                "remote_port": f.remote_port,
                "listen_port": f.listen_port,
            })).collect::<Vec<_>>(),
        }))
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SshTunnelRequest {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub auth_method: String,
    #[serde(default)]
    pub private_key: String,
    #[serde(default)]
    pub passphrase: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub host_key_fingerprint: String,
    #[serde(default)]
    pub keepalive: Option<u32>,
    #[serde(default)]
    pub proxy: ProxyConfig,
    #[serde(default)]
    pub notes: String,
}

// ── secrets at rest (ADR 0015 secretbox) ────────────────────────────────────

fn seal(cfg: &AppConfig, v: &str) -> String {
    match &cfg.secret_key {
        Some(k) if !v.is_empty() && !v.starts_with("v1:") => k.encrypt(v),
        _ => v.to_string(),
    }
}

fn unseal(cfg: &AppConfig, v: &str) -> Result<String, AppError> {
    if !v.starts_with("v1:") {
        return Ok(v.to_string());
    }
    let key = cfg.secret_key.as_ref().ok_or_else(|| {
        AppError::BadRequest("a stored secret is encrypted but ORCH_SECRET_KEY is not set".into())
    })?;
    key.decrypt(v).map_err(AppError::BadRequest)
}

fn seal_secrets(cfg: &AppConfig, t: &mut SshTunnel) {
    t.private_key = seal(cfg, &t.private_key);
    t.passphrase = seal(cfg, &t.passphrase);
    t.password = seal(cfg, &t.password);
    t.proxy.service_token_secret = seal(cfg, &t.proxy.service_token_secret);
}

fn open_secrets(cfg: &AppConfig, t: &mut SshTunnel) -> Result<(), AppError> {
    t.private_key = unseal(cfg, &t.private_key)?;
    t.passphrase = unseal(cfg, &t.passphrase)?;
    t.password = unseal(cfg, &t.password)?;
    t.proxy.service_token_secret = unseal(cfg, &t.proxy.service_token_secret)?;
    Ok(())
}

// ── store ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SshTunnelStore {
    memory: Arc<RwLock<HashMap<Uuid, SshTunnel>>>,
    rqlite: Option<RqliteClient>,
    /// Serialises forward allocation so two requests never get the same listen port.
    alloc: Arc<Mutex<()>>,
}

impl SshTunnelStore {
    pub fn new(rqlite: Option<RqliteClient>) -> Self {
        Self {
            memory: Arc::new(RwLock::new(HashMap::new())),
            rqlite,
            alloc: Arc::new(Mutex::new(())),
        }
    }

    pub async fn list(&self) -> Result<Vec<SshTunnel>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq.list_json::<SshTunnel>(KIND_SSH_TUNNEL).await {
                Ok(mut list) => {
                    list.sort_by(|a, b| a.name.cmp(&b.name));
                    let mut map = self.memory.write().await;
                    map.clear();
                    for t in &list {
                        map.insert(t.id, t.clone());
                    }
                    return Ok(list);
                }
                Err(e) => warn!(error = %e, "rqlite list ssh tunnels failed; using memory"),
            }
        }
        let mut list: Vec<_> = self.memory.read().await.values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(list)
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<SshTunnel>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq
                .get_json::<SshTunnel>(KIND_SSH_TUNNEL, &id.to_string())
                .await
            {
                Ok(found) => {
                    if let Some(t) = &found {
                        self.memory.write().await.insert(t.id, t.clone());
                    }
                    return Ok(found);
                }
                Err(e) => warn!(error = %e, "rqlite get ssh tunnel failed"),
            }
        }
        Ok(self.memory.read().await.get(&id).cloned())
    }

    pub async fn upsert(&self, t: &SshTunnel) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.put_json(KIND_SSH_TUNNEL, &t.id.to_string(), t).await?;
        }
        self.memory.write().await.insert(t.id, t.clone());
        Ok(())
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.delete(KIND_SSH_TUNNEL, &id.to_string()).await?;
        }
        self.memory.write().await.remove(&id);
        Ok(())
    }
}

// ── gateway client ──────────────────────────────────────────────────────────

fn gateway_url(state: &AppState) -> Result<&str, AppError> {
    state.config.sshtunnel_url.as_deref().ok_or_else(|| {
        AppError::BadRequest(
            "SSH tunnels are not configured: start cdc-sshtunnel and set ORCH_SSHTUNNEL_URL".into(),
        )
    })
}

async fn gateway(
    state: &AppState,
    method: Method,
    path: &str,
    body: Option<Value>,
    wait: Duration,
) -> Result<Value, AppError> {
    let url = format!("{}{}", gateway_url(state)?, path);
    let mut req = state.http.request(method, &url).timeout(wait);
    if let Some(t) = &state.config.sshtunnel_token {
        req = req.bearer_auth(t);
    }
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("cdc-sshtunnel unreachable: {e}")))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = v
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("request failed")
            .to_string();
        return Err(match status.as_u16() {
            400 => AppError::BadRequest(format!("SSH tunnel: {msg}")),
            401 => {
                AppError::Upstream("cdc-sshtunnel rejected the token (ORCH_SSHTUNNEL_TOKEN)".into())
            }
            404 => AppError::NotFound(format!("cdc-sshtunnel: {msg}")),
            _ => AppError::Upstream(format!("cdc-sshtunnel: {msg}")),
        });
    }
    Ok(v)
}

const SHORT: Duration = Duration::from_secs(10);
/// SSH login through Cloudflare + a dial can take a while.
const PROBE: Duration = Duration::from_secs(45);

async fn push(state: &AppState, t: &SshTunnel) -> Result<Value, AppError> {
    if t.host_key_fingerprint.trim().is_empty() {
        return Err(AppError::BadRequest(format!(
            "tunnel {} has no pinned host key: test it and confirm the fingerprint",
            t.name
        )));
    }
    gateway(
        state,
        Method::PUT,
        &format!("/v1/tunnels/{}", t.id),
        Some(t.gateway_spec(&state.config)?),
        SHORT,
    )
    .await
}

/// Keeps the gateway in step with the store (it restarts with no tunnels).
pub fn spawn_reconcile(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if state.config.sshtunnel_url.is_none() {
            return;
        }
        loop {
            if let Err(e) = reconcile(&state).await {
                warn!(error = %e, "ssh tunnel reconcile failed");
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    })
}

async fn reconcile(state: &AppState) -> Result<(), AppError> {
    let live = gateway(state, Method::GET, "/v1/tunnels", None, SHORT).await?;
    let mut versions: HashMap<String, String> = HashMap::new();
    for t in live
        .get("tunnels")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        if let (Some(id), Some(v)) = (
            t.get("id").and_then(Value::as_str),
            t.get("version").and_then(Value::as_str),
        ) {
            versions.insert(id.to_string(), v.to_string());
        }
    }
    let stored = state.ssh_tunnels.list().await?;
    for t in &stored {
        if t.host_key_fingerprint.trim().is_empty() {
            continue;
        }
        if versions.get(&t.id.to_string()) != Some(&t.updated_at.to_rfc3339()) {
            match push(state, t).await {
                Ok(_) => info!(tunnel = %t.name, "ssh tunnel pushed to gateway"),
                Err(e) => warn!(tunnel = %t.name, error = %e, "ssh tunnel push failed"),
            }
        }
    }
    for id in versions.keys() {
        if !stored.iter().any(|t| &t.id.to_string() == id) {
            let _ = gateway(
                state,
                Method::DELETE,
                &format!("/v1/tunnels/{id}"),
                None,
                SHORT,
            )
            .await;
        }
    }
    Ok(())
}

// ── forwards ────────────────────────────────────────────────────────────────

/// Lowest free port in `[min, max]` not used by any tunnel's forward.
fn next_free_port(all: &[SshTunnel], min: u16, max: u16) -> Option<u16> {
    let used: std::collections::HashSet<u16> = all
        .iter()
        .flat_map(|t| t.forwards.iter().map(|f| f.listen_port))
        .collect();
    (min..=max).find(|p| !used.contains(p))
}

fn find_forward(t: &SshTunnel, host: &str, port: u16) -> Option<u16> {
    t.forwards
        .iter()
        .find(|f| f.remote_host.eq_ignore_ascii_case(host) && f.remote_port == port)
        .map(|f| f.listen_port)
}

/// Returns the stable `cdc-sshtunnel` port for `host:port` behind tunnel `tid`, allocating and
/// persisting it the first time. A new forward is pushed to the gateway right away (best effort:
/// the reconcile loop retries); it never restarts the tunnel's SSH session.
pub async fn ensure_forward(
    state: &AppState,
    tid: Uuid,
    host: &str,
    port: u16,
) -> Result<u16, AppError> {
    let host = host.trim();
    if host.is_empty() || port == 0 {
        return Err(AppError::BadRequest(
            "SSH tunnel: the database host and port (as seen from the SSH server) are required"
                .into(),
        ));
    }
    let _guard = state.ssh_tunnels.alloc.lock().await;
    let mut t =
        state.ssh_tunnels.get(tid).await?.ok_or_else(|| {
            AppError::BadRequest(format!("SSH tunnel {tid} not found (deleted?)"))
        })?;
    if let Some(p) = find_forward(&t, host, port) {
        return Ok(p);
    }
    let all = state.ssh_tunnels.list().await?;
    let (min, max) = (
        state.config.sshtunnel_port_min,
        state.config.sshtunnel_port_max,
    );
    let listen_port = next_free_port(&all, min, max).ok_or_else(|| {
        AppError::BadRequest(format!("no free SSH tunnel forward port in {min}-{max}"))
    })?;
    t.forwards.push(SshForward {
        remote_host: host.to_string(),
        remote_port: port,
        listen_port,
        created_at: Utc::now(),
    });
    t.updated_at = Utc::now();
    state.ssh_tunnels.upsert(&t).await?;
    info!(tunnel = %t.name, listen_port, remote = %format!("{host}:{port}"), "ssh tunnel forward allocated");
    if state.config.sshtunnel_url.is_some() {
        if let Err(e) = push(state, &t).await {
            warn!(tunnel = %t.name, error = %e, "ssh tunnel push after new forward failed");
        }
    }
    Ok(listen_port)
}

fn default_port(kind: &str) -> u16 {
    match kind.trim().to_ascii_lowercase().as_str() {
        "mysql" | "mariadb" => 3306,
        "sqlserver" | "mssql" => 1433,
        "oracle" => 1521,
        "redis" | "valkey" | "tair" => 6379,
        "mongodb" | "mongo" => 27017,
        "ssh" => 22,
        "rsync" => 873,
        _ => 5432,
    }
}

/// Applies the connection method to one connection map (`kind` = engine, for the default port).
///
/// - `ssh_tunnel_id` set: `host`/`port` become the tunnel forward; the database address the
///   operator typed is kept in `ssh_remote_host`/`ssh_remote_port`. Returns the tunnel name.
/// - `ssh_tunnel_id` empty (Direct): a connection that used to be tunnelled gets its real
///   host/port back and the `ssh_*` keys are removed.
pub async fn materialize_conn(
    state: &AppState,
    kind: &str,
    conn: &mut HashMap<String, String>,
) -> Result<Option<String>, AppError> {
    let forward_host = state.config.sshtunnel_forward_host.as_str();
    let tid_raw = conn
        .get(KEY_TUNNEL)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let (remote_host, remote_port) = intended_endpoint(conn, forward_host, kind);
    if tid_raw.is_empty() {
        restore_direct(conn, forward_host);
        return Ok(None);
    }
    let tid = Uuid::parse_str(&tid_raw)
        .map_err(|_| AppError::BadRequest(format!("ssh_tunnel_id is not a UUID: {tid_raw}")))?;
    let listen_port = ensure_forward(state, tid, &remote_host, remote_port).await?;
    conn.insert(KEY_REMOTE_HOST.into(), remote_host);
    conn.insert(KEY_REMOTE_PORT.into(), remote_port.to_string());
    conn.insert("host".into(), forward_host.to_string());
    conn.insert("port".into(), listen_port.to_string());
    let name = state
        .ssh_tunnels
        .get(tid)
        .await?
        .map(|t| t.name)
        .unwrap_or_default();
    Ok(Some(name))
}

/// The database address the operator means: `ssh_remote_*` when set, otherwise `host`/`port`
/// unless they already point at the gateway (a profile saved earlier).
fn intended_endpoint(
    conn: &HashMap<String, String>,
    forward_host: &str,
    kind: &str,
) -> (String, u16) {
    let get = |k: &str| {
        conn.get(k)
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    let host_is_gateway = get("host").eq_ignore_ascii_case(forward_host);
    let mut host = get(KEY_REMOTE_HOST);
    if host.is_empty() && !host_is_gateway {
        host = get("host");
    }
    let mut port = get(KEY_REMOTE_PORT).parse::<u16>().ok();
    if port.is_none() && !host_is_gateway {
        port = get("port").parse::<u16>().ok();
    }
    (host, port.unwrap_or_else(|| default_port(kind)))
}

fn restore_direct(conn: &mut HashMap<String, String>, forward_host: &str) {
    let host_is_gateway = conn
        .get("host")
        .is_some_and(|h| h.trim().eq_ignore_ascii_case(forward_host));
    if host_is_gateway {
        if let Some(h) = conn.get(KEY_REMOTE_HOST).cloned().filter(|h| !h.is_empty()) {
            conn.insert("host".into(), h);
        }
        if let Some(p) = conn.get(KEY_REMOTE_PORT).cloned().filter(|p| !p.is_empty()) {
            conn.insert("port".into(), p);
        }
    }
    conn.remove(KEY_TUNNEL);
    conn.remove(KEY_REMOTE_HOST);
    conn.remove(KEY_REMOTE_PORT);
}

/// Same as [`materialize_conn`] for a JSON connection object (Copy & Sync payloads). Returns
/// the tunnel name when one was used.
pub async fn materialize_value(
    state: &AppState,
    conn: &mut Value,
) -> Result<Option<String>, AppError> {
    let Some(obj) = conn.as_object() else {
        return Ok(None);
    };
    let has_tunnel = obj
        .get(KEY_TUNNEL)
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty());
    if !has_tunnel && !obj.contains_key(KEY_REMOTE_HOST) {
        return Ok(None);
    }
    let kind = endpoint_kind_of(obj);
    let port_is_number = matches!(obj.get("port"), Some(Value::Number(_)));
    let mut map = routing_keys(obj);
    let used = materialize_conn(state, &kind, &mut map).await?;
    let o = conn.as_object_mut().expect("object");
    write_routing_keys(o, map, port_is_number);
    if let Some(name) = &used {
        o.insert("via_ssh_tunnel".into(), json!(name));
    }
    Ok(used)
}

/// File sync endpoints are SSH hosts (they carry a `path`; `rsync` for daemon mode);
/// databases carry a `type` (or `engine`).
fn endpoint_kind_of(obj: &serde_json::Map<String, Value>) -> String {
    if !obj.contains_key("path") {
        return obj
            .get("type")
            .or_else(|| obj.get("engine"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
    }
    let daemon = obj
        .get("protocol")
        .and_then(Value::as_str)
        .is_some_and(|p| p.eq_ignore_ascii_case("daemon"));
    if daemon { "rsync" } else { "ssh" }.to_string()
}

/// The host/port/ssh_* keys of a JSON connection as strings (numbers included).
fn routing_keys(obj: &serde_json::Map<String, Value>) -> HashMap<String, String> {
    ["host", "port", KEY_TUNNEL, KEY_REMOTE_HOST, KEY_REMOTE_PORT]
        .into_iter()
        .filter_map(|k| {
            let v = match obj.get(k)? {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => return None,
            };
            Some((k.to_string(), v))
        })
        .collect()
}

/// Writes the materialized keys back; the ssh_* intent keys are dropped (the sidecar only needs
/// host/port), and a numeric `port` stays numeric.
fn write_routing_keys(
    o: &mut serde_json::Map<String, Value>,
    map: HashMap<String, String>,
    port_is_number: bool,
) {
    for k in [KEY_TUNNEL, KEY_REMOTE_HOST, KEY_REMOTE_PORT] {
        o.remove(k);
    }
    for (k, v) in map {
        let val = match v.parse::<u64>() {
            Ok(n) if k == "port" && port_is_number => Value::from(n),
            _ => Value::String(v),
        };
        o.insert(k, val);
    }
}

/// Every pipeline that routes a side through tunnel `id` (names, for the delete guard / list).
async fn referencing_profiles(state: &AppState, id: Uuid) -> Vec<String> {
    let ids = id.to_string();
    let uses = |c: &HashMap<String, String>| c.get(KEY_TUNNEL).is_some_and(|v| v.trim() == ids);
    state
        .profiles
        .list()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|p| {
            uses(&p.source.connection) || p.target.as_ref().is_some_and(|t| uses(&t.connection))
        })
        .map(|p| p.name)
        .collect()
}

// ── handlers ────────────────────────────────────────────────────────────────

fn actor(headers: &HeaderMap) -> String {
    headers
        .get("x-user")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .unwrap_or("api")
        .to_string()
}

fn normalize_fp(fp: &str) -> String {
    let fp = fp.trim().trim_end_matches('=');
    if fp.is_empty() || fp.starts_with("SHA256:") {
        fp.to_string()
    } else {
        format!("SHA256:{fp}")
    }
}

/// Fills redacted secrets from the stored tunnel (same login only).
fn merge_secrets(t: &mut SshTunnel, old: Option<&SshTunnel>) -> Result<(), AppError> {
    let same_login = old.is_some_and(|o| o.login() == t.login());
    let pairs: [(&mut String, Option<&String>); 4] = [
        (&mut t.private_key, old.map(|o| &o.private_key)),
        (&mut t.passphrase, old.map(|o| &o.passphrase)),
        (&mut t.password, old.map(|o| &o.password)),
        (
            &mut t.proxy.service_token_secret,
            old.map(|o| &o.proxy.service_token_secret),
        ),
    ];
    for (v, prev) in pairs {
        if v.as_str() == REDACTED {
            if !same_login {
                return Err(AppError::BadRequest(
                    "the SSH login changed (host, port, user or proxy): enter the secrets again"
                        .into(),
                ));
            }
            *v = prev.cloned().unwrap_or_default();
        }
    }
    Ok(())
}

fn parse_proxy_kind(raw: &str) -> Result<&'static str, AppError> {
    match raw.trim() {
        "" | "none" => Ok("none"),
        "cloudflare_access" => Ok("cloudflare_access"),
        other => Err(AppError::BadRequest(format!(
            "proxy.kind must be none or cloudflare_access, not {other}"
        ))),
    }
}

fn parse_auth_method(raw: &str) -> Result<&'static str, AppError> {
    match raw.trim() {
        "" | "key" => Ok("key"),
        "password" => Ok("password"),
        other => Err(AppError::BadRequest(format!(
            "auth_method must be key or password, not {other}"
        ))),
    }
}

/// Checks that need the merged secrets (host, credentials for the chosen auth, token pair).
fn validate_tunnel(t: &mut SshTunnel) -> Result<(), AppError> {
    let cloudflare = t.proxy.is_cloudflare();
    if t.host.is_empty() && !(cloudflare && !t.proxy.hostname.is_empty()) {
        return Err(AppError::BadRequest("host is required".into()));
    }
    if cloudflare && t.proxy.hostname.is_empty() {
        t.proxy.hostname = t.host.clone();
    }
    let missing = match t.auth_method.as_str() {
        "key" => t.private_key.is_empty(),
        "password" => t.password.is_empty(),
        _ => false,
    };
    if missing {
        return Err(AppError::BadRequest(format!(
            "a {} is required",
            if t.auth_method == "key" {
                "private key"
            } else {
                "password"
            }
        )));
    }
    if cloudflare && t.proxy.service_token_id.is_empty() != t.proxy.service_token_secret.is_empty()
    {
        return Err(AppError::BadRequest(
            "Cloudflare service token: give both the client ID and the secret".into(),
        ));
    }
    Ok(())
}

fn build(
    id: Uuid,
    r: SshTunnelRequest,
    old: Option<&SshTunnel>,
    require_name: bool,
) -> Result<SshTunnel, AppError> {
    let proxy_kind = parse_proxy_kind(&r.proxy.kind)?;
    let auth_method = parse_auth_method(&r.auth_method)?;
    if require_name && r.name.trim().is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    if r.username.trim().is_empty() {
        return Err(AppError::BadRequest("username is required".into()));
    }
    let now = Utc::now();
    let mut private_key = r.private_key.trim().to_string();
    if private_key.contains("PRIVATE KEY") && !private_key.ends_with('\n') {
        private_key.push('\n');
    }
    let mut t = SshTunnel {
        id,
        name: r.name.trim().to_string(),
        host: r.host.trim().to_string(),
        port: r.port.filter(|p| *p > 0),
        username: r.username.trim().to_string(),
        auth_method: auth_method.into(),
        private_key,
        passphrase: r.passphrase,
        password: r.password,
        host_key_fingerprint: normalize_fp(&r.host_key_fingerprint),
        keepalive: r.keepalive.filter(|k| *k > 0),
        proxy: ProxyConfig {
            kind: proxy_kind.into(),
            hostname: r.proxy.hostname.trim().to_string(),
            service_token_id: r.proxy.service_token_id.trim().to_string(),
            service_token_secret: r.proxy.service_token_secret.trim().to_string(),
        },
        forwards: old.map(|o| o.forwards.clone()).unwrap_or_default(),
        notes: r.notes.trim().to_string(),
        created_at: old.map(|o| o.created_at).unwrap_or(now),
        updated_at: now,
    };
    merge_secrets(&mut t, old)?;
    validate_tunnel(&mut t)?;
    Ok(t)
}

fn view(state: &AppState, t: SshTunnel, status: Value, used_by: Vec<String>) -> Value {
    let fh = state.config.sshtunnel_forward_host.clone();
    let mut v = serde_json::to_value(t.redacted()).unwrap_or(Value::Null);
    v["status"] = status;
    v["used_by"] = json!(used_by);
    v["forward_host"] = json!(fh);
    v
}

pub async fn list_tunnels(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let stored = state.ssh_tunnels.list().await?;
    let live = if state.config.sshtunnel_url.is_some() {
        gateway(&state, Method::GET, "/v1/tunnels", None, SHORT)
            .await
            .ok()
    } else {
        None
    };
    let status: HashMap<String, Value> = live
        .as_ref()
        .and_then(|v| v.get("tunnels").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|t| {
            t.get("id")
                .and_then(Value::as_str)
                .map(|id| (id.to_string(), t.clone()))
        })
        .collect();
    let profiles = state.profiles.list().await.unwrap_or_default();
    let tunnels: Vec<Value> = stored
        .into_iter()
        .map(|t| {
            let ids = t.id.to_string();
            let used_by = profiles
                .iter()
                .filter(|p| {
                    let uses = |c: &HashMap<String, String>| {
                        c.get(KEY_TUNNEL).is_some_and(|v| v.trim() == ids)
                    };
                    uses(&p.source.connection)
                        || p.target.as_ref().is_some_and(|t| uses(&t.connection))
                })
                .map(|p| p.name.clone())
                .collect();
            let st = status.get(&ids).cloned().unwrap_or(Value::Null);
            view(&state, t, st, used_by)
        })
        .collect();
    Ok(Json(json!({
        "configured": state.config.sshtunnel_url.is_some(),
        "gateway_reachable": live.is_some(),
        "forward_host": state.config.sshtunnel_forward_host,
        "tunnels": tunnels,
    })))
}

pub async fn create_tunnel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<SshTunnelRequest>,
) -> Result<Json<Value>, AppError> {
    let mut t = build(Uuid::new_v4(), r, None, true)?;
    seal_secrets(&state.config, &mut t);
    state.ssh_tunnels.upsert(&t).await?;
    let pushed = if t.host_key_fingerprint.is_empty() {
        Ok(Value::Null)
    } else {
        push(&state, &t).await
    };
    crate::audit::log_action(
        &actor(&headers),
        "ssh_tunnel.create",
        "success",
        &format!("id={} name={}", t.id, t.name),
    );
    let mut out = view(&state, t, Value::Null, vec![]);
    if let Err(e) = pushed {
        out["gateway_error"] = json!(e.to_string());
    }
    Ok(Json(out))
}

pub async fn update_tunnel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(r): Json<SshTunnelRequest>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.ssh_tunnels.alloc.lock().await;
    let mut old = state
        .ssh_tunnels
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("SSH tunnel {id} not found")))?;
    open_secrets(&state.config, &mut old)?;
    let mut t = build(id, r, Some(&old), true)?;
    seal_secrets(&state.config, &mut t);
    state.ssh_tunnels.upsert(&t).await?;
    drop(_guard);
    let pushed = if t.host_key_fingerprint.is_empty() {
        Ok(Value::Null)
    } else {
        push(&state, &t).await
    };
    crate::audit::log_action(
        &actor(&headers),
        "ssh_tunnel.update",
        "success",
        &format!("id={id}"),
    );
    let used_by = referencing_profiles(&state, id).await;
    let mut out = view(&state, t, Value::Null, used_by);
    if let Err(e) = pushed {
        out["gateway_error"] = json!(e.to_string());
    }
    Ok(Json(out))
}

pub async fn delete_tunnel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let used_by = referencing_profiles(&state, id).await;
    if !used_by.is_empty() {
        return Err(AppError::Conflict(format!(
            "SSH tunnel is used by pipeline(s): {} — switch them to Direct or another tunnel first",
            used_by.join(", ")
        )));
    }
    state.ssh_tunnels.delete(id).await?;
    if state.config.sshtunnel_url.is_some() {
        let _ = gateway(
            &state,
            Method::DELETE,
            &format!("/v1/tunnels/{id}"),
            None,
            SHORT,
        )
        .await;
    }
    crate::audit::log_action(
        &actor(&headers),
        "ssh_tunnel.delete",
        "success",
        &format!("id={id}"),
    );
    Ok(Json(json!({ "status": true, "id": id })))
}

#[derive(Debug, Deserialize)]
pub struct TestRequest {
    /// The editor's form (may be unsaved). With `id`, redacted secrets come from that tunnel.
    #[serde(default)]
    pub tunnel: SshTunnelRequest,
    #[serde(default)]
    pub id: Option<Uuid>,
    /// Optional database address (as seen from the SSH server) to dial through the session.
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
}

/// SSH login test for a (possibly unsaved) tunnel, plus an optional dial to the database.
/// An unpinned host key stops before authentication and returns the observed fingerprint.
pub async fn test_tunnel(
    State(state): State<AppState>,
    Json(r): Json<TestRequest>,
) -> Result<Json<Value>, AppError> {
    let old = match r.id {
        Some(id) => {
            let mut o = state
                .ssh_tunnels
                .get(id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("SSH tunnel {id} not found")))?;
            open_secrets(&state.config, &mut o)?;
            Some(o)
        }
        None => None,
    };
    let t = build(
        r.id.unwrap_or_else(Uuid::nil),
        r.tunnel,
        old.as_ref(),
        false,
    )?;
    let mut spec = t.gateway_spec(&state.config)?;
    spec["forwards"] = json!([]);
    let v = gateway(
        &state,
        Method::POST,
        "/v1/probe",
        Some(json!({ "tunnel": spec, "host": r.host.trim(), "port": r.port.unwrap_or(0) })),
        PROBE,
    )
    .await?;
    Ok(Json(v))
}

#[derive(Debug, Deserialize)]
pub struct ProbeRequest {
    pub host: String,
    pub port: u16,
}

/// Dial host:port through the running tunnel session.
pub async fn probe_tunnel(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(r): Json<ProbeRequest>,
) -> Result<Json<Value>, AppError> {
    gateway(
        &state,
        Method::POST,
        &format!("/v1/tunnels/{id}/probe"),
        Some(json!({ "host": r.host.trim(), "port": r.port })),
        PROBE,
    )
    .await
    .map(Json)
}

pub async fn reconnect_tunnel(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    gateway(
        &state,
        Method::POST,
        &format!("/v1/tunnels/{id}/reconnect"),
        None,
        SHORT,
    )
    .await
    .map(Json)
}

pub async fn capability(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    if state.config.sshtunnel_url.is_none() {
        return Ok(Json(json!({ "configured": false })));
    }
    let v = gateway(&state, Method::GET, "/v1/capability", None, SHORT).await;
    Ok(Json(match v {
        Ok(mut v) => {
            v["configured"] = json!(true);
            v["reachable"] = json!(true);
            v
        }
        Err(e) => json!({ "configured": true, "reachable": false, "error": e.to_string() }),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tunnel(forwards: Vec<(u16, &str, u16)>) -> SshTunnel {
        SshTunnel {
            id: Uuid::new_v4(),
            name: "site".into(),
            host: "ssh.site.example".into(),
            port: None,
            username: "cdc".into(),
            auth_method: "key".into(),
            private_key: "KEY".into(),
            passphrase: String::new(),
            password: String::new(),
            host_key_fingerprint: "SHA256:abc".into(),
            keepalive: None,
            proxy: ProxyConfig {
                kind: "cloudflare_access".into(),
                hostname: "ssh.site.example".into(),
                service_token_id: "id".into(),
                service_token_secret: "TOKEN-SECRET".into(),
            },
            forwards: forwards
                .into_iter()
                .map(|(lp, h, p)| SshForward {
                    remote_host: h.into(),
                    remote_port: p,
                    listen_port: lp,
                    created_at: Utc::now(),
                })
                .collect(),
            notes: String::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn conn(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn redaction_hides_every_secret() {
        let t = tunnel(vec![]).redacted();
        assert_eq!(t.private_key, REDACTED);
        assert_eq!(t.proxy.service_token_secret, REDACTED);
        assert_eq!(t.password, "");
        assert_eq!(t.proxy.service_token_id, "id");
    }

    #[test]
    fn free_port_skips_ports_of_all_tunnels() {
        let a = tunnel(vec![(22000, "db", 5432), (22002, "db2", 5432)]);
        let b = tunnel(vec![(22001, "x", 5432)]);
        assert_eq!(next_free_port(&[a.clone(), b], 22000, 22999), Some(22003));
        assert_eq!(next_free_port(&[a], 22000, 22000), None);
    }

    #[test]
    fn forward_lookup_is_case_insensitive_and_port_exact() {
        let t = tunnel(vec![(22005, "DB.internal", 5432)]);
        assert_eq!(find_forward(&t, "db.internal", 5432), Some(22005));
        assert_eq!(find_forward(&t, "db.internal", 5433), None);
    }

    #[test]
    fn intended_endpoint_prefers_remote_keys_and_ignores_gateway_host() {
        let c = conn(&[
            ("host", "cdc-sshtunnel"),
            ("port", "22005"),
            (KEY_REMOTE_HOST, "127.0.0.1"),
            (KEY_REMOTE_PORT, "5433"),
        ]);
        assert_eq!(
            intended_endpoint(&c, "cdc-sshtunnel", "postgres"),
            ("127.0.0.1".into(), 5433)
        );
        // First save from the wizard: host/port are the database as seen from the SSH server.
        let c = conn(&[("host", "10.0.0.5"), ("port", "3306")]);
        assert_eq!(
            intended_endpoint(&c, "cdc-sshtunnel", "mysql"),
            ("10.0.0.5".into(), 3306)
        );
        // Gateway address without remote keys: never treated as the database address.
        let c = conn(&[("host", "cdc-sshtunnel"), ("port", "22005")]);
        assert_eq!(
            intended_endpoint(&c, "cdc-sshtunnel", "postgres"),
            (String::new(), 5432)
        );
    }

    #[test]
    fn switching_back_to_direct_restores_the_database_address() {
        let mut c = conn(&[
            ("host", "cdc-sshtunnel"),
            ("port", "22005"),
            (KEY_REMOTE_HOST, "db.site"),
            (KEY_REMOTE_PORT, "5432"),
            ("database", "app"),
        ]);
        restore_direct(&mut c, "cdc-sshtunnel");
        assert_eq!(c.get("host").unwrap(), "db.site");
        assert_eq!(c.get("port").unwrap(), "5432");
        assert!(!c.contains_key(KEY_REMOTE_HOST) && !c.contains_key(KEY_TUNNEL));
        assert_eq!(c.get("database").unwrap(), "app");
        // A direct connection the operator edited is left alone.
        let mut d = conn(&[
            ("host", "db.other"),
            ("port", "5432"),
            (KEY_REMOTE_HOST, "stale"),
        ]);
        restore_direct(&mut d, "cdc-sshtunnel");
        assert_eq!(d.get("host").unwrap(), "db.other");
    }

    #[test]
    fn redacted_secret_kept_only_for_the_same_login() {
        let old = tunnel(vec![(22000, "db", 5432)]);
        let req = SshTunnelRequest {
            name: "site".into(),
            host: old.host.clone(),
            username: "cdc".into(),
            auth_method: "key".into(),
            private_key: REDACTED.into(),
            proxy: ProxyConfig {
                kind: "cloudflare_access".into(),
                hostname: old.proxy.hostname.clone(),
                service_token_id: "id".into(),
                service_token_secret: REDACTED.into(),
            },
            ..Default::default()
        };
        let t = build(old.id, req.clone(), Some(&old), true).unwrap();
        assert_eq!(t.private_key, "KEY");
        assert_eq!(t.proxy.service_token_secret, "TOKEN-SECRET");
        assert_eq!(t.forwards, old.forwards, "forwards survive an edit");

        let mut moved = req;
        moved.host = "evil.example".into();
        moved.proxy.hostname = "evil.example".into();
        assert!(build(old.id, moved, Some(&old), true).is_err());
    }

    #[test]
    fn build_validates_auth_and_proxy() {
        let base = SshTunnelRequest {
            name: "n".into(),
            host: "h".into(),
            username: "u".into(),
            ..Default::default()
        };
        assert!(
            build(Uuid::nil(), base.clone(), None, true).is_err(),
            "key required"
        );
        let mut pw = base.clone();
        pw.auth_method = "password".into();
        pw.password = "p".into();
        let t = build(Uuid::nil(), pw, None, true).unwrap();
        assert_eq!(t.proxy.kind, "none");
        let mut cf = base;
        cf.private_key = "k".into();
        cf.proxy.kind = "cloudflare_access".into();
        cf.proxy.service_token_id = "only-id".into();
        assert!(
            build(Uuid::nil(), cf, None, true).is_err(),
            "token id without secret"
        );
    }

    #[test]
    fn json_endpoint_kind_and_routing_keys() {
        let db = json!({"type": "mysql", "host": "h", "port": 3306, "ssh_tunnel_id": "t"});
        let o = db.as_object().unwrap();
        assert_eq!(endpoint_kind_of(o), "mysql");
        let keys = routing_keys(o);
        assert_eq!(keys["port"], "3306");
        assert_eq!(keys[KEY_TUNNEL], "t");
        let ssh = json!({"path": "/data", "host": "h"});
        assert_eq!(endpoint_kind_of(ssh.as_object().unwrap()), "ssh");
        let daemon = json!({"path": "m", "protocol": "DAEMON"});
        assert_eq!(endpoint_kind_of(daemon.as_object().unwrap()), "rsync");

        let mut out = json!({"host": "h", "port": 3306, "ssh_tunnel_id": "t"})
            .as_object()
            .unwrap()
            .clone();
        let mut m = HashMap::new();
        m.insert("host".to_string(), "cdc-sshtunnel".to_string());
        m.insert("port".to_string(), "22001".to_string());
        write_routing_keys(&mut out, m.clone(), true);
        assert_eq!(out["port"], json!(22001));
        assert!(!out.contains_key(KEY_TUNNEL));
        write_routing_keys(&mut out, m, false);
        assert_eq!(out["port"], json!("22001"));
    }

    #[test]
    fn fingerprint_is_normalised() {
        assert_eq!(normalize_fp(" abc= "), "SHA256:abc");
        assert_eq!(normalize_fp("SHA256:abc"), "SHA256:abc");
        assert_eq!(normalize_fp(""), "");
    }
}
