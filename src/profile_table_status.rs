//! Per-table source/target row counts, Kafka backlog, and JDBC offsets for a profile.

use std::collections::HashMap;

use std::time::Duration;

use chrono::Utc;
use reqwest::Method;
use serde::Serialize;
use serde_json::Value;
use tokio::time::timeout;
use uuid::Uuid;

use crate::connector_catalog;
use crate::diagnose::{diagnose, diagnose_or_side, Diagnosis, Side};
use crate::error::AppError;
use crate::incremental_cdc;
use crate::models::{CdcProfile, TableMapping};
use crate::reconcile::{
    connect_request, connector_name_for, jdbc_target_schema, sink_connector_name_for,
    source_table_include_list, topic_prefix_for,
};
use crate::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TableSyncPhase {
    Complete,
    InProgress,
    Behind,
    Unknown,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct TableSyncStatusRow {
    pub source_schema: String,
    pub source_table: String,
    pub target_schema: String,
    pub target_table: String,
    pub source_qualified: String,
    pub target_qualified: String,
    pub kafka_topic: Option<String>,
    pub phase: TableSyncPhase,
    pub source_row_count: Option<i64>,
    pub target_row_count: Option<i64>,
    /// Approximate rows not yet on target (`source - target` when both counts are known).
    pub pending_rows: Option<i64>,
    /// Records emitted on the capture topic (high watermark − log start).
    pub kafka_messages: Option<i64>,
    /// Rows captured but not yet reflected on target (`kafka − target`, when both known).
    pub processing_rows: Option<i64>,
    pub source_offset: Option<Value>,
    /// Incremental rolling-window / full-copy rule for this table (`None` = watermark).
    /// Such tables keep no JDBC offset and re-publish rows on every poll.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poll_rule: Option<crate::models::TablePollRule>,
    pub source_count_error: Option<String>,
    pub target_count_error: Option<String>,
    pub kafka_error: Option<String>,
    /// Where each error above comes from and how to fix it (same order: source,
    /// target, kafka). Omitted when there is no error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_count_diagnosis: Option<Diagnosis>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_count_diagnosis: Option<Diagnosis>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kafka_diagnosis: Option<Diagnosis>,
}

/// A connector of a Running profile that is FAILED or PAUSED, with its diagnosis.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectorIssue {
    pub connector: String,
    /// `source` or `sink`.
    pub role: &'static str,
    /// Short message (state + first line / root cause of the trace).
    pub message: String,
    pub diagnosis: Diagnosis,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileTableSyncStatusResponse {
    pub timestamp: chrono::DateTime<Utc>,
    pub profile_id: Uuid,
    pub profile_name: String,
    pub profile_status: String,
    pub capture_mode: String,
    pub tables: Vec<TableSyncStatusRow>,
    pub counts_skipped: bool,
    /// Why nothing is moving although the profile says Running: a FAILED connector/task
    /// or a connector left PAUSED in Kafka Connect. Unfinished tables are then `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector_issue: Option<String>,
    /// Same problems as `connector_issue`, one entry per connector/task, diagnosed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub connector_issues: Vec<ConnectorIssue>,
}

#[derive(Debug, serde::Deserialize)]
pub struct TableSyncStatusQuery {
    /// When true, skip `COUNT(*)` on source/target (offsets + Kafka only).
    #[serde(default)]
    pub skip_counts: bool,
    /// Live polling: count the target only. Source `COUNT(*)` (slow on large Oracle
    /// tables, load on production) is left to an occasional full refresh.
    #[serde(default)]
    pub skip_source_counts: bool,
    /// Comma-separated `schema.table` source names: report only these (case-insensitive).
    /// Lets a paginated UI poll just the visible page instead of every table.
    #[serde(default)]
    pub tables: Option<String>,
}

/// Keep the entries named in `filter` (all when it is empty).
fn select_entries(entries: Vec<TableEntry>, filter: Option<&str>) -> Vec<TableEntry> {
    let wanted: std::collections::HashSet<String> = filter
        .unwrap_or_default()
        .split(',')
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    if wanted.is_empty() {
        return entries;
    }
    entries
        .into_iter()
        .filter(|e| wanted.contains(&e.source_qualified.to_ascii_lowercase()))
        .collect()
}

pub async fn profile_table_sync_status(
    state: &AppState,
    id: Uuid,
    q: TableSyncStatusQuery,
) -> Result<ProfileTableSyncStatusResponse, AppError> {
    let mut profile = state
        .profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("profile not found: {id}")))?;
    incremental_cdc::hydrate_upsert_keys_from_mappings(&mut profile);

    let entries = table_entries(&profile);
    if entries.is_empty() {
        return Err(AppError::BadRequest(
            "profile has no table mappings or source tables".into(),
        ));
    }
    let entries = select_entries(entries, q.tables.as_deref());

    let prefix = topic_prefix_for(&profile);
    let offsets_by_table = if let Some(name) = profile.connector_name.as_deref() {
        parse_source_offsets(fetch_connector_offsets(state, name).await)
    } else {
        HashMap::new()
    };

    let connector_issues = if profile.status == crate::models::ProfileStatus::Running {
        connector_issues(state, &profile).await
    } else {
        Vec::new()
    };
    let connector_issue = (!connector_issues.is_empty()).then(|| {
        connector_issues
            .iter()
            .map(|i| i.message.as_str())
            .collect::<Vec<_>>()
            .join(" · ")
    });

    // Unread messages per capture topic for the managed sink (one rpk call).
    let sink_lag = sink_lag_by_topic(state, &profile).await;

    // One quick TCP check per side before counting: when a database is down, every
    // table would otherwise wait out its own count timeout, one after another.
    let source_down = if q.skip_counts || q.skip_source_counts {
        None
    } else {
        probe_endpoint(&profile.source.source_type, &profile.source.connection).await
    };
    let target_down = match (q.skip_counts, profile.target.as_ref()) {
        (false, Some(t)) => probe_endpoint(&t.target_type, &t.connection).await,
        _ => None,
    };

    let (profile, prefix, offsets_by_table, sink_lag, connector_issue, q) = (
        &profile,
        &prefix,
        &offsets_by_table,
        &sink_lag,
        &connector_issue,
        &q,
    );
    let (source_down, target_down) = (&source_down, &target_down);
    let ctx = TableSyncCtx {
        state,
        profile,
        prefix,
        offsets_by_table,
        sink_lag,
        connector_issue,
        q,
        source_down,
        target_down,
    };
    let mut rows = Vec::with_capacity(entries.len());
    for chunk in entries.chunks(COUNT_CONCURRENCY) {
        let batch = futures_util::future::join_all(
            chunk
                .iter()
                .cloned()
                .map(|entry| build_table_sync_row(&ctx, entry)),
        )
        .await;
        rows.extend(batch);
    }

    Ok(ProfileTableSyncStatusResponse {
        timestamp: Utc::now(),
        profile_id: profile.id,
        profile_name: profile.name.clone(),
        profile_status: format!("{:?}", profile.status),
        capture_mode: format!("{:?}", profile.source.capture_mode),
        tables: rows,
        counts_skipped: q.skip_counts,
        connector_issue: connector_issue.clone(),
        connector_issues,
    })
}

