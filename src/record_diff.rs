//! Record diff: which rows of one pipeline table exist only in the source, only in the
//! target, or (optionally) in both with different values — compared by key column(s).
//!
//! Runs as a background job per requested table (ADR 0013):
//!
//! 1. **Plan** — inspect both tables through cdc-jdbc-admin, map columns (pipeline column
//!    mapping, else case-insensitive names), pick key columns (request → mapping PK →
//!    source PK → target PK).
//! 2. **Scan** — one forward-only `SELECT key, compared… FROM t` per side, both at once,
//!    values in jdbc-admin's canonical form (engine-neutral strings). Each row is hashed on
//!    its key into one of N bucket files on local disk. No `ORDER BY`, so collation
//!    differences between engines (Oracle vs Postgres) cannot fake a difference, and the
//!    databases do no sort work.
//! 3. **Compare** — one bucket at a time in memory (`HashMap` of the source bucket probed by
//!    the target bucket), so memory is bounded by the bucket size, not the table size.
//! 4. **Recheck** — a live pipeline keeps changing both sides during the scan. Each
//!    candidate difference (up to `recheck_limit` per kind) is re-read by key on both sides;
//!    rows that have since converged are dropped and counted as `resolved_in_flight`.
//! 5. **Results** — NDJSON files per kind (`source_only`, `target_only`, `mismatched`),
//!    paged by the API and downloadable as NDJSON or CSV. A `summary.json` beside them
//!    lets finished jobs survive an orchestrator restart.
//!
//! Nothing is ever written to the source or the target.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::hash::{DefaultHasher, Hasher};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, RwLock, Semaphore};
use uuid::Uuid;

use crate::clients::JdbcAdminClient;
use crate::error::AppError;
use crate::models::{
    CdcProfile, ProfilePattern, SchemaSelectionMode, SourceTableColumn, TableMapping,
};
use crate::AppState;

/// Rows per bucket the planner aims for (memory per compared bucket ≈ rows × row size).
const ROWS_PER_BUCKET: i64 = 200_000;
const MIN_BUCKETS: usize = 16;
const MAX_BUCKETS: usize = 512;
/// Rows pulled from a stream per batch.
const SCAN_BATCH: usize = 2000;
/// Keys per `/v1/lookup` call during recheck (jdbc-admin batches its SQL internally).
const RECHECK_CHUNK: usize = 1000;
/// Finished jobs kept per pipeline; older ones (and their files) are deleted.
const KEEP_FINISHED_PER_PROFILE: usize = 20;
/// Most result rows one page request may return.
pub const MAX_PAGE_LIMIT: usize = 500;
/// Separator between key components inside a canonical key (never in normal data).
const KEY_SEP: char = '\u{1f}';

// ── Public request / status types ────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct RecordDiffRequest {
    /// Source table as `SCHEMA.TABLE` — one of the pipeline's tables.
    pub table: String,
    /// Source column names forming the key; empty = mapping PK, else source/target PK.
    #[serde(default)]
    pub key_columns: Vec<String>,
    /// Source columns whose values are compared for rows present on both sides.
    #[serde(default)]
    pub compare_columns: Vec<String>,
    /// Compare every mapped non-key column (overrides `compare_columns`).
    #[serde(default)]
    pub compare_all_columns: bool,
    #[serde(default)]
    pub options: DiffOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffOptions {
    /// Ignore trailing spaces (Oracle/MSSQL `CHAR` padding vs `varchar`). Default on.
    #[serde(default = "yes")]
    pub trim_trailing_spaces: bool,
    /// Compare text case-insensitively (keys and values). Default off.
    #[serde(default)]
    pub case_insensitive: bool,
    /// Re-read candidate differences by key to drop rows that converged during the scan.
    #[serde(default = "yes")]
    pub recheck: bool,
    /// Most candidates rechecked per kind; the rest are reported as `unverified`.
    #[serde(default = "default_recheck_limit")]
    pub recheck_limit: u64,
}

fn yes() -> bool {
    true
}
fn default_recheck_limit() -> u64 {
    10_000
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            trim_trailing_spaces: true,
            case_insensitive: false,
            recheck: true,
            recheck_limit: default_recheck_limit(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiffState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiffPhase {
    Queued,
    Planning,
    Scanning,
    Comparing,
    Rechecking,
    Done,
}

/// Which result file.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiffKind {
    SourceOnly,
    TargetOnly,
    Mismatched,
}

impl DiffKind {
    pub const ALL: [DiffKind; 3] = [
        DiffKind::SourceOnly,
        DiffKind::TargetOnly,
        DiffKind::Mismatched,
    ];

    fn file(self) -> &'static str {
        match self {
            DiffKind::SourceOnly => "source_only.ndjson",
            DiffKind::TargetOnly => "target_only.ndjson",
            DiffKind::Mismatched => "mismatched.ndjson",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DiffKind::SourceOnly => "source_only",
            DiffKind::TargetOnly => "target_only",
            DiffKind::Mismatched => "mismatched",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "source_only" | "source" => Some(DiffKind::SourceOnly),
            "target_only" | "target" => Some(DiffKind::TargetOnly),
            "mismatched" | "changed" => Some(DiffKind::Mismatched),
            _ => None,
        }
    }
}

/// Resolved tables and columns the job compares.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiffPlan {
    pub source_schema: String,
    pub source_table: String,
    pub target_schema: String,
    pub target_table: String,
    /// Where the key came from: `request`, `mapping`, `source_pk` or `target_pk`.
    pub key_source: String,
    pub source_key_columns: Vec<String>,
    pub target_key_columns: Vec<String>,
    pub source_compare_columns: Vec<String>,
    pub target_compare_columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffCounts {
    pub source_rows: u64,
    pub target_rows: u64,
    pub matched: u64,
    pub source_only: u64,
    pub target_only: u64,
    pub mismatched: u64,
    /// Extra rows sharing a key already seen on that side (key not unique there).
    pub source_duplicate_keys: u64,
    pub target_duplicate_keys: u64,
    /// Rows skipped because a key column is NULL.
    pub source_null_keys: u64,
    pub target_null_keys: u64,
    /// Candidates that converged by the time they were rechecked (in-flight replication).
    pub resolved_in_flight: u64,
    /// Reported differences that were not rechecked (recheck off or over the limit).
    pub unverified: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffProgress {
    pub source_rows_read: u64,
    pub target_rows_read: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_rows_estimate: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_rows_estimate: Option<u64>,
    pub buckets_done: u64,
    pub buckets_total: u64,
    pub recheck_done: u64,
    pub recheck_total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffJobStatus {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub profile_name: String,
    /// Requested source table (`SCHEMA.TABLE`).
    pub table: String,
    pub state: DiffState,
    pub phase: DiffPhase,
    pub actor: String,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub options: DiffOptions,
    /// Request as sent (columns) so the UI can re-run it.
    pub requested_key_columns: Vec<String>,
    pub requested_compare_columns: Vec<String>,
    pub compare_all_columns: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<DiffPlan>,
    pub counts: DiffCounts,
    pub progress: DiffProgress,
}

impl DiffJobStatus {
    fn is_active(&self) -> bool {
        matches!(self.state, DiffState::Queued | DiffState::Running)
    }
}

// ── Registry ─────────────────────────────────────────────────────────────────

struct DiffJob {
    status: DiffJobStatus,
    cancel: Arc<AtomicBool>,
    live: Arc<LiveProgress>,
}

#[derive(Default)]
struct LiveProgress {
    source_read: AtomicU64,
    target_read: AtomicU64,
    buckets_done: AtomicU64,
    recheck_done: AtomicU64,
}

impl DiffJob {
    fn snapshot(&self) -> DiffJobStatus {
        let mut s = self.status.clone();
        if s.state == DiffState::Running {
            let p = &mut s.progress;
            p.source_rows_read = p
                .source_rows_read
                .max(self.live.source_read.load(Ordering::Relaxed));
            p.target_rows_read = p
                .target_rows_read
                .max(self.live.target_read.load(Ordering::Relaxed));
            p.buckets_done = p
                .buckets_done
                .max(self.live.buckets_done.load(Ordering::Relaxed));
            p.recheck_done = p
                .recheck_done
                .max(self.live.recheck_done.load(Ordering::Relaxed));
        }
        s
    }
}

/// All record-diff jobs on this instance plus the concurrency gate.
pub struct RecordDiffs {
    jobs: RwLock<HashMap<Uuid, DiffJob>>,
    slots: Arc<Semaphore>,
    dir: PathBuf,
}

pub type RecordDiffRegistry = Arc<RecordDiffs>;

impl RecordDiffs {
    /// `ORCH_RECORD_DIFF_DIR` (default `<tmp>/cdc-record-diff`) and
    /// `ORCH_RECORD_DIFF_CONCURRENCY` (default 2 jobs at once; the rest queue).
    pub fn from_env() -> RecordDiffRegistry {
        let dir = std::env::var("ORCH_RECORD_DIFF_DIR")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("cdc-record-diff"));
        let slots = std::env::var("ORCH_RECORD_DIFF_CONCURRENCY")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(2);
        Arc::new(Self::new(dir, slots))
    }

    fn new(dir: PathBuf, slots: usize) -> Self {
        Self {
            jobs: RwLock::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(slots)),
            dir,
        }
    }

    fn job_dir(&self, id: Uuid) -> PathBuf {
        self.dir.join(id.to_string())
    }

    /// Reload finished jobs from `summary.json` files. A job that was still running when
    /// the orchestrator stopped is marked failed (its work files are removed).
    pub async fn restore(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut jobs = self.jobs.write().await;
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(raw) = fs::read(path.join("summary.json")) else {
                continue;
            };
            let Ok(mut status) = serde_json::from_slice::<DiffJobStatus>(&raw) else {
                continue;
            };
            if status.is_active() {
                status.state = DiffState::Failed;
                status.phase = DiffPhase::Done;
                status.error =
                    Some("the orchestrator restarted while this diff was running".into());
                status.finished_at.get_or_insert_with(Utc::now);
                let _ = fs::remove_dir_all(path.join("work"));
                let _ = write_summary(&path, &status);
            }
            jobs.insert(
                status.id,
                DiffJob {
                    status,
                    cancel: Arc::new(AtomicBool::new(false)),
                    live: Arc::new(LiveProgress::default()),
                },
            );
        }
        if !jobs.is_empty() {
            tracing::info!(jobs = jobs.len(), dir = %self.dir.display(), "restored record diff jobs");
        }
    }
}

