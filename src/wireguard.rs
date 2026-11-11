//! WireGuard tunnels for the Copy & Sync tools (ADR 0010).
//!
//! A tunnel is one WireGuard peering between the `cdc-wireguard` gateway (userspace wireguard-go)
//! and a WireGuard server at a source or target site. The site exposes one UDP port; database
//! ports stay closed to the outside. A connection with `tunnel_id` is rewritten, just before a
//! run or a connection test, to a gateway forward (`cdc-wireguard:21xxx`) that dials the
//! connection's host:port inside the tunnel — so every tool works unchanged.
//!
//! Tunnel definitions (with private / preshared keys) live in rqlite; the API redacts secrets.
//! The gateway keeps them in memory only; a reconcile loop pushes them again after a restart.

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
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

use crate::error::AppError;
use crate::AppState;
use hyper_cdc_orchestrator::persist::{RqliteClient, KIND_WIREGUARD};

pub const REDACTED: &str = "********";

// ── model ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireguardTunnel {
    pub id: Uuid,
    pub name: String,
    /// This side (gateway): its tunnel address, e.g. `10.66.1.2/32`.
    pub address: String,
    /// Secret. Generated when the tunnel is created unless one is given.
    #[serde(default)]
    pub private_key: String,
    /// Derived from `private_key`; the site lists it as its peer.
    #[serde(default)]
    pub public_key: String,
    /// Resolvers at the site, for database host names that only resolve there.
    #[serde(default)]
    pub dns: Vec<String>,
    #[serde(default)]
    pub mtu: Option<u32>,
    /// The site's WireGuard server: `host:port` (UDP) it listens on.
    pub peer_endpoint: String,
    pub peer_public_key: String,
    /// Optional extra secret shared by both sides (post-quantum hardening).
    #[serde(default)]
    pub preshared_key: String,
    /// The site's own tunnel address, e.g. `10.66.1.1/32` (used in the site configuration).
    #[serde(default)]
    pub site_address: String,
    /// Networks reachable through the tunnel: the site's tunnel address plus the database
    /// host(s)/subnet, e.g. `10.66.1.1/32`, `192.168.10.0/24`.
    pub allowed_ips: Vec<String>,
    #[serde(default)]
    pub keepalive: Option<u32>,
    #[serde(default)]
    pub notes: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WireguardTunnel {
    pub fn redacted(mut self) -> Self {
        if !self.private_key.is_empty() {
            self.private_key = REDACTED.into();
        }
        if !self.preshared_key.is_empty() {
            self.preshared_key = REDACTED.into();
        }
        self
    }

    /// What the gateway needs (includes secrets; never returned by the API).
    fn gateway_spec(&self) -> Value {
        json!({
            "id": self.id.to_string(),
            "name": self.name,
            "version": self.updated_at.to_rfc3339(),
            "private_key": self.private_key,
            "addresses": [self.address],
            "dns": self.dns,
            "mtu": self.mtu.unwrap_or(0),
            "peer": {
                "public_key": self.peer_public_key,
                "preshared_key": self.preshared_key,
                "endpoint": self.peer_endpoint,
                "allowed_ips": self.allowed_ips,
                "keepalive": self.keepalive.unwrap_or(25),
            },
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct TunnelRequest {
    pub name: String,
    pub address: String,
    #[serde(default)]
    pub private_key: String,
    #[serde(default)]
    pub dns: Vec<String>,
    #[serde(default)]
    pub mtu: Option<u32>,
    pub peer_endpoint: String,
    #[serde(default)]
    pub peer_public_key: String,
    #[serde(default)]
    pub preshared_key: String,
    #[serde(default)]
    pub site_address: String,
    #[serde(default)]
    pub allowed_ips: Vec<String>,
    #[serde(default)]
    pub keepalive: Option<u32>,
    #[serde(default)]
    pub notes: String,
    /// Generate the site's key pair too: its public key becomes `peer_public_key`; the private
    /// key is returned once (`site_private_key`) for the site configuration and never stored.
    #[serde(default)]
    pub generate_site_keys: bool,
    /// Generate a preshared key.
    #[serde(default)]
    pub generate_preshared_key: bool,
}

// ── store ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TunnelStore {
    memory: Arc<RwLock<HashMap<Uuid, WireguardTunnel>>>,
    rqlite: Option<RqliteClient>,
}

impl TunnelStore {
    pub fn new(rqlite: Option<RqliteClient>) -> Self {
        Self {
            memory: Arc::new(RwLock::new(HashMap::new())),
            rqlite,
        }
    }

    pub async fn list(&self) -> Result<Vec<WireguardTunnel>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq.list_json::<WireguardTunnel>(KIND_WIREGUARD).await {
                Ok(mut list) => {
                    list.sort_by(|a, b| a.name.cmp(&b.name));
                    let mut map = self.memory.write().await;
                    map.clear();
                    for t in &list {
                        map.insert(t.id, t.clone());
                    }
                    return Ok(list);
                }
                Err(e) => warn!(error = %e, "rqlite list wireguard tunnels failed; using memory"),
            }
        }
        let mut list: Vec<_> = self.memory.read().await.values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(list)
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<WireguardTunnel>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq
                .get_json::<WireguardTunnel>(KIND_WIREGUARD, &id.to_string())
                .await
            {
                Ok(found) => {
                    if let Some(t) = &found {
                        self.memory.write().await.insert(t.id, t.clone());
                    }
                    return Ok(found);
                }
                Err(e) => warn!(error = %e, "rqlite get wireguard tunnel failed"),
            }
        }
        Ok(self.memory.read().await.get(&id).cloned())
    }

    pub async fn upsert(&self, t: &WireguardTunnel) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.put_json(KIND_WIREGUARD, &t.id.to_string(), t).await?;
        }
        self.memory.write().await.insert(t.id, t.clone());
        Ok(())
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.delete(KIND_WIREGUARD, &id.to_string()).await?;
        }
        self.memory.write().await.remove(&id);
        Ok(())
    }
}