/// Summarise FAILED / PAUSED connectors of a Running profile (source + managed sink).
async fn connector_issues(state: &AppState, profile: &CdcProfile) -> Vec<ConnectorIssue> {
    let mut names = vec![(connector_name_for(profile), "source")];
    if profile.has_managed_sink() {
        names.push((sink_connector_name_for(profile), "sink"));
    }
    let mut issues = Vec::new();
    for (name, role) in names {
        let path = format!("/connectors/{}/status", urlencoding::encode(&name));
        let status = match connect_request(state, Method::GET, &path, None).await {
            Ok(v) => v,
            Err(_) => continue, // missing / unreachable: other columns already say so
        };
        let side = if role == "sink" {
            Side::Target
        } else {
            Side::Source
        };
        for (message, raw) in describe_connector_status(&name, &status) {
            let diagnosis = diagnose_or_side(&raw, side);
            issues.push(ConnectorIssue {
                connector: name.clone(),
                role,
                message,
                diagnosis,
            });
        }
    }
    issues
}

/// `(short message, text to diagnose)` per FAILED connector/task, or one entry for a
/// connector left PAUSED. The full trace is diagnosed so a root cause deep in the
/// `Caused by:` chain is found.
fn describe_connector_status(name: &str, status: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let state = status
        .pointer("/connector/state")
        .and_then(Value::as_str)
        .unwrap_or("");
    if state == "FAILED" {
        let trace = status.pointer("/connector/trace");
        out.push((
            format!("{name} FAILED: {}", trace_summary(trace)),
            trace
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ));
    }
    for task in status
        .get("tasks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if task.get("state").and_then(Value::as_str) == Some("FAILED") {
            let id = task.get("id").and_then(Value::as_i64).unwrap_or(0);
            out.push((
                format!(
                    "{name} task {id} FAILED: {}",
                    trace_summary(task.get("trace"))
                ),
                task.get("trace")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            ));
        }
    }
    if out.is_empty() && state == "PAUSED" {
        out.push((
            format!("{name} is PAUSED in Kafka Connect — use Resume or Restart"),
            String::new(),
        ));
    }
    out
}

