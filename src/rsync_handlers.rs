use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit::log_action;
use crate::error::AppError;
use crate::models::{
    RsyncEndpoint, RsyncOptions, RsyncProfile, RsyncProfileRequest, RsyncProfilesResponse,
    ValidationResponse,
};
use crate::rsync::{self, keep_endpoint_secrets, profile_run_payload, validate_profile};
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

fn check_request(req: &RsyncProfileRequest) -> Result<(), AppError> {
    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("Profile name is required".into()));
    }
    validate_profile(&req.source, &req.target, &req.options)
}

async fn load(state: &AppState, id: Uuid) -> Result<RsyncProfile, AppError> {
    state
        .rsync_profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("rsync profile {id} not found")))
}

pub async fn rsync_capability(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    rsync::capability(&state.config, &state.http)
        .await
        .map(Json)
}

pub async fn list_rsync_jobs(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    rsync::list_jobs(&state.config, &state.http).await.map(Json)
}

pub async fn get_rsync_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    rsync::get_job(&state.config, &state.http, &job_id)
        .await
        .map(Json)
}

pub async fn rsync_job_logs(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    rsync::job_logs(&state.config, &state.http, &job_id)
        .await
        .map(Json)
}

pub async fn stop_rsync_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let out = rsync::stop_job(&state.config, &state.http, &job_id).await?;
    log_action(
        &actor_from_headers(&headers),
        "rsync.job.stop",
        "success",
        &format!("job_id={job_id}"),
    );
    Ok(Json(out))
}

pub async fn delete_rsync_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let out = rsync::delete_job(&state.config, &state.http, &job_id).await?;
    log_action(
        &actor_from_headers(&headers),
        "rsync.job.delete",
        "success",
        &format!("job_id={job_id}"),
    );
    Ok(Json(out))
}

pub async fn list_rsync_profiles(
    State(state): State<AppState>,
) -> Result<Json<RsyncProfilesResponse>, AppError> {
    let mut profiles = state.rsync_profiles.list().await?;
    for p in &mut profiles {
        p.redact_secrets();
    }
    Ok(Json(RsyncProfilesResponse { profiles }))
}

pub async fn create_rsync_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut req): Json<RsyncProfileRequest>,
) -> Result<Json<RsyncProfile>, AppError> {
    // A copied profile may carry redacted markers: they never become literal secrets.
    keep_endpoint_secrets("source", &mut req.source, None)?;
    keep_endpoint_secrets("target", &mut req.target, None)?;
    check_request(&req)?;
    let now = Utc::now();
    let mut profile = RsyncProfile {
        id: Uuid::new_v4(),
        name: req.name.trim().to_string(),
        source: req.source,
        target: req.target,
        options: req.options,
        last_job_id: None,
        created_at: now,
        updated_at: now,
    };
    state.rsync_profiles.upsert(&profile).await?;
    log_action(
        &actor_from_headers(&headers),
        "rsync.profile.create",
        "success",
        &format!(
            "id={} name={} delete={}",
            profile.id, profile.name, profile.options.delete
        ),
    );
    profile.redact_secrets();
    Ok(Json(profile))
}

pub async fn get_rsync_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<RsyncProfile>, AppError> {
    let mut profile = load(&state, id).await?;
    profile.redact_secrets();
    Ok(Json(profile))
}

pub async fn update_rsync_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(mut req): Json<RsyncProfileRequest>,
) -> Result<Json<RsyncProfile>, AppError> {
    let existing = load(&state, id).await?;
    keep_endpoint_secrets("source", &mut req.source, Some(&existing.source))?;
    keep_endpoint_secrets("target", &mut req.target, Some(&existing.target))?;
    check_request(&req)?;
    let mut profile = RsyncProfile {
        id,
        name: req.name.trim().to_string(),
        source: req.source,
        target: req.target,
        options: req.options,
        last_job_id: existing.last_job_id,
        created_at: existing.created_at,
        updated_at: Utc::now(),
    };
    state.rsync_profiles.upsert(&profile).await?;
    log_action(
        &actor_from_headers(&headers),
        "rsync.profile.update",
        "success",
        &format!("id={} delete={}", profile.id, profile.options.delete),
    );
    profile.redact_secrets();
    Ok(Json(profile))
}

/// Deletes the profile and, best effort, its relay staging copy on the sidecar.
pub async fn delete_rsync_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    state.rsync_profiles.delete(id).await?;
    let staging = match rsync::delete_staging(&state.config, &state.http, &id.to_string()).await {
        Ok(_) => "deleted",
        Err(AppError::NotFound(_)) => "none",
        Err(e) => {
            tracing::warn!(error = %e, profile_id = %id, "rsync staging cleanup failed");
            "kept"
        }
    };
    log_action(
        &actor_from_headers(&headers),
        "rsync.profile.delete",
        "success",
        &format!("id={id} staging={staging}"),
    );
    Ok(Json(
        json!({ "status": true, "id": id, "staging": staging }),
    ))
}

