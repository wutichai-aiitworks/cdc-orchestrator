//! AI vector sync (ADR 0018): keep a PGVector collection in step with a pipeline's CDC topics.
//!
//! The orchestrator owns the sync definitions (rqlite, kind `vectorsync_profile`) and pushes the
//! desired set to the `cdc-vectorsync` sidecar, which runs pydebeziumai's document builder behind
//! its own Kafka consumer. The sidecar holds specs (with secrets) in memory only, so a reconcile
//! loop re-sends them after a sidecar restart and removes syncs that were paused or deleted.
//!
//! Switch: `ORCH_VECTORSYNC_ENABLED` (compose maps `VECTORSYNC_ENABLED`) + `ORCH_VECTORSYNC_URL`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use reqwest::{Client, Method};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::warn;
use urlencoding::encode;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::{
    VectorSyncEmbedding, VectorSyncProfile, VectorSyncProfileRequest, VectorSyncTarget,
};
use crate::saved_secret::REDACTED;
use crate::tool_sidecar::SidecarAccess;
use crate::AppState;
use hyper_cdc_orchestrator::persist::{RqliteClient, KIND_VECTORSYNC};

pub const PROVIDERS: [&str; 3] = ["ollama", "openai", "hash"];
const DISABLED: &str =
    "AI vector sync is turned off. Set VECTORSYNC_ENABLED=true in docker/.env and redeploy.";

// ── switch + sidecar access ─────────────────────────────────────────────────────────────

pub fn enabled(cfg: &AppConfig) -> bool {
    cfg.vectorsync_enabled && cfg.vectorsync_url.is_some()
}

fn access<'a>(cfg: &'a AppConfig, client: &'a Client) -> Result<SidecarAccess<'a>, AppError> {
    if !cfg.vectorsync_enabled {
        return Err(AppError::NotFound(DISABLED.into()));
    }
    let base_url = cfg
        .vectorsync_url
        .as_deref()
        .map(|u| u.trim_end_matches('/'))
        .ok_or_else(|| {
            AppError::BadRequest(
                "vectorsync sidecar is not configured (ORCH_VECTORSYNC_URL)".into(),
            )
        })?;
    Ok(SidecarAccess {
        client,
        base_url,
        bearer_token: cfg.vectorsync_token.as_deref(),
        tool: "vectorsync",
        token_env: "ORCH_VECTORSYNC_TOKEN",
        extra_bad_request: &[],
    })
}

pub async fn sidecar(
    cfg: &AppConfig,
    client: &Client,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, AppError> {
    access(cfg, client)?.request(method, path, body).await
}

pub fn ensure_enabled(cfg: &AppConfig) -> Result<(), AppError> {
    access(cfg, &Client::new()).map(|_| ())
}

// ── store ───────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct VectorSyncStore {
    memory: Arc<RwLock<HashMap<Uuid, VectorSyncProfile>>>,
    rqlite: Option<RqliteClient>,
}

impl VectorSyncStore {
    pub fn new(rqlite: Option<RqliteClient>) -> Self {
        Self {
            memory: Arc::new(RwLock::new(HashMap::new())),
            rqlite,
        }
    }

    pub async fn list(&self) -> Result<Vec<VectorSyncProfile>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq.list_json::<VectorSyncProfile>(KIND_VECTORSYNC).await {
                Ok(mut profiles) => {
                    profiles.sort_by_key(|p| p.name.to_lowercase());
                    let mut map = self.memory.write().await;
                    map.clear();
                    for p in &profiles {
                        map.insert(p.id, p.clone());
                    }
                    return Ok(profiles);
                }
                Err(e) => warn!(error = %e, "rqlite list vectorsync profiles failed; using memory"),
            }
        }
        let mut profiles: Vec<_> = self.memory.read().await.values().cloned().collect();
        profiles.sort_by_key(|p| p.name.to_lowercase());
        Ok(profiles)
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<VectorSyncProfile>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq
                .get_json::<VectorSyncProfile>(KIND_VECTORSYNC, &id.to_string())
                .await
            {
                Ok(found) => {
                    if let Some(p) = &found {
                        self.memory.write().await.insert(p.id, p.clone());
                    }
                    return Ok(found);
                }
                Err(e) => warn!(error = %e, "rqlite get vectorsync profile failed"),
            }
        }
        Ok(self.memory.read().await.get(&id).cloned())
    }

    pub async fn upsert(&self, profile: &VectorSyncProfile) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.put_json(KIND_VECTORSYNC, &profile.id.to_string(), profile)
                .await?;
        }
        self.memory
            .write()
            .await
            .insert(profile.id, profile.clone());
        Ok(())
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.delete(KIND_VECTORSYNC, &id.to_string()).await?;
        }
        self.memory.write().await.remove(&id);
        Ok(())
    }
}

