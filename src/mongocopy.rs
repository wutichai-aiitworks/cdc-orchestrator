//! HTTP client for the cdc-mongocopy sidecar.

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::{MongoCopyProfile, SchemaCompareConn};
use reqwest::header::{HeaderValue, AUTHORIZATION};
use reqwest::{Client, Method};
use serde_json::{json, Value};
use urlencoding::encode;

pub fn is_mongo(db_type: &str) -> bool {
    matches!(
        db_type.trim().to_ascii_lowercase().as_str(),
        "mongo" | "mongodb"
    )
}

pub fn require_mongo_pair(
    source: &SchemaCompareConn,
    target: &SchemaCompareConn,
) -> Result<(), AppError> {
    if !is_mongo(&source.db_type) {
        return Err(AppError::BadRequest(
            "mongocopy source must be MongoDB".into(),
        ));
    }
    if !is_mongo(&target.db_type) {
        return Err(AppError::BadRequest(
            "mongocopy target must be MongoDB".into(),
        ));
    }
    if source.host.trim().is_empty() || target.host.trim().is_empty() {
        return Err(AppError::BadRequest(
            "host is required on source and target".into(),
        ));
    }
    Ok(())
}

pub fn profile_run_payload(profile: &MongoCopyProfile) -> Value {
    json!({
        "id": profile.id.to_string(),
        "name": profile.name,
        "source": profile.source,
        "target": profile.target,
        "include_collections": profile.include_collections,
        "options": profile.options,
    })
}

fn auth_headers(
    cfg: &AppConfig,
) -> Result<Vec<(reqwest::header::HeaderName, HeaderValue)>, AppError> {
    let mut headers = Vec::new();
    if let Some(token) = &cfg.mongocopy_token {
        let value = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|e| AppError::Internal(format!("mongocopy auth header: {e}")))?;
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
    let base = cfg.mongocopy_url.as_ref().ok_or_else(|| {
        AppError::BadRequest("mongocopy sidecar is not configured (ORCH_MONGOCOPY_URL)".into())
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
        .map_err(|e| AppError::Upstream(format!("mongocopy sidecar request failed: {e}")))?;
    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("mongocopy sidecar read body: {e}")))?;
    let parsed: Value = serde_json::from_str(&text).unwrap_or(json!({ "raw": text }));
    if status.is_success() {
        Ok(parsed)
    } else {
        let msg = parsed
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or(&text);
        Err(AppError::Upstream(format!(
            "mongocopy sidecar {}: {msg}",
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
    sidecar_request(
        cfg,
        client,
        Method::GET,
        &format!("/v1/jobs/{}", encode(job_id)),
        None,
    )
    .await
}

pub async fn job_logs(cfg: &AppConfig, client: &Client, job_id: &str) -> Result<Value, AppError> {
    sidecar_request(
        cfg,
        client,
        Method::GET,
        &format!("/v1/jobs/{}/logs", encode(job_id)),
        None,
    )
    .await
}

pub async fn start_job(cfg: &AppConfig, client: &Client, body: Value) -> Result<Value, AppError> {
    sidecar_request(cfg, client, Method::POST, "/v1/jobs", Some(body)).await
}

pub async fn stop_job(cfg: &AppConfig, client: &Client, job_id: &str) -> Result<Value, AppError> {
    sidecar_request(
        cfg,
        client,
        Method::POST,
        &format!("/v1/jobs/{}/stop", encode(job_id)),
        None,
    )
    .await
}
