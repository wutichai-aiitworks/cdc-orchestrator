use chrono::Utc;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::info;
use uuid::Uuid;

use crate::models::CdcProfile;
use crate::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct AuditEvent {
    pub audit_id: String,
    pub timestamp: String,
    pub actor: String,
    pub action: String,
    pub outcome: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_name: Option<String>,
}

pub fn log_action(actor: &str, action: &str, outcome: &str, detail: &str) -> String {
    let audit_id = Uuid::new_v4().to_string();
    info!(
        audit_id = %audit_id,
        actor = %actor,
        action = %action,
        outcome = %outcome,
        detail = %detail,
        timestamp = %Utc::now().to_rfc3339(),
        "audit_event"
    );
    audit_id
}

/// Structured audit: always logged; persisted to Redis when `ORCH_AUDIT_REDIS_URL` is set.
pub async fn emit(state: &AppState, event: AuditEvent) -> String {
    info!(
        audit_id = %event.audit_id,
        actor = %event.actor,
        action = %event.action,
        outcome = %event.outcome,
        detail = %event.detail,
        profile_id = ?event.profile_id,
        profile_name = ?event.profile_name,
        timestamp = %event.timestamp,
        "audit_event"
    );
    if let Some(client) = state.audit_redis.as_ref() {
        if let Err(e) = persist_redis(state, client, &event).await {
            tracing::warn!(error = %e, action = %event.action, "audit redis persist failed");
        }
    }
    if let Some(rqlite) = state.rqlite.as_ref() {
        if let Err(e) = persist_rqlite(state, rqlite, &event).await {
            tracing::warn!(error = %e, action = %event.action, "audit rqlite persist failed");
        }
    }
    event.audit_id
}

async fn persist_rqlite(
    state: &AppState,
    rqlite: &hyper_cdc_orchestrator::persist::RqliteClient,
    event: &AuditEvent,
) -> Result<(), hyper_cdc_orchestrator::persist::PersistError> {
    rqlite
        .insert_audit_event(
            &event.audit_id,
            &event.timestamp,
            &event.actor,
            &event.action,
            &event.outcome,
            &event.detail,
            event.profile_id.as_deref(),
            event.profile_name.as_deref(),
        )
        .await?;
    rqlite
        .trim_audit_events(state.config.audit_rqlite_max_rows)
        .await?;
    let (level, phase, message) = pipeline_log_fields(&event.action, &event.outcome);
    rqlite
        .insert_pipeline_log(
            &event.audit_id,
            &event.timestamp,
            event.profile_id.as_deref(),
            event.profile_name.as_deref(),
            &level,
            &phase,
            &event.action,
            &event.outcome,
            &message,
            &event.detail,
        )
        .await?;
    rqlite
        .trim_pipeline_logs(state.config.audit_rqlite_max_rows)
        .await?;
    Ok(())
}

fn pipeline_log_fields(action: &str, outcome: &str) -> (String, String, String) {
    let phase = if action.starts_with("profile.") {
        "control".into()
    } else if action.starts_with("sync.") {
        "sync".into()
    } else if action.starts_with("connector.") {
        "connector".into()
    } else {
        "ops".into()
    };
    let level =
        if outcome == "failure" || action == "connector.failed" || action == "sync.dlq_growth" {
            "error".into()
        } else {
            "info".into()
        };
    let message = match action {
        "profile.start" => "Pipeline started".into(),
        "profile.stop" => "Pipeline stopped".into(),
        "profile.pause" => "Pipeline paused".into(),
        "profile.resume" => "Pipeline resumed".into(),
        "profile.restart" => "Pipeline restarted".into(),
        "profile.resync" => "Pipeline resync requested".into(),
        "profile.test_initial_sync" => "Test initial sync (row limit)".into(),
        "profile.initial_load_start" => "Initial load started (streaming)".into(),
        "profile.initial_load_cancel" => "Initial load cancel requested".into(),
        "profile.initial_load" => "Initial load finished".into(),
        "profile.create" => "Pipeline created".into(),
        "profile.update" => "Pipeline configuration updated".into(),
        "profile.delete" => "Pipeline deleted".into(),
        "profile.enable" => "Pipeline enable flag changed".into(),
        "sync.interval_tick" => "JDBC poll interval (scheduled)".into(),
        "sync.watermark_advanced" => "Source watermark advanced".into(),
        "sync.dlq_growth" => "Dead-letter queue grew (sink rejects)".into(),
        "connector.failed" => "Connector entered FAILED state".into(),
        "connector.paused" => "Connector paused".into(),
        "connector.running" => "Connector running".into(),
        "profile.split" => "Pipeline split: tables moved to a new pipeline".into(),
        "connector.auto_restart" => "Connector auto-restarted after a temporary failure".into(),
        "connector.state_changed" => "Connector state changed".into(),
        other => other.to_string(),
    };
    (level, phase, message)
}