// ── validation ──────────────────────────────────────────────────────────────────────────

fn valid_collection(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric() || c == '_')
        && s.len() <= 63
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn valid_topic(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 249
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Field checks the UI can show next to the input. The sidecar re-checks templates.
pub fn validate(req: &VectorSyncProfileRequest) -> Result<(), AppError> {
    validate_basics(req)?;
    validate_embedding(&req.embedding)?;
    validate_target(&req.target)?;
    validate_projection(&req.projection)?;
    validate_id_strategy(&req.id_strategy)
}

fn bad(m: &str) -> Result<(), AppError> {
    Err(AppError::BadRequest(m.to_string()))
}

fn validate_basics(req: &VectorSyncProfileRequest) -> Result<(), AppError> {
    if req.name.trim().is_empty() {
        return bad("name is required");
    }
    if req.topics.is_empty() {
        return bad("topics: pick at least one table to sync");
    }
    if let Some(t) = req.topics.iter().find(|t| !valid_topic(t)) {
        return Err(AppError::BadRequest(format!(
            "topics: {t:?} is not a topic name"
        )));
    }
    if !matches!(req.start_from.as_str(), "" | "earliest" | "latest") {
        return bad("start_from must be earliest or latest");
    }
    if !matches!(req.delete_mode.as_str(), "" | "hard" | "soft") {
        return bad("delete_mode must be hard or soft");
    }
    Ok(())
}

fn validate_embedding(e: &VectorSyncEmbedding) -> Result<(), AppError> {
    let provider = e.provider.trim();
    if !provider.is_empty() && !PROVIDERS.contains(&provider) {
        return bad("embedding.provider must be ollama, openai or hash");
    }
    if provider == "openai" && e.model.trim().is_empty() {
        return bad("embedding.model is required for an OpenAI-compatible server");
    }
    if e.dimensions.is_some_and(|d| d == 0 || d > 16000) {
        return bad("embedding.dimensions must be 1-16000");
    }
    Ok(())
}

fn validate_target(t: &VectorSyncTarget) -> Result<(), AppError> {
    if t.host.trim().is_empty() || t.database.trim().is_empty() || t.username.trim().is_empty() {
        return bad("target host, database and username are required");
    }
    if !t.port.trim().is_empty() && !matches!(t.port.trim().parse::<u32>(), Ok(1..=65535)) {
        return bad("target.port must be 1-65535");
    }
    if !valid_collection(t.collection.trim()) {
        return bad(
            "target.collection: letters, digits, _ . - (max 63, starts with a letter, digit or _)",
        );
    }
    Ok(())
}

fn validate_projection(projection: &Value) -> Result<(), AppError> {
    let err = projection_tables(projection)
        .into_iter()
        .find_map(|(name, cfg)| {
            let tpl = cfg.get("content_template").and_then(Value::as_str)?;
            template_error(tpl).map(|e| format!("projection {name}: {e}"))
        });
    err.map_or(Ok(()), |e| Err(AppError::BadRequest(e)))
}

fn validate_id_strategy(id: &Value) -> Result<(), AppError> {
    let composite = id.get("kind").and_then(Value::as_str) == Some("composite");
    let no_fields = id
        .get("pk_fields")
        .and_then(Value::as_array)
        .is_none_or(|a| a.is_empty());
    if composite && no_fields {
        return bad("id_strategy composite needs pk_fields");
    }
    Ok(())
}

fn projection_tables(p: &Value) -> Vec<(String, &Value)> {
    let mut out = Vec::new();
    if let Some(d) = p.get("default") {
        out.push(("default".to_string(), d));
    }
    if let Some(tables) = p.get("tables").and_then(Value::as_object) {
        for (k, v) in tables {
            out.push((k.clone(), v));
        }
    }
    out
}

/// Python `string.Template` rules: `$name`, `${name}`, `$$`. Anything else after `$` is invalid.
pub fn template_error(tpl: &str) -> Option<String> {
    let b = tpl.as_bytes();
    let mut i = 0;
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    while i < b.len() {
        if b[i] != b'$' {
            i += 1;
            continue;
        }
        match b.get(i + 1) {
            Some(b'$') => i += 2,
            Some(b'{') => {
                let end = tpl[i + 2..].find('}').map(|e| i + 2 + e);
                match end {
                    Some(e)
                        if e > i + 2
                            && b[i + 2..e].iter().all(|c| ident(*c))
                            && !b[i + 2].is_ascii_digit() =>
                    {
                        i = e + 1
                    }
                    _ => return Some("Invalid template: ${...} must hold a column name".into()),
                }
            }
            Some(c) if (c.is_ascii_alphabetic() || *c == b'_') => {
                i += 2;
                while i < b.len() && ident(b[i]) {
                    i += 1;
                }
            }
            _ => return Some(
                "Invalid template: use $column or ${column}; write $$ for a literal dollar sign"
                    .into(),
            ),
        }
    }
    None
}

// ── profile <-> request, secrets ────────────────────────────────────────────────────────

pub fn from_request(req: VectorSyncProfileRequest, id: Uuid) -> VectorSyncProfile {
    let now = chrono::Utc::now();
    let mut p = VectorSyncProfile {
        id,
        name: String::new(),
        description: String::new(),
        enabled: true,
        source_profile_id: None,
        topics: vec![],
        start_from: String::new(),
        embedding: Default::default(),
        target: Default::default(),
        projection: Value::Null,
        id_strategy: Value::Null,
        delete_mode: String::new(),
        batch: Value::Null,
        generation: 0,
        created_at: now,
        updated_at: now,
    };
    apply_request(&mut p, req);
    p
}

pub fn apply_request(p: &mut VectorSyncProfile, req: VectorSyncProfileRequest) {
    p.name = req.name.trim().to_string();
    p.description = req.description.trim().to_string();
    p.enabled = req.enabled;
    p.source_profile_id = req.source_profile_id;
    let mut topics: Vec<String> = req
        .topics
        .into_iter()
        .map(|t| t.trim().to_string())
        .collect();
    topics.sort();
    topics.dedup();
    p.topics = topics;
    p.start_from = if req.start_from == "latest" {
        "latest".into()
    } else {
        "earliest".into()
    };
    p.embedding = req.embedding;
    if p.embedding.provider.trim().is_empty() {
        p.embedding.provider = "ollama".into();
    }
    p.target = req.target;
    p.target.collection = p.target.collection.trim().to_string();
    p.projection = req.projection;
    p.id_strategy = req.id_strategy;
    p.delete_mode = if req.delete_mode == "soft" {
        "soft".into()
    } else {
        "hard".into()
    };
    p.batch = req.batch;
    p.updated_at = chrono::Utc::now();
}

/// `********` (or empty for the API key) in an update keeps the stored secret.
pub fn keep_secrets(new: &mut VectorSyncProfile, old: &VectorSyncProfile) {
    if new.target.password == REDACTED {
        new.target.password = old.target.password.clone();
    }
    if new.embedding.api_key == REDACTED {
        new.embedding.api_key = old.embedding.api_key.clone();
    }
}

pub fn redact(mut p: VectorSyncProfile) -> VectorSyncProfile {
    if !p.target.password.is_empty() {
        p.target.password = REDACTED.into();
    }
    if !p.embedding.api_key.is_empty() {
        p.embedding.api_key = REDACTED.into();
    }
    p
}

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

/// Encrypt secrets at rest when ORCH_SECRET_KEY is configured (ADR 0015 secretbox).
pub fn seal_secrets(cfg: &AppConfig, p: &mut VectorSyncProfile) {
    p.target.password = seal(cfg, &p.target.password);
    p.embedding.api_key = seal(cfg, &p.embedding.api_key);
}

/// Plain secrets for a request to the sidecar.
pub fn open_secrets(cfg: &AppConfig, p: &mut VectorSyncProfile) -> Result<(), AppError> {
    p.target.password = unseal(cfg, &p.target.password)?;
    p.embedding.api_key = unseal(cfg, &p.embedding.api_key)?;
    Ok(())
}

// ── sidecar spec ────────────────────────────────────────────────────────────────────────

pub fn revision(p: &VectorSyncProfile) -> String {
    p.updated_at.to_rfc3339()
}

pub fn spec(cfg: &AppConfig, p: &VectorSyncProfile) -> Result<Value, AppError> {
    let mut p = p.clone();
    open_secrets(cfg, &mut p)?;
    Ok(json!({
        "id": p.id.to_string(),
        "name": p.name,
        "revision": revision(&p),
        "generation": p.generation,
        "topics": p.topics,
        "start_from": p.start_from,
        "embedding": p.embedding,
        "target": p.target,
        "projection": p.projection,
        "id_strategy": p.id_strategy,
        "delete_mode": p.delete_mode,
        "batch": p.batch,
    }))
}

pub async fn push(state: &AppState, p: &VectorSyncProfile) -> Result<Value, AppError> {
    let body = spec(&state.config, p)?;
    sidecar(
        &state.config,
        &state.http,
        Method::PUT,
        &format!("/v1/syncs/{}", p.id),
        Some(body),
    )
    .await
}

pub async fn stop(state: &AppState, id: Uuid, forget: bool) -> Result<Value, AppError> {
    let q = if forget { "?forget=true" } else { "" };
    sidecar(
        &state.config,
        &state.http,
        Method::DELETE,
        &format!("/v1/syncs/{}{q}", encode(&id.to_string())),
        None,
    )
    .await
}

/// Running status by sync id (empty when the sidecar is unreachable).
pub async fn live_status(state: &AppState) -> Result<HashMap<String, Value>, AppError> {
    let v = sidecar(&state.config, &state.http, Method::GET, "/v1/syncs", None).await?;
    Ok(v.get("syncs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| Some((s.get("id")?.as_str()?.to_string(), s.clone())))
        .collect())
}

/// What the UI shows for a sync: stored state + the sidecar's live view.
pub fn with_status(p: VectorSyncProfile, live: Option<&Value>, sidecar_ok: bool) -> Value {
    let running_rev = live.and_then(|l| l.get("revision")).and_then(Value::as_str);
    let mut status = match (p.enabled, live, sidecar_ok) {
        (false, _, _) => json!({"state": "paused", "message": "Paused"}),
        (true, Some(l), _) => l.clone(),
        (true, None, true) => {
            json!({"state": "pending", "message": "Waiting for the sidecar to start it"})
        }
        (true, None, false) => {
            json!({"state": "unknown", "message": "The vectorsync sidecar is not reachable"})
        }
    };
    if p.enabled && running_rev.is_some_and(|r| r != revision(&p)) {
        status["state"] = json!("pending");
        status["message"] = json!("Applying the latest changes");
    }
    let mut out = serde_json::to_value(redact(p)).unwrap_or(Value::Null);
    out["status"] = status;
    out
}

// ── reconcile ───────────────────────────────────────────────────────────────────────────

pub fn spawn_reconcile(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if !enabled(&state.config) {
            return;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
        loop {
            if let Err(e) = reconcile(&state).await {
                warn!(error = %e, "vectorsync reconcile failed");
            }
            tokio::time::sleep(Duration::from_secs(state.config.vectorsync_reconcile_secs)).await;
        }
    })
}

/// Desired (enabled profiles) vs running (sidecar): push new/changed specs, stop the rest.
pub async fn reconcile(state: &AppState) -> Result<(), AppError> {
    let live = live_status(state).await?;
    let profiles = state.vectorsync_profiles.list().await?;
    let mut wanted = std::collections::HashSet::new();
    for p in profiles.iter().filter(|p| p.enabled) {
        let id = p.id.to_string();
        wanted.insert(id.clone());
        let current = live.get(&id);
        let same = current.is_some_and(|l| {
            l.get("revision").and_then(Value::as_str) == Some(revision(p).as_str())
                && l.get("generation").and_then(Value::as_u64) == Some(u64::from(p.generation))
        });
        if !same {
            if let Err(e) = push(state, p).await {
                warn!(sync = %p.name, error = %e, "vectorsync push failed");
            }
        }
    }
    for id in live.keys().filter(|id| !wanted.contains(*id)) {
        if let Ok(uuid) = Uuid::parse_str(id) {
            let _ = stop(state, uuid, false).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{VectorSyncEmbedding, VectorSyncTarget};

    fn req() -> VectorSyncProfileRequest {
        VectorSyncProfileRequest {
            name: "Orders search".into(),
            description: String::new(),
            enabled: true,
            source_profile_id: None,
            topics: vec!["demo.bookings.orders".into()],
            start_from: String::new(),
            embedding: VectorSyncEmbedding {
                provider: "ollama".into(),
                model: "bge-m3".into(),
                ..Default::default()
            },
            target: VectorSyncTarget {
                host: "cdc-pgvector".into(),
                database: "vector".into(),
                username: "vector".into(),
                password: "vector".into(),
                collection: "orders_docs".into(),
                ..Default::default()
            },
            projection: json!({"tables": {"orders": {"content_template": "Order $id: ${note} costs $$5"}}}),
            id_strategy: Value::Null,
            delete_mode: String::new(),
            batch: Value::Null,
        }
    }

    #[test]
    fn valid_request_passes_and_defaults_apply() {
        validate(&req()).unwrap();
        let p = from_request(req(), Uuid::new_v4());
        assert_eq!(p.start_from, "earliest");
        assert_eq!(p.delete_mode, "hard");
        assert!(p.enabled);
    }

    /// Breaks a valid request in one way; the expected error mentions the &str.
    type Mutation = Box<dyn Fn(&mut VectorSyncProfileRequest)>;

    #[test]
    fn rejects_bad_fields() {
        let cases: Vec<(Mutation, &str)> = vec![
            (Box::new(|r| r.topics.clear()), "topics"),
            (
                Box::new(|r| r.topics = vec!["bad topic".into()]),
                "topic name",
            ),
            (
                Box::new(|r| r.target.collection = "-x".into()),
                "collection",
            ),
            (Box::new(|r| r.target.port = "99999".into()), "port"),
            (
                Box::new(|r| r.embedding.provider = "nope".into()),
                "provider",
            ),
            (Box::new(|r| r.start_from = "middle".into()), "start_from"),
            (
                Box::new(|r| r.projection = json!({"default": {"content_template": "cost $"}})),
                "template",
            ),
            (
                Box::new(|r| r.id_strategy = json!({"kind": "composite"})),
                "pk_fields",
            ),
        ];
        for (mutate, want) in cases {
            let mut r = req();
            mutate(&mut r);
            let err = validate(&r).unwrap_err().to_string();
            assert!(err.contains(want), "{err} should mention {want}");
        }
    }

    #[test]
    fn template_rules_match_python_string_template() {
        assert!(template_error("$a ${b_1} $$ 5").is_none());
        assert!(template_error("price $").is_some());
        assert!(template_error("${}").is_some());
        assert!(template_error("${1a}").is_some());
        assert!(template_error("$1").is_some());
    }

    #[test]
    fn secrets_are_redacted_and_kept_on_update() {
        let old = from_request(req(), Uuid::new_v4());
        let shown = redact(old.clone());
        assert_eq!(shown.target.password, REDACTED);
        assert_eq!(shown.embedding.api_key, "");
        let mut new = shown.clone();
        keep_secrets(&mut new, &old);
        assert_eq!(new.target.password, "vector");
    }

    #[test]
    fn status_reflects_pause_pending_and_revision_drift() {
        let p = from_request(req(), Uuid::new_v4());
        let mut paused = p.clone();
        paused.enabled = false;
        assert_eq!(with_status(paused, None, true)["status"]["state"], "paused");
        assert_eq!(
            with_status(p.clone(), None, true)["status"]["state"],
            "pending"
        );
        assert_eq!(
            with_status(p.clone(), None, false)["status"]["state"],
            "unknown"
        );
        let live = json!({"state": "running", "revision": "old"});
        assert_eq!(
            with_status(p.clone(), Some(&live), true)["status"]["state"],
            "pending"
        );
        let live = json!({"state": "running", "revision": revision(&p)});
        assert_eq!(
            with_status(p, Some(&live), true)["status"]["state"],
            "running"
        );
    }

    #[test]
    fn collection_and_topic_names() {
        assert!(valid_collection("orders_docs.v2"));
        assert!(!valid_collection(".x"));
        assert!(!valid_collection(&"a".repeat(64)));
        assert!(valid_topic("cdc_hydra.public.networks"));
        assert!(!valid_topic("a b"));
    }
}
