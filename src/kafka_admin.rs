//! Kafbat UI v1.5.0-parity Kafka admin APIs (brokers, ACLs, schemas, ksql, produce, groups).

use crate::audit::log_action;
use crate::command::{run_docker, run_docker_stdin};
use crate::error::AppError;
use crate::models::{
    AclUpsertRequest, ActionResponse, GroupLagItem, KsqlDropRequest, KsqlQueryRequest,
    KsqlTerminateRequest, ProduceMessageRequest, ResetOffsetsRequest, SchemaCompatibilityRequest,
    SchemaCreateRequest, SchemaUpdateRequest, SetCompatibilityRequest,
};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

#[derive(Debug, Serialize, Clone)]
pub struct BrokerItem {
    pub id: i32,
    pub host: String,
    pub port: i32,
    pub controller: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_size: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_count: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct BrokersResponse {
    pub items: Vec<BrokerItem>,
    pub controller_id: Option<i32>,
    pub cluster_id: Option<String>,
    pub version: Option<String>,
    pub broker_count: usize,
    pub timestamp: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct BrokerDetailResponse {
    pub broker: BrokerItem,
    pub segment_size: Option<i64>,
    pub segment_count: Option<i64>,
    pub configs: Vec<ConfigEntry>,
    pub logdirs: Vec<BrokerLogdir>,
    pub raw: String,
    pub timestamp: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ConfigEntry {
    /// Redpanda cluster-config name (underscores). Use this for `PUT`.
    pub name: String,
    /// Kafka-style dotted name shown in Kafbat UI.
    pub display_name: String,
    /// Raw Redpanda value (JSON strings for listeners / directories).
    pub value: String,
    /// Human-readable value (listener URIs, `128 MB`, `604800000 ms`).
    pub display_value: String,
    pub source: Option<String>,
}

impl ConfigEntry {
    fn new(name: impl Into<String>, value: impl Into<String>, source: Option<String>) -> Self {
        let name = name.into();
        let value = value.into();
        let display_name = kafka_display_name(&name);
        let display_value = format_config_display_value(&display_name, &value);
        Self {
            name,
            display_name,
            value,
            display_value,
            source,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct BrokerLogdirPartition {
    pub partition: i32,
    pub size: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_lag: Option<i64>,
}

#[derive(Debug, Serialize, Clone)]
pub struct BrokerLogdirTopic {
    pub name: String,
    pub partitions: Vec<BrokerLogdirPartition>,
}

#[derive(Debug, Serialize, Clone)]
pub struct BrokerLogdir {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub topics_count: usize,
    pub partitions_count: usize,
    pub size: i64,
    pub topics: Vec<BrokerLogdirTopic>,
}

#[derive(Debug, Serialize)]
pub struct BrokerLogdirsResponse {
    pub broker_id: i32,
    pub items: Vec<BrokerLogdir>,
    pub timestamp: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct BrokerConfigsResponse {
    pub broker_id: i32,
    pub items: Vec<ConfigEntry>,
    pub timestamp: chrono::DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct BrokerConfigUpdateRequest {
    pub name: Option<String>,
    pub value: Option<String>,
    pub configs: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize, Clone)]
pub struct MetricSample {
    pub name: String,
    pub labels: HashMap<String, String>,
    pub value: f64,
}

#[derive(Debug, Serialize)]
pub struct BrokerMetricsResponse {
    pub broker_id: i32,
    pub segment_size: Option<i64>,
    pub segment_count: Option<i64>,
    pub metrics: Vec<MetricSample>,
    pub node: Option<Value>,
    pub health: Option<Value>,
    pub timestamp: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct AclItem {
    pub principal: String,
    pub host: String,
    pub resource_type: String,
    pub resource_name: String,
    pub pattern_type: String,
    pub operation: String,
    pub permission: String,
}

#[derive(Debug, Serialize)]
pub struct AclsResponse {
    pub items: Vec<AclItem>,
    pub total: usize,
    pub configured: bool,
    pub timestamp: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct SchemaSubjectItem {
    pub subject: String,
    pub id: Option<i32>,
    pub version: Option<i32>,
    pub schema_type: Option<String>,
    pub compatibility: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SchemasResponse {
    pub items: Vec<SchemaSubjectItem>,
    pub total: usize,
    pub configured: bool,
    pub timestamp: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct SchemaDetailResponse {
    pub subject: String,
    pub id: Option<i32>,
    pub version: Option<i32>,
    pub schema_type: Option<String>,
    pub schema: String,
    pub versions: Vec<i32>,
    pub compatibility: Option<String>,
    pub timestamp: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct KsqlListResponse {
    pub configured: bool,
    pub items: Vec<Value>,
    pub timestamp: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ConsumerGroupDetailResponse {
    pub group: String,
    pub state: Option<String>,
    pub members: Option<i32>,
    pub coordinator: Option<String>,
    pub balancer: Option<String>,
    pub total_lag: i64,
    pub entries: Vec<GroupLagItem>,
    pub raw: String,
    pub timestamp: chrono::DateTime<Utc>,
}

async fn rpk(state: &AppState, extra: &[&str]) -> Result<String, AppError> {
    let mut args = vec![
        "exec".to_string(),
        state.config.redpanda_container.clone(),
        "rpk".to_string(),
    ];
    args.extend(extra.iter().map(|s| (*s).to_string()));
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_docker(&state.config, &refs).await
}

async fn rpk_stdin(state: &AppState, extra: &[&str], stdin: &str) -> Result<String, AppError> {
    let mut args = vec![
        "exec".to_string(),
        "-i".to_string(),
        state.config.redpanda_container.clone(),
        "rpk".to_string(),
    ];
    args.extend(extra.iter().map(|s| (*s).to_string()));
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_docker_stdin(&state.config, &refs, stdin).await
}

async fn docker_exec(state: &AppState, extra: &[&str]) -> Result<String, AppError> {
    let mut args = vec!["exec".to_string(), state.config.redpanda_container.clone()];
    args.extend(extra.iter().map(|s| (*s).to_string()));
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_docker(&state.config, &refs).await
}

async fn redpanda_admin(state: &AppState, path: &str) -> Result<String, AppError> {
    let url = format!("http://127.0.0.1:9644{path}");
    docker_exec(state, &["curl", "-fsS", "--max-time", "8", &url]).await
}

fn actor_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-user")
        .or_else(|| headers.get("x-actor"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("system")
        .to_string()
}

pub async fn list_brokers(
    State(state): State<AppState>,
) -> Result<Json<BrokersResponse>, AppError> {
    let raw = rpk(&state, &["cluster", "info"]).await?;
    let (cluster_id, mut items) = parse_cluster_info(&raw);
    apply_disk_usage(&mut items, &load_logdir_rows(&state, None).await);
    let controller_id = items.iter().find(|b| b.controller).map(|b| b.id);
    let version = rpk(&state, &["version"])
        .await
        .ok()
        .and_then(|v| v.lines().next().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty());
    Ok(Json(BrokersResponse {
        broker_count: items.len(),
        items,
        controller_id,
        cluster_id,
        version,
        timestamp: Utc::now(),
    }))
}

pub async fn get_broker(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> Result<Json<BrokerDetailResponse>, AppError> {
    let mut broker = load_broker(&state, id).await?;
    let rows = load_logdir_rows(&state, Some(id)).await;
    apply_disk_usage(std::slice::from_mut(&mut broker), &rows);
    let logdirs = aggregate_logdirs(&rows);
    let configs = load_cluster_configs(&state).await;
    Ok(Json(BrokerDetailResponse {
        segment_size: broker.segment_size,
        segment_count: broker.segment_count,
        broker,
        configs,
        logdirs,
        raw: String::new(),
        timestamp: Utc::now(),
    }))
}

pub async fn get_broker_logdirs(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> Result<Json<BrokerLogdirsResponse>, AppError> {
    let _broker = load_broker(&state, id).await?;
    let items = aggregate_logdirs(&load_logdir_rows(&state, Some(id)).await);
    Ok(Json(BrokerLogdirsResponse {
        broker_id: id,
        items,
        timestamp: Utc::now(),
    }))
}

pub async fn get_broker_configs(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> Result<Json<BrokerConfigsResponse>, AppError> {
    let _broker = load_broker(&state, id).await?;
    Ok(Json(BrokerConfigsResponse {
        broker_id: id,
        items: load_cluster_configs(&state).await,
        timestamp: Utc::now(),
    }))
}

pub async fn update_broker_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i32>,
    Json(payload): Json<BrokerConfigUpdateRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    let _broker = load_broker(&state, id).await?;
    let updates = collect_config_updates(&payload)?;
    let mut output = String::new();
    for (name, value) in &updates {
        let result = rpk(&state, &["cluster", "config", "set", name, value]).await?;
        if !result.trim().is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(result.trim());
        }
    }
    let names = updates
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let audit_id = log_action(
        &actor_from_headers(&headers),
        "broker.config.update",
        "success",
        &format!("broker={id} configs={names}"),
    );
    Ok(Json(ActionResponse {
        action: "broker.config.update".into(),
        status: "success".into(),
        message: if output.is_empty() {
            format!("Updated {names}")
        } else {
            output
        },
        audit_id,
        timestamp: Utc::now(),
    }))
}

pub async fn get_broker_metrics(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> Result<Json<BrokerMetricsResponse>, AppError> {
    let mut broker = load_broker(&state, id).await?;
    let rows = load_logdir_rows(&state, Some(id)).await;
    apply_disk_usage(std::slice::from_mut(&mut broker), &rows);
    let metrics = match redpanda_admin(&state, "/public_metrics").await {
        Ok(raw) => parse_prometheus_metrics(&raw),
        Err(_) => Vec::new(),
    };
    let node = match redpanda_admin(&state, &format!("/v1/brokers/{id}")).await {
        Ok(raw) => serde_json::from_str(&raw).ok(),
        Err(_) => None,
    };
    let health = match rpk(&state, &["cluster", "health"]).await {
        Ok(raw) => Some(parse_cluster_health(&raw)),
        Err(_) => None,
    };
    Ok(Json(BrokerMetricsResponse {
        broker_id: id,
        segment_size: broker.segment_size,
        segment_count: broker.segment_count,
        metrics,
        node,
        health,
        timestamp: Utc::now(),
    }))
}

async fn load_broker(state: &AppState, id: i32) -> Result<BrokerItem, AppError> {
    let raw = rpk(state, &["cluster", "info"]).await?;
    let (_cluster_id, items) = parse_cluster_info(&raw);
    items
        .into_iter()
        .find(|b| b.id == id)
        .ok_or_else(|| AppError::NotFound(format!("broker {id} not found")))
}

async fn load_logdir_rows(state: &AppState, broker_id: Option<i32>) -> Vec<LogdirRow> {
    let raw = if let Some(id) = broker_id {
        let id_s = id.to_string();
        rpk(
            state,
            &["cluster", "logdirs", "describe", "--broker", &id_s],
        )
        .await
    } else {
        rpk(state, &["cluster", "logdirs", "describe"]).await
    };
    raw.ok().map(|s| parse_logdir_rows(&s)).unwrap_or_default()
}

const SOURCE_STATIC: &str = "Static broker config";
const SOURCE_DEFAULT: &str = "Default config";

async fn load_cluster_configs(state: &AppState) -> Vec<ConfigEntry> {
    let mut items = if let Ok(raw) = redpanda_admin(state, "/v1/cluster_config").await {
        parse_cluster_config_json(&raw)
    } else {
        Vec::new()
    };
    if items.is_empty() {
        if let Ok(raw) = rpk(state, &["cluster", "config", "export", "-f", "/dev/stdout"]).await {
            items = parse_config_export(&raw);
        }
    }
    if items.is_empty() {
        items = parse_config_list(
            &rpk(state, &["cluster", "config", "list"])
                .await
                .unwrap_or_default(),
        );
    }

    let mut static_keys = json_object_keys(
        &redpanda_admin(state, "/v1/cluster_config?include_defaults=false")
            .await
            .unwrap_or_default(),
    );
    if let Ok(raw) = redpanda_admin(state, "/v1/node_config").await {
        merge_node_configs(&mut items, &mut static_keys, &raw);
    }
    apply_config_sources(&mut items, &static_keys);
    items.sort_by(static_config_first);
    items
}

fn merge_node_configs(items: &mut Vec<ConfigEntry>, static_keys: &mut HashSet<String>, raw: &str) {
    for item in parse_cluster_config_json(raw) {
        static_keys.insert(item.name.clone());
        if !items.iter().any(|existing| existing.name == item.name) {
            items.push(item);
        }
    }
}

fn static_rank(entry: &ConfigEntry) -> u8 {
    if entry.source.as_deref() == Some(SOURCE_STATIC) {
        0
    } else {
        1
    }
}

fn static_config_first(a: &ConfigEntry, b: &ConfigEntry) -> std::cmp::Ordering {
    static_rank(a)
        .cmp(&static_rank(b))
        .then_with(|| a.display_name.cmp(&b.display_name))
}

pub async fn list_acls(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<AclsResponse>, AppError> {
    match rpk(&state, &["security", "acl", "list", "--format", "json"]).await {
        Ok(raw) => {
            let mut items = parse_acl_list_json(&raw);
            filter_acl_items(&mut items, &query);
            let total = items.len();
            Ok(Json(AclsResponse {
                items,
                total,
                configured: true,
                timestamp: Utc::now(),
            }))
        }
        Err(e) => acl_list_unavailable(e),
    }
}

fn query_text(query: &HashMap<String, String>, key: &str) -> Option<String> {
    let value = query.get(key)?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn acl_matches_search(item: &AclItem, search: &str) -> bool {
    let principal = item.principal.to_ascii_lowercase();
    let resource = item.resource_name.to_ascii_lowercase();
    let host = item.host.to_ascii_lowercase();
    principal.contains(search) || resource.contains(search) || host.contains(search)
}

fn filter_acl_items(items: &mut Vec<AclItem>, query: &HashMap<String, String>) {
    if let Some(search) = query_text(query, "search") {
        let search = search.to_ascii_lowercase();
        items.retain(|item| acl_matches_search(item, &search));
    }
    if let Some(host) = query_text(query, "host") {
        items.retain(|item| item.host == host || item.host == "*");
    }
    if let Some(resource) = query_text(query, "resource") {
        items.retain(|item| item.resource_name == resource);
    }
    if let Some(rt) = query_text(query, "resource_type") {
        let rt = rt.to_ascii_lowercase();
        items.retain(|item| item.resource_type.to_ascii_lowercase() == rt);
    }
    if let Some(op) = query_text(query, "operation") {
        let op = op.to_ascii_lowercase();
        items.retain(|item| item.operation.to_ascii_lowercase() == op);
    }
    if let Some(perm) = query_text(query, "permission") {
        let perm = perm.to_ascii_lowercase();
        items.retain(|item| item.permission.to_ascii_lowercase() == perm);
    }
    if let Some(pt) = query_text(query, "pattern_type") {
        let pt = pt.to_ascii_lowercase();
        items.retain(|item| item.pattern_type.to_ascii_lowercase() == pt);
    }
}

fn acl_list_unavailable(err: AppError) -> Result<Json<AclsResponse>, AppError> {
    let msg = err.to_string_message().to_ascii_lowercase();
    if msg.contains("acl") || msg.contains("authorizer") || msg.contains("not enabled") {
        Ok(Json(AclsResponse {
            items: vec![],
            total: 0,
            configured: false,
            timestamp: Utc::now(),
        }))
    } else {
        Err(err)
    }
}

pub async fn create_acl(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<AclUpsertRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    let args = acl_command_args("create", &payload)?;
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let output = rpk(&state, &refs).await?;
    // Detect ERROR column in rpk tabular output
    if let Some(err_msg) = acl_output_error(&output) {
        return Err(AppError::Internal(format!("ACL create failed: {err_msg}")));
    }
    let audit_id = log_action(
        &actor_from_headers(&headers),
        "acl.create",
        "success",
        &payload.principal,
    );
    Ok(Json(ActionResponse {
        action: "acl.create".into(),
        status: "success".into(),
        message: "ACL created".into(),
        audit_id,
        timestamp: Utc::now(),
    }))
}

pub async fn delete_acl(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<AclUpsertRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    let mut args = acl_command_args("delete", &payload)?;
    args.push("--no-confirm".into());
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let output = rpk(&state, &refs).await?;
    if let Some(err_msg) = acl_output_error(&output) {
        return Err(AppError::Internal(format!("ACL delete failed: {err_msg}")));
    }
    let audit_id = log_action(
        &actor_from_headers(&headers),
        "acl.delete",
        "success",
        &payload.principal,
    );
    Ok(Json(ActionResponse {
        action: "acl.delete".into(),
        status: "success".into(),
        message: "ACL deleted".into(),
        audit_id,
        timestamp: Utc::now(),
    }))
}

/// Detect a non-empty ERROR column in rpk acl tabular output.
fn acl_output_error(raw: &str) -> Option<String> {
    let mut header: Vec<String> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let cols: Vec<&str> = trimmed.split_whitespace().collect();
        if cols[0].eq_ignore_ascii_case("PRINCIPAL") {
            header = cols.iter().map(|c| c.to_ascii_uppercase()).collect();
            continue;
        }
        if header.is_empty() {
            continue;
        }
        if let Some(err_val) = acl_error_cell(&header, &cols) {
            return Some(err_val);
        }
    }
    None
}

// ── Schema Registry REST API helpers ─────────────────────────────────────────

fn acl_error_cell(header: &[String], cols: &[&str]) -> Option<String> {
    let err_idx = header.iter().position(|h| h == "ERROR")?;
    let err_val = cols.get(err_idx)?;
    if err_val.is_empty() || *err_val == "-" {
        None
    } else {
        Some((*err_val).to_string())
    }
}

fn sr_base(state: &AppState) -> Option<String> {
    state
        .config
        .schema_registry_url
        .as_deref()
        .map(|u| u.trim_end_matches('/').to_string())
}

async fn sr_get(state: &AppState, path: &str) -> Result<Value, AppError> {
    let base = sr_base(state)
        .ok_or_else(|| AppError::Internal("Schema Registry not configured".into()))?;
    let client = HttpClient::new();
    let resp = client
        .get(format!("{base}{path}"))
        .header(
            "Accept",
            "application/vnd.schemaregistry.v1+json, application/json",
        )
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Schema Registry request failed: {e}")))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = body
            .get("message")
            .or_else(|| body.get("error_message"))
            .and_then(|v| v.as_str())
            .unwrap_or("Schema Registry error")
            .to_string();
        return Err(AppError::Internal(format!("SR {status}: {msg}")));
    }
    Ok(body)
}

async fn sr_post(state: &AppState, path: &str, body: Value) -> Result<Value, AppError> {
    let base = sr_base(state)
        .ok_or_else(|| AppError::Internal("Schema Registry not configured".into()))?;
    let client = HttpClient::new();
    let resp = client
        .post(format!("{base}{path}"))
        .header("Content-Type", "application/vnd.schemaregistry.v1+json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Schema Registry request failed: {e}")))?;
    let status = resp.status();
    let resp_body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = resp_body
            .get("message")
            .or_else(|| resp_body.get("error_message"))
            .and_then(|v| v.as_str())
            .unwrap_or("Schema Registry error")
            .to_string();
        return Err(AppError::Internal(format!("SR {status}: {msg}")));
    }
    Ok(resp_body)
}

async fn sr_put(state: &AppState, path: &str, body: Value) -> Result<Value, AppError> {
    let base = sr_base(state)
        .ok_or_else(|| AppError::Internal("Schema Registry not configured".into()))?;
    let client = HttpClient::new();
    let resp = client
        .put(format!("{base}{path}"))
        .header("Content-Type", "application/vnd.schemaregistry.v1+json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Schema Registry request failed: {e}")))?;
    let status = resp.status();
    let resp_body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = resp_body
            .get("message")
            .or_else(|| resp_body.get("error_message"))
            .and_then(|v| v.as_str())
            .unwrap_or("Schema Registry error")
            .to_string();
        return Err(AppError::Internal(format!("SR {status}: {msg}")));
    }
    Ok(resp_body)
}

async fn sr_delete(state: &AppState, path: &str) -> Result<Value, AppError> {
    let base = sr_base(state)
        .ok_or_else(|| AppError::Internal("Schema Registry not configured".into()))?;
    let client = HttpClient::new();
    let resp = client
        .delete(format!("{base}{path}"))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Schema Registry request failed: {e}")))?;
    let status = resp.status();
    let resp_body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = resp_body
            .get("message")
            .or_else(|| resp_body.get("error_message"))
            .and_then(|v| v.as_str())
            .unwrap_or("Schema Registry error")
            .to_string();
        return Err(AppError::Internal(format!("SR {status}: {msg}")));
    }
    Ok(resp_body)
}

fn sr_get_str(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(|s| s.to_string())
}

fn sr_get_i64(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| x.as_i64())
}

// ── Schema Registry handlers ──────────────────────────────────────────────────

pub async fn list_schemas(
    State(state): State<AppState>,
) -> Result<Json<SchemasResponse>, AppError> {
    if sr_base(&state).is_none() {
        return Ok(Json(SchemasResponse {
            items: vec![],
            total: 0,
            configured: false,
            timestamp: Utc::now(),
        }));
    }

    // GET /subjects?deleted=false
    let subjects_val = match sr_get(&state, "/subjects").await {
        Ok(v) => v,
        Err(_) => {
            return Ok(Json(SchemasResponse {
                items: vec![],
                total: 0,
                configured: false,
                timestamp: Utc::now(),
            }))
        }
    };

    let subjects: Vec<String> = match subjects_val.as_array() {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        None => vec![],
    };

    let mut items = Vec::with_capacity(subjects.len());
    for subject in subjects {
        let enc = urlencoding::encode(&subject);
        // GET /subjects/{subject}/versions/latest
        let detail = sr_get(&state, &format!("/subjects/{enc}/versions/latest"))
            .await
            .ok();
        let (id, version, schema_type) = if let Some(ref d) = detail {
            (
                sr_get_i64(d, "id").map(|v| v as i32),
                sr_get_i64(d, "version").map(|v| v as i32),
                sr_get_str(d, "schemaType").or_else(|| Some("AVRO".to_string())),
            )
        } else {
            (None, None, None)
        };

        // GET /config/{subject} for per-subject compatibility
        let compat_val = sr_get(&state, &format!("/config/{enc}")).await.ok();
        let compatibility = compat_val
            .as_ref()
            .and_then(|v| {
                v.get("compatibilityLevel")
                    .or_else(|| v.get("compatibility"))
            })
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        items.push(SchemaSubjectItem {
            subject,
            id,
            version,
            schema_type,
            compatibility,
        });
    }

    let total = items.len();
    Ok(Json(SchemasResponse {
        items,
        total,
        configured: true,
        timestamp: Utc::now(),
    }))
}

pub async fn get_schema(
    State(state): State<AppState>,
    Path(subject): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<SchemaDetailResponse>, AppError> {
    let version = query
        .get("version")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "latest".to_string());

    let enc = urlencoding::encode(&subject);

    // GET /subjects/{subject}/versions/{version}
    let detail = sr_get(&state, &format!("/subjects/{enc}/versions/{version}")).await?;

    let id = sr_get_i64(&detail, "id").map(|v| v as i32);
    let ver = sr_get_i64(&detail, "version").map(|v| v as i32);
    let schema_type = sr_get_str(&detail, "schemaType").or_else(|| Some("AVRO".to_string()));

    // The schema field is a JSON-encoded string in SR API
    let schema_str = detail
        .get("schema")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // GET /subjects/{subject}/versions — list all versions
    let versions_val = sr_get(&state, &format!("/subjects/{enc}/versions"))
        .await
        .unwrap_or(Value::Array(vec![]));
    let versions: Vec<i32> = versions_val
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_i64().map(|n| n as i32))
                .collect()
        })
        .unwrap_or_default();

    // GET /config/{subject}
    let compat_val = sr_get(&state, &format!("/config/{enc}")).await.ok();
    let compatibility = compat_val
        .as_ref()
        .and_then(|v| {
            v.get("compatibilityLevel")
                .or_else(|| v.get("compatibility"))
        })
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(Json(SchemaDetailResponse {
        subject,
        id,
        version: ver,
        schema_type,
        schema: schema_str,
        versions,
        compatibility,
        timestamp: Utc::now(),
    }))
}

pub async fn create_schema(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SchemaCreateRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    let subject = payload.subject.trim().to_string();
    if subject.is_empty() || payload.schema.trim().is_empty() {
        return Err(AppError::BadRequest(
            "subject and schema are required".into(),
        ));
    }
    let schema_type = payload
        .schema_type
        .as_deref()
        .unwrap_or("AVRO")
        .to_ascii_uppercase();

    let enc = urlencoding::encode(&subject);

    // POST /subjects/{subject}/versions
    let body = json!({
        "schema": payload.schema.trim(),
        "schemaType": schema_type,
    });
    let result = sr_post(&state, &format!("/subjects/{enc}/versions"), body).await?;
    let schema_id = result.get("id").and_then(|v| v.as_i64()).unwrap_or(0);

    // Optionally set compatibility
    if let Some(compat) = payload
        .compatibility
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        let _ = sr_put(
            &state,
            &format!("/config/{enc}"),
            json!({ "compatibility": compat }),
        )
        .await;
    }

    let audit_id = log_action(
        &actor_from_headers(&headers),
        "schema.create",
        "success",
        &subject,
    );
    Ok(Json(ActionResponse {
        action: "schema.create".into(),
        status: "success".into(),
        message: format!("created schema {subject} (id={schema_id})"),
        audit_id,
        timestamp: Utc::now(),
    }))
}

pub async fn update_schema(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(subject): Path<String>,
    Json(payload): Json<SchemaUpdateRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    if payload.schema.trim().is_empty() {
        return Err(AppError::BadRequest("schema is required".into()));
    }
    let schema_type = payload
        .schema_type
        .as_deref()
        .unwrap_or("AVRO")
        .to_ascii_uppercase();

    let enc = urlencoding::encode(&subject);

    let body = json!({
        "schema": payload.schema.trim(),
        "schemaType": schema_type,
    });
    let result = sr_post(&state, &format!("/subjects/{enc}/versions"), body).await?;
    let schema_id = result.get("id").and_then(|v| v.as_i64()).unwrap_or(0);

    let audit_id = log_action(
        &actor_from_headers(&headers),
        "schema.update",
        "success",
        &subject,
    );
    Ok(Json(ActionResponse {
        action: "schema.update".into(),
        status: "success".into(),
        message: format!("updated schema {subject} (id={schema_id})"),
        audit_id,
        timestamp: Utc::now(),
    }))
}

pub async fn delete_schema(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(subject): Path<String>,
) -> Result<Json<ActionResponse>, AppError> {
    let enc = urlencoding::encode(&subject);
    // Soft-delete first, then permanent
    let _ = sr_delete(&state, &format!("/subjects/{enc}")).await;
    let _ = sr_delete(&state, &format!("/subjects/{enc}?permanent=true")).await;

    let audit_id = log_action(
        &actor_from_headers(&headers),
        "schema.delete",
        "success",
        &subject,
    );
    Ok(Json(ActionResponse {
        action: "schema.delete".into(),
        status: "success".into(),
        message: format!("deleted schema {subject}"),
        audit_id,
        timestamp: Utc::now(),
    }))
}

// ── Schema Registry extras ────────────────────────────────────────────────────

/// POST /schemas/:subject/compatibility — check if a new schema is compatible
pub async fn check_schema_compatibility(
    State(state): State<AppState>,
    Path(subject): Path<String>,
    Json(payload): Json<SchemaCompatibilityRequest>,
) -> Result<Json<Value>, AppError> {
    if payload.schema.trim().is_empty() {
        return Err(AppError::BadRequest("schema is required".into()));
    }
    let schema_type = payload
        .schema_type
        .as_deref()
        .unwrap_or("AVRO")
        .to_ascii_uppercase();

    let enc = urlencoding::encode(&subject);
    let body = json!({
        "schema": payload.schema.trim(),
        "schemaType": schema_type,
    });
    // POST /compatibility/subjects/{subject}/versions/latest
    let result = sr_post(
        &state,
        &format!("/compatibility/subjects/{enc}/versions/latest"),
        body,
    )
    .await;

    let (compatible, message) = match result {
        Ok(v) => {
            let ok = v
                .get("is_compatible")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            (ok, v.to_string())
        }
        Err(e) => (false, e.to_string_message()),
    };
    Ok(Json(
        json!({ "compatible": compatible, "message": message }),
    ))
}

/// PUT /schemas/:subject/compatibility — set compatibility level for a subject
pub async fn set_schema_compatibility(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(subject): Path<String>,
    Json(payload): Json<SetCompatibilityRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    let level = payload.compatibility.trim().to_ascii_uppercase();
    if level.is_empty() {
        return Err(AppError::BadRequest(
            "compatibility level is required".into(),
        ));
    }
    let enc = urlencoding::encode(&subject);
    sr_put(
        &state,
        &format!("/config/{enc}"),
        json!({ "compatibility": level }),
    )
    .await?;
    let audit_id = log_action(
        &actor_from_headers(&headers),
        "schema.set_compatibility",
        "success",
        &subject,
    );
    Ok(Json(ActionResponse {
        action: "schema.set_compatibility".into(),
        status: "success".into(),
        message: format!("set compatibility to {level} for {subject}"),
        audit_id,
        timestamp: Utc::now(),
    }))
}

/// DELETE /schemas/:subject/versions/:version — delete a specific schema version
pub async fn delete_schema_version(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((subject, version)): Path<(String, String)>,
) -> Result<Json<ActionResponse>, AppError> {
    let enc = urlencoding::encode(&subject);
    // Soft-delete first, then permanent
    let _ = sr_delete(&state, &format!("/subjects/{enc}/versions/{version}")).await;
    let _ = sr_delete(
        &state,
        &format!("/subjects/{enc}/versions/{version}?permanent=true"),
    )
    .await;

    let audit_id = log_action(
        &actor_from_headers(&headers),
        "schema.delete_version",
        "success",
        &format!("{subject} v{version}"),
    );
    Ok(Json(ActionResponse {
        action: "schema.delete_version".into(),
        status: "success".into(),
        message: format!("deleted {subject} version {version}"),
        audit_id,
        timestamp: Utc::now(),
    }))
}

// ── KSQL DB ───────────────────────────────────────────────────────────────────

fn ksql_base(state: &AppState) -> Result<String, AppError> {
    state
        .config
        .ksql_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_end_matches('/').to_string())
        .ok_or_else(|| AppError::BadRequest("KSQL DB is not configured (set ORCH_KSQL_URL)".into()))
}

fn ksql_error_from_value(value: &Value) -> Option<String> {
    let from_obj = |obj: &Value| -> Option<String> {
        let is_err = obj.get("error_code").is_some()
            || obj.get("@type").and_then(|v| v.as_str()) == Some("statement_error");
        if !is_err {
            return None;
        }
        obj.get("message")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .or_else(|| obj.get("error_code").map(|c| format!("ksqlDB error {c}")))
    };
    if let Some(msg) = from_obj(value) {
        return Some(msg);
    }
    if let Some(arr) = value.as_array() {
        for item in arr {
            if let Some(msg) = from_obj(item) {
                return Some(msg);
            }
        }
    }
    None
}

fn extract_ksql_collection(value: &Value) -> Vec<Value> {
    if let Some(arr) = value.as_array() {
        if let Some(first) = arr.first() {
            if let Some(items) = ksql_named_collection(first) {
                return items;
            }
        }
        return arr.clone();
    }
    ksql_named_collection(value).unwrap_or_default()
}

fn ksql_named_collection(obj: &Value) -> Option<Vec<Value>> {
    const KEYS: &[&str] = &[
        "tables",
        "streams",
        "queries",
        "topics",
        "kafkaTopics",
        "functions",
        "connectors",
        "properties",
        "sourceDescriptions",
        "queryDescription",
    ];
    for key in KEYS {
        if let Some(arr) = obj.get(*key).and_then(|v| v.as_array()) {
            return Some(arr.clone());
        }
        if *key == "queryDescription" {
            if let Some(desc) = obj.get(*key) {
                return Some(vec![desc.clone()]);
            }
        }
    }
    None
}

fn first_ksql_statement(sql: &str) -> String {
    sql.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("--") && !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KsqlRoute {
    Statement,
    Pull,
    Push,
}

fn ksql_route(sql: &str) -> KsqlRoute {
    let upper = first_ksql_statement(sql).to_ascii_uppercase();
    if upper.starts_with("SELECT") || upper.starts_with("PRINT ") {
        if upper.contains("EMIT CHANGES") || upper.starts_with("PRINT ") {
            KsqlRoute::Push
        } else {
            KsqlRoute::Pull
        }
    } else {
        KsqlRoute::Statement
    }
}

fn ensure_push_limit(sql: &str) -> String {
    let trimmed = sql.trim().trim_end_matches(';').trim().to_string();
    let upper = trimmed.to_ascii_uppercase();
    let needs_limit = (upper.contains("EMIT CHANGES") || upper.starts_with("PRINT "))
        && !upper.contains(" LIMIT ");
    if needs_limit {
        format!("{trimmed} LIMIT 20;")
    } else if trimmed.ends_with(';') {
        trimmed
    } else {
        format!("{trimmed};")
    }
}

fn parse_ksql_schema(schema: &str) -> Vec<String> {
    schema
        .split(',')
        .map(|part| {
            let part = part.trim().trim_matches('`');
            part.split_whitespace()
                .next()
                .unwrap_or(part)
                .trim_matches('`')
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn rows_from_query_items(items: &[Value]) -> Value {
    let mut columns: Vec<String> = Vec::new();
    let mut rows: Vec<Value> = Vec::new();
    let mut query_id = Value::Null;
    for item in items {
        if let Some(header) = item.get("header") {
            query_id = header.get("queryId").cloned().unwrap_or(Value::Null);
            if let Some(cols) = header.get("columnNames").and_then(|c| c.as_array()) {
                columns = cols
                    .iter()
                    .filter_map(|c| c.as_str().map(ToString::to_string))
                    .collect();
            } else if let Some(schema) = header.get("schema").and_then(|s| s.as_str()) {
                columns = parse_ksql_schema(schema);
            }
        } else if let Some(row) = item.get("row") {
            let vals = row
                .get("columns")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            let mut obj = Map::new();
            for (i, val) in vals.into_iter().enumerate() {
                let key = columns.get(i).cloned().unwrap_or_else(|| format!("c{i}"));
                obj.insert(key, val);
            }
            rows.push(Value::Object(obj));
        }
    }
    json!({
        "kind": "query",
        "queryId": query_id,
        "columns": columns,
        "rows": rows,
        "rowCount": rows.len(),
    })
}

fn parse_query_stream_body(text: &str) -> Value {
    let items: Vec<Value> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let parsed = rows_from_query_items(&items);
    if parsed.get("rowCount").and_then(|v| v.as_u64()).unwrap_or(0) > 0
        || parsed
            .get("columns")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false)
    {
        parsed
    } else {
        json!({ "kind": "query", "raw": text, "rows": [], "columns": [] })
    }
}

fn normalize_ksql_result(value: Value) -> Value {
    if let Some(arr) = value.as_array() {
        if arr
            .iter()
            .any(|item| item.get("header").is_some() || item.get("row").is_some())
        {
            return rows_from_query_items(arr);
        }
        let items = extract_ksql_collection(&value);
        if !items.is_empty() {
            return json!({ "kind": "statement", "result": items });
        }
        return json!({ "kind": "statement", "result": value });
    }
    json!({ "kind": "statement", "result": value })
}

fn default_stream_props(extra: Option<HashMap<String, String>>) -> Map<String, Value> {
    let mut props = Map::new();
    props.insert(
        "ksql.streams.auto.offset.reset".into(),
        Value::String("earliest".into()),
    );
    if let Some(extra) = extra {
        for (k, v) in extra {
            props.insert(k, Value::String(v));
        }
    }
    props
}

async fn ksql_post_json(
    state: &AppState,
    path: &str,
    body: Value,
) -> Result<(reqwest::StatusCode, Value, String), AppError> {
    let base = ksql_base(state)?;
    let url = format!("{base}{path}");
    let resp = state
        .http
        .post(url)
        .timeout(Duration::from_secs(25))
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("ksqlDB unreachable: {e}")))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let value = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }));
    Ok((status, value, text))
}

pub async fn ksql_info(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let Ok(base) = ksql_base(&state) else {
        return Ok(Json(json!({ "configured": false, "reachable": false })));
    };
    let url = format!("{base}/info");
    match state
        .http
        .get(url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let value: Value = resp.json().await.unwrap_or(Value::Null);
            Ok(Json(json!({
                "configured": true,
                "reachable": true,
                "info": value,
            })))
        }
        Ok(resp) => Ok(Json(json!({
            "configured": true,
            "reachable": false,
            "status": resp.status().as_u16(),
        }))),
        Err(e) => Ok(Json(json!({
            "configured": true,
            "reachable": false,
            "error": e.to_string(),
        }))),
    }
}

