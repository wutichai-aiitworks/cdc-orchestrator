//! Split some tables of a pipeline into a new pipeline with the same source and target.
//!
//! The new pipeline is a copy of the original restricted to the chosen tables (their
//! mappings and per-table poll rules move with them) and is saved as a Draft — never
//! started here. It gets its own identity on the shared source: connector + sink names,
//! Kafka topic prefix and (for logical replication) replication slot all derive from
//! its new name, and a slot pinned in the copied connection is dropped so two pipelines
//! can never fight over one slot.
//!
//! The original loses those tables in its saved configuration. If it is running, its
//! connectors keep the old table list until it is restarted — on purpose: while both
//! pipelines carry a table they upsert the same rows by key, so starting the new
//! pipeline first and restarting the original second loses nothing.

use std::collections::HashSet;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::models::{CaptureMode, CdcProfile, HistoryItem, ProfileStatus, TableSelectionMode};
use crate::reconcile::{connector_name_for, sink_connector_name_for, topic_prefix_for};

#[derive(Debug, Deserialize)]
pub struct SplitRequest {
    /// Name of the new pipeline.
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// `schema.table` source names to move (as listed in `source.tables`).
    pub tables: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SplitResponse {
    pub original: CdcProfile,
    pub created: CdcProfile,
    pub moved_tables: Vec<String>,
    /// The original is running/paused: its connectors still carry the moved tables
    /// until it is restarted (start the new pipeline first, then restart this one).
    pub original_restart_required: bool,
}

fn norm(t: &str) -> String {
    t.trim().to_ascii_lowercase()
}

fn mapping_key(m: &crate::models::TableMapping) -> String {
    norm(&format!("{}.{}", m.source_schema, m.source_table))
}

type SplitTableLists = (HashSet<String>, Vec<String>, Vec<String>);

fn split_table_lists(
    original: &CdcProfile,
    req: &SplitRequest,
) -> Result<SplitTableLists, AppError> {
    let wanted: HashSet<String> = req
        .tables
        .iter()
        .map(|t| norm(t))
        .filter(|t| !t.is_empty())
        .collect();
    if wanted.is_empty() {
        return Err(AppError::BadRequest(
            "select at least one table to move".into(),
        ));
    }
    let owned: HashSet<String> = original.source.tables.iter().map(|t| norm(t)).collect();
    let unknown: Vec<&String> = wanted.iter().filter(|t| !owned.contains(*t)).collect();
    if !unknown.is_empty() {
        let mut list: Vec<String> = unknown.into_iter().cloned().collect();
        list.sort();
        return Err(AppError::BadRequest(format!(
            "not tables of this pipeline: {}",
            list.join(", ")
        )));
    }
    let (moved, kept): (Vec<String>, Vec<String>) = original
        .source
        .tables
        .iter()
        .filter(|t| !t.trim().is_empty())
        .cloned()
        .partition(|t| wanted.contains(&norm(t)));
    if kept.is_empty() {
        return Err(AppError::BadRequest(
            "leave at least one table in this pipeline — to move all of them, rename the pipeline instead".into(),
        ));
    }
    Ok((wanted, moved, kept))
}

fn ensure_split_allowed(original: &CdcProfile, name: &str) -> Result<(), AppError> {
    if name.is_empty() {
        return Err(AppError::BadRequest("name the new pipeline".into()));
    }
    if original.source.capture_mode == CaptureMode::Trigger {
        return Err(AppError::BadRequest(
            "Trigger-capture pipelines cannot be split: their triggers and inbox belong to one pipeline. Create a new pipeline for those tables instead.".into(),
        ));
    }
    if original.pattern.is_olake_capture() {
        return Err(AppError::BadRequest(
            "Analytics-first (OLake) pipelines cannot be split.".into(),
        ));
    }
    if original.source.table_mode != TableSelectionMode::SpecificTables
        || original.source.tables.iter().all(|t| t.trim().is_empty())
    {
        return Err(AppError::BadRequest(
            "Only pipelines with an explicit table list can be split. Choose specific tables on the pipeline first.".into(),
        ));
    }
    Ok(())
}

/// Build the new pipeline and the trimmed original. Pure: the caller persists both.
pub fn plan_split(
    original: &CdcProfile,
    others: &[CdcProfile],
    req: &SplitRequest,
) -> Result<(CdcProfile, CdcProfile, Vec<String>), AppError> {
    let name = req.name.trim();
    ensure_split_allowed(original, name)?;
    let (_wanted, moved, kept) = split_table_lists(original, req)?;

    // New identity must not collide with any pipeline (names, connectors, topics).
    let mut created = original.clone();
    created.name = name.to_string();
    created.connector_name = None;
    created.sink_connector_name = None;
    let new_conn = connector_name_for(&created);
    let new_sink = sink_connector_name_for(&created);
    let new_prefix = topic_prefix_for(&created);
    for p in others.iter().chain(std::iter::once(original)) {
        if p.name.trim().eq_ignore_ascii_case(name) {
            return Err(AppError::BadRequest(format!(
                "a pipeline named '{}' already exists",
                p.name
            )));
        }
        if connector_name_for(p) == new_conn || sink_connector_name_for(p) == new_sink {
            return Err(AppError::BadRequest(format!(
                "the name '{name}' gives connector '{new_conn}', already used by '{}'. Pick a more distinct name.",
                p.name
            )));
        }
        if topic_prefix_for(p) == new_prefix {
            return Err(AppError::BadRequest(format!(
                "the name '{name}' gives Kafka topic prefix '{new_prefix}', already used by '{}'. Pick a more distinct name.",
                p.name
            )));
        }
    }

    let now = Utc::now();
    let moved_set: HashSet<String> = moved.iter().map(|t| norm(t)).collect();

    created.id = Uuid::new_v4();
    created.description = req
        .description
        .clone()
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| format!("Split from '{}'.", original.name));
    created.status = ProfileStatus::Draft;
    created.enabled = true;
    created.autostart = false;
    created.last_error = None;
    created.created_at = now;
    created.updated_at = now;
    created.history = Vec::new();
    created.source.tables = moved.clone();
    // A pinned slot would be shared with the original: let it derive from the new name.
    created.source.connection.remove("slot");
    created
        .source
        .table_poll
        .retain(|r| moved_set.contains(&norm(&r.table)));
    created
        .flow
        .mappings
        .retain(|m| moved_set.contains(&mapping_key(m)));
    if created.governance.dataset.trim().is_empty() || created.governance.dataset == original.name {
        created.governance.dataset = created.name.clone();
    }
    created.push_history(HistoryItem {
        timestamp: now,
        level: "info".into(),
        event: "profile_created".into(),
        detail: format!(
            "Split from '{}' with {} table(s): {}",
            original.name,
            moved.len(),
            moved.join(", ")
        ),
    });