/// First line of a Java stack trace plus its innermost `Caused by:` (the actual reason,
/// e.g. `UnknownHostException`), each trimmed to a readable length.
fn trace_summary(trace: Option<&Value>) -> String {
    let Some(text) = trace.and_then(Value::as_str) else {
        return "no error detail from Kafka Connect".into();
    };
    let clip = |s: &str| -> String {
        let s = s.trim();
        if s.chars().count() > 300 {
            format!("{}…", s.chars().take(300).collect::<String>())
        } else {
            s.to_string()
        }
    };
    let head = text.lines().next().unwrap_or_default();
    let root = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("Caused by: "))
        .next_back();
    match root {
        Some(r) if r.trim() != head.trim() => format!("{} (caused by {})", clip(head), clip(r)),
        _ => clip(head),
    }
}

/// Kafka topic a table is captured into.
///
/// Debezium (Logical / SyncOnce) names topics `{prefix}.{schema}.{table}`; the JDBC
/// connectors used by Trigger and Incremental capture write `{prefix}.{TABLE}`. Using
/// the JDBC form for Debezium pipelines reported "topic not found" on every table.
fn capture_topic(profile: &CdcProfile, prefix: &str, schema: &str, table: &str) -> String {
    use crate::models::CaptureMode;
    match profile.source.capture_mode {
        CaptureMode::Logical | CaptureMode::SyncOnce if !schema.is_empty() => {
            format!("{prefix}.{schema}.{table}")
        }
        _ => format!("{prefix}.{table}"),
    }
}

#[derive(Clone)]
struct TableEntry {
    source_schema: String,
    source_table: String,
    target_schema: String,
    target_table: String,
    source_qualified: String,
    target_qualified: String,
}

fn table_entries(profile: &CdcProfile) -> Vec<TableEntry> {
    // Count the same target table the copy/sink write to (Oracle → Postgres folds to lowercase).
    table_entries_raw(profile)
        .into_iter()
        .map(|mut e| {
            let (schema, table) = crate::postgres_trigger_cdc::fold_oracle_postgres_target(
                profile,
                &e.target_schema,
                &e.target_table,
            );
            e.target_qualified = crate::models::qualified_table(&schema, &table);
            e.target_schema = schema;
            e.target_table = table;
            e
        })
        .collect()
}

fn table_entries_raw(profile: &CdcProfile) -> Vec<TableEntry> {
    if !profile.flow.mappings.is_empty() {
        return profile.flow.mappings.iter().map(mapping_to_entry).collect();
    }
    let target_schema = jdbc_target_schema(profile).to_string();
    source_table_include_list(profile)
        .into_iter()
        .filter_map(|qualified| {
            let (source_schema, source_table) = split_qualified(&qualified)?;
            let target_table = source_table.clone();
            Some(TableEntry {
                source_qualified: qualified,
                target_qualified: crate::models::qualified_table(&target_schema, &target_table),
                source_schema,
                source_table,
                target_schema: target_schema.clone(),
                target_table,
            })
        })
        .collect()
}

struct TableSyncCtx<'a> {
    state: &'a AppState,
    profile: &'a CdcProfile,
    prefix: &'a str,
    offsets_by_table: &'a HashMap<String, Value>,
    sink_lag: &'a Option<HashMap<String, i64>>,
    connector_issue: &'a Option<String>,
    q: &'a TableSyncStatusQuery,
    source_down: &'a Option<String>,
    target_down: &'a Option<String>,
}

enum TableCountSide {
    Source,
    Target,
}

