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
pub const KIND_PGDUMP: &str = "pgdump_profile";
pub const KIND_RSYNC: &str = "rsync_profile";

const SCHEMA_SQL: &str = "CREATE TABLE IF NOT EXISTS cdc_config (
  kind TEXT NOT NULL,
  id TEXT NOT NULL,
  body TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (kind, id)
)";

const AUDIT_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS cdc_audit_events (
  audit_id TEXT PRIMARY KEY,
  timestamp TEXT NOT NULL,
  actor TEXT NOT NULL,
  action TEXT NOT NULL,
  outcome TEXT NOT NULL,
  detail TEXT NOT NULL,
  profile_id TEXT,
  profile_name TEXT
)";

const AUDIT_INDEX_TS: &str =
    "CREATE INDEX IF NOT EXISTS idx_cdc_audit_timestamp ON cdc_audit_events(timestamp)";
const AUDIT_INDEX_ACTION: &str =
    "CREATE INDEX IF NOT EXISTS idx_cdc_audit_action ON cdc_audit_events(action)";
const AUDIT_INDEX_PROFILE: &str =
    "CREATE INDEX IF NOT EXISTS idx_cdc_audit_profile ON cdc_audit_events(profile_id)";

const PIPELINE_LOG_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS cdc_pipeline_logs (
  log_id TEXT PRIMARY KEY,
  timestamp TEXT NOT NULL,
  profile_id TEXT,
  profile_name TEXT,
  level TEXT NOT NULL,
  phase TEXT NOT NULL,
  action TEXT NOT NULL,
  outcome TEXT NOT NULL,
  message TEXT NOT NULL,
  detail TEXT NOT NULL
)";

const PIPELINE_LOG_INDEX_TS: &str =
    "CREATE INDEX IF NOT EXISTS idx_cdc_pipeline_log_timestamp ON cdc_pipeline_logs(timestamp)";
const PIPELINE_LOG_INDEX_PROFILE: &str =
    "CREATE INDEX IF NOT EXISTS idx_cdc_pipeline_log_profile ON cdc_pipeline_logs(profile_id)";
