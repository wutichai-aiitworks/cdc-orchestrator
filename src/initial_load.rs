//! Initial load: copy every row of a pipeline's source tables into its PostgreSQL target
//! once, through a single streaming read per table.
//!
//! Incremental pipelines that poll with a rolling window only ever re-read recent rows,
//! so rows outside the window (e.g. legacy rows whose `UPDATE_DATE` is NULL) reach the
//! target only through this load. It runs in the background because a multi-million-row
//! table takes far longer than any HTTP/proxy timeout:
//!
//! * the source sees one forward-only `SELECT *` per table (`cdc-jdbc-admin /v1/stream`);
//! * rows are inserted with `ON CONFLICT DO NOTHING`, so the load can run beside the live
//!   pipeline (fresher window upserts win) and can be re-run after a failure;
//! * `clear_target` truncates each target table just before its copy starts.
//!
//! Job state is kept in memory on this orchestrator instance; the outcome is also written
//! to the profile history and the audit log.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::connector_catalog::{stream_jdbc_admin_to_postgres, StreamCopyOptions};
use crate::error::AppError;
use crate::models::{CdcProfile, HistoryItem, ProfilePattern};
use crate::postgres_trigger_cdc::{parse_qualified_tables, CapturedTable};
use crate::AppState;

/// Rows per target `INSERT` during an initial load.
const INITIAL_LOAD_BATCH_ROWS: usize = 2000;

pub type InitialLoadRegistry = Arc<RwLock<HashMap<Uuid, InitialLoadJob>>>;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TableState {
    Pending,
    Running,
    Done,
    Failed,
    Skipped,
}

pub struct InitialLoadJob {
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    state: JobState,
    clear_target: bool,
    actor: String,
    error: Option<String>,
    tables: Vec<JobTable>,
    cancel: Arc<AtomicBool>,
}

