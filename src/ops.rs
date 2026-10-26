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
