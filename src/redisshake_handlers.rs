use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit::log_action;
use crate::error::AppError;
use crate::models::{
    RedisShakeProfile, RedisShakeProfileRequest, RedisShakeProfilesResponse, SchemaCompareConn,
    ValidationResponse,
};
use crate::redisshake::{self, normalize_db_options, profile_run_payload, require_redis_pair};
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

pub async fn redisshake_capability(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    redisshake::capability(&state.config, &state.http)
        .await
        .map(Json)
}

pub async fn list_redisshake_jobs(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    redisshake::list_jobs(&state.config, &state.http)
        .await
        .map(Json)
}

pub async fn get_redisshake_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    redisshake::get_job(&state.config, &state.http, &job_id)
        .await
        .map(Json)
}

pub async fn redisshake_job_logs(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    redisshake::job_logs(&state.config, &state.http, &job_id)
        .await
        .map(Json)
}

pub async fn stop_redisshake_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let out = redisshake::stop_job(&state.config, &state.http, &job_id).await?;
    tool_events::stop_requested(
        &state,
        "redisshake",
        &actor_from_headers(&headers),
        &job_id,
        &out,
    )
    .await;
    log_action(
        &actor_from_headers(&headers),
        "redisshake.job.stop",
        "success",
        &format!("job_id={job_id}"),
    );
    Ok(Json(out))
}

pub async fn list_redisshake_profiles(
    State(state): State<AppState>,
) -> Result<Json<RedisShakeProfilesResponse>, AppError> {
    let mut profiles = state.redisshake_profiles.list().await?;
    for profile in &mut profiles {
        profile.redact_secrets();
    }
    Ok(Json(RedisShakeProfilesResponse { profiles }))
}

pub async fn create_redisshake_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut req): Json<RedisShakeProfileRequest>,
) -> Result<Json<RedisShakeProfile>, AppError> {
    require_redis_pair(&req.source, &req.target)?;
    normalize_db_options(&mut req.options)?;
    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("Profile name is required".into()));
    }
    let now = Utc::now();
    let profile = RedisShakeProfile {
        id: Uuid::new_v4(),
        name: req.name.trim().to_string(),
        source: req.source,
        target: req.target,
        options: req.options,
        last_job_id: None,
        created_at: now,
        updated_at: now,
    };
    state.redisshake_profiles.upsert(&profile).await?;
    tool_events::profile_created(
        &state,
        "redisshake",
        &actor_from_headers(&headers),
        profile.id,
        &profile.name,
        &profile,
    )
    .await;
    log_action(
        &actor_from_headers(&headers),
        "redisshake.profile.create",
        "success",
        &format!("id={} name={}", profile.id, profile.name),
    );
    Ok(Json(profile))
}

pub async fn get_redisshake_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<RedisShakeProfile>, AppError> {
    let mut profile = state
        .redisshake_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("redisshake profile {id} not found")))?;
    profile.redact_secrets();
    Ok(Json(profile))
}

pub async fn update_redisshake_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(mut req): Json<RedisShakeProfileRequest>,
) -> Result<Json<RedisShakeProfile>, AppError> {
    require_redis_pair(&req.source, &req.target)?;
    normalize_db_options(&mut req.options)?;
    let existing = state
        .redisshake_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("redisshake profile {id} not found")))?;

    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("Profile name is required".into()));
    }

    let mut source = req.source;
    let mut target = req.target;
    // Stored passwords stay with their login; an edited host must be given the password again.
    keep_password("source", &mut source, Some(&existing.source))?;
    keep_password("target", &mut target, Some(&existing.target))?;

    let profile = RedisShakeProfile {
        id,
        name: req.name.trim().to_string(),
        source,
        target,
        options: req.options,
        last_job_id: existing.last_job_id.clone(),
        created_at: existing.created_at,
        updated_at: Utc::now(),
    };
    state.redisshake_profiles.upsert(&profile).await?;
    tool_events::profile_updated(
        &state,
        "redisshake",
        &actor_from_headers(&headers),
        id,
        &profile.name,
        &existing,
        &profile,
    )
    .await;
    log_action(
        &actor_from_headers(&headers),
        "redisshake.profile.update",
        "success",
        &format!("id={}", profile.id),
    );
    Ok(Json(profile))
}