pub async fn ksql_tables(
    State(state): State<AppState>,
) -> Result<Json<KsqlListResponse>, AppError> {
    ksql_show(&state, "SHOW TABLES;").await
}

pub async fn ksql_streams(
    State(state): State<AppState>,
) -> Result<Json<KsqlListResponse>, AppError> {
    ksql_show(&state, "SHOW STREAMS;").await
}

pub async fn ksql_query(
    State(state): State<AppState>,
    Json(payload): Json<KsqlQueryRequest>,
) -> Result<Json<Value>, AppError> {
    let ksql = payload.ksql.trim();
    if ksql.is_empty() {
        return Err(AppError::BadRequest("ksql statement is required".into()));
    }
    let props = default_stream_props(payload.streams_properties);
    match ksql_route(ksql) {
        KsqlRoute::Statement => {
            ksql_statement(
                &state,
                "/ksql",
                json!({ "ksql": ksql, "streamsProperties": props }),
            )
            .await
        }
        KsqlRoute::Pull => {
            ksql_statement(
                &state,
                "/query",
                json!({ "ksql": ksql, "streamsProperties": props }),
            )
            .await
        }
        KsqlRoute::Push => ksql_push(&state, ksql, props).await,
    }
}

async fn ksql_statement(
    state: &AppState,
    path: &str,
    body: Value,
) -> Result<Json<Value>, AppError> {
    let (status, value, _) = ksql_post_json(state, path, body).await?;
    if let Some(msg) = ksql_error_from_value(&value) {
        return Err(AppError::BadRequest(msg));
    }
    if !status.is_success() {
        return Err(AppError::Upstream(value.to_string()));
    }
    Ok(Json(normalize_ksql_result(value)))
}