// ── API entry points ─────────────────────────────────────────────────────────

/// Queue a diff of one table. Refused when that table already has an active diff.
pub async fn start(
    state: &AppState,
    profile: &CdcProfile,
    request: RecordDiffRequest,
    actor: &str,
) -> Result<DiffJobStatus, AppError> {
    if profile.pattern != ProfilePattern::SourceToTarget || profile.target.is_none() {
        return Err(AppError::BadRequest(
            "record diff applies to database replica (source_to_target) pipelines".into(),
        ));
    }
    if !crate::connector_catalog::jdbc_admin_configured() {
        return Err(AppError::BadRequest(
            "record diff reads both databases through cdc-jdbc-admin — set ORCH_JDBC_ADMIN_URL"
                .into(),
        ));
    }
    let resolved = resolve_table(profile, &request.table)?;
    let table = format!("{}.{}", resolved.source_schema, resolved.source_table);
    let registry = state.record_diffs.clone();

    let id = Uuid::new_v4();
    let cancel = Arc::new(AtomicBool::new(false));
    let live = Arc::new(LiveProgress::default());
    let status = DiffJobStatus {
        id,
        profile_id: profile.id,
        profile_name: profile.name.clone(),
        table: table.clone(),
        state: DiffState::Queued,
        phase: DiffPhase::Queued,
        actor: actor.to_string(),
        created_at: Utc::now(),
        started_at: None,
        finished_at: None,
        error: None,
        options: request.options.clone(),
        requested_key_columns: clean(&request.key_columns),
        requested_compare_columns: clean(&request.compare_columns),
        compare_all_columns: request.compare_all_columns,
        plan: None,
        counts: DiffCounts::default(),
        progress: DiffProgress::default(),
    };
    {
        let mut jobs = registry.jobs.write().await;
        if jobs.values().any(|j| {
            j.status.profile_id == profile.id
                && j.status.is_active()
                && j.status.table.eq_ignore_ascii_case(&table)
        }) {
            return Err(AppError::Conflict(format!(
                "a record diff of {table} is already running for this pipeline"
            )));
        }
        jobs.insert(
            id,
            DiffJob {
                status: status.clone(),
                cancel: cancel.clone(),
                live: live.clone(),
            },
        );
    }

    let task_state = state.clone();
    let profile = profile.clone();
    tokio::spawn(async move {
        let registry = task_state.record_diffs.clone();
        let _permit = tokio::select! {
            permit = registry.slots.clone().acquire_owned() => permit.ok(),
            _ = wait_cancel(cancel.clone()) => None,
        };
        let outcome = if cancel.load(Ordering::Relaxed) {
            Err(AppError::BadRequest("cancelled before it started".into()))
        } else {
            update(&registry, id, |s| {
                s.state = DiffState::Running;
                s.phase = DiffPhase::Planning;
                s.started_at = Some(Utc::now());
            })
            .await;
            run(
                &registry,
                id,
                &profile,
                &resolved,
                &request,
                cancel.clone(),
                live.clone(),
            )
            .await
        };
        finish(&task_state, id, outcome, cancel.load(Ordering::Relaxed)).await;
    });
    Ok(status)
}

pub async fn get(state: &AppState, profile_id: Uuid, job_id: Uuid) -> Option<DiffJobStatus> {
    state
        .record_diffs
        .jobs
        .read()
        .await
        .get(&job_id)
        .filter(|j| j.status.profile_id == profile_id)
        .map(DiffJob::snapshot)
}

/// Jobs of one pipeline, newest first.
pub async fn list(state: &AppState, profile_id: Uuid) -> Vec<DiffJobStatus> {
    let jobs = state.record_diffs.jobs.read().await;
    let mut out: Vec<DiffJobStatus> = jobs
        .values()
        .filter(|j| j.status.profile_id == profile_id)
        .map(DiffJob::snapshot)
        .collect();
    out.sort_by_key(|b| std::cmp::Reverse(b.created_at));
    out
}

/// Ask an active job to stop (it halts between batches / buckets).
pub async fn cancel(state: &AppState, profile_id: Uuid, job_id: Uuid) -> Option<DiffJobStatus> {
    let jobs = state.record_diffs.jobs.read().await;
    let job = jobs
        .get(&job_id)
        .filter(|j| j.status.profile_id == profile_id)?;
    if job.status.is_active() {
        job.cancel.store(true, Ordering::Relaxed);
    }
    Some(job.snapshot())
}

/// Delete a finished job and its files. Active jobs must be cancelled first.
pub async fn delete(state: &AppState, profile_id: Uuid, job_id: Uuid) -> Result<(), AppError> {
    let registry = &state.record_diffs;
    let mut jobs = registry.jobs.write().await;
    let job = jobs
        .get(&job_id)
        .filter(|j| j.status.profile_id == profile_id)
        .ok_or_else(|| AppError::NotFound("record diff not found".into()))?;
    if job.status.is_active() {
        return Err(AppError::Conflict(
            "cancel the record diff before deleting it".into(),
        ));
    }
    jobs.remove(&job_id);
    let dir = registry.job_dir(job_id);
    tokio::task::spawn_blocking(move || fs::remove_dir_all(dir))
        .await
        .ok();
    Ok(())
}

