//! Restart connector tasks that failed for a temporary reason.
//!
//! Kafka Connect never retries a FAILED task: a source behind an SSH tunnel that drops
//! for a minute leaves the pipeline dead ("will not recover until manually restarted")
//! even after the database is reachable again. For connectors owned by a Running
//! pipeline whose failure is diagnosed as connectivity (network, Kafka, Kafka Connect),
//! the watcher restarts only the failed tasks with exponential backoff and records
//! every attempt in the audit trail. Configuration / data errors (missing publication,
//! wrong password, bad mapping) are left alone — retrying cannot fix them.
//!
//! The backoff only spaces out retries while the database is still down: between
//! attempts the watcher checks the database port (every 10 s) and retries as soon as it
//! answers, so a fixed tunnel is picked up within seconds, not after the next backoff.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::Method;

use crate::diagnose::{diagnose, ErrorArea, Side};
use crate::models::{CdcProfile, ConnectorSnapshot, ProfileStatus};
use crate::reconcile::{connect_request, connector_name_for, sink_connector_name_for};
use crate::AppState;

const FIRST_DELAY: Duration = Duration::from_secs(30);
const MAX_DELAY: Duration = Duration::from_secs(600);
/// A failure this long after the previous attempt starts a fresh backoff sequence.
const RESET_AFTER: Duration = Duration::from_secs(1800);
/// How often to check whether the failed endpoint is reachable again while waiting.
const PROBE_EVERY: Duration = Duration::from_secs(10);

struct Attempt {
    count: u32,
    last: Instant,
    next: Instant,
    last_probe: Option<Instant>,
}

static ATTEMPTS: Mutex<Option<HashMap<String, Attempt>>> = Mutex::new(None);

/// Delay before attempt `n` (0-based): 30 s, 1 m, 2 m, 4 m, 8 m, then 10 m.
fn backoff(n: u32) -> Duration {
    FIRST_DELAY
        .checked_mul(1u32 << n.min(10))
        .unwrap_or(MAX_DELAY)
        .min(MAX_DELAY)
}

/// Record an attempt now and schedule the next one.
fn bump(entry: &mut Attempt, now: Instant) -> u32 {
    entry.count += 1;
    entry.last = now;
    entry.next = now + backoff(entry.count);
    entry.count
}

/// The database this connector talks to answers on its port again.
async fn endpoint_reachable(profile: &CdcProfile, side: Side) -> bool {
    let (db_type, conn) = match side {
        Side::Target => match profile.target.as_ref() {
            Some(t) => (t.target_type.as_str(), &t.connection),
            None => return false,
        },
        _ => (
            profile.source.source_type.as_str(),
            &profile.source.connection,
        ),
    };
    // `None` = reachable (or not checkable — then wait for the normal backoff instead).
    conn.get("host").is_some_and(|h| !h.trim().is_empty())
        && crate::profile_table_status::probe_endpoint(db_type, conn)
            .await
            .is_none()
}

/// The failure is worth retrying: the component was unreachable, not misconfigured.
fn is_transient(trace: &str, side: Side) -> Option<String> {
    let d = diagnose(trace, side)?;
    matches!(
        d.area,
        ErrorArea::Network | ErrorArea::Kafka | ErrorArea::KafkaConnect
    )
    .then_some(d.title)
}

/// The Running profile that owns connector `name`, with the side it serves.
fn owner<'a>(profiles: &'a [CdcProfile], name: &str) -> Option<(&'a CdcProfile, Side)> {
    profiles
        .iter()
        .filter(|p| p.status == ProfileStatus::Running && p.enabled)
        .find_map(|p| {
            let primary = connector_name_for(p);
            if name == primary
                || name.starts_with(&crate::incremental_cdc::poll_connector_prefix(&primary))
            {
                Some((p, Side::Source))
            } else if p.has_managed_sink() && name == sink_connector_name_for(p) {
                Some((p, Side::Target))
            } else {
                None
            }
        })
}