async fn ksql_push(
    state: &AppState,
    ksql: &str,
    props: Map<String, Value>,
) -> Result<Json<Value>, AppError> {
    let sql = ensure_push_limit(ksql);
    let (status, value, text) = ksql_post_json(
        state,
        "/query-stream",
        json!({
            "sql": sql,
            "properties": props,
        }),
    )
    .await?;
    if status.as_u16() == 404 || status.as_u16() == 405 {
        return ksql_statement(
            state,
            "/query",
            json!({ "ksql": sql, "streamsProperties": props }),
        )
        .await;
    }
    if let Some(msg) = ksql_error_from_value(&value) {
        return Err(AppError::BadRequest(msg));
    }
    if !status.is_success() {
        return Err(AppError::Upstream(value.to_string()));
    }
    if text.lines().filter(|l| !l.trim().is_empty()).count() > 1 {
        Ok(Json(parse_query_stream_body(&text)))
    } else {
        Ok(Json(normalize_ksql_result(value)))
    }
}

/// GET /ksqldb/queries — list running persistent queries (SHOW QUERIES;)
pub async fn ksql_queries(
    State(state): State<AppState>,
) -> Result<Json<KsqlListResponse>, AppError> {
    ksql_show(&state, "SHOW QUERIES;").await
}

/// POST /ksqldb/terminate — terminate a persistent query
pub async fn ksql_terminate(
    State(state): State<AppState>,
    Json(payload): Json<KsqlTerminateRequest>,
) -> Result<Json<Value>, AppError> {
    let qid = payload.query_id.trim();
    if qid.is_empty() {
        return Err(AppError::BadRequest("query_id is required".into()));
    }
    let stmt = format!("TERMINATE {qid};");
    let (status, value, _) = ksql_post_json(&state, "/ksql", json!({ "ksql": stmt })).await?;
    if let Some(msg) = ksql_error_from_value(&value) {
        return Err(AppError::BadRequest(msg));
    }
    if !status.is_success() {
        return Err(AppError::Upstream(value.to_string()));
    }
    Ok(Json(json!({ "status": "terminated", "query_id": qid })))
}

