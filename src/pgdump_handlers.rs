use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit::log_action;
use crate::error::AppError;
use crate::models::{
    PgDumpDestination, PgDumpProfile, PgDumpProfileRequest, PgDumpProfilesResponse,
    SchemaCompareConn, ValidationResponse,
};
use crate::pgdump::{
    self, fill_send_secrets, keep_destination_secrets, profile_run_payload, validate_destinations,
    validate_profile, MODE_DUMP_RESTORE,
};
use crate::AppState;

use crate::saved_secret::{keep_password, REDACTED};

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

fn check_request(req: &PgDumpProfileRequest) -> Result<(), AppError> {
    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("Profile name is required".into()));
    }
    validate_profile(
        req.mode.trim(),
        &req.source,
        req.target.as_ref(),
        req.options.format.as_deref(),
    )?;
    validate_destinations(&req.destinations)
}

pub async fn pgdump_capability(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    pgdump::capability(&state.config, &state.http)
        .await
        .map(Json)
}

pub async fn list_pgdump_jobs(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    pgdump::list_jobs(&state.config, &state.http)
        .await
        .map(Json)
}

pub async fn get_pgdump_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    pgdump::get_job(&state.config, &state.http, &job_id)
        .await
        .map(Json)
}

pub async fn pgdump_job_logs(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    pgdump::job_logs(&state.config, &state.http, &job_id)
        .await
        .map(Json)
}

pub async fn stop_pgdump_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let out = pgdump::stop_job(&state.config, &state.http, &job_id).await?;
    log_action(
        &actor_from_headers(&headers),
        "pgdump.job.stop",
        "success",
        &format!("job_id={job_id}"),
    );
    Ok(Json(out))
}

/// Remove a finished job and its dump file from the sidecar volume.
pub async fn delete_pgdump_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let out = pgdump::delete_job(&state.config, &state.http, &job_id).await?;
    log_action(
        &actor_from_headers(&headers),
        "pgdump.job.delete",
        "success",
        &format!("job_id={job_id}"),
    );
    Ok(Json(out))
}

/// Stream the dump file from the sidecar without buffering it in memory.
pub async fn download_pgdump_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Response, AppError> {
    let upstream = pgdump::open_file(&state.config, &job_id).await?;
    let mut builder = Response::builder().status(StatusCode::OK);
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_DISPOSITION,
    ] {
        if let Some(v) = upstream.headers().get(&name) {
            builder = builder.header(&name, v.clone());
        }
    }
    if let Some(v) = upstream.headers().get("x-dump-sha256") {
        builder = builder.header("x-dump-sha256", v.clone());
    }
    builder = builder.header(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    log_action(
        &actor_from_headers(&headers),
        "pgdump.job.download",
        "success",
        &format!("job_id={job_id}"),
    );
    builder
        .body(Body::from_stream(upstream.bytes_stream()))
        .map_err(|e| AppError::Internal(format!("pgdump download response: {e}")))
}

#[derive(serde::Deserialize)]
pub struct PgDumpSendRequest {
    pub destination: PgDumpDestination,
    /// Profile whose saved SFTP server fills a redacted `********` secret (same login only).
    #[serde(default)]
    pub profile_id: Option<Uuid>,
}

/// Send a finished job's dump file to one SFTP server entered ad hoc. Nothing is saved; the
/// credential-free result is appended to the job's uploads by the sidecar.
pub async fn send_pgdump_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    Json(mut req): Json<PgDumpSendRequest>,
) -> Result<Json<Value>, AppError> {
    let saved = match req.profile_id {
        Some(id) => load(&state, id).await?.destinations,
        None => Vec::new(),
    };
    fill_send_secrets(&mut req.destination, &saved)?;
    validate_destinations(std::slice::from_ref(&req.destination))?;
    let d = &req.destination;
    let target = format!(
        "{}@{}:{}:{}",
        d.username.trim(),
        d.host.trim(),
        d.port.trim(),
        d.remote_dir.trim()
    );
    let body = serde_json::to_value(&req.destination)
        .map_err(|e| AppError::Internal(format!("pgdump send body: {e}")))?;
    let out = pgdump::send_file(
        &state.config,
        &state.http,
        &job_id,
        json!({ "destination": body }),
    )
    .await;
    log_action(
        &actor_from_headers(&headers),
        "pgdump.job.send",
        if out.is_ok() { "success" } else { "failure" },
        &format!("job_id={job_id} sftp={target}"),
    );
    out.map(Json)
}

pub async fn list_pgdump_profiles(
    State(state): State<AppState>,
) -> Result<Json<PgDumpProfilesResponse>, AppError> {
    let mut profiles = state.pgdump_profiles.list().await?;
    for profile in &mut profiles {
        profile.redact_secrets();
    }
    Ok(Json(PgDumpProfilesResponse { profiles }))
}

pub async fn create_pgdump_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PgDumpProfileRequest>,
) -> Result<Json<PgDumpProfile>, AppError> {
    check_request(&req)?;
    let now = Utc::now();
    let mode = req.mode.trim().to_string();
    let mut profile = PgDumpProfile {
        id: Uuid::new_v4(),
        name: req.name.trim().to_string(),
        target: if mode == MODE_DUMP_RESTORE {
            req.target
        } else {
            None
        },
        mode,
        source: req.source,
        include_tables: req.include_tables,
        options: req.options,
        destinations: req.destinations,
        last_job_id: None,
        created_at: now,
        updated_at: now,
    };
    state.pgdump_profiles.upsert(&profile).await?;
    log_action(
        &actor_from_headers(&headers),
        "pgdump.profile.create",
        "success",
        &format!(
            "id={} name={} mode={}",
            profile.id, profile.name, profile.mode
        ),
    );
    profile.redact_secrets();
    Ok(Json(profile))
}