async fn table_row_count(
    ctx: &TableSyncCtx<'_>,
    entry: &TableEntry,
    which: TableCountSide,
) -> (Option<i64>, Option<String>) {
    let skip = match which {
        TableCountSide::Source => ctx.q.skip_counts || ctx.q.skip_source_counts,
        TableCountSide::Target => ctx.q.skip_counts,
    };
    if skip {
        return (None, None);
    }
    let down = match which {
        TableCountSide::Source => ctx.source_down,
        TableCountSide::Target => ctx.target_down,
    };
    if let Some(msg) = down {
        return (None, Some(msg.clone()));
    }
    match which {
        TableCountSide::Source => {
            count_side(
                &ctx.profile.source.source_type,
                &ctx.profile.source.connection,
                &entry.source_schema,
                &entry.source_table,
            )
            .await
        }
        TableCountSide::Target => {
            let Some(target) = ctx.profile.target.as_ref() else {
                return (None, Some("no target configured".into()));
            };
            count_side(
                &target.target_type,
                &target.connection,
                &entry.target_schema,
                &entry.target_table,
            )
            .await
        }
    }
}

fn table_processing_rows(
    sink_lag: &Option<HashMap<String, i64>>,
    topic: &str,
    kafka_messages: Option<i64>,
    target_row_count: Option<i64>,
    poll_rule: &Option<crate::models::TablePollRule>,
) -> Option<i64> {
    match sink_lag {
        Some(lags) => lags.get(topic).copied().or(kafka_messages),
        None => match (kafka_messages, target_row_count, poll_rule) {
            (Some(k), Some(t), None) => Some((k - t).max(0)),
            _ => None,
        },
    }
}

async fn build_table_sync_row(ctx: &TableSyncCtx<'_>, entry: TableEntry) -> TableSyncStatusRow {
    let topic = capture_topic(
        ctx.profile,
        ctx.prefix,
        &entry.source_schema,
        &entry.source_table,
    );
    let (kafka_messages, kafka_error) = topic_message_total(ctx.state, &topic).await;

    let source_offset = ctx
        .offsets_by_table
        .get(&entry.source_qualified.to_ascii_uppercase())
        .or_else(|| {
            ctx.offsets_by_table
                .get(&entry.source_table.to_ascii_uppercase())
        })
        .cloned();

    let (source_row_count, source_count_error) =
        table_row_count(ctx, &entry, TableCountSide::Source).await;
    let (target_row_count, target_count_error) =
        table_row_count(ctx, &entry, TableCountSide::Target).await;

    let pending_rows = match (source_row_count, target_row_count) {
        (Some(s), Some(t)) => Some((s - t).max(0)),
        _ => None,
    };

    let poll_rule = if ctx.profile.source.capture_mode == crate::models::CaptureMode::Incremental {
        crate::incremental_cdc::table_poll_rule(ctx.profile, &entry.source_qualified).cloned()
    } else {
        None
    };
    let processing_rows = table_processing_rows(
        ctx.sink_lag,
        &topic,
        kafka_messages,
        target_row_count,
        &poll_rule,
    );

    let mut phase = classify_phase(
        source_row_count,
        target_row_count,
        pending_rows,
        kafka_messages,
        processing_rows,
        &source_count_error,
        &target_count_error,
    );
    if ctx.connector_issue.is_some() && !matches!(phase, TableSyncPhase::Complete) {
        phase = TableSyncPhase::Error;
    }

    TableSyncStatusRow {
        source_schema: entry.source_schema,
        source_table: entry.source_table,
        target_schema: entry.target_schema,
        target_table: entry.target_table,
        source_qualified: entry.source_qualified,
        target_qualified: entry.target_qualified,
        kafka_topic: Some(topic),
        phase,
        source_row_count,
        target_row_count,
        pending_rows,
        kafka_messages,
        processing_rows,
        source_offset,
        poll_rule,
        source_count_diagnosis: source_count_error
            .as_deref()
            .map(|e| diagnose_or_side(e, Side::Source)),
        target_count_diagnosis: target_count_error
            .as_deref()
            .map(|e| diagnose_or_side(e, Side::Target)),
        kafka_diagnosis: kafka_error
            .as_deref()
            .and_then(|e| diagnose(e, Side::Unknown)),
        source_count_error,
        target_count_error,
        kafka_error,
    }
}