pub fn new_event(
    actor: &str,
    action: &str,
    outcome: &str,
    detail: &str,
    profile_id: Option<uuid::Uuid>,
    profile_name: Option<&str>,
) -> AuditEvent {
    AuditEvent {
        audit_id: Uuid::new_v4().to_string(),
        timestamp: Utc::now().to_rfc3339(),
        actor: actor.to_string(),
        action: action.to_string(),
        outcome: outcome.to_string(),
        detail: detail.to_string(),
        profile_id: profile_id.map(|id| id.to_string()),
        profile_name: profile_name.map(str::to_string),
    }
}

pub fn detail_json(value: Value) -> String {
    value.to_string()
}

/// Snapshot fields operators need when triaging a pipeline (no secrets).
pub fn profile_snapshot(profile: &CdcProfile) -> Value {
    json!({
        "id": profile.id,
        "name": profile.name,
        "status": format!("{:?}", profile.status),
        "enabled": profile.enabled,
        "pattern": format!("{:?}", profile.pattern),
        "capture_mode": format!("{:?}", profile.source.capture_mode),
        "source_connector": profile.connector_name,
        "sink_connector": profile.sink_connector_name,
        "delivery_mode": format!("{:?}", profile.flow.schedule.delivery_mode),
        "interval_seconds": profile.flow.schedule.interval_seconds,
        "last_error": profile.last_error,
        "updated_at": profile.updated_at.to_rfc3339(),
    })
}

pub async fn emit_system(
    state: &AppState,
    action: &str,
    outcome: &str,
    detail: Value,
    profile_id: Option<Uuid>,
    profile_name: Option<&str>,
) -> String {
    let event = new_event(
        "system:cdc",
        action,
        outcome,
        &detail_json(detail),
        profile_id,
        profile_name,
    );
    emit(state, event).await
}

async fn persist_redis(
    state: &AppState,
    client: &ConnectionManager,
    event: &AuditEvent,
) -> Result<(), redis::RedisError> {
    let payload = serde_json::to_string(event).map_err(|e| {
        redis::RedisError::from((redis::ErrorKind::TypeError, "audit json", e.to_string()))
    })?;
    let mut conn = client.clone();
    let key = state.config.audit_redis_key.as_str();
    let _: () = conn.lpush(key, payload).await?;
    let _: () = conn
        .ltrim(key, 0, state.config.audit_redis_max_len as isize - 1)
        .await?;
    Ok(())
}

pub async fn open_redis(url: &str) -> Option<ConnectionManager> {
    match redis::Client::open(url) {
        Ok(client) => match ConnectionManager::new(client).await {
            Ok(mgr) => {
                info!(redis = %url, "audit events will be persisted to Redis");
                Some(mgr)
            }
            Err(e) => {
                tracing::warn!(error = %e, redis = %url, "audit redis connection failed");
                None
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, redis = %url, "audit redis URL invalid");
            None
        }
    }
}