/// POST /ksqldb/drop — drop a stream or table
pub async fn ksql_drop(
    State(state): State<AppState>,
    Json(payload): Json<KsqlDropRequest>,
) -> Result<Json<Value>, AppError> {
    let entity_type = payload.entity_type.trim().to_ascii_uppercase();
    let name = payload.name.trim();
    if entity_type.is_empty() || name.is_empty() {
        return Err(AppError::BadRequest(
            "entity_type and name are required".into(),
        ));
    }
    if entity_type != "STREAM" && entity_type != "TABLE" {
        return Err(AppError::BadRequest(
            "entity_type must be STREAM or TABLE".into(),
        ));
    }
    let delete_topic = payload.delete_topic.unwrap_or(false);
    let stmt = if delete_topic {
        format!("DROP {entity_type} {name} DELETE TOPIC;")
    } else {
        format!("DROP {entity_type} {name};")
    };
    let (status, value, _) = ksql_post_json(&state, "/ksql", json!({ "ksql": stmt })).await?;
    if let Some(msg) = ksql_error_from_value(&value) {
        return Err(AppError::BadRequest(msg));
    }
    if !status.is_success() {
        return Err(AppError::Upstream(value.to_string()));
    }
    Ok(Json(json!({ "status": "dropped", "name": name })))
}

/// GET /ksqldb/explain/:query_id — explain a persistent query
pub async fn ksql_explain(
    State(state): State<AppState>,
    Path(query_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let stmt = format!("EXPLAIN {};", query_id.trim());
    let (status, value, _) = ksql_post_json(&state, "/ksql", json!({ "ksql": stmt })).await?;
    if let Some(msg) = ksql_error_from_value(&value) {
        return Err(AppError::BadRequest(msg));
    }
    if !status.is_success() {
        return Err(AppError::Upstream(value.to_string()));
    }
    Ok(Json(value))
}

pub async fn produce_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(topic): Path<String>,
    Json(payload): Json<ProduceMessageRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    if payload.value.is_empty() {
        return Err(AppError::BadRequest("message value is required".into()));
    }
    let partition = payload.partition.map(|p| p.to_string());
    let mut extra = vec!["topic".to_string(), "produce".to_string(), topic.clone()];
    if let Some(key) = payload.key.as_deref().filter(|s| !s.is_empty()) {
        extra.push("-k".into());
        extra.push(key.to_string());
    }
    if let Some(p) = partition.as_deref() {
        extra.push("-p".into());
        extra.push(p.to_string());
    }
    if let Some(hdrs) = &payload.headers {
        for h in hdrs {
            if h.name.trim().is_empty() {
                continue;
            }
            extra.push("-H".into());
            extra.push(format!("{}:{}", h.name.trim(), h.value));
        }
    }
    let refs: Vec<&str> = extra.iter().map(|s| s.as_str()).collect();
    let output = rpk_stdin(&state, &refs, &payload.value).await?;
    let audit_id = log_action(
        &actor_from_headers(&headers),
        "topic.produce",
        "success",
        &topic,
    );
    Ok(Json(ActionResponse {
        action: "topic.produce".into(),
        status: "success".into(),
        message: if output.is_empty() {
            format!("produced message to {topic}")
        } else {
            output
        },
        audit_id,
        timestamp: Utc::now(),
    }))
}