pub async fn get_pgdump_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PgDumpProfile>, AppError> {
    let mut profile = load(&state, id).await?;
    profile.redact_secrets();
    Ok(Json(profile))
}

async fn load(state: &AppState, id: Uuid) -> Result<PgDumpProfile, AppError> {
    state
        .pgdump_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("pgdump profile {id} not found")))
}

pub async fn update_pgdump_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<PgDumpProfileRequest>,
) -> Result<Json<PgDumpProfile>, AppError> {
    check_request(&req)?;
    let existing = load(&state, id).await?;
    let mode = req.mode.trim().to_string();
    let mut source = req.source;
    keep_password("source", &mut source, Some(&existing.source))?;
    let target = if mode == MODE_DUMP_RESTORE {
        match req.target {
            Some(mut t) => {
                keep_password("target", &mut t, existing.target.as_ref())?;
                Some(t)
            }
            None => None,
        }
    } else {
        None
    };
    let mut destinations = req.destinations;
    keep_destination_secrets(&mut destinations, &existing.destinations);
    validate_destinations(&destinations)?;
    let mut profile = PgDumpProfile {
        id,
        name: req.name.trim().to_string(),
        mode,
        source,
        target,
        include_tables: req.include_tables,
        options: req.options,
        destinations,
        last_job_id: existing.last_job_id,
        created_at: existing.created_at,
        updated_at: Utc::now(),
    };
    state.pgdump_profiles.upsert(&profile).await?;
    log_action(
        &actor_from_headers(&headers),
        "pgdump.profile.update",
        "success",
        &format!("id={}", profile.id),
    );
    profile.redact_secrets();
    Ok(Json(profile))
}

pub async fn delete_pgdump_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    state.pgdump_profiles.delete(id).await?;
    log_action(
        &actor_from_headers(&headers),
        "pgdump.profile.delete",
        "success",
        &format!("id={id}"),
    );
    Ok(Json(json!({ "status": true, "id": id })))
}

pub async fn run_pgdump_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let mut profile = load(&state, id).await?;
    validate_profile(
        &profile.mode,
        &profile.source,
        profile.target.as_ref(),
        profile.options.format.as_deref(),
    )?;
    validate_destinations(&profile.destinations)?;
    let job = pgdump::start_job(&state.config, &state.http, profile_run_payload(&profile)).await?;
    if let Some(job_id) = job.get("id").and_then(|v| v.as_str()) {
        profile.last_job_id = Some(job_id.to_string());
        profile.updated_at = Utc::now();
        state.pgdump_profiles.upsert(&profile).await?;
    }
    log_action(
        &actor_from_headers(&headers),
        "pgdump.profile.run",
        "success",
        &format!(
            "profile_id={id} mode={} sftp_destinations={}",
            profile.mode,
            profile.destinations.len()
        ),
    );
    Ok(Json(job))
}

#[derive(serde::Deserialize)]
pub struct PgDumpTestConnectionRequest {
    pub connection: SchemaCompareConn,
    #[serde(default)]
    pub sslmode: Option<String>,
    /// Saved profile whose stored password fills a redacted `********` (same login only).
    #[serde(default)]
    pub profile_id: Option<Uuid>,
    /// `source` (default) or `target`.
    #[serde(default)]
    pub side: Option<String>,
}

/// Connect from the pgdump sidecar — where pg_dump runs — and describe the database
/// (server version, client compatibility, schemas with table counts and sizes).
/// A failed connection is a normal answer (`ok: false`, `error`, `hint`).
pub async fn test_pgdump_connection(
    State(state): State<AppState>,
    Json(req): Json<PgDumpTestConnectionRequest>,
) -> Result<Json<Value>, AppError> {
    let side = req.side.as_deref().unwrap_or("source");
    if side != "source" && side != "target" {
        return Err(AppError::BadRequest("side must be source or target".into()));
    }
    let mut conn = req.connection;
    if conn.password == REDACTED {
        let stored = match req.profile_id {
            Some(id) => {
                let profile = load(&state, id).await?;
                if side == "source" {
                    Some(profile.source)
                } else {
                    profile.target
                }
            }
            None => None,
        };
        keep_password(side, &mut conn, stored.as_ref())?;
    }
    validate_profile(pgdump::MODE_DUMP, &conn, None, None)?;
    let body =
        json!({ "connection": conn, "sslmode": req.sslmode.filter(|s| !s.trim().is_empty()) });
    Ok(Json(
        pgdump::test_connection(&state.config, &state.http, body).await?,
    ))
}

#[derive(serde::Deserialize)]
pub struct PgDumpValidateRequest {
    pub connection: SchemaCompareConn,
}

pub async fn validate_pgdump_connection(
    Json(req): Json<PgDumpValidateRequest>,
) -> Result<Json<ValidationResponse>, AppError> {
    let (valid, message) = match validate_profile(pgdump::MODE_DUMP, &req.connection, None, None) {
        Ok(()) => (
            true,
            "Connection fields look valid; run a dump to verify reachability and privileges."
                .to_string(),
        ),
        Err(e) => (false, e.to_string()),
    };
    Ok(Json(ValidationResponse {
        valid,
        message,
        timestamp: Utc::now(),
    }))
}
