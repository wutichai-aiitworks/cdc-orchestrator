//! Watcher — poll Connect state, match rules, notify.

mod diff;
mod dispatch;
mod rules;

use std::sync::Arc;
use std::time::Duration;

use reqwest::Method;
use tokio::time::MissedTickBehavior;

use crate::connector_parse::parse_connector_list;
use crate::error::AppError;
use crate::models::{CaptureMode, CdcAlertPayload, HistoryEntry, ProfileStatus};
use crate::reconcile::connect_request;
use crate::AppState;

use dispatch::handle_transition;

pub fn spawn_watcher(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run_loop(state))
}

/// Background job: delete aged rows from trigger-capture changelog tables so
/// production source disks stay bounded. Only running pipelines with
/// `source.changelog_retention_hours > 0` are pruned (paused pipelines keep
/// unconsumed events).
pub fn spawn_changelog_pruner(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run_prune_loop(state))
}

/// Background job: periodically empty the target and replay the whole source for
/// pipelines with `flow.schedule.full_refresh_minutes` set.
///
/// This is the fallback for sources that cannot stream updates and deletes — an
/// incrementing-column poll only ever sees new rows, so a full reload is the only way
/// those pipelines converge on the source.
pub fn spawn_refresh_scheduler(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run_refresh_loop(state))
}

async fn run_refresh_loop(state: Arc<AppState>) {
    let mut ticker = tokio::time::interval(Duration::from_secs(60));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        if let Err(e) = refresh_tick(state.as_ref()).await {
            tracing::warn!(target = "cdc_refresh_scheduler", error = %e, "refresh sweep failed");
        }
    }
}

/// Minutes since the last completed full refresh, read back from profile history.
///
/// Using the persisted history instead of in-memory state means a restart does not
/// trigger an immediate extra refresh.
fn minutes_since_last_refresh(history: &[crate::models::HistoryItem]) -> Option<i64> {
    history
        .iter()
        .rev()
        .find(|item| item.event == REFRESH_EVENT)
        .map(|item| (chrono::Utc::now() - item.timestamp).num_minutes())
}

const REFRESH_EVENT: &str = "profile_resynced";

async fn refresh_tick(state: &AppState) -> Result<(), AppError> {
    for profile in state.profiles.list().await? {
        let Some(every) = profile
            .flow
            .schedule
            .full_refresh_minutes
            .filter(|m| *m > 0)
        else {
            continue;
        };
        if !profile.enabled || profile.status != ProfileStatus::Running {
            continue;
        }
        if minutes_since_last_refresh(&profile.history).is_some_and(|mins| mins < every as i64) {
            continue;
        }
        let mut profile = profile;
        match crate::reconcile::reconcile_resync(state, &mut profile, true).await {
            Ok(()) => {
                state.profiles.upsert(&profile).await?;
                tracing::info!(
                    target = "cdc_refresh_scheduler",
                    profile = %profile.name,
                    every_minutes = every,
                    "scheduled full refresh complete"
                );
            }
            Err(e) => {
                tracing::warn!(
                    target = "cdc_refresh_scheduler",
                    profile = %profile.name,
                    error = %e,
                    "scheduled full refresh failed"
                );
            }
        }
    }
    Ok(())
}

async fn run_prune_loop(state: Arc<AppState>) {
    let secs = state.config.changelog_prune_interval_secs.max(60);
    let mut ticker = tokio::time::interval(Duration::from_secs(secs));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        if let Err(e) = prune_tick(state.as_ref()).await {
            tracing::warn!(target = "cdc_changelog_pruner", error = %e, "prune sweep failed");
        }
    }
}

async fn prune_tick(state: &AppState) -> Result<(), AppError> {
    let profiles = state.profiles.list().await?;
    for profile in profiles {
        if profile.source.capture_mode != CaptureMode::Trigger {
            continue;
        }
        if profile.status != ProfileStatus::Running {
            continue;
        }
        let Some(hours) = profile.source.changelog_retention_hours.filter(|h| *h > 0) else {
            continue;
        };
        match crate::trigger_cdc::prune_changelog(
            &profile.source.source_type,
            &profile.source.connection,
            hours,
        )
        .await
        {
            Ok(Some(deleted)) if deleted > 0 => {
                tracing::info!(
                    target = "cdc_changelog_pruner",
                    profile = %profile.name,
                    hours,
                    deleted,
                    "pruned trigger changelog"
                );
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                let vendor = crate::trigger_cdc::Vendor::parse(&profile.source.source_type)
                    .unwrap_or(crate::trigger_cdc::Vendor::Oracle);
                let owner = crate::trigger_cdc::changelog_owner(vendor, &profile.source.connection);
                let sql = crate::trigger_cdc::prune_sql(vendor, hours, owner.as_deref());
                tracing::warn!(
                    target = "cdc_changelog_pruner",
                    profile = %profile.name,
                    source_type = %profile.source.source_type,
                    hours,
                    sql = %sql,
                    "native changelog prune is not bundled for this vendor; schedule this DELETE on the source"
                );
            }
            Err(e) => {
                tracing::warn!(
                    target = "cdc_changelog_pruner",
                    profile = %profile.name,
                    error = %e,
                    "failed to prune trigger changelog"
                );
            }
        }
    }
    Ok(())
}

