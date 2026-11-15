//! Redis sync supervisor: alert when a real-time RedisShake sync stops, and restart it under the
//! profile's auto-restart policy (docs/production-rollout-guideline.md §2.2).
//!
//! RedisShake keeps no checkpoint, so every (re)start is a **full snapshot** (BGSAVE fork + RDB)
//! on the source. The policy therefore defaults to `off`; `window` restarts only inside the
//! profile's clock window (ORCH_TIME_ZONE_OFFSET time, default 00:00-05:00), `always` at any time.
//! Restarts back off 10 min → 30 min → 90 min and stop after three in 24 h ("gave up" alert),
//! so a sync that keeps failing cannot hammer the source with snapshots.
//!
//! Alerts go to rules with condition `redis_sync_down` (subject = the profile name). The down /
//! recovered / restart audit entries carry no `profile_id` (these are Copy & Sync profiles, not
//! pipelines), so they never trigger `pipeline_failed` rules.
//!
//! State is in memory (one orchestrator instance); after an orchestrator restart a still-failed
//! sync is alerted again once.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Timelike, Utc};
use serde_json::{json, Value};
use tokio::time::MissedTickBehavior;
use uuid::Uuid;

use crate::error::AppError;
use crate::models::RedisShakeProfile;
use crate::notify::render::{NotificationEvent, Severity};
use crate::AppState;

pub const CONDITION: &str = "redis_sync_down";
const ACTOR: &str = "system:watcher";
const TICK: Duration = Duration::from_secs(60);
const MAX_RESTARTS_PER_DAY: usize = 3;

#[derive(Default)]
struct Track {
    /// Job id already alerted as down (one alert per failed run).
    alerted_job: Option<String>,
    down: bool,
    gave_up: bool,
    restarts: Vec<DateTime<Utc>>,
    next_try: Option<DateTime<Utc>>,
}

static STATE: Mutex<Option<HashMap<Uuid, Track>>> = Mutex::new(None);

pub fn spawn(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if state.config.redis_shake_url.is_none() {
            return;
        }
        let mut iv = tokio::time::interval(TICK);
        iv.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            iv.tick().await;
            if let Err(e) = tick(&state, Utc::now()).await {
                tracing::warn!(target: "cdc_watcher", error = %e, "redis sync supervisor tick failed");
            }
        }
    })
}

// ── pure decisions (unit-tested) ────────────────────────────────────────────

/// What the latest run of a real-time profile means.
#[derive(Debug, PartialEq, Eq)]
pub enum RunState {
    Running,
    /// Stopped by a person: never restarted, never alerted.
    StoppedByUser,
    /// Failed, lost (sidecar restart), or exited although it should follow changes.
    Down,
    NeverRun,
}

pub fn run_state(latest_status: Option<&str>) -> RunState {
    match latest_status {
        None => RunState::NeverRun,
        Some("running" | "starting" | "queued" | "pending") => RunState::Running,
        Some("stopped") => RunState::StoppedByUser,
        Some(_) => RunState::Down,
    }
}

/// Is `now` inside `window` (`HH:MM-HH:MM`, local time = UTC + offset)? Wraps past midnight.
pub fn in_window(window: &str, offset_minutes: i32, now: DateTime<Utc>) -> bool {
    let Some((start, end)) = crate::redisshake::parse_window(window) else {
        return false;
    };
    let utc_min = (now.hour() * 60 + now.minute()) as i32;
    let local = (utc_min + offset_minutes).rem_euclid(24 * 60) as u32;
    if start < end {
        (start..end).contains(&local)
    } else {
        local >= start || local < end
    }
}

/// Wait before the next restart after `n` restarts in the last 24 h.
pub fn backoff(n: usize) -> chrono::Duration {
    chrono::Duration::minutes(match n {
        0 | 1 => 10,
        2 => 30,
        _ => 90,
    })
}

/// May the supervisor restart now? `Err(reason)` explains why not (shown in status/MCP).
pub fn may_restart(
    policy: &str,
    window: &str,
    offset_minutes: i32,
    restarts_last_day: usize,
    next_try: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<(), &'static str> {
    match policy {
        "always" => {}
        "window" => {
            let w = if window.trim().is_empty() {
                crate::redisshake::DEFAULT_RESTART_WINDOW
            } else {
                window
            };
            if !in_window(w, offset_minutes, now) {
                return Err("outside the restart window");
            }
        }
        _ => return Err("auto-restart is off"),
    }
    if restarts_last_day >= MAX_RESTARTS_PER_DAY {
        return Err("gave up: 3 restarts in 24 h");
    }
    if next_try.is_some_and(|t| now < t) {
        return Err("waiting for backoff");
    }
    Ok(())
}

// ── loop ────────────────────────────────────────────────────────────────────

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

