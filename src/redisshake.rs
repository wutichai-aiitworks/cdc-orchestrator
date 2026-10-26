//! HTTP client for the cdc-redisshake sidecar.

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::{RedisShakeProfile, SchemaCompareConn};
use reqwest::header::{HeaderValue, AUTHORIZATION};
use reqwest::{Client, Method};
use serde_json::{json, Value};
use urlencoding::encode;

pub fn is_redis(db_type: &str) -> bool {
    matches!(
        db_type.trim().to_ascii_lowercase().as_str(),
        "redis" | "valkey" | "tair"
    )
}

pub fn require_redis_pair(
    source: &SchemaCompareConn,
    target: &SchemaCompareConn,
) -> Result<(), AppError> {
    if !is_redis(&source.db_type) {
        return Err(AppError::BadRequest(
            "RedisShake source must be Redis (or Valkey/Tair)".into(),
        ));
    }
    if !is_redis(&target.db_type) {
        return Err(AppError::BadRequest(
            "RedisShake target must be Redis (or Valkey/Tair)".into(),
        ));
    }
    Ok(())
}

pub fn profile_run_payload(profile: &RedisShakeProfile) -> Value {
    json!({
        "id": profile.id.to_string(),
        "name": profile.name,
        "source": profile.source,
        "target": profile.target,
        "options": profile.options,
    })
}

fn auth_headers(
    cfg: &AppConfig,
) -> Result<Vec<(reqwest::header::HeaderName, HeaderValue)>, AppError> {
    let mut headers = Vec::new();
    if let Some(token) = &cfg.redis_shake_token {
        let value = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|e| AppError::Internal(format!("redisshake auth header: {e}")))?;
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
    let base = cfg.redis_shake_url.as_ref().ok_or_else(|| {
        AppError::BadRequest("RedisShake sidecar is not configured (ORCH_REDIS_SHAKE_URL)".into())
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
        .map_err(|e| AppError::Upstream(format!("redisshake sidecar request failed: {e}")))?;
    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("redisshake sidecar read body: {e}")))?;
    let parsed: Value = serde_json::from_str(&text).unwrap_or(json!({ "raw": text }));
    if status.is_success() {
        Ok(parsed)
    } else {
        let msg = parsed
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or(&text);
        Err(AppError::Upstream(format!(
            "redisshake sidecar {}: {msg}",
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
