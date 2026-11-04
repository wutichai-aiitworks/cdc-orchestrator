//! Pieces shared by the Copy & Sync sidecar clients (pgcopy, pgloader, mongocopy, redisshake).

use reqwest::header::{HeaderValue, AUTHORIZATION};
use reqwest::{Client, Method};
use serde::Deserialize;
use serde_json::{json, Value};
use urlencoding::encode;
use uuid::Uuid;

use crate::error::AppError;
use crate::models::SchemaCompareConn;
use crate::saved_secret::{keep_password, REDACTED};

/// Sidecar HTTP status → API error, keeping the sidecar's message (the editor shows it).
/// 409 = the transfer already has an active run.
#[allow(dead_code)]
pub fn map_error(tool: &str, token_env: &str, status: u16, msg: &str) -> AppError {
    map_error_status(tool, token_env, status, msg, &[])
}

/// Like [`map_error`], but treats extra status codes as client errors (e.g. pgdump 507).
pub fn map_error_status(
    tool: &str,
    token_env: &str,
    status: u16,
    msg: &str,
    extra_bad_request: &[u16],
) -> AppError {
    let text = format!("{tool} sidecar {status}: {msg}");
    if extra_bad_request.contains(&status) {
        return AppError::BadRequest(text);
    }
    match status {
        404 => AppError::NotFound(text),
        401 | 403 => AppError::Upstream(format!("{text} (check {token_env})")),
        400..=499 => AppError::BadRequest(text),
        _ => AppError::Upstream(text),
    }
}

/// JSON request to a sidecar base URL (`/v1/...`).
#[allow(clippy::too_many_arguments)]
pub async fn sidecar_json_request(
    client: &Client,
    base_url: &str,
    bearer_token: Option<&str>,
    tool: &str,
    token_env: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
    extra_bad_request: &[u16],
) -> Result<Value, AppError> {
    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let mut req = client.request(method, &url);
    if let Some(token) = bearer_token {
        let value = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|e| AppError::Internal(format!("{tool} auth header: {e}")))?;
        req = req.header(AUTHORIZATION, value);
    }
    if let Some(body) = body {
        req = req.json(&body);
    }
    let res = req
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("{tool} sidecar request failed: {e}")))?;
    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("{tool} sidecar read body: {e}")))?;
    let parsed: Value = serde_json::from_str(&text).unwrap_or(json!({ "raw": text }));
    if status.is_success() {
        Ok(parsed)
    } else {
        let msg = parsed
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or(&text);
        Err(map_error_status(
            tool,
            token_env,
            status.as_u16(),
            msg,
            extra_bad_request,
        ))
    }
}

/// Shared `/v1/*` job API implemented by Copy & Sync sidecars.
pub struct SidecarAccess<'a> {
    pub client: &'a Client,
    pub base_url: &'a str,
    pub bearer_token: Option<&'a str>,
    pub tool: &'static str,
    pub token_env: &'static str,
    pub extra_bad_request: &'static [u16],
}

impl SidecarAccess<'_> {
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, AppError> {
        sidecar_json_request(
            self.client,
            self.base_url,
            self.bearer_token,
            self.tool,
            self.token_env,
            method,
            path,
            body,
            self.extra_bad_request,
        )
        .await
    }

    pub async fn capability(&self) -> Result<Value, AppError> {
        self.request(Method::GET, "/v1/capability", None).await
    }

    pub async fn discover(&self) -> Result<Value, AppError> {
        self.request(Method::GET, "/v1/discover", None).await
    }

    pub async fn list_jobs(&self) -> Result<Value, AppError> {
        self.request(Method::GET, "/v1/jobs", None).await
    }

    pub async fn get_job(&self, job_id: &str) -> Result<Value, AppError> {
        self.request(Method::GET, &format!("/v1/jobs/{}", encode(job_id)), None)
            .await
    }

    pub async fn job_logs(&self, job_id: &str) -> Result<Value, AppError> {
        self.request(
            Method::GET,
            &format!("/v1/jobs/{}/logs", encode(job_id)),
            None,
        )
        .await
    }

    pub async fn start_job(&self, body: Value) -> Result<Value, AppError> {
        self.request(Method::POST, "/v1/jobs", Some(body)).await
    }

    pub async fn stop_job(&self, job_id: &str) -> Result<Value, AppError> {
        self.request(
            Method::POST,
            &format!("/v1/jobs/{}/stop", encode(job_id)),
            None,
        )
        .await
    }

    pub async fn test_connection(&self, body: Value) -> Result<Value, AppError> {
        self.request(Method::POST, "/v1/test-connection", Some(body))
            .await
    }

    pub async fn delete_job(&self, job_id: &str) -> Result<Value, AppError> {
        self.request(
            Method::DELETE,
            &format!("/v1/jobs/{}", encode(job_id)),
            None,
        )
        .await
    }
}