async fn tick(state: &AppState, now: DateTime<Utc>) -> Result<(), AppError> {
    let realtime: Vec<RedisShakeProfile> = state
        .redisshake_profiles
        .list()
        .await?
        .into_iter()
        .filter(|p| p.options.sync_aof)
        .collect();
    if realtime.is_empty() {
        return Ok(());
    }
    let jobs = crate::redisshake::list_jobs(&state.config, &state.http).await?;
    let jobs: Vec<Value> = jobs
        .get("jobs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let rules = crate::watcher::rules::load_enabled_rules(state)
        .await
        .unwrap_or_default();
    for p in realtime {
        let run = LatestRun::of(&jobs, p.id);
        // Decide under the lock, act (await) after it.
        let action = {
            let mut guard = STATE.lock().unwrap();
            let track = guard
                .get_or_insert_with(HashMap::new)
                .entry(p.id)
                .or_default();
            decide(track, &p, &run, state.config.time_zone_offset_minutes, now)
        };
        act(state, &rules, &p, &run, action).await;
    }
    Ok(())
}

/// The newest run of one profile, as the sidecar reports it.
pub struct LatestRun {
    job_id: String,
    status: Option<String>,
    error: Option<String>,
}

impl LatestRun {
    fn of(jobs: &[Value], id: Uuid) -> Self {
        let id = id.to_string();
        let latest = jobs
            .iter()
            .filter(|j| s(j, "profile_id") == id)
            .max_by(|a, b| s(a, "started_at").cmp(s(b, "started_at")));
        Self {
            job_id: latest.map(|j| s(j, "id").to_string()).unwrap_or_default(),
            status: latest.map(|j| s(j, "status").to_string()),
            error: latest
                .map(|j| s(j, "error").to_string())
                .filter(|e| !e.is_empty()),
        }
    }
}

/// What to do for one profile this tick.
#[derive(Debug, PartialEq, Eq)]
pub enum Do {
    Nothing,
    Recovered,
    Down {
        restart: Result<(), &'static str>,
        first_alert: bool,
        gave_up_now: bool,
    },
}

/// Updates the profile's tracking state and returns the action (no I/O).
fn decide(
    t: &mut Track,
    p: &RedisShakeProfile,
    run: &LatestRun,
    offset_minutes: i32,
    now: DateTime<Utc>,
) -> Do {
    t.restarts
        .retain(|r| now - *r < chrono::Duration::hours(24));
    match run_state(run.status.as_deref()) {
        RunState::NeverRun | RunState::StoppedByUser => {
            *t = Track::default();
            Do::Nothing
        }
        RunState::Running if t.down => {
            *t = Track {
                restarts: std::mem::take(&mut t.restarts),
                next_try: t.next_try,
                ..Track::default()
            };
            Do::Recovered
        }
        RunState::Running => Do::Nothing,
        RunState::Down => decide_down(t, p, run, offset_minutes, now),
    }
}

fn decide_down(
    t: &mut Track,
    p: &RedisShakeProfile,
    run: &LatestRun,
    offset_minutes: i32,
    now: DateTime<Utc>,
) -> Do {
    t.down = true;
    let first_alert = t.alerted_job.as_deref() != Some(run.job_id.as_str());
    if first_alert {
        t.alerted_job = Some(run.job_id.clone());
    }
    let restart = may_restart(
        &p.options.auto_restart,
        &p.options.restart_window,
        offset_minutes,
        t.restarts.len(),
        t.next_try,
        now,
    );
    let gave_up_now = matches!(restart, Err(r) if r.starts_with("gave up")) && !t.gave_up;
    t.gave_up |= gave_up_now;
    if restart.is_ok() {
        t.restarts.push(now);
        t.next_try = Some(now + backoff(t.restarts.len()));
    }
    Do::Down {
        restart,
        first_alert,
        gave_up_now,
    }
}

async fn act(
    state: &AppState,
    rules: &[crate::models::AlertRule],
    p: &RedisShakeProfile,
    run: &LatestRun,
    action: Do,
) {
    match action {
        Do::Nothing => {}
        Do::Recovered => {
            audit(
                state,
                "redisshake.recovered",
                "success",
                json!({"profile": p.name, "profile_id": p.id, "job_id": run.job_id}),
            )
            .await;
            notify(
                state,
                rules,
                p,
                Severity::Resolved,
                "Redis sync is running again",
                "The real-time sync is following source changes again.",
                vec![],
            )
            .await;
        }
        Do::Down {
            restart,
            first_alert,
            gave_up_now,
        } => {
            if first_alert {
                alert_down(state, rules, p, run).await;
            }
            if gave_up_now {
                audit(state, "redisshake.auto_restart", "failure", json!({"profile": p.name, "profile_id": p.id, "reason": "3 restarts in 24 h; gave up"})).await;
                notify(state, rules, p, Severity::Critical, "Redis sync auto-restart gave up", "Three restarts in 24 hours failed. Check the source, the target and the sync log, then start it manually.", vec![]).await;
            }
            if restart.is_ok() {
                restart_now(state, rules, p).await;
            }
        }
    }
}

fn policy_text(p: &RedisShakeProfile) -> String {
    match p.options.auto_restart.as_str() {
        "always" => "Auto-restart: any time.".to_string(),
        "window" => {
            let w = if p.options.restart_window.is_empty() {
                crate::redisshake::DEFAULT_RESTART_WINDOW
            } else {
                &p.options.restart_window
            };
            format!("Auto-restart: inside {w}.")
        }
        _ => "Auto-restart is off: restart it at night (a restart takes a full snapshot on the source).".into(),
    }
}

async fn alert_down(
    state: &AppState,
    rules: &[crate::models::AlertRule],
    p: &RedisShakeProfile,
    run: &LatestRun,
) {
    let status = run.status.as_deref().unwrap_or("down");
    audit(
        state,
        "redisshake.down",
        "failure",
        json!({"profile": p.name, "profile_id": p.id, "job_id": run.job_id, "status": run.status, "error": run.error}),
    )
    .await;
    notify(
        state,
        rules,
        p,
        Severity::Critical,
        "Redis real-time sync stopped",
        &format!("The sync is {status}. {}", policy_text(p)),
        run.error.clone().into_iter().collect(),
    )
    .await;
}

async fn restart_now(state: &AppState, rules: &[crate::models::AlertRule], p: &RedisShakeProfile) {
    let q = crate::redisshake_handlers::RedisShakeRunQuery::default();
    let res = crate::redisshake_handlers::start_profile_run(state, p.id, &q, ACTOR).await;
    let Ok(job) = res else {
        let e = res.err().map(|e| e.to_string()).unwrap_or_default();
        audit(
            state,
            "redisshake.auto_restart",
            "failure",
            json!({"profile": p.name, "profile_id": p.id, "error": e}),
        )
        .await;
        return;
    };
    audit(
        state,
        "redisshake.auto_restart",
        "success",
        json!({"profile": p.name, "profile_id": p.id, "new_job_id": job.get("id"), "full_snapshot": p.options.sync_rdb}),
    )
    .await;
    notify(
        state,
        rules,
        p,
        Severity::Info,
        "Restarting Redis sync",
        "Auto-restart started a new run with a full snapshot of the source.",
        vec![],
    )
    .await;
}

async fn audit(state: &AppState, action: &str, outcome: &str, detail: Value) {
    // No profile_id: a failed Copy & Sync run must not fire `pipeline_failed` rules.
    let ev = crate::audit::new_event(
        ACTOR,
        action,
        outcome,
        &crate::audit::detail_json(detail),
        None,
        None,
    );
    crate::audit::emit(state, ev).await;
}

async fn notify(
    state: &AppState,
    rules: &[crate::models::AlertRule],
    p: &RedisShakeProfile,
    severity: Severity,
    title: &str,
    summary: &str,
    errors: Vec<String>,
) {
    let matched = crate::watcher::rules::rules_for(rules, CONDITION, &p.name);
    if matched.is_empty() {
        return;
    }
    let ev = NotificationEvent {
        condition: CONDITION.into(),
        severity,
        title: title.into(),
        summary: summary.into(),
        subject: p.name.clone(),
        subject_kind: "pipeline".into(),
        from: None,
        to: match severity {
            Severity::Resolved => "running",
            Severity::Info => "restarting",
            _ => "stopped",
        }
        .into(),
        errors,
        ts: Utc::now(),
        rule: None,
        link: crate::notify::link(state, "/redisshake"),
        test: false,
    };
    crate::notify::fanout(state, &matched, ev).await;
}

/// Supervisor view of one profile, for the status API / MCP.
pub fn status_of(id: Uuid) -> Value {
    let guard = STATE.lock().unwrap();
    match guard.as_ref().and_then(|m| m.get(&id)) {
        Some(t) => json!({
            "down": t.down,
            "restarts_last_24h": t.restarts.len(),
            "next_restart_after": t.next_try,
            "gave_up": t.gave_up,
        }),
        None => json!({"down": false, "restarts_last_24h": 0}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 15, h, m, 0).unwrap()
    }

    #[test]
    fn run_state_maps_statuses() {
        assert_eq!(run_state(None), RunState::NeverRun);
        assert_eq!(run_state(Some("running")), RunState::Running);
        assert_eq!(run_state(Some("stopped")), RunState::StoppedByUser);
        assert_eq!(run_state(Some("failed")), RunState::Down);
        // A real-time sync that exits "successfully" no longer follows changes.
        assert_eq!(run_state(Some("succeeded")), RunState::Down);
    }

    #[test]
    fn window_respects_offset_and_midnight_wrap() {
        // 00:00-05:00 at UTC+7: 18:30 UTC = 01:30 local → inside; 23:00 UTC = 06:00 local → outside.
        assert!(in_window("00:00-05:00", 7 * 60, at(18, 30)));
        assert!(!in_window("00:00-05:00", 7 * 60, at(23, 0)));
        // Wrapping window 22:00-04:00 (UTC).
        assert!(in_window("22:00-04:00", 0, at(23, 15)));
        assert!(in_window("22:00-04:00", 0, at(3, 59)));
        assert!(!in_window("22:00-04:00", 0, at(4, 0)));
        assert!(!in_window("not-a-window", 0, at(1, 0)));
    }

    fn profile(policy: &str) -> RedisShakeProfile {
        serde_json::from_value(json!({
            "id": Uuid::nil(), "name": "r",
            "source": {"type": "redis", "host": "s"}, "target": {"type": "redis", "host": "t"},
            "options": {"sync_aof": true, "auto_restart": policy},
            "created_at": at(0, 0), "updated_at": at(0, 0)
        }))
        .unwrap()
    }

    fn run(job: &str, status: &str) -> LatestRun {
        LatestRun {
            job_id: job.into(),
            status: Some(status.into()),
            error: None,
        }
    }

    #[test]
    fn decide_alerts_once_restarts_and_recovers() {
        let p = profile("always");
        let mut t = Track::default();
        // First failure: alert + restart.
        let d = decide(&mut t, &p, &run("j1", "failed"), 0, at(3, 0));
        assert_eq!(
            d,
            Do::Down {
                restart: Ok(()),
                first_alert: true,
                gave_up_now: false
            }
        );
        // Same failed job next tick: no second alert, backoff holds the restart.
        let d = decide(&mut t, &p, &run("j1", "failed"), 0, at(3, 1));
        assert_eq!(
            d,
            Do::Down {
                restart: Err("waiting for backoff"),
                first_alert: false,
                gave_up_now: false
            }
        );
        // The new run is up: recovered once, then quiet.
        assert_eq!(
            decide(&mut t, &p, &run("j2", "running"), 0, at(3, 2)),
            Do::Recovered
        );
        assert_eq!(
            decide(&mut t, &p, &run("j2", "running"), 0, at(3, 3)),
            Do::Nothing
        );
        assert_eq!(
            t.restarts.len(),
            1,
            "the 24 h restart count survives recovery"
        );
        // Stopped by a person: state reset, nothing done.
        assert_eq!(
            decide(&mut t, &p, &run("j2", "stopped"), 0, at(3, 4)),
            Do::Nothing
        );
        assert!(!t.down && t.restarts.is_empty());
    }

    #[test]
    fn decide_gives_up_once_after_three_restarts() {
        let p = profile("always");
        let mut t = Track {
            restarts: vec![at(1, 0), at(1, 30), at(2, 0)],
            ..Track::default()
        };
        let d = decide(&mut t, &p, &run("j9", "failed"), 0, at(3, 0));
        assert_eq!(
            d,
            Do::Down {
                restart: Err("gave up: 3 restarts in 24 h"),
                first_alert: true,
                gave_up_now: true
            }
        );
        let d = decide(&mut t, &p, &run("j9", "failed"), 0, at(3, 1));
        assert_eq!(
            d,
            Do::Down {
                restart: Err("gave up: 3 restarts in 24 h"),
                first_alert: false,
                gave_up_now: false
            }
        );
    }

    #[test]
    fn restart_policy_backoff_and_cap() {
        let night = at(18, 30); // 01:30 at UTC+7
        let day = at(3, 0); // 10:00 at UTC+7
        assert_eq!(
            may_restart("", "", 420, 0, None, night),
            Err("auto-restart is off")
        );
        assert_eq!(
            may_restart("off", "", 420, 0, None, night),
            Err("auto-restart is off")
        );
        assert!(
            may_restart("window", "", 420, 0, None, night).is_ok(),
            "default window 00:00-05:00"
        );
        assert_eq!(
            may_restart("window", "", 420, 0, None, day),
            Err("outside the restart window")
        );
        assert!(may_restart("always", "", 420, 0, None, day).is_ok());
        assert_eq!(
            may_restart(
                "always",
                "",
                0,
                0,
                Some(day + chrono::Duration::minutes(5)),
                day
            ),
            Err("waiting for backoff")
        );
        assert_eq!(
            may_restart("always", "", 0, 3, None, day),
            Err("gave up: 3 restarts in 24 h")
        );
        assert_eq!(backoff(1), chrono::Duration::minutes(10));
        assert_eq!(backoff(2), chrono::Duration::minutes(30));
        assert_eq!(backoff(3), chrono::Duration::minutes(90));
    }
}
