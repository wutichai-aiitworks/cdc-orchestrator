use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit::log_action;
use crate::error::AppError;
use crate::models::{
    PgCopyProfile, PgCopyProfileRequest, PgCopyProfilesResponse, SchemaCompareConn,
    ValidationResponse,
};
use crate::pgcopy::{self, profile_run_payload, require_postgres_pair};
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

pub async fn pgcopy_capability(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    pgcopy::capability(&state.config, &state.http)
        .await
        .map(Json)
}

pub async fn list_pgcopy_jobs(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    pgcopy::list_jobs(&state.config, &state.http)
        .await
        .map(Json)
}

pub async fn get_pgcopy_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    pgcopy::get_job(&state.config, &state.http, &job_id)
        .await
        .map(Json)
}

pub async fn pgcopy_job_logs(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    pgcopy::job_logs(&state.config, &state.http, &job_id)
        .await
        .map(Json)
}

pub async fn stop_pgcopy_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let out = pgcopy::stop_job(&state.config, &state.http, &job_id).await?;
    tool_events::stop_requested(
        &state,
        "pgcopy",
        &actor_from_headers(&headers),
        &job_id,
        &out,
    )
    .await;
    log_action(
        &actor_from_headers(&headers),
        "pgcopy.job.stop",
        "success",
        &format!("job_id={job_id}"),
    );
    Ok(Json(out))
}

pub async fn list_pgcopy_profiles(
    State(state): State<AppState>,
) -> Result<Json<PgCopyProfilesResponse>, AppError> {
    let mut profiles = state.pgcopy_profiles.list().await?;
    for profile in &mut profiles {
        profile.redact_secrets();
    }
    Ok(Json(PgCopyProfilesResponse { profiles }))
}

pub async fn create_pgcopy_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PgCopyProfileRequest>,
) -> Result<Json<PgCopyProfile>, AppError> {
    require_postgres_pair(&req.source, &req.target)?;
    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("Profile name is required".into()));
    }
    let now = Utc::now();
    let profile = PgCopyProfile {
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
    state.pgcopy_profiles.upsert(&profile).await?;
    tool_events::profile_created(
        &state,
        "pgcopy",
        &actor_from_headers(&headers),
        profile.id,
        &profile.name,
        &profile,
    )
    .await;
    log_action(
        &actor_from_headers(&headers),
        "pgcopy.profile.create",
        "success",
        &format!("id={} name={}", profile.id, profile.name),
    );
    Ok(Json(profile))
}

pub async fn get_pgcopy_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PgCopyProfile>, AppError> {
    let mut profile = state
        .pgcopy_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("pgcopy profile {id} not found")))?;
    profile.redact_secrets();
    Ok(Json(profile))
}

pub async fn update_pgcopy_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<PgCopyProfileRequest>,
) -> Result<Json<PgCopyProfile>, AppError> {
    require_postgres_pair(&req.source, &req.target)?;
    let existing = state
        .pgcopy_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("pgcopy profile {id} not found")))?;

    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("Profile name is required".into()));
    }

    let mut source = req.source;
    let mut target = req.target;
    // Stored passwords stay with their login; an edited host must be given the password again.
    keep_password("source", &mut source, Some(&existing.source))?;
    keep_password("target", &mut target, Some(&existing.target))?;

    let profile = PgCopyProfile {
        id,
        name: req.name.trim().to_string(),
        source,
        target,
        include_tables: req.include_tables,
        options: req.options,
        last_job_id: existing.last_job_id.clone(),
        created_at: existing.created_at,
        updated_at: Utc::now(),
    };
    state.pgcopy_profiles.upsert(&profile).await?;
    tool_events::profile_updated(
        &state,
        "pgcopy",
        &actor_from_headers(&headers),
        id,
        &profile.name,
        &existing,
        &profile,
    )
    .await;
    log_action(
        &actor_from_headers(&headers),
        "pgcopy.profile.update",
        "success",
        &format!("id={}", profile.id),
    );
    Ok(Json(profile))
}