    let mut trimmed = original.clone();
    trimmed.source.tables = kept;
    trimmed
        .source
        .table_poll
        .retain(|r| !moved_set.contains(&norm(&r.table)));
    trimmed
        .flow
        .mappings
        .retain(|m| !moved_set.contains(&mapping_key(m)));
    trimmed.updated_at = now;
    let running = matches!(
        original.status,
        ProfileStatus::Running | ProfileStatus::Paused
    );
    trimmed.push_history(HistoryItem {
        timestamp: now,
        level: "info".into(),
        event: "profile_split".into(),
        detail: format!(
            "Moved {} table(s) to new pipeline '{}': {}{}",
            moved.len(),
            created.name,
            moved.join(", "),
            if running {
                ". Restart this pipeline after starting the new one to stop capturing them here."
            } else {
                ""
            }
        ),
    });

    Ok((created, trimmed, moved))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TableMapping;

    fn profile() -> CdcProfile {
        let mut p = crate::postgres_trigger_cdc::tests::profile();
        p.name = "CollabX Sync".into();
        p.source.capture_mode = CaptureMode::Logical;
        p.source.table_mode = TableSelectionMode::SpecificTables;
        p.source.tables = vec![
            "public.archived_messages".into(),
            "public.groups".into(),
            "public.group_users".into(),
        ];
        p.source
            .connection
            .insert("slot".into(), "cdc_collabx_sync".into());
        p.flow.mappings = p
            .source
            .tables
            .iter()
            .map(|t| {
                let (s, n) = t.split_once('.').unwrap();
                TableMapping {
                    source_schema: s.into(),
                    source_table: n.into(),
                    target_schema: "collabx".into(),
                    target_table: n.into(),
                    identical: true,
                    columns: vec![],
                    pk_columns: vec![],
                }
            })
            .collect();
        p.status = ProfileStatus::Running;
        p.connector_name = Some("cdc-draft---postgres".into());
        p
    }

    fn req(name: &str, tables: &[&str]) -> SplitRequest {
        SplitRequest {
            name: name.into(),
            description: None,
            tables: tables.iter().map(|t| t.to_string()).collect(),
        }
    }

    #[test]
    fn moves_tables_mappings_and_gets_its_own_identity() {
        let orig = profile();
        let (new, trimmed, moved) = plan_split(
            &orig,
            &[],
            &req("CollabX Groups", &["PUBLIC.groups", "public.group_users"]),
        )
        .unwrap();
        assert_eq!(moved, vec!["public.groups", "public.group_users"]);
        assert_eq!(new.source.tables, moved);
        assert_eq!(new.flow.mappings.len(), 2);
        assert_eq!(new.status, ProfileStatus::Draft);
        assert!(!new.autostart);
        assert_ne!(new.id, orig.id);
        assert!(new.connector_name.is_none());
        assert_eq!(connector_name_for(&new), "cdc-collabx-groups");
        assert_eq!(topic_prefix_for(&new), "collabx_groups");
        assert!(
            !new.source.connection.contains_key("slot"),
            "slot must not be shared"
        );
        // Original keeps the rest and its own identity.
        assert_eq!(trimmed.source.tables, vec!["public.archived_messages"]);
        assert_eq!(trimmed.flow.mappings.len(), 1);
        assert_eq!(connector_name_for(&trimmed), "cdc-draft---postgres");
        assert_eq!(trimmed.history.last().unwrap().event, "profile_split");
    }

    #[test]
    fn rejects_bad_requests() {
        let orig = profile();
        let err = |r: SplitRequest, others: &[CdcProfile]| {
            plan_split(&orig, others, &r).unwrap_err().to_string()
        };
        assert!(err(req("", &["public.groups"]), &[]).contains("name"));
        assert!(err(req("X", &[]), &[]).contains("at least one"));
        assert!(err(req("X", &["public.nope"]), &[]).contains("public.nope"));
        assert!(err(
            req(
                "X",
                &[
                    "public.archived_messages",
                    "public.groups",
                    "public.group_users"
                ]
            ),
            &[]
        )
        .contains("leave at least one"));
        // Same name, or a name that maps to the same topic prefix as another pipeline.
        assert!(err(req("collabx sync", &["public.groups"]), &[]).contains("already exists"));
        let mut other = profile();
        other.name = "CollabX-Groups".into();
        other.connector_name = None;
        assert!(err(req("CollabX Groups", &["public.groups"]), &[other]).contains("already used"));
        let mut trig = profile();
        trig.source.capture_mode = CaptureMode::Trigger;
        assert!(plan_split(&trig, &[], &req("X", &["public.groups"])).is_err());
    }
}
