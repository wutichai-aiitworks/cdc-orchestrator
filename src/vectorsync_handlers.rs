//! `/cdc/vectorsync/*` — AI vector sync (ADR 0018). See `vectorsync.rs` for the model.

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use urlencoding::encode;
use uuid::Uuid;

use crate::audit::log_action;
use crate::error::AppError;
use crate::models::{
    VectorSyncEmbedding, VectorSyncProfile, VectorSyncProfileRequest, VectorSyncTarget,
};
use crate::saved_secret::REDACTED;
use crate::vectorsync::{self, sidecar};
use crate::AppState;

fn actor(headers: &HeaderMap) -> String {
    headers
        .get("x-user")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .unwrap_or("api")
        .to_string()
}

async fn load(state: &AppState, id: Uuid) -> Result<VectorSyncProfile, AppError> {
    state
        .vectorsync_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("vector sync {id} not found")))
}

/// Saved secrets for a test/preview against an existing sync: only when the request still
/// points at the same login (password) or the same server (API key). A changed host never
/// receives a stored secret.
async fn fill_saved(
    state: &AppState,
    profile_id: Option<Uuid>,
    target: Option<&mut VectorSyncTarget>,
    embedding: Option<&mut VectorSyncEmbedding>,
) -> Result<(), AppError> {
    let Some(id) = profile_id else { return Ok(()) };
    let mut saved = load(state, id).await?;
    vectorsync::open_secrets(&state.config, &mut saved)?;
    if let Some(t) = target {
        if t.password == REDACTED {
            let same = t.host.trim() == saved.target.host
                && t.username.trim() == saved.target.username
                && t.database.trim() == saved.target.database
                && t.port.trim() == saved.target.port.trim();
            t.password = if same {
                saved.target.password.clone()
            } else {
                String::new()
            };
        }
    }
    if let Some(e) = embedding {
        if e.api_key == REDACTED {
            let same = e.base_url.trim() == saved.embedding.base_url.trim()
                && e.provider == saved.embedding.provider;
            e.api_key = if same {
                saved.embedding.api_key.clone()
            } else {
                String::new()
            };
        }
    }
    Ok(())
}

// ── capability / sources / test / preview ───────────────────────────────────────────────

pub async fn capability(State(state): State<AppState>) -> Json<Value> {
    if !vectorsync::enabled(&state.config) {
        return Json(json!({
            "enabled": false,
            "hint": "Set VECTORSYNC_ENABLED=true in docker/.env, then ./docker/deploy.sh (starts cdc-vectorsync and shows this page)."
        }));
    }
    match sidecar(
        &state.config,
        &state.http,
        Method::GET,
        "/v1/capability",
        None,
    )
    .await
    {
        Ok(mut v) => {
            v["enabled"] = json!(true);
            v["sidecar_ready"] = json!(true);
            v["secrets_encrypted"] = json!(state.config.secret_key.is_some());
            Json(v)
        }
        Err(e) => Json(json!({
            "enabled": true,
            "sidecar_ready": false,
            "error": e.to_string(),
            "hint": "cdc-vectorsync is not answering. Check `docker logs cdc-vectorsync`."
        })),
    }
}

/// CDC pipelines with the topics they currently have (for the "pick tables" step), plus any
/// other data topics under `other`.
pub async fn sources(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let topics = sidecar(&state.config, &state.http, Method::GET, "/v1/topics", None).await?;
    let topics: Vec<Value> = topics
        .get("topics")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut profiles = state.profiles.list().await?;
    // Longest prefix first so `crm_eu.` wins over `crm.` for crm_eu topics.
    profiles.sort_by_key(|p| std::cmp::Reverse(crate::reconcile::topic_prefix_for(p).len()));
    let mut claimed = vec![false; topics.len()];
    let mut groups = Vec::new();
    for p in &profiles {
        let prefix = format!("{}.", crate::reconcile::topic_prefix_for(p));
        let mut mine = Vec::new();
        for (i, t) in topics.iter().enumerate() {
            let name = t.get("name").and_then(Value::as_str).unwrap_or_default();
            if !claimed[i] && name.starts_with(&prefix) {
                claimed[i] = true;
                let mut t = t.clone();
                t["table"] = json!(&name[prefix.len()..]);
                mine.push(t);
            }
        }
        groups.push(json!({
            "profile_id": p.id,
            "name": p.name,
            "status": p.status,
            "source_type": p.source.source_type,
            "topic_prefix": prefix,
            "topics": mine,
        }));
    }
    groups.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase()
            .cmp(&b["name"].as_str().unwrap_or_default().to_lowercase())
    });
    let other: Vec<Value> = topics
        .into_iter()
        .zip(claimed)
        .filter(|(_, c)| !c)
        .map(|(t, _)| t)
        .collect();
    Ok(Json(json!({"pipelines": groups, "other": other})))
}