fn mapping_to_entry(m: &TableMapping) -> TableEntry {
    let source_schema = m.source_schema.trim().to_string();
    let source_table = m.source_table.trim().to_string();
    let target_schema = if m.target_schema.trim().is_empty() {
        source_schema.clone()
    } else {
        m.target_schema.trim().to_string()
    };
    let target_table = if m.identical || m.target_table.trim().is_empty() {
        source_table.clone()
    } else {
        m.target_table.trim().to_string()
    };
    TableEntry {
        source_qualified: m.source_qualified(),
        target_qualified: crate::models::qualified_table(&target_schema, &target_table),
        source_schema,
        source_table,
        target_schema,
        target_table,
    }
}

fn split_qualified(qualified: &str) -> Option<(String, String)> {
    let qualified = qualified.trim();
    if qualified.is_empty() {
        return None;
    }
    let (schema, table) = qualified.rsplit_once('.')?;
    let schema = schema.trim();
    let table = table.trim();
    if schema.is_empty() || table.is_empty() {
        return None;
    }
    Some((schema.to_string(), table.to_string()))
}

const COUNT_CONCURRENCY: usize = 4;
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

fn default_port(db_type: &str) -> Option<u16> {
    match db_type.trim().to_ascii_lowercase().as_str() {
        "postgres" | "postgresql" => Some(5432),
        "oracle" => Some(1521),
        "mysql" | "mariadb" => Some(3306),
        "sqlserver" | "mssql" => Some(1433),
        "db2" => Some(50000),
        "informix" => Some(9088),
        _ => None,
    }
}

/// `Some(message)` when the database port refuses or does not answer within 3 s.
///
/// `None` means "go ahead and count": reachable, or not checkable from here (no host,
/// file database, or a host name only the jdbc-admin container can resolve).
pub(crate) async fn probe_endpoint(
    db_type: &str,
    connection: &HashMap<String, String>,
) -> Option<String> {
    let host = connection
        .get("host")
        .map(|h| h.trim())
        .filter(|h| !h.is_empty())?;
    let port = connection
        .get("port")
        .and_then(|p| p.trim().parse::<u16>().ok())
        .or_else(|| default_port(db_type))?;
    match timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect((host, port))).await {
        Ok(Ok(_)) => None,
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => Some(format!(
            "Connection to {host}:{port} refused — not counted (checked before counting)"
        )),
        Ok(Err(e)) => {
            let msg = e.to_string().to_ascii_lowercase();
            // DNS failures here may be specific to this container; let the count decide.
            if msg.contains("lookup") || msg.contains("resolve") || msg.contains("not known") {
                None
            } else {
                Some(format!("could not connect to server {host}:{port}: {e}"))
            }
        }
        Err(_) => Some(format!(
            "could not connect to server {host}:{port}: no answer within {}s — not counted",
            PROBE_TIMEOUT.as_secs()
        )),
    }
}

async fn count_side(
    db_type: &str,
    connection: &HashMap<String, String>,
    schema: &str,
    table: &str,
) -> (Option<i64>, Option<String>) {
    match timeout(
        Duration::from_secs(45),
        connector_catalog::count_table_rows(db_type, connection, schema, table),
    )
    .await
    {
        Ok(Ok(resp)) => (Some(resp.count.max(0)), None),
        Ok(Err(e)) => (None, Some(e.to_string())),
        Err(_) => (None, Some("count timed out after 45s".into())),
    }
}

fn classify_phase_without_source(
    target: Option<i64>,
    kafka: Option<i64>,
    backlog: Option<i64>,
    source_err: &Option<String>,
) -> Option<TableSyncPhase> {
    if let (Some(_), Some(k), Some(b)) = (target, kafka, backlog) {
        if k > 0 && b == 0 {
            return Some(TableSyncPhase::Complete);
        }
        if b > 0 {
            return Some(TableSyncPhase::InProgress);
        }
    }
    if source_err.is_some() {
        return Some(TableSyncPhase::Error);
    }
    None
}

fn classify_phase_with_counts(
    source: Option<i64>,
    target: Option<i64>,
    pending: Option<i64>,
    kafka: Option<i64>,
) -> TableSyncPhase {
    match (source, target, pending) {
        (Some(s), Some(_t), Some(p)) if s > 0 && p == 0 => TableSyncPhase::Complete,
        (Some(s), Some(t), Some(p)) if s > 0 && t == 0 && p == s => TableSyncPhase::InProgress,
        (Some(_), Some(_), Some(p)) if p > 0 => TableSyncPhase::Behind,
        (None, None, _) if kafka.unwrap_or(0) > 0 => TableSyncPhase::InProgress,
        (None, None, _) => TableSyncPhase::Unknown,
        _ if kafka.is_some() && target.is_none() => TableSyncPhase::InProgress,
        _ => TableSyncPhase::Unknown,
    }
}

