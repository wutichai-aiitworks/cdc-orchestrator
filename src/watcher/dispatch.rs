use chrono::Utc;
use serde_json::json;

use crate::audit::{detail_json, emit, new_event};
use crate::models::{AlertRule, CdcAlertPayload, HistoryEntry, StateTransition};
use crate::notify::render::{NotificationEvent, Severity};
use crate::reconcile::{connector_name_for, sink_connector_name_for};
use crate::watcher::rules::rules_matching_transition;
use crate::AppState;

pub async fn handle_transition(state: &AppState, t: &StateTransition, enabled_rules: &[AlertRule]) {
    let matched = rules_matching_transition(enabled_rules, t);

    let rule_label = if matched.is_empty() {
        None
    } else {
        Some(
            matched
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        )
    };

    let mut errors: Vec<String> = Vec::new();
    for te in &t.snapshot.task_errors {
        errors.push(format!("Task {}: {}", te.task_id, te.trace));
    }

    let recovered = crate::watcher::rules::is_recovery(t);
    let event_type = match t.to_state.as_str() {
        "FAILED" => "connector_failed",
        "PAUSED" => "connector_paused",
        _ if recovered => "connector_recovered",
        _ => "connector_state_changed",
    };

    let iso = Utc::now();
    let payload = CdcAlertPayload {
        type_: "cdc-alert".to_string(),
        ts: iso,
        connector: t.connector.clone(),
        from: t.from_state.clone(),
        to: t.to_state.clone(),
        rule: rule_label,
        task_errors: t.snapshot.task_errors.clone(),
    };

    state.history.write().await.push(HistoryEntry {
        payload: payload.clone(),
        resolved: Some(false),
    });

    if let Ok(json) = serde_json::to_string(&payload) {
        let _ = state.alerts_tx.send(json);
    }

    if !matched.is_empty() {
        let (severity, title) = match event_type {
            "connector_failed" => (
                Severity::Critical,
                format!("Connector failed: {}", t.connector),
            ),
            "connector_paused" => (
                Severity::Warning,
                format!("Connector paused: {}", t.connector),
            ),
            "connector_recovered" => (
                Severity::Resolved,
                format!("Connector recovered: {}", t.connector),
            ),
            _ => (
                Severity::Info,
                format!(
                    "Connector {}: {}",
                    t.to_state.to_ascii_lowercase(),
                    t.connector
                ),
            ),
        };
        let ev = NotificationEvent {
            condition: event_type.to_string(),
            severity,
            title,
            summary: format!(
                "{} → {}",
                t.from_state.as_deref().unwrap_or("new"),
                t.to_state
            ),
            subject: t.connector.clone(),
            subject_kind: "connector".into(),
            from: t.from_state.clone(),
            to: t.to_state.clone(),
            errors: errors.clone(),
            ts: iso,
            rule: None,
            link: crate::notify::link(state, "/connectors"),
            test: false,
        };
        crate::notify::fanout(state, &matched, ev).await;
    }

    let audit_action = match t.to_state.as_str() {
        "FAILED" => "connector.failed",
        "PAUSED" => "connector.paused",
        "RUNNING" => "connector.running",
        _ => "connector.state_changed",
    };
    let outcome = if t.to_state == "FAILED" {
        "failure"
    } else {
        "success"
    };
    let linked = state.profiles.list().await.ok().and_then(|profiles| {
        profiles.into_iter().find(|p| {
            connector_name_for(p) == t.connector || sink_connector_name_for(p) == t.connector
        })
    });
    let detail = detail_json(json!({
        "connector": t.connector,
        "from_state": t.from_state,
        "to_state": t.to_state,
        "task_errors": errors,
    }));
    let event = new_event(
        "system:watcher",
        audit_action,
        outcome,
        &detail,
        linked.as_ref().map(|p| p.id),
        linked.as_ref().map(|p| p.name.as_str()),
    );
    emit(state, event).await;
}
