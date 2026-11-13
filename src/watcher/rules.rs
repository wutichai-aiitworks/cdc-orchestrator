use globset::Glob;

use crate::error::AppError;
use crate::models::{AlertRule, StateTransition};
use crate::AppState;

/// Conditions a rule can use (ADR 0015). `connector_paused_N_min` is the pre-0015 spelling
/// of `connector_paused`.
pub const CONDITIONS: &[&str] = &[
    "connector_failed",
    "connector_paused",
    "connector_recovered",
    "connector_failed_for",
    "connector_state_changed",
    "dlq_growth",
    "pipeline_failed",
    "latency_above",
];

pub fn normalize_condition(c: &str) -> &str {
    match c {
        "connector_paused_N_min" => "connector_paused",
        other => other,
    }
}

pub async fn load_enabled_rules(state: &AppState) -> Result<Vec<AlertRule>, AppError> {
    if let Some(rq) = state.rqlite.as_ref() {
        let all = rq
            .list_json::<AlertRule>(hyper_cdc_orchestrator::persist::KIND_RULE)
            .await?;
        return Ok(all.into_iter().filter(|r| r.enabled).collect());
    }
    Ok(Vec::new())
}

/// Glob match of the rule's filter (connector, topic or pipeline name). An invalid or empty
/// filter matches everything, as before.
pub fn filter_matches(rule: &AlertRule, subject: &str) -> bool {
    let pattern = rule.connector_filter.trim();
    if pattern.is_empty() {
        return true;
    }
    match Glob::new(pattern) {
        Ok(g) => g.compile_matcher().is_match(subject),
        Err(_) => true,
    }
}

pub fn rules_matching_transition<'a>(
    rules: &'a [AlertRule],
    t: &'a StateTransition,
) -> Vec<&'a AlertRule> {
    rules
        .iter()
        .filter(|r| rule_matches_transition(r, t))
        .collect()
}

/// Enabled rules with `condition` whose filter matches `subject`.
pub fn rules_for<'a>(rules: &'a [AlertRule], condition: &str, subject: &str) -> Vec<&'a AlertRule> {
    rules
        .iter()
        .filter(|r| {
            r.enabled
                && normalize_condition(&r.condition_type) == condition
                && filter_matches(r, subject)
        })
        .collect()
}

pub fn is_recovery(t: &StateTransition) -> bool {
    t.to_state == "RUNNING"
        && matches!(
            t.from_state.as_deref(),
            Some("FAILED" | "PAUSED" | "UNASSIGNED")
        )
}

fn rule_matches_transition(rule: &AlertRule, t: &StateTransition) -> bool {
    if !rule.enabled || !filter_matches(rule, &t.connector) {
        return false;
    }
    match normalize_condition(&rule.condition_type) {
        "connector_failed" => t.to_state == "FAILED",
        "connector_paused" => t.to_state == "PAUSED",
        "connector_recovered" => is_recovery(t),
        "connector_state_changed" => true,
        _ => false,
    }
}

/// Minutes for `connector_failed_for` (`conditionValue` as `{"minutes": n}` or a number).
pub fn failed_for_minutes(rule: &AlertRule) -> u32 {
    let v = rule.condition_value.as_ref();
    v.and_then(|v| {
        v.get("minutes")
            .and_then(|m| m.as_u64())
            .or_else(|| v.as_u64())
    })
    .unwrap_or(10)
    .clamp(1, 7 * 24 * 60) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ConnectorSnapshot;

    fn rule(condition: &str, filter: &str) -> AlertRule {
        serde_json::from_value(serde_json::json!({
            "id": "r", "name": "r", "conditionType": condition, "connectorFilter": filter,
            "channels": [], "enabled": true
        }))
        .unwrap()
    }

    fn t(name: &str, from: Option<&str>, to: &str) -> StateTransition {
        let snapshot: ConnectorSnapshot = serde_json::from_value(serde_json::json!({
            "name": name, "state": to, "taskErrors": []
        }))
        .unwrap_or_else(|_| panic!("ConnectorSnapshot shape changed"));
        StateTransition {
            connector: name.into(),
            from_state: from.map(Into::into),
            to_state: to.into(),
            snapshot,
        }
    }

    #[test]
    fn conditions_match_the_transitions_they_name() {
        let rules = vec![
            rule("connector_failed", "orders-*"),
            rule("connector_recovered", "*"),
            rule("connector_paused_N_min", "*"),
            rule("connector_state_changed", "*"),
        ];
        let failed = t("orders-src", Some("RUNNING"), "FAILED");
        let names: Vec<_> = rules_matching_transition(&rules, &failed)
            .iter()
            .map(|r| r.condition_type.clone())
            .collect();
        assert_eq!(names, vec!["connector_failed", "connector_state_changed"]);
        assert!(rules_matching_transition(
            &rules[..1],
            &t("billing-src", Some("RUNNING"), "FAILED")
        )
        .is_empty());
        assert_eq!(
            rules_matching_transition(&rules[1..2], &t("x", Some("FAILED"), "RUNNING")).len(),
            1
        );
        assert!(
            rules_matching_transition(&rules[1..2], &t("x", None, "RUNNING")).is_empty(),
            "a new connector is not a recovery"
        );
        assert_eq!(
            rules_matching_transition(&rules[2..3], &t("x", Some("RUNNING"), "PAUSED")).len(),
            1
        );
    }

    #[test]
    fn failed_for_reads_minutes_in_either_shape() {
        let mut r = rule("connector_failed_for", "*");
        assert_eq!(failed_for_minutes(&r), 10);
        r.condition_value = Some(serde_json::json!({"minutes": 30}));
        assert_eq!(failed_for_minutes(&r), 30);
        r.condition_value = Some(serde_json::json!(5));
        assert_eq!(failed_for_minutes(&r), 5);
    }

    #[test]
    fn rules_for_filters_by_condition_and_subject() {
        let rules = vec![rule("pipeline_failed", "prod-*"), rule("dlq_growth", "*")];
        assert_eq!(rules_for(&rules, "pipeline_failed", "prod-orders").len(), 1);
        assert!(rules_for(&rules, "pipeline_failed", "dev-orders").is_empty());
        assert_eq!(rules_for(&rules, "dlq_growth", "dlq.orders").len(), 1);
    }
}
