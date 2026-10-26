use globset::Glob;

use crate::error::AppError;
use crate::models::{AlertRule, StateTransition};
use crate::AppState;

pub async fn load_enabled_rules(state: &AppState) -> Result<Vec<AlertRule>, AppError> {
    if let Some(rq) = state.rqlite.as_ref() {
        let all = rq
            .list_json::<AlertRule>(hyper_cdc_orchestrator::persist::KIND_RULE)
            .await?;
        return Ok(all.into_iter().filter(|r| r.enabled).collect());
    }
    Ok(Vec::new())
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

fn rule_matches_transition(rule: &AlertRule, t: &StateTransition) -> bool {
    if !rule.enabled {
        return false;
    }
    let matcher = Glob::new(&rule.connector_filter)
        .or_else(|_| Glob::new("*"))
        .map(|g| g.compile_matcher())
        .ok();
    if let Some(m) = matcher {
        if !m.is_match(&t.connector) {
            return false;
        }
    }
    match rule.condition_type.as_str() {
        "connector_failed" => t.to_state == "FAILED",
        "connector_paused" | "connector_paused_N_min" => t.to_state == "PAUSED",
        _ => false,
    }
}