// ── gateway client ──────────────────────────────────────────────────────────

fn gateway_url(state: &AppState) -> Result<&str, AppError> {
    state.config.wireguard_url.as_deref().ok_or_else(|| {
        AppError::BadRequest(
            "WireGuard is not configured: start cdc-wireguard and set ORCH_WIREGUARD_URL".into(),
        )
    })
}

async fn gateway(
    state: &AppState,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, AppError> {
    let url = format!("{}{}", gateway_url(state)?, path);
    let mut req = state
        .http
        .request(method, &url)
        .timeout(Duration::from_secs(20));
    if let Some(t) = &state.config.wireguard_token {
        req = req.bearer_auth(t);
    }
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("cdc-wireguard unreachable: {e}")))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = v
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("request failed")
            .to_string();
        return Err(if status.as_u16() == 400 {
            AppError::BadRequest(format!("WireGuard: {msg}"))
        } else if status.as_u16() == 401 {
            AppError::Upstream("cdc-wireguard rejected the token (ORCH_WIREGUARD_TOKEN)".into())
        } else {
            AppError::Upstream(format!("cdc-wireguard: {msg}"))
        });
    }
    Ok(v)
}

async fn keys(state: &AppState, private_key: Option<&str>) -> Result<(String, String), AppError> {
    let body = match private_key {
        Some(k) => json!({ "private_key": k }),
        None => json!({}),
    };
    let v = gateway(state, Method::POST, "/v1/keys", Some(body)).await?;
    let get = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    Ok((get("private_key"), get("public_key")))
}

async fn push(state: &AppState, t: &WireguardTunnel) -> Result<Value, AppError> {
    gateway(
        state,
        Method::PUT,
        &format!("/v1/tunnels/{}", t.id),
        Some(t.gateway_spec()),
    )
    .await
}

/// Keeps the gateway in step with the store (it restarts with no tunnels; edits elsewhere).
pub fn spawn_reconcile(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if state.config.wireguard_url.is_none() {
            return;
        }
        loop {
            if let Err(e) = reconcile(&state).await {
                warn!(error = %e, "wireguard reconcile failed");
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    })
}

