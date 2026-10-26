//! Profile store: rqlite when configured, else in-memory.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::error::AppError;
use crate::models::{
    CdcProfile, MongoCopyProfile, PgCopyProfile, PgLoaderProfile, RedisShakeProfile,
    SchemaCompareProfile,
};
use hyper_cdc_orchestrator::persist::{
    RqliteClient, KIND_COMPARE, KIND_MONGOCOPY, KIND_PGCOPY, KIND_PGLOADER, KIND_PROFILE,
    KIND_REDISSHAKE,
};

#[derive(Clone)]
pub struct ProfileStore {
    memory: Arc<RwLock<HashMap<Uuid, CdcProfile>>>,
    rqlite: Option<RqliteClient>,
    /// When true, persistence errors surface instead of silent memory fallback.
    strict_persistence: bool,
}

impl ProfileStore {
    pub fn new(rqlite: Option<RqliteClient>, strict_persistence: bool) -> Self {
        Self {
            memory: Arc::new(RwLock::new(HashMap::new())),
            rqlite,
            strict_persistence,
        }
    }

    pub fn persistence_enabled(&self) -> bool {
        self.rqlite.is_some()
    }

    pub async fn list(&self) -> Result<Vec<CdcProfile>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq.list_json::<CdcProfile>(KIND_PROFILE).await {
                Ok(profiles) => {
                    let mut map = self.memory.write().await;
                    map.clear();
                    for p in &profiles {
                        map.insert(p.id, p.clone());
                    }
                    return Ok(profiles);
                }
                Err(e) => {
                    if self.strict_persistence {
                        return Err(e.into());
                    }
                    tracing::warn!(error = %e, "rqlite list profiles failed; using memory");
                }
            }
        }
        Ok(self.memory.read().await.values().cloned().collect())
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<CdcProfile>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq
                .get_json::<CdcProfile>(KIND_PROFILE, &id.to_string())
                .await
            {
                Ok(Some(p)) => {
                    self.memory.write().await.insert(p.id, p.clone());
                    return Ok(Some(p));
                }
                Ok(None) => {}
                Err(e) => {
                    if self.strict_persistence {
                        return Err(e.into());
                    }
                    tracing::warn!(error = %e, "rqlite get profile failed");
                }
            }
        }
        Ok(self.memory.read().await.get(&id).cloned())
    }

    pub async fn upsert(&self, profile: &CdcProfile) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.put_json(KIND_PROFILE, &profile.id.to_string(), profile)
                .await?;
        } else if self.strict_persistence {
            return Err(AppError::Internal(
                "strict persistence enabled but rqlite is not configured".into(),
            ));
        }
        self.memory
            .write()
            .await
            .insert(profile.id, profile.clone());
        Ok(())
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.delete(KIND_PROFILE, &id.to_string()).await?;
        } else if self.strict_persistence {
            return Err(AppError::Internal(
                "strict persistence enabled but rqlite is not configured".into(),
            ));
        }
        self.memory.write().await.remove(&id);
        Ok(())
    }
}

// ── Schema Compare Profile Store ─────────────────────────────────────────────

/// Lightweight store for schema-compare saved profiles.
/// Uses rqlite when available, otherwise in-memory only.
#[derive(Clone)]
pub struct CompareProfileStore {
    memory: Arc<RwLock<HashMap<Uuid, SchemaCompareProfile>>>,
    rqlite: Option<RqliteClient>,
}

impl CompareProfileStore {
    pub fn new(rqlite: Option<RqliteClient>) -> Self {
        Self {
            memory: Arc::new(RwLock::new(HashMap::new())),
            rqlite,
        }
    }

    pub async fn list(&self) -> Result<Vec<SchemaCompareProfile>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq.list_json::<SchemaCompareProfile>(KIND_COMPARE).await {
                Ok(profiles) => {
                    let mut map = self.memory.write().await;
                    map.clear();
                    for p in &profiles {
                        map.insert(p.id, p.clone());
                    }
                    return Ok(profiles);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "rqlite list compare profiles failed; using memory")
                }
            }
        }
        let mut profiles: Vec<SchemaCompareProfile> =
            self.memory.read().await.values().cloned().collect();
        profiles.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(profiles)
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<SchemaCompareProfile>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq
                .get_json::<SchemaCompareProfile>(KIND_COMPARE, &id.to_string())
                .await
            {
                Ok(Some(p)) => {
                    self.memory.write().await.insert(p.id, p.clone());
                    return Ok(Some(p));
                }
                Ok(None) => return Ok(None),
                Err(e) => tracing::warn!(error = %e, "rqlite get compare profile failed"),
            }
        }
        Ok(self.memory.read().await.get(&id).cloned())
    }

    pub async fn upsert(&self, profile: &SchemaCompareProfile) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.put_json(KIND_COMPARE, &profile.id.to_string(), profile)
                .await?;
        }
        self.memory
            .write()
            .await
            .insert(profile.id, profile.clone());
        Ok(())
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.delete(KIND_COMPARE, &id.to_string()).await?;
        }
        self.memory.write().await.remove(&id);
        Ok(())
    }
}

