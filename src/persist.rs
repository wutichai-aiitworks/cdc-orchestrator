//! rqlite persistence for CDC control-plane configuration.

use chrono::Utc;
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;

pub const KIND_PROFILE: &str = "profile";
pub const KIND_RULE: &str = "rule";
/// Notification channel (webhook, email, LINE, …) — ADR 0015.
pub const KIND_CHANNEL: &str = "channel";
pub const KIND_PROCESSING: &str = "processing";
pub const KIND_SECRET: &str = "secret";
pub const KIND_COMPARE: &str = "compare_profile";
pub const KIND_PGCOPY: &str = "pgcopy_profile";
pub const KIND_REDISSHAKE: &str = "redisshake_profile";
pub const KIND_PGLOADER: &str = "pgloader_profile";
pub const KIND_MONGOCOPY: &str = "mongocopy_profile";
pub const KIND_PGDUMP: &str = "pgdump_profile";
pub const KIND_RSYNC: &str = "rsync_profile";
pub const KIND_WIREGUARD: &str = "wireguard_tunnel";

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

/// Copy & Sync tool timeline (ADR 0009): profile changes, run start/finish, stops, connection
/// tests. Separate from cdc_audit_events so pipeline noise cannot evict run history.
const TOOL_EVENTS_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS cdc_tool_events (
  event_id TEXT PRIMARY KEY,
  timestamp TEXT NOT NULL,
  tool TEXT NOT NULL,
  profile_id TEXT,
  profile_name TEXT,
  job_id TEXT,
  kind TEXT NOT NULL,
  status TEXT,
  actor TEXT NOT NULL,
  summary TEXT NOT NULL,
  detail TEXT NOT NULL
)";
const TOOL_EVENTS_INDEX_PROFILE: &str =
    "CREATE INDEX IF NOT EXISTS idx_cdc_tool_events_profile ON cdc_tool_events(tool, profile_id, timestamp)";
const TOOL_EVENTS_INDEX_TS: &str =
    "CREATE INDEX IF NOT EXISTS idx_cdc_tool_events_ts ON cdc_tool_events(timestamp)";

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

/// One final notification delivery result (ADR 0015).
const DELIVERY_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS cdc_notification_deliveries (
  delivery_id TEXT PRIMARY KEY,
  timestamp TEXT NOT NULL,
  channel_id TEXT,
  channel_name TEXT NOT NULL,
  channel_type TEXT NOT NULL,
  rule_id TEXT,
  rule_name TEXT,
  condition TEXT NOT NULL,
  subject TEXT NOT NULL,
  outcome TEXT NOT NULL,
  attempts INTEGER NOT NULL,
  status_code INTEGER,
  error TEXT,
  duration_ms INTEGER NOT NULL,
  test INTEGER NOT NULL
)";
const DELIVERY_INDEX_TS: &str =
    "CREATE INDEX IF NOT EXISTS idx_cdc_notification_deliveries_ts ON cdc_notification_deliveries(timestamp)";
const DELIVERY_INDEX_CHANNEL: &str =
    "CREATE INDEX IF NOT EXISTS idx_cdc_notification_deliveries_channel ON cdc_notification_deliveries(channel_id)";

