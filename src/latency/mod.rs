//! Per-pipeline latency (ADR 0016).
//!
//! A sampler (every `ORCH_LATENCY_SAMPLE_SECS`) measures each running pipeline:
//! - **estimate** (all pipelines, no writes): capture delay from Debezium offset commit time vs
//!   newest Kafka record; sink delay from the age of the oldest record the sink has not
//!   committed (younger than the offset-flush interval counts as caught up); throughput from
//!   high-watermark deltas.
//! - **trigger_inbox** (trigger capture → Postgres target): `applied_at − changed_at` in the
//!   target inbox.
//! - **heartbeat** (opt-in probe, `probe.rs`): exact end-to-end.
//!
//! Samples stay in memory (6 h); 5-minute rollups go to rqlite (30 days). `latency_above`
//! rules are evaluated here and delivered through `notify::fanout`.

pub mod kafka;
pub mod probe;
pub mod stats;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::Method as HttpMethod;
use chrono::{DateTime, TimeDelta, Utc};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::models::{AlertRule, CaptureMode, CdcProfile, ProfileStatus};
use crate::notify::render::{NotificationEvent, Severity};
use crate::reconcile::{connect_request, connector_name_for, sink_connector_name_for};
use crate::AppState;
use probe::ProbeState;
use stats::{Method, Sample, Status};

const KEEP_IN_MEMORY: TimeDelta = TimeDelta::hours(6);
const RETENTION_DAYS: i64 = 30;

#[derive(Default)]
struct Series {
    name: String,
    samples: VecDeque<Sample>,
    highs: HashMap<kafka::Tp, i64>,
    sum_high: Option<(i64, Instant)>,
    last_event_at: Option<DateTime<Utc>>,
    bucket: Option<DateTime<Utc>>,
    pending: Vec<Sample>,
}

struct Probe {
    task: tokio::task::JoinHandle<()>,
    state: Arc<Mutex<ProbeState>>,
}

/// (rule id, pipeline id) → when the threshold was first exceeded, and whether it fired.
type Breaches = HashMap<(String, Uuid), (Instant, bool)>;

