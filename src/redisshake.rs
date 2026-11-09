//! HTTP client for the cdc-redisshake sidecar.

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::{RedisShakeProfile, RedisShakeSyncOptions, SchemaCompareConn};
use crate::tool_sidecar::SidecarAccess;
use reqwest::Client;
use serde_json::{json, Value};

pub fn is_redis(db_type: &str) -> bool {
    matches!(
        db_type.trim().to_ascii_lowercase().as_str(),
        "redis" | "valkey" | "tair"
    )
}

pub fn require_redis_pair(
    source: &SchemaCompareConn,
    target: &SchemaCompareConn,
) -> Result<(), AppError> {
    if !is_redis(&source.db_type) {
        return Err(AppError::BadRequest(
            "RedisShake source must be Redis (or Valkey/Tair)".into(),
        ));
    }
    if !is_redis(&target.db_type) {
        return Err(AppError::BadRequest(
            "RedisShake target must be Redis (or Valkey/Tair)".into(),
        ));
    }
    Ok(())
}

/// Highest database number accepted. Redis defaults to 16 (`databases` in redis.conf), but the
/// setting can be raised; anything above this is almost certainly a typo.
pub const MAX_DB: u32 = 9999;

/// Normalises the database choice (sorted, no duplicates) and rejects combinations RedisShake
/// cannot honour. Redis Cluster has only database 0.
pub fn normalize_db_options(o: &mut RedisShakeSyncOptions) -> Result<(), AppError> {
    o.source_dbs.sort_unstable();
    o.source_dbs.dedup();
    if let Some(db) = o
        .source_dbs
        .iter()
        .chain(o.target_db.iter())
        .find(|d| **d > MAX_DB)
    {
        return Err(AppError::BadRequest(format!(
            "Database {db} is out of range (0-{MAX_DB})"
        )));
    }
    if o.source_cluster && o.source_dbs.iter().any(|d| *d != 0) {
        return Err(AppError::BadRequest(
            "A Redis Cluster source has only database 0".into(),
        ));
    }
    if let Some(t) = o.target_db {
        if o.source_dbs.len() != 1 {
            return Err(AppError::BadRequest(
                "A target database needs exactly one source database".into(),
            ));
        }
        if o.target_cluster && t != 0 {
            return Err(AppError::BadRequest(
                "A Redis Cluster target has only database 0".into(),
            ));
        }
        // Same number on both sides: nothing to remap.
        if o.source_dbs[0] == t {
            o.target_db = None;
        }
    }
    Ok(())
}

/// Applies a per-run start mode. `once` = full copy, then exit; `cdc` = follow source changes,
/// with (`snapshot` true / profile default) or without an initial full copy.
pub fn apply_run_mode(
    o: &mut RedisShakeSyncOptions,
    mode: Option<&str>,
    snapshot: Option<bool>,
    empty_target: Option<bool>,
) -> Result<(), AppError> {
    match mode.map(str::trim).filter(|m| !m.is_empty()) {
        None => {
            if snapshot.is_some() {
                return Err(AppError::BadRequest(
                    "snapshot is only used with mode=cdc".into(),
                ));
            }
        }
        Some("once") => {
            if snapshot == Some(false) {
                return Err(AppError::BadRequest(
                    "a one-time sync is a full copy; snapshot cannot be false".into(),
                ));
            }
            o.sync_rdb = true;
            o.sync_aof = false;
        }
        Some("cdc") => {
            o.sync_aof = true;
            if let Some(s) = snapshot {
                o.sync_rdb = s;
            }
        }
        Some(other) => {
            return Err(AppError::BadRequest(format!(
                "mode must be once or cdc, not {other}"
            )));
        }
    }
    if !o.sync_rdb && !o.sync_aof {
        return Err(AppError::BadRequest(
            "nothing to sync: turn on the full copy or live changes".into(),
        ));
    }
    if let Some(e) = empty_target {
        if e && !o.sync_rdb {
            return Err(AppError::BadRequest(
                "empty_target needs a full copy (one-time sync or CDC with snapshot); nothing would refill the target".into(),
            ));
        }
        o.empty_target = e;
    }
    Ok(())
}

/// Timeline name and detail of a run with these options.
pub fn run_mode_label(o: &RedisShakeSyncOptions) -> (&'static str, Value) {
    let empty = o.empty_target && o.sync_rdb;
    match (o.sync_aof, empty) {
        (true, false) => ("CDC", json!({ "mode": "cdc", "snapshot": o.sync_rdb })),
        (true, true) => (
            "CDC, target emptied first",
            json!({ "mode": "cdc", "snapshot": true, "empty_target": true }),
        ),
        (false, false) => ("One-time sync", json!({ "mode": "once", "snapshot": true })),
        (false, true) => (
            "One-time sync, target emptied first",
            json!({ "mode": "once", "snapshot": true, "empty_target": true }),
        ),
    }
}

pub fn profile_run_payload(profile: &RedisShakeProfile) -> Value {
    json!({
        // One job per run: the sidecar assigns the job id; this ties the run to its transfer.
        "profile_id": profile.id.to_string(),
        "name": profile.name,
        "source": profile.source,
        "target": profile.target,
        "options": profile.options,
    })
}