async fn reconcile(state: &AppState) -> Result<(), AppError> {
    let live = gateway(state, Method::GET, "/v1/tunnels", None).await?;
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
    let stored = state.wireguard_tunnels.list().await?;
    for t in &stored {
        if versions.get(&t.id.to_string()) != Some(&t.updated_at.to_rfc3339()) {
            match push(state, t).await {
                Ok(_) => info!(tunnel = %t.name, "wireguard tunnel pushed to gateway"),
                Err(e) => warn!(tunnel = %t.name, error = %e, "wireguard tunnel push failed"),
            }
        }
    }
    for id in versions.keys() {
        if !stored.iter().any(|t| &t.id.to_string() == id) {
            let _ = gateway(state, Method::DELETE, &format!("/v1/tunnels/{id}"), None).await;
        }
    }
    Ok(())
}

// ── routing connections through tunnels ─────────────────────────────────────

fn default_port(kind: &str) -> u16 {
    match kind.trim().to_ascii_lowercase().as_str() {
        "redis" | "valkey" | "tair" => 6379,
        "mysql" | "mariadb" => 3306,
        "sqlserver" | "mssql" => 1433,
        "mongodb" | "mongo" => 27017,
        "ssh" | "sftp" => 22,
        "rsync" | "rsyncd" => 873,
        _ => 5432,
    }
}

/// Rewrites every `source` / `target` / `connection` / `endpoint` object in a sidecar payload that
/// has a `tunnel_id` to the gateway forward for its host:port. Returns the tunnels used (names).
/// File sync: a tunnelled endpoint forces the relay transport (direct mode would have the
/// source host connect to the target itself, which cannot use our tunnel).
pub async fn route_payload(state: &AppState, body: &mut Value) -> Result<Vec<String>, AppError> {
    let mut used = Vec::new();
    let mut rsync_tunnel = false;
    let opts = body.get("options").cloned().unwrap_or(Value::Null);
    for key in ["source", "target", "connection", "endpoint"] {
        let Some(conn) = body.get_mut(key) else {
            continue;
        };
        let Some(tid) = conn
            .get("tunnel_id")
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok())
        else {
            if let Some(o) = conn.as_object_mut() {
                o.remove("tunnel_id");
            }
            continue;
        };
        // A cluster hands out node addresses the tunnel forward cannot follow.
        let side = if key == "target" { "target" } else { "source" };
        if opts.get(format!("{side}_cluster")).and_then(Value::as_bool) == Some(true) {
            return Err(AppError::BadRequest(format!(
                "the {side} is a Redis Cluster: cluster redirects cannot follow a WireGuard forward; use a single server or a proxy at the site"
            )));
        }
        let tunnel = state.wireguard_tunnels.get(tid).await?.ok_or_else(|| {
            AppError::BadRequest(format!("WireGuard tunnel {tid} not found (deleted?)"))
        })?;
        let host = conn
            .get("host")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if host.is_empty() {
            return Err(AppError::BadRequest(format!(
                "{key}: a host is required to use a WireGuard tunnel"
            )));
        }
        // File sync endpoints are SSH hosts (they carry a `path`); databases carry a `type`.
        let is_ssh = conn.get("path").is_some();
        let daemon = conn
            .get("protocol")
            .and_then(Value::as_str)
            .is_some_and(|p| p.eq_ignore_ascii_case("daemon"));
        let kind = if is_ssh && daemon {
            "rsync".to_string()
        } else if is_ssh {
            "ssh".to_string()
        } else {
            conn.get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        rsync_tunnel |= is_ssh;
        let port = match conn.get("port") {
            Some(Value::String(s)) if !s.trim().is_empty() => s.trim().parse::<u16>().ok(),
            Some(Value::Number(n)) => n.as_u64().and_then(|n| u16::try_from(n).ok()),
            _ => None,
        }
        .unwrap_or_else(|| default_port(&kind));
        let fwd = ensure_forward(state, &tunnel, &host, port).await?;
        let o = conn.as_object_mut().expect("connection object");
        o.insert("host".into(), json!(fwd.0));
        o.insert(
            "port".into(),
            if matches!(o.get("port"), Some(Value::Number(_))) {
                json!(fwd.1)
            } else {
                json!(fwd.1.to_string())
            },
        );
        o.remove("tunnel_id");
        o.insert("via_tunnel".into(), json!(tunnel.name));
        used.push(tunnel.name.clone());
    }
    if rsync_tunnel {
        if let Some(o) = body.get_mut("options").and_then(Value::as_object_mut) {
            o.insert("transport".into(), json!("relay"));
        }
    }
    Ok(used)
}

