//! Database version of each pipeline's source and target (ADR 0019 §8).
//!
//! The version comes from JDBC metadata (`jdbc-admin /v1/test`, any login can read it) and is
//! kept per pipeline in rqlite (kind `db_version`, separate from the profile so a refresh can
//! never overwrite a concurrent edit or lifecycle change). It is refreshed:
//!
//! * in the background every `ORCH_DB_VERSION_REFRESH_SECS` (default 3600, min 60);
//! * right after a pipeline is created or saved;
//! * on every Test Connection of a saved pipeline (free: the test already reads it);
//! * on demand (`POST /cdc/profiles/{id}/db-versions/refresh`).
//!
//! A different version simply replaces the old one (`previous` + `changed_at` are kept for a
//! tooltip). An unreachable database keeps the last known version and records `error`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::warn;
use uuid::Uuid;

use crate::clients::{JdbcAdminClient, ServerInfo};
use crate::error::AppError;
use crate::models::CdcProfile;
use crate::AppState;
use hyper_cdc_orchestrator::persist::RqliteClient;

const KIND: &str = "db_version";
/// Engines jdbc-admin ships drivers for (others would only ever record an error).
const JDBC_TYPES: [&str; 7] = [
    "postgres",
    "postgresql",
    "mysql",
    "mariadb",
    "sqlserver",
    "oracle",
    "sqlite",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct VersionStamp {
    #[serde(default)]
    pub product: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub major: i64,
    #[serde(default)]
    pub minor: i64,
    /// Last time the database was asked (successfully or not).
    pub checked_at: Option<DateTime<Utc>>,
    /// When the version last changed (None = never seen a change).
    #[serde(default)]
    pub changed_at: Option<DateTime<Utc>>,
    /// The version before the last change.
    #[serde(default)]
    pub previous: Option<String>,
    /// Why the last check failed (the version shown is then the last known one).
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileVersions {
    pub profile_id: Uuid,
    #[serde(default)]
    pub source: Option<VersionStamp>,
    #[serde(default)]
    pub target: Option<VersionStamp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Source,
    Target,
}

/// New stamp from the previous one and the latest check.
pub fn next_stamp(
    prev: Option<&VersionStamp>,
    result: Result<&ServerInfo, &str>,
    now: DateTime<Utc>,
) -> VersionStamp {
    match result {
        Ok(info) => {
            let mut s = VersionStamp {
                product: info.product.clone(),
                version: info.version.clone(),
                major: info.major,
                minor: info.minor,
                checked_at: Some(now),
                changed_at: prev.and_then(|p| p.changed_at),
                previous: prev.and_then(|p| p.previous.clone()),
                error: None,
            };
            if let Some(p) = prev.filter(|p| !p.version.is_empty()) {
                if p.version != info.version || p.product != info.product {
                    s.previous = Some(format!("{} {}", p.product, p.version).trim().to_string());
                    s.changed_at = Some(now);
                }
            }
            s
        }
        Err(e) => {
            let mut s = prev.cloned().unwrap_or_default();
            s.checked_at = Some(now);
            s.error = Some(e.lines().next().unwrap_or(e).chars().take(300).collect());
            s
        }
    }
}

// ── store ───────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct DbVersionStore {
    memory: Arc<RwLock<HashMap<Uuid, ProfileVersions>>>,
    rqlite: Option<RqliteClient>,
}

impl DbVersionStore {
    pub fn new(rqlite: Option<RqliteClient>) -> Self {
        Self {
            memory: Arc::new(RwLock::new(HashMap::new())),
            rqlite,
        }
    }

    pub async fn all(&self) -> HashMap<Uuid, ProfileVersions> {
        if let Some(rq) = &self.rqlite {
            match rq.list_json::<ProfileVersions>(KIND).await {
                Ok(list) => {
                    let map: HashMap<Uuid, ProfileVersions> =
                        list.into_iter().map(|v| (v.profile_id, v)).collect();
                    *self.memory.write().await = map.clone();
                    return map;
                }
                Err(e) => warn!(error = %e, "rqlite list db versions failed; using memory"),
            }
        }
        self.memory.read().await.clone()
    }

    pub async fn get(&self, id: Uuid) -> Option<ProfileVersions> {
        if let Some(rq) = &self.rqlite {
            if let Ok(found) = rq.get_json::<ProfileVersions>(KIND, &id.to_string()).await {
                return found;
            }
        }
        self.memory.read().await.get(&id).cloned()
    }

    async fn put(&self, v: &ProfileVersions) {
        if let Some(rq) = &self.rqlite {
            if let Err(e) = rq.put_json(KIND, &v.profile_id.to_string(), v).await {
                warn!(error = %e, "rqlite put db version failed");
            }
        }
        self.memory.write().await.insert(v.profile_id, v.clone());
    }

    pub async fn delete(&self, id: Uuid) {
        if let Some(rq) = &self.rqlite {
            let _ = rq.delete(KIND, &id.to_string()).await;
        }
        self.memory.write().await.remove(&id);
    }

    /// Record one check for one side.
    pub async fn record(
        &self,
        id: Uuid,
        side: Side,
        result: Result<&ServerInfo, &str>,
    ) -> ProfileVersions {
        let mut v = self.get(id).await.unwrap_or(ProfileVersions {
            profile_id: id,
            ..Default::default()
        });
        let now = Utc::now();
        match side {
            Side::Source => v.source = Some(next_stamp(v.source.as_ref(), result, now)),
            Side::Target => v.target = Some(next_stamp(v.target.as_ref(), result, now)),
        }
        self.put(&v).await;
        v
    }
}

// ── refresh ─────────────────────────────────────────────────────────────────────────────

fn jdbc_type(t: &str) -> bool {
    JDBC_TYPES.contains(&t.trim().to_ascii_lowercase().as_str())
}

async fn check(
    admin: &JdbcAdminClient,
    db_type: &str,
    conn: &HashMap<String, String>,
) -> Result<ServerInfo, String> {
    match admin.test_info(db_type, conn).await {
        Ok((_, Some(info))) => Ok(info),
        Ok((_, None)) => {
            Err("cdc-jdbc-admin did not report a version (rebuild cdc-jdbc-admin)".into())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Ask the source and (database) target of one pipeline for their version.
pub async fn refresh_profile(state: &AppState, profile: &CdcProfile) -> Option<ProfileVersions> {
    let admin = JdbcAdminClient::from_env()?;
    let store = &state.db_versions;
    let mut out = None;
    if jdbc_type(&profile.source.source_type) {
        let r = check(
            &admin,
            &profile.source.source_type,
            &profile.source.connection,
        )
        .await;
        out = Some(
            store
                .record(profile.id, Side::Source, r.as_ref().map_err(String::as_str))
                .await,
        );
    }
    if let Some(t) = profile
        .target
        .as_ref()
        .filter(|t| jdbc_type(&t.target_type))
    {
        let r = check(&admin, &t.target_type, &t.connection).await;
        out = Some(
            store
                .record(profile.id, Side::Target, r.as_ref().map_err(String::as_str))
                .await,
        );
    }
    out
}

/// Fire-and-forget refresh after a save.
pub fn refresh_soon(state: &AppState, profile: &CdcProfile) {
    let state = state.clone();
    let profile = profile.clone();
    tokio::spawn(async move {
        refresh_profile(&state, &profile).await;
    });
}

pub fn spawn_refresher(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if JdbcAdminClient::from_env().is_none() {
            return;
        }
        let every = std::env::var("ORCH_DB_VERSION_REFRESH_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|v| *v >= 60)
            .unwrap_or(3600);
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            match state.profiles.list().await {
                Ok(profiles) => {
                    for p in &profiles {
                        refresh_profile(&state, p).await;
                    }
                }
                Err(e) => warn!(error = %e, "db version refresh: list profiles failed"),
            }
            tokio::time::sleep(Duration::from_secs(every)).await;
        }
    })
}

// ── HTTP ────────────────────────────────────────────────────────────────────────────────

/// `GET /cdc/profiles/db-versions` → `{versions: {<profile_id>: {source, target}}}`.
pub async fn list_versions(State(state): State<AppState>) -> Json<Value> {
    let all = state.db_versions.all().await;
    let map: serde_json::Map<String, Value> = all
        .into_iter()
        .map(|(id, v)| {
            (
                id.to_string(),
                json!({"source": v.source, "target": v.target}),
            )
        })
        .collect();
    Json(json!({ "versions": map }))
}

/// `POST /cdc/profiles/{id}/db-versions/refresh` — ask both databases now.
pub async fn refresh_versions(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let profile = state
        .profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("profile not found: {id}")))?;
    if JdbcAdminClient::from_env().is_none() {
        return Err(AppError::BadRequest(
            "database versions need cdc-jdbc-admin (ORCH_JDBC_ADMIN_URL)".into(),
        ));
    }
    let v = refresh_profile(&state, &profile)
        .await
        .unwrap_or(ProfileVersions {
            profile_id: id,
            ..Default::default()
        });
    Ok(Json(json!({"source": v.source, "target": v.target})))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(v: &str) -> ServerInfo {
        ServerInfo {
            product: "PostgreSQL".into(),
            version: v.into(),
            major: 17,
            minor: 6,
            driver: String::new(),
        }
    }

    #[test]
    fn first_check_has_no_change() {
        let now = Utc::now();
        let s = next_stamp(None, Ok(&info("17.6")), now);
        assert_eq!(s.version, "17.6");
        assert_eq!(s.checked_at, Some(now));
        assert!(s.changed_at.is_none() && s.previous.is_none() && s.error.is_none());
    }

    #[test]
    fn upgrade_replaces_version_and_remembers_previous() {
        let t0 = Utc::now();
        let a = next_stamp(None, Ok(&info("17.6")), t0);
        let t1 = t0 + chrono::Duration::hours(1);
        let same = next_stamp(Some(&a), Ok(&info("17.6")), t1);
        assert!(same.changed_at.is_none(), "same version is not a change");
        let t2 = t1 + chrono::Duration::hours(1);
        let up = next_stamp(Some(&same), Ok(&info("17.7")), t2);
        assert_eq!(up.version, "17.7");
        assert_eq!(up.previous.as_deref(), Some("PostgreSQL 17.6"));
        assert_eq!(up.changed_at, Some(t2));
        let t3 = t2 + chrono::Duration::hours(1);
        let later = next_stamp(Some(&up), Ok(&info("17.7")), t3);
        assert_eq!(
            later.changed_at,
            Some(t2),
            "the change date stays until the next change"
        );
    }

    #[test]
    fn failure_keeps_last_known_version() {
        let t0 = Utc::now();
        let a = next_stamp(None, Ok(&info("17.6")), t0);
        let down = next_stamp(
            Some(&a),
            Err("Connection refused\nmore"),
            t0 + chrono::Duration::hours(1),
        );
        assert_eq!(down.version, "17.6");
        assert_eq!(down.error.as_deref(), Some("Connection refused"));
        let back = next_stamp(
            Some(&down),
            Ok(&info("17.6")),
            t0 + chrono::Duration::hours(2),
        );
        assert!(back.error.is_none());
    }

    #[test]
    fn only_jdbc_engines_are_checked() {
        assert!(jdbc_type("PostgreSQL"));
        assert!(jdbc_type("oracle"));
        assert!(!jdbc_type("db2"));
        assert!(!jdbc_type("s3"));
    }
}
