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
use crate::error::AppError;
use crate::incremental_cdc;
use crate::models::{CdcProfile, TableMapping};
use crate::reconcile::{connect_request, jdbc_target_schema, source_table_include_list, topic_prefix_for};
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
    pub source_count_error: Option<String>,
    pub target_count_error: Option<String>,
    pub kafka_error: Option<String>,
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
}

#[derive(Debug, serde::Deserialize)]
pub struct TableSyncStatusQuery {
    /// When true, skip `COUNT(*)` on source/target (offsets + Kafka only).
    #[serde(default)]
    pub skip_counts: bool,
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

    let prefix = topic_prefix_for(&profile);
    let offsets_by_table = if let Some(name) = profile.connector_name.as_deref() {
        parse_source_offsets(fetch_connector_offsets(state, name).await)
    } else {
        HashMap::new()
    };

    let mut rows = Vec::with_capacity(entries.len());
    for entry in entries {
        let topic = format!("{prefix}.{}", entry.source_table);
        let (kafka_messages, kafka_error) = topic_message_total(state, &topic).await;

        let source_offset = offsets_by_table
            .get(&entry.source_qualified.to_ascii_uppercase())
            .or_else(|| offsets_by_table.get(&entry.source_table.to_ascii_uppercase()))
            .cloned();

        let (source_row_count, source_count_error) = if q.skip_counts {
            (None, None)
        } else {
            count_side(
                &profile.source.source_type,
                &profile.source.connection,
                &entry.source_schema,
                &entry.source_table,
            )
            .await
        };

        let (target_row_count, target_count_error) = if q.skip_counts {
            (None, None)
        } else if let Some(target) = profile.target.as_ref() {
            count_side(
                &target.target_type,
                &target.connection,
                &entry.target_schema,
                &entry.target_table,
            )
            .await
        } else {
            (None, Some("no target configured".into()))
        };

        let pending_rows = match (source_row_count, target_row_count) {
            (Some(s), Some(t)) => Some((s - t).max(0)),
            _ => None,
        };

        let processing_rows = match (kafka_messages, target_row_count) {
            (Some(k), Some(t)) => Some((k - t).max(0)),
            _ => None,
        };

        let phase = classify_phase(
            source_row_count,
            target_row_count,
            pending_rows,
            kafka_messages,
            &source_count_error,
            &target_count_error,
        );

        rows.push(TableSyncStatusRow {
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
            source_count_error,
            target_count_error,
            kafka_error,
        });
    }

    Ok(ProfileTableSyncStatusResponse {
        timestamp: Utc::now(),
        profile_id: profile.id,
        profile_name: profile.name.clone(),
        profile_status: format!("{:?}", profile.status),
        capture_mode: format!("{:?}", profile.source.capture_mode),
        tables: rows,
        counts_skipped: q.skip_counts,
    })
}

struct TableEntry {
    source_schema: String,
    source_table: String,
    target_schema: String,
    target_table: String,
    source_qualified: String,
    target_qualified: String,
}

fn table_entries(profile: &CdcProfile) -> Vec<TableEntry> {
    if !profile.flow.mappings.is_empty() {
        return profile
            .flow
            .mappings
            .iter()
            .map(mapping_to_entry)
            .collect();
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

fn classify_phase(
    source: Option<i64>,
    target: Option<i64>,
    pending: Option<i64>,
    kafka: Option<i64>,
    source_err: &Option<String>,
    target_err: &Option<String>,
) -> TableSyncPhase {
    if source_err.is_some() || target_err.is_some() {
        return TableSyncPhase::Error;
    }
    match (source, target, pending) {
        (Some(s), Some(_t), Some(p)) if s > 0 && p == 0 => TableSyncPhase::Complete,
        (Some(s), Some(t), Some(p)) if s > 0 && t == 0 && p == s => TableSyncPhase::InProgress,
        (Some(_), Some(_), Some(p)) if p > 0 => TableSyncPhase::Behind,
        (None, None, _) => {
            if kafka.unwrap_or(0) > 0 {
                TableSyncPhase::InProgress
            } else {
                TableSyncPhase::Unknown
            }
        }
        _ => {
            if kafka.is_some() && target.is_none() {
                TableSyncPhase::InProgress
            } else {
                TableSyncPhase::Unknown
            }
        }
    }
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
        (Some(0), Some("topic not found or no partitions".into()))
    }
}