async fn ensure_forward(
    state: &AppState,
    t: &WireguardTunnel,
    host: &str,
    port: u16,
) -> Result<(String, u16), AppError> {
    let req = json!({ "tunnel_id": t.id.to_string(), "host": host, "port": port });
    let v = match gateway(state, Method::POST, "/v1/forwards", Some(req.clone())).await {
        Ok(v) => v,
        // The gateway restarted and has not been reconciled yet: push this tunnel and retry.
        Err(AppError::BadRequest(m)) if m.contains("not configured") => {
            push(state, t).await?;
            gateway(state, Method::POST, "/v1/forwards", Some(req)).await?
        }
        Err(e) => return Err(e),
    };
    let fhost = v
        .get("host")
        .and_then(Value::as_str)
        .unwrap_or("cdc-wireguard")
        .to_string();
    let fport = v
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|p| u16::try_from(p).ok())
        .ok_or_else(|| AppError::Upstream("cdc-wireguard returned no forward port".into()))?;
    Ok((fhost, fport))
}

// ── site configuration ──────────────────────────────────────────────────────

/// wg-quick configuration for the site's WireGuard server, plus what else the site must allow.
/// `site_private_key` / `preshared_key` are filled in only when they were just generated (shown
/// once); otherwise the file carries placeholders — the site's private key is never stored.
pub fn site_config(
    t: &WireguardTunnel,
    site_private_key: Option<&str>,
    preshared_key: Option<&str>,
) -> Value {
    let listen_port = t
        .peer_endpoint
        .rsplit(':')
        .next()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(51820);
    let gw_ip = t
        .address
        .split('/')
        .next()
        .unwrap_or(&t.address)
        .to_string();
    let site_addr = if t.site_address.trim().is_empty() {
        "10.66.1.1/32".to_string()
    } else {
        t.site_address.clone()
    };
    let lan: Vec<&String> = t
        .allowed_ips
        .iter()
        .filter(|a| a.split('/').next() != site_addr.split('/').next())
        .collect();
    let mut conf = String::new();
    conf.push_str(&format!(
        "# /etc/wireguard/wg0.conf on the site's WireGuard server — tunnel \"{}\"\n",
        t.name
    ));
    conf.push_str("[Interface]\n");
    conf.push_str(&format!("Address = {site_addr}\n"));
    conf.push_str(&format!("ListenPort = {listen_port}\n"));
    conf.push_str(&format!(
        "PrivateKey = {}\n",
        site_private_key.unwrap_or("<the site's private key: wg genkey>")
    ));
    if !lan.is_empty() {
        conf.push_str("# The database is on another host: forward and masquerade the gateway's traffic to it.\n");
        conf.push_str("# Replace eth0 with the interface that reaches the database network.\n");
        conf.push_str(
            "# Needs IP forwarding on (persistent, see the checklist: net.ipv4.ip_forward=1).\n",
        );
        conf.push_str(&format!(
            "PostUp = iptables -A FORWARD -i %i -s {gw_ip}/32 -j ACCEPT; iptables -A FORWARD -o %i -m state --state RELATED,ESTABLISHED -j ACCEPT; iptables -t nat -A POSTROUTING -s {gw_ip}/32 -o eth0 -j MASQUERADE\n"
        ));
        conf.push_str(&format!(
            "PostDown = iptables -D FORWARD -i %i -s {gw_ip}/32 -j ACCEPT; iptables -D FORWARD -o %i -m state --state RELATED,ESTABLISHED -j ACCEPT; iptables -t nat -D POSTROUTING -s {gw_ip}/32 -o eth0 -j MASQUERADE\n"
        ));
    }
    conf.push_str("\n[Peer]\n");
    conf.push_str(
        "# Hyper CDC gateway (cdc-wireguard). It dials in; no Endpoint is needed here.\n",
    );
    conf.push_str(&format!("PublicKey = {}\n", t.public_key));
    if !t.preshared_key.is_empty() {
        conf.push_str(&format!(
            "PresharedKey = {}\n",
            preshared_key.unwrap_or("<the preshared key shown when it was generated>")
        ));
    }
    conf.push_str(&format!("AllowedIPs = {gw_ip}/32\n"));

    let db_addr = lan
        .first()
        .map(|s| s.to_string())
        .unwrap_or_else(|| site_addr.split('/').next().unwrap_or("").to_string());
    json!({
        "config": conf,
        // false: the file still has a placeholder private key; rotate the site keys to get a full one.
        "complete": site_private_key.is_some() && (t.preshared_key.is_empty() || preshared_key.is_some()),
        "listen_port": listen_port,
        "gateway_tunnel_ip": gw_ip,
        "site_tunnel_address": site_addr,
        "checklist": [
            format!("Open UDP {listen_port} inbound on the site's firewall/router to the WireGuard server (only this port; database ports stay closed)."),
            "Install WireGuard (Linux: apt install wireguard / dnf install wireguard-tools), save the file above as /etc/wireguard/wg0.conf (chmod 600), then: systemctl enable --now wg-quick@wg0.".to_string(),
            "If the database is on another host (the config has PostUp rules): turn on IP forwarding persistently — echo 'net.ipv4.ip_forward=1' > /etc/sysctl.d/99-wireguard.conf && sysctl --system — and set eth0 in PostUp/PostDown to the interface that reaches the database.".to_string(),
            format!("Let the database accept the gateway's tunnel address {gw_ip}: PostgreSQL pg_hba.conf `host all <user> {gw_ip}/32 scram-sha-256` and listen_addresses covering {db_addr}; Redis bind/protected-mode and ACL/requirepass; MongoDB net.bindIp; MySQL bind-address and a user@'{gw_ip}'."),
            "If the database runs on the WireGuard server itself, use the site tunnel address as the database host in the profile; otherwise use the database's LAN address (covered by AllowedIPs).".to_string(),
            "Check: `wg show` on the site lists a recent handshake once the tunnel is saved here; then use Probe on this page with the database host and port.".to_string(),
        ],
    })
}