#[derive(Deserialize)]
pub struct TestRequest {
    #[serde(default)]
    pub profile_id: Option<Uuid>,
    #[serde(default)]
    pub embedding: Option<VectorSyncEmbedding>,
    #[serde(default)]
    pub target: Option<VectorSyncTarget>,
}

pub async fn test(
    State(state): State<AppState>,
    Json(mut req): Json<TestRequest>,
) -> Result<Json<Value>, AppError> {
    vectorsync::ensure_enabled(&state.config)?;
    fill_saved(
        &state,
        req.profile_id,
        req.target.as_mut(),
        req.embedding.as_mut(),
    )
    .await?;
    let body = json!({"embedding": req.embedding, "target": req.target});
    sidecar(
        &state.config,
        &state.http,
        Method::POST,
        "/v1/test",
        Some(body),
    )
    .await
    .map(Json)
}

pub async fn embedding_models(
    State(state): State<AppState>,
    Json(mut req): Json<TestRequest>,
) -> Result<Json<Value>, AppError> {
    vectorsync::ensure_enabled(&state.config)?;
    fill_saved(&state, req.profile_id, None, req.embedding.as_mut()).await?;
    sidecar(
        &state.config,
        &state.http,
        Method::POST,
        "/v1/embedding-models",
        Some(json!({"embedding": req.embedding})),
    )
    .await
    .map(Json)
}

pub async fn preview(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    sidecar(
        &state.config,
        &state.http,
        Method::POST,
        "/v1/preview",
        Some(body),
    )
    .await
    .map(Json)
}

// ── profiles ────────────────────────────────────────────────────────────────────────────

pub async fn list_profiles(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    vectorsync::ensure_enabled(&state.config)?;
    let profiles = state.vectorsync_profiles.list().await?;
    let (live, ok) = match vectorsync::live_status(&state).await {
        Ok(l) => (l, true),
        Err(_) => (Default::default(), false),
    };
    let items: Vec<Value> = profiles
        .into_iter()
        .map(|p| {
            let l = live.get(&p.id.to_string());
            vectorsync::with_status(p, l, ok)
        })
        .collect();
    Ok(Json(json!({"profiles": items, "sidecar_ready": ok})))
}

pub async fn get_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    vectorsync::ensure_enabled(&state.config)?;
    let p = load(&state, id).await?;
    let live = sidecar(
        &state.config,
        &state.http,
        Method::GET,
        &format!("/v1/syncs/{id}"),
        None,
    )
    .await;
    let ok = !matches!(live, Err(AppError::Upstream(_)));
    Ok(Json(vectorsync::with_status(p, live.ok().as_ref(), ok)))
}

async fn save_and_apply(state: &AppState, mut p: VectorSyncProfile) -> Result<Value, AppError> {
    vectorsync::seal_secrets(&state.config, &mut p);
    state.vectorsync_profiles.upsert(&p).await?;
    // Apply now instead of waiting for the reconcile tick; a sidecar error is shown as status.
    let applied = if p.enabled {
        vectorsync::push(state, &p).await
    } else {
        vectorsync::stop(state, p.id, false).await
    };
    let mut out = vectorsync::with_status(p, applied.as_ref().ok(), applied.is_ok());
    if let Err(e) = applied {
        out["status"] = json!({"state": "error", "message": e.to_string()});
    }
    Ok(out)
}

pub async fn create_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<VectorSyncProfileRequest>,
) -> Result<Json<Value>, AppError> {
    vectorsync::ensure_enabled(&state.config)?;
    vectorsync::validate(&req)?;
    let existing = state.vectorsync_profiles.list().await?;
    if existing.iter().any(|p| {
        p.target.collection == req.target.collection.trim()
            && p.target.host == req.target.host.trim()
            && p.target.database == req.target.database.trim()
    }) {
        return Err(AppError::Conflict(format!(
            "collection '{}' is already written by another sync — pick another name",
            req.target.collection.trim()
        )));
    }
    let p = vectorsync::from_request(req, Uuid::new_v4());
    log_action(
        &actor(&headers),
        "vectorsync.create",
        "ok",
        &format!("{} ({})", p.name, p.id),
    );
    save_and_apply(&state, p).await.map(Json)
}

