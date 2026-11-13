//! Copy & Sync timeline (ADR 0009): an audit trail per profile of the six tools (pgcopy, pgdump,
//! pgloader, mongocopy, redisshake, rsync).
//!
//! Recorded by the handlers: profile created / updated (field diff) / deleted, run started or
//! refused, stop requested, connection tested. Recorded by the run watcher, which reads every
//! sidecar's job list: run finished (status, exit code, error, duration), stamped with the job's
//! own times so the timeline shows when things happened, not when they were noticed.
//!
//! Run events have deterministic ids (tool + job + kind) and are inserted with INSERT OR IGNORE,
//! so watcher polls and restarts never duplicate them. The sidecars keep only the newest runs;
//! this table keeps the history (retention by age and row count). No secrets: values of keys that
//! look like secrets are masked, and run details copy a whitelist of job fields.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::Json;
use chrono::{DateTime, SecondsFormat, Utc};
use hyper_cdc_orchestrator::persist::ToolEventRow;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

use crate::error::AppError;
use crate::AppState;

pub const TOOLS: [&str; 6] = [
    "pgcopy",
    "pgdump",
    "pgloader",
    "mongocopy",
    "redisshake",
    "rsync",
];
const MEM_CAP: usize = 5_000;
const MAX_CHANGES: usize = 60;
/// Keys never recorded in a diff or snapshot (bookkeeping, not configuration).
const IGNORED_KEYS: [&str; 4] = ["updated_at", "created_at", "last_job_id", "id"];
/// Job fields copied into a run.finished event. Connection objects (source/target) are left out.
/// Log lines kept with a failed or stopped run (the sidecar prunes old runs and their logs).
const LOG_TAIL_LINES: usize = 25;
const LOG_TAIL_MAX_CHARS: usize = 6_000;
const JOB_FIELDS: [&str; 19] = [
    "status",
    "exit_code",
    "error",
    "created_at",
    "started_at",
    "finished_at",
    "phase",
    "dry_run",
    "resumed_from",
    "stats",
    "transport",
    "transport_reason",
    "size_bytes",
    "bytes",
    "tables",
    "uploads",
    "mode",
    "snapshot",
    "emptied",
];

/// How a run is named in the timeline: "Preview" (rsync dry run), "One-time sync" / "CDC"
/// (Redis sync start modes), else "Run".
pub fn run_label(job: &Value) -> &'static str {
    if job.get("dry_run").and_then(Value::as_bool) == Some(true) {
        return "Preview";
    }
    match job.get("mode").and_then(Value::as_str) {
        Some("once") => "One-time sync",
        Some("cdc") => "CDC",
        _ => "Run",
    }
}

// ── in-memory log (no rqlite, or rqlite write failed) ───────────────────────

#[derive(Clone, Default)]
pub struct MemoryLog {
    inner: Arc<RwLock<MemInner>>,
}

#[derive(Default)]
struct MemInner {
    events: VecDeque<ToolEventRow>,
    ids: HashSet<String>,
}

impl MemoryLog {
    async fn insert(&self, e: ToolEventRow) {
        let mut g = self.inner.write().await;
        if !g.ids.insert(e.event_id.clone()) {
            return;
        }
        g.events.push_back(e);
        while g.events.len() > MEM_CAP {
            if let Some(old) = g.events.pop_front() {
                g.ids.remove(&old.event_id);
            }
        }
    }

    async fn list(&self, f: &EventFilter) -> Vec<ToolEventRow> {
        let g = self.inner.read().await;
        g.events.iter().filter(|e| f.matches(e)).cloned().collect()
    }
}

// ── building and recording events ───────────────────────────────────────────

/// RFC 3339, UTC, milliseconds, `Z`: one format so text order is time order in SQL.
pub fn norm_ts(raw: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(raw.trim()).ok().map(|t| {
        t.with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Millis, true)
    })
}

fn now_ts() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn run_event_id(tool: &str, job_id: &str, kind: &str) -> String {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("hyper-cdc:{tool}:{job_id}:{kind}").as_bytes(),
    )
    .to_string()
}

fn base(tool: &str, kind: &str, actor: &str) -> ToolEventRow {
    ToolEventRow {
        event_id: Uuid::new_v4().to_string(),
        timestamp: now_ts(),
        tool: tool.to_string(),
        profile_id: None,
        profile_name: None,
        job_id: None,
        kind: kind.to_string(),
        status: None,
        actor: actor.to_string(),
        summary: String::new(),
        detail: "{}".into(),
    }
}

