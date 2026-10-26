//! rqlite persistence for CDC control-plane configuration.

use chrono::Utc;
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;

pub const KIND_PROFILE: &str = "profile";
pub const KIND_RULE: &str = "rule";
pub const KIND_PROCESSING: &str = "processing";
pub const KIND_SECRET: &str = "secret";
pub const KIND_COMPARE: &str = "compare_profile";
pub const KIND_PGCOPY: &str = "pgcopy_profile";
pub const KIND_REDISSHAKE: &str = "redisshake_profile";
pub const KIND_PGLOADER: &str = "pgloader_profile";
pub const KIND_MONGOCOPY: &str = "mongocopy_profile";

const SCHEMA_SQL: &str = "CREATE TABLE IF NOT EXISTS cdc_config (
  kind TEXT NOT NULL,
  id TEXT NOT NULL,
  body TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (kind, id)
)";

#[derive(Debug)]
pub enum PersistError {
    Upstream(String),
    Internal(String),
}

impl std::fmt::Display for PersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Upstream(m) | Self::Internal(m) => write!(f, "{m}"),
        }
    }
}

#[derive(Clone)]
pub struct RqliteClient {
    base: String,
    inner: Client,
    user: Option<String>,
    pass: Option<String>,
}

impl RqliteClient {
    pub fn try_new(
        base: &str,
        user: Option<String>,
        pass: Option<String>,
    ) -> Result<Self, PersistError> {
        let base = base.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            return Err(PersistError::Internal("rqlite URL is empty".into()));
        }
        let inner = Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| PersistError::Internal(format!("rqlite http client: {e}")))?;
        Ok(Self {
            base,
            inner,
            user,
            pass,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    fn apply_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match (&self.user, &self.pass) {
            (Some(user), pass) => builder.basic_auth(user, pass.clone()),
            _ => builder,
        }
    }

    pub async fn ensure_schema(&self) -> Result<(), PersistError> {
        self.execute(json!([SCHEMA_SQL])).await?;
        Ok(())
    }

    pub async fn put_json<T: Serialize>(
        &self,
        kind: &str,
        id: &str,
        value: &T,
    ) -> Result<(), PersistError> {
        let body = serde_json::to_string(value)
            .map_err(|e| PersistError::Internal(format!("encode {kind}/{id}: {e}")))?;
        let ts = Utc::now().to_rfc3339();
        self.execute(json!([[
            "INSERT INTO cdc_config(kind, id, body, updated_at) VALUES(?, ?, ?, ?) \
             ON CONFLICT(kind, id) DO UPDATE SET body=excluded.body, updated_at=excluded.updated_at",
            kind,
            id,
            body,
            ts
        ]]))
        .await?;
        Ok(())
    }

    pub async fn get_json<T: DeserializeOwned>(
        &self,
        kind: &str,
        id: &str,
    ) -> Result<Option<T>, PersistError> {
        let rows = self
            .query(json!([[
                "SELECT body FROM cdc_config WHERE kind=? AND id=?",
                kind,
                id
            ]]))
            .await?;
        match rows.first() {
            Some(row) => decode_body(row.first(), kind, id).map(Some),
            None => Ok(None),
        }
    }

    pub async fn list_json<T: DeserializeOwned>(&self, kind: &str) -> Result<Vec<T>, PersistError> {
        let rows = self
            .query(json!([[
                "SELECT id, body FROM cdc_config WHERE kind=? ORDER BY id",
                kind
            ]]))
            .await?;
        let mut out = Vec::new();
        for row in rows {
            let id = row.first().and_then(Value::as_str).unwrap_or("unknown");
            match decode_body(row.get(1), kind, id) {
                Ok(item) => out.push(item),
                Err(e) => tracing::warn!(kind, id, error = %e, "skip invalid rqlite row"),
            }
        }
        Ok(out)
    }

    pub async fn delete(&self, kind: &str, id: &str) -> Result<(), PersistError> {
        self.execute(json!([[
            "DELETE FROM cdc_config WHERE kind=? AND id=?",
            kind,
            id
        ]]))
        .await?;
        Ok(())
    }

    async fn execute(&self, body: Value) -> Result<(), PersistError> {
        let url = format!("{}/db/execute?level=strong", self.base);
        let resp = self
            .apply_auth(self.inner.post(url).json(&body))
            .send()
            .await
            .map_err(|e| PersistError::Upstream(format!("rqlite execute: {e}")))?;
        let status = resp.status();
        let payload: Value = resp
            .json()
            .await
            .map_err(|e| PersistError::Upstream(format!("rqlite execute json: {e}")))?;
        if !status.is_success() {
            return Err(PersistError::Upstream(format!(
                "rqlite execute {status}: {payload}"
            )));
        }
        first_result_error(&payload).map_err(PersistError::Upstream)?;
        Ok(())
    }

    async fn query(&self, body: Value) -> Result<Vec<Vec<Value>>, PersistError> {
        let url = format!("{}/db/query?level=strong", self.base);
        let resp = self
            .apply_auth(self.inner.post(url).json(&body))
            .send()
            .await
            .map_err(|e| PersistError::Upstream(format!("rqlite query: {e}")))?;
        let status = resp.status();
        let payload: Value = resp
            .json()
            .await
            .map_err(|e| PersistError::Upstream(format!("rqlite query json: {e}")))?;
        if !status.is_success() {
            return Err(PersistError::Upstream(format!(
                "rqlite query {status}: {payload}"
            )));
        }
        first_result_error(&payload).map_err(PersistError::Upstream)?;
        Ok(result_values(&payload))
    }
}

fn decode_body<T: DeserializeOwned>(
    value: Option<&Value>,
    kind: &str,
    id: &str,
) -> Result<T, PersistError> {
    let raw = match value {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => {
            return Err(PersistError::Upstream(format!(
                "rqlite missing body for {kind}/{id}"
            )))
        }
    };
    serde_json::from_str(&raw)
        .map_err(|e| PersistError::Upstream(format!("decode {kind}/{id}: {e}")))
}

fn first_result_error(payload: &Value) -> Result<(), String> {
    if let Some(results) = payload.get("results").and_then(Value::as_array) {
        for item in results {
            if let Some(err) = item.get("error").and_then(Value::as_str) {
                if !err.is_empty() {
                    return Err(err.to_string());
                }
            }
        }
    }
    Ok(())
}

fn result_values(payload: &Value) -> Vec<Vec<Value>> {
    payload
        .get("results")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("values"))
        .and_then(Value::as_array)
        .map(|rows| rows.iter().filter_map(Value::as_array).cloned().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_query_values() {
        let payload = json!({
            "results": [{
                "columns": ["id", "body"],
                "values": [["p1", "{\"name\":\"demo\"}"]]
            }]
        });
        let rows = result_values(&payload);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], json!("p1"));
    }

    #[test]
    fn surfaces_rqlite_sql_error() {
        let payload = json!({ "results": [{ "error": "no such table: cdc_config" }] });
        assert_eq!(
            first_result_error(&payload).unwrap_err(),
            "no such table: cdc_config"
        );
    }
}
