use chrono::Utc;
use tracing::info;
use uuid::Uuid;

pub fn log_action(actor: &str, action: &str, outcome: &str, detail: &str) -> String {
    let audit_id = Uuid::new_v4().to_string();
    info!(
        audit_id = %audit_id,
        actor = %actor,
        action = %action,
        outcome = %outcome,
        detail = %detail,
        timestamp = %Utc::now().to_rfc3339(),
        "audit_event"
    );
    audit_id
}