// ── API ─────────────────────────────────────────────────────────────────────

fn redacted_or(v: &str, old: Option<&str>) -> String {
    if v == REDACTED {
        old.unwrap_or_default().to_string()
    } else {
        v.trim().to_string()
    }
}

fn validate_request(r: &TunnelRequest) -> Result<(), AppError> {
    if r.name.trim().is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    if r.address.trim().is_empty() {
        return Err(AppError::BadRequest(
            "address (this gateway's tunnel IP, e.g. 10.66.1.2/32) is required".into(),
        ));
    }
    if !r.peer_endpoint.contains(':') {
        return Err(AppError::BadRequest("peer_endpoint must be host:port of the site's WireGuard (UDP), e.g. vpn.site.example:51820".into()));
    }
    if r.allowed_ips.iter().all(|a| a.trim().is_empty()) {
        return Err(AppError::BadRequest(
            "allowed_ips: list the site's tunnel address and the database network(s)".into(),
        ));
    }
    if r.peer_public_key.trim().is_empty() && !r.generate_site_keys {
        return Err(AppError::BadRequest("peer_public_key is required (the site's WireGuard public key), or ask to generate the site keys".into()));
    }
    Ok(())
}

async fn build(
    state: &AppState,
    id: Uuid,
    r: TunnelRequest,
    old: Option<&WireguardTunnel>,
) -> Result<(WireguardTunnel, Value), AppError> {
    validate_request(&r)?;
    let mut extra = json!({});
    let given = redacted_or(&r.private_key, old.map(|o| o.private_key.as_str()));
    let (private_key, public_key) = if given.is_empty() {
        keys(state, None).await?
    } else {
        keys(state, Some(&given)).await?
    };
    let mut peer_public_key = r.peer_public_key.trim().to_string();
    if r.generate_site_keys {
        let (sp, spub) = keys(state, None).await?;
        peer_public_key = spub;
        extra["site_private_key"] = json!(sp); // shown once, never stored
    }
    let mut preshared_key = redacted_or(&r.preshared_key, old.map(|o| o.preshared_key.as_str()));
    if r.generate_preshared_key {
        let v = gateway(state, Method::POST, "/v1/keys/preshared", Some(json!({}))).await?;
        preshared_key = v
            .get("preshared_key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        extra["preshared_key"] = json!(preshared_key); // shown once
    }
    let now = Utc::now();
    let t = WireguardTunnel {
        id,
        name: r.name.trim().to_string(),
        address: r.address.trim().to_string(),
        private_key,
        public_key,
        dns: r
            .dns
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        mtu: r.mtu.filter(|m| *m > 0),
        peer_endpoint: r.peer_endpoint.trim().to_string(),
        peer_public_key,
        preshared_key,
        site_address: r.site_address.trim().to_string(),
        allowed_ips: r
            .allowed_ips
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        keepalive: r.keepalive,
        notes: r.notes.trim().to_string(),
        created_at: old.map(|o| o.created_at).unwrap_or(now),
        updated_at: now,
    };
    Ok((t, extra))
}

fn actor(headers: &HeaderMap) -> String {
    headers
        .get("x-user")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .unwrap_or("api")
        .to_string()
}

pub async fn list_tunnels(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let stored = state.wireguard_tunnels.list().await?;
    let live = gateway(&state, Method::GET, "/v1/tunnels", None).await.ok();
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
    let tunnels: Vec<Value> = stored
        .into_iter()
        .map(|t| {
            let st = status
                .get(&t.id.to_string())
                .cloned()
                .unwrap_or(Value::Null);
            let mut v = serde_json::to_value(t.redacted()).unwrap_or(Value::Null);
            v["status"] = st;
            v
        })
        .collect();
    Ok(Json(json!({
        "configured": state.config.wireguard_url.is_some(),
        "gateway_reachable": live.is_some(),
        "tunnels": tunnels,
    })))
}

pub async fn create_tunnel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<TunnelRequest>,
) -> Result<Json<Value>, AppError> {
    let (t, extra) = build(&state, Uuid::new_v4(), r, None).await?;
    state.wireguard_tunnels.upsert(&t).await?;
    let pushed = push(&state, &t).await;
    crate::audit::log_action(
        &actor(&headers),
        "wireguard.tunnel.create",
        "success",
        &format!("id={} name={}", t.id, t.name),
    );
    let mut out = serde_json::to_value(t.clone().redacted()).unwrap_or(Value::Null);
    out["once"] = extra;
    out["site"] = site_config(
        &t,
        out["once"].get("site_private_key").and_then(Value::as_str),
        out["once"].get("preshared_key").and_then(Value::as_str),
    );
    if let Err(e) = pushed {
        out["gateway_error"] = json!(e.to_string());
    }
    Ok(Json(out))
}