// ── pgcopydb saved profiles ───────────────────────────────────────────────────

#[derive(Clone)]
pub struct PgCopyProfileStore {
    memory: Arc<RwLock<HashMap<Uuid, PgCopyProfile>>>,
    rqlite: Option<RqliteClient>,
}

impl PgCopyProfileStore {
    pub fn new(rqlite: Option<RqliteClient>) -> Self {
        Self {
            memory: Arc::new(RwLock::new(HashMap::new())),
            rqlite,
        }
    }

    pub async fn list(&self) -> Result<Vec<PgCopyProfile>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq.list_json::<PgCopyProfile>(KIND_PGCOPY).await {
                Ok(mut profiles) => {
                    profiles.sort_by(|a, b| a.name.cmp(&b.name));
                    let mut map = self.memory.write().await;
                    map.clear();
                    for p in &profiles {
                        map.insert(p.id, p.clone());
                    }
                    return Ok(profiles);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "rqlite list pgcopy profiles failed; using memory")
                }
            }
        }
        let mut profiles: Vec<PgCopyProfile> = self.memory.read().await.values().cloned().collect();
        profiles.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(profiles)
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<PgCopyProfile>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq
                .get_json::<PgCopyProfile>(KIND_PGCOPY, &id.to_string())
                .await
            {
                Ok(found) => {
                    if let Some(p) = &found {
                        self.memory.write().await.insert(p.id, p.clone());
                    }
                    return Ok(found);
                }
                Err(e) => tracing::warn!(error = %e, "rqlite get pgcopy profile failed"),
            }
        }
        Ok(self.memory.read().await.get(&id).cloned())
    }

    pub async fn upsert(&self, profile: &PgCopyProfile) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.put_json(KIND_PGCOPY, &profile.id.to_string(), profile)
                .await?;
        }
        self.memory
            .write()
            .await
            .insert(profile.id, profile.clone());
        Ok(())
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.delete(KIND_PGCOPY, &id.to_string()).await?;
        }
        self.memory.write().await.remove(&id);
        Ok(())
    }
}

// ── RedisShake saved profiles ─────────────────────────────────────────────────

#[derive(Clone)]
pub struct RedisShakeProfileStore {
    memory: Arc<RwLock<HashMap<Uuid, RedisShakeProfile>>>,
    rqlite: Option<RqliteClient>,
}

impl RedisShakeProfileStore {
    pub fn new(rqlite: Option<RqliteClient>) -> Self {
        Self {
            memory: Arc::new(RwLock::new(HashMap::new())),
            rqlite,
        }
    }

    pub async fn list(&self) -> Result<Vec<RedisShakeProfile>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq.list_json::<RedisShakeProfile>(KIND_REDISSHAKE).await {
                Ok(mut profiles) => {
                    profiles.sort_by(|a, b| a.name.cmp(&b.name));
                    let mut map = self.memory.write().await;
                    map.clear();
                    for p in &profiles {
                        map.insert(p.id, p.clone());
                    }
                    return Ok(profiles);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "rqlite list redisshake profiles failed; using memory")
                }
            }
        }
        let mut profiles: Vec<RedisShakeProfile> =
            self.memory.read().await.values().cloned().collect();
        profiles.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(profiles)
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<RedisShakeProfile>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq
                .get_json::<RedisShakeProfile>(KIND_REDISSHAKE, &id.to_string())
                .await
            {
                Ok(found) => {
                    if let Some(p) = &found {
                        self.memory.write().await.insert(p.id, p.clone());
                    }
                    return Ok(found);
                }
                Err(e) => tracing::warn!(error = %e, "rqlite get redisshake profile failed"),
            }
        }
        Ok(self.memory.read().await.get(&id).cloned())
    }

    pub async fn upsert(&self, profile: &RedisShakeProfile) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.put_json(KIND_REDISSHAKE, &profile.id.to_string(), profile)
                .await?;
        }
        self.memory
            .write()
            .await
            .insert(profile.id, profile.clone());
        Ok(())
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.delete(KIND_REDISSHAKE, &id.to_string()).await?;
        }
        self.memory.write().await.remove(&id);
        Ok(())
    }
}

