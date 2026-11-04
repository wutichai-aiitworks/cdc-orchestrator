//! Day-2 ops: cluster, logs, notifications, alerts WebSocket, metrics.

use std::convert::Infallible;
use std::process::Stdio;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use uuid::Uuid;

use crate::audit::log_action;
use crate::command::run_docker;
use crate::error::AppError;
use crate::models::{
    ActionResponse, AlertRule, CapabilityFeatures, CapabilityResponse, HistoryEntry,
};
use crate::reconcile::connect_request;
use crate::AppState;
use axum::http::HeaderMap;
use hyper_cdc_orchestrator::persist::KIND_RULE;

pub async fn capability(State(state): State<AppState>) -> Json<CapabilityResponse> {
    Json(CapabilityResponse {
        service: "hyper-cdc-orchestrator".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        api_prefix: "/cdc".into(),
        features: CapabilityFeatures {
            engine: true,
            profiles: true,
            connectors: true,
            kafka: true,
            logs: true,
            notifications: state.rqlite.is_some(),
            alerts_ws: true,
            metrics: true,
            persistence: state.profiles.persistence_enabled(),
            profile_reconcile: true,
            function_processing: true,
            datalake: true,
            brokers: true,
            schema_registry: true,
            ksql: state.config.ksql_url.is_some(),
            acls: true,
            pgcopy: state.config.pgcopy_url.is_some(),
            redisshake: state.config.redis_shake_url.is_some(),
            pgloader: state.config.pgloader_url.is_some(),
            mongocopy: state.config.mongocopy_url.is_some(),
        },
        timestamp: Utc::now(),
    })
}

pub async fn cluster_info(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let v = connect_request(&state, Method::GET, "/", None).await?;
    Ok(Json(v))
}

