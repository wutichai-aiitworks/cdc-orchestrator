use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit::log_action;
use crate::error::AppError;
use crate::models::{
    MongoCopyProfile, MongoCopyProfileRequest, MongoCopyProfilesResponse, SchemaCompareConn,
    ValidationResponse,
};
use crate::mongocopy::{self, profile_run_payload, require_mongo_pair};
use crate::saved_secret::keep_password;
use crate::tool_events;
use crate::AppState;

fn actor_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-user")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .map(|_| "bearer")
        })
        .unwrap_or("api")
        .to_string()
}

pub async fn mongocopy_capability(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    mongocopy::capability(&state.config, &state.http)
        .await
        .map(Json)
}

pub async fn list_mongocopy_jobs(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    mongocopy::list_jobs(&state.config, &state.http)
        .await
        .map(Json)
}

pub async fn get_mongocopy_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    mongocopy::get_job(&state.config, &state.http, &job_id)
        .await
        .map(Json)
}

pub async fn mongocopy_job_logs(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    mongocopy::job_logs(&state.config, &state.http, &job_id)
        .await
        .map(Json)
}

pub async fn stop_mongocopy_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let out = mongocopy::stop_job(&state.config, &state.http, &job_id).await?;
    tool_events::stop_requested(
        &state,
        "mongocopy",
        &actor_from_headers(&headers),
        &job_id,
        &out,
    )
    .await;
    log_action(
        &actor_from_headers(&headers),
        "mongocopy.job.stop",
        "success",
        &format!("job_id={job_id}"),
    );
    Ok(Json(out))
}

pub async fn list_mongocopy_profiles(
    State(state): State<AppState>,
) -> Result<Json<MongoCopyProfilesResponse>, AppError> {
    let mut profiles = state.mongocopy_profiles.list().await?;
    for profile in &mut profiles {
        profile.redact_secrets();
    }
    Ok(Json(MongoCopyProfilesResponse { profiles }))
}

pub async fn create_mongocopy_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<MongoCopyProfileRequest>,
) -> Result<Json<MongoCopyProfile>, AppError> {
    require_mongo_pair(&req.source, &req.target)?;
    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("Profile name is required".into()));
    }
    let now = Utc::now();
    let profile = MongoCopyProfile {
        id: Uuid::new_v4(),
        name: req.name.trim().to_string(),
        source: req.source,
        target: req.target,
        include_collections: req.include_collections,
        options: req.options,
        last_job_id: None,
        created_at: now,
        updated_at: now,
    };
    state.mongocopy_profiles.upsert(&profile).await?;
    tool_events::profile_created(
        &state,
        "mongocopy",
        &actor_from_headers(&headers),
        profile.id,
        &profile.name,
        &profile,
    )
    .await;
    log_action(
        &actor_from_headers(&headers),
        "mongocopy.profile.create",
        "success",
        &format!("id={} name={}", profile.id, profile.name),
    );
    Ok(Json(profile))
}

pub async fn get_mongocopy_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<MongoCopyProfile>, AppError> {
    let mut profile = state
        .mongocopy_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("mongocopy profile {id} not found")))?;
    profile.redact_secrets();
    Ok(Json(profile))
}

pub async fn update_mongocopy_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<MongoCopyProfileRequest>,
) -> Result<Json<MongoCopyProfile>, AppError> {
    require_mongo_pair(&req.source, &req.target)?;
    let existing = state
        .mongocopy_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("mongocopy profile {id} not found")))?;
    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("Profile name is required".into()));
    }
    let mut source = req.source;
    let mut target = req.target;
    // Stored passwords stay with their login; an edited host must be given the password again.
    keep_password("source", &mut source, Some(&existing.source))?;
    keep_password("target", &mut target, Some(&existing.target))?;
    let profile = MongoCopyProfile {
        id,
        name: req.name.trim().to_string(),
        source,
        target,
        include_collections: req.include_collections,
        options: req.options,
        last_job_id: existing.last_job_id.clone(),
        created_at: existing.created_at,
        updated_at: Utc::now(),
    };
    state.mongocopy_profiles.upsert(&profile).await?;
    tool_events::profile_updated(
        &state,
        "mongocopy",
        &actor_from_headers(&headers),
        id,
        &profile.name,
        &existing,
        &profile,
    )
    .await;
    log_action(
        &actor_from_headers(&headers),
        "mongocopy.profile.update",
        "success",
        &format!("id={}", profile.id),
    );
    Ok(Json(profile))
}