// ── pgloader saved profiles ───────────────────────────────────────────────────

#[derive(Clone)]
pub struct PgLoaderProfileStore {
    memory: Arc<RwLock<HashMap<Uuid, PgLoaderProfile>>>,
    rqlite: Option<RqliteClient>,
}

impl PgLoaderProfileStore {
    pub fn new(rqlite: Option<RqliteClient>) -> Self {
        Self {
            memory: Arc::new(RwLock::new(HashMap::new())),
            rqlite,
        }
    }

    pub async fn list(&self) -> Result<Vec<PgLoaderProfile>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq.list_json::<PgLoaderProfile>(KIND_PGLOADER).await {
                Ok(mut profiles) => {
                    profiles.sort_by(|a, b| a.name.cmp(&b.name));
                    let mut map = self.memory.write().await;
                    map.clear();
                    for p in &profiles {
                        map.insert(p.id, p.clone());
                    }
                    return Ok(profiles);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "rqlite list pgloader profiles failed; using memory")
                }
            }
        }
        let mut profiles: Vec<PgLoaderProfile> =
            self.memory.read().await.values().cloned().collect();
        profiles.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(profiles)
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<PgLoaderProfile>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq
                .get_json::<PgLoaderProfile>(KIND_PGLOADER, &id.to_string())
                .await
            {
                Ok(found) => {
                    if let Some(p) = &found {
                        self.memory.write().await.insert(p.id, p.clone());
                    }
                    return Ok(found);
                }
                Err(e) => tracing::warn!(error = %e, "rqlite get pgloader profile failed"),
            }
        }
        Ok(self.memory.read().await.get(&id).cloned())
    }

    pub async fn upsert(&self, profile: &PgLoaderProfile) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.put_json(KIND_PGLOADER, &profile.id.to_string(), profile)
                .await?;
        }
        self.memory
            .write()
            .await
            .insert(profile.id, profile.clone());
        Ok(())
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.delete(KIND_PGLOADER, &id.to_string()).await?;
        }
        self.memory.write().await.remove(&id);
        Ok(())
    }
}

// ── mongocopy saved profiles ──────────────────────────────────────────────────

#[derive(Clone)]
pub struct MongoCopyProfileStore {
    memory: Arc<RwLock<HashMap<Uuid, MongoCopyProfile>>>,
    rqlite: Option<RqliteClient>,
}

impl MongoCopyProfileStore {
    pub fn new(rqlite: Option<RqliteClient>) -> Self {
        Self {
            memory: Arc::new(RwLock::new(HashMap::new())),
            rqlite,
        }
    }

    pub async fn list(&self) -> Result<Vec<MongoCopyProfile>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq.list_json::<MongoCopyProfile>(KIND_MONGOCOPY).await {
                Ok(mut profiles) => {
                    profiles.sort_by(|a, b| a.name.cmp(&b.name));
                    let mut map = self.memory.write().await;
                    map.clear();
                    for p in &profiles {
                        map.insert(p.id, p.clone());
                    }
                    return Ok(profiles);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "rqlite list mongocopy profiles failed; using memory")
                }
            }
        }
        let mut profiles: Vec<MongoCopyProfile> =
            self.memory.read().await.values().cloned().collect();
        profiles.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(profiles)
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<MongoCopyProfile>, AppError> {
        if let Some(rq) = &self.rqlite {
            match rq
                .get_json::<MongoCopyProfile>(KIND_MONGOCOPY, &id.to_string())
                .await
            {
                Ok(found) => {
                    if let Some(p) = &found {
                        self.memory.write().await.insert(p.id, p.clone());
                    }
                    return Ok(found);
                }
                Err(e) => tracing::warn!(error = %e, "rqlite get mongocopy profile failed"),
            }
        }
        Ok(self.memory.read().await.get(&id).cloned())
    }

    pub async fn upsert(&self, profile: &MongoCopyProfile) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.put_json(KIND_MONGOCOPY, &profile.id.to_string(), profile)
                .await?;
        }
        self.memory
            .write()
            .await
            .insert(profile.id, profile.clone());
        Ok(())
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        if let Some(rq) = &self.rqlite {
            rq.delete(KIND_MONGOCOPY, &id.to_string()).await?;
        }
        self.memory.write().await.remove(&id);
        Ok(())
    }
}
