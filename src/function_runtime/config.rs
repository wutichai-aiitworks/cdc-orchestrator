use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    Transform,
    Filter,
    Enrich,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Grpc,
    DataService,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    FailPipeline,
    EmitUnchanged,
    Drop,
    DeadLetter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchPolicy {
    #[serde(default = "default_max_records")]
    pub max_records: usize,
    #[serde(default = "default_max_bytes")]
    pub max_bytes: usize,
    #[serde(default = "default_max_wait_ms")]
    pub max_wait_ms: u64,
}

impl Default for BatchPolicy {
    fn default() -> Self {
        Self {
            max_records: default_max_records(),
            max_bytes: default_max_bytes(),
            max_wait_ms: default_max_wait_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    #[serde(default = "default_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_backoff_ms")]
    pub initial_backoff_ms: u64,
    #[serde(default = "default_max_backoff_ms")]
    pub max_backoff_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: default_attempts(),
            initial_backoff_ms: default_backoff_ms(),
            max_backoff_ms: default_max_backoff_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataServiceEnrichmentConfig {
    pub base_url: String,
    pub datasource: String,
    pub mapper: String,
    pub api_key_env: String,
    pub lookup_field: String,
    #[serde(default = "default_lookup_parameter")]
    pub lookup_parameter: String,
    pub result_key_field: String,
    #[serde(default)]
    pub output_field: Option<String>,
    #[serde(default)]
    pub static_parameters: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingStageSpec {
    pub id: String,
    pub name: String,
    pub kind: StageKind,
    pub execution: ExecutionMode,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub dataservice: Option<DataServiceEnrichmentConfig>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub retry: RetryPolicy,
    #[serde(default = "default_failure_policy")]
    pub failure_policy: FailurePolicy,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub attributes: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingPipelineSpec {
    pub id: Uuid,
    pub name: String,
    pub input_topic: String,
    pub output_topic: String,
    pub dead_letter_topic: String,
    pub consumer_group: String,
    #[serde(default)]
    pub batch: BatchPolicy,
    #[serde(default)]
    pub stages: Vec<ProcessingStageSpec>,
    #[serde(default = "default_true")]
    pub preserve_partition_order: bool,
    #[serde(default)]
    pub enabled: bool,
}

impl ProcessingPipelineSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("pipeline name is required".into());
        }
        for (field, value) in [
            ("input_topic", &self.input_topic),
            ("output_topic", &self.output_topic),
            ("dead_letter_topic", &self.dead_letter_topic),
            ("consumer_group", &self.consumer_group),
        ] {
            if value.trim().is_empty() {
                return Err(format!("{field} is required"));
            }
        }
        if self.input_topic == self.output_topic {
            return Err("input_topic and output_topic must differ".into());
        }
        if self.batch.max_records == 0 || self.batch.max_records > 10_000 {
            return Err("batch.max_records must be between 1 and 10000".into());
        }
        if self.batch.max_bytes == 0 || self.batch.max_bytes > 64 * 1024 * 1024 {
            return Err("batch.max_bytes must be between 1 and 67108864".into());
        }

        let mut ids = std::collections::HashSet::new();
        for stage in &self.stages {
            if stage.id.trim().is_empty() || !ids.insert(stage.id.as_str()) {
                return Err("stage ids must be non-empty and unique".into());
            }
            match stage.execution {
                ExecutionMode::Grpc if stage.endpoint.as_deref().unwrap_or("").is_empty() => {
                    return Err(format!("stage {} requires endpoint", stage.id));
                }
                ExecutionMode::DataService => {
                    return Err(format!(
                        "stage {} uses removed DataService enrichment; use execution=grpc",
                        stage.id
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn default_max_records() -> usize {
    500
}

fn default_max_bytes() -> usize {
    4 * 1024 * 1024
}

fn default_max_wait_ms() -> u64 {
    25
}

fn default_attempts() -> u32 {
    3
}

fn default_backoff_ms() -> u64 {
    100
}

fn default_max_backoff_ms() -> u64 {
    5_000
}

fn default_timeout_ms() -> u64 {
    10_000
}

fn default_lookup_parameter() -> String {
    "list".into()
}

fn default_failure_policy() -> FailurePolicy {
    FailurePolicy::DeadLetter
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_same_input_and_output_topic() {
        let spec = ProcessingPipelineSpec {
            id: Uuid::new_v4(),
            name: "customers".into(),
            input_topic: "customers".into(),
            output_topic: "customers".into(),
            dead_letter_topic: "customers.dlq".into(),
            consumer_group: "customers.processor".into(),
            batch: BatchPolicy::default(),
            stages: vec![],
            preserve_partition_order: true,
            enabled: true,
        };
        assert!(spec.validate().is_err());
    }
}
