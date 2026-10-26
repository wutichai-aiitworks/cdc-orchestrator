use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit::log_action;
use crate::error::AppError;
use crate::models::{
    PgLoaderProfile, PgLoaderProfileRequest, PgLoaderProfilesResponse, SchemaCompareConn,
    ValidationResponse,
};
use crate::pgloader::{self, profile_run_payload, require_pgloader_pair};
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

pub async fn pgloader_capability(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    pgloader::capability(&state.config, &state.http)
        .await
        .map(Json)
}

pub async fn list_pgloader_jobs(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    pgloader::list_jobs(&state.config, &state.http)
        .await
        .map(Json)
}

pub async fn get_pgloader_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    pgloader::get_job(&state.config, &state.http, &job_id)
        .await
        .map(Json)
}

pub async fn pgloader_job_logs(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    pgloader::job_logs(&state.config, &state.http, &job_id)
        .await
        .map(Json)
}

pub async fn stop_pgloader_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let out = pgloader::stop_job(&state.config, &state.http, &job_id).await?;
    log_action(
        &actor_from_headers(&headers),
        "pgloader.job.stop",
        "success",
        &format!("job_id={job_id}"),
    );
    Ok(Json(out))
}

pub async fn list_pgloader_profiles(
    State(state): State<AppState>,
) -> Result<Json<PgLoaderProfilesResponse>, AppError> {
    let mut profiles = state.pgloader_profiles.list().await?;
    for profile in &mut profiles {
        profile.redact_secrets();
    }
    Ok(Json(PgLoaderProfilesResponse { profiles }))
}

pub async fn create_pgloader_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PgLoaderProfileRequest>,
) -> Result<Json<PgLoaderProfile>, AppError> {
    require_pgloader_pair(&req.source, &req.target)?;
    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("Profile name is required".into()));
    }
    let now = Utc::now();
    let profile = PgLoaderProfile {
        id: Uuid::new_v4(),
        name: req.name.trim().to_string(),
        source: req.source,
        target: req.target,
        include_tables: req.include_tables,
        options: req.options,
        last_job_id: None,
        created_at: now,
        updated_at: now,
    };
    state.pgloader_profiles.upsert(&profile).await?;
    log_action(
        &actor_from_headers(&headers),
        "pgloader.profile.create",
        "success",
        &format!("id={} name={}", profile.id, profile.name),
    );
    Ok(Json(profile))
}

pub async fn get_pgloader_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PgLoaderProfile>, AppError> {
    let mut profile = state
        .pgloader_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("pgloader profile {id} not found")))?;
    profile.redact_secrets();
    Ok(Json(profile))
}

pub async fn update_pgloader_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<PgLoaderProfileRequest>,
) -> Result<Json<PgLoaderProfile>, AppError> {
    require_pgloader_pair(&req.source, &req.target)?;
    let existing = state
        .pgloader_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("pgloader profile {id} not found")))?;
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
    let profile = PgLoaderProfile {
        id,
        name: req.name.trim().to_string(),
        source,
        target,
        include_tables: req.include_tables,
        options: req.options,
        last_job_id: existing.last_job_id,
        created_at: existing.created_at,
        updated_at: Utc::now(),
    };
    state.pgloader_profiles.upsert(&profile).await?;
    log_action(
        &actor_from_headers(&headers),
        "pgloader.profile.update",
        "success",
        &format!("id={}", profile.id),
    );
    Ok(Json(profile))
}

pub async fn delete_pgloader_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    state.pgloader_profiles.delete(id).await?;
    log_action(
        &actor_from_headers(&headers),
        "pgloader.profile.delete",
        "success",
        &format!("id={id}"),
    );
    Ok(Json(json!({ "status": true, "id": id })))
}

pub async fn run_pgloader_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let mut profile = state
        .pgloader_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("pgloader profile {id} not found")))?;
    require_pgloader_pair(&profile.source, &profile.target)?;
    let body = profile_run_payload(&profile);
    let job = pgloader::start_job(&state.config, &state.http, body).await?;
    if let Some(job_id) = job.get("id").and_then(|v| v.as_str()) {
        profile.last_job_id = Some(job_id.to_string());
        profile.updated_at = Utc::now();
        state.pgloader_profiles.upsert(&profile).await?;
    }
    log_action(
        &actor_from_headers(&headers),
        "pgloader.profile.run",
        "success",
        &format!("profile_id={id}"),
    );
    Ok(Json(job))
}

#[derive(serde::Deserialize)]
pub struct PgLoaderValidateRequest {
    pub connection: SchemaCompareConn,
}

pub async fn validate_pgloader_connection(
    Json(req): Json<PgLoaderValidateRequest>,
) -> Result<Json<ValidationResponse>, AppError> {
    let kind = req.connection.db_type.trim().to_ascii_lowercase();
    if pgloader::is_postgres(&kind) {
        if req.connection.host.trim().is_empty() || req.connection.database.trim().is_empty() {
            return Ok(Json(ValidationResponse {
                valid: false,
                message: "Postgres target needs host and database".into(),
                timestamp: Utc::now(),
            }));
        }
        return Ok(Json(ValidationResponse {
            valid: true,
            message: "Connection fields look valid.".into(),
            timestamp: Utc::now(),
        }));
    }
    if !pgloader::is_pgloader_source(&kind) {
        return Ok(Json(ValidationResponse {
            valid: false,
            message: "Source must be MySQL, SQL Server, SQLite, or CSV".into(),
            timestamp: Utc::now(),
        }));
    }
    if matches!(kind.as_str(), "sqlite" | "csv") {
        if req.connection.database.trim().is_empty() && req.connection.host.trim().is_empty() {
            return Ok(Json(ValidationResponse {
                valid: false,
                message: "SQLite/CSV needs a file path in database".into(),
                timestamp: Utc::now(),
            }));
        }
    } else if req.connection.host.trim().is_empty() || req.connection.database.trim().is_empty() {
        return Ok(Json(ValidationResponse {
            valid: false,
            message: "host and database are required".into(),
            timestamp: Utc::now(),
        }));
    }
    Ok(Json(ValidationResponse {
        valid: true,
        message: "Connection fields look valid; run load to verify credentials.".into(),
        timestamp: Utc::now(),
    }))
}