fn classify_phase(
    source: Option<i64>,
    target: Option<i64>,
    pending: Option<i64>,
    kafka: Option<i64>,
    backlog: Option<i64>,
    source_err: &Option<String>,
    target_err: &Option<String>,
) -> TableSyncPhase {
    if target_err.is_some() {
        return TableSyncPhase::Error;
    }
    if source.is_none() {
        if let Some(phase) = classify_phase_without_source(target, kafka, backlog, source_err) {
            return phase;
        }
    }
    classify_phase_with_counts(source, target, pending, kafka)
}

async fn fetch_connector_offsets(state: &AppState, connector: &str) -> Value {
    let path = format!("/connectors/{}/offsets", urlencoding::encode(connector));
    match connect_request(state, Method::GET, &path, None).await {
        Ok(v) => v,
        Err(e) => json_nullable_error(e.to_string()),
    }
}

fn json_nullable_error(msg: String) -> Value {
    serde_json::json!({ "error": msg })
}

fn parse_source_offsets(raw: Value) -> HashMap<String, Value> {
    let mut out = HashMap::new();
    let Some(items) = raw.as_array() else {
        return out;
    };
    for item in items {
        let Some(partition) = item.get("partition") else {
            continue;
        };
        let table_name = partition
            .get("table")
            .or_else(|| partition.get("table_name"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string());
        let Some(name) = table_name else {
            continue;
        };
        let offset = item.get("offset").cloned().unwrap_or(Value::Null);
        out.insert(name.to_ascii_uppercase(), offset.clone());
        if let Some((_, short)) = name.rsplit_once('.') {
            out.insert(short.trim().to_ascii_uppercase(), offset);
        }
    }
    out
}

/// Consumer lag of the managed sink per topic (`connect-{sink}` group), or `None` when
/// the profile has no managed sink or the group does not exist yet.
///
/// The sink commits its position every `offset.flush.interval.ms` (60 s by default), so
/// a few already-applied messages can still count as lag until the next commit.
async fn sink_lag_by_topic(state: &AppState, profile: &CdcProfile) -> Option<HashMap<String, i64>> {
    if !profile.has_managed_sink() {
        return None;
    }
    let group = format!("connect-{}", sink_connector_name_for(profile));
    let raw = crate::command::run_docker(
        &state.config,
        &[
            "exec",
            state.config.redpanda_container.as_str(),
            "rpk",
            "group",
            "describe",
            group.as_str(),
        ],
    )
    .await
    .ok()?;
    parse_group_lag(&raw)
}

/// Sum `LAG` per `TOPIC` from `rpk group describe`. `None` for a dead / empty group.
fn parse_group_lag(raw: &str) -> Option<HashMap<String, i64>> {
    let mut header: Vec<String> = Vec::new();
    let mut lags: HashMap<String, i64> = HashMap::new();
    for line in raw.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.first() == Some(&"STATE") && cols.get(1) == Some(&"Dead") {
            return None;
        }
        if cols.first() == Some(&"TOPIC") {
            header = cols.iter().map(|c| c.to_ascii_uppercase()).collect();
            continue;
        }
        if header.is_empty() || cols.len() < header.len().min(6) {
            continue;
        }
        let at = |name: &str| {
            header
                .iter()
                .position(|h| h == name)
                .and_then(|i| cols.get(i))
        };
        let (Some(topic), Some(lag)) = (at("TOPIC"), at("LAG").and_then(|v| v.parse::<i64>().ok()))
        else {
            continue;
        };
        *lags.entry((*topic).to_string()).or_default() += lag.max(0);
    }
    (!header.is_empty()).then_some(lags)
}

/// `kafka_error` when the capture topic does not exist (yet). Clients match on it.
pub(crate) const MISSING_TOPIC: &str = "topic not found or no partitions";