/// Path of a finished job's result file, for paging and download.
pub async fn result_file(
    state: &AppState,
    profile_id: Uuid,
    job_id: Uuid,
    kind: DiffKind,
) -> Result<(DiffJobStatus, PathBuf), AppError> {
    let status = get(state, profile_id, job_id)
        .await
        .ok_or_else(|| AppError::NotFound("record diff not found".into()))?;
    if status.state != DiffState::Succeeded {
        return Err(AppError::BadRequest(format!(
            "results are available once the diff succeeds (it is {:?})",
            status.state
        )));
    }
    let path = state.record_diffs.job_dir(job_id).join(kind.file());
    Ok((status, path))
}

#[derive(Debug, Serialize)]
pub struct DiffRowsPage {
    pub kind: DiffKind,
    pub offset: usize,
    pub limit: usize,
    pub total: u64,
    pub rows: Vec<Value>,
}

/// One page of a result file (`offset` rows skipped, up to `limit` returned).
pub async fn rows_page(
    state: &AppState,
    profile_id: Uuid,
    job_id: Uuid,
    kind: DiffKind,
    offset: usize,
    limit: usize,
) -> Result<DiffRowsPage, AppError> {
    let (status, path) = result_file(state, profile_id, job_id, kind).await?;
    let limit = limit.clamp(1, MAX_PAGE_LIMIT);
    let total = match kind {
        DiffKind::SourceOnly => status.counts.source_only,
        DiffKind::TargetOnly => status.counts.target_only,
        DiffKind::Mismatched => status.counts.mismatched,
    };
    let rows = tokio::task::spawn_blocking(move || read_page(&path, offset, limit))
        .await
        .map_err(|e| AppError::Internal(format!("record diff page: {e}")))??;
    Ok(DiffRowsPage {
        kind,
        offset,
        limit,
        total,
        rows,
    })
}

fn read_page(path: &Path, offset: usize, limit: usize) -> Result<Vec<Value>, AppError> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_err("open result", e)),
    };
    let mut out = Vec::with_capacity(limit);
    for line in BufReader::new(file).lines().skip(offset).take(limit) {
        let line = line.map_err(|e| io_err("read result", e))?;
        if let Ok(v) = serde_json::from_str::<Value>(&line) {
            out.push(v);
        }
    }
    Ok(out)
}