pub async fn restart_connector_task(
    State(state): State<AppState>,
    Path((name, task)): Path<(String, u32)>,
    headers: HeaderMap,
) -> Result<Json<ActionResponse>, AppError> {
    let path = format!(
        "/connectors/{}/tasks/{}/restart",
        urlencoding::encode(&name),
        task
    );
    connect_request(&state, Method::POST, &path, None).await?;
    let actor = headers
        .get("x-user")
        .or_else(|| headers.get("x-actor"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("system");
    let audit_id = log_action(
        actor,
        "connect.task.restart",
        "success",
        &format!("{name}/{task}"),
    );
    Ok(Json(ActionResponse {
        action: "connect.task.restart".to_string(),
        status: "success".to_string(),
        message: format!("connector {name} task {task} restart requested"),
        audit_id,
        timestamp: Utc::now(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct TailQuery {
    #[serde(default = "default_tail")]
    pub tail: u32,
}

fn default_tail() -> u32 {
    200
}

pub async fn log_services(State(state): State<AppState>) -> Json<Value> {
    let mut items = serde_json::Map::new();
    for (logical, cn) in &state.config.log_containers {
        items.insert(
            logical.clone(),
            json!({ "service": logical, "container": cn }),
        );
    }
    Json(Value::Object(items))
}

pub async fn snapshot_logs(
    State(state): State<AppState>,
    Path(service): Path<String>,
    Query(q): Query<TailQuery>,
) -> Result<Json<Vec<String>>, AppError> {
    let container = state
        .config
        .log_containers
        .get(&service)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("unknown log service: {service}")))?;
    let output = run_docker(
        &state.config,
        &["logs", "--tail", &q.tail.max(1).to_string(), &container],
    )
    .await?;
    let lines = output
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    Ok(Json(lines))
}

pub async fn stream_logs(
    State(state): State<AppState>,
    Path(service): Path<String>,
    Query(q): Query<TailQuery>,
) -> Result<Response, AppError> {
    let container = state
        .config
        .log_containers
        .get(&service)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("unknown log service: {service}")))?;

    let mut child = Command::new("docker")
        .args([
            "logs",
            "-f",
            "--tail",
            &q.tail.max(1).to_string(),
            &container,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| AppError::Upstream(format!("docker logs spawn: {e}")))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Internal("missing docker stdout".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::Internal("missing docker stderr".into()))?;

    let stream = async_stream::stream! {
        let mut out_lines = BufReader::new(stdout).lines();
        let mut err_lines = BufReader::new(stderr).lines();
        loop {
            tokio::select! {
                line = out_lines.next_line() => {
                    match line {
                        Ok(Some(l)) => yield Ok::<_, Infallible>(Event::default().data(l)),
                        Ok(None) => break,
                        Err(e) => {
                            yield Ok(Event::default().data(format!("[cdc-error] {e}")));
                            break;
                        }
                    }
                }
                line = err_lines.next_line() => {
                    match line {
                        Ok(Some(l)) => yield Ok::<_, Infallible>(Event::default().data(l)),
                        Ok(None) => {},
                        Err(e) => {
                            yield Ok(Event::default().data(format!("[cdc-error] {e}")));
                            break;
                        }
                    }
                }
            }
        }
        let _ = child.kill().await;
    };

    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::new())
        .into_response())
}

pub async fn get_loggers(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let v = connect_request(&state, Method::GET, "/admin/loggers", None).await?;
    Ok(Json(v))
}

pub async fn put_logger(
    State(state): State<AppState>,
    Path(logger): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let path = format!("/admin/loggers/{}", urlencoding::encode(&logger));
    let v = connect_request(&state, Method::PUT, &path, Some(body)).await?;
    Ok(Json(v))
}

pub async fn list_rules(State(state): State<AppState>) -> Result<Json<Vec<AlertRule>>, AppError> {
    if let Some(rq) = &state.rqlite {
        return Ok(Json(rq.list_json::<AlertRule>(KIND_RULE).await?));
    }
    Err(AppError::BadRequest("rqlite is not configured".into()))
}

pub async fn get_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AlertRule>, AppError> {
    if let Some(rq) = &state.rqlite {
        return rq
            .get_json::<AlertRule>(KIND_RULE, &id)
            .await?
            .map(Json)
            .ok_or_else(|| AppError::NotFound(format!("rule not found: {id}")));
    }
    Err(AppError::BadRequest("rqlite is not configured".into()))
}

pub async fn create_rule(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut rule): Json<AlertRule>,
) -> Result<Json<Value>, AppError> {
    if rule.id.trim().is_empty() {
        rule.id = Uuid::new_v4().to_string();
    }
    if rule.created_at.is_none() {
        rule.created_at = Some(Utc::now());
    }
    if let Some(rq) = &state.rqlite {
        rq.put_json(KIND_RULE, &rule.id, &rule).await?;
    } else {
        return Err(AppError::BadRequest("rqlite is not configured".into()));
    }
    let actor = headers
        .get("x-user")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("system");
    log_action(actor, "notification.rule.create", "success", &rule.id);
    Ok(Json(json!({ "id": rule.id, "status": true })))
}

pub async fn update_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(mut rule): Json<AlertRule>,
) -> Result<Json<Value>, AppError> {
    rule.id = id.clone();
    if let Some(rq) = &state.rqlite {
        rq.put_json(KIND_RULE, &id, &rule).await?;
    } else {
        return Err(AppError::BadRequest("rqlite is not configured".into()));
    }
    let actor = headers
        .get("x-user")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("system");
    log_action(actor, "notification.rule.update", "success", &id);
    Ok(Json(json!({ "status": true })))
}

pub async fn delete_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    if let Some(rq) = &state.rqlite {
        rq.delete(KIND_RULE, &id).await?;
    } else {
        return Err(AppError::BadRequest("rqlite is not configured".into()));
    }
    let actor = headers
        .get("x-user")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("system");
    log_action(actor, "notification.rule.delete", "success", &id);
    Ok(Json(json!({ "status": true })))
}

#[derive(Debug, Deserialize)]
pub struct HistoryLim {
    #[serde(default = "default_hist")]
    pub limit: usize,
}