async fn topic_message_total(state: &AppState, topic: &str) -> (Option<i64>, Option<String>) {
    let raw = match crate::command::run_docker(
        &state.config,
        &[
            "exec",
            state.config.redpanda_container.as_str(),
            "rpk",
            "topic",
            "describe",
            topic,
            "--print-partitions",
        ],
    )
    .await
    {
        Ok(s) => s,
        Err(e) => return (None, Some(e.to_string())),
    };

    let mut header: Vec<String> = Vec::new();
    let mut total: i64 = 0;
    let mut saw = false;
    for line in raw.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.is_empty() {
            continue;
        }
        if cols[0].eq_ignore_ascii_case("PARTITION") {
            header = cols.iter().map(|c| c.to_ascii_uppercase()).collect();
            continue;
        }
        if cols[0].parse::<i32>().is_err() {
            continue;
        }
        saw = true;
        let start = header
            .iter()
            .position(|h| h == "LOG-START-OFFSET")
            .and_then(|i| cols.get(i).and_then(|v| v.parse::<i64>().ok()));
        let hw = header
            .iter()
            .position(|h| h == "HIGH-WATERMARK")
            .and_then(|i| cols.get(i).and_then(|v| v.parse::<i64>().ok()));
        if let (Some(start), Some(hw)) = (start, hw) {
            if hw >= start {
                total += hw - start;
            }
        }
    }
    if saw {
        (Some(total), None)
    } else {
        // No topic yet is a state, not a count of zero: leave the count unknown so the
        // table shows "—" rather than "0 processing". The message is stable API text.
        (None, Some(MISSING_TOPIC.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TableMapping;

    #[test]
    fn phase_without_source_count_uses_kafka() {
        let none = None;
        let down = Some("connection refused".to_string());
        assert!(matches!(
            classify_phase(None, Some(600), None, Some(1277), Some(0), &none, &none),
            TableSyncPhase::Complete
        ));
        assert!(matches!(
            classify_phase(None, Some(100), None, Some(600), Some(500), &none, &none),
            TableSyncPhase::InProgress
        ));
        // Source down but every captured record is on target: still complete.
        assert!(matches!(
            classify_phase(None, Some(600), None, Some(600), Some(0), &down, &none),
            TableSyncPhase::Complete
        ));
        assert!(matches!(
            classify_phase(None, Some(0), None, Some(0), None, &down, &none),
            TableSyncPhase::Error
        ));
        assert!(matches!(
            classify_phase(None, None, None, Some(0), None, &none, &none),
            TableSyncPhase::Unknown
        ));
    }

    #[tokio::test]
    async fn probe_reports_refused_port_and_passes_open_one() {
        let open = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let open_port = open.local_addr().unwrap().port();
        let closed_port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        }; // dropped: nothing listens there now
        let conn = |port: u16| {
            HashMap::from([
                ("host".to_string(), "127.0.0.1".to_string()),
                ("port".to_string(), port.to_string()),
            ])
        };
        assert!(probe_endpoint("postgres", &conn(open_port)).await.is_none());
        let msg = probe_endpoint("postgres", &conn(closed_port))
            .await
            .unwrap();
        assert!(msg.contains("refused"));
        // The message must land on "Network → source" in the diagnosis.
        let d = crate::diagnose::diagnose(&msg, Side::Source).unwrap();
        assert_eq!(d.area, crate::diagnose::ErrorArea::Network);
        // No host: nothing to check, let the count run.
        assert!(probe_endpoint("postgres", &HashMap::new()).await.is_none());
    }

    #[test]
    fn tables_filter_is_case_insensitive_and_optional() {
        let e = |q: &str| TableEntry {
            source_schema: "public".into(),
            source_table: q.into(),
            target_schema: "t".into(),
            target_table: q.into(),
            source_qualified: format!("public.{q}"),
            target_qualified: format!("t.{q}"),
        };
        let all = || vec![e("groups"), e("group_users"), e("archived_messages")];
        assert_eq!(select_entries(all(), None).len(), 3);
        assert_eq!(select_entries(all(), Some(" , ")).len(), 3);
        let some = select_entries(all(), Some("PUBLIC.Groups, public.archived_messages"));
        let names: Vec<_> = some.iter().map(|x| x.source_table.as_str()).collect();
        assert_eq!(names, vec!["groups", "archived_messages"]);
    }

    #[test]
    fn group_lag_is_summed_per_topic() {
        let raw = "GROUP        connect-x-sink\nSTATE        Stable\nTOTAL-LAG    26\n\nTOPIC   PARTITION  CURRENT-OFFSET  LOG-START-OFFSET  LOG-END-OFFSET  LAG  MEMBER-ID  CLIENT-ID  HOST\nt.a     0          10              0                 17              7    m  c  h\nt.a     1          5               0                 5               0    m  c  h\nt.b     0          0               0                 19              19   m  c  h\n";
        let lags = parse_group_lag(raw).unwrap();
        assert_eq!(lags.get("t.a"), Some(&7));
        assert_eq!(lags.get("t.b"), Some(&19));
        assert!(parse_group_lag("GROUP g\nSTATE        Dead\n").is_none());
    }

    #[test]
    fn debezium_topics_include_schema() {
        let mut p = crate::postgres_trigger_cdc::tests::profile();
        p.source.capture_mode = crate::models::CaptureMode::SyncOnce;
        assert_eq!(
            capture_topic(&p, "cx", "public", "groups"),
            "cx.public.groups"
        );
        p.source.capture_mode = crate::models::CaptureMode::Incremental;
        assert_eq!(
            capture_topic(&p, "cx", "PANDORA", "PM_PRODUCT"),
            "cx.PM_PRODUCT"
        );
    }

    #[test]
    fn failed_task_reports_root_cause() {
        let status = serde_json::json!({
            "connector": {"state": "RUNNING"},
            "tasks": [{"id": 0, "state": "FAILED", "trace":
                "io.debezium.DebeziumException: Couldn't obtain encoding\n\tat x.y(Z.java:1)\nCaused by: org.postgresql.util.PSQLException: attempt failed\nCaused by: java.net.UnknownHostException: pg-src\n\tat a.b"}]
        });
        let out: Vec<String> = describe_connector_status("cdc-x", &status)
            .into_iter()
            .map(|m| m.0)
            .collect();
        assert_eq!(out.len(), 1);
        assert!(out[0].starts_with("cdc-x task 0 FAILED: io.debezium.DebeziumException"));
        assert!(out[0].contains("caused by java.net.UnknownHostException: pg-src"));
    }

    #[test]
    fn paused_connector_is_an_issue_but_running_is_not() {
        let paused = serde_json::json!({"connector": {"state": "PAUSED"}, "tasks": [{"id": 0, "state": "PAUSED"}]});
        assert!(describe_connector_status("s", &paused)[0]
            .0
            .contains("PAUSED"));
        let ok = serde_json::json!({"connector": {"state": "RUNNING"}, "tasks": [{"id": 0, "state": "RUNNING"}]});
        assert!(describe_connector_status("s", &ok).is_empty());
        // A failed task is the reason; don't also report the pause.
        let both = serde_json::json!({"connector": {"state": "PAUSED"}, "tasks": [{"id": 0, "state": "FAILED", "trace": "E: boom"}]});
        assert_eq!(
            describe_connector_status("s", &both)[0].0,
            "s task 0 FAILED: E: boom"
        );
    }

    fn oracle_profile() -> CdcProfile {
        let mut p = crate::postgres_trigger_cdc::tests::profile();
        p.source.source_type = "oracle".into();
        p.source.schemas = vec!["PANDORA".into()];
        p.source.tables = vec!["PANDORA.PM_PRODUCT".into()];
        p
    }

    #[test]
    fn oracle_to_postgres_counts_the_folded_target_table() {
        let mut p = oracle_profile();
        p.flow.mappings = vec![TableMapping {
            source_schema: "PANDORA".into(),
            source_table: "PM_PRODUCT".into(),
            target_schema: "PANDORA".into(),
            target_table: "PM_PRODUCT".into(),
            identical: true,
            columns: vec![],
            pk_columns: vec![],
        }];
        let e = &table_entries(&p)[0];
        assert_eq!(
            (e.target_schema.as_str(), e.target_table.as_str()),
            ("pandora", "pm_product")
        );
        assert_eq!(e.source_qualified, "PANDORA.PM_PRODUCT");

        p.flow.mappings.clear();
        let e = &table_entries(&p)[0];
        assert_eq!(
            (e.target_schema.as_str(), e.target_table.as_str()),
            ("pandora", "pm_product")
        );
    }

    #[test]
    fn postgres_targets_keep_their_case() {
        let mut p = crate::postgres_trigger_cdc::tests::profile();
        p.source.tables = vec!["bookings.Tickets".into()];
        let e = &table_entries(&p)[0];
        assert_eq!(e.target_table, "Tickets");
    }
}