pub async fn delete_mongocopy_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let deleted_name = state
        .mongocopy_profiles
        .get(id)
        .await
        .ok()
        .flatten()
        .map(|p| p.name);
    state.mongocopy_profiles.delete(id).await?;
    tool_events::profile_deleted(
        &state,
        "mongocopy",
        &actor_from_headers(&headers),
        id,
        deleted_name.as_deref(),
    )
    .await;
    log_action(
        &actor_from_headers(&headers),
        "mongocopy.profile.delete",
        "success",
        &format!("id={id}"),
    );
    Ok(Json(json!({ "status": true, "id": id })))
}

pub async fn run_mongocopy_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let mut profile = state
        .mongocopy_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("mongocopy profile {id} not found")))?;
    require_mongo_pair(&profile.source, &profile.target)?;
    let body = profile_run_payload(&profile);
    // WireGuard (ADR 0010): connections with a tunnel go to a gateway forward.
    let mut run_body = body;
    let started = match crate::wireguard::route_payload(&state, &mut run_body).await {
        Ok(_) => mongocopy::start_job(&state.config, &state.http, run_body).await,
        Err(e) => Err(e),
    };
    tool_events::run_requested(
        &state,
        "mongocopy",
        &actor_from_headers(&headers),
        id,
        &profile.name,
        &started,
        false,
    )
    .await;
    let job = started?;
    if let Some(job_id) = job.get("id").and_then(|v| v.as_str()) {
        profile.last_job_id = Some(job_id.to_string());
        profile.updated_at = Utc::now();
        state.mongocopy_profiles.upsert(&profile).await?;
    }
    log_action(
        &actor_from_headers(&headers),
        "mongocopy.profile.run",
        "success",
        &format!("profile_id={id}"),
    );
    Ok(Json(job))
}

#[derive(serde::Deserialize)]
pub struct MongoCopyValidateRequest {
    pub connection: SchemaCompareConn,
}

pub async fn validate_mongocopy_connection(
    Json(req): Json<MongoCopyValidateRequest>,
) -> Result<Json<ValidationResponse>, AppError> {
    if !mongocopy::is_mongo(&req.connection.db_type) {
        return Ok(Json(ValidationResponse {
            valid: false,
            message: "mongocopy supports MongoDB only".into(),
            timestamp: Utc::now(),
        }));
    }
    if req.connection.host.trim().is_empty() {
        return Ok(Json(ValidationResponse {
            valid: false,
            message: "host is required".into(),
            timestamp: Utc::now(),
        }));
    }
    Ok(Json(ValidationResponse {
        valid: true,
        message: "Connection fields look valid; run copy to verify credentials.".into(),
        timestamp: Utc::now(),
    }))
}

/// Connect from the mongocopy sidecar (where the tool runs) and describe the server. A failed
/// connection is a normal answer (`ok: false`, `error`, `hint`).
pub async fn test_mongocopy_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<crate::tool_sidecar::TestConnectionRequest>,
) -> Result<Json<Value>, AppError> {
    let side = req.side.clone().unwrap_or_else(|| "source".into());
    let mut tested: Option<(Uuid, String)> = None;
    let stored = match req.profile_id {
        Some(id) => Some(
            state
                .mongocopy_profiles
                .get(id)
                .await?
                .map(|p| {
                    tested = Some((id, p.name));
                    (p.source, p.target)
                })
                .ok_or_else(|| AppError::NotFound(format!("mongocopy profile {id} not found")))?,
        ),
        None => None,
    };
    let body = crate::tool_sidecar::test_body(req, stored)?;
    let mut body = body;
    let via = crate::wireguard::route_payload(&state, &mut body).await?;
    let mut out = mongocopy::test_connection(&state.config, &state.http, body).await?;
    if !via.is_empty() {
        out["via_tunnel"] = json!(via.join(", "));
    }
    tool_events::connection_tested(
        &state,
        "mongocopy",
        &actor_from_headers(&headers),
        tested,
        &side,
        &out,
    )
    .await;
    Ok(Json(out))
}