/// For `connect-{name}` consumer groups, the JDBC sink connector may have subscribed
/// to topics whose offsets haven't been committed yet (lag = full high-watermark).
/// `rpk group describe` silently omits those. This function fetches the full topic
/// list from the Kafka Connect REST API and adds missing topics to the entries.
pub async fn augment_connect_group(
    state: &AppState,
    group: &str,
    entries: &mut Vec<GroupLagItem>,
    total_lag: &mut i64,
) {
    let connector_name = match group.strip_prefix("connect-") {
        Some(n) => n,
        None => return,
    };
    let url = format!(
        "{}/connectors/{}/topics",
        state.config.connect_base_url, connector_name
    );
    let resp = match state.http.get(&url).send().await {
        Ok(r) => r,
        Err(_) => return,
    };
    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => return,
    };
    // Response shape: { "connector_name": { "topics": ["t1","t2",...] } }
    let topics: Vec<String> = body
        .as_object()
        .and_then(|m| m.values().next())
        .and_then(|v| v.get("topics"))
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let committed: std::collections::HashSet<String> =
        entries.iter().map(|e| e.topic.clone()).collect();

    for topic in topics {
        if committed.contains(&topic) {
            continue;
        }
        append_uncommitted_topic(state, &topic, entries, total_lag).await;
    }
}

async fn append_uncommitted_topic(
    state: &AppState,
    topic: &str,
    entries: &mut Vec<GroupLagItem>,
    total_lag: &mut i64,
) {
    let raw = match rpk(state, &["topic", "describe", "-p", topic]).await {
        Ok(r) => r,
        Err(_) => return,
    };
    for line in raw.lines() {
        if let Some(item) = lag_item_from_describe_line(topic, line) {
            *total_lag += item.lag;
            entries.push(item);
        }
    }
}

fn lag_item_from_describe_line(topic: &str, line: &str) -> Option<GroupLagItem> {
    let cols: Vec<&str> = line.split_whitespace().collect();
    if cols.len() < 6 {
        return None;
    }
    let partition = cols[0].parse::<i32>().ok()?;
    let high_watermark = cols[5].parse::<i64>().ok()?;
    Some(GroupLagItem {
        topic: topic.to_string(),
        partition,
        current_offset: 0,
        log_end_offset: high_watermark,
        lag: high_watermark,
        consumer_id: None,
        host: None,
    })
}

pub async fn get_consumer_group(
    State(state): State<AppState>,
    Path(group): Path<String>,
) -> Result<Json<ConsumerGroupDetailResponse>, AppError> {
    let raw = rpk(&state, &["group", "describe", group.as_str()]).await?;
    let mut parsed = parse_group_describe(&raw);
    augment_connect_group(&state, &group, &mut parsed.entries, &mut parsed.total_lag).await;
    Ok(Json(ConsumerGroupDetailResponse {
        group,
        state: parsed.state,
        members: parsed.members,
        coordinator: parsed.coordinator,
        balancer: parsed.balancer,
        total_lag: parsed.total_lag,
        entries: parsed.entries,
        raw,
        timestamp: Utc::now(),
    }))
}

pub async fn delete_consumer_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(group): Path<String>,
) -> Result<Json<ActionResponse>, AppError> {
    let output = rpk_group_call(&state, &["group", "delete", group.as_str()], &group)
        .await
        .inspect_err(|_| {
            log_action(
                &actor_from_headers(&headers),
                "group.delete",
                "error",
                &group,
            );
        })?;
    let audit_id = log_action(
        &actor_from_headers(&headers),
        "group.delete",
        "success",
        &group,
    );
    Ok(Json(ActionResponse {
        action: "group.delete".into(),
        status: "success".into(),
        message: if output.is_empty() {
            format!("deleted consumer group {group}")
        } else {
            output
        },
        audit_id,
        timestamp: Utc::now(),
    }))
}

/// DELETE /consumer-groups/{group}/topics/{topic}/offsets
/// Forcefully deletes committed offsets for a specific topic within a consumer group.
/// The broker requires the group to be in a Dead state or the topic/partitions to be unassigned.
/// Collect per-row failures that rpk prints in a trailing `ERROR` column.
///
/// `rpk group seek` and `rpk group offset-delete` exit 0 even when every partition
/// failed, reporting the reason inside the table instead. Without reading that column
/// the API reports success for an operation that did nothing.
fn rpk_row_errors(output: &str) -> Vec<String> {
    let mut lines = output.lines().filter(|line| !line.trim().is_empty());
    let Some(header) = lines.next() else {
        return Vec::new();
    };
    let cols: Vec<&str> = header.split_whitespace().collect();
    // `rpk group seek` / `offset-delete` report failures in a trailing ERROR column.
    // `rpk group delete` reports them in a STATUS column (value is "OK" on success).
    // Prefer ERROR when both exist so per-partition tables are read correctly.
    let (error_idx, is_status) = match cols.iter().position(|c| c.eq_ignore_ascii_case("ERROR")) {
        Some(idx) => (idx, false),
        None => match cols.iter().position(|c| c.eq_ignore_ascii_case("STATUS")) {
            Some(idx) => (idx, true),
            None => return Vec::new(),
        },
    };
    let mut errors: Vec<String> = Vec::new();
    for line in lines {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() <= error_idx {
            continue;
        }
        // The message is free text, so it runs from the column to end of line.
        let message = parts[error_idx..].join(" ").trim().to_string();
        // A STATUS of "OK" (delete succeeded) is not an error.
        if is_status && message.eq_ignore_ascii_case("OK") {
            continue;
        }
        if !message.is_empty() && !errors.contains(&message) {
            errors.push(message);
        }
    }
    errors
}

/// Strip the leading table columns (topic, partition, …) that rpk puts in front of a
/// broker error so only the error itself is shown.
fn strip_rpk_row_prefix(raw: &str) -> String {
    let is_error_code = |token: &str| {
        token.len() > 1
            && token.ends_with(':')
            && token[..token.len() - 1]
                .chars()
                .all(|c| c.is_ascii_uppercase() || c == '_')
    };
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            match tokens.iter().position(|t| is_error_code(t)) {
                Some(idx) => tokens[idx..].join(" "),
                None => line.trim().to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Run an rpk group command, treating both exit-code failures and the errors rpk
/// reports inside its table as errors.
async fn rpk_group_call(state: &AppState, args: &[&str], group: &str) -> Result<String, AppError> {
    let output = match rpk(state, args).await {
        Ok(output) => output,
        Err(AppError::Upstream(raw)) => {
            let cleaned = strip_rpk_row_prefix(&raw);
            return Err(AppError::BadRequest(group_error_message(group, &[cleaned])));
        }
        Err(other) => return Err(other),
    };
    let errors = rpk_row_errors(&output);
    if errors.is_empty() {
        Ok(output)
    } else {
        Err(AppError::BadRequest(group_error_message(group, &errors)))
    }
}

/// Turn rpk's raw partition errors into something the operator can act on.
fn group_error_message(group: &str, errors: &[String]) -> String {
    let joined = errors.join("; ");
    let upper = joined.to_ascii_uppercase();
    if upper.contains("NON-EMPTY GROUP")
        || upper.contains("NON_EMPTY_GROUP")
        || upper.contains("GROUP_SUBSCRIBED_TO_TOPIC")
    {
        return format!(
            "{joined} — consumer group '{group}' still has active members. \
             Stop the consumer first (for a Kafka Connect sink: \
             PUT /connectors/<connector>/stop), then retry."
        );
    }
    joined
}

pub async fn delete_topic_offsets(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((group, topic)): Path<(String, String)>,
) -> Result<Json<ActionResponse>, AppError> {
    let output = rpk_group_call(
        &state,
        &[
            "group",
            "offset-delete",
            group.as_str(),
            "--topic",
            topic.as_str(),
        ],
        &group,
    )
    .await
    .inspect_err(|_| {
        log_action(
            &actor_from_headers(&headers),
            "group.offset_delete",
            "error",
            &format!("{group}/{topic}"),
        );
    })?;
    let audit_id = log_action(
        &actor_from_headers(&headers),
        "group.offset_delete",
        "success",
        &format!("{group}/{topic}"),
    );
    Ok(Json(ActionResponse {
        action: "group.offset_delete".into(),
        status: "success".into(),
        message: if output.trim().is_empty() {
            format!("deleted offsets for topic '{topic}' in group '{group}'")
        } else {
            output
        },
        audit_id,
        timestamp: Utc::now(),
    }))
}

pub async fn reset_consumer_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(group): Path<String>,
    Json(payload): Json<ResetOffsetsRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    let reset_type = payload.reset_type.trim().to_ascii_lowercase();
    let to = match reset_type.as_str() {
        "earliest" | "start" => "start".to_string(),
        "latest" | "end" => "end".to_string(),
        "timestamp" => {
            let ts = payload.timestamp_ms.ok_or_else(|| {
                AppError::BadRequest("timestamp_ms is required for timestamp reset".into())
            })?;
            format!("timestamp://{ts}")
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "unsupported reset_type '{other}' (use earliest, latest, or timestamp)"
            )))
        }
    };
    let mut extra = vec![
        "group".to_string(),
        "seek".to_string(),
        group.clone(),
        "--to".to_string(),
        to,
    ];
    if let Some(topic) = payload
        .topic
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        extra.push("--topics".into());
        if let Some(parts) = payload.partitions.as_ref().filter(|p| !p.is_empty()) {
            extra.push(format!(
                "{}:{}",
                topic,
                parts
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        } else {
            extra.push(topic.to_string());
        }
    }
    let refs: Vec<&str> = extra.iter().map(|s| s.as_str()).collect();
    let output = rpk_group_call(&state, &refs, &group)
        .await
        .inspect_err(|_| {
            log_action(
                &actor_from_headers(&headers),
                "group.reset",
                "error",
                &group,
            );
        })?;
    let audit_id = log_action(
        &actor_from_headers(&headers),
        "group.reset",
        "success",
        &group,
    );
    Ok(Json(ActionResponse {
        action: "group.reset".into(),
        status: "success".into(),
        message: if output.is_empty() {
            format!("reset offsets for {group}")
        } else {
            output
        },
        audit_id,
        timestamp: Utc::now(),
    }))
}

async fn ksql_show(state: &AppState, statement: &str) -> Result<Json<KsqlListResponse>, AppError> {
    if ksql_base(state).is_err() {
        return Ok(Json(KsqlListResponse {
            configured: false,
            items: vec![],
            timestamp: Utc::now(),
        }));
    }
    let (status, value, _) = ksql_post_json(state, "/ksql", json!({ "ksql": statement })).await?;
    if let Some(msg) = ksql_error_from_value(&value) {
        return Err(AppError::Upstream(msg));
    }
    if !status.is_success() {
        return Err(AppError::Upstream(value.to_string()));
    }
    Ok(Json(KsqlListResponse {
        configured: true,
        items: extract_ksql_collection(&value),
        timestamp: Utc::now(),
    }))
}

fn acl_command_args(action: &str, payload: &AclUpsertRequest) -> Result<Vec<String>, AppError> {
    let principal = payload.principal.trim();
    let resource_type = payload.resource_type.trim().to_ascii_lowercase();
    let resource_name = payload.resource_name.trim();
    let operation = payload.operation.trim().to_ascii_lowercase();
    let permission = payload.permission.trim().to_ascii_lowercase();
    if principal.is_empty() || resource_type.is_empty() || operation.is_empty() {
        return Err(AppError::BadRequest(
            "principal, resource_type, and operation are required".into(),
        ));
    }
    let host = payload
        .host
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("*");
    let pattern = payload
        .pattern_type
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("literal");
    let mut args = vec![
        "security".into(),
        "acl".into(),
        action.into(),
        "--operation".into(),
        operation,
        "--resource-pattern-type".into(),
        pattern.to_string(),
    ];
    if permission == "deny" {
        args.push("--deny-principal".into());
        args.push(principal.to_string());
        args.push("--deny-host".into());
        args.push(host.to_string());
    } else {
        args.push("--allow-principal".into());
        args.push(principal.to_string());
        args.push("--allow-host".into());
        args.push(host.to_string());
    }
    match resource_type.as_str() {
        "topic" => {
            args.push("--topic".into());
            args.push(resource_name.to_string());
        }
        "group" => {
            args.push("--group".into());
            args.push(resource_name.to_string());
        }
        "transactional_id" | "transactional-id" => {
            args.push("--transactional-id".into());
            args.push(resource_name.to_string());
        }
        "cluster" => args.push("--cluster".into()),
        "delegation_token" | "delegationtoken" => {
            args.push("--delegation-token".into());
            args.push(resource_name.to_string());
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "unsupported resource_type '{other}'"
            )))
        }
    }
    Ok(args)
}

