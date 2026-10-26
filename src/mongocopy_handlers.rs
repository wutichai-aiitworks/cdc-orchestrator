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
    if source.password == "********" {
        source.password = existing.source.password;
    }
    if target.password == "********" {
        target.password = existing.target.password;
    }
    let profile = MongoCopyProfile {
        id,
        name: req.name.trim().to_string(),
        source,
        target,
        include_collections: req.include_collections,
        options: req.options,
        last_job_id: existing.last_job_id,
        created_at: existing.created_at,
        updated_at: Utc::now(),
    };
    state.mongocopy_profiles.upsert(&profile).await?;
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
    state.mongocopy_profiles.delete(id).await?;
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
    let job = mongocopy::start_job(&state.config, &state.http, body).await?;
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