async fn run_loop(state: Arc<AppState>) {
    let mut ticker = tokio::time::interval(Duration::from_millis(
        state.config.poll_interval.as_millis().max(1) as u64,
    ));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        if let Err(e) = tick(state.as_ref()).await {
            tracing::warn!(target = "cdc_watcher", error = %e, "tick failed");
            *state.watcher_errors.write().await += 1;
        }
    }
}

async fn tick(state: &AppState) -> Result<(), AppError> {
    let v = connect_request(
        state,
        Method::GET,
        "/connectors?expand=status&expand=info",
        None,
    )
    .await?;
    let next = parse_connector_list(&v);
    let prev = state.connector_snapshots.read().await.clone();

    if prev.is_empty() && !next.is_empty() {
        *state.connector_snapshots.write().await = next;
        check_dlq_growth(state).await?;
        return Ok(());
    }

    let transitions = diff::diff_transitions(&prev, &next);
    let rules_owned = rules::load_enabled_rules(state).await?;
    for t in transitions {
        handle_transition(state, &t, rules_owned.as_slice()).await;
    }
    *state.connector_snapshots.write().await = next;
    check_dlq_growth(state).await?;
    Ok(())
}

async fn check_dlq_growth(state: &AppState) -> Result<(), AppError> {
    let statuses = crate::kafka_topics::load_profile_dlq_statuses(state).await?;
    let mut increases = Vec::new();
    {
        let mut previous = state.dlq_high_watermarks.write().await;
        for status in statuses.into_iter().filter(|status| status.enabled) {
            let Some(topic) = status.topic else {
                continue;
            };
            let old = previous.insert(topic.clone(), status.high_watermark);
            if let Some(old) = old.filter(|old| status.high_watermark > *old) {
                increases.push((topic, old, status.high_watermark, status.messages_count));
            }
        }
    }

    for (topic, old, current, messages_count) in increases {
        let payload = CdcAlertPayload {
            type_: "cdc-alert".to_string(),
            ts: chrono::Utc::now(),
            connector: topic.clone(),
            from: Some(old.to_string()),
            to: current.to_string(),
            rule: Some("DLQ high-watermark increased".to_string()),
            task_errors: vec![],
        };
        state.history.write().await.push(HistoryEntry {
            payload: payload.clone(),
            resolved: Some(false),
        });
        if let Ok(json) = serde_json::to_string(&payload) {
            let _ = state.alerts_tx.send(json);
        }
        tracing::warn!(
            target = "cdc_dlq_watcher",
            topic,
            previous_high_watermark = old,
            high_watermark = current,
            messages = messages_count,
            "DLQ high-watermark increased"
        );
    }
    Ok(())
}

#[cfg(test)]
mod refresh_tests {
    use super::*;
    use crate::models::{HistoryItem, MAX_HISTORY_ITEMS};
    use chrono::{Duration as ChronoDuration, Utc};

    fn history(event: &str, minutes_ago: i64) -> HistoryItem {
        HistoryItem {
            timestamp: Utc::now() - ChronoDuration::minutes(minutes_ago),
            level: "info".to_string(),
            event: event.to_string(),
            detail: String::new(),
        }
    }

    #[test]
    fn a_profile_never_refreshed_is_due_immediately() {
        assert_eq!(minutes_since_last_refresh(&[]), None);
    }

    #[test]
    fn age_comes_from_the_most_recent_refresh_only() {
        let history = vec![
            history(REFRESH_EVENT, 600),
            history("profile_started", 5),
            history(REFRESH_EVENT, 30),
        ];
        let mins = minutes_since_last_refresh(&history).expect("history has a refresh");
        assert!((29..=31).contains(&mins), "unexpected age: {mins}");
    }

    #[test]
    fn unrelated_events_do_not_count_as_a_refresh() {
        let history = vec![history("profile_started", 1), history("profile_paused", 2)];
        assert_eq!(minutes_since_last_refresh(&history), None);
    }

    #[test]
    fn history_is_capped_so_recurring_refreshes_cannot_grow_it_forever() {
        let mut items: Vec<HistoryItem> = Vec::new();
        for i in 0..(MAX_HISTORY_ITEMS + 50) {
            crate::models::push_capped_history(&mut items, history(&format!("event{i}"), 0));
        }
        assert_eq!(items.len(), MAX_HISTORY_ITEMS);
        // The newest entries survive and the oldest are the ones dropped.
        assert_eq!(
            items.last().unwrap().event,
            format!("event{}", MAX_HISTORY_ITEMS + 49)
        );
        assert_eq!(items.first().unwrap().event, "event50");
    }
}