pub async fn update_tunnel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(r): Json<TunnelRequest>,
) -> Result<Json<Value>, AppError> {
    let old = state
        .wireguard_tunnels
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("tunnel {id} not found")))?;
    let (t, extra) = build(&state, id, r, Some(&old)).await?;
    state.wireguard_tunnels.upsert(&t).await?;
    let pushed = push(&state, &t).await;
    crate::audit::log_action(
        &actor(&headers),
        "wireguard.tunnel.update",
        "success",
        &format!("id={id}"),
    );
    let mut out = serde_json::to_value(t.clone().redacted()).unwrap_or(Value::Null);
    out["once"] = extra;
    // New site keys: hand back the complete site file now (its private key is not kept).
    if let Some(sp) = out["once"].get("site_private_key").and_then(Value::as_str) {
        let psk = out["once"].get("preshared_key").and_then(Value::as_str);
        out["site"] = site_config(&t, Some(sp), psk);
    }
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
    state.wireguard_tunnels.delete(id).await?;
    let _ = gateway(&state, Method::DELETE, &format!("/v1/tunnels/{id}"), None).await;
    crate::audit::log_action(
        &actor(&headers),
        "wireguard.tunnel.delete",
        "success",
        &format!("id={id}"),
    );
    Ok(Json(json!({ "status": true, "id": id })))
}

