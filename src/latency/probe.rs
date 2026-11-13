//! Heartbeat probe (ADR 0016): exact end-to-end latency for one pipeline.
//!
//! The orchestrator writes a sequence number into `hyper_cdc.hyper_cdc_heartbeat` on the
//! source, then polls the target until that number arrives. Latency = time first seen on
//! the target − time written, both on the orchestrator's clock (no DB clock skew), with
//! 250 ms resolution. The table is captured and replicated like any other table.
//!
//! Supported: Postgres source with log-based capture → Postgres target.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::AppError;
use crate::models::{CaptureMode, CdcProfile, ProfileStatus};
use crate::postgres_trigger_cdc::quote_ident;
use crate::AppState;

pub const SCHEMA: &str = "hyper_cdc";
pub const TABLE: &str = "hyper_cdc_heartbeat";
pub const QUALIFIED: &str = "hyper_cdc.hyper_cdc_heartbeat";
const POLL: Duration = Duration::from_millis(250);
/// Give up on one heartbeat after this long and write the next one.
const MAX_WAIT: Duration = Duration::from_secs(600);

#[derive(Debug, Default, Clone)]
pub struct ProbeState {
    pub last_e2e_ms: Option<i64>,
    pub observed_at: Option<DateTime<Utc>>,
    /// A heartbeat written and not yet seen on the target.
    pub pending_since: Option<(Instant, DateTime<Utc>)>,
    pub error: Option<String>,
}

impl ProbeState {
    /// Current end-to-end value: the last measurement, or the age of an outstanding
    /// heartbeat when that is already longer (the pipeline is falling behind right now).
    pub fn current_ms(&self) -> Option<i64> {
        let pending = self
            .pending_since
            .map(|(at, _)| at.elapsed().as_millis() as i64);
        match (self.last_e2e_ms, pending) {
            (Some(l), Some(p)) => Some(l.max(p)),
            (Some(l), None) => Some(l),
            (None, Some(p)) if p > 5_000 => Some(p),
            _ => None,
        }
    }
}

fn is_pg(t: &str) -> bool {
    matches!(
        t.trim().to_ascii_lowercase().as_str(),
        "postgres" | "postgresql"
    )
}

/// Why the probe cannot run for this pipeline, if it cannot.
pub fn unsupported_reason(p: &CdcProfile) -> Option<String> {
    if !p.has_managed_sink() {
        return Some("the heartbeat probe needs a source → target pipeline".into());
    }
    if !is_pg(&p.source.source_type) || p.source.capture_mode != CaptureMode::Logical {
        return Some(
            "the heartbeat probe currently supports PostgreSQL sources with log-based capture"
                .into(),
        );
    }
    match p.target.as_ref() {
        Some(t) if is_pg(&t.target_type) => None,
        _ => Some("the heartbeat probe currently supports PostgreSQL targets".into()),
    }
}

async fn source_client(p: &CdcProfile) -> Result<tokio_postgres::Client, AppError> {
    let insight = crate::connector_catalog::insight_from_connection(
        &p.source.source_type,
        &p.source.connection,
    );
    crate::connector_catalog::postgres_client(&insight).await
}

async fn target_client(p: &CdcProfile) -> Result<tokio_postgres::Client, AppError> {
    let t = p
        .target
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("pipeline has no target".into()))?;
    let insight = crate::connector_catalog::insight_from_connection(&t.target_type, &t.connection);
    crate::connector_catalog::postgres_client(&insight).await
}

/// Creates the heartbeat table on the source and adds it to the publication when the
/// publication lists tables. Returns notes for the operator.
pub async fn setup_source(p: &CdcProfile) -> Result<Vec<String>, AppError> {
    if let Some(why) = unsupported_reason(p) {
        return Err(AppError::BadRequest(why));
    }
    let client = source_client(p).await?;
    client
        .batch_execute(&format!(
            "CREATE SCHEMA IF NOT EXISTS {SCHEMA};\n\
             CREATE TABLE IF NOT EXISTS {QUALIFIED} (\n\
               pipeline_id text PRIMARY KEY,\n\
               seq bigint NOT NULL,\n\
               ts_ms bigint NOT NULL\n\
             );"
        ))
        .await
        .map_err(|e| {
            AppError::BadRequest(format!(
                "could not create {QUALIFIED} on the source (needs CREATE on the database): {}",
                crate::connector_catalog::postgres_ddl_detail(&e)
            ))
        })?;
    let publication = p
        .source
        .connection
        .get("publication")
        .cloned()
        .unwrap_or_else(|| "dbz_publication".into());
    let mut notes = Vec::new();
    let row = client
        .query_opt(
            "SELECT puballtables FROM pg_publication WHERE pubname = $1",
            &[&publication],
        )
        .await
        .map_err(|e| AppError::Upstream(format!("reading publication {publication}: {e}")))?;
    match row {
        None => notes.push(format!(
            "publication {publication} does not exist yet; include {QUALIFIED} when you create it"
        )),
        Some(r) if r.get::<_, bool>(0) => {
            notes.push(format!("publication {publication} covers all tables"))
        }
        Some(_) => {
            let present = client
                .query_opt(
                    "SELECT 1 FROM pg_publication_tables WHERE pubname = $1 AND schemaname = $2 AND tablename = $3",
                    &[&publication, &SCHEMA, &TABLE],
                )
                .await
                .map_err(|e| AppError::Upstream(e.to_string()))?;
            if present.is_none() {
                client
                    .batch_execute(&format!("ALTER PUBLICATION {} ADD TABLE {QUALIFIED}", quote_ident(&publication)?))
                    .await
                    .map_err(|e| {
                        AppError::BadRequest(format!(
                            "could not add {QUALIFIED} to publication {publication} (needs publication owner): {}",
                            crate::connector_catalog::postgres_ddl_detail(&e)
                        ))
                    })?;
                notes.push(format!("added {QUALIFIED} to publication {publication}"));
            }
        }
    }
    Ok(notes)
}

