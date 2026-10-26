//! Parse Kafka Connect expanded connector list into watcher snapshots.

use std::collections::HashMap;

use serde_json::Value;

use crate::models::{ConnectorSnapshot, TaskErrorBrief};

pub fn parse_connector_list(value: &Value) -> HashMap<String, ConnectorSnapshot> {
    let mut map = HashMap::new();
    let Some(obj) = value.as_object() else {
        return map;
    };
    for (name, payload) in obj {
        map.insert(name.clone(), normalize_connector(name, payload));
    }
    map
}

fn normalize_connector(name: &str, payload: &Value) -> ConnectorSnapshot {
    let state = aggregate_state(payload).unwrap_or_else(|| "UNKNOWN".to_string());
    let task_errors = collect_task_errors(payload);
    ConnectorSnapshot {
        name: name.to_string(),
        state,
        task_errors,
    }
}

fn aggregate_state(payload: &Value) -> Option<String> {
    let status = payload.get("status")?;
    let conn = status.get("connector").and_then(|c| c.get("state"))?;
    let conn_s = normalize_state_token(conn)?;

    let mut any_failed_task = false;
    if let Some(tasks) = status.get("tasks").and_then(|t| t.as_array()) {
        for t in tasks {
            let st = t.get("state").and_then(normalize_state_token);
            if st.as_deref() == Some("FAILED") {
                any_failed_task = true;
                break;
            }
        }
    }

    if any_failed_task || conn_s == "FAILED" {
        return Some("FAILED".into());
    }
    Some(conn_s)
}

fn normalize_state_token(v: &Value) -> Option<String> {
    Some(v.as_str()?.to_uppercase())
}

fn collect_task_errors(payload: &Value) -> Vec<TaskErrorBrief> {
    let mut out = Vec::new();
    let Some(status) = payload.get("status") else {
        return out;
    };
    let Some(tasks) = status.get("tasks").and_then(|t| t.as_array()) else {
        return out;
    };
    for task in tasks {
        let Some(id) = task.get("id").and_then(|i| i.as_u64()).map(|u| u as u32) else {
            continue;
        };
        if task.get("state").and_then(|s| s.as_str()).unwrap_or("") != "FAILED" {
            continue;
        }
        let trace = task
            .get("trace")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        if trace.is_empty() {
            continue;
        }
        out.push(TaskErrorBrief { task_id: id, trace });
    }
    out
}