#[derive(Clone, Default)]
pub struct LatencyStore {
    series: Arc<RwLock<HashMap<Uuid, Series>>>,
    probes: Arc<Mutex<HashMap<Uuid, Probe>>>,
    breaches: Arc<Mutex<Breaches>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Current {
    pub profile_id: Uuid,
    pub name: String,
    #[serde(flatten)]
    pub sample: Sample,
    /// p95 of end-to-end over the last hour (in-memory samples).
    pub e2e_p95_1h_ms: Option<i64>,
    pub probe_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Point {
    pub t: DateTime<Utc>,
    pub e2e_ms: Option<i64>,
    /// Rollup points only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub e2e_p95_ms: Option<i64>,
    pub capture_ms: Option<i64>,
    pub sink_ms: Option<i64>,
    pub lag_records: Option<i64>,
    pub throughput_rps: Option<f64>,
}

impl LatencyStore {
    pub async fn current_all(&self) -> Vec<Current> {
        let series = self.series.read().await;
        let probes = self.probes.lock().await;
        let mut out = Vec::new();
        for (id, s) in series.iter() {
            let Some(last) = s.samples.back() else {
                continue;
            };
            let probe_error = match probes.get(id) {
                Some(p) => p.state.lock().await.error.clone(),
                None => None,
            };
            out.push(Current {
                profile_id: *id,
                name: s.name.clone(),
                sample: last.clone(),
                e2e_p95_1h_ms: p95_since(s, Utc::now() - TimeDelta::hours(1)),
                probe_enabled: probes.contains_key(id),
                probe_error,
            });
        }
        out.sort_by_key(|a| a.name.to_lowercase());
        out
    }

    pub async fn current(&self, id: Uuid) -> Option<Current> {
        self.current_all()
            .await
            .into_iter()
            .find(|c| c.profile_id == id)
    }

    /// In-memory points since `since`, and when the oldest one starts (None: no samples).
    async fn memory_points(
        &self,
        id: Uuid,
        since: DateTime<Utc>,
    ) -> (Vec<Point>, Option<DateTime<Utc>>) {
        let series = self.series.read().await;
        let Some(s) = series.get(&id) else {
            return (Vec::new(), None);
        };
        let points: Vec<Point> = s
            .samples
            .iter()
            .filter(|x| x.ts >= since)
            .map(|x| Point {
                t: x.ts,
                e2e_ms: x.e2e_ms,
                e2e_p95_ms: None,
                capture_ms: x.capture_ms,
                sink_ms: x.sink_ms,
                lag_records: x.lag_records,
                throughput_rps: x.throughput_rps,
            })
            .collect();
        let first = points.first().map(|p| p.t);
        (points, first)
    }
}

fn p95_since(s: &Series, since: DateTime<Utc>) -> Option<i64> {
    let mut v: Vec<i64> = s
        .samples
        .iter()
        .filter(|x| x.ts >= since)
        .filter_map(|x| x.e2e_ms)
        .collect();
    v.sort_unstable();
    stats::percentile(&v, 95.0)
}

/// Points for `range` (`1h`, `6h`, `24h`, `7d`, `30d`). Up to 6 h: raw samples, preceded by
/// 5-minute rollups for any part before memory starts (after a restart). Longer: rollups.
pub async fn history(state: &AppState, id: Uuid, range: &str) -> (Vec<Point>, &'static str) {
    let span = match range {
        "6h" => TimeDelta::hours(6),
        "24h" => TimeDelta::hours(24),
        "7d" => TimeDelta::days(7),
        "30d" => TimeDelta::days(30),
        _ => TimeDelta::hours(1),
    };
    let since = Utc::now() - span;
    let rollups = |until: Option<DateTime<Utc>>| async move {
        let Some(rq) = &state.rqlite else {
            return Vec::new();
        };
        rq.list_metrics_rollups(&id.to_string(), &since.to_rfc3339())
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|r| {
                let t = DateTime::parse_from_rfc3339(&r.timestamp)
                    .ok()?
                    .with_timezone(&Utc);
                // A bucket overlapping in-memory samples is shown by the samples themselves.
                if until.is_some_and(|u| t + stats::BUCKET > u) {
                    return None;
                }
                Some(Point {
                    t,
                    e2e_ms: r.e2e_p50_ms,
                    e2e_p95_ms: r.e2e_p95_ms,
                    capture_ms: r.capture_max_ms,
                    sink_ms: r.sink_max_ms,
                    lag_records: r.lag_max,
                    throughput_rps: r.throughput_avg,
                })
            })
            .collect::<Vec<Point>>()
    };
    if span <= KEEP_IN_MEMORY {
        let (memory, first) = state.latency.memory_points(id, since).await;
        let mut points = match first {
            Some(f) if f > since + TimeDelta::minutes(2) => rollups(Some(f)).await,
            Some(_) => Vec::new(),
            None => rollups(None).await,
        };
        let resolution = if memory.is_empty() { "5m" } else { "sample" };
        points.extend(memory);
        return (points, resolution);
    }
    (rollups(None).await, "5m")
}

// ── Sampler ─────────────────────────────────────────────────────────────────

pub fn spawn_sampler(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let bootstrap = state.config.kafka_bootstrap.clone();
        let consumer = match tokio::task::spawn_blocking(move || kafka::consumer(&bootstrap)).await
        {
            Ok(Ok(c)) => Some(Arc::new(c)),
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "latency sampler: Kafka client unavailable; Kafka-based estimates are off");
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, "latency sampler: Kafka client setup panicked");
                None
            }
        };
        let mut ticker =
            tokio::time::interval(Duration::from_secs(state.config.latency_sample_secs));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_trim = Instant::now() - Duration::from_secs(3600);
        loop {
            ticker.tick().await;
            if let Err(e) = tick(&state, consumer.clone()).await {
                tracing::warn!(error = %e, "latency sampler tick failed");
            }
            if last_trim.elapsed() >= Duration::from_secs(3600) {
                last_trim = Instant::now();
                if let Some(rq) = &state.rqlite {
                    let before = (Utc::now() - TimeDelta::days(RETENTION_DAYS)).to_rfc3339();
                    let _ = rq.trim_metrics_rollups(&before).await;
                }
            }
        }
    })
}