/// One result line as a CSV record: key columns, status, then `column|source|target`
/// triples for mismatches (header from [`csv_header`]).
pub fn csv_line(plan: &DiffPlan, kind: DiffKind, line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line).ok()?;
    let key_cols = match kind {
        DiffKind::TargetOnly => &plan.target_key_columns,
        _ => &plan.source_key_columns,
    };
    let mut fields: Vec<String> = key_cols
        .iter()
        .map(|c| v["key"][c].as_str().unwrap_or("").to_string())
        .collect();
    fields.push(v["status"].as_str().unwrap_or("").to_string());
    if kind == DiffKind::Mismatched {
        let diffs = v["differences"]
            .as_array()
            .map(|d| {
                d.iter()
                    .map(|x| {
                        format!(
                            "{}: {} → {}",
                            x["column"].as_str().unwrap_or(""),
                            x["source"].as_str().unwrap_or("NULL"),
                            x["target"].as_str().unwrap_or("NULL")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();
        fields.push(diffs);
    }
    Some(
        fields
            .iter()
            .map(|f| csv_field(f))
            .collect::<Vec<_>>()
            .join(",")
            + "\n",
    )
}

pub fn csv_header(plan: &DiffPlan, kind: DiffKind) -> String {
    let mut cols: Vec<String> = match kind {
        DiffKind::TargetOnly => plan.target_key_columns.clone(),
        _ => plan.source_key_columns.clone(),
    };
    cols.push("status".into());
    if kind == DiffKind::Mismatched {
        cols.push("differences".into());
    }
    cols.iter()
        .map(|f| csv_field(f))
        .collect::<Vec<_>>()
        .join(",")
        + "\n"
}

fn csv_field(value: &str) -> String {
    // Leading = + - @ would run as a formula in spreadsheet apps.
    let guarded = if value.starts_with(['=', '+', '-', '@']) {
        format!("'{value}")
    } else {
        value.to_string()
    };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

// ── Table and column resolution ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ResolvedTable {
    pub source_schema: String,
    pub source_table: String,
    pub target_schema: String,
    pub target_table: String,
    mapping: Option<TableMapping>,
}

/// Map a requested `SCHEMA.TABLE` to the pipeline's source and target table.
pub fn resolve_table(profile: &CdcProfile, requested: &str) -> Result<ResolvedTable, AppError> {
    let wanted = requested.trim();
    let (schema, table) = wanted
        .rsplit_once('.')
        .filter(|(s, t)| !s.trim().is_empty() && !t.trim().is_empty())
        .ok_or_else(|| AppError::BadRequest("table must be SCHEMA.TABLE".into()))?;
    let mapping_for = |s: &str, t: &str| {
        profile
            .flow
            .mappings
            .iter()
            .find(|m| {
                m.source_qualified()
                    .eq_ignore_ascii_case(&format!("{s}.{t}"))
            })
            .cloned()
    };

    // Pipelines with an explicit table list: the table must be one of them.
    let include = crate::reconcile::source_table_include_list(profile);
    if !include.is_empty() {
        let tables = crate::postgres_trigger_cdc::parse_qualified_tables(profile)?;
        let hit = tables
            .into_iter()
            .find(|t| {
                format!("{}.{}", t.source_schema, t.source_table).eq_ignore_ascii_case(wanted)
            })
            .ok_or_else(|| {
                AppError::BadRequest(format!("'{wanted}' is not one of this pipeline's tables"))
            })?;
        let mapping = mapping_for(&hit.source_schema, &hit.source_table);
        return Ok(ResolvedTable {
            source_schema: hit.source_schema,
            source_table: hit.source_table,
            target_schema: hit.target_schema,
            target_table: hit.target_table,
            mapping,
        });
    }

    // All tables of the selected schemas: any table in those schemas, same names on the target.
    let schema = schema.trim();
    let table = table.trim();
    if profile.source.schema_mode == SchemaSelectionMode::SpecificSchemas
        && !profile
            .source
            .schemas
            .iter()
            .any(|s| s.trim().eq_ignore_ascii_case(schema))
    {
        return Err(AppError::BadRequest(format!(
            "schema '{schema}' is not captured by this pipeline"
        )));
    }
    let mapping = mapping_for(schema, table);
    let (target_schema, target_table) = match &mapping {
        Some(m) if !m.identical && !m.target_table.trim().is_empty() => (
            if m.target_schema.trim().is_empty() {
                schema.to_string()
            } else {
                m.target_schema.trim().to_string()
            },
            m.target_table.trim().to_string(),
        ),
        _ => (schema.to_string(), table.to_string()),
    };
    let (target_schema, target_table) = crate::postgres_trigger_cdc::fold_oracle_postgres_target(
        profile,
        &target_schema,
        &target_table,
    );
    Ok(ResolvedTable {
        source_schema: schema.to_string(),
        source_table: table.to_string(),
        target_schema,
        target_table,
        mapping,
    })
}

fn clean(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

/// Decide key and compared columns from both tables' metadata.
pub fn plan_columns(
    resolved: &ResolvedTable,
    request: &RecordDiffRequest,
    source: (&[SourceTableColumn], &[String]),
    target: (&[SourceTableColumn], &[String]),
) -> Result<DiffPlan, AppError> {
    let (src_cols, src_pk) = source;
    let (tgt_cols, tgt_pk) = target;
    if src_cols.is_empty() {
        return Err(AppError::BadRequest(format!(
            "source table {}.{} has no columns (does it exist?)",
            resolved.source_schema, resolved.source_table
        )));
    }
    if tgt_cols.is_empty() {
        return Err(AppError::BadRequest(format!(
            "target table {}.{} has no columns (does it exist?)",
            resolved.target_schema, resolved.target_table
        )));
    }
    let src_name = |name: &str| {
        src_cols
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name.trim()))
            .map(|c| c.name.clone())
    };
    let tgt_name = |name: &str| {
        tgt_cols
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name.trim()))
            .map(|c| c.name.clone())
    };

    // Source column → (target column, has transform), from the mapping when it renames.
    let mapped: HashMap<String, (String, bool)> = resolved
        .mapping
        .as_ref()
        .filter(|m| !m.identical)
        .map(|m| {
            m.columns
                .iter()
                .filter(|c| c.effective_sources().len() == 1 && !c.target.trim().is_empty())
                .map(|c| {
                    (
                        c.effective_sources()[0].trim().to_ascii_lowercase(),
                        (
                            c.target.trim().to_string(),
                            c.transform.as_deref().is_some_and(|t| !t.trim().is_empty()),
                        ),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let to_target = |src: &str| -> Option<(String, bool)> {
        match mapped.get(&src.to_ascii_lowercase()) {
            Some((t, transformed)) => tgt_name(t).map(|t| (t, *transformed)),
            None => tgt_name(src).map(|t| (t, false)),
        }
    };
    let to_source = |tgt: &str| -> Option<String> {
        mapped
            .iter()
            .find(|(_, (t, _))| t.eq_ignore_ascii_case(tgt))
            .and_then(|(s, _)| src_name(s))
            .or_else(|| src_name(tgt))
    };

    // Key columns, as source names.
    let requested = clean(&request.key_columns);
    let mapping_pk: Vec<String> = resolved
        .mapping
        .as_ref()
        .map(|m| clean(&m.pk_columns))
        .unwrap_or_default();
    let (key_source, src_keys): (&str, Vec<String>) = if !requested.is_empty() {
        let keys = requested
            .iter()
            .map(|k| {
                src_name(k).ok_or_else(|| {
                    AppError::BadRequest(format!("key column '{k}' is not a source column"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        ("request", keys)
    } else if !mapping_pk.is_empty() {
        let keys = mapping_pk
            .iter()
            .map(|k| {
                to_source(k).ok_or_else(|| {
                    AppError::BadRequest(format!(
                        "mapping key column '{k}' has no matching source column"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        ("mapping", keys)
    } else if !src_pk.is_empty() {
        (
            "source_pk",
            src_pk.iter().filter_map(|k| src_name(k)).collect(),
        )
    } else if !tgt_pk.is_empty() {
        let keys = tgt_pk
            .iter()
            .map(|k| {
                to_source(k).ok_or_else(|| {
                    AppError::BadRequest(format!(
                        "target primary-key column '{k}' has no matching source column"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        ("target_pk", keys)
    } else {
        return Err(AppError::BadRequest(
            "neither table has a primary key — choose the key column(s) to compare by".into(),
        ));
    };
    let mut seen = HashSet::new();
    let src_keys: Vec<String> = src_keys
        .into_iter()
        .filter(|k| seen.insert(k.to_ascii_lowercase()))
        .collect();
    if src_keys.is_empty() {
        return Err(AppError::BadRequest("no key columns to compare by".into()));
    }
    let mut tgt_keys = Vec::with_capacity(src_keys.len());
    for k in &src_keys {
        match to_target(k) {
            Some((_, true)) => {
                return Err(AppError::BadRequest(format!(
                    "key column '{k}' is transformed on its way to the target; keys must be copied as-is"
                )))
            }
            Some((t, false)) => tgt_keys.push(t),
            None => {
                return Err(AppError::BadRequest(format!(
                    "key column '{k}' has no matching target column"
                )))
            }
        }
    }

    // Compared columns.
    let mut warnings = Vec::new();
    let mut src_compare = Vec::new();
    let mut tgt_compare = Vec::new();
    let is_key = |c: &str| src_keys.iter().any(|k| k.eq_ignore_ascii_case(c));
    if request.compare_all_columns {
        let mut ordered: Vec<&SourceTableColumn> = src_cols.iter().collect();
        ordered.sort_by_key(|c| c.position);
        for col in ordered {
            if is_key(&col.name) {
                continue;
            }
            match to_target(&col.name) {
                Some((_, true)) => warnings.push(format!(
                    "{} skipped: transformed on its way to the target",
                    col.name
                )),
                Some((t, false)) => {
                    src_compare.push(col.name.clone());
                    tgt_compare.push(t);
                }
                None => warnings.push(format!("{} skipped: no matching target column", col.name)),
            }
        }
    } else {
        for c in clean(&request.compare_columns) {
            let s = src_name(&c).ok_or_else(|| {
                AppError::BadRequest(format!("compare column '{c}' is not a source column"))
            })?;
            if is_key(&s)
                || src_compare
                    .iter()
                    .any(|x: &String| x.eq_ignore_ascii_case(&s))
            {
                continue;
            }
            match to_target(&s) {
                Some((t, transformed)) => {
                    if transformed {
                        warnings.push(format!(
                            "{s} is transformed on its way to the target; expect value differences"
                        ));
                    }
                    src_compare.push(s);
                    tgt_compare.push(t);
                }
                None => {
                    return Err(AppError::BadRequest(format!(
                        "compare column '{c}' has no matching target column"
                    )))
                }
            }
        }
    }

    Ok(DiffPlan {
        source_schema: resolved.source_schema.clone(),
        source_table: resolved.source_table.clone(),
        target_schema: resolved.target_schema.clone(),
        target_table: resolved.target_table.clone(),
        key_source: key_source.to_string(),
        source_key_columns: src_keys,
        target_key_columns: tgt_keys,
        source_compare_columns: src_compare,
        target_compare_columns: tgt_compare,
        warnings,
    })
}

// ── Job runner ───────────────────────────────────────────────────────────────

async fn wait_cancel(cancel: Arc<AtomicBool>) {
    while !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

async fn update(registry: &RecordDiffs, id: Uuid, f: impl FnOnce(&mut DiffJobStatus)) {
    if let Some(job) = registry.jobs.write().await.get_mut(&id) {
        f(&mut job.status);
    }
}

fn cancelled(cancel: &AtomicBool) -> Result<(), AppError> {
    if cancel.load(Ordering::Relaxed) {
        Err(AppError::BadRequest("record diff cancelled".into()))
    } else {
        Ok(())
    }
}

fn io_err(what: &str, e: std::io::Error) -> AppError {
    AppError::Internal(format!("record diff {what}: {e}"))
}

#[allow(clippy::too_many_arguments)]
async fn run(
    registry: &RecordDiffs,
    id: Uuid,
    profile: &CdcProfile,
    resolved: &ResolvedTable,
    request: &RecordDiffRequest,
    cancel: Arc<AtomicBool>,
    live: Arc<LiveProgress>,
) -> Result<(DiffPlan, DiffCounts), AppError> {
    let admin = JdbcAdminClient::from_env()
        .ok_or_else(|| AppError::BadRequest("cdc-jdbc-admin is not configured".into()))?;
    let target = profile
        .target
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("pipeline has no target".into()))?;
    let src_type = profile.source.source_type.as_str();
    let tgt_type = target.target_type.as_str();
    let src_conn = &profile.source.connection;
    let tgt_conn = &target.connection;

    // 1. Plan.
    let (src_meta, tgt_meta, src_count, tgt_count) = tokio::join!(
        admin.inspect(
            src_type,
            src_conn,
            &resolved.source_schema,
            &resolved.source_table
        ),
        admin.inspect(
            tgt_type,
            tgt_conn,
            &resolved.target_schema,
            &resolved.target_table
        ),
        admin.count_table(
            src_type,
            src_conn,
            &resolved.source_schema,
            &resolved.source_table
        ),
        admin.count_table(
            tgt_type,
            tgt_conn,
            &resolved.target_schema,
            &resolved.target_table
        ),
    );
    let src_meta = src_meta.map_err(|e| prefix("source", e))?;
    let tgt_meta = tgt_meta.map_err(|e| prefix("target", e))?;
    let plan = plan_columns(
        resolved,
        request,
        (&src_meta.columns, &src_meta.primary_key),
        (&tgt_meta.columns, &tgt_meta.primary_key),
    )?;
    let src_est = src_count.ok().map(|c| c.count.max(0) as u64);
    let tgt_est = tgt_count.ok().map(|c| c.count.max(0) as u64);
    let largest = src_est.unwrap_or(0).max(tgt_est.unwrap_or(0)) as i64;
    let buckets = ((largest / ROWS_PER_BUCKET) as usize + 1).clamp(MIN_BUCKETS, MAX_BUCKETS);

    let job_dir = registry.job_dir(id);
    let work = job_dir.join("work");
    fs::create_dir_all(&work).map_err(|e| io_err("create work dir", e))?;
    update(registry, id, |s| {
        s.plan = Some(plan.clone());
        s.phase = DiffPhase::Scanning;
        s.progress.source_rows_estimate = src_est;
        s.progress.target_rows_estimate = tgt_est;
        s.progress.buckets_total = buckets as u64;
    })
    .await;
    if let Some(snapshot) = registry.jobs.read().await.get(&id).map(DiffJob::snapshot) {
        let _ = write_summary(&job_dir, &snapshot);
    }

    let norm = Normalizer::from(&request.options);
    let result = async {
        // 2. Scan both sides at once into bucket files.
        let src_cols: Vec<String> = plan
            .source_key_columns
            .iter()
            .chain(&plan.source_compare_columns)
            .cloned()
            .collect();
        let tgt_cols: Vec<String> = plan
            .target_key_columns
            .iter()
            .chain(&plan.target_compare_columns)
            .cloned()
            .collect();
        let key_len = plan.source_key_columns.len();
        let (src_scan, tgt_scan) = tokio::join!(
            scan_side(
                &admin,
                src_type,
                src_conn,
                &plan.source_schema,
                &plan.source_table,
                &src_cols,
                key_len,
                work.join("src"),
                buckets,
                norm,
                &live.source_read,
                &cancel,
            ),
            scan_side(
                &admin,
                tgt_type,
                tgt_conn,
                &plan.target_schema,
                &plan.target_table,
                &tgt_cols,
                key_len,
                work.join("tgt"),
                buckets,
                norm,
                &live.target_read,
                &cancel,
            ),
        );
        let src_scan = src_scan.map_err(|e| prefix("source", e))?;
        let tgt_scan = tgt_scan.map_err(|e| prefix("target", e))?;
        cancelled(&cancel)?;

        // 3. Compare bucket by bucket.
        update(registry, id, |s| {
            s.phase = DiffPhase::Comparing;
            s.progress.source_rows_read = src_scan.rows;
            s.progress.target_rows_read = tgt_scan.rows;
        })
        .await;
        let compare_work = work.clone();
        let compare_cancel = cancel.clone();
        let compare_live = live.clone();
        let mut counts = tokio::task::spawn_blocking(move || {
            compare_buckets(&compare_work, buckets, norm, &compare_cancel, &compare_live)
        })
        .await
        .map_err(|e| AppError::Internal(format!("record diff compare: {e}")))??;
        counts.source_rows = src_scan.rows;
        counts.target_rows = tgt_scan.rows;
        counts.source_null_keys = src_scan.null_keys;
        counts.target_null_keys = tgt_scan.null_keys;
        cancelled(&cancel)?;

        // 4. Recheck candidates and write the result files.
        let recheck_total = if request.options.recheck {
            DiffKind::ALL
                .iter()
                .map(|k| candidate_count(&counts, *k).min(request.options.recheck_limit))
                .sum()
        } else {
            0
        };
        update(registry, id, |s| {
            s.phase = DiffPhase::Rechecking;
            s.progress.buckets_done = buckets as u64;
            s.progress.recheck_total = recheck_total;
        })
        .await;
        let ctx = RecheckCtx {
            admin: &admin,
            plan: &plan,
            src_type,
            tgt_type,
            src_conn,
            tgt_conn,
            norm,
            options: &request.options,
            cancel: &cancel,
            live: &live,
        };
        let mut finals = counts.clone();
        finals.source_only = 0;
        finals.target_only = 0;
        finals.mismatched = 0;
        for kind in DiffKind::ALL {
            let (kept, resolved_n, unverified) = recheck_kind(
                &ctx,
                kind,
                &work.join(cand_file(kind)),
                &job_dir.join(kind.file()),
            )
            .await?;
            match kind {
                DiffKind::SourceOnly => finals.source_only = kept,
                DiffKind::TargetOnly => finals.target_only = kept,
                DiffKind::Mismatched => finals.mismatched = kept,
            }
            finals.resolved_in_flight += resolved_n;
            finals.unverified += unverified;
        }
        Ok::<DiffCounts, AppError>(finals)
    }
    .await;
    let _ = fs::remove_dir_all(&work);
    result.map(|counts| (plan, counts))
}

fn prefix(side: &str, e: AppError) -> AppError {
    AppError::BadRequest(format!("{side}: {}", e.to_string_message()))
}

fn candidate_count(counts: &DiffCounts, kind: DiffKind) -> u64 {
    match kind {
        DiffKind::SourceOnly => counts.source_only,
        DiffKind::TargetOnly => counts.target_only,
        DiffKind::Mismatched => counts.mismatched,
    }
}

async fn finish(
    state: &AppState,
    id: Uuid,
    outcome: Result<(DiffPlan, DiffCounts), AppError>,
    was_cancelled: bool,
) {
    let registry = &state.record_diffs;
    let job_dir = registry.job_dir(id);
    let snapshot = {
        let mut jobs = registry.jobs.write().await;
        let Some(job) = jobs.get_mut(&id) else {
            return;
        };
        let mut status = job.snapshot();
        status.phase = DiffPhase::Done;
        status.finished_at = Some(Utc::now());
        match outcome {
            Ok((plan, counts)) => {
                status.state = DiffState::Succeeded;
                status.plan = Some(plan);
                status.progress.recheck_done = status.progress.recheck_total;
                status.counts = counts;
            }
            Err(e) => {
                status.state = if was_cancelled {
                    DiffState::Cancelled
                } else {
                    DiffState::Failed
                };
                status.error = Some(e.to_string_message());
                for kind in DiffKind::ALL {
                    let _ = fs::remove_file(job_dir.join(kind.file()));
                }
            }
        }
        job.status = status.clone();
        status
    };
    if let Err(e) = fs::create_dir_all(&job_dir).and_then(|_| write_summary(&job_dir, &snapshot)) {
        tracing::warn!(job = %id, error = %e, "could not write record diff summary");
    }
    prune(state, snapshot.profile_id).await;

    let outcome = if snapshot.state == DiffState::Succeeded {
        "success"
    } else {
        "failure"
    };
    crate::audit::emit_system(
        state,
        "profile.record_diff",
        outcome,
        json!({
            "job_id": snapshot.id,
            "table": snapshot.table,
            "requested_by": snapshot.actor,
            "state": snapshot.state,
            "key_columns": snapshot.plan.as_ref().map(|p| p.source_key_columns.clone()),
            "counts": snapshot.counts,
            "error": snapshot.error,
        }),
        Some(snapshot.profile_id),
        Some(snapshot.profile_name.as_str()),
    )
    .await;
}

/// Keep the newest finished jobs per pipeline; delete the rest and their files.
async fn prune(state: &AppState, profile_id: Uuid) {
    let registry = &state.record_diffs;
    let doomed: Vec<Uuid> = {
        let mut jobs = registry.jobs.write().await;
        let mut finished: Vec<(Uuid, DateTime<Utc>)> = jobs
            .values()
            .filter(|j| j.status.profile_id == profile_id && !j.status.is_active())
            .map(|j| (j.status.id, j.status.created_at))
            .collect();
        finished.sort_by_key(|b| std::cmp::Reverse(b.1));
        let doomed: Vec<Uuid> = finished
            .into_iter()
            .skip(KEEP_FINISHED_PER_PROFILE)
            .map(|(id, _)| id)
            .collect();
        for id in &doomed {
            jobs.remove(id);
        }
        doomed
    };
    for id in doomed {
        let _ = fs::remove_dir_all(registry.job_dir(id));
    }
}

fn write_summary(dir: &Path, status: &DiffJobStatus) -> std::io::Result<()> {
    let tmp = dir.join("summary.json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(status).unwrap_or_default())?;
    fs::rename(tmp, dir.join("summary.json"))
}

// ── Normalisation ────────────────────────────────────────────────────────────

/// Comparison options applied on top of jdbc-admin's canonical values.
#[derive(Debug, Clone, Copy)]
struct Normalizer {
    trim: bool,
    fold_case: bool,
}

impl From<&DiffOptions> for Normalizer {
    fn from(o: &DiffOptions) -> Self {
        Self {
            trim: o.trim_trailing_spaces,
            fold_case: o.case_insensitive,
        }
    }
}

impl Normalizer {
    fn value(self, v: Option<&str>) -> Option<String> {
        let v = v?;
        let v = if self.trim {
            v.trim_end_matches(' ')
        } else {
            v
        };
        Some(if self.fold_case {
            v.to_lowercase()
        } else {
            v.to_string()
        })
    }

    /// Canonical key string, or `None` when a key component is NULL.
    fn key(self, parts: &[Option<String>]) -> Option<String> {
        let mut out = String::new();
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                out.push(KEY_SEP);
            }
            out.push_str(&self.value(p.as_deref())?);
        }
        Some(out)
    }
}

fn bucket_of(key: &str, buckets: usize) -> usize {
    let mut h = DefaultHasher::new();
    h.write(key.as_bytes());
    (h.finish() % buckets as u64) as usize
}

fn as_text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

// ── Scan ─────────────────────────────────────────────────────────────────────

struct ScanResult {
    rows: u64,
    null_keys: u64,
}

/// One bucket line: `[canonical_key, [raw key…], [raw compared value…]]`.
#[derive(Serialize, Deserialize)]
struct BucketRow(String, Vec<Option<String>>, Vec<Option<String>>);

#[allow(clippy::too_many_arguments)]
async fn scan_side(
    admin: &JdbcAdminClient,
    db_type: &str,
    conn: &HashMap<String, String>,
    schema: &str,
    table: &str,
    columns: &[String],
    key_len: usize,
    dir: PathBuf,
    buckets: usize,
    norm: Normalizer,
    read: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<ScanResult, AppError> {
    let mut stream = admin
        .open_stream_with(db_type, conn, schema, table, None, columns, true)
        .await?;
    if stream.columns().len() != columns.len() {
        return Err(AppError::Upstream(format!(
            "{schema}.{table}: expected {} columns, got {} — is cdc-jdbc-admin up to date?",
            columns.len(),
            stream.columns().len()
        )));
    }
    // Disk writes happen on a blocking thread; batches cross a bounded channel.
    let (tx, mut rx) = mpsc::channel::<Vec<Vec<Value>>>(4);
    let writer = tokio::task::spawn_blocking(move || -> Result<ScanResult, AppError> {
        fs::create_dir_all(&dir).map_err(|e| io_err("create bucket dir", e))?;
        let mut files = (0..buckets)
            .map(|b| {
                File::create(dir.join(format!("{b:04}.ndjson")))
                    .map(|f| BufWriter::with_capacity(16 * 1024, f))
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| io_err("create bucket", e))?;
        let mut result = ScanResult {
            rows: 0,
            null_keys: 0,
        };
        while let Some(batch) = rx.blocking_recv() {
            for row in batch {
                result.rows += 1;
                let raw: Vec<Option<String>> = row.iter().map(as_text).collect();
                let (key_raw, values) = raw.split_at(key_len.min(raw.len()));
                let Some(key) = norm.key(key_raw) else {
                    result.null_keys += 1;
                    continue;
                };
                let b = bucket_of(&key, buckets);
                let line =
                    serde_json::to_string(&BucketRow(key, key_raw.to_vec(), values.to_vec()))
                        .map_err(|e| AppError::Internal(format!("record diff encode: {e}")))?;
                files[b]
                    .write_all(line.as_bytes())
                    .and_then(|_| files[b].write_all(b"\n"))
                    .map_err(|e| io_err("write bucket", e))?;
            }
        }
        for f in &mut files {
            f.flush().map_err(|e| io_err("flush bucket", e))?;
        }
        Ok(result)
    });

    let mut read_err = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            read_err = Some(AppError::BadRequest("record diff cancelled".into()));
            break;
        }
        match stream.next_batch(SCAN_BATCH).await {
            Ok(rows) if rows.is_empty() => break,
            Ok(rows) => {
                read.fetch_add(rows.len() as u64, Ordering::Relaxed);
                if tx.send(rows).await.is_err() {
                    break; // writer failed; its error is reported below
                }
            }
            Err(e) => {
                read_err = Some(e);
                break;
            }
        }
    }
    drop(tx);
    let written = writer
        .await
        .map_err(|e| AppError::Internal(format!("record diff writer: {e}")))?;
    match read_err {
        Some(e) => Err(e),
        None => written,
    }
}

// ── Compare ──────────────────────────────────────────────────────────────────

/// Candidate line written by compare: raw keys of both sides plus raw values.
#[derive(Serialize, Deserialize)]
struct Candidate {
    /// Raw key (canonical, before options) from the side the row was found on.
    key: Vec<Option<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    source: Vec<Option<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    target: Vec<Option<String>>,
}

fn cand_file(kind: DiffKind) -> String {
    format!("cand_{}", kind.file())
}

fn compare_buckets(
    work: &Path,
    buckets: usize,
    norm: Normalizer,
    cancel: &AtomicBool,
    live: &LiveProgress,
) -> Result<DiffCounts, AppError> {
    let open = |kind: DiffKind| -> Result<BufWriter<File>, AppError> {
        File::create(work.join(cand_file(kind)))
            .map(BufWriter::new)
            .map_err(|e| io_err("create candidates", e))
    };
    let mut out_src = open(DiffKind::SourceOnly)?;
    let mut out_tgt = open(DiffKind::TargetOnly)?;
    let mut out_mis = open(DiffKind::Mismatched)?;
    let mut counts = DiffCounts::default();
    let emit = |w: &mut BufWriter<File>, c: &Candidate| -> Result<(), AppError> {
        let line = serde_json::to_string(c).map_err(|e| AppError::Internal(e.to_string()))?;
        w.write_all(line.as_bytes())
            .and_then(|_| w.write_all(b"\n"))
            .map_err(|e| io_err("write candidates", e))
    };

    for b in 0..buckets {
        cancelled(cancel)?;
        let src_path = work.join("src").join(format!("{b:04}.ndjson"));
        let tgt_path = work.join("tgt").join(format!("{b:04}.ndjson"));
        let mut source: HashMap<String, BucketRow> = HashMap::new();
        for line in read_lines(&src_path)? {
            let row: BucketRow = serde_json::from_str(&line.map_err(|e| io_err("read bucket", e))?)
                .map_err(|e| AppError::Internal(format!("record diff bucket: {e}")))?;
            if source.contains_key(&row.0) {
                counts.source_duplicate_keys += 1;
            } else {
                source.insert(row.0.clone(), row);
            }
        }
        let mut target_seen: HashSet<String> = HashSet::new();
        for line in read_lines(&tgt_path)? {
            let row: BucketRow = serde_json::from_str(&line.map_err(|e| io_err("read bucket", e))?)
                .map_err(|e| AppError::Internal(format!("record diff bucket: {e}")))?;
            if !target_seen.insert(row.0.clone()) {
                counts.target_duplicate_keys += 1;
                continue;
            }
            match source.remove(&row.0) {
                Some(src) => {
                    if values_differ(norm, &src.2, &row.2) {
                        counts.mismatched += 1;
                        emit(
                            &mut out_mis,
                            &Candidate {
                                key: src.1,
                                source: src.2,
                                target: row.2,
                            },
                        )?;
                    } else {
                        counts.matched += 1;
                    }
                }
                None => {
                    counts.target_only += 1;
                    emit(
                        &mut out_tgt,
                        &Candidate {
                            key: row.1,
                            source: vec![],
                            target: row.2,
                        },
                    )?;
                }
            }
        }
        for (_, src) in source {
            counts.source_only += 1;
            emit(
                &mut out_src,
                &Candidate {
                    key: src.1,
                    source: src.2,
                    target: vec![],
                },
            )?;
        }
        let _ = fs::remove_file(&src_path);
        let _ = fs::remove_file(&tgt_path);
        live.buckets_done.fetch_add(1, Ordering::Relaxed);
    }
    for w in [&mut out_src, &mut out_tgt, &mut out_mis] {
        w.flush().map_err(|e| io_err("flush candidates", e))?;
    }
    Ok(counts)
}

fn read_lines(path: &Path) -> Result<std::io::Lines<BufReader<File>>, AppError> {
    File::open(path)
        .map(|f| BufReader::with_capacity(64 * 1024, f).lines())
        .map_err(|e| io_err("open bucket", e))
}

fn values_differ(norm: Normalizer, a: &[Option<String>], b: &[Option<String>]) -> bool {
    a.len() != b.len()
        || a.iter()
            .zip(b)
            .any(|(x, y)| norm.value(x.as_deref()) != norm.value(y.as_deref()))
}

// ── Recheck and results ──────────────────────────────────────────────────────

struct RecheckCtx<'a> {
    admin: &'a JdbcAdminClient,
    plan: &'a DiffPlan,
    src_type: &'a str,
    tgt_type: &'a str,
    src_conn: &'a HashMap<String, String>,
    tgt_conn: &'a HashMap<String, String>,
    norm: Normalizer,
    options: &'a DiffOptions,
    cancel: &'a AtomicBool,
    live: &'a LiveProgress,
}

/// Re-read one side by key: canonical key → compared values.
async fn lookup_side(
    ctx: &RecheckCtx<'_>,
    source_side: bool,
    keys: &[Vec<Option<String>>],
) -> Result<HashMap<String, Vec<Option<String>>>, AppError> {
    let plan = ctx.plan;
    let (db_type, conn, schema, table, key_cols, cmp_cols) = if source_side {
        (
            ctx.src_type,
            ctx.src_conn,
            &plan.source_schema,
            &plan.source_table,
            &plan.source_key_columns,
            &plan.source_compare_columns,
        )
    } else {
        (
            ctx.tgt_type,
            ctx.tgt_conn,
            &plan.target_schema,
            &plan.target_table,
            &plan.target_key_columns,
            &plan.target_compare_columns,
        )
    };
    let columns: Vec<String> = key_cols.iter().chain(cmp_cols).cloned().collect();
    let found = ctx
        .admin
        .lookup(db_type, conn, schema, table, key_cols, &columns, keys)
        .await
        .map_err(|e| {
            prefix(
                if source_side {
                    "source recheck"
                } else {
                    "target recheck"
                },
                e,
            )
        })?;
    let n = key_cols.len();
    Ok(found
        .rows
        .into_iter()
        .filter_map(|row| {
            let (k, v) = row.split_at(n.min(row.len()));
            ctx.norm.key(k).map(|key| (key, v.to_vec()))
        })
        .collect())
}

/// Recheck one kind's candidates and write its result file.
/// Returns (reported, resolved in flight, unverified).
async fn recheck_kind(
    ctx: &RecheckCtx<'_>,
    kind: DiffKind,
    candidates: &Path,
    out_path: &Path,
) -> Result<(u64, u64, u64), AppError> {
    let mut out = BufWriter::new(File::create(out_path).map_err(|e| io_err("create result", e))?);
    let mut reported = 0u64;
    let mut resolved = 0u64;
    let mut unverified = 0u64;
    let mut checked = 0u64;
    let limit = if ctx.options.recheck {
        ctx.options.recheck_limit
    } else {
        0
    };
    let plan = ctx.plan;
    let key_names = match kind {
        DiffKind::TargetOnly => &plan.target_key_columns,
        _ => &plan.source_key_columns,
    };

    let mut lines = read_lines(candidates)?;
    loop {
        cancelled(ctx.cancel)?;
        let mut chunk: Vec<Candidate> = Vec::with_capacity(RECHECK_CHUNK);
        for line in lines.by_ref().take(RECHECK_CHUNK) {
            let c: Candidate =
                serde_json::from_str(&line.map_err(|e| io_err("read candidates", e))?)
                    .map_err(|e| AppError::Internal(format!("record diff candidate: {e}")))?;
            chunk.push(c);
        }
        if chunk.is_empty() {
            break;
        }
        // Only the first `limit` candidates are rechecked.
        let verify_n = (limit.saturating_sub(checked) as usize).min(chunk.len());
        let (verify, rest) = chunk.split_at(verify_n);
        let mut lines_out: Vec<Value> = Vec::with_capacity(chunk.len());

        if !verify.is_empty() {
            let keys: Vec<Vec<Option<String>>> = verify.iter().map(|c| c.key.clone()).collect();
            let (src_now, tgt_now) = tokio::try_join!(
                lookup_side(ctx, true, &keys),
                lookup_side(ctx, false, &keys)
            )?;
            for c in verify {
                let Some(k) = ctx.norm.key(&c.key) else {
                    continue;
                };
                let (in_src, in_tgt) = (src_now.get(&k), tgt_now.get(&k));
                let still = match kind {
                    DiffKind::SourceOnly => in_src.is_some() && in_tgt.is_none(),
                    DiffKind::TargetOnly => in_tgt.is_some() && in_src.is_none(),
                    DiffKind::Mismatched => match (in_src, in_tgt) {
                        (Some(s), Some(t)) => values_differ(ctx.norm, s, t),
                        _ => false,
                    },
                };
                if !still {
                    resolved += 1;
                    continue;
                }
                let fresh = match kind {
                    DiffKind::Mismatched => Candidate {
                        key: c.key.clone(),
                        source: in_src.cloned().unwrap_or_default(),
                        target: in_tgt.cloned().unwrap_or_default(),
                    },
                    _ => Candidate {
                        key: c.key.clone(),
                        source: c.source.clone(),
                        target: c.target.clone(),
                    },
                };
                lines_out.push(result_line(ctx, kind, key_names, &fresh, "confirmed"));
            }
            checked += verify.len() as u64;
            ctx.live
                .recheck_done
                .fetch_add(verify.len() as u64, Ordering::Relaxed);
        }
        for c in rest {
            unverified += 1;
            lines_out.push(result_line(ctx, kind, key_names, c, "unverified"));
        }
        for v in lines_out {
            reported += 1;
            out.write_all(v.to_string().as_bytes())
                .and_then(|_| out.write_all(b"\n"))
                .map_err(|e| io_err("write result", e))?;
        }
    }
    out.flush().map_err(|e| io_err("flush result", e))?;
    Ok((reported, resolved, unverified))
}

fn result_line(
    ctx: &RecheckCtx<'_>,
    kind: DiffKind,
    key_names: &[String],
    c: &Candidate,
    status: &str,
) -> Value {
    let plan = ctx.plan;
    let key: serde_json::Map<String, Value> = key_names
        .iter()
        .zip(&c.key)
        .map(|(n, v)| (n.clone(), v.clone().map_or(Value::Null, Value::String)))
        .collect();
    let mut line = json!({ "key": key, "status": status });
    match kind {
        DiffKind::Mismatched => {
            let diffs: Vec<Value> = plan
                .source_compare_columns
                .iter()
                .zip(&plan.target_compare_columns)
                .enumerate()
                .filter_map(|(i, (s, t))| {
                    let a = c.source.get(i).cloned().flatten();
                    let b = c.target.get(i).cloned().flatten();
                    (ctx.norm.value(a.as_deref()) != ctx.norm.value(b.as_deref())).then(
                        || json!({ "column": s, "target_column": t, "source": a, "target": b }),
                    )
                })
                .collect();
            line["differences"] = Value::Array(diffs);
        }
        DiffKind::SourceOnly | DiffKind::TargetOnly => {
            let (names, values) = if kind == DiffKind::SourceOnly {
                (&plan.source_compare_columns, &c.source)
            } else {
                (&plan.target_compare_columns, &c.target)
            };
            if !names.is_empty() {
                let row: serde_json::Map<String, Value> = names
                    .iter()
                    .zip(values)
                    .map(|(n, v)| (n.clone(), v.clone().map_or(Value::Null, Value::String)))
                    .collect();
                line["values"] = Value::Object(row);
            }
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ColumnMapping;

    fn col(name: &str, pos: i32, pk: bool) -> SourceTableColumn {
        serde_json::from_value(json!({
            "name": name, "data_type": "varchar", "nullable": true, "position": pos,
            "primary_key": pk, "unique": pk
        }))
        .unwrap()
    }

    fn resolved(mapping: Option<TableMapping>) -> ResolvedTable {
        ResolvedTable {
            source_schema: "PANDORA".into(),
            source_table: "PM_PRODUCT".into(),
            target_schema: "pandora".into(),
            target_table: "pm_product".into(),
            mapping,
        }
    }

    fn req(keys: &[&str], compare: &[&str], all: bool) -> RecordDiffRequest {
        RecordDiffRequest {
            table: "PANDORA.PM_PRODUCT".into(),
            key_columns: keys.iter().map(|s| s.to_string()).collect(),
            compare_columns: compare.iter().map(|s| s.to_string()).collect(),
            compare_all_columns: all,
            options: DiffOptions::default(),
        }
    }

    #[test]
    fn plan_defaults_to_source_pk_and_matches_names_case_insensitively() {
        let src = [
            col("BRANCH_NO", 1, true),
            col("PRO_CODE", 2, true),
            col("NAME", 3, false),
        ];
        let tgt = [
            col("branch_no", 1, true),
            col("pro_code", 2, true),
            col("name", 3, false),
        ];
        let plan = plan_columns(
            &resolved(None),
            &req(&[], &[], true),
            (&src, &["BRANCH_NO".into(), "PRO_CODE".into()]),
            (&tgt, &[]),
        )
        .unwrap();
        assert_eq!(plan.key_source, "source_pk");
        assert_eq!(plan.target_key_columns, vec!["branch_no", "pro_code"]);
        assert_eq!(plan.source_compare_columns, vec!["NAME"]);
        assert_eq!(plan.target_compare_columns, vec!["name"]);
    }

    #[test]
    fn plan_follows_column_mapping_and_skips_transformed_columns() {
        let mapping = TableMapping {
            source_schema: "PANDORA".into(),
            source_table: "PM_PRODUCT".into(),
            target_schema: "pandora".into(),
            target_table: "pm_product".into(),
            identical: false,
            columns: vec![
                ColumnMapping {
                    source: "PRO_CODE".into(),
                    sources: vec![],
                    target: "product_code".into(),
                    data_type: String::new(),
                    transform: None,
                    transform_args: None,
                },
                ColumnMapping {
                    source: "NAME".into(),
                    sources: vec![],
                    target: "name".into(),
                    data_type: String::new(),
                    transform: Some("upper".into()),
                    transform_args: None,
                },
            ],
            pk_columns: vec!["product_code".into()],
        };
        let src = [col("PRO_CODE", 1, false), col("NAME", 2, false)];
        let tgt = [col("product_code", 1, false), col("name", 2, false)];
        let plan = plan_columns(
            &resolved(Some(mapping)),
            &req(&[], &[], true),
            (&src, &[]),
            (&tgt, &[]),
        )
        .unwrap();
        assert_eq!(plan.key_source, "mapping");
        assert_eq!(plan.source_key_columns, vec!["PRO_CODE"]);
        assert_eq!(plan.target_key_columns, vec!["product_code"]);
        assert!(plan.source_compare_columns.is_empty());
        assert_eq!(plan.warnings.len(), 1);
    }

    #[test]
    fn plan_requires_a_key_and_known_columns() {
        let src = [col("A", 1, false)];
        let tgt = [col("a", 1, false)];
        assert!(plan_columns(
            &resolved(None),
            &req(&[], &[], false),
            (&src, &[]),
            (&tgt, &[])
        )
        .is_err());
        assert!(plan_columns(
            &resolved(None),
            &req(&["NOPE"], &[], false),
            (&src, &[]),
            (&tgt, &[])
        )
        .is_err());
        let ok = plan_columns(
            &resolved(None),
            &req(&["a"], &[], false),
            (&src, &[]),
            (&tgt, &[]),
        )
        .unwrap();
        assert_eq!(ok.key_source, "request");
        assert_eq!(ok.source_key_columns, vec!["A"]);
    }

    #[test]
    fn normalizer_handles_padding_case_and_null_keys() {
        let n = Normalizer {
            trim: true,
            fold_case: true,
        };
        assert_eq!(
            n.key(&[Some("AB  ".into()), Some("1".into())]),
            n.key(&[Some("ab".into()), Some("1".into())])
        );
        assert_eq!(n.key(&[Some("a".into()), None]), None);
        let strict = Normalizer {
            trim: false,
            fold_case: false,
        };
        assert_ne!(
            strict.key(&[Some("AB ".into())]),
            strict.key(&[Some("AB".into())])
        );
        // Separator keeps ("a","bc") and ("ab","c") apart.
        assert_ne!(
            strict.key(&[Some("a".into()), Some("bc".into())]),
            strict.key(&[Some("ab".into()), Some("c".into())])
        );
    }

    fn write_bucket(dir: &Path, b: usize, rows: &[(&str, &str)]) {
        fs::create_dir_all(dir).unwrap();
        let mut f = File::create(dir.join(format!("{b:04}.ndjson"))).unwrap();
        for (k, v) in rows {
            let row = BucketRow(
                k.to_string(),
                vec![Some(k.to_string())],
                vec![Some(v.to_string())],
            );
            writeln!(f, "{}", serde_json::to_string(&row).unwrap()).unwrap();
        }
    }

    #[test]
    fn compare_finds_both_directions_mismatches_and_duplicates() {
        let work = std::env::temp_dir().join(format!("rd-test-{}", Uuid::new_v4()));
        write_bucket(
            &work.join("src"),
            0,
            &[("1", "a"), ("2", "b"), ("3", "c"), ("3", "dup")],
        );
        write_bucket(
            &work.join("tgt"),
            0,
            &[("2", "b"), ("3", "CHANGED"), ("4", "d"), ("4", "d")],
        );
        let counts = compare_buckets(
            &work,
            1,
            Normalizer {
                trim: true,
                fold_case: false,
            },
            &AtomicBool::new(false),
            &LiveProgress::default(),
        )
        .unwrap();
        assert_eq!(counts.matched, 1);
        assert_eq!(counts.source_only, 1);
        assert_eq!(counts.target_only, 1);
        assert_eq!(counts.mismatched, 1);
        assert_eq!(counts.source_duplicate_keys, 1);
        assert_eq!(counts.target_duplicate_keys, 1);
        let src_only = fs::read_to_string(work.join(cand_file(DiffKind::SourceOnly))).unwrap();
        assert!(src_only.contains("\"1\""));
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn bucket_assignment_is_stable_and_in_range() {
        for k in ["a", "b", "1\u{1f}2", ""] {
            let b = bucket_of(k, 64);
            assert!(b < 64);
            assert_eq!(b, bucket_of(k, 64));
        }
    }

    #[test]
    fn csv_guards_formulas_and_quotes() {
        assert_eq!(csv_field("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("plain"), "plain");
    }
}
