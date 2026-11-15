//! The pipeline's own PostgreSQL replication slot: inspect it, and drop it when the pipeline is
//! deleted (B-02).
//!
//! An unused logical slot keeps every WAL segment from its position onwards, so a forgotten slot
//! fills the production source's disk until PostgreSQL stops. Deleting a pipeline therefore drops
//! its slot unless the caller explicitly keeps it (slot-first rollouts that will reuse it).
//!
//! Only the slot this pipeline uses (`reconcile::postgres_slot_name`) is ever touched, and only
//! for log-based capture on a PostgreSQL source.

use std::time::Duration;

use serde_json::{json, Value};

use crate::error::AppError;
use crate::models::{CaptureMode, CdcProfile};

/// Does this pipeline own a replication slot on its source?
pub fn applies(profile: &CdcProfile) -> bool {
    profile.source.capture_mode == CaptureMode::Logical
        && matches!(
            profile
                .source
                .source_type
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "postgres" | "postgresql"
        )
}

async fn client(profile: &CdcProfile) -> Result<tokio_postgres::Client, AppError> {
    let insight =
        crate::connector_catalog::insight_from_connection("postgres", &profile.source.connection);
    crate::connector_catalog::postgres_client(&insight).await
}

struct SlotRow {
    active: bool,
    active_pid: Option<i32>,
    retained_bytes: Option<i64>,
    wal_status: Option<String>,
}

async fn read(client: &tokio_postgres::Client, slot: &str) -> Result<Option<SlotRow>, AppError> {
    let row = client
        .query_opt(
            // wal_status exists from PG 13; to_jsonb keeps the query valid on 10-12.
            "SELECT s.active, s.active_pid, \
                    pg_wal_lsn_diff(pg_current_wal_lsn(), s.restart_lsn)::bigint, \
                    to_jsonb(s) ->> 'wal_status' \
             FROM pg_replication_slots s WHERE s.slot_name = $1",
            &[&slot],
        )
        .await
        .map_err(|e| {
            AppError::Upstream(format!("could not read replication slot '{slot}': {e}"))
        })?;
    Ok(row.map(|r| SlotRow {
        active: r.get(0),
        active_pid: r.get(1),
        retained_bytes: r.get(2),
        wal_status: r.get(3),
    }))
}

pub fn pretty_bytes(b: i64) -> String {
    let b = b.max(0) as f64;
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} B", b as i64)
    } else {
        format!("{v:.1} {}", U[i])
    }
}

/// `GET /cdc/profiles/{id}/slot`: what deleting the pipeline would drop. Never fails the request
/// for an unreachable source: `error` says why the state is unknown.
pub async fn describe(profile: &CdcProfile) -> Value {
    if !applies(profile) {
        return json!({"applies": false});
    }
    let slot = crate::reconcile::postgres_slot_name(profile);
    let c = match client(profile).await {
        Ok(c) => c,
        Err(e) => {
            return json!({"applies": true, "slot_name": slot, "exists": null, "error": e.to_string()})
        }
    };
    match read(&c, &slot).await {
        Ok(Some(r)) => json!({
            "applies": true,
            "slot_name": slot,
            "exists": true,
            "active": r.active,
            "wal_status": r.wal_status,
            "retained_bytes": r.retained_bytes,
            "retained": r.retained_bytes.map(pretty_bytes),
        }),
        Ok(None) => json!({"applies": true, "slot_name": slot, "exists": false}),
        Err(e) => {
            json!({"applies": true, "slot_name": slot, "exists": null, "error": e.to_string()})
        }
    }
}

/// Drops the pipeline's slot after its connectors were deleted. Waits up to `wait` for the
/// slot to become inactive (the connector's WAL sender exits); a sender that still holds it after
/// that is ended with pg_terminate_backend, since the slot belongs to this pipeline only.
///
/// Returns the outcome for the report; `Err` means the slot is still there.
pub async fn drop_slot(profile: &CdcProfile, wait: Duration) -> Result<Value, AppError> {
    let slot = crate::reconcile::postgres_slot_name(profile);
    let fix = format!("SELECT pg_drop_replication_slot('{slot}');");
    let c = client(profile).await.map_err(|e| {
        AppError::Conflict(format!(
            "could not reach the source to drop replication slot '{slot}': {e}. Fix the connection and retry, \
             or delete with \"keep the slot\" and run on the source: {fix}"
        ))
    })?;
    let mut deadline = tokio::time::Instant::now() + wait;
    let mut terminated = false;
    loop {
        let Some(row) = read(&c, &slot).await? else {
            return Ok(
                json!({"slot_name": slot, "result": if terminated { "dropped" } else { "absent" }}),
            );
        };
        let retained = row.retained_bytes;
        if !row.active {
            c.execute("SELECT pg_drop_replication_slot($1)", &[&slot])
                .await
                .map_err(|e| {
                    AppError::Conflict(format!(
                        "could not drop replication slot '{slot}': {e}. Retry, or run on the source: {fix}"
                    ))
                })?;
            return Ok(json!({
                "slot_name": slot,
                "result": "dropped",
                "freed": retained.map(pretty_bytes),
                "terminated_sender": terminated,
            }));
        }
        if tokio::time::Instant::now() >= deadline {
            if terminated {
                return Err(AppError::Conflict(format!(
                    "replication slot '{slot}' is still in use after the connectors were deleted. \
                     Check what reads it (pg_stat_replication), then run on the source: {fix}"
                )));
            }
            if let Some(pid) = row.active_pid {
                let _ = c.execute("SELECT pg_terminate_backend($1)", &[&pid]).await;
            }
            terminated = true;
            deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_are_readable() {
        assert_eq!(pretty_bytes(0), "0 B");
        assert_eq!(pretty_bytes(1536), "1.5 KB");
        assert_eq!(pretty_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
}