/// Persist (rqlite) or keep in memory. Never fails the caller's request.
pub async fn record(state: &AppState, e: ToolEventRow) {
    info!(tool = %e.tool, kind = %e.kind, status = ?e.status, profile_id = ?e.profile_id,
          job_id = ?e.job_id, actor = %e.actor, summary = %e.summary, "tool_event");
    match state.rqlite.as_ref() {
        Some(rqlite) => {
            if let Err(err) = rqlite.insert_tool_event(&e).await {
                warn!(error = %err, kind = %e.kind, "tool event rqlite write failed; kept in memory");
                state.tool_events_mem.insert(e).await;
            }
        }
        None => state.tool_events_mem.insert(e).await,
    }
}

fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "secret",
        "token",
        "private_key",
        "passphrase",
        "api_key",
        "credential",
    ]
    .iter()
    .any(|s| k.contains(s))
}

/// Masks values of secret-looking keys (any depth). Empty values stay empty (no login is a fact).
pub fn redact(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                if is_secret_key(k) {
                    let empty = val.is_null() || val.as_str().is_some_and(str::is_empty);
                    if !empty {
                        *val = json!("********");
                    }
                } else {
                    redact(val);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact),
        _ => {}
    }
}

fn snapshot<P: Serialize>(profile: &P) -> Value {
    let mut v = serde_json::to_value(profile).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut v {
        for k in IGNORED_KEYS {
            map.remove(k);
        }
    }
    redact(&mut v);
    v
}

fn blank(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// Leaf-level differences: `[{field, before, after}]`; secrets as `{field, secret: true}`.
/// Arrays compare as a whole. Treats missing, null, "" and [] as the same (no change).
pub fn changes(before: &Value, after: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    diff_into("", before, after, &mut out);
    out.truncate(MAX_CHANGES);
    out
}

fn diff_into(path: &str, a: &Value, b: &Value, out: &mut Vec<Value>) {
    if let (Value::Object(ma), Value::Object(mb)) = (a, b) {
        diff_objects(path, ma, mb, out);
        return;
    }
    if changed(a, b) {
        let (mut va, mut vb) = (a.clone(), b.clone());
        redact(&mut va);
        redact(&mut vb);
        out.push(json!({ "field": path, "before": va, "after": vb }));
    }
}

fn diff_objects(
    path: &str,
    ma: &Map<String, Value>,
    mb: &Map<String, Value>,
    out: &mut Vec<Value>,
) {
    let mut keys: Vec<&String> = ma.keys().chain(mb.keys()).collect();
    keys.sort();
    keys.dedup();
    let top = path.is_empty();
    for k in keys {
        if top && IGNORED_KEYS.contains(&k.as_str()) {
            continue;
        }
        let field = if top {
            k.clone()
        } else {
            format!("{path}.{k}")
        };
        let va = ma.get(k).unwrap_or(&Value::Null);
        let vb = mb.get(k).unwrap_or(&Value::Null);
        if !is_secret_key(k) {
            diff_into(&field, va, vb, out);
        } else if changed(va, vb) {
            out.push(json!({ "field": field, "secret": true }));
        }
    }
}

/// Different, counting missing, null, "" and [] as the same.
fn changed(a: &Value, b: &Value) -> bool {
    a != b && !(blank(a) && blank(b))
}

/// "1 h 05 m", "3 m 12 s", "4 s" — the same style as the UI's formatDuration.
pub fn fmt_duration(secs: i64) -> String {
    let s = secs.max(0);
    match s {
        0..=59 => format!("{s} s"),
        60..=3599 => format!("{} m {:02} s", s / 60, s % 60),
        _ => format!("{} h {:02} m", s / 3600, (s % 3600) / 60),
    }
}

fn short(msg: &str, max: usize) -> String {
    let one = msg.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        one
    } else {
        format!("{}…", one.chars().take(max).collect::<String>())
    }
}

// ── what the handlers call ──────────────────────────────────────────────────

pub async fn profile_created<P: Serialize>(
    state: &AppState,
    tool: &str,
    actor: &str,
    id: Uuid,
    name: &str,
    profile: &P,
) {
    let mut e = base(tool, "profile.created", actor);
    e.profile_id = Some(id.to_string());
    e.profile_name = Some(name.to_string());
    e.summary = format!("Profile created by {actor}");
    e.detail = json!({ "profile": snapshot(profile) }).to_string();
    record(state, e).await;
}

