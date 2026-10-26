use crate::function_runtime::config::ProcessingPipelineSpec;
use crate::persist::{RqliteClient, KIND_PROCESSING};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct ProcessingPipelineStore {
    entries: Arc<RwLock<HashMap<Uuid, ProcessingPipelineSpec>>>,
    rqlite: Option<RqliteClient>,
}

impl ProcessingPipelineStore {
    pub fn new(rqlite: Option<RqliteClient>) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            rqlite,
        }
    }

    pub async fn list(&self) -> Result<Vec<ProcessingPipelineSpec>, String> {
        if let Some(rq) = &self.rqlite {
            let items = rq
                .list_json::<ProcessingPipelineSpec>(KIND_PROCESSING)
                .await
                .map_err(|e| e.to_string())?;
            let mut map = self.entries.write().await;
            map.clear();
            for item in &items {
                map.insert(item.id, item.clone());
            }
            return Ok(items);
        }
        Ok(self.entries.read().await.values().cloned().collect())
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<ProcessingPipelineSpec>, String> {
        if let Some(rq) = &self.rqlite {
            return rq
                .get_json::<ProcessingPipelineSpec>(KIND_PROCESSING, &id.to_string())
                .await
                .map_err(|e| e.to_string());
        }
        Ok(self.entries.read().await.get(&id).cloned())
    }

    pub async fn upsert(
        &self,
        spec: ProcessingPipelineSpec,
    ) -> Result<ProcessingPipelineSpec, String> {
        spec.validate()?;
        if let Some(rq) = &self.rqlite {
            rq.put_json(KIND_PROCESSING, &spec.id.to_string(), &spec)
                .await
                .map_err(|e| e.to_string())?;
        }
        self.entries.write().await.insert(spec.id, spec.clone());
        Ok(spec)
    }

    pub async fn delete(&self, id: Uuid) -> Result<bool, String> {
        if let Some(rq) = &self.rqlite {
            let existed = rq
                .get_json::<ProcessingPipelineSpec>(KIND_PROCESSING, &id.to_string())
                .await
                .map_err(|e| e.to_string())?
                .is_some();
            rq.delete(KIND_PROCESSING, &id.to_string())
                .await
                .map_err(|e| e.to_string())?;
            self.entries.write().await.remove(&id);
            return Ok(existed);
        }
        Ok(self.entries.write().await.remove(&id).is_some())
    }
}