pub fn parse_cluster_info(raw: &str) -> (Option<String>, Vec<BrokerItem>) {
    let mut cluster_id = None;
    let mut items = Vec::new();
    let mut section = "";
    let mut header: Vec<String> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if skip_cluster_line(trimmed) {
            continue;
        }
        if let Some(next) = cluster_section_name(trimmed) {
            section = next;
            continue;
        }
        if note_cluster_id(&mut cluster_id, section, trimmed) {
            continue;
        }
        if section != "brokers" {
            continue;
        }
        let cols: Vec<&str> = trimmed.split_whitespace().collect();
        if let Some(item) = broker_from_cols(&cols, &mut header) {
            items.push(item);
        }
    }
    (cluster_id, items)
}

fn skip_cluster_line(trimmed: &str) -> bool {
    trimmed.is_empty() || trimmed.chars().all(|c| c == '=' || c == '-')
}

fn cluster_section_name(trimmed: &str) -> Option<&'static str> {
    match trimmed.to_ascii_uppercase().as_str() {
        "CLUSTER" => Some("cluster"),
        "BROKERS" => Some("brokers"),
        _ => None,
    }
}

fn note_cluster_id(cluster_id: &mut Option<String>, section: &str, trimmed: &str) -> bool {
    if section == "cluster" && !trimmed.contains(' ') {
        *cluster_id = Some(trimmed.to_string());
        true
    } else {
        false
    }
}

fn broker_from_cols(cols: &[&str], header: &mut Vec<String>) -> Option<BrokerItem> {
    if cols.is_empty() {
        return None;
    }
    if cols[0].eq_ignore_ascii_case("ID") {
        *header = cols.iter().map(|c| c.to_ascii_uppercase()).collect();
        return None;
    }
    let id = cols[0].trim_end_matches('*').parse::<i32>().ok()?;
    let host_idx = header.iter().position(|h| h == "HOST").unwrap_or(1);
    let port_idx = header.iter().position(|h| h == "PORT").unwrap_or(2);
    Some(BrokerItem {
        id,
        host: cols.get(host_idx).unwrap_or(&"").to_string(),
        port: cols
            .get(port_idx)
            .and_then(|v| v.parse().ok())
            .unwrap_or(9092),
        controller: cols[0].ends_with('*'),
        segment_size: None,
        segment_count: None,
    })
}

pub fn parse_config_list(raw: &str) -> Vec<ConfigEntry> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let cols: Vec<&str> = trimmed.split_whitespace().collect();
        if cols.len() < 2
            || cols[0].eq_ignore_ascii_case("KEY")
            || cols[0].eq_ignore_ascii_case("NAME")
        {
            continue;
        }
        out.push(ConfigEntry::new(
            cols[0],
            cols[1],
            Some(humanize_config_source(cols.get(2).copied())),
        ));
    }
    out
}

#[derive(Debug, Clone)]
struct LogdirRow {
    broker: i32,
    dir: String,
    topic: String,
    partition: i32,
    size: i64,
    offset_lag: Option<i64>,
    error: Option<String>,
}

fn apply_disk_usage(items: &mut [BrokerItem], rows: &[LogdirRow]) {
    let mut size: HashMap<i32, i64> = HashMap::new();
    let mut count: HashMap<i32, i64> = HashMap::new();
    for row in rows {
        *size.entry(row.broker).or_default() += row.size;
        if !row.topic.is_empty() {
            *count.entry(row.broker).or_default() += 1;
        }
    }
    for item in items {
        if let Some(bytes) = size.get(&item.id).copied() {
            item.segment_size = Some(bytes);
        }
        if let Some(segments) = count.get(&item.id).copied() {
            item.segment_count = Some(segments);
        }
    }
}

fn collect_config_updates(
    payload: &BrokerConfigUpdateRequest,
) -> Result<Vec<(String, String)>, AppError> {
    let mut out = Vec::new();
    if let Some(map) = &payload.configs {
        for (name, value) in map {
            if !name.trim().is_empty() {
                out.push((resolve_redpanda_config_name(name), value.clone()));
            }
        }
    }
    if let Some(name) = payload
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push((
            resolve_redpanda_config_name(name),
            payload.value.clone().unwrap_or_default(),
        ));
    }
    if out.is_empty() {
        return Err(AppError::BadRequest(
            "config name and value are required".into(),
        ));
    }
    Ok(out)
}

fn parse_logdir_rows(raw: &str) -> Vec<LogdirRow> {
    let mut header: Vec<String> = Vec::new();
    let mut rows = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let cols: Vec<&str> = trimmed.split_whitespace().collect();
        if cols.is_empty() {
            continue;
        }
        if cols[0].eq_ignore_ascii_case("BROKER") {
            header = cols.iter().map(|c| c.to_ascii_uppercase()).collect();
            continue;
        }
        if header.is_empty() {
            continue;
        }
        let get = |name: &str| {
            header
                .iter()
                .position(|h| h == name)
                .and_then(|idx| cols.get(idx).copied())
        };
        let Some(broker) = get("BROKER").and_then(|v| v.parse().ok()) else {
            continue;
        };
        rows.push(LogdirRow {
            broker,
            dir: get("DIR").unwrap_or("").to_string(),
            topic: get("TOPIC").unwrap_or("").to_string(),
            partition: get("PARTITION").and_then(|v| v.parse().ok()).unwrap_or(0),
            size: get("SIZE").and_then(|v| v.parse().ok()).unwrap_or(0),
            offset_lag: get("OFFSET_LAG")
                .or_else(|| get("OFFSET-LAG"))
                .and_then(|v| v.parse().ok()),
            error: get("ERROR")
                .filter(|s| !s.is_empty())
                .map(ToString::to_string),
        });
    }
    rows
}

fn aggregate_logdirs(rows: &[LogdirRow]) -> Vec<BrokerLogdir> {
    let mut by_dir: HashMap<String, Vec<&LogdirRow>> = HashMap::new();
    for row in rows {
        if row.dir.is_empty() {
            continue;
        }
        by_dir.entry(row.dir.clone()).or_default().push(row);
    }
    let mut items: Vec<BrokerLogdir> = by_dir
        .into_iter()
        .map(|(name, dir_rows)| {
            let error = dir_rows.iter().find_map(|row| row.error.clone());
            let mut topics_map: HashMap<String, Vec<BrokerLogdirPartition>> = HashMap::new();
            let mut size = 0i64;
            for row in &dir_rows {
                size += row.size;
                if row.topic.is_empty() {
                    continue;
                }
                topics_map
                    .entry(row.topic.clone())
                    .or_default()
                    .push(BrokerLogdirPartition {
                        partition: row.partition,
                        size: row.size,
                        offset_lag: row.offset_lag,
                    });
            }
            let mut topics: Vec<BrokerLogdirTopic> = topics_map
                .into_iter()
                .map(|(topic_name, mut partitions)| {
                    partitions.sort_by_key(|p| p.partition);
                    BrokerLogdirTopic {
                        name: topic_name,
                        partitions,
                    }
                })
                .collect();
            topics.sort_by(|a, b| a.name.cmp(&b.name));
            let partitions_count = topics.iter().map(|topic| topic.partitions.len()).sum();
            BrokerLogdir {
                name,
                error,
                topics_count: topics.len(),
                partitions_count,
                size,
                topics,
            }
        })
        .collect();
    items.sort_by(|a, b| a.name.cmp(&b.name));
    items
}

fn json_object_keys(raw: &str) -> HashSet<String> {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default()
}

fn humanize_config_source(raw: Option<&str>) -> String {
    let upper = raw.unwrap_or("").trim().to_ascii_uppercase();
    match upper.as_str() {
        "STATIC_BROKER_CONFIG" | "STATIC BROKER CONFIG" => SOURCE_STATIC.into(),
        "DEFAULT_CONFIG" | "DEFAULT CONFIG" => SOURCE_DEFAULT.into(),
        "DYNAMIC_CLUSTER_CONFIG" | "DYNAMIC_BROKER_CONFIG" | "DYNAMIC_DEFAULT_BROKER_CONFIG" => {
            SOURCE_STATIC.into()
        }
        "" => SOURCE_DEFAULT.into(),
        other if other.eq_ignore_ascii_case(SOURCE_STATIC) => SOURCE_STATIC.into(),
        other if other.eq_ignore_ascii_case(SOURCE_DEFAULT) => SOURCE_DEFAULT.into(),
        _ => SOURCE_DEFAULT.into(),
    }
}

fn apply_config_sources(items: &mut [ConfigEntry], static_keys: &HashSet<String>) {
    for item in items.iter_mut() {
        item.source = Some(if static_keys.contains(&item.name) {
            SOURCE_STATIC.into()
        } else {
            SOURCE_DEFAULT.into()
        });
    }
}

fn json_value_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => v.clone(),
        other => other.to_string(),
    }
}

/// Redpanda underscore name → Kafka dotted name (Kafbat Configs tab).
const KAFKA_CONFIG_ALIASES: &[(&str, &str)] = &[
    ("kafka_api", "listeners"),
    ("advertised_kafka_api", "advertised.listeners"),
    ("data_directory", "log.dirs"),
    ("auto_create_topics_enabled", "auto.create.topics.enable"),
    ("log_segment_size", "log.segment.bytes"),
    ("log_retention_ms", "log.retention.ms"),
    ("retention_bytes", "log.retention.bytes"),
    ("default_topic_partitions", "num.partitions"),
    ("default_topic_replications", "default.replication.factor"),
    ("log_cleanup_policy", "log.cleanup.policy"),
    ("log_compression_type", "compression.type"),
    ("log_message_timestamp_type", "log.message.timestamp.type"),
    ("kafka_batch_max_bytes", "message.max.bytes"),
    ("fetch_max_bytes", "fetch.max.bytes"),
    (
        "group_max_session_timeout_ms",
        "group.max.session.timeout.ms",
    ),
    (
        "group_min_session_timeout_ms",
        "group.min.session.timeout.ms",
    ),
    (
        "group_initial_rebalance_delay",
        "group.initial.rebalance.delay.ms",
    ),
    ("enable_idempotence", "enable.idempotence"),
    (
        "transactional_id_expiration_ms",
        "transactional.id.expiration.ms",
    ),
    ("log_segment_ms", "log.roll.ms"),
];

fn kafka_display_name(redpanda_name: &str) -> String {
    for (redpanda, kafka) in KAFKA_CONFIG_ALIASES {
        if *redpanda == redpanda_name {
            return (*kafka).to_string();
        }
    }
    redpanda_name.replace('_', ".")
}