pub async fn delete_redisshake_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let deleted_name = state
        .redisshake_profiles
        .get(id)
        .await
        .ok()
        .flatten()
        .map(|p| p.name);
    state.redisshake_profiles.delete(id).await?;
    tool_events::profile_deleted(
        &state,
        "redisshake",
        &actor_from_headers(&headers),
        id,
        deleted_name.as_deref(),
    )
    .await;
    log_action(
        &actor_from_headers(&headers),
        "redisshake.profile.delete",
        "success",
        &format!("id={id}"),
    );
    Ok(Json(json!({ "status": true, "id": id })))
}

/// Start mode for one run, overriding the profile's default (`options.sync_rdb` / `sync_aof`).
#[derive(serde::Deserialize, Default)]
pub struct RedisShakeRunQuery {
    /// `once`: full copy, then the run ends. `cdc`: follow source changes until stopped.
    #[serde(default)]
    pub mode: Option<String>,
    /// CDC only: begin with a full copy (initial snapshot). Default: the profile's `sync_rdb`.
    #[serde(default)]
    pub snapshot: Option<bool>,
    /// Empty the target before the full copy (overrides the profile's `empty_target`).
    #[serde(default)]
    pub empty_target: Option<bool>,
}

pub async fn run_redisshake_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<RedisShakeRunQuery>,
) -> Result<Json<Value>, AppError> {
    let mut profile = state
        .redisshake_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("redisshake profile {id} not found")))?;
    require_redis_pair(&profile.source, &profile.target)?;

    // The run uses a copy of the options; the saved profile keeps its default mode.
    let mut run_profile = profile.clone();
    redisshake::apply_run_mode(
        &mut run_profile.options,
        q.mode.as_deref(),
        q.snapshot,
        q.empty_target,
    )?;
    let (label, mode_detail) = redisshake::run_mode_label(&run_profile.options);
    let body = profile_run_payload(&run_profile);
    // WireGuard (ADR 0010): connections with a tunnel go to a gateway forward.
    let mut run_body = body;
    let started = match crate::wireguard::route_payload(&state, &mut run_body).await {
        Ok(_) => redisshake::start_job(&state.config, &state.http, run_body).await,
        Err(e) => Err(e),
    };
    tool_events::run_requested_as(
        &state,
        "redisshake",
        &actor_from_headers(&headers),
        id,
        &profile.name,
        &started,
        label,
        mode_detail,
    )
    .await;
    let job = started?;
    if let Some(job_id) = job.get("id").and_then(|v| v.as_str()) {
        profile.last_job_id = Some(job_id.to_string());
        profile.updated_at = Utc::now();
        state.redisshake_profiles.upsert(&profile).await?;
    }
    log_action(
        &actor_from_headers(&headers),
        "redisshake.profile.run",
        "success",
        &format!("profile_id={id}"),
    );
    Ok(Json(job))
}

#[derive(serde::Deserialize)]
pub struct RedisShakeValidateRequest {
    pub connection: SchemaCompareConn,
}

pub async fn validate_redisshake_connection(
    Json(req): Json<RedisShakeValidateRequest>,
) -> Result<Json<ValidationResponse>, AppError> {
    if !redisshake::is_redis(&req.connection.db_type) {
        return Ok(Json(ValidationResponse {
            valid: false,
            message: "RedisShake supports Redis / Valkey / Tair only".into(),
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
        message: "Connection fields look valid; run sync to verify PSync and ACL access.".into(),
        timestamp: Utc::now(),
    }))
}

/// Connect from the redisshake sidecar (where the tool runs) and describe the server. A failed
/// connection is a normal answer (`ok: false`, `error`, `hint`).
pub async fn test_redisshake_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<crate::tool_sidecar::TestConnectionRequest>,
) -> Result<Json<Value>, AppError> {
    let side = req.side.clone().unwrap_or_else(|| "source".into());
    let mut tested: Option<(Uuid, String)> = None;
    let stored = match req.profile_id {
        Some(id) => Some(
            state
                .redisshake_profiles
                .get(id)
                .await?
                .map(|p| {
                    tested = Some((id, p.name));
                    (p.source, p.target)
                })
                .ok_or_else(|| AppError::NotFound(format!("redisshake profile {id} not found")))?,
        ),
        None => None,
    };
    let body = crate::tool_sidecar::test_body(req, stored)?;
    let mut body = body;
    let via = crate::wireguard::route_payload(&state, &mut body).await?;
    let mut out = redisshake::test_connection(&state.config, &state.http, body).await?;
    if !via.is_empty() {
        out["via_tunnel"] = json!(via.join(", "));
    }
    tool_events::connection_tested(
        &state,
        "redisshake",
        &actor_from_headers(&headers),
        tested,
        &side,
        &out,
    )
    .await;
    Ok(Json(out))
}