struct JobTable {
    table: CapturedTable,
    state: TableState,
    rows: Arc<AtomicI64>,
    error: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct InitialLoadRequest {
    /// Truncate each target table before copying into it.
    #[serde(default)]
    pub clear_target: bool,
    /// Subset of `source.tables` (`SCHEMA.TABLE`) to load; empty = every selected table.
    #[serde(default)]
    pub tables: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct InitialLoadStatus {
    pub profile_id: Uuid,
    pub state: JobState,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub clear_target: bool,
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub rows_copied: i64,
    pub tables: Vec<InitialLoadTableStatus>,
}

#[derive(Debug, Serialize)]
pub struct InitialLoadTableStatus {
    pub source: String,
    pub target: String,
    pub state: TableState,
    pub rows_copied: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl InitialLoadJob {
    fn status(&self, profile_id: Uuid) -> InitialLoadStatus {
        let tables: Vec<InitialLoadTableStatus> = self
            .tables
            .iter()
            .map(|t| InitialLoadTableStatus {
                source: format!("{}.{}", t.table.source_schema, t.table.source_table),
                target: format!("{}.{}", t.table.target_schema, t.table.target_table),
                state: t.state,
                rows_copied: t.rows.load(Ordering::Relaxed),
                error: t.error.clone(),
            })
            .collect();
        InitialLoadStatus {
            profile_id,
            state: self.state,
            started_at: self.started_at,
            finished_at: self.finished_at,
            clear_target: self.clear_target,
            actor: self.actor.clone(),
            error: self.error.clone(),
            rows_copied: tables.iter().map(|t| t.rows_copied).sum(),
            tables,
        }
    }
}

/// Tables an initial load of `profile` would copy, honouring an optional subset.
pub fn plan_tables(
    profile: &CdcProfile,
    requested: &[String],
) -> Result<Vec<CapturedTable>, AppError> {
    if profile.pattern != ProfilePattern::SourceToTarget {
        return Err(AppError::BadRequest(
            "initial load applies only to database replica (source_to_target) pipelines".into(),
        ));
    }
    let target = profile
        .target
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("pipeline has no target to load into".into()))?;
    if !matches!(
        target.target_type.trim().to_ascii_lowercase().as_str(),
        "postgres" | "postgresql"
    ) {
        return Err(AppError::BadRequest(format!(
            "initial load supports PostgreSQL targets only, not '{}'",
            target.target_type
        )));
    }
    let all = parse_qualified_tables(profile)?;
    let wanted: Vec<String> = requested
        .iter()
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    if wanted.is_empty() {
        return Ok(all);
    }
    for w in &wanted {
        if !all
            .iter()
            .any(|t| format!("{}.{}", t.source_schema, t.source_table).to_ascii_lowercase() == *w)
        {
            return Err(AppError::BadRequest(format!(
                "'{w}' is not one of this pipeline's selected tables"
            )));
        }
    }
    Ok(all
        .into_iter()
        .filter(|t| {
            wanted.contains(&format!("{}.{}", t.source_schema, t.source_table).to_ascii_lowercase())
        })
        .collect())
}

/// Start a background initial load. Fails if one is already running for this pipeline.
pub async fn start(
    state: &AppState,
    profile: &CdcProfile,
    request: &InitialLoadRequest,
    actor: &str,
) -> Result<InitialLoadStatus, AppError> {
    if !profile.enabled {
        return Err(AppError::BadRequest(
            "profile is disabled; enable it before running an initial load".into(),
        ));
    }
    if !crate::connector_catalog::jdbc_admin_configured() {
        return Err(AppError::BadRequest(
            "initial load streams through cdc-jdbc-admin — set ORCH_JDBC_ADMIN_URL".into(),
        ));
    }
    let tables = plan_tables(profile, &request.tables)?;
    if tables.is_empty() {
        return Err(AppError::BadRequest("no tables to load".into()));
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let status = {
        let mut jobs = state.initial_loads.write().await;
        if jobs
            .get(&profile.id)
            .is_some_and(|job| job.state == JobState::Running)
        {
            return Err(AppError::BadRequest(
                "an initial load is already running for this pipeline".into(),
            ));
        }
        let job = InitialLoadJob {
            started_at: Utc::now(),
            finished_at: None,
            state: JobState::Running,
            clear_target: request.clear_target,
            actor: actor.to_string(),
            error: None,
            tables: tables
                .into_iter()
                .map(|table| JobTable {
                    table,
                    state: TableState::Pending,
                    rows: Arc::new(AtomicI64::new(0)),
                    error: None,
                })
                .collect(),
            cancel: cancel.clone(),
        };
        let status = job.status(profile.id);
        jobs.insert(profile.id, job);
        status
    };

    let task_state = state.clone();
    let profile = profile.clone();
    let clear_target = request.clear_target;
    tokio::spawn(async move {
        run(&task_state, &profile, clear_target, cancel).await;
    });
    Ok(status)
}

pub async fn status(state: &AppState, profile_id: Uuid) -> Option<InitialLoadStatus> {
    state
        .initial_loads
        .read()
        .await
        .get(&profile_id)
        .map(|job| job.status(profile_id))
}

/// Ask a running load to stop; it halts between insert batches.
pub async fn cancel(state: &AppState, profile_id: Uuid) -> Option<InitialLoadStatus> {
    let jobs = state.initial_loads.read().await;
    let job = jobs.get(&profile_id)?;
    if job.state == JobState::Running {
        job.cancel.store(true, Ordering::Relaxed);
    }
    Some(job.status(profile_id))
}

async fn run(state: &AppState, profile: &CdcProfile, clear_target: bool, cancel: Arc<AtomicBool>) {
    let Some(target) = profile.target.as_ref() else {
        return;
    };
    let count = state
        .initial_loads
        .read()
        .await
        .get(&profile.id)
        .map_or(0, |job| job.tables.len());

    let mut failure: Option<String> = None;
    for index in 0..count {
        let (table, rows) = {
            let mut jobs = state.initial_loads.write().await;
            let Some(job) = jobs.get_mut(&profile.id) else {
                return;
            };
            let entry = &mut job.tables[index];
            if failure.is_some() {
                entry.state = TableState::Skipped;
                continue;
            }
            entry.state = TableState::Running;
            (entry.table.clone(), entry.rows.clone())
        };
        let result = stream_jdbc_admin_to_postgres(
            &profile.source.source_type,
            &profile.source.connection,
            &target.connection,
            &table.source_schema,
            &table.source_table,
            &table.target_schema,
            &table.target_table,
            &StreamCopyOptions {
                truncate_before_copy: clear_target,
                max_rows: None,
                batch_rows: INITIAL_LOAD_BATCH_ROWS,
                skip_existing: true,
                progress: Some(rows),
                cancel: Some(cancel.clone()),
            },
        )
        .await;
        let mut jobs = state.initial_loads.write().await;
        let Some(job) = jobs.get_mut(&profile.id) else {
            return;
        };
        let entry = &mut job.tables[index];
        match result {
            Ok(n) => {
                entry.rows.store(n, Ordering::Relaxed);
                entry.state = TableState::Done;
            }
            Err(e) => {
                let msg = e.to_string_message();
                entry.state = TableState::Failed;
                entry.error = Some(msg.clone());
                failure = Some(format!(
                    "{}.{}: {msg}",
                    table.source_schema, table.source_table
                ));
            }
        }
    }

    let summary = {
        let mut jobs = state.initial_loads.write().await;
        let Some(job) = jobs.get_mut(&profile.id) else {
            return;
        };
        job.finished_at = Some(Utc::now());
        job.state = match (&failure, cancel.load(Ordering::Relaxed)) {
            (None, _) => JobState::Succeeded,
            (Some(_), true) => JobState::Cancelled,
            (Some(_), false) => JobState::Failed,
        };
        job.error = failure.clone();
        job.status(profile.id)
    };
    record_outcome(state, profile.id, &summary).await;
}

/// Write the finished load into the profile history and the audit log.
async fn record_outcome(state: &AppState, profile_id: Uuid, summary: &InitialLoadStatus) {
    let per_table = summary
        .tables
        .iter()
        .map(|t| format!("{}={}", t.target, t.rows_copied))
        .collect::<Vec<_>>()
        .join(", ");
    let (level, event, detail) = match summary.state {
        JobState::Succeeded => (
            "info",
            "profile_initial_load",
            format!(
                "Initial load copied {} row(s) across {} table(s) [{per_table}]{}. Existing target rows were kept.",
                summary.rows_copied,
                summary.tables.len(),
                if summary.clear_target { " after clearing the targets" } else { "" }
            ),
        ),
        JobState::Cancelled => (
            "warning",
            "profile_initial_load_cancelled",
            format!(
                "Initial load cancelled after {} row(s) [{per_table}]. Re-run it to finish; rows already copied are kept.",
                summary.rows_copied
            ),
        ),
        _ => (
            "error",
            "profile_initial_load_failed",
            format!(
                "Initial load failed after {} row(s) [{per_table}]: {}. Re-run it to finish; rows already copied are kept.",
                summary.rows_copied,
                summary.error.as_deref().unwrap_or("unknown error")
            ),
        ),
    };
    let mut profile_name = None;
    if let Ok(Some(mut profile)) = state.profiles.get(profile_id).await {
        profile_name = Some(profile.name.clone());
        profile.push_history(HistoryItem {
            timestamp: Utc::now(),
            level: level.to_string(),
            event: event.to_string(),
            detail: detail.clone(),
        });
        profile.updated_at = Utc::now();
        if let Err(e) = state.profiles.upsert(&profile).await {
            tracing::warn!(profile = %profile_id, error = %e, "could not save initial load history");
        }
    }
    let outcome = if summary.state == JobState::Succeeded {
        "success"
    } else {
        "failure"
    };
    crate::audit::emit_system(
        state,
        "profile.initial_load",
        outcome,
        json!({
            "requested_by": summary.actor,
            "clear_target": summary.clear_target,
            "state": summary.state,
            "rows_copied": summary.rows_copied,
            "tables": summary.tables,
            "error": summary.error,
        }),
        Some(profile_id),
        profile_name.as_deref(),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CaptureMode, DeliveryMode, FlowConfig, ProfileStatus, ScheduleConfig, SchemaSelectionMode,
        SourceConfig, TableSelectionMode, TargetConfig, TransformStrategy,
    };

    fn profile(target_type: &str) -> CdcProfile {
        let conn = HashMap::from([
            ("host".to_string(), "db".to_string()),
            ("port".to_string(), "1521".to_string()),
            ("database".to_string(), "pandora".to_string()),
            ("username".to_string(), "u".to_string()),
            ("password".to_string(), "p".to_string()),
        ]);
        CdcProfile {
            autostart: false,
            id: Uuid::new_v4(),
            name: "Oracle replica".into(),
            description: String::new(),
            pattern: ProfilePattern::SourceToTarget,
            stream_platform: None,
            transform_strategy: TransformStrategy::None,
            source: SourceConfig {
                source_type: "oracle".into(),
                connection: conn.clone(),
                capture_mode: CaptureMode::Incremental,
                schema_mode: SchemaSelectionMode::SpecificSchemas,
                table_mode: TableSelectionMode::SpecificTables,
                schemas: vec!["PANDORA".into(), "CRM_STG".into()],
                tables: vec!["PANDORA.PM_PRODUCT".into(), "CRM_STG.PM_PROBRAND".into()],
                changelog_retention_hours: None,
                timestamp_column: Some("UPDATE_DATE".into()),
                incrementing_column: None,
                upsert_key_columns: vec!["BRANCH_NO".into(), "PRO_CODE".into()],
                table_poll: vec![],
                start_position: Default::default(),
            },
            target: Some(TargetConfig {
                name: "replica".into(),
                target_type: target_type.into(),
                connection: conn,
                schemas: vec![],
            }),
            lake: None,
            governance: crate::models::GovernanceConfig::default(),
            sink_error_policy: crate::models::SinkErrorPolicy::FailFast,
            flow: FlowConfig {
                transforms: vec![],
                schedule: ScheduleConfig {
                    delivery_mode: DeliveryMode::MicroBatch,
                    interval_seconds: Some(300),
                    full_refresh_minutes: None,
                },
                mappings: vec![],
            },
            status: ProfileStatus::Stopped,
            enabled: true,
            tags: vec![],
            connector_name: None,
            sink_connector_name: None,
            latency_probe: Default::default(),
            last_error: None,
            history: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn plan_defaults_to_every_selected_table() {
        let tables = plan_tables(&profile("postgres"), &[]).unwrap();
        assert_eq!(tables.len(), 2);
    }

    #[test]
    fn plan_honours_a_case_insensitive_subset() {
        let tables = plan_tables(&profile("postgres"), &["pandora.pm_product".into()]).unwrap();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].source_table, "PM_PRODUCT");
    }

    #[test]
    fn plan_rejects_unknown_tables_and_non_postgres_targets() {
        assert!(plan_tables(&profile("postgres"), &["PANDORA.NOPE".into()]).is_err());
        assert!(plan_tables(&profile("mysql"), &[]).is_err());
    }
}