fn resolve_redpanda_config_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    for (redpanda, kafka) in KAFKA_CONFIG_ALIASES {
        if trimmed.eq_ignore_ascii_case(kafka) {
            return (*redpanda).to_string();
        }
    }
    if trimmed.contains('.') {
        return trimmed.replace('.', "_");
    }
    trimmed.to_string()
}

fn format_config_display_value(display_name: &str, value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if matches!(display_name, "listeners" | "advertised.listeners") {
        if let Some(formatted) = format_listener_list(trimmed) {
            return formatted;
        }
    }
    if display_name == "log.dirs" {
        if let Some(path) = format_data_directory(trimmed) {
            return path;
        }
    }
    if display_name.ends_with(".bytes") {
        if let Ok(n) = trimmed.parse::<i64>() {
            return format_bytes_display(n);
        }
    }
    if display_name.ends_with(".ms") && trimmed.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
        return format!("{trimmed} ms");
    }
    trimmed.to_string()
}

fn format_listener_list(raw: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let arr = value.as_array()?;
    let parts: Vec<String> = arr
        .iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            let address = obj.get("address").and_then(Value::as_str).unwrap_or("");
            if address.is_empty() {
                return None;
            }
            let port = obj.get("port").and_then(Value::as_i64).unwrap_or(0);
            let name = obj.get("name").and_then(Value::as_str).unwrap_or("");
            if name.is_empty() {
                Some(format!("{address}:{port}"))
            } else {
                Some(format!("{name}://{address}:{port}"))
            }
        })
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(","))
    }
}

fn format_data_directory(raw: &str) -> Option<String> {
    if !raw.starts_with('{') {
        return None;
    }
    let value: Value = serde_json::from_str(raw).ok()?;
    value
        .get("data_directory")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn format_bytes_display(bytes: i64) -> String {
    if bytes < 0 {
        return bytes.to_string();
    }
    const UNITS: [&str; 7] = ["B", "KB", "MB", "GB", "TB", "PB", "EB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if (value - value.round()).abs() < 1e-6 {
        format!("{} {}", value.round() as i64, UNITS[unit])
    } else if value >= 10.0 {
        format!("{:.0} {}", value, UNITS[unit])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

pub fn parse_cluster_config_json(raw: &str) -> Vec<ConfigEntry> {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    let mut items: Vec<ConfigEntry> = object
        .iter()
        .map(|(name, value)| {
            ConfigEntry::new(
                name.clone(),
                json_value_to_string(value),
                Some(SOURCE_DEFAULT.into()),
            )
        })
        .collect();
    items.sort_by(|a, b| a.name.cmp(&b.name));
    items
}

pub fn parse_config_export(raw: &str) -> Vec<ConfigEntry> {
    let mut items = Vec::new();
    let mut current_key: Option<String> = None;
    let mut current_values: Vec<String> = Vec::new();
    let flush = flush_export_entry;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if indented && trimmed.starts_with("- ") {
            current_values.push(trimmed.trim_start_matches("- ").trim().to_string());
            continue;
        }
        if indented {
            continue;
        }
        flush(&mut current_key, &mut current_values, &mut items);
        let Some((key, rest)) = trimmed.split_once(':') else {
            continue;
        };
        current_key = Some(key.trim().to_string());
        let value = rest.trim();
        if !value.is_empty() && value != "[]" {
            current_values.push(value.to_string());
        }
    }
    flush(&mut current_key, &mut current_values, &mut items);
    items
}

fn flush_export_entry(
    key: &mut Option<String>,
    values: &mut Vec<String>,
    items: &mut Vec<ConfigEntry>,
) {
    let Some(name) = key.take() else {
        return;
    };
    let value = if values.len() <= 1 {
        values.first().cloned().unwrap_or_default()
    } else {
        format!("[{}]", values.join(","))
    };
    items.push(ConfigEntry::new(name, value, Some(SOURCE_DEFAULT.into())));
    values.clear();
}

pub fn parse_prometheus_metrics(raw: &str) -> Vec<MetricSample> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(sample) = parse_prometheus_line(trimmed) {
            out.push(sample);
        }
    }
    out
}

fn parse_prometheus_line(line: &str) -> Option<MetricSample> {
    let (name, labels, rest) = if let Some(start) = line.find('{') {
        let end = line.rfind('}')?;
        if end < start {
            return None;
        }
        (
            line[..start].trim().to_string(),
            parse_prom_labels(&line[start + 1..end]),
            line[end + 1..].trim(),
        )
    } else {
        let mut parts = line.splitn(2, char::is_whitespace);
        (
            parts.next()?.to_string(),
            HashMap::new(),
            parts.next().unwrap_or("").trim(),
        )
    };
    if name.is_empty() {
        return None;
    }
    let value = rest.split_whitespace().next()?.parse().ok()?;
    Some(MetricSample {
        name,
        labels,
        value,
    })
}

fn parse_prom_labels(raw: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut rest = raw.trim();
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else {
            break;
        };
        let key = rest[..eq].trim().to_string();
        rest = rest[eq + 1..].trim_start();
        if rest.starts_with('"') {
            rest = &rest[1..];
            if let Some(end) = rest.find('"') {
                map.insert(key, rest[..end].to_string());
                rest = rest[end + 1..].trim_start_matches(',').trim_start();
            } else {
                break;
            }
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            map.insert(key, rest[..end].trim().to_string());
            rest = rest[end..].trim_start_matches(',').trim_start();
        }
    }
    map
}

fn parse_cluster_health(raw: &str) -> Value {
    let mut object = Map::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let name = key.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        if name.is_empty() {
            continue;
        }
        object.insert(name, json!(value.trim()));
    }
    Value::Object(object)
}

