//! Schedule overview: what each pipeline's worker does next.
//!
//! The orchestrator does not run the JDBC polls itself (Kafka Connect does, on its own
//! timer), but the poll-audit watcher keeps a per-pipeline clock aligned to the same
//! interval, and the refresh scheduler decides full refreshes from persisted history.
//! This module turns both into one list the UI can show as "next run" per pipeline.

use axum::extract::State;
use axum::Json;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::Serialize;
use std::time::Instant;
use uuid::Uuid;

use crate::error::AppError;
use crate::models::{CaptureMode, CdcProfile, DeliveryMode, ProfilePattern, ProfileStatus};
use crate::postgres_trigger_cdc::trigger_poll_interval_ms;
use crate::AppState;

/// How often the refresh scheduler sweeps for due full refreshes (see `watcher`).
pub const REFRESH_SWEEP_SECS: u64 = 60;

/// What drives one pipeline's work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleKind {
    /// Interval-driven JDBC poll (incremental / trigger capture): has a next tick.
    Poll,
    /// Continuous log capture (logical decoding): no interval, runs as changes happen.
    Stream,
    /// One-time snapshot: runs once on Start, nothing is scheduled afterwards.
    Once,
    /// Not running (draft, stopped, paused, failed or disabled): nothing will run.
    Idle,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduleEntry {
    pub profile_id: Uuid,
    pub name: String,
    pub tags: Vec<String>,
    pub status: ProfileStatus,
    pub enabled: bool,
    pub pattern: ProfilePattern,
    pub capture_mode: CaptureMode,
    pub delivery_mode: DeliveryMode,
    pub kind: ScheduleKind,
    /// Poll interval the connector runs on (configured micro-batch interval or the
    /// 1 s default). Present for poll pipelines even while idle, so the list shows what
    /// would run.
    pub interval_ms: Option<u64>,
    /// Where `interval_ms` comes from: `micro_batch` (configured) or `default`.
    pub interval_source: Option<&'static str>,
    /// Last tick of the orchestrator's poll-audit clock for this pipeline. Aligned to the
    /// connector's interval, not the connector's exact poll instant.
    pub last_tick_at: Option<DateTime<Utc>>,
    /// `last_tick_at + interval_ms`. In the past when the audit sweep has not caught up yet.
    pub next_tick_at: Option<DateTime<Utc>>,
    pub full_refresh_minutes: Option<u32>,
    pub last_full_refresh_at: Option<DateTime<Utc>>,
    /// When the refresh scheduler will next empty the target and replay the source. Equal
    /// to `now` when a running pipeline has never refreshed: the next sweep runs it.
    pub next_full_refresh_at: Option<DateTime<Utc>>,
    /// Soonest of `next_tick_at` and `next_full_refresh_at`; the list is sorted on it.
    pub next_run_at: Option<DateTime<Utc>>,
    /// Last source offsets the poll audit read from Kafka Connect, as returned by Connect.
    pub source_offsets: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ScheduleResponse {
    pub timestamp: DateTime<Utc>,
    /// How often the poll-audit clock is checked; `last_tick_at` is accurate to this.
    pub poll_audit_check_secs: u64,
    pub refresh_sweep_secs: u64,
    pub entries: Vec<ScheduleEntry>,
}

fn is_running(profile: &CdcProfile) -> bool {
    profile.enabled && profile.status == ProfileStatus::Running
}

fn kind_for(profile: &CdcProfile) -> ScheduleKind {
    if !is_running(profile) {
        return ScheduleKind::Idle;
    }
    match profile.source.capture_mode {
        CaptureMode::Incremental | CaptureMode::Trigger
            if profile.pattern == ProfilePattern::SourceToTarget =>
        {
            ScheduleKind::Poll
        }
        CaptureMode::SyncOnce => ScheduleKind::Once,
        _ => ScheduleKind::Stream,
    }
}

fn polls_on_interval(profile: &CdcProfile) -> bool {
    matches!(
        profile.source.capture_mode,
        CaptureMode::Incremental | CaptureMode::Trigger
    ) && profile.pattern == ProfilePattern::SourceToTarget
}

/// Build one entry. `clock` is the poll-audit clock reading for this pipeline, if any,
/// and `now_instant` the monotonic instant that pairs with `now`.
pub fn build_entry(
    profile: &CdcProfile,
    now: DateTime<Utc>,
    now_instant: Instant,
    clock: Option<Instant>,
    source_offsets: Option<String>,
) -> ScheduleEntry {
    let kind = kind_for(profile);
    let (interval_ms, interval_source) = if polls_on_interval(profile) {
        let configured = profile.flow.schedule.delivery_mode == DeliveryMode::MicroBatch
            && profile
                .flow
                .schedule
                .interval_seconds
                .is_some_and(|s| s > 0);
        (
            Some(trigger_poll_interval_ms(profile)),
            Some(if configured { "micro_batch" } else { "default" }),
        )
    } else {
        (None, None)
    };

    let (last_tick_at, next_tick_at) = match (kind, interval_ms, clock) {
        (ScheduleKind::Poll, Some(ms), Some(tick)) => {
            let elapsed = now_instant.saturating_duration_since(tick);
            let last = now - ChronoDuration::from_std(elapsed).unwrap_or_default();
            let next = last + ChronoDuration::milliseconds(ms as i64);
            (Some(last), Some(next))
        }
        _ => (None, None),
    };

    let full_refresh_minutes = profile
        .flow
        .schedule
        .full_refresh_minutes
        .filter(|m| *m > 0);
    let last_full_refresh_at =
        full_refresh_minutes.and_then(|_| crate::watcher::last_full_refresh_at(&profile.history));
    let next_full_refresh_at = match (kind, full_refresh_minutes) {
        (ScheduleKind::Idle, _) | (_, None) => None,
        (_, Some(every)) => Some(
            last_full_refresh_at
                .map(|last| last + ChronoDuration::minutes(i64::from(every)))
                .unwrap_or(now),
        ),
    };

    let next_run_at = [next_tick_at, next_full_refresh_at]
        .into_iter()
        .flatten()
        .min();

    ScheduleEntry {
        profile_id: profile.id,
        name: profile.name.clone(),
        tags: profile.tags.clone(),
        status: profile.status.clone(),
        enabled: profile.enabled,
        pattern: profile.pattern.clone(),
        capture_mode: profile.source.capture_mode.clone(),
        delivery_mode: profile.flow.schedule.delivery_mode.clone(),
        kind,
        interval_ms,
        interval_source,
        last_tick_at,
        next_tick_at,
        full_refresh_minutes,
        last_full_refresh_at,
        next_full_refresh_at,
        next_run_at,
        source_offsets,
    }
}

/// Sort: pipelines with a next run first (soonest first), then streaming, then idle;
/// ties by name.
pub fn sort_entries(entries: &mut [ScheduleEntry]) {
    entries.sort_by(|a, b| {
        let rank = |e: &ScheduleEntry| match (e.next_run_at.is_some(), e.kind) {
            (true, _) => 0,
            (false, ScheduleKind::Stream) => 1,
            (false, ScheduleKind::Once) => 2,
            (false, ScheduleKind::Poll) => 3,
            (false, ScheduleKind::Idle) => 4,
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.next_run_at.cmp(&b.next_run_at))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// `GET /cdc/schedule` — every pipeline with its interval and next run.
pub async fn get_schedule(
    State(state): State<AppState>,
) -> Result<Json<ScheduleResponse>, AppError> {
    let profiles = state.profiles.list().await?;
    let now = Utc::now();
    let now_instant = Instant::now();
    let clock = state.poll_audit_clock.read().await;
    let offsets = state.poll_audit_offsets.read().await;
    let mut entries: Vec<ScheduleEntry> = profiles
        .iter()
        .map(|p| {
            build_entry(
                p,
                now,
                now_instant,
                clock.get(&p.id).copied(),
                offsets.get(&p.id).cloned(),
            )
        })
        .collect();
    drop(clock);
    drop(offsets);
    sort_entries(&mut entries);
    Ok(Json(ScheduleResponse {
        timestamp: now,
        poll_audit_check_secs: state.config.poll_audit_check_secs.max(1),
        refresh_sweep_secs: REFRESH_SWEEP_SECS,
        entries,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::HistoryItem;
    use std::time::Duration;

    fn running_incremental(interval_seconds: Option<u32>) -> CdcProfile {
        let mut p = crate::postgres_trigger_cdc::tests::profile();
        p.status = ProfileStatus::Running;
        p.enabled = true;
        p.pattern = ProfilePattern::SourceToTarget;
        p.source.capture_mode = CaptureMode::Incremental;
        p.flow.schedule.delivery_mode = if interval_seconds.is_some() {
            DeliveryMode::MicroBatch
        } else {
            DeliveryMode::Realtime
        };
        p.flow.schedule.interval_seconds = interval_seconds;
        p
    }

    #[test]
    fn poll_pipeline_reports_interval_and_next_tick_from_audit_clock() {
        let p = running_incremental(Some(30));
        let now = Utc::now();
        let now_instant = Instant::now();
        let tick = now_instant - Duration::from_secs(10);
        let e = build_entry(&p, now, now_instant, Some(tick), Some("{}".into()));
        assert_eq!(e.kind, ScheduleKind::Poll);
        assert_eq!(e.interval_ms, Some(30_000));
        assert_eq!(e.interval_source, Some("micro_batch"));
        let next = e.next_tick_at.expect("next tick");
        let secs = (next - now).num_seconds();
        assert!((19..=20).contains(&secs), "expected ~20s, got {secs}");
        assert_eq!(e.next_run_at, e.next_tick_at);
        assert_eq!(e.source_offsets.as_deref(), Some("{}"));
    }

    #[test]
    fn realtime_poll_uses_default_interval_and_waits_for_first_audit_tick() {
        let p = running_incremental(None);
        let e = build_entry(&p, Utc::now(), Instant::now(), None, None);
        assert_eq!(e.kind, ScheduleKind::Poll);
        assert_eq!(e.interval_ms, Some(1000));
        assert_eq!(e.interval_source, Some("default"));
        assert!(e.next_tick_at.is_none());
        assert!(e.next_run_at.is_none());
    }

    #[test]
    fn idle_pipeline_keeps_configured_interval_but_schedules_nothing() {
        let mut p = running_incremental(Some(60));
        p.status = ProfileStatus::Stopped;
        p.flow.schedule.full_refresh_minutes = Some(15);
        let e = build_entry(&p, Utc::now(), Instant::now(), Some(Instant::now()), None);
        assert_eq!(e.kind, ScheduleKind::Idle);
        assert_eq!(e.interval_ms, Some(60_000));
        assert!(e.next_tick_at.is_none());
        assert!(e.next_full_refresh_at.is_none());
        assert_eq!(e.full_refresh_minutes, Some(15));
    }

    #[test]
    fn full_refresh_next_run_follows_last_resync_in_history() {
        let mut p = running_incremental(Some(30));
        p.flow.schedule.full_refresh_minutes = Some(60);
        let now = Utc::now();
        p.history.push(HistoryItem {
            timestamp: now - ChronoDuration::minutes(45),
            level: "info".into(),
            event: "profile_resynced".into(),
            detail: "scheduled".into(),
        });
        let e = build_entry(&p, now, Instant::now(), None, None);
        let next = e.next_full_refresh_at.expect("next refresh");
        assert_eq!((next - now).num_minutes(), 15);
        // No poll tick yet, so the refresh is the next run.
        assert_eq!(e.next_run_at, Some(next));

        // Never refreshed: due at the next sweep.
        p.history.clear();
        let e = build_entry(&p, now, Instant::now(), None, None);
        assert_eq!(e.next_full_refresh_at, Some(now));
    }

    #[test]
    fn logical_capture_is_a_stream_without_interval() {
        let mut p = running_incremental(None);
        p.source.capture_mode = CaptureMode::Logical;
        let e = build_entry(&p, Utc::now(), Instant::now(), None, None);
        assert_eq!(e.kind, ScheduleKind::Stream);
        assert!(e.interval_ms.is_none());
    }

    #[test]
    fn entries_sort_soonest_first_then_stream_then_idle() {
        let now = Utc::now();
        let now_instant = Instant::now();
        let mut soon = running_incremental(Some(10));
        soon.name = "b soon".into();
        let mut later = running_incremental(Some(100));
        later.name = "a later".into();
        let mut stream = running_incremental(None);
        stream.name = "stream".into();
        stream.source.capture_mode = CaptureMode::Logical;
        let mut idle = running_incremental(Some(5));
        idle.name = "idle".into();
        idle.status = ProfileStatus::Draft;
        let mut entries = vec![
            build_entry(&idle, now, now_instant, None, None),
            build_entry(&stream, now, now_instant, None, None),
            build_entry(&later, now, now_instant, Some(now_instant), None),
            build_entry(&soon, now, now_instant, Some(now_instant), None),
        ];
        sort_entries(&mut entries);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["b soon", "a later", "stream", "idle"]);
    }
}
