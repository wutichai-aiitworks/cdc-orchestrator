//! HTTP client for the cdc-pgcopydb sidecar.

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::{PgCopyProfile, SchemaCompareConn};
use reqwest::header::{HeaderValue, AUTHORIZATION};
use reqwest::{Client, Method};
use serde_json::{json, Value};
use urlencoding::encode;

pub fn is_postgres(db_type: &str) -> bool {
    matches!(
        db_type.trim().to_ascii_lowercase().as_str(),
        "postgres" | "postgresql" | "pg"
    )
}

pub fn require_postgres_pair(
    source: &SchemaCompareConn,
    target: &SchemaCompareConn,
) -> Result<(), AppError> {
    if !is_postgres(&source.db_type) {
        return Err(AppError::BadRequest(
            "pgcopydb source must be PostgreSQL".into(),
        ));
    }
    if !is_postgres(&target.db_type) {
        return Err(AppError::BadRequest(
            "pgcopydb target must be PostgreSQL".into(),
        ));
    }
    Ok(())
}

pub fn profile_run_payload(profile: &PgCopyProfile) -> Value {
    json!({
        "id": profile.id.to_string(),
        "name": profile.name,
        "source": profile.source,
        "target": profile.target,
        "include_tables": profile.include_tables,
        "options": profile.options,
    })
}

fn auth_headers(
    cfg: &AppConfig,
) -> Result<Vec<(reqwest::header::HeaderName, HeaderValue)>, AppError> {
    let mut headers = Vec::new();
    if let Some(token) = &cfg.pgcopy_token {
        let value = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|e| AppError::Internal(format!("pgcopy auth header: {e}")))?;
        headers.push((AUTHORIZATION, value));
    }
    Ok(headers)
}

pub async fn sidecar_request(
    cfg: &AppConfig,
    client: &Client,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, AppError> {
    let base = cfg.pgcopy_url.as_ref().ok_or_else(|| {
        AppError::BadRequest("pgcopydb sidecar is not configured (ORCH_PGCOPY_URL)".into())
    })?;
    let url = format!("{}{}", base.trim_end_matches('/'), path);
    let mut req = client.request(method, &url);
    for (name, value) in auth_headers(cfg)? {
        req = req.header(name, value);
    }
    if let Some(body) = body {
        req = req.json(&body);
    }
    let res = req
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("pgcopy sidecar request failed: {e}")))?;
    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("pgcopy sidecar read body: {e}")))?;
    let parsed: Value = serde_json::from_str(&text).unwrap_or(json!({ "raw": text }));
    if status.is_success() {
        Ok(parsed)
    } else {
        let msg = parsed
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or(&text);
        Err(AppError::Upstream(format!(
            "pgcopy sidecar {}: {msg}",
            status.as_u16()
        )))
    }
}

pub async fn capability(cfg: &AppConfig, client: &Client) -> Result<Value, AppError> {
    sidecar_request(cfg, client, Method::GET, "/v1/capability", None).await
}

pub async fn discover(cfg: &AppConfig, client: &Client) -> Result<Value, AppError> {
    sidecar_request(cfg, client, Method::GET, "/v1/discover", None).await
}

pub async fn list_jobs(cfg: &AppConfig, client: &Client) -> Result<Value, AppError> {
    sidecar_request(cfg, client, Method::GET, "/v1/jobs", None).await
}

pub async fn get_job(cfg: &AppConfig, client: &Client, job_id: &str) -> Result<Value, AppError> {
    let path = format!("/v1/jobs/{}", encode(job_id));
    sidecar_request(cfg, client, Method::GET, &path, None).await
}

pub async fn job_logs(cfg: &AppConfig, client: &Client, job_id: &str) -> Result<Value, AppError> {
    let path = format!("/v1/jobs/{}/logs", encode(job_id));
    sidecar_request(cfg, client, Method::GET, &path, None).await
}

pub async fn start_job(cfg: &AppConfig, client: &Client, body: Value) -> Result<Value, AppError> {
    sidecar_request(cfg, client, Method::POST, "/v1/jobs", Some(body)).await
}

pub async fn stop_job(cfg: &AppConfig, client: &Client, job_id: &str) -> Result<Value, AppError> {
    let path = format!("/v1/jobs/{}/stop", encode(job_id));
    sidecar_request(cfg, client, Method::POST, &path, None).await
}