pub async fn profile_updated<P: Serialize>(
    state: &AppState,
    tool: &str,
    actor: &str,
    id: Uuid,
    name: &str,
    before: &P,
    after: &P,
) {
    let list = changes(&snapshot_raw(before), &snapshot_raw(after));
    let mut e = base(tool, "profile.updated", actor);
    e.profile_id = Some(id.to_string());
    e.profile_name = Some(name.to_string());
    e.summary = if list.is_empty() {
        format!("Saved by {actor}, nothing changed")
    } else {
        let fields: Vec<String> = list
            .iter()
            .filter_map(|c| c["field"].as_str().map(str::to_string))
            .collect();
        format!(
            "{} changed by {actor}: {}",
            plural(fields.len(), "setting"),
            short(&fields.join(", "), 140)
        )
    };
    e.detail = json!({ "changes": list }).to_string();
    record(state, e).await;
}

/// Unredacted snapshot for diffing: `changes` masks secrets itself, and needs the real values to
/// notice that a password changed.
fn snapshot_raw<P: Serialize>(profile: &P) -> Value {
    let mut v = serde_json::to_value(profile).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut v {
        for k in IGNORED_KEYS {
            map.remove(k);
        }
    }
    v
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

pub async fn profile_deleted(
    state: &AppState,
    tool: &str,
    actor: &str,
    id: Uuid,
    name: Option<&str>,
) {
    let mut e = base(tool, "profile.deleted", actor);
    e.profile_id = Some(id.to_string());
    e.profile_name = name.map(str::to_string);
    e.summary = format!("Profile deleted by {actor}");
    record(state, e).await;
}

/// After asking the sidecar to start a run: the job (started) or the refusal (start_failed).
/// `preview`: a dry run (rsync), labelled "Preview".
pub async fn run_requested(
    state: &AppState,
    tool: &str,
    actor: &str,
    id: Uuid,
    name: &str,
    result: &Result<Value, AppError>,
    preview: bool,
) {
    let what = if preview { "Preview" } else { "Run" };
    run_requested_as(
        state,
        tool,
        actor,
        id,
        name,
        result,
        what,
        json!({ "preview": preview }),
    )
    .await
}

/// `run_requested` with the run's name ("CDC", "One-time sync"…) and extra detail (its mode).
#[allow(clippy::too_many_arguments)]
pub async fn run_requested_as(
    state: &AppState,
    tool: &str,
    actor: &str,
    id: Uuid,
    name: &str,
    result: &Result<Value, AppError>,
    what: &str,
    extra: Value,
) {
    let preview = extra.get("preview").and_then(Value::as_bool) == Some(true);
    match result {
        Ok(job) => {
            let job_id = job
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let mut e = base(tool, "run.started", actor);
            if !job_id.is_empty() {
                e.event_id = run_event_id(tool, &job_id, "run.started");
                e.job_id = Some(job_id);
            }
            e.profile_id = Some(id.to_string());
            e.profile_name = Some(name.to_string());
            e.status = Some("started".into());
            e.summary = format!("{what} started by {actor}");
            let mut detail = extra.clone();
            if let Value::Object(m) = &mut detail {
                m.retain(|k, v| !(k == "preview" && v == &json!(false)));
            }
            if detail.as_object().is_some_and(|m| !m.is_empty()) {
                e.detail = detail.to_string();
            }
            record(state, e).await;
        }
        Err(err) => {
            let mut e = base(tool, "run.start_failed", actor);
            e.profile_id = Some(id.to_string());
            e.profile_name = Some(name.to_string());
            e.status = Some("failed".into());
            e.summary = format!("{what} could not start: {}", short(&err.to_string(), 160));
            let mut detail = extra.clone();
            if let Value::Object(m) = &mut detail {
                m.insert("error".into(), json!(err.to_string()));
                if !preview {
                    m.remove("preview");
                }
            }
            e.detail = detail.to_string();
            record(state, e).await;
        }
    }
}

pub async fn stop_requested(
    state: &AppState,
    tool: &str,
    actor: &str,
    job_id: &str,
    response: &Value,
) {
    let job = if response.get("profile_id").is_some() {
        response.clone()
    } else {
        get_job(state, tool, job_id).await.unwrap_or(Value::Null)
    };
    let mut e = base(tool, "run.stop_requested", actor);
    e.job_id = Some(job_id.to_string());
    e.profile_id = job
        .get("profile_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    e.profile_name = job.get("name").and_then(Value::as_str).map(str::to_string);
    e.summary = format!("Stop requested by {actor}");
    record(state, e).await;
}

/// Connection tests of a saved profile (drafts have no profile to attach them to).
pub async fn connection_tested(
    state: &AppState,
    tool: &str,
    actor: &str,
    profile: Option<(Uuid, String)>,
    side: &str,
    out: &Value,
) {
    let Some((id, name)) = profile else { return };
    let ok = out.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let side_label = match side {
        "target" => "Target",
        "route" => "Route",
        _ => "Source",
    };
    let mut e = base(tool, "connection.tested", actor);
    e.profile_id = Some(id.to_string());
    e.profile_name = Some(name);
    e.status = Some(if ok { "ok" } else { "failed" }.into());
    e.summary = if ok {
        format!("{side_label} connection OK")
    } else {
        let why = out.get("error").and_then(Value::as_str).unwrap_or("failed");
        format!("{side_label} connection failed: {}", short(why, 140))
    };
    let mut detail = Map::new();
    detail.insert("side".into(), json!(side));
    for k in [
        "target",
        "error",
        "hint",
        "warning",
        "facts",
        "server_version",
    ] {
        if let Some(v) = out.get(k).filter(|v| !v.is_null()) {
            detail.insert(k.into(), v.clone());
        }
    }
    let mut detail = Value::Object(detail);
    redact(&mut detail);
    e.detail = detail.to_string();
    record(state, e).await;
}

// ── sidecar dispatch ────────────────────────────────────────────────────────

async fn list_jobs(state: &AppState, tool: &str) -> Result<Value, AppError> {
    let (c, h) = (&state.config, &state.http);
    match tool {
        "pgcopy" => crate::pgcopy::list_jobs(c, h).await,
        "pgdump" => crate::pgdump::list_jobs(c, h).await,
        "pgloader" => crate::pgloader::list_jobs(c, h).await,
        "mongocopy" => crate::mongocopy::list_jobs(c, h).await,
        "redisshake" => crate::redisshake::list_jobs(c, h).await,
        "rsync" => crate::rsync::list_jobs(c, h).await,
        other => Err(AppError::BadRequest(format!("unknown tool {other}"))),
    }
}

async fn get_job(state: &AppState, tool: &str, job_id: &str) -> Result<Value, AppError> {
    let (c, h) = (&state.config, &state.http);
    match tool {
        "pgcopy" => crate::pgcopy::get_job(c, h, job_id).await,
        "pgdump" => crate::pgdump::get_job(c, h, job_id).await,
        "pgloader" => crate::pgloader::get_job(c, h, job_id).await,
        "mongocopy" => crate::mongocopy::get_job(c, h, job_id).await,
        "redisshake" => crate::redisshake::get_job(c, h, job_id).await,
        "rsync" => crate::rsync::get_job(c, h, job_id).await,
        other => Err(AppError::BadRequest(format!("unknown tool {other}"))),
    }
}

async fn job_logs(state: &AppState, tool: &str, job_id: &str) -> Result<Value, AppError> {
    let (c, h) = (&state.config, &state.http);
    match tool {
        "pgcopy" => crate::pgcopy::job_logs(c, h, job_id).await,
        "pgdump" => crate::pgdump::job_logs(c, h, job_id).await,
        "pgloader" => crate::pgloader::job_logs(c, h, job_id).await,
        "mongocopy" => crate::mongocopy::job_logs(c, h, job_id).await,
        "redisshake" => crate::redisshake::job_logs(c, h, job_id).await,
        "rsync" => crate::rsync::job_logs(c, h, job_id).await,
        other => Err(AppError::BadRequest(format!("unknown tool {other}"))),
    }
}

/// Last lines of a run log: ANSI colours removed, `user:password@` in URIs masked, size-capped.
fn re(cell: &'static std::sync::OnceLock<regex::Regex>, pattern: &str) -> &'static regex::Regex {
    cell.get_or_init(|| regex::Regex::new(pattern).expect("static regex"))
}
static ANSI_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
static URI_PASS_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
static ERROR_LINE_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

pub fn log_tail(log: &str) -> String {
    let ansi = re(&ANSI_RE, r"\x1b\[[0-9;]*[A-Za-z]");
    let uri_pass = re(&URI_PASS_RE, r"(://[^:/@\s]+):[^@\s]+@");
    let lines: Vec<String> = log
        .lines()
        .map(|l| ansi.replace_all(l, "").trim_end().to_string())
        .filter(|l| !l.trim().is_empty())
        .collect();
    let tail = lines[lines.len().saturating_sub(LOG_TAIL_LINES)..].join("\n");
    let tail = uri_pass.replace_all(&tail, "$1:****@").to_string();
    if tail.chars().count() > LOG_TAIL_MAX_CHARS {
        let skip = tail.chars().count() - LOG_TAIL_MAX_CHARS;
        format!("…{}", tail.chars().skip(skip).collect::<String>())
    } else {
        tail
    }
}

/// The first ERROR/FATAL line of a log tail (usually the cause; later ones are consequences),
/// without its timestamp and source prefix.
pub fn first_error_line(tail: &str) -> Option<String> {
    let line_re = re(
        &ERROR_LINE_RE,
        r"(?i)\b(?:ERROR|FATAL|PANIC)\b[:\]]?\s+(?:\S+\.(?:c|go|py|rs):\d+\s+)?(.+)",
    );
    tail.lines()
        .find_map(|l| line_re.captures(l).map(|c| c[1].trim().to_string()))
        .filter(|s| !s.is_empty())
}

// ── run watcher ─────────────────────────────────────────────────────────────

fn is_terminal(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "stopped")
}

/// The run.finished event for a job that has ended, from the sidecar's job record (and, for a
/// failed or stopped run, the end of its log).
pub fn finished_event(tool: &str, job: &Value, tail: Option<&str>) -> Option<ToolEventRow> {
    let job_id = job.get("id")?.as_str()?;
    let status = job.get("status")?.as_str()?;
    if !is_terminal(status) {
        return None;
    }
    let ts = |k: &str| job.get(k).and_then(Value::as_str).and_then(norm_ts);
    let started = ts("started_at").or_else(|| ts("created_at"));
    let finished = ts("finished_at");
    let duration = started
        .zip(finished.clone())
        .and_then(|(s, f)| Some((parse_ts(&f)? - parse_ts(&s)?).num_seconds()));
    let error = job
        .get("error")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    // No error from the sidecar (e.g. pgcopydb exit 12): the log says why.
    let reason = error.clone().or_else(|| tail.and_then(first_error_line));

    let mut e = base(tool, "run.finished", "system");
    e.event_id = run_event_id(tool, job_id, "run.finished");
    e.timestamp = finished.unwrap_or_else(now_ts);
    e.job_id = Some(job_id.to_string());
    e.profile_id = job
        .get("profile_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    e.profile_name = job.get("name").and_then(Value::as_str).map(str::to_string);
    e.status = Some(status.to_string());
    e.summary = finished_summary(job, status, duration, reason.as_deref());
    // The sidecar's error is already in JOB_FIELDS; only a log-derived reason is added.
    let log_reason = if error.is_none() {
        reason.as_deref()
    } else {
        None
    };
    e.detail = finished_detail(job, duration, log_reason, tail);
    Some(e)
}

fn parse_ts(s: &str) -> Option<DateTime<chrono::FixedOffset>> {
    DateTime::parse_from_rfc3339(s).ok()
}

/// "pgcopydb run succeeded in 3 m 12 s", "… failed after 4 s, exit 12: <reason>".
fn finished_summary(
    job: &Value,
    status: &str,
    duration: Option<i64>,
    reason: Option<&str>,
) -> String {
    let what = run_label(job);
    let (verb, prep) = match status {
        "succeeded" => ("succeeded", "in"),
        "stopped" => ("stopped", "after"),
        _ => ("failed", "after"),
    };
    let mut s = match duration.map(fmt_duration) {
        Some(t) => format!("{what} {verb} {prep} {t}"),
        None => format!("{what} {verb}"),
    };
    if verb != "failed" {
        return s;
    }
    if let Some(code) = job.get("exit_code").and_then(Value::as_i64) {
        s.push_str(&format!(", exit {code}"));
    }
    if let Some(err) = reason {
        s.push_str(&format!(": {}", short(err, 140)));
    }
    s
}

/// Redacted JSON detail: the job's fields, duration, a log-derived reason and the log tail.
fn finished_detail(
    job: &Value,
    duration: Option<i64>,
    log_reason: Option<&str>,
    tail: Option<&str>,
) -> String {
    let mut detail = Map::new();
    for k in JOB_FIELDS {
        if let Some(v) = job.get(k).filter(|v| !v.is_null()) {
            detail.insert(k.into(), v.clone());
        }
    }
    if let Some(d) = duration {
        detail.insert("duration_secs".into(), json!(d));
    }
    if let Some(r) = log_reason {
        detail.insert("reason".into(), json!(r));
    }
    if let Some(t) = tail.filter(|t| !t.is_empty()) {
        detail.insert("log_tail".into(), json!(t));
    }
    let mut detail = Value::Object(detail);
    redact(&mut detail);
    detail.to_string()
}

/// run.started seen by the watcher (runs started outside this orchestrator, or when the handler's
/// record was lost). Same id as the handler's record, so it never duplicates it.
pub fn started_event(tool: &str, job: &Value) -> Option<ToolEventRow> {
    let job_id = job.get("id")?.as_str()?;
    let ts = |k: &str| job.get(k).and_then(Value::as_str).and_then(norm_ts);
    let mut e = base(tool, "run.started", "system");
    e.event_id = run_event_id(tool, job_id, "run.started");
    e.timestamp = ts("started_at")
        .or_else(|| ts("created_at"))
        .unwrap_or_else(now_ts);
    e.job_id = Some(job_id.to_string());
    e.profile_id = job
        .get("profile_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    e.profile_name = job.get("name").and_then(Value::as_str).map(str::to_string);
    e.status = Some("started".into());
    e.summary = format!("{} started", run_label(job));
    Some(e)
}

/// Polls every configured sidecar; records run.started / run.finished once per job.
/// Also applies retention once an hour.
pub fn spawn_run_watcher(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let every = Duration::from_secs(state.config.tool_events_poll_secs);
        let mut seen: HashSet<String> = HashSet::new();
        let mut last_trim = std::time::Instant::now() - Duration::from_secs(3600);
        loop {
            poll_tools(&state, every, &mut seen).await;
            if last_trim.elapsed() >= Duration::from_secs(3600) {
                last_trim = std::time::Instant::now();
                trim_retention(&state).await;
            }
            tokio::time::sleep(every).await;
        }
    })
}

/// One pass over every sidecar's job list. `seen` holds the event ids already recorded.
async fn poll_tools(state: &AppState, every: Duration, seen: &mut HashSet<String>) {
    let mut still: HashSet<String> = HashSet::new();
    for tool in TOOLS {
        let Ok(list) = list_jobs(state, tool).await else {
            continue;
        }; // not configured / down
        let jobs = list.get("jobs").and_then(Value::as_array);
        for job in jobs.into_iter().flatten() {
            for ev in job_events(state, tool, job, every, seen).await {
                still.insert(ev.event_id.clone());
                if seen.insert(ev.event_id.clone()) {
                    record(state, ev).await;
                }
            }
        }
    }
    // Forget jobs the sidecars no longer list (INSERT OR IGNORE covers a restart).
    seen.retain(|id| still.contains(id));
}

/// run.started / run.finished events a job listing implies right now.
async fn job_events(
    state: &AppState,
    tool: &str,
    job: &Value,
    every: Duration,
    seen: &HashSet<String>,
) -> Vec<ToolEventRow> {
    let status = job
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut events = Vec::new();
    // A running job is announced once it is clearly not the one a handler just started.
    if is_terminal(status) || created_before(job, 2 * every.as_secs() as i64) {
        events.extend(started_event(tool, job));
    }
    let id = job.get("id").and_then(Value::as_str);
    // Fetch the log only for a failed/stopped run not yet recorded.
    let needs_tail = matches!(status, "failed" | "stopped")
        && id.is_some_and(|id| !seen.contains(&run_event_id(tool, id, "run.finished")));
    let tail = match id.filter(|_| needs_tail) {
        Some(id) => job_logs(state, tool, id)
            .await
            .ok()
            .and_then(|v| v.get("log").and_then(Value::as_str).map(log_tail)),
        None => None,
    };
    events.extend(finished_event(tool, job, tail.as_deref()));
    events
}

/// The job's `created_at` is more than `secs` ago.
fn created_before(job: &Value, secs: i64) -> bool {
    job.get("created_at")
        .and_then(Value::as_str)
        .and_then(parse_ts)
        .is_some_and(|t| (Utc::now() - t.with_timezone(&Utc)).num_seconds() > secs)
}

async fn trim_retention(state: &AppState) {
    let Some(rqlite) = state.rqlite.as_ref() else {
        return;
    };
    let days = i64::from(state.config.tool_events_retention_days);
    let before =
        (Utc::now() - chrono::Duration::days(days)).to_rfc3339_opts(SecondsFormat::Millis, true);
    if let Err(e) = rqlite
        .trim_tool_events(&before, state.config.tool_events_max_rows)
        .await
    {
        warn!(error = %e, "tool event retention failed");
    }
}

// ── API: GET /cdc/tool-events ───────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct EventFilter {
    pub tool: Option<String>,
    pub profile_id: Option<String>,
    /// Prefix: `run.`, `profile.`, `connection.` or an exact kind.
    pub kind: Option<String>,
    pub status: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub limit: Option<usize>,
}

impl EventFilter {
    fn matches(&self, e: &ToolEventRow) -> bool {
        let eq = |want: &Option<String>, have: Option<&str>| {
            want.as_deref().is_none_or(|w| have == Some(w))
        };
        eq(&self.tool, Some(&e.tool))
            && eq(&self.profile_id, e.profile_id.as_deref())
            && self.kind.as_deref().is_none_or(|k| e.kind.starts_with(k))
            && eq(&self.status, e.status.as_deref())
            && self
                .since
                .as_deref()
                .is_none_or(|s| e.timestamp.as_str() >= s)
            && self
                .until
                .as_deref()
                .is_none_or(|u| e.timestamp.as_str() <= u)
    }
}

fn to_json(e: ToolEventRow) -> Value {
    let detail: Value =
        serde_json::from_str(&e.detail).unwrap_or_else(|_| json!({ "raw": e.detail }));
    json!({
        "id": e.event_id,
        "timestamp": e.timestamp,
        "tool": e.tool,
        "profile_id": e.profile_id,
        "profile_name": e.profile_name,
        "job_id": e.job_id,
        "kind": e.kind,
        "status": e.status,
        "actor": e.actor,
        "summary": e.summary,
        "detail": detail,
    })
}

pub async fn list_events(
    State(state): State<AppState>,
    Query(mut f): Query<EventFilter>,
) -> Result<Json<Value>, AppError> {
    if let Some(t) = f.tool.as_deref() {
        if !TOOLS.contains(&t) {
            return Err(AppError::BadRequest(format!(
                "tool must be one of {}",
                TOOLS.join(", ")
            )));
        }
    }
    for (name, v) in [("since", &mut f.since), ("until", &mut f.until)] {
        if let Some(raw) = v.as_deref() {
            *v =
                Some(norm_ts(raw).ok_or_else(|| {
                    AppError::BadRequest(format!("{name} must be an RFC 3339 time"))
                })?);
        }
    }
    let limit = f.limit.unwrap_or(500).clamp(1, 2000);
    let mut rows = match state.rqlite.as_ref() {
        Some(rqlite) => rqlite
            .list_tool_events(
                limit,
                f.tool.as_deref(),
                f.profile_id.as_deref(),
                f.kind.as_deref(),
                f.status.as_deref(),
                f.since.as_deref(),
                f.until.as_deref(),
            )
            .await
            .map_err(|e| AppError::Upstream(format!("tool events: {e}")))?,
        None => Vec::new(),
    };
    // Memory holds everything without rqlite, and failed writes with it.
    let ids: HashSet<String> = rows.iter().map(|r| r.event_id.clone()).collect();
    rows.extend(
        state
            .tool_events_mem
            .list(&f)
            .await
            .into_iter()
            .filter(|r| !ids.contains(&r.event_id)),
    );
    rows.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    rows.truncate(limit);
    Ok(Json(json!({
        "events": rows.into_iter().map(to_json).collect::<Vec<_>>(),
        "persisted": state.rqlite.is_some(),
        "retention_days": state.config.tool_events_retention_days,
        "limit": limit,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_normalise_to_utc_millis() {
        assert_eq!(
            norm_ts("2026-11-05T22:38:03.723+00:00").unwrap(),
            "2026-11-05T22:38:03.723Z"
        );
        assert_eq!(
            norm_ts("2026-11-06T05:38:03+07:00").unwrap(),
            "2026-11-05T22:38:03.000Z"
        );
        assert!(norm_ts("yesterday").is_none());
    }

    #[test]
    fn run_ids_are_deterministic_per_tool_job_and_kind() {
        assert_eq!(
            run_event_id("pgcopy", "j1", "run.finished"),
            run_event_id("pgcopy", "j1", "run.finished")
        );
        assert_ne!(
            run_event_id("pgcopy", "j1", "run.finished"),
            run_event_id("pgcopy", "j1", "run.started")
        );
        assert_ne!(
            run_event_id("pgcopy", "j1", "run.finished"),
            run_event_id("rsync", "j1", "run.finished")
        );
    }

    #[test]
    fn diff_lists_leaf_changes_and_masks_secrets() {
        let before = json!({ "name": "a", "updated_at": "x", "source": { "host": "h1", "password": "p1" }, "options": { "dbs": [0] } });
        let after = json!({ "name": "a", "updated_at": "y", "source": { "host": "h2", "password": "p2" }, "options": { "dbs": [0, 2], "new": "" } });
        let c = changes(&before, &after);
        assert_eq!(
            c,
            vec![
                json!({ "field": "options.dbs", "before": [0], "after": [0, 2] }),
                json!({ "field": "source.host", "before": "h1", "after": "h2" }),
                json!({ "field": "source.password", "secret": true }),
            ]
        );
        assert!(changes(&before, &before).is_empty());
    }

    #[test]
    fn redact_masks_nested_secrets_but_keeps_empty_ones() {
        let mut v = json!({ "target": { "password": "x", "ssh_private_key": "k", "user": "u" }, "token": "" });
        redact(&mut v);
        assert_eq!(
            v,
            json!({ "target": { "password": "********", "ssh_private_key": "********", "user": "u" }, "token": "" })
        );
    }

    #[test]
    fn finished_event_uses_job_times_and_whitelists_fields() {
        let job = json!({
            "id": "j9", "profile_id": "p1", "name": "copy", "status": "failed", "exit_code": 12,
            "error": "relation \"x\" does not exist", "created_at": "2026-11-05T22:38:00.000+00:00",
            "started_at": "2026-11-05T22:38:00.000+00:00", "finished_at": "2026-11-05T22:41:12.000+00:00",
            "source": { "host": "db", "password": "secret" }, "argv": ["pgcopydb"],
        });
        let e = finished_event("pgcopy", &job, None).unwrap();
        assert_eq!(e.timestamp, "2026-11-05T22:41:12.000Z");
        assert_eq!(e.status.as_deref(), Some("failed"));
        assert_eq!(
            e.summary,
            "Run failed after 3 m 12 s, exit 12: relation \"x\" does not exist"
        );
        let d: Value = serde_json::from_str(&e.detail).unwrap();
        assert_eq!(d["duration_secs"], 192);
        assert!(d.get("source").is_none() && d.get("argv").is_none());
        assert!(
            finished_event("pgcopy", &json!({ "id": "j", "status": "running" }), None).is_none()
        );
    }

    #[test]
    fn log_tail_explains_a_failure_without_an_error_field() {
        let log = "\x1b[32mINFO\x1b[0m start\n\n2026-11-05 22:37:42.025 960 ERROR  pgsql.c:463               Connection to database failed: fe_sendauth: no password supplied\nretry postgres://app:hunter2@db:5432/x\n";
        let tail = log_tail(log);
        assert!(
            !tail.contains('\x1b')
                && !tail.contains("hunter2")
                && tail.contains("postgres://app:****@db")
        );
        assert_eq!(
            first_error_line(&tail).as_deref(),
            Some("Connection to database failed: fe_sendauth: no password supplied")
        );
        let job = json!({ "id": "j", "status": "failed", "exit_code": 12,
            "started_at": "2026-11-05T22:36:41.915+00:00", "finished_at": "2026-11-05T22:37:42.027+00:00" });
        let e = finished_event("pgcopy", &job, Some(&tail)).unwrap();
        assert_eq!(e.summary, "Run failed after 1 m 00 s, exit 12: Connection to database failed: fe_sendauth: no password supplied");
        let d: Value = serde_json::from_str(&e.detail).unwrap();
        assert!(d["log_tail"]
            .as_str()
            .unwrap()
            .ends_with("postgres://app:****@db:5432/x"));
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(fmt_duration(4), "4 s");
        assert_eq!(fmt_duration(192), "3 m 12 s");
        assert_eq!(fmt_duration(3900), "1 h 05 m");
    }
}