fn access<'a>(cfg: &'a AppConfig, client: &'a Client) -> Result<SidecarAccess<'a>, AppError> {
    Ok(SidecarAccess {
        client,
        base_url: cfg.redis_shake_url.as_deref().ok_or_else(|| {
            AppError::BadRequest(
                "RedisShake sidecar is not configured (ORCH_REDIS_SHAKE_URL)".into(),
            )
        })?,
        bearer_token: cfg.redis_shake_token.as_deref(),
        tool: "redisshake",
        token_env: "ORCH_REDIS_SHAKE_TOKEN",
        extra_bad_request: &[],
    })
}

crate::sidecar_v1_handlers!(access);

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(source_dbs: Vec<u32>, target_db: Option<u32>) -> RedisShakeSyncOptions {
        RedisShakeSyncOptions {
            source_dbs,
            target_db,
            ..Default::default()
        }
    }

    #[test]
    fn sorts_and_dedups_source_dbs() {
        let mut o = opts(vec![5, 0, 5, 2], None);
        normalize_db_options(&mut o).unwrap();
        assert_eq!(o.source_dbs, vec![0, 2, 5]);
    }

    #[test]
    fn target_db_needs_exactly_one_source_db() {
        assert!(normalize_db_options(&mut opts(vec![], Some(1))).is_err());
        assert!(normalize_db_options(&mut opts(vec![0, 2], Some(1))).is_err());
        let mut o = opts(vec![2], Some(0));
        normalize_db_options(&mut o).unwrap();
        assert_eq!(o.target_db, Some(0));
    }

    #[test]
    fn same_db_on_both_sides_drops_the_remap() {
        let mut o = opts(vec![3], Some(3));
        normalize_db_options(&mut o).unwrap();
        assert_eq!(o.target_db, None);
    }

    #[test]
    fn cluster_sides_only_have_db_zero() {
        let mut o = opts(vec![1], None);
        o.source_cluster = true;
        assert!(normalize_db_options(&mut o).is_err());
        let mut o = opts(vec![1], Some(2));
        o.target_cluster = true;
        assert!(normalize_db_options(&mut o).is_err());
        let mut o = opts(vec![0], None);
        o.source_cluster = true;
        assert!(normalize_db_options(&mut o).is_ok());
    }

    #[test]
    fn run_modes_override_the_profile_default_for_one_run() {
        let cdc_profile = || RedisShakeSyncOptions {
            sync_rdb: true,
            sync_aof: true,
            ..Default::default()
        };
        let mut o = cdc_profile();
        apply_run_mode(&mut o, Some("once"), None, None).unwrap();
        assert!(o.sync_rdb && !o.sync_aof);
        assert_eq!(run_mode_label(&o).0, "One-time sync");

        let mut o = cdc_profile();
        apply_run_mode(&mut o, Some("cdc"), Some(false), None).unwrap(); // changes only
        assert!(!o.sync_rdb && o.sync_aof);
        assert_eq!(
            run_mode_label(&o).1,
            json!({ "mode": "cdc", "snapshot": false })
        );

        let mut o = cdc_profile();
        apply_run_mode(&mut o, None, None, None).unwrap(); // profile default
        assert!(o.sync_rdb && o.sync_aof);

        assert!(apply_run_mode(&mut cdc_profile(), Some("once"), Some(false), None).is_err());
        assert!(apply_run_mode(&mut cdc_profile(), Some("weekly"), None, None).is_err());
        assert!(apply_run_mode(&mut cdc_profile(), None, Some(true), None).is_err());
    }

    #[test]
    fn empty_target_only_before_a_full_copy() {
        let cdc_profile = || RedisShakeSyncOptions {
            sync_rdb: true,
            sync_aof: true,
            ..Default::default()
        };
        let mut o = cdc_profile();
        apply_run_mode(&mut o, Some("once"), None, Some(true)).unwrap();
        assert!(o.empty_target);
        assert_eq!(run_mode_label(&o).0, "One-time sync, target emptied first");
        assert_eq!(run_mode_label(&o).1["empty_target"], json!(true));
        // CDC from now has no full copy: an explicit request is refused.
        assert!(apply_run_mode(&mut cdc_profile(), Some("cdc"), Some(false), Some(true)).is_err());
        // A profile default of "empty first" is not applied to a changes-only run.
        let mut o = RedisShakeSyncOptions {
            empty_target: true,
            ..cdc_profile()
        };
        apply_run_mode(&mut o, Some("cdc"), Some(false), None).unwrap();
        assert_eq!(run_mode_label(&o).0, "CDC");
        // One run can skip the profile's default.
        let mut o = RedisShakeSyncOptions {
            empty_target: true,
            ..cdc_profile()
        };
        apply_run_mode(&mut o, Some("once"), None, Some(false)).unwrap();
        assert!(!o.empty_target);
    }

    #[test]
    fn rejects_out_of_range_numbers() {
        assert!(normalize_db_options(&mut opts(vec![MAX_DB + 1], None)).is_err());
    }
}