/// Removes this pipeline's heartbeat row on the source (best effort; the table may be
/// shared with other pipelines and stays).
pub async fn remove_row(p: &CdcProfile) {
    if let Ok(client) = source_client(p).await {
        let _ = client
            .execute(
                &format!("DELETE FROM {QUALIFIED} WHERE pipeline_id = $1"),
                &[&p.id.to_string()],
            )
            .await;
    }
}

/// One probe loop for one pipeline; ends when the probe is disabled or the pipeline stops.
pub fn spawn(
    state: AppState,
    profile_id: Uuid,
    shared: Arc<Mutex<ProbeState>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut source: Option<tokio_postgres::Client> = None;
        let mut target: Option<tokio_postgres::Client> = None;
        loop {
            let Ok(Some(p)) = state.profiles.get(profile_id).await else {
                break;
            };
            if !p.latency_probe.enabled
                || p.status != ProfileStatus::Running
                || unsupported_reason(&p).is_some()
            {
                break;
            }
            let interval =
                Duration::from_secs(u64::from(p.latency_probe.interval_secs.clamp(5, 300)));
            let started = Instant::now();
            if let Err(e) = beat(&p, &mut source, &mut target, &shared).await {
                source = None;
                target = None;
                let mut s = shared.lock().await;
                s.error = Some(e);
                s.pending_since = None;
            }
            tokio::time::sleep(interval.saturating_sub(started.elapsed())).await;
        }
        shared.lock().await.pending_since = None;
    })
}

async fn beat(
    p: &CdcProfile,
    source: &mut Option<tokio_postgres::Client>,
    target: &mut Option<tokio_postgres::Client>,
    shared: &Arc<Mutex<ProbeState>>,
) -> Result<(), String> {
    if source.is_none() {
        *source = Some(source_client(p).await.map_err(|e| format!("source: {e}"))?);
    }
    if target.is_none() {
        *target = Some(target_client(p).await.map_err(|e| format!("target: {e}"))?);
    }
    let (src, tgt) = (source.as_ref().unwrap(), target.as_ref().unwrap());
    let id = p.id.to_string();
    let seq = Utc::now().timestamp_millis();
    src.execute(
        &format!(
            "INSERT INTO {QUALIFIED}(pipeline_id, seq, ts_ms) VALUES ($1, $2, $2) \
             ON CONFLICT (pipeline_id) DO UPDATE SET seq = EXCLUDED.seq, ts_ms = EXCLUDED.ts_ms"
        ),
        &[&id, &seq],
    )
    .await
    .map_err(|e| format!("writing heartbeat on the source: {e}"))?;
    let written = Instant::now();
    shared.lock().await.pending_since = Some((written, Utc::now()));

    let schema = quote_ident(crate::reconcile::jdbc_target_schema(p)).map_err(|e| e.to_string())?;
    let sql = format!("SELECT seq FROM {schema}.\"{TABLE}\" WHERE pipeline_id = $1");
    while written.elapsed() < MAX_WAIT {
        match tgt.query_opt(&sql, &[&id]).await {
            Ok(Some(row)) if row.get::<_, i64>(0) >= seq => {
                let mut s = shared.lock().await;
                s.last_e2e_ms = Some(written.elapsed().as_millis() as i64);
                s.observed_at = Some(Utc::now());
                s.pending_since = None;
                s.error = None;
                return Ok(());
            }
            Ok(_) => {}
            // The sink creates the table with the first heartbeat; until then it is missing.
            Err(e) if e.code() == Some(&tokio_postgres::error::SqlState::UNDEFINED_TABLE) => {}
            Err(e) => return Err(format!("reading heartbeat on the target: {e}")),
        }
        tokio::time::sleep(POLL).await;
    }
    let mut s = shared.lock().await;
    s.pending_since = None;
    s.error = Some(format!(
        "heartbeat not seen on the target after {} s",
        MAX_WAIT.as_secs()
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_value_grows_while_a_heartbeat_is_outstanding() {
        let mut s = ProbeState {
            last_e2e_ms: Some(800),
            ..Default::default()
        };
        assert_eq!(s.current_ms(), Some(800));
        s.pending_since = Some((Instant::now() - Duration::from_secs(30), Utc::now()));
        assert!(s.current_ms().unwrap() >= 30_000);
        let fresh = ProbeState {
            pending_since: Some((Instant::now(), Utc::now())),
            ..Default::default()
        };
        assert_eq!(
            fresh.current_ms(),
            None,
            "no value until something is known"
        );
    }
}