/// Standard `/v1/*` sidecar handlers (capability, jobs, test-connection).
#[macro_export]
macro_rules! sidecar_v1_handlers {
    ($access:ident) => {
        pub async fn capability(
            cfg: &$crate::config::AppConfig,
            client: &reqwest::Client,
        ) -> Result<serde_json::Value, $crate::error::AppError> {
            $access(cfg, client)?.capability().await
        }

        pub async fn discover(
            cfg: &$crate::config::AppConfig,
            client: &reqwest::Client,
        ) -> Result<serde_json::Value, $crate::error::AppError> {
            $access(cfg, client)?.discover().await
        }

        pub async fn list_jobs(
            cfg: &$crate::config::AppConfig,
            client: &reqwest::Client,
        ) -> Result<serde_json::Value, $crate::error::AppError> {
            $access(cfg, client)?.list_jobs().await
        }

        pub async fn get_job(
            cfg: &$crate::config::AppConfig,
            client: &reqwest::Client,
            job_id: &str,
        ) -> Result<serde_json::Value, $crate::error::AppError> {
            $access(cfg, client)?.get_job(job_id).await
        }

        pub async fn job_logs(
            cfg: &$crate::config::AppConfig,
            client: &reqwest::Client,
            job_id: &str,
        ) -> Result<serde_json::Value, $crate::error::AppError> {
            $access(cfg, client)?.job_logs(job_id).await
        }

        pub async fn start_job(
            cfg: &$crate::config::AppConfig,
            client: &reqwest::Client,
            body: serde_json::Value,
        ) -> Result<serde_json::Value, $crate::error::AppError> {
            $access(cfg, client)?.start_job(body).await
        }

        pub async fn stop_job(
            cfg: &$crate::config::AppConfig,
            client: &reqwest::Client,
            job_id: &str,
        ) -> Result<serde_json::Value, $crate::error::AppError> {
            $access(cfg, client)?.stop_job(job_id).await
        }

        pub async fn test_connection(
            cfg: &$crate::config::AppConfig,
            client: &reqwest::Client,
            body: serde_json::Value,
        ) -> Result<serde_json::Value, $crate::error::AppError> {
            $access(cfg, client)?.test_connection(body).await
        }
    };
}

/// `POST /cdc/{tool}/test-connection`
#[derive(Debug, Deserialize)]
pub struct TestConnectionRequest {
    pub connection: SchemaCompareConn,
    /// `source` (default) or `target`.
    #[serde(default)]
    pub side: Option<String>,
    /// Saved profile whose stored password fills a redacted `********` (same login only).
    #[serde(default)]
    pub profile_id: Option<Uuid>,
    /// Settings outside the connection that change how it connects (TLS, cluster, auth db, CSV separator…).
    #[serde(default)]
    pub options: Option<Value>,
}

/// Validates the side, restores a redacted password from the stored profile (same login only),
/// and builds the sidecar body. `stored` is the saved profile's (source, target).
pub fn test_body(
    req: TestConnectionRequest,
    stored: Option<(SchemaCompareConn, SchemaCompareConn)>,
) -> Result<Value, AppError> {
    let side = req.side.as_deref().unwrap_or("source").to_string();
    if side != "source" && side != "target" {
        return Err(AppError::BadRequest("side must be source or target".into()));
    }
    let mut conn = req.connection;
    if conn.password == REDACTED {
        let saved = stored.map(|(s, t)| if side == "source" { s } else { t });
        keep_password(&side, &mut conn, saved.as_ref())?;
    }
    if conn.host.trim().is_empty() && conn.database.trim().is_empty() {
        return Err(AppError::BadRequest(
            "connection host (or file path) is required".into(),
        ));
    }
    Ok(
        json!({ "connection": conn, "side": side, "options": req.options.unwrap_or_else(|| json!({})) }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(host: &str, password: &str) -> SchemaCompareConn {
        SchemaCompareConn {
            db_type: "redis".into(),
            host: host.into(),
            port: "6379".into(),
            database: String::new(),
            username: String::new(),
            password: password.into(),
            schema: String::new(),
        }
    }

    fn req(c: SchemaCompareConn, side: &str) -> TestConnectionRequest {
        TestConnectionRequest {
            connection: c,
            side: Some(side.into()),
            profile_id: None,
            options: Some(json!({ "target_tls": true })),
        }
    }

    #[test]
    fn saved_password_is_used_only_for_its_own_side_and_login() {
        let stored = Some((conn("src", "src-pw"), conn("tgt", "tgt-pw")));
        let body = test_body(req(conn("tgt", REDACTED), "target"), stored.clone()).unwrap();
        assert_eq!(body["connection"]["password"], "tgt-pw");
        assert_eq!(body["options"]["target_tls"], true);
        let err =
            test_body(req(conn("elsewhere", REDACTED), "target"), stored.clone()).unwrap_err();
        assert!(
            err.to_string().contains("enter the target password again"),
            "{err}"
        );
        assert!(test_body(req(conn("src", ""), "middle"), stored).is_err());
    }

    #[test]
    fn errors_keep_the_sidecar_message() {
        assert!(
            matches!(map_error("pgcopy", "ORCH_PGCOPY_TOKEN", 409, "already running"), AppError::BadRequest(m) if m.contains("already running"))
        );
        assert!(matches!(
            map_error("pgcopy", "ORCH_PGCOPY_TOKEN", 404, "job not found"),
            AppError::NotFound(_)
        ));
        assert!(matches!(
            map_error("pgcopy", "ORCH_PGCOPY_TOKEN", 500, "boom"),
            AppError::Upstream(_)
        ));
    }
}