fn default_hist() -> usize {
    500
}

pub async fn alert_history(
    State(state): State<AppState>,
    Query(q): Query<HistoryLim>,
) -> Json<Vec<HistoryEntry>> {
    let hist = state.history.read().await.list_recent_owned();
    let cap = q.limit.clamp(1, 500);
    Json(hist.into_iter().take(cap).collect())
}

#[derive(Debug, serde::Deserialize)]
pub struct AuditEventsQuery {
    #[serde(default = "default_audit_limit")]
    pub limit: usize,
    pub action: Option<String>,
    pub profile_id: Option<String>,
    pub outcome: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
}

fn default_audit_limit() -> usize {
    100
}

pub async fn list_audit_events(
    State(state): State<AppState>,
    Query(q): Query<AuditEventsQuery>,
) -> Result<Json<serde_json::Value>, crate::error::AppError> {
    let Some(rqlite) = state.rqlite.as_ref() else {
        return Err(crate::error::AppError::BadRequest(
            "audit database requires ORCH_RQLITE_URL".into(),
        ));
    };
    let rows = rqlite
        .list_audit_events(
            q.limit,
            q.action.as_deref().filter(|s| !s.is_empty()),
            q.profile_id.as_deref().filter(|s| !s.is_empty()),
            q.outcome.as_deref().filter(|s| !s.is_empty()),
            q.since.as_deref().filter(|s| !s.is_empty()),
            q.until.as_deref().filter(|s| !s.is_empty()),
        )
        .await
        .map_err(|e| crate::error::AppError::Upstream(e.to_string()))?;
    Ok(Json(serde_json::json!({
        "items": rows,
        "count": rows.len(),
        "store": "rqlite",
        "table": "cdc_audit_events",
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct PipelineLogsQuery {
    #[serde(default = "default_audit_limit")]
    pub limit: usize,
    pub action: Option<String>,
    pub profile_id: Option<String>,
    pub outcome: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub level: Option<String>,
    pub phase: Option<String>,
}

pub async fn list_pipeline_logs(
    State(state): State<AppState>,
    Query(q): Query<PipelineLogsQuery>,
) -> Result<Json<Value>, AppError> {
    let Some(rqlite) = state.rqlite.as_ref() else {
        return Err(AppError::BadRequest(
            "pipeline logs require ORCH_RQLITE_URL".into(),
        ));
    };
    let rows = rqlite
        .list_pipeline_logs(
            q.limit,
            q.action.as_deref().filter(|s| !s.is_empty()),
            q.profile_id.as_deref().filter(|s| !s.is_empty()),
            q.outcome.as_deref().filter(|s| !s.is_empty()),
            q.since.as_deref().filter(|s| !s.is_empty()),
            q.until.as_deref().filter(|s| !s.is_empty()),
            q.level.as_deref().filter(|s| !s.is_empty()),
            q.phase.as_deref().filter(|s| !s.is_empty()),
        )
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;
    Ok(Json(json!({
        "items": rows,
        "count": rows.len(),
        "store": "rqlite",
        "table": "cdc_pipeline_logs",
    })))
}

pub async fn profile_pipeline_logs(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<PipelineLogsQuery>,
) -> Result<Json<Value>, AppError> {
    let Some(rqlite) = state.rqlite.as_ref() else {
        return Err(AppError::BadRequest(
            "pipeline logs require ORCH_RQLITE_URL".into(),
        ));
    };
    let rows = rqlite
        .list_pipeline_logs(
            q.limit,
            q.action.as_deref().filter(|s| !s.is_empty()),
            Some(&id.to_string()),
            q.outcome.as_deref().filter(|s| !s.is_empty()),
            q.since.as_deref().filter(|s| !s.is_empty()),
            q.until.as_deref().filter(|s| !s.is_empty()),
            q.level.as_deref().filter(|s| !s.is_empty()),
            q.phase.as_deref().filter(|s| !s.is_empty()),
        )
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;
    Ok(Json(json!({
        "items": rows,
        "count": rows.len(),
        "profile_id": id,
        "store": "rqlite",
        "table": "cdc_pipeline_logs",
    })))
}

pub async fn profile_table_sync_status(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<crate::profile_table_status::TableSyncStatusQuery>,
) -> Result<Json<crate::profile_table_status::ProfileTableSyncStatusResponse>, AppError> {
    let resp = crate::profile_table_status::profile_table_sync_status(&state, id, q).await?;
    Ok(Json(resp))
}

pub async fn profile_investigate(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let mut profile = state
        .profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("profile not found: {id}")))?;
    crate::incremental_cdc::hydrate_upsert_keys_from_mappings(&mut profile);

    let source_status = if let Some(name) = profile.connector_name.as_deref() {
        connector_status_json(&state, name).await
    } else {
        json!(null)
    };
    let sink_status = if let Some(name) = profile.sink_connector_name.as_deref() {
        connector_status_json(&state, name).await
    } else {
        json!(null)
    };
    let source_offsets = if let Some(name) = profile.connector_name.as_deref() {
        connector_offsets_json(&state, name).await
    } else {
        json!(null)
    };

    let pipeline_logs = if let Some(rqlite) = state.rqlite.as_ref() {
        rqlite
            .list_pipeline_logs(
                100,
                None,
                Some(&id.to_string()),
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    profile.redact_secrets();
    Ok(Json(json!({
        "timestamp": Utc::now(),
        "profile": crate::audit::profile_snapshot(&profile),
        "connectors": {
            "source": { "name": profile.connector_name, "status": source_status, "offsets": source_offsets },
            "sink": { "name": profile.sink_connector_name, "status": sink_status },
        },
        "recent_pipeline_logs": pipeline_logs,
        "profile_history": profile.history,
    })))
}

async fn connector_status_json(state: &AppState, name: &str) -> Value {
    let path = format!("/connectors/{}/status", urlencoding::encode(name));
    match connect_request(state, Method::GET, &path, None).await {
        Ok(v) => v,
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn connector_offsets_json(state: &AppState, name: &str) -> Value {
    let path = format!("/connectors/{}/offsets", urlencoding::encode(name));
    match connect_request(state, Method::GET, &path, None).await {
        Ok(v) => v,
        Err(e) => json!({ "error": e.to_string() }),
    }
}

pub async fn ws_alerts(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |sock| relay(sock, state))
}

async fn relay(mut socket: WebSocket, state: AppState) {
    let mut rx = state.alerts_tx.subscribe();
    loop {
        match rx.recv().await {
            Ok(msg) => {
                if socket.send(Message::Text(msg)).await.is_err() {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}

pub async fn metrics(State(state): State<AppState>) -> Response {
    let snaps = state.connector_snapshots.read().await;
    let mut running = 0u64;
    let mut failed = 0u64;
    let mut paused = 0u64;
    for s in snaps.values() {
        match s.state.as_str() {
            "RUNNING" => running += 1,
            "FAILED" => failed += 1,
            "PAUSED" => paused += 1,
            _ => {}
        }
    }
    let watcher_errors = *state.watcher_errors.read().await;
    let history_len = state.history.read().await.list_recent_owned().len();
    let body = format!(
        "# HELP cdc_connectors_total Connectors by state\n\
         # TYPE cdc_connectors_total gauge\n\
         cdc_connectors_total{{state=\"running\"}} {running}\n\
         cdc_connectors_total{{state=\"failed\"}} {failed}\n\
         cdc_connectors_total{{state=\"paused\"}} {paused}\n\
         # HELP cdc_watcher_errors_total Watcher tick failures\n\
         # TYPE cdc_watcher_errors_total counter\n\
         cdc_watcher_errors_total {watcher_errors}\n\
         # HELP cdc_alert_history_entries In-memory alert history size\n\
         # TYPE cdc_alert_history_entries gauge\n\
         cdc_alert_history_entries {history_len}\n"
    );
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}
