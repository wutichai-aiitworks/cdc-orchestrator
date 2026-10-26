use chrono::Utc;

use crate::models::{AlertRule, CdcAlertPayload, HistoryEntry, StateTransition, WebhookBody};
use crate::watcher::rules::rules_matching_transition;
use crate::AppState;

pub async fn handle_transition(state: &AppState, t: &StateTransition, enabled_rules: &[AlertRule]) {
    let matched = rules_matching_transition(enabled_rules, t);
    let webhook_rules: Vec<&AlertRule> = matched
        .iter()
        .copied()
        .filter(|r| r.channels.iter().any(|c| c == "webhook"))
        .collect();

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

    let event_type = match t.to_state.as_str() {
        "FAILED" => "connector_failed",
        "PAUSED" => "connector_paused",
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

    for r in webhook_rules {
        let Some(ref url) = r.webhook_url else {
            continue;
        };
        let body = WebhookBody {
            event: event_type.to_string(),
            connector: t.connector.clone(),
            from: t.from_state.clone(),
            to: t.to_state.clone(),
            ts: iso,
            errors: errors.clone(),
        };
        if let Err(e) = post_webhook(url.as_str(), r.webhook_secret.as_deref(), &body).await {
            tracing::warn!(rule = %r.id, error = %e, "webhook failed");
        }
    }
}

async fn post_webhook(url: &str, secret: Option<&str>, body: &WebhookBody) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.post(url).json(body);
    if let Some(s) = secret {
        req = req.header("X-Webhook-Secret", s);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("{}", resp.status()))
    }
}
