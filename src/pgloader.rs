//! HTTP client for the cdc-pgloader sidecar.

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::{PgLoaderProfile, SchemaCompareConn};
use reqwest::header::{HeaderValue, AUTHORIZATION};
use reqwest::{Client, Method};
use serde_json::{json, Value};
use urlencoding::encode;

const SOURCE_TYPES: &[&str] = &["mysql", "mariadb", "mssql", "sqlserver", "sqlite", "csv"];

pub fn is_pgloader_source(db_type: &str) -> bool {
    SOURCE_TYPES.contains(&db_type.trim().to_ascii_lowercase().as_str())
}

pub fn is_postgres(db_type: &str) -> bool {
    matches!(
        db_type.trim().to_ascii_lowercase().as_str(),
        "postgres" | "postgresql" | "pg"
    )
}

pub fn require_pgloader_pair(
    source: &SchemaCompareConn,
    target: &SchemaCompareConn,
) -> Result<(), AppError> {
    if !is_pgloader_source(&source.db_type) {
        return Err(AppError::BadRequest(
            "pgloader source must be MySQL, SQL Server, SQLite, or CSV".into(),
        ));
    }
    if !is_postgres(&target.db_type) {
        return Err(AppError::BadRequest(
            "pgloader target must be PostgreSQL".into(),
        ));
    }
    if matches!(
        source.db_type.trim().to_ascii_lowercase().as_str(),
        "sqlite" | "csv"
    ) && source.database.trim().is_empty()
        && source.host.trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "SQLite/CSV source needs a file path in database".into(),
        ));
    }
    Ok(())
}

pub fn profile_run_payload(profile: &PgLoaderProfile) -> Value {
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
    if let Some(token) = &cfg.pgloader_token {
        let value = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|e| AppError::Internal(format!("pgloader auth header: {e}")))?;
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
    let base = cfg.pgloader_url.as_ref().ok_or_else(|| {
        AppError::BadRequest("pgloader sidecar is not configured (ORCH_PGLOADER_URL)".into())
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
        .map_err(|e| AppError::Upstream(format!("pgloader sidecar request failed: {e}")))?;
    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("pgloader sidecar read body: {e}")))?;
    let parsed: Value = serde_json::from_str(&text).unwrap_or(json!({ "raw": text }));
    if status.is_success() {
        Ok(parsed)
    } else {
        let msg = parsed
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or(&text);
        Err(AppError::Upstream(format!(
            "pgloader sidecar {}: {msg}",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(db_type: &str, database: &str) -> SchemaCompareConn {
        SchemaCompareConn {
            db_type: db_type.into(),
            host: "src".into(),
            port: "3306".into(),
            database: database.into(),
            username: "u".into(),
            password: "p".into(),
            schema: String::new(),
        }
    }

    #[test]
    fn accepts_mysql_to_postgres() {
        let target = SchemaCompareConn {
            db_type: "postgres".into(),
            host: "tgt".into(),
            port: "5432".into(),
            database: "demo".into(),
            username: "u".into(),
            password: "p".into(),
            schema: String::new(),
        };
        assert!(require_pgloader_pair(&conn("mysql", "shop"), &target).is_ok());
        assert!(require_pgloader_pair(&conn("postgres", "shop"), &target).is_err());
    }
}
