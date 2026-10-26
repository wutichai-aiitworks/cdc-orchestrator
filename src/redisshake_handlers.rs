use axum::extract::{Path, State};
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
use crate::redisshake::{self, profile_run_payload, require_redis_pair};
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
    Json(req): Json<RedisShakeProfileRequest>,
) -> Result<Json<RedisShakeProfile>, AppError> {
    require_redis_pair(&req.source, &req.target)?;
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
    Json(req): Json<RedisShakeProfileRequest>,
) -> Result<Json<RedisShakeProfile>, AppError> {
    require_redis_pair(&req.source, &req.target)?;
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
    if source.password == "********" {
        source.password = existing.source.password;
    }
    if target.password == "********" {
        target.password = existing.target.password;
    }

    let profile = RedisShakeProfile {
        id,
        name: req.name.trim().to_string(),
        source,
        target,
        options: req.options,
        last_job_id: existing.last_job_id,
        created_at: existing.created_at,
        updated_at: Utc::now(),
    };
    state.redisshake_profiles.upsert(&profile).await?;
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
    state.redisshake_profiles.delete(id).await?;
    log_action(
        &actor_from_headers(&headers),
        "redisshake.profile.delete",
        "success",
        &format!("id={id}"),
    );
    Ok(Json(json!({ "status": true, "id": id })))
}

pub async fn run_redisshake_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let mut profile = state
        .redisshake_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("redisshake profile {id} not found")))?;
    require_redis_pair(&profile.source, &profile.target)?;

    let body = profile_run_payload(&profile);
    let job = redisshake::start_job(&state.config, &state.http, body).await?;
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
