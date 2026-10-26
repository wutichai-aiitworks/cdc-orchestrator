use std::collections::HashMap;

use crate::models::{ConnectorSnapshot, StateTransition};

pub fn diff_transitions(
    prev: &HashMap<String, ConnectorSnapshot>,
    next: &HashMap<String, ConnectorSnapshot>,
) -> Vec<StateTransition> {
    let mut out = Vec::new();
    for (name, snapshot) in next {
        let from = prev.get(name).map(|s| s.state.clone());
        let to = snapshot.state.clone();
        if from.as_deref() == Some(to.as_str()) {
            continue;
        }
        out.push(StateTransition {
            connector: name.clone(),
            from_state: from,
            to_state: to,
            snapshot: snapshot.clone(),
        });
    }
    out.sort_by(|a, b| a.connector.cmp(&b.connector));
    out
}