pub fn parse_acl_list_json(raw: &str) -> Vec<AclItem> {
    // rpk security acl list --format json → {"matches":[{...}]}
    let v: Value = serde_json::from_str(raw.trim()).unwrap_or(Value::Null);
    let matches = v.get("matches").and_then(|m| m.as_array());
    let Some(arr) = matches else {
        return vec![];
    };
    arr.iter()
        .filter_map(|m| {
            Some(AclItem {
                principal: m.get("principal")?.as_str()?.to_string(),
                host: m
                    .get("host")
                    .and_then(|v| v.as_str())
                    .unwrap_or("*")
                    .to_string(),
                resource_type: m
                    .get("resource_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                resource_name: m
                    .get("resource_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                pattern_type: m
                    .get("resource_pattern_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("LITERAL")
                    .to_string(),
                operation: m
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                permission: m
                    .get("permission")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

// Text parser kept for tests; production uses parse_acl_list_json.
#[cfg(test)]
pub fn parse_acl_list(raw: &str) -> Vec<AclItem> {
    let mut items = Vec::new();
    let mut header: Vec<String> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let cols: Vec<&str> = trimmed.split_whitespace().collect();
        if cols.is_empty() {
            continue;
        }
        if cols[0].eq_ignore_ascii_case("PRINCIPAL") {
            header = cols.iter().map(|c| c.to_ascii_uppercase()).collect();
            continue;
        }
        if header.is_empty() {
            continue;
        }
        let get = |name: &str| {
            header
                .iter()
                .position(|h| h == name)
                .and_then(|idx| cols.get(idx).copied())
                .unwrap_or("")
                .to_string()
        };
        items.push(AclItem {
            principal: get("PRINCIPAL"),
            host: get("HOST"),
            resource_type: get("RESOURCE-TYPE"),
            resource_name: get("RESOURCE-NAME"),
            pattern_type: get("RESOURCE-PATTERN-TYPE"),
            operation: get("OPERATION"),
            permission: get("PERMISSION"),
        });
    }
    items
}

#[derive(Default)]
pub struct GroupDescribeParsed {
    pub state: Option<String>,
    pub members: Option<i32>,
    pub coordinator: Option<String>,
    pub balancer: Option<String>,
    pub total_lag: i64,
    pub entries: Vec<GroupLagItem>,
}

pub fn parse_group_describe(raw: &str) -> GroupDescribeParsed {
    let mut parsed = GroupDescribeParsed::default();
    let mut header: Vec<String> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let cols: Vec<&str> = trimmed.split_whitespace().collect();
        if skip_group_line(&cols) {
            continue;
        }
        if take_group_header(&mut parsed, &mut header, &cols) {
            continue;
        }
        if let Some(item) = group_lag_from_cols(&header, &cols) {
            parsed.entries.push(item);
        }
    }
    if parsed.total_lag == 0 {
        parsed.total_lag = parsed.entries.iter().map(|e| e.lag).sum();
    }
    parsed
}

fn skip_group_line(cols: &[&str]) -> bool {
    cols.is_empty()
        || (cols[0].eq_ignore_ascii_case("GROUP")
            && cols.len() >= 2
            && !cols[1].chars().all(|c| c.is_ascii_digit() || c == '-'))
}

fn take_group_header(
    parsed: &mut GroupDescribeParsed,
    header: &mut Vec<String>,
    cols: &[&str],
) -> bool {
    if cols.len() == 2 && !cols[0].eq_ignore_ascii_case("TOPIC") {
        apply_group_summary(parsed, cols);
        return true;
    }
    if cols[0].eq_ignore_ascii_case("TOPIC") {
        *header = cols.iter().map(|c| c.to_ascii_uppercase()).collect();
        return true;
    }
    false
}

fn apply_group_summary(parsed: &mut GroupDescribeParsed, cols: &[&str]) {
    match cols[0].to_ascii_uppercase().as_str() {
        "STATE" => parsed.state = Some(cols[1].to_string()),
        "MEMBERS" => parsed.members = cols[1].parse().ok(),
        "COORDINATOR" => parsed.coordinator = Some(cols[1].to_string()),
        "BALANCER" => parsed.balancer = Some(cols[1].to_string()),
        "TOTAL-LAG" => parsed.total_lag = cols[1].parse().unwrap_or(0),
        _ => {}
    }
}

fn header_i64(header: &[String], cols: &[&str], name: &str, fallback_idx: usize) -> i64 {
    header
        .iter()
        .position(|h| h == name)
        .and_then(|idx| cols.get(idx))
        .and_then(|v| v.parse().ok())
        .or_else(|| cols.get(fallback_idx).and_then(|v| v.parse().ok()))
        .unwrap_or(0)
}

fn header_text(header: &[String], cols: &[&str], name: &str) -> Option<String> {
    header
        .iter()
        .position(|h| h == name)
        .and_then(|idx| cols.get(idx))
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty() && s != "-")
}

fn group_lag_from_cols(header: &[String], cols: &[&str]) -> Option<GroupLagItem> {
    if header.is_empty() || cols.len() < 5 || cols[1].parse::<i32>().is_err() {
        return None;
    }
    Some(GroupLagItem {
        topic: cols[0].to_string(),
        partition: cols[1].parse().unwrap_or(0),
        current_offset: header_i64(header, cols, "CURRENT-OFFSET", 2),
        log_end_offset: header_i64(header, cols, "LOG-END-OFFSET", 4),
        lag: header_i64(header, cols, "LAG", 5),
        consumer_id: header_text(header, cols, "MEMBER-ID"),
        host: header_text(header, cols, "HOST"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_seek_failure_reported_inside_rpk_table() {
        // rpk exits 0 here, so the error is only visible in the trailing column.
        let raw = "TOPIC           PARTITION  PRIOR-OFFSET  CURRENT-OFFSET  ERROR\n\
                   test01.G_BOOKS  0          -1            -1              INVALID_OPERATION: seeking a non-empty group is not allowed.";
        let errors = rpk_row_errors(raw);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("non-empty group"));
        let msg = group_error_message("connect-cdc-test01-sink", &errors);
        assert!(msg.contains("still has active members"), "{msg}");
        assert!(msg.contains("connect-cdc-test01-sink"), "{msg}");
    }

    #[test]
    fn successful_seek_reports_no_errors() {
        let raw = "TOPIC           PARTITION  PRIOR-OFFSET  CURRENT-OFFSET  ERROR\n\
                   test01.G_BOOKS  0          1000          0               ";
        assert!(rpk_row_errors(raw).is_empty());
    }

    #[test]
    fn output_without_error_column_is_not_flagged() {
        let raw = "TOPIC           PARTITION  OFFSET\ntest01.G_BOOKS  0          1000";
        assert!(rpk_row_errors(raw).is_empty());
        assert!(rpk_row_errors("").is_empty());
    }

    #[test]
    fn group_delete_status_ok_is_not_flagged() {
        let raw = "GROUP                    STATUS\nconnect-cdc-test01-sink  OK";
        assert!(rpk_row_errors(raw).is_empty());
    }

    #[test]
    fn group_delete_non_empty_is_flagged_via_status_column() {
        // `rpk group delete` on a live sink group exits 0 but reports the failure
        // in the STATUS column rather than a dedicated ERROR column.
        let raw = "GROUP                    STATUS\n\
                   connect-cdc-test01-sink  NON_EMPTY_GROUP: the group is not empty";
        let errors = rpk_row_errors(raw);
        assert_eq!(errors.len(), 1);
        let msg = group_error_message("connect-cdc-test01-sink", &errors);
        assert!(msg.contains("still has active members"), "{msg}");
        assert!(msg.contains("Stop the consumer first"), "{msg}");
    }

    #[test]
    fn duplicate_partition_errors_are_reported_once() {
        let raw = "TOPIC  PARTITION  PRIOR-OFFSET  CURRENT-OFFSET  ERROR\n\
                   t      0          -1            -1              GROUP_SUBSCRIBED_TO_TOPIC\n\
                   t      1          -1            -1              GROUP_SUBSCRIBED_TO_TOPIC";
        let errors = rpk_row_errors(raw);
        assert_eq!(errors.len(), 1);
        assert!(group_error_message("g", &errors).contains("Stop the consumer first"));
    }

    #[test]
    fn strips_table_prefix_from_rpk_failure_text() {
        let raw = "test01.G_BOOKS  0     GROUP_SUBSCRIBED_TO_TOPIC: Deleting offsets of a topic is forbidden while the consumer group is actively subscribed to it.";
        let cleaned = strip_rpk_row_prefix(raw);
        assert!(
            cleaned.starts_with("GROUP_SUBSCRIBED_TO_TOPIC:"),
            "{cleaned}"
        );
        assert!(!cleaned.contains("test01.G_BOOKS"), "{cleaned}");
        assert!(group_error_message("g", &[cleaned]).contains("Stop the consumer first"));
    }

    #[test]
    fn keeps_plain_failure_text_unchanged() {
        assert_eq!(
            strip_rpk_row_prefix("unable to reach broker"),
            "unable to reach broker"
        );
    }

    #[test]
    fn parses_cluster_info_controller() {
        let raw = "CLUSTER\n=======\nabc\n\nBROKERS\n=======\nID    HOST    PORT\n0*    kafka   9092\n1     kafka2  9092\n";
        let (id, brokers) = parse_cluster_info(raw);
        assert_eq!(id.as_deref(), Some("abc"));
        assert_eq!(brokers.len(), 2);
        assert!(brokers[0].controller);
        assert_eq!(brokers[0].host, "kafka");
        assert!(!brokers[1].controller);
    }

    #[test]
    fn parses_acl_rows() {
        let raw = "PRINCIPAL     HOST  RESOURCE-TYPE  RESOURCE-NAME  RESOURCE-PATTERN-TYPE  OPERATION  PERMISSION\nUser:alice    *     TOPIC          demo           LITERAL                READ       ALLOW\n";
        let items = parse_acl_list(raw);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].principal, "User:alice");
        assert_eq!(items[0].resource_name, "demo");
        assert_eq!(items[0].operation, "READ");
    }

    #[test]
    fn parses_group_describe_summary() {
        let raw = "GROUP        g1\nSTATE        Empty\nMEMBERS      0\nTOTAL-LAG    12\n\nTOPIC  PARTITION  CURRENT-OFFSET  LOG-END-OFFSET  LAG\nfoo    0          1               13              12\n";
        let parsed = parse_group_describe(raw);
        assert_eq!(parsed.state.as_deref(), Some("Empty"));
        assert_eq!(parsed.total_lag, 12);
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].topic, "foo");
    }

    #[test]
    fn parses_logdirs_and_aggregates() {
        let raw = "BROKER  DIR  TOPIC  PARTITION  SIZE  ERROR\n0  /var/lib/redpanda/data  demo  0  1024  \n0  /var/lib/redpanda/data  demo  1  2048  \n0  /var/lib/redpanda/data  other  0  8  io\n";
        let rows = parse_logdir_rows(raw);
        assert_eq!(rows.len(), 3);
        let dirs = aggregate_logdirs(&rows);
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0].name, "/var/lib/redpanda/data");
        assert_eq!(dirs[0].topics_count, 2);
        assert_eq!(dirs[0].partitions_count, 3);
        assert_eq!(dirs[0].size, 3080);
        assert_eq!(dirs[0].error.as_deref(), Some("io"));
    }

    #[test]
    fn parses_prometheus_and_cluster_config() {
        let metrics = parse_prometheus_metrics(
            "# HELP x\nredpanda_storage_disk_total_bytes{} 12.5\nredpanda_kafka_request_bytes_total{redpanda_request=\"produce\",redpanda_topic=\"demo\"} 9\n",
        );
        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0].name, "redpanda_storage_disk_total_bytes");
        assert_eq!(metrics[0].value, 12.5);
        assert_eq!(
            metrics[1].labels.get("redpanda_topic").map(String::as_str),
            Some("demo")
        );

        let mut configs = parse_cluster_config_json(
            r#"{"log_retention_ms":604800000,"auto_create_topics_enabled":true,"cloud_storage_access_key":null}"#,
        );
        assert_eq!(configs.len(), 3);
        assert_eq!(configs[0].name, "auto_create_topics_enabled");
        assert_eq!(configs[0].value, "true");
        let mut static_keys = HashSet::new();
        static_keys.insert("auto_create_topics_enabled".into());
        apply_config_sources(&mut configs, &static_keys);
        assert_eq!(configs[0].source.as_deref(), Some(SOURCE_STATIC));
        assert_eq!(
            configs
                .iter()
                .find(|c| c.name == "log_retention_ms")
                .and_then(|c| c.source.as_deref()),
            Some(SOURCE_DEFAULT)
        );
        let auto_create = configs
            .iter()
            .find(|c| c.name == "auto_create_topics_enabled")
            .expect("auto_create_topics_enabled");
        assert_eq!(auto_create.display_name, "auto.create.topics.enable");
        assert_eq!(auto_create.display_value, "true");
        let retention = configs
            .iter()
            .find(|c| c.name == "log_retention_ms")
            .expect("log_retention_ms");
        assert_eq!(retention.display_name, "log.retention.ms");
        assert_eq!(retention.display_value, "604800000 ms");
    }

    #[test]
    fn maps_kafka_dotted_names_and_formats_values() {
        assert_eq!(
            kafka_display_name("auto_create_topics_enabled"),
            "auto.create.topics.enable"
        );
        assert_eq!(
            resolve_redpanda_config_name("auto.create.topics.enable"),
            "auto_create_topics_enabled"
        );
        assert_eq!(resolve_redpanda_config_name("Listeners"), "kafka_api");
        assert_eq!(
            resolve_redpanda_config_name("auto_create_topics_enabled"),
            "auto_create_topics_enabled"
        );

        let listeners = ConfigEntry::new(
            "kafka_api",
            r#"[{"address":"0.0.0.0","name":"internal","port":9092},{"address":"0.0.0.0","name":"external","port":19092}]"#,
            None,
        );
        assert_eq!(listeners.display_name, "listeners");
        assert_eq!(
            listeners.display_value,
            "internal://0.0.0.0:9092,external://0.0.0.0:19092"
        );

        let advertised = ConfigEntry::new(
            "advertised_kafka_api",
            r#"[{"address":"cdc-message","name":"internal","port":9092},{"address":"127.0.0.1","name":"external","port":19092}]"#,
            None,
        );
        assert_eq!(advertised.display_name, "advertised.listeners");
        assert_eq!(
            advertised.display_value,
            "internal://cdc-message:9092,external://127.0.0.1:19092"
        );

        let dirs = ConfigEntry::new(
            "data_directory",
            r#"{"data_directory":"/var/lib/redpanda/data"}"#,
            None,
        );
        assert_eq!(dirs.display_name, "log.dirs");
        assert_eq!(dirs.display_value, "/var/lib/redpanda/data");

        let segment = ConfigEntry::new("log_segment_size", "134217728", None);
        assert_eq!(segment.display_name, "log.segment.bytes");
        assert_eq!(segment.display_value, "128 MB");

        let partitions = ConfigEntry::new("default_topic_partitions", "1", None);
        assert_eq!(partitions.display_name, "num.partitions");
        assert_eq!(
            resolve_redpanda_config_name("num.partitions"),
            "default_topic_partitions"
        );
        assert_eq!(
            resolve_redpanda_config_name("default.replication.factor"),
            "default_topic_replications"
        );
        assert_eq!(format_bytes_display(134217728), "128 MB");
    }

    #[test]
    fn extracts_ksql_queries_collection() {
        let value = json!([{
            "@type": "queries",
            "queries": [{ "id": "CSAS_FOO_0", "state": "RUNNING" }]
        }]);
        let items = extract_ksql_collection(&value);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], "CSAS_FOO_0");
    }

    #[test]
    fn routes_select_emit_changes_as_push() {
        assert_eq!(
            ksql_route("SELECT * FROM bookings EMIT CHANGES;"),
            KsqlRoute::Push
        );
        assert_eq!(ksql_route("SELECT * FROM bookings;"), KsqlRoute::Pull);
        assert_eq!(ksql_route("SHOW TABLES;"), KsqlRoute::Statement);
        assert!(ensure_push_limit("SELECT * FROM x EMIT CHANGES").contains("LIMIT 20"));
    }

    #[test]
    fn parses_ksql_query_rows() {
        let value = json!([
            { "header": { "queryId": "q1", "schema": "`ID` INTEGER, `NAME` VARCHAR" } },
            { "row": { "columns": [1, "alpha"] } }
        ]);
        let parsed = normalize_ksql_result(value);
        assert_eq!(parsed["kind"], "query");
        assert_eq!(parsed["rowCount"], 1);
        assert_eq!(parsed["rows"][0]["ID"], 1);
        assert_eq!(parsed["rows"][0]["NAME"], "alpha");
    }
}