pub async fn tunnel_site_config(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let t = state
        .wireguard_tunnels
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("tunnel {id} not found")))?;
    Ok(Json(site_config(&t, None, None)))
}

#[derive(Deserialize, Default)]
pub struct SiteKeysRequest {
    /// true: new preshared key; false: stop using one (the stored key could not be shown, so the
    /// site file would be incomplete); unset: new one only when the tunnel already uses one.
    #[serde(default)]
    pub preshared_key: Option<bool>,
}

/// New key pair for the site of an existing tunnel: the gateway switches to the new site public
/// key, and the complete site `wg0.conf` (private key filled in) is returned once. The site's old
/// configuration stops working until the new file is installed there.
pub async fn rotate_site_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    body: Option<Json<SiteKeysRequest>>,
) -> Result<Json<Value>, AppError> {
    let mut t = state
        .wireguard_tunnels
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("tunnel {id} not found")))?;
    let (site_private, site_public) = keys(&state, None).await?;
    t.peer_public_key = site_public;
    let mut once = json!({ "site_private_key": site_private });
    let want_psk = body
        .and_then(|b| b.0.preshared_key)
        .unwrap_or(!t.preshared_key.is_empty());
    if want_psk {
        let v = gateway(&state, Method::POST, "/v1/keys/preshared", Some(json!({}))).await?;
        t.preshared_key = v
            .get("preshared_key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        once["preshared_key"] = json!(t.preshared_key);
    } else {
        t.preshared_key.clear();
    }
    t.updated_at = Utc::now();
    state.wireguard_tunnels.upsert(&t).await?;
    let pushed = push(&state, &t).await;
    crate::audit::log_action(
        &actor(&headers),
        "wireguard.tunnel.site_keys",
        "success",
        &format!("id={id} preshared={want_psk}"),
    );
    let mut out = serde_json::to_value(t.clone().redacted()).unwrap_or(Value::Null);
    out["site"] = site_config(
        &t,
        once.get("site_private_key").and_then(Value::as_str),
        once.get("preshared_key").and_then(Value::as_str),
    );
    out["once"] = once;
    if let Err(e) = pushed {
        out["gateway_error"] = json!(e.to_string());
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct ProbeRequest {
    pub host: String,
    pub port: u16,
}

pub async fn probe_tunnel(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(r): Json<ProbeRequest>,
) -> Result<Json<Value>, AppError> {
    let t = state
        .wireguard_tunnels
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("tunnel {id} not found")))?;
    let body = json!({ "host": r.host.trim(), "port": r.port });
    let path = format!("/v1/tunnels/{id}/probe");
    let v = match gateway(&state, Method::POST, &path, Some(body.clone())).await {
        Ok(v) => v,
        // The gateway restarted and does not have this tunnel yet: push it and retry once.
        Err(e) if e.to_string().contains("not configured") => {
            push(&state, &t).await?;
            gateway(&state, Method::POST, &path, Some(body)).await?
        }
        Err(e) => return Err(e),
    };
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct UdpTestRequest {
    pub endpoint: String,
}

/// Before a tunnel exists: is the site's WireGuard UDP port open end to end? The site runs a UDP
/// echo on the port (`socat UDP4-LISTEN:<port>,fork PIPE`); the gateway sends test datagrams and
/// reports whether they came back.
pub async fn udp_test(
    State(state): State<AppState>,
    Json(r): Json<UdpTestRequest>,
) -> Result<Json<Value>, AppError> {
    let endpoint = r.endpoint.trim();
    if !endpoint.contains(':') {
        return Err(AppError::BadRequest(
            "endpoint must be host:port, e.g. vpn.site.example:51820".into(),
        ));
    }
    gateway(
        &state,
        Method::POST,
        "/v1/udp-probe",
        Some(json!({ "endpoint": endpoint })),
    )
    .await
    .map(Json)
}

pub async fn wireguard_capability(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    if state.config.wireguard_url.is_none() {
        return Ok(Json(json!({ "configured": false })));
    }
    let v = gateway(&state, Method::GET, "/v1/capability", None).await;
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

    fn tunnel() -> WireguardTunnel {
        WireguardTunnel {
            id: Uuid::nil(),
            name: "Source site".into(),
            address: "10.66.1.2/32".into(),
            private_key: "SECRET-GATEWAY-PRIVATE".into(),
            public_key: "GWPUB=".into(),
            dns: vec![],
            mtu: None,
            peer_endpoint: "vpn.source.example:51820".into(),
            peer_public_key: "SITEPUB=".into(),
            preshared_key: "SECRET-PSK".into(),
            site_address: "10.66.1.1/32".into(),
            allowed_ips: vec!["10.66.1.1/32".into(), "192.168.10.0/24".into()],
            keepalive: None,
            notes: String::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn redaction_hides_both_secrets() {
        let t = tunnel().redacted();
        assert_eq!(t.private_key, REDACTED);
        assert_eq!(t.preshared_key, REDACTED);
        assert_eq!(t.public_key, "GWPUB=");
    }

    #[test]
    fn site_config_peers_with_the_gateway_and_forwards_to_the_lan() {
        let s = site_config(&tunnel(), None, None);
        let c = s["config"].as_str().unwrap();
        assert!(c.contains("Address = 10.66.1.1/32"));
        assert!(c.contains("ListenPort = 51820"));
        assert!(c.contains("PublicKey = GWPUB="));
        assert!(c.contains("AllowedIPs = 10.66.1.2/32"));
        assert!(c.contains("MASQUERADE") && c.contains("-s 10.66.1.2/32"));
        assert!(
            c.contains("PresharedKey = <"),
            "never prints the stored preshared key"
        );
        assert!(
            !c.contains("SECRET-GATEWAY-PRIVATE") && !c.contains("SECRET-PSK"),
            "stored secrets never appear"
        );
        assert_eq!(s["gateway_tunnel_ip"], "10.66.1.2");
        // Database on the WireGuard host itself: no forwarding rules needed.
        let mut t = tunnel();
        t.allowed_ips = vec!["10.66.1.1/32".into()];
        assert!(!site_config(&t, None, None)["config"]
            .as_str()
            .unwrap()
            .contains("MASQUERADE"));
    }

    #[test]
    fn site_config_is_complete_only_with_just_generated_secrets() {
        let t = tunnel(); // uses a preshared key
        assert_eq!(site_config(&t, None, None)["complete"], false);
        assert_eq!(
            site_config(&t, Some("NEW-SITE-PRIV="), None)["complete"],
            false
        );
        let s = site_config(&t, Some("NEW-SITE-PRIV="), Some("NEW-PSK="));
        let c = s["config"].as_str().unwrap();
        assert_eq!(s["complete"], true);
        assert!(c.contains("PrivateKey = NEW-SITE-PRIV="));
        assert!(c.contains("PresharedKey = NEW-PSK="));
        assert!(
            !c.contains("SECRET-PSK"),
            "the stored preshared key is still never printed"
        );
        let mut no_psk = tunnel();
        no_psk.preshared_key.clear();
        assert_eq!(
            site_config(&no_psk, Some("NEW-SITE-PRIV="), None)["complete"],
            true
        );
    }

    #[test]
    fn gateway_spec_carries_the_peer() {
        let v = tunnel().gateway_spec();
        assert_eq!(v["addresses"], json!(["10.66.1.2/32"]));
        assert_eq!(v["peer"]["endpoint"], "vpn.source.example:51820");
        assert_eq!(
            v["peer"]["allowed_ips"],
            json!(["10.66.1.1/32", "192.168.10.0/24"])
        );
        assert_eq!(v["peer"]["keepalive"], 25);
    }

    #[test]
    fn default_ports_follow_the_engine() {
        assert_eq!(default_port("redis"), 6379);
        assert_eq!(default_port("mongodb"), 27017);
        assert_eq!(default_port("ssh"), 22);
        assert_eq!(default_port("rsync"), 873);
        assert_eq!(default_port("postgres"), 5432);
    }
}