const PIPELINE_LOG_INDEX_ACTION: &str =
    "CREATE INDEX IF NOT EXISTS idx_cdc_pipeline_log_action ON cdc_pipeline_logs(action)";

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
        for sql in [
            AUDIT_TABLE_SQL,
            AUDIT_INDEX_TS,
            AUDIT_INDEX_ACTION,
            AUDIT_INDEX_PROFILE,
            PIPELINE_LOG_TABLE_SQL,
            PIPELINE_LOG_INDEX_TS,
            PIPELINE_LOG_INDEX_PROFILE,
            PIPELINE_LOG_INDEX_ACTION,
        ] {
            self.execute(json!([sql])).await?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_audit_event(
        &self,
        audit_id: &str,
        timestamp: &str,
        actor: &str,
        action: &str,
        outcome: &str,
        detail: &str,
        profile_id: Option<&str>,
        profile_name: Option<&str>,
    ) -> Result<(), PersistError> {
        self.execute(json!([[
            "INSERT INTO cdc_audit_events(audit_id, timestamp, actor, action, outcome, detail, profile_id, profile_name) \
             VALUES(?, ?, ?, ?, ?, ?, ?, ?)",
            audit_id,
            timestamp,
            actor,
            action,
            outcome,
            detail,
            profile_id,
            profile_name
        ]]))
        .await
    }

    /// Drop oldest rows when the table grows past `max_rows`.
    pub async fn trim_audit_events(&self, max_rows: usize) -> Result<(), PersistError> {
        self.trim_oldest_by_timestamp("cdc_audit_events", "audit_id", max_rows)
            .await
    }

    pub async fn list_audit_events(
        &self,
        limit: usize,
        action: Option<&str>,
        profile_id: Option<&str>,
        outcome: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<Vec<AuditEventRow>, PersistError> {
        self.query_event_rows(
            "SELECT audit_id, timestamp, actor, action, outcome, detail, profile_id, profile_name \
             FROM cdc_audit_events",
            limit,
            |sql, params| {
                push_eq_filter(sql, params, "action", action);
                push_eq_filter(sql, params, "profile_id", profile_id);
                push_eq_filter(sql, params, "outcome", outcome);
                push_ge_filter(sql, params, "timestamp", since);
                push_le_filter(sql, params, "timestamp", until);
            },
            decode_audit_row,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_pipeline_log(
        &self,
        log_id: &str,
        timestamp: &str,
        profile_id: Option<&str>,
        profile_name: Option<&str>,
        level: &str,
        phase: &str,
        action: &str,
        outcome: &str,
        message: &str,
        detail: &str,
    ) -> Result<(), PersistError> {
        self.execute(json!([[
            "INSERT INTO cdc_pipeline_logs(log_id, timestamp, profile_id, profile_name, level, phase, action, outcome, message, detail) \
             VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            log_id,
            timestamp,
            profile_id,
            profile_name,
            level,
            phase,
            action,
            outcome,
            message,
            detail
        ]]))
        .await
    }

    pub async fn trim_pipeline_logs(&self, max_rows: usize) -> Result<(), PersistError> {
        self.trim_oldest_by_timestamp("cdc_pipeline_logs", "log_id", max_rows)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list_pipeline_logs(
        &self,
        limit: usize,
        action: Option<&str>,
        profile_id: Option<&str>,
        outcome: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
        level: Option<&str>,
        phase: Option<&str>,
    ) -> Result<Vec<PipelineLogRow>, PersistError> {
        self.query_event_rows(
            "SELECT log_id, timestamp, profile_id, profile_name, level, phase, action, outcome, message, detail \
             FROM cdc_pipeline_logs",
            limit,
            |sql, params| {
                push_eq_filter(sql, params, "action", action);
                push_eq_filter(sql, params, "profile_id", profile_id);
                push_eq_filter(sql, params, "outcome", outcome);
                push_ge_filter(sql, params, "timestamp", since);
                push_le_filter(sql, params, "timestamp", until);
                push_eq_filter(sql, params, "level", level);
                push_eq_filter(sql, params, "phase", phase);
            },
            decode_pipeline_log_row,
        )
        .await
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

    async fn query_event_rows<T, F, G>(
        &self,
        select_from: &str,
        limit: usize,
        apply_filters: F,
        decode_row: G,
    ) -> Result<Vec<T>, PersistError>
    where
        F: FnOnce(&mut String, &mut Vec<Value>),
        G: Fn(&[Value]) -> Result<T, PersistError>,
    {
        let cap = limit.clamp(1, 2000) as i64;
        let mut sql = format!("{select_from} WHERE 1=1");
        let mut params: Vec<Value> = Vec::new();
        apply_filters(&mut sql, &mut params);
        sql.push_str(" ORDER BY timestamp DESC LIMIT ?");
        params.push(json!(cap));
        let mut stmt: Vec<Value> = vec![json!(sql)];
        stmt.extend(params);
        let rows = self.query(json!([stmt])).await?;
        rows.into_iter().map(|row| decode_row(&row)).collect()
    }

    async fn trim_oldest_by_timestamp(
        &self,
        table: &str,
        id_column: &str,
        max_rows: usize,
    ) -> Result<(), PersistError> {
        if max_rows == 0 {
            return Ok(());
        }
        let max = max_rows as i64;
        let sql = format!(
            "DELETE FROM {table} WHERE {id_column} IN (\
               SELECT {id_column} FROM {table} ORDER BY timestamp ASC \
               LIMIT MAX(0, (SELECT COUNT(*) FROM {table}) - ?)\
             )"
        );
        self.execute(json!([[sql, max]])).await
    }

    async fn execute(&self, body: Value) -> Result<(), PersistError> {
        let url = format!("{}/db/execute?level=strong", self.base);
        let resp = self
            .apply_auth(self.inner.post(url).json(&body))
            .send()
            .await
            .map_err(|e| PersistError::Upstream(format!("rqlite execute: {e}")))?;
        let status = resp.status();
        let raw = resp
            .text()
            .await
            .map_err(|e| PersistError::Upstream(format!("rqlite execute body: {e}")))?;
        let payload: Value = serde_json::from_str(&raw).map_err(|e| {
            PersistError::Upstream(format!(
                "rqlite execute json: {e} (body: {})",
                raw.chars().take(200).collect::<String>()
            ))
        })?;
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
        let raw = resp
            .text()
            .await
            .map_err(|e| PersistError::Upstream(format!("rqlite query body: {e}")))?;
        let payload: Value = serde_json::from_str(&raw).map_err(|e| {
            PersistError::Upstream(format!(
                "rqlite query json: {e} (body: {})",
                raw.chars().take(200).collect::<String>()
            ))
        })?;
        if !status.is_success() {
            return Err(PersistError::Upstream(format!(
                "rqlite query {status}: {payload}"
            )));
        }
        first_result_error(&payload).map_err(PersistError::Upstream)?;
        Ok(result_values(&payload))
    }
}

fn push_eq_filter(sql: &mut String, params: &mut Vec<Value>, column: &str, value: Option<&str>) {
    if let Some(v) = value {
        sql.push_str(&format!(" AND {column}=?"));
        params.push(json!(v));
    }
}

fn push_ge_filter(sql: &mut String, params: &mut Vec<Value>, column: &str, value: Option<&str>) {
    if let Some(v) = value {
        sql.push_str(&format!(" AND {column}>=?"));
        params.push(json!(v));
    }
}

fn push_le_filter(sql: &mut String, params: &mut Vec<Value>, column: &str, value: Option<&str>) {
    if let Some(v) = value {
        sql.push_str(&format!(" AND {column}<=?"));
        params.push(json!(v));
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditEventRow {
    pub audit_id: String,
    pub timestamp: String,
    pub actor: String,
    pub action: String,
    pub outcome: String,
    pub detail: String,
    pub profile_id: Option<String>,
    pub profile_name: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PipelineLogRow {
    pub log_id: String,
    pub timestamp: String,
    pub profile_id: Option<String>,
    pub profile_name: Option<String>,
    pub level: String,
    pub phase: String,
    pub action: String,
    pub outcome: String,
    pub message: String,
    pub detail: String,
}

fn decode_pipeline_log_row(row: &[Value]) -> Result<PipelineLogRow, PersistError> {
    let col = |i: usize| -> Option<String> {
        row.get(i).and_then(|v| match v {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        })
    };
    Ok(PipelineLogRow {
        log_id: col(0)
            .ok_or_else(|| PersistError::Internal("pipeline log missing log_id".into()))?,
        timestamp: col(1)
            .ok_or_else(|| PersistError::Internal("pipeline log missing timestamp".into()))?,
        profile_id: col(2),
        profile_name: col(3),
        level: col(4).unwrap_or_else(|| "info".into()),
        phase: col(5).unwrap_or_else(|| "sync".into()),
        action: col(6).unwrap_or_default(),
        outcome: col(7).unwrap_or_default(),
        message: col(8).unwrap_or_default(),
        detail: col(9).unwrap_or_default(),
    })
}

fn decode_audit_row(row: &[Value]) -> Result<AuditEventRow, PersistError> {
    let col = |i: usize| -> Option<String> {
        row.get(i).and_then(|v| match v {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        })
    };
    Ok(AuditEventRow {
        audit_id: col(0)
            .ok_or_else(|| PersistError::Internal("audit row missing audit_id".into()))?,
        timestamp: col(1)
            .ok_or_else(|| PersistError::Internal("audit row missing timestamp".into()))?,
        actor: col(2).unwrap_or_default(),
        action: col(3).unwrap_or_default(),
        outcome: col(4).unwrap_or_default(),
        detail: col(5).unwrap_or_default(),
        profile_id: col(6),
        profile_name: col(7),
    })
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