async fn recovery_attempt_number(
    snap: &ConnectorSnapshot,
    profile: &crate::models::CdcProfile,
    side: Side,
    now: Instant,
) -> Option<u32> {
    let attempt_no = {
        let mut guard = ATTEMPTS.lock().unwrap();
        let map = guard.get_or_insert_with(HashMap::new);
        let entry = map.entry(snap.name.clone()).or_insert(Attempt {
            count: 0,
            last: now,
            next: now + FIRST_DELAY,
            last_probe: None,
        });
        if now.duration_since(entry.last) > RESET_AFTER && entry.count > 0 {
            entry.count = 0;
            entry.next = now + FIRST_DELAY;
        }
        if now < entry.next {
            let probe_due = entry
                .last_probe
                .is_none_or(|t| now.duration_since(t) >= PROBE_EVERY);
            if !probe_due {
                return None;
            }
            entry.last_probe = Some(now);
            None
        } else {
            Some(bump(entry, now))
        }
    };
    match attempt_no {
        Some(n) => Some(n),
        None if !endpoint_reachable(profile, side).await => None,
        None => {
            let mut guard = ATTEMPTS.lock().unwrap();
            guard
                .as_mut()
                .and_then(|m| m.get_mut(&snap.name))
                .map(|entry| bump(entry, Instant::now()))
        }
    }
}

pub(super) async fn recover(state: &AppState, snapshots: &HashMap<String, ConnectorSnapshot>) {
    let failed: Vec<&ConnectorSnapshot> = snapshots
        .values()
        .filter(|s| s.state == "FAILED" && !s.task_errors.is_empty())
        .collect();
    if failed.is_empty() {
        return;
    }
    let Ok(profiles) = state.profiles.list().await else {
        return;
    };

    for snap in failed {
        let Some((profile, side)) = owner(&profiles, &snap.name) else {
            continue;
        };
        let Some(reason) = snap
            .task_errors
            .iter()
            .find_map(|e| is_transient(&e.trace, side))
        else {
            continue;
        };

        let now = Instant::now();
        let Some(attempt_no) = recovery_attempt_number(snap, profile, side, now).await else {
            continue;
        };

        let path = format!(
            "/connectors/{}/restart?includeTasks=true&onlyFailed=true",
            urlencoding::encode(&snap.name)
        );
        let result = connect_request(state, Method::POST, &path, None).await;
        let next_in = backoff(attempt_no).as_secs();
        tracing::info!(
            target = "cdc_watcher",
            connector = %snap.name,
            attempt = attempt_no,
            ok = result.is_ok(),
            "auto-restart of failed tasks"
        );
        crate::audit::emit_system(
            state,
            "connector.auto_restart",
            if result.is_ok() { "success" } else { "failure" },
            serde_json::json!({
                "connector": snap.name,
                "side": if side == Side::Target { "target" } else { "source" },
                "reason": reason,
                "attempt": attempt_no,
                "next_retry_in_seconds": next_in,
                "error": result.err().map(|e| e.to_string()),
            }),
            Some(profile.id),
            Some(profile.name.as_str()),
        )
        .await;
    }

    // Forget connectors that are healthy again or gone, so a later failure starts at 30 s.
    let mut guard = ATTEMPTS.lock().unwrap();
    if let Some(map) = guard.as_mut() {
        map.retain(|name, a| {
            snapshots.get(name).is_some_and(|s| s.state == "FAILED")
                || Instant::now().duration_since(a.last) < RESET_AFTER
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(backoff(0), Duration::from_secs(30));
        assert_eq!(backoff(1), Duration::from_secs(60));
        assert_eq!(backoff(4), Duration::from_secs(480));
        assert_eq!(backoff(5), MAX_DELAY);
        assert_eq!(backoff(30), MAX_DELAY);
    }

    #[test]
    fn only_connectivity_failures_are_retried() {
        let refused = "io.debezium.DebeziumException: Couldn't obtain encoding for database collabx\nCaused by: java.net.ConnectException: Connection refused";
        assert!(is_transient(refused, Side::Source).is_some());
        let no_pub = "ConnectException: Publication autocreation is disabled, please create one";
        assert!(is_transient(no_pub, Side::Source).is_none());
        let bad_pw = "FATAL: password authentication failed for user \"collabx\"";
        assert!(is_transient(bad_pw, Side::Source).is_none());
    }
}