#[derive(Deserialize, Default)]
pub struct RunQuery {
    /// `true` = preview (--dry-run --itemize-changes) regardless of the profile setting.
    #[serde(default)]
    pub dry_run: Option<bool>,
}

pub async fn run_rsync_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<RunQuery>,
) -> Result<Json<Value>, AppError> {
    let mut profile = load(&state, id).await?;
    validate_profile(&profile.source, &profile.target, &profile.options)?;
    let job = rsync::start_job(
        &state.config,
        &state.http,
        profile_run_payload(&profile, q.dry_run),
    )
    .await?;
    if let Some(job_id) = job.get("id").and_then(|v| v.as_str()) {
        profile.last_job_id = Some(job_id.to_string());
        profile.updated_at = Utc::now();
        state.rsync_profiles.upsert(&profile).await?;
    }
    let preview = q.dry_run.unwrap_or(profile.options.dry_run);
    log_action(
        &actor_from_headers(&headers),
        if preview {
            "rsync.profile.preview"
        } else {
            "rsync.profile.run"
        },
        "success",
        &format!(
            "profile_id={id} transport={} delete={}",
            profile.options.transport.as_deref().unwrap_or("auto"),
            profile.options.delete
        ),
    );
    Ok(Json(job))
}

/// Fill redacted secrets from the saved profile (same login only) for a test call.
async fn with_saved(
    state: &AppState,
    profile_id: Option<Uuid>,
    side: &str,
    endpoint: &mut RsyncEndpoint,
) -> Result<(), AppError> {
    let stored = match profile_id {
        Some(id) => {
            let p = load(state, id).await?;
            Some(if side == "target" { p.target } else { p.source })
        }
        None => None,
    };
    keep_endpoint_secrets(side, endpoint, stored.as_ref())
}

#[derive(Deserialize)]
pub struct RsyncTestConnectionRequest {
    pub endpoint: RsyncEndpoint,
    /// `source` (default) or `target`.
    #[serde(default)]
    pub side: Option<String>,
    #[serde(default)]
    pub profile_id: Option<Uuid>,
}

/// Log in from the sidecar and describe the host: OS, rsync binary/flavour, path state, free space,
/// host key. A failed login is a normal answer (`ok: false`, `error`, `hint`).
pub async fn test_rsync_connection(
    State(state): State<AppState>,
    Json(req): Json<RsyncTestConnectionRequest>,
) -> Result<Json<Value>, AppError> {
    let side = req.side.as_deref().unwrap_or("source");
    if side != "source" && side != "target" {
        return Err(AppError::BadRequest("side must be source or target".into()));
    }
    let mut endpoint = req.endpoint;
    with_saved(&state, req.profile_id, side, &mut endpoint).await?;
    rsync::check_endpoint(side, &endpoint)?;
    let body = json!({ "endpoint": endpoint, "side": side });
    Ok(Json(
        rsync::test_connection(&state.config, &state.http, body).await?,
    ))
}

#[derive(Deserialize)]
pub struct RsyncTestRouteRequest {
    pub source: RsyncEndpoint,
    pub target: RsyncEndpoint,
    #[serde(default)]
    pub profile_id: Option<Uuid>,
}

/// Would a run go direct? Logs in to the source and from there to the target with the forwarded key.
pub async fn test_rsync_route(
    State(state): State<AppState>,
    Json(req): Json<RsyncTestRouteRequest>,
) -> Result<Json<Value>, AppError> {
    let (mut source, mut target) = (req.source, req.target);
    with_saved(&state, req.profile_id, "source", &mut source).await?;
    with_saved(&state, req.profile_id, "target", &mut target).await?;
    validate_profile(&source, &target, &RsyncOptions::default())?;
    let body = json!({ "source": source, "target": target });
    Ok(Json(
        rsync::test_route(&state.config, &state.http, body).await?,
    ))
}

pub async fn list_rsync_staging(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    rsync::list_staging(&state.config, &state.http)
        .await
        .map(Json)
}

pub async fn delete_rsync_staging(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let out = rsync::delete_staging(&state.config, &state.http, &id).await?;
    log_action(
        &actor_from_headers(&headers),
        "rsync.staging.delete",
        "success",
        &format!("id={id}"),
    );
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct RsyncValidateRequest {
    pub source: RsyncEndpoint,
    pub target: RsyncEndpoint,
    #[serde(default)]
    pub options: RsyncOptions,
}

pub async fn validate_rsync_profile(
    Json(req): Json<RsyncValidateRequest>,
) -> Result<Json<ValidationResponse>, AppError> {
    let (valid, message) = match validate_profile(&req.source, &req.target, &req.options) {
        Ok(()) => (
            true,
            "Fields look valid; test the connections to verify logins and rsync.".to_string(),
        ),
        Err(e) => (false, e.to_string()),
    };
    Ok(Json(ValidationResponse {
        valid,
        message,
        timestamp: Utc::now(),
    }))
}