async fn tick(
    state: &AppState,
    consumer: Option<Arc<rdkafka::consumer::BaseConsumer>>,
) -> Result<(), crate::error::AppError> {
    let profiles = state.profiles.list().await?;
    let rules = crate::watcher::rules::load_enabled_rules(state)
        .await
        .unwrap_or_default();
    let ids: std::collections::HashSet<Uuid> = profiles.iter().map(|p| p.id).collect();
    state
        .latency
        .series
        .write()
        .await
        .retain(|id, _| ids.contains(id));
    manage_probes(state, &profiles).await;

    for p in profiles
        .iter()
        .filter(|p| p.has_managed_sink() || p.status == ProfileStatus::Running)
    {
        let sample = match tokio::time::timeout(
            Duration::from_secs(25),
            sample_one(state, p, consumer.clone()),
        )
        .await
        {
            Ok(s) => s,
            Err(_) => unknown(p, "sampling timed out"),
        };
        record(state, p, sample.clone()).await;
        evaluate_alerts(state, &rules, p, &sample).await;
    }
    Ok(())
}

fn unknown(p: &CdcProfile, note: &str) -> Sample {
    Sample {
        ts: Utc::now(),
        e2e_ms: None,
        capture_ms: None,
        sink_ms: None,
        lag_records: None,
        throughput_rps: None,
        last_event_at: None,
        method: Method::Estimate,
        status: if p.status == ProfileStatus::Running {
            Status::Unknown
        } else {
            Status::Stopped
        },
        resolution_ms: None,
        note: Some(note.to_string()),
    }
}

async fn manage_probes(state: &AppState, profiles: &[CdcProfile]) {
    let mut probes = state.latency.probes.lock().await;
    probes.retain(|_, p| !p.task.is_finished());
    for p in profiles {
        let want = p.latency_probe.enabled
            && p.status == ProfileStatus::Running
            && probe::unsupported_reason(p).is_none();
        if want && !probes.contains_key(&p.id) {
            let shared = Arc::new(Mutex::new(ProbeState::default()));
            let task = probe::spawn(state.clone(), p.id, shared.clone());
            probes.insert(
                p.id,
                Probe {
                    task,
                    state: shared,
                },
            );
        } else if !want {
            if let Some(old) = probes.remove(&p.id) {
                old.task.abort();
            }
        }
    }
}

/// Committed sink offsets from Connect (`GET /connectors/{sink}/offsets`).
fn committed_offsets(v: &Value) -> HashMap<kafka::Tp, i64> {
    let mut out = HashMap::new();
    for e in v
        .get("offsets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let part = e.get("partition");
        let topic = part
            .and_then(|p| p.get("kafka_topic"))
            .and_then(Value::as_str);
        let partition = part
            .and_then(|p| p.get("kafka_partition"))
            .and_then(Value::as_i64);
        let offset = e
            .get("offset")
            .and_then(|o| o.get("kafka_offset"))
            .and_then(Value::as_i64);
        if let (Some(t), Some(p), Some(o)) = (topic, partition, offset) {
            out.insert((t.to_string(), p as i32), o);
        }
    }
    out
}

