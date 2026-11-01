//! Emit audit events on each JDBC poll interval for running incremental/trigger pipelines.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Method;
use tokio::sync::RwLock;
use tokio::time::MissedTickBehavior;
use uuid::Uuid;

use crate::audit::{detail_json, emit, emit_system, new_event};
use crate::error::AppError;
use crate::models::{CaptureMode, ProfilePattern, ProfileStatus};
use crate::postgres_trigger_cdc::trigger_poll_interval_ms;
use crate::reconcile::{connect_request, connector_name_for};
use crate::AppState;

pub type PollAuditClock = Arc<RwLock<HashMap<Uuid, Instant>>>;
pub type PollAuditOffsets = Arc<RwLock<HashMap<Uuid, String>>>;

pub fn spawn_poll_audit(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run_loop(state))
}

async fn run_loop(state: Arc<AppState>) {
    let secs = state.config.poll_audit_check_secs.max(1);
    let mut ticker = tokio::time::interval(Duration::from_secs(secs));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        if let Err(e) = tick(state.as_ref()).await {
            tracing::warn!(target = "cdc_poll_audit", error = %e, "poll audit tick failed");
        }
    }
}

async fn tick(state: &AppState) -> Result<(), AppError> {
    let profiles = state.profiles.list().await?;
    let now = Instant::now();
    for profile in profiles {
        if !profile.enabled || profile.status != ProfileStatus::Running {
            state.poll_audit_clock.write().await.remove(&profile.id);
            state.poll_audit_offsets.write().await.remove(&profile.id);
            continue;
        }
        if profile.pattern != ProfilePattern::SourceToTarget {
            continue;
        }
        if !matches!(
            profile.source.capture_mode,
            CaptureMode::Incremental | CaptureMode::Trigger
        ) {
            continue;
        }
        let interval_ms = trigger_poll_interval_ms(&profile);
        let due = {
            let mut clock = state.poll_audit_clock.write().await;
            let entry = clock.entry(profile.id).or_insert(now);
            if entry.elapsed() < Duration::from_millis(interval_ms) {
                false
            } else {
                *entry = now;
                true
            }
        };
        if !due {
            continue;
        }
        let connector = connector_name_for(&profile);
        let offsets = fetch_source_offsets(state, &connector).await;
        let watermark_advanced = {
            let mut offsets_map = state.poll_audit_offsets.write().await;
            let prev = offsets_map.insert(profile.id, offsets.clone());
            prev.is_some_and(|p| p != offsets)
        };
        let interval_secs = profile
            .flow
            .schedule
            .interval_seconds
            .unwrap_or(if interval_ms >= 1000 {
                (interval_ms / 1000) as u32
            } else {
                1
            });
        let tick_detail = detail_json(serde_json::json!({
            "connector": connector,
            "interval_ms": interval_ms,
            "interval_seconds": interval_secs,
            "delivery_mode": format!("{:?}", profile.flow.schedule.delivery_mode),
            "capture_mode": format!("{:?}", profile.source.capture_mode),
            "offsets": offsets,
            "watermark_advanced": watermark_advanced,
        }));
        let event = new_event(
            "system:poll_audit",
            "sync.interval_tick",
            "success",
            &tick_detail,
            Some(profile.id),
            Some(profile.name.as_str()),
        );
        emit(state, event).await;
        if watermark_advanced {
            emit_system(
                state,
                "sync.watermark_advanced",
                "success",
                serde_json::json!({
                    "connector": connector,
                    "offsets": offsets,
                }),
                Some(profile.id),
                Some(profile.name.as_str()),
            )
            .await;
        }
    }
    Ok(())
}

async fn fetch_source_offsets(state: &AppState, connector: &str) -> String {
    let path = format!("/connectors/{}/offsets", urlencoding::encode(connector));
    match connect_request(state, Method::GET, &path, None).await {
        Ok(v) => v.to_string(),
        Err(e) => format!("unavailable: {e}"),
    }
}