/// 5-minute latency rollups per pipeline (ADR 0016). Keyed by (profile_id, bucket).
const METRICS_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS cdc_pipeline_metrics (
  profile_id TEXT NOT NULL,
  timestamp TEXT NOT NULL,
  e2e_p50_ms INTEGER,
  e2e_p95_ms INTEGER,
  e2e_max_ms INTEGER,
  capture_max_ms INTEGER,
  sink_max_ms INTEGER,
  lag_max INTEGER,
  throughput_avg REAL,
  samples INTEGER NOT NULL,
  method TEXT NOT NULL,
  PRIMARY KEY (profile_id, timestamp)
)";
const METRICS_INDEX_TS: &str =
    "CREATE INDEX IF NOT EXISTS idx_cdc_pipeline_metrics_ts ON cdc_pipeline_metrics(timestamp)";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MetricsRollup {
    pub profile_id: String,
    /// Bucket start (RFC 3339, UTC).
    pub timestamp: String,
    pub e2e_p50_ms: Option<i64>,
    pub e2e_p95_ms: Option<i64>,
    pub e2e_max_ms: Option<i64>,
    pub capture_max_ms: Option<i64>,
    pub sink_max_ms: Option<i64>,
    pub lag_max: Option<i64>,
    pub throughput_avg: Option<f64>,
    pub samples: u32,
    pub method: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DeliveryRow {
    pub delivery_id: String,
    pub timestamp: String,
    pub channel_id: Option<String>,
    pub channel_name: String,
    pub channel_type: String,
    pub rule_id: Option<String>,
    pub rule_name: Option<String>,
    pub condition: String,
    pub subject: String,
    /// `delivered`, `failed` or `dropped` (queue full).
    pub outcome: String,
    pub attempts: u32,
    pub status_code: Option<u16>,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub test: bool,
}

fn decode_delivery_row(row: &[Value]) -> Result<DeliveryRow, PersistError> {
    let s = |i: usize| -> Option<String> {
        row.get(i).and_then(|v| match v {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        })
    };
    let n = |i: usize| row.get(i).and_then(Value::as_i64);
    Ok(DeliveryRow {
        delivery_id: s(0).ok_or_else(|| PersistError::Internal("delivery missing id".into()))?,
        timestamp: s(1).unwrap_or_default(),
        channel_id: s(2),
        channel_name: s(3).unwrap_or_default(),
        channel_type: s(4).unwrap_or_default(),
        rule_id: s(5),
        rule_name: s(6),
        condition: s(7).unwrap_or_default(),
        subject: s(8).unwrap_or_default(),
        outcome: s(9).unwrap_or_default(),
        attempts: n(10).unwrap_or(0).max(0) as u32,
        status_code: n(11).map(|v| v as u16),
        error: s(12),
        duration_ms: n(13).unwrap_or(0).max(0) as u64,
        test: n(14).unwrap_or(0) != 0,
    })
}

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
            TOOL_EVENTS_TABLE_SQL,
            TOOL_EVENTS_INDEX_PROFILE,
            TOOL_EVENTS_INDEX_TS,
            DELIVERY_TABLE_SQL,
            DELIVERY_INDEX_TS,
            DELIVERY_INDEX_CHANNEL,
            METRICS_TABLE_SQL,
            METRICS_INDEX_TS,
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

    /// Insert a tool event. Ids are deterministic for run events, so a repeated insert (watcher
    /// poll, restart) is ignored rather than duplicated.
    pub async fn insert_tool_event(&self, e: &ToolEventRow) -> Result<(), PersistError> {
        self.execute(json!([[
            "INSERT OR IGNORE INTO cdc_tool_events(event_id, timestamp, tool, profile_id, profile_name, job_id, kind, status, actor, summary, detail) \
             VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            e.event_id,
            e.timestamp,
            e.tool,
            e.profile_id,
            e.profile_name,
            e.job_id,
            e.kind,
            e.status,
            e.actor,
            e.summary,
            e.detail
        ]]))
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list_tool_events(
        &self,
        limit: usize,
        tool: Option<&str>,
        profile_id: Option<&str>,
        kind_prefix: Option<&str>,
        status: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<Vec<ToolEventRow>, PersistError> {
        self.query_event_rows(
            "SELECT event_id, timestamp, tool, profile_id, profile_name, job_id, kind, status, actor, summary, detail \
             FROM cdc_tool_events",
            limit,
            |sql, params| {
                push_eq_filter(sql, params, "tool", tool);
                push_eq_filter(sql, params, "profile_id", profile_id);
                if let Some(k) = kind_prefix {
                    sql.push_str(" AND kind LIKE ?");
                    params.push(json!(format!("{k}%")));
                }
                push_eq_filter(sql, params, "status", status);
                push_ge_filter(sql, params, "timestamp", since);
                push_le_filter(sql, params, "timestamp", until);
            },
            decode_tool_event_row,
        )
        .await
    }

    /// Retention: drop events older than `before` (RFC 3339), then keep at most `max_rows`.
    pub async fn trim_tool_events(
        &self,
        before: &str,
        max_rows: usize,
    ) -> Result<(), PersistError> {
        self.execute(json!([[
            "DELETE FROM cdc_tool_events WHERE timestamp < ?",
            before
        ]]))
        .await?;
        self.trim_oldest_by_timestamp("cdc_tool_events", "event_id", max_rows)
            .await
    }

    pub async fn insert_delivery(&self, d: &DeliveryRow) -> Result<(), PersistError> {
        self.execute(json!([[
            "INSERT OR IGNORE INTO cdc_notification_deliveries(delivery_id, timestamp, channel_id, channel_name, channel_type, rule_id, rule_name, condition, subject, outcome, attempts, status_code, error, duration_ms, test) \
             VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            d.delivery_id,
            d.timestamp,
            d.channel_id,
            d.channel_name,
            d.channel_type,
            d.rule_id,
            d.rule_name,
            d.condition,
            d.subject,
            d.outcome,
            d.attempts,
            d.status_code,
            d.error,
            d.duration_ms,
            i64::from(d.test)
        ]]))
        .await
    }

    pub async fn list_deliveries(
        &self,
        limit: usize,
        channel_id: Option<&str>,
        rule_id: Option<&str>,
        outcome: Option<&str>,
    ) -> Result<Vec<DeliveryRow>, PersistError> {
        self.query_event_rows(
            "SELECT delivery_id, timestamp, channel_id, channel_name, channel_type, rule_id, rule_name, condition, subject, outcome, attempts, status_code, error, duration_ms, test \
             FROM cdc_notification_deliveries",
            limit,
            |sql, params| {
                push_eq_filter(sql, params, "channel_id", channel_id);
                push_eq_filter(sql, params, "rule_id", rule_id);
                push_eq_filter(sql, params, "outcome", outcome);
            },
            decode_delivery_row,
        )
        .await
    }

    pub async fn trim_deliveries(&self, max_rows: usize) -> Result<(), PersistError> {
        self.trim_oldest_by_timestamp("cdc_notification_deliveries", "delivery_id", max_rows)
            .await
    }

    pub async fn insert_metrics_rollup(&self, m: &MetricsRollup) -> Result<(), PersistError> {
        self.execute(json!([[
            "INSERT OR REPLACE INTO cdc_pipeline_metrics(profile_id, timestamp, e2e_p50_ms, e2e_p95_ms, e2e_max_ms, capture_max_ms, sink_max_ms, lag_max, throughput_avg, samples, method) \
             VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            m.profile_id,
            m.timestamp,
            m.e2e_p50_ms,
            m.e2e_p95_ms,
            m.e2e_max_ms,
            m.capture_max_ms,
            m.sink_max_ms,
            m.lag_max,
            m.throughput_avg,
            m.samples,
            m.method
        ]]))
        .await
    }

    /// Rollups for one pipeline since `since` (RFC 3339), oldest first.
    pub async fn list_metrics_rollups(
        &self,
        profile_id: &str,
        since: &str,
    ) -> Result<Vec<MetricsRollup>, PersistError> {
        let rows = self
            .query(json!([[
                "SELECT profile_id, timestamp, e2e_p50_ms, e2e_p95_ms, e2e_max_ms, capture_max_ms, sink_max_ms, lag_max, throughput_avg, samples, method \
                 FROM cdc_pipeline_metrics WHERE profile_id = ? AND timestamp >= ? ORDER BY timestamp ASC LIMIT 10000",
                profile_id,
                since
            ]]))
            .await?;
        let i = |row: &[Value], n: usize| row.get(n).and_then(Value::as_i64);
        Ok(rows
            .iter()
            .map(|row| MetricsRollup {
                profile_id: row
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                timestamp: row
                    .get(1)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                e2e_p50_ms: i(row, 2),
                e2e_p95_ms: i(row, 3),
                e2e_max_ms: i(row, 4),
                capture_max_ms: i(row, 5),
                sink_max_ms: i(row, 6),
                lag_max: i(row, 7),
                throughput_avg: row.get(8).and_then(Value::as_f64),
                samples: i(row, 9).unwrap_or(0).max(0) as u32,
                method: row
                    .get(10)
                    .and_then(Value::as_str)
                    .unwrap_or("estimate")
                    .to_string(),
            })
            .collect())
    }

    /// Retention: drop rollups older than `before` (RFC 3339).
    pub async fn trim_metrics_rollups(&self, before: &str) -> Result<(), PersistError> {
        self.execute(json!([[
            "DELETE FROM cdc_pipeline_metrics WHERE timestamp < ?",
            before
        ]]))
        .await
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

/// One entry of a Copy & Sync profile's timeline (ADR 0009). `detail` is a JSON string.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ToolEventRow {
    pub event_id: String,
    pub timestamp: String,
    pub tool: String,
    pub profile_id: Option<String>,
    pub profile_name: Option<String>,
    pub job_id: Option<String>,
    pub kind: String,
    pub status: Option<String>,
    pub actor: String,
    pub summary: String,
    pub detail: String,
}

fn decode_tool_event_row(row: &[Value]) -> Result<ToolEventRow, PersistError> {
    let col = |i: usize| -> Option<String> {
        row.get(i).and_then(|v| match v {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        })
    };
    Ok(ToolEventRow {
        event_id: col(0)
            .ok_or_else(|| PersistError::Internal("tool event missing event_id".into()))?,
        timestamp: col(1)
            .ok_or_else(|| PersistError::Internal("tool event missing timestamp".into()))?,
        tool: col(2).unwrap_or_default(),
        profile_id: col(3),
        profile_name: col(4),
        job_id: col(5),
        kind: col(6).unwrap_or_default(),
        status: col(7),
        actor: col(8).unwrap_or_default(),
        summary: col(9).unwrap_or_default(),
        detail: col(10).unwrap_or_else(|| "{}".into()),
    })
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