/// Active topics of a connector (`GET /connectors/{n}/topics`).
fn active_topics(v: &Value, name: &str) -> Vec<String> {
    v.get(name)
        .and_then(|c| c.get("topics"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

async fn connect_get(state: &AppState, path: &str) -> Option<Value> {
    connect_request(state, HttpMethod::GET, path, None)
        .await
        .ok()
}

async fn sample_one(
    state: &AppState,
    p: &CdcProfile,
    consumer: Option<Arc<rdkafka::consumer::BaseConsumer>>,
) -> Sample {
    let now = Utc::now();
    if p.status != ProfileStatus::Running {
        return unknown(p, "pipeline is not running");
    }
    let source = connector_name_for(p);
    let managed = p.has_managed_sink();
    let (committed, topics) = pipeline_topics(state, p, &source, managed).await;

    let (prev_highs, prev_sum, prev_last_event) = {
        let series = state.latency.series.read().await;
        series
            .get(&p.id)
            .map(|s| (s.highs.clone(), s.sum_high, s.last_event_at))
            .unwrap_or_default()
    };

    let mut note = None;
    let scan = match consumer {
        Some(c) if !topics.is_empty() => {
            tokio::task::spawn_blocking(move || kafka::scan(&c, &topics, &committed, &prev_highs))
                .await
                .ok()
        }
        Some(_) => None,
        None => {
            note = Some(
                "Kafka is not reachable from the orchestrator (ORCH_KAFKA_BOOTSTRAP)".to_string(),
            );
            None
        }
    };

    let flush = state.config.connect_offset_flush_ms;
    let sum_high: Option<i64> = scan.as_ref().map(|s| s.highs.values().sum());
    let throughput = throughput_since(sum_high, prev_sum);
    let newest_ms = scan.as_ref().and_then(|s| s.newest_ms);
    let last_event_at = newest_ms
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .or(prev_last_event);
    let sink_scan = scan.as_ref().filter(|_| managed);
    let lag = sink_scan.map(|s| s.lag);
    let sink_ms = sink_scan.and_then(|s| {
        stats::sink_delay_ms(now.timestamp_millis(), s.lag, s.oldest_unconsumed_ms, flush)
    });
    let capture_ms = capture_delay_ms(state, p, &source, newest_ms).await;

    // Most exact method available; otherwise an estimate within the commit granularity.
    let (exact, exact_note) = exact_latency(state, p).await;
    if exact_note.is_some() {
        note = exact_note;
    }
    let (method, resolution, e2e) = match exact {
        Some(x) => x,
        None => (
            Method::Estimate,
            Some(flush),
            estimate_e2e(p, sink_ms, capture_ms, newest_ms),
        ),
    };

    let status = if method == Method::Heartbeat {
        if e2e.is_some_and(|v| v > flush.max(30_000)) {
            Status::Behind
        } else {
            Status::Ok
        }
    } else {
        stats::classify(true, sink_ms, lag, last_event_at, now)
    };
    if let (Some(s), Some(scan)) = (sum_high, scan) {
        let mut series = state.latency.series.write().await;
        let entry = series.entry(p.id).or_default();
        entry.highs = scan.highs;
        entry.sum_high = Some((s, Instant::now()));
    }
    Sample {
        ts: now,
        e2e_ms: e2e,
        capture_ms,
        sink_ms,
        lag_records: lag,
        throughput_rps: throughput.map(|t| (t * 100.0).round() / 100.0),
        last_event_at,
        method,
        status,
        resolution_ms: resolution,
        note,
    }
}

/// The sink's committed offsets (managed sinks only) and the topics to scan: the source's
/// active topics plus any the sink has committed on.
async fn pipeline_topics(
    state: &AppState,
    p: &CdcProfile,
    source: &str,
    managed: bool,
) -> (HashMap<kafka::Tp, i64>, Vec<String>) {
    let committed = if managed {
        let sink = sink_connector_name_for(p);
        connect_get(
            state,
            &format!("/connectors/{}/offsets", urlencoding::encode(&sink)),
        )
        .await
        .map(|v| committed_offsets(&v))
        .unwrap_or_default()
    } else {
        HashMap::new()
    };
    let mut topics: Vec<String> = connect_get(
        state,
        &format!("/connectors/{}/topics", urlencoding::encode(source)),
    )
    .await
    .map(|v| active_topics(&v, source))
    .unwrap_or_default();
    for (t, _) in committed.keys() {
        if !topics.contains(t) {
            topics.push(t.clone());
        }
    }
    (committed, topics)
}

/// Records per second since the previous sample's high-watermark sum.
fn throughput_since(sum_high: Option<i64>, prev: Option<(i64, Instant)>) -> Option<f64> {
    let (cur, (prev, at)) = sum_high.zip(prev)?;
    let secs = at.elapsed().as_secs_f64();
    (cur >= prev && secs > 0.5).then(|| (cur - prev) as f64 / secs)
}

async fn capture_delay_ms(
    state: &AppState,
    p: &CdcProfile,
    source: &str,
    newest_ms: Option<i64>,
) -> Option<i64> {
    if p.source.capture_mode != CaptureMode::Logical {
        // Polling captures: a change waits up to one poll interval.
        return Some(crate::postgres_trigger_cdc::trigger_poll_interval_ms(p) as i64);
    }
    let source_ms = connect_get(
        state,
        &format!("/connectors/{}/offsets", urlencoding::encode(source)),
    )
    .await
    .and_then(|v| stats::offset_source_ms(&v));
    stats::capture_delay_ms(newest_ms, source_ms, state.config.connect_offset_flush_ms)
}

/// Estimated e2e. A delay inside the commit granularity has no honest number: leave it empty
/// (status ok, "below resolution"). Polling captures always wait up to a poll interval.
fn estimate_e2e(
    p: &CdcProfile,
    sink_ms: Option<i64>,
    capture_ms: Option<i64>,
    newest_ms: Option<i64>,
) -> Option<i64> {
    let polling = p.source.capture_mode != CaptureMode::Logical;
    match sink_ms {
        Some(s) if s > 0 => Some(capture_ms.unwrap_or(0) + s),
        _ if newest_ms.is_some() && (polling || capture_ms.is_some_and(|c| c > 0)) => capture_ms,
        _ => None,
    }
}

type Exact = (Method, Option<i64>, Option<i64>);

/// A measured e2e (heartbeat probe, or the trigger inbox on a Postgres target) as
/// (method, resolution, e2e), and a note when the measurement failed.
async fn exact_latency(state: &AppState, p: &CdcProfile) -> (Option<Exact>, Option<String>) {
    let probe_val = {
        let probes = state.latency.probes.lock().await;
        match probes.get(&p.id) {
            Some(pr) => Some(pr.state.lock().await.clone()),
            None => None,
        }
    };
    if let Some(ps) = probe_val {
        if let Some(v) = ps.current_ms() {
            return (Some((Method::Heartbeat, Some(250), Some(v))), None);
        }
        return (None, ps.error.map(|err| format!("heartbeat: {err}")));
    }
    if !trigger_to_postgres(p) {
        return (None, None);
    }
    match inbox_latency(p, state.config.latency_sample_secs * 2).await {
        Ok(Some(v)) => (Some((Method::TriggerInbox, None, Some(v))), None),
        Ok(None) => (None, None),
        Err(e) => (None, Some(e)),
    }
}

fn trigger_to_postgres(p: &CdcProfile) -> bool {
    p.source.capture_mode == CaptureMode::Trigger
        && p.target.as_ref().is_some_and(|t| {
            matches!(
                t.target_type.to_ascii_lowercase().as_str(),
                "postgres" | "postgresql"
            )
        })
}

/// Trigger capture with a Postgres target: recent `applied_at − changed_at` (p50, ms).
async fn inbox_latency(p: &CdcProfile, window_secs: u64) -> Result<Option<i64>, String> {
    let t = p.target.as_ref().ok_or("no target")?;
    let insight = crate::connector_catalog::insight_from_connection(&t.target_type, &t.connection);
    let client = crate::connector_catalog::postgres_client(&insight)
        .await
        .map_err(|e| format!("target: {e}"))?;
    let sql = format!(
        "SELECT (percentile_cont(0.5) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM (applied_at - changed_at)) * 1000))::bigint \
         FROM {}.{} WHERE applied_at > clock_timestamp() - make_interval(secs => $1::double precision)",
        crate::postgres_trigger_cdc::CAPTURE_SCHEMA,
        crate::postgres_trigger_cdc::INBOX
    );
    let row = client
        .query_one(&sql, &[&(window_secs as f64)])
        .await
        .map_err(|e| format!("reading trigger inbox: {e}"))?;
    Ok(row.get::<_, Option<i64>>(0).map(|v| v.max(0)))
}

async fn record(state: &AppState, p: &CdcProfile, sample: Sample) {
    let bucket = stats::bucket_of(sample.ts);
    let flush = {
        let mut series = state.latency.series.write().await;
        let s = series.entry(p.id).or_default();
        s.name = p.name.clone();
        if let Some(t) = sample.last_event_at {
            s.last_event_at = Some(t);
        }
        let cut = sample.ts - KEEP_IN_MEMORY;
        while s.samples.front().is_some_and(|f| f.ts < cut) {
            s.samples.pop_front();
        }
        s.samples.push_back(sample.clone());
        let done = match s.bucket {
            Some(b) if b != bucket && !s.pending.is_empty() => {
                Some((b, std::mem::take(&mut s.pending)))
            }
            _ => None,
        };
        s.bucket = Some(bucket);
        if sample.status != Status::Stopped {
            s.pending.push(sample);
        }
        done
    };
    if let (Some((b, samples)), Some(rq)) = (flush, &state.rqlite) {
        let r = stats::rollup(&p.id.to_string(), b, &samples);
        if let Err(e) = rq.insert_metrics_rollup(&r).await {
            tracing::warn!(error = %e, "latency rollup write failed");
        }
    }
}

// ── latency_above ───────────────────────────────────────────────────────────

/// `{seconds, forMinutes}` from a `latency_above` rule (defaults 60 s for 5 min).
pub fn threshold(rule: &AlertRule) -> (i64, u32) {
    let v = rule.condition_value.as_ref();
    let secs = v
        .and_then(|v| v.get("seconds"))
        .and_then(Value::as_i64)
        .unwrap_or(60)
        .max(1);
    let mins = v
        .and_then(|v| v.get("forMinutes"))
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .min(24 * 60) as u32;
    (secs, mins)
}

fn fmt_ms(ms: i64) -> String {
    if ms < 1_000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.0)
    } else {
        format!("{}m {}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

async fn evaluate_alerts(state: &AppState, rules: &[AlertRule], p: &CdcProfile, s: &Sample) {
    let matched = crate::watcher::rules::rules_for(rules, "latency_above", &p.name);
    if matched.is_empty() {
        return;
    }
    let Some(e2e) = s.e2e_ms else { return };
    for rule in matched {
        let (secs, mins) = threshold(rule);
        let key = (rule.id.clone(), p.id);
        let Some(high) = breach_transition(state, key, e2e > secs * 1000, mins).await else {
            continue;
        };
        let ev = latency_event(state, p, s, e2e, secs, mins, high);
        crate::notify::fanout(state, &[rule], ev).await;
    }
}

/// Tracks a rule's breach for one pipeline: `Some(true)` once it has stayed above the threshold
/// for `mins`, `Some(false)` when a fired breach recovers, otherwise `None`.
async fn breach_transition(
    state: &AppState,
    key: (String, Uuid),
    above: bool,
    mins: u32,
) -> Option<bool> {
    let mut b = state.latency.breaches.lock().await;
    if !above {
        return matches!(b.remove(&key), Some((_, true))).then_some(false);
    }
    let entry = b.entry(key).or_insert((Instant::now(), false));
    let held = entry.0.elapsed() >= Duration::from_secs(u64::from(mins) * 60);
    if entry.1 || !held {
        return None;
    }
    entry.1 = true;
    Some(true)
}

fn latency_event(
    state: &AppState,
    p: &CdcProfile,
    s: &Sample,
    e2e: i64,
    secs: i64,
    mins: u32,
    high: bool,
) -> NotificationEvent {
    let measured = format!(
        "End-to-end {} (threshold {})",
        fmt_ms(e2e),
        fmt_ms(secs * 1000)
    );
    let (condition, severity, title, summary) = if high {
        (
            "latency_above",
            Severity::Warning,
            format!("Pipeline latency high: {}", p.name),
            format!("{measured} for {mins} min · {}", s.method.as_str()),
        )
    } else {
        (
            "latency_recovered",
            Severity::Resolved,
            format!("Pipeline latency back to normal: {}", p.name),
            measured,
        )
    };
    NotificationEvent {
        condition: condition.into(),
        severity,
        title,
        summary,
        subject: p.name.clone(),
        subject_kind: "pipeline".into(),
        from: None,
        to: fmt_ms(e2e),
        errors: vec![],
        ts: Utc::now(),
        rule: None,
        link: crate::notify::link(state, "/pipelines"),
        test: false,
    }
}

/// Prometheus lines for `/metrics`.
pub async fn prometheus(state: &AppState) -> String {
    let mut out = String::from(
        "# HELP cdc_pipeline_latency_seconds Pipeline latency by kind (e2e, capture, sink).\n# TYPE cdc_pipeline_latency_seconds gauge\n",
    );
    let current = state.latency.current_all().await;
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    for c in &current {
        let labels = format!(
            "profile=\"{}\",profile_id=\"{}\",method=\"{}\"",
            esc(&c.name),
            c.profile_id,
            c.sample.method.as_str()
        );
        for (kind, v) in [
            ("e2e", c.sample.e2e_ms),
            ("capture", c.sample.capture_ms),
            ("sink", c.sample.sink_ms),
        ] {
            if let Some(ms) = v {
                out.push_str(&format!(
                    "cdc_pipeline_latency_seconds{{{labels},kind=\"{kind}\"}} {}\n",
                    ms as f64 / 1000.0
                ));
            }
        }
    }
    out.push_str("# HELP cdc_pipeline_lag_records Records the sink has not committed.\n# TYPE cdc_pipeline_lag_records gauge\n");
    for c in &current {
        if let Some(l) = c.sample.lag_records {
            out.push_str(&format!(
                "cdc_pipeline_lag_records{{profile=\"{}\",profile_id=\"{}\"}} {l}\n",
                esc(&c.name),
                c.profile_id
            ));
        }
    }
    out.push_str("# HELP cdc_pipeline_throughput_rps Records per second into the pipeline topics.\n# TYPE cdc_pipeline_throughput_rps gauge\n");
    for c in &current {
        if let Some(t) = c.sample.throughput_rps {
            out.push_str(&format!(
                "cdc_pipeline_throughput_rps{{profile=\"{}\",profile_id=\"{}\"}} {t}\n",
                esc(&c.name),
                c.profile_id
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_committed_offsets_and_active_topics_from_connect() {
        let offsets = json!({"offsets":[
            {"partition":{"kafka_partition":0,"kafka_topic":"a.public.t"},"offset":{"kafka_offset":37310}},
            {"partition":{"server":"x"},"offset":{"lsn":1}}
        ]});
        let m = committed_offsets(&offsets);
        assert_eq!(m.len(), 1);
        assert_eq!(m[&("a.public.t".to_string(), 0)], 37310);
        let topics = json!({"cdc-x": {"topics": ["a.public.t", "a.public.u"]}});
        assert_eq!(
            active_topics(&topics, "cdc-x"),
            vec!["a.public.t", "a.public.u"]
        );
        assert!(active_topics(&topics, "other").is_empty());
    }

    #[test]
    fn threshold_defaults_and_formatting() {
        let rule: AlertRule = serde_json::from_value(json!({
            "id":"r","name":"r","conditionType":"latency_above","connectorFilter":"*","channels":[],
            "conditionValue":{"seconds":30,"forMinutes":2}
        }))
        .unwrap();
        assert_eq!(threshold(&rule), (30, 2));
        let mut d = rule.clone();
        d.condition_value = None;
        assert_eq!(threshold(&d), (60, 5));
        assert_eq!(fmt_ms(850), "850 ms");
        assert_eq!(fmt_ms(12_300), "12.3 s");
        assert_eq!(fmt_ms(130_000), "2m 10s");
    }
}