pub async fn delete_pgcopy_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let deleted_name = state
        .pgcopy_profiles
        .get(id)
        .await
        .ok()
        .flatten()
        .map(|p| p.name);
    state.pgcopy_profiles.delete(id).await?;
    tool_events::profile_deleted(
        &state,
        "pgcopy",
        &actor_from_headers(&headers),
        id,
        deleted_name.as_deref(),
    )
    .await;
    log_action(
        &actor_from_headers(&headers),
        "pgcopy.profile.delete",
        "success",
        &format!("id={id}"),
    );
    Ok(Json(json!({ "status": true, "id": id })))
}

pub async fn run_pgcopy_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let mut profile = state
        .pgcopy_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("pgcopy profile {id} not found")))?;
    require_postgres_pair(&profile.source, &profile.target)?;

    let body = profile_run_payload(&profile);
    // WireGuard (ADR 0010): connections with a tunnel go to a gateway forward.
    let mut run_body = body;
    let started = match crate::wireguard::route_payload(&state, &mut run_body).await {
        Ok(_) => pgcopy::start_job(&state.config, &state.http, run_body).await,
        Err(e) => Err(e),
    };
    tool_events::run_requested(
        &state,
        "pgcopy",
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
        state.pgcopy_profiles.upsert(&profile).await?;
    }
    log_action(
        &actor_from_headers(&headers),
        "pgcopy.profile.run",
        "success",
        &format!("profile_id={id}"),
    );
    Ok(Json(job))
}

#[derive(serde::Deserialize)]
pub struct PgCopyValidateRequest {
    pub connection: SchemaCompareConn,
}

pub async fn validate_pgcopy_connection(
    Json(req): Json<PgCopyValidateRequest>,
) -> Result<Json<ValidationResponse>, AppError> {
    if !pgcopy::is_postgres(&req.connection.db_type) {
        return Ok(Json(ValidationResponse {
            valid: false,
            message: "pgcopydb supports PostgreSQL only".into(),
            timestamp: Utc::now(),
        }));
    }
    if req.connection.host.trim().is_empty() || req.connection.database.trim().is_empty() {
        return Ok(Json(ValidationResponse {
            valid: false,
            message: "host and database are required".into(),
            timestamp: Utc::now(),
        }));
    }
    Ok(Json(ValidationResponse {
        valid: true,
        message: "Connection fields look valid; run clone to verify replication privileges.".into(),
        timestamp: Utc::now(),
    }))
}

/// Connect from the pgcopy sidecar (where the tool runs) and describe the server. A failed
/// connection is a normal answer (`ok: false`, `error`, `hint`).
pub async fn test_pgcopy_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<crate::tool_sidecar::TestConnectionRequest>,
) -> Result<Json<Value>, AppError> {
    let side = req.side.clone().unwrap_or_else(|| "source".into());
    let mut tested: Option<(Uuid, String)> = None;
    let stored = match req.profile_id {
        Some(id) => Some(
            state
                .pgcopy_profiles
                .get(id)
                .await?
                .map(|p| {
                    tested = Some((id, p.name));
                    (p.source, p.target)
                })
                .ok_or_else(|| AppError::NotFound(format!("pgcopy profile {id} not found")))?,
        ),
        None => None,
    };
    let body = crate::tool_sidecar::test_body(req, stored)?;
    let mut body = body;
    let via = crate::wireguard::route_payload(&state, &mut body).await?;
    let mut out = pgcopy::test_connection(&state.config, &state.http, body).await?;
    if !via.is_empty() {
        out["via_tunnel"] = json!(via.join(", "));
    }
    tool_events::connection_tested(
        &state,
        "pgcopy",
        &actor_from_headers(&headers),
        tested,
        &side,
        &out,
    )
    .await;
    Ok(Json(out))
}