pub async fn update_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<VectorSyncProfileRequest>,
) -> Result<Json<Value>, AppError> {
    vectorsync::ensure_enabled(&state.config)?;
    vectorsync::validate(&req)?;
    let old = load(&state, id).await?;
    let mut p = old.clone();
    vectorsync::apply_request(&mut p, req);
    vectorsync::keep_secrets(&mut p, &old);
    // A different model or collection makes the old vectors incomparable: rebuild.
    let model_changed = p.embedding.provider != old.embedding.provider
        || p.embedding.model != old.embedding.model
        || p.embedding.dimensions != old.embedding.dimensions;
    if model_changed || p.target.collection != old.target.collection {
        p.generation = old.generation + 1;
    }
    log_action(
        &actor(&headers),
        "vectorsync.update",
        "ok",
        &format!("{} ({id}) gen={}", p.name, p.generation),
    );
    save_and_apply(&state, p).await.map(Json)
}

#[derive(Deserialize)]
pub struct DeleteQuery {
    #[serde(default)]
    pub drop_collection: bool,
}

pub async fn delete_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<DeleteQuery>,
) -> Result<Json<Value>, AppError> {
    vectorsync::ensure_enabled(&state.config)?;
    let p = load(&state, id).await?;
    vectorsync::stop(&state, id, true).await?;
    let mut dropped = Value::Null;
    if q.drop_collection {
        let spec = vectorsync::spec(&state.config, &p)?;
        dropped = sidecar(
            &state.config,
            &state.http,
            Method::POST,
            "/v1/collections/drop",
            Some(json!({"target": spec["target"]})),
        )
        .await?;
    }
    state.vectorsync_profiles.delete(id).await?;
    log_action(
        &actor(&headers),
        "vectorsync.delete",
        "ok",
        &format!("{} ({id}) drop_collection={}", p.name, q.drop_collection),
    );
    Ok(Json(
        json!({"deleted": true, "id": id, "collection": dropped}),
    ))
}

async fn set_enabled(
    state: &AppState,
    headers: &HeaderMap,
    id: Uuid,
    enabled: bool,
) -> Result<Value, AppError> {
    vectorsync::ensure_enabled(&state.config)?;
    let mut p = load(state, id).await?;
    p.enabled = enabled;
    p.updated_at = chrono::Utc::now();
    log_action(
        &actor(headers),
        if enabled {
            "vectorsync.resume"
        } else {
            "vectorsync.pause"
        },
        "ok",
        &format!("{} ({id})", p.name),
    );
    state.vectorsync_profiles.upsert(&p).await?;
    let applied = if enabled {
        vectorsync::push(state, &p).await
    } else {
        vectorsync::stop(state, id, false).await
    };
    let mut out = vectorsync::with_status(
        p,
        applied.as_ref().ok().filter(|_| enabled),
        applied.is_ok(),
    );
    if let Err(e) = applied {
        out["status"] = json!({"state": "error", "message": e.to_string()});
    }
    Ok(out)
}

pub async fn pause(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    set_enabled(&state, &headers, id, false).await.map(Json)
}

pub async fn resume(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    set_enabled(&state, &headers, id, true).await.map(Json)
}

/// Clear the collection and re-read everything the topics still retain (new consumer group).
pub async fn rebuild(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    vectorsync::ensure_enabled(&state.config)?;
    let mut p = load(&state, id).await?;
    p.generation += 1;
    p.enabled = true;
    p.updated_at = chrono::Utc::now();
    log_action(
        &actor(&headers),
        "vectorsync.rebuild",
        "ok",
        &format!("{} ({id}) gen={}", p.name, p.generation),
    );
    state.vectorsync_profiles.upsert(&p).await?;
    let applied = vectorsync::push(&state, &p).await;
    let mut out = vectorsync::with_status(p, applied.as_ref().ok(), applied.is_ok());
    if let Err(e) = applied {
        out["status"] = json!({"state": "error", "message": e.to_string()});
    }
    Ok(Json(out))
}

pub async fn search(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    load(&state, id).await?;
    sidecar(
        &state.config,
        &state.http,
        Method::POST,
        &format!("/v1/syncs/{id}/search"),
        Some(body),
    )
    .await
    .map(Json)
}

#[derive(Deserialize)]
pub struct LimitQuery {
    #[serde(default)]
    pub limit: Option<u32>,
}

pub async fn dead_letters(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<Value>, AppError> {
    load(&state, id).await?;
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    sidecar(
        &state.config,
        &state.http,
        Method::GET,
        &format!(
            "/v1/syncs/{}/dead-letters?limit={limit}",
            encode(&id.to_string())
        ),
        None,
    )
    .await
    .map(Json)
}

pub async fn clear_dead_letters(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let p = load(&state, id).await?;
    log_action(
        &actor(&headers),
        "vectorsync.clear_dead_letters",
        "ok",
        &format!("{} ({id})", p.name),
    );
    sidecar(
        &state.config,
        &state.http,
        Method::DELETE,
        &format!("/v1/syncs/{id}/dead-letters"),
        None,
    )
    .await
    .map(Json)
}
