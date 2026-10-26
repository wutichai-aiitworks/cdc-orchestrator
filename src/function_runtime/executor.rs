use crate::function_proto::cdc_function_client::CdcFunctionClient;
use crate::function_proto::{
    BatchResult, CdcRecord, Decision, FunctionError, RecordBatch, RecordResult,
};
use crate::function_runtime::config::{FailurePolicy, ProcessingPipelineSpec, ProcessingStageSpec};
use async_trait::async_trait;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::iter;
use tonic::transport::Channel;
use tonic::Request;

#[derive(Debug, Clone)]
pub struct StageError {
    pub message: String,
    pub retryable: bool,
}

impl StageError {
    pub fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }

    pub fn terminal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }
}

impl fmt::Display for StageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for StageError {}

#[async_trait]
pub trait BatchStage: Send + Sync {
    fn id(&self) -> &str;
    async fn process(&self, batch: RecordBatch) -> Result<BatchResult, StageError>;
}

pub struct GrpcBatchStage {
    id: String,
    timeout: Duration,
    client: CdcFunctionClient<Channel>,
}

impl GrpcBatchStage {
    pub async fn connect(spec: &ProcessingStageSpec) -> Result<Self, StageError> {
        let endpoint = spec
            .endpoint
            .as_deref()
            .ok_or_else(|| StageError::terminal("gRPC endpoint is required"))?;
        let client = CdcFunctionClient::connect(endpoint.to_string())
            .await
            .map_err(|error| {
                StageError::retryable(format!("connect function {}: {error}", spec.id))
            })?;
        Ok(Self {
            id: spec.id.clone(),
            timeout: Duration::from_millis(spec.timeout_ms),
            client,
        })
    }
}

#[async_trait]
impl BatchStage for GrpcBatchStage {
    fn id(&self) -> &str {
        &self.id
    }

    async fn process(&self, batch: RecordBatch) -> Result<BatchResult, StageError> {
        let invocation_id = batch.invocation_id.clone();
        let mut client = self.client.clone();
        let request = Request::new(iter([batch]));
        let response = tokio::time::timeout(self.timeout, client.process(request))
            .await
            .map_err(|_| StageError::retryable(format!("function {} timed out", self.id)))?
            .map_err(|status| {
                let retryable = matches!(
                    status.code(),
                    tonic::Code::Unavailable
                        | tonic::Code::DeadlineExceeded
                        | tonic::Code::ResourceExhausted
                        | tonic::Code::Aborted
                );
                StageError {
                    message: format!("function {}: {status}", self.id),
                    retryable,
                }
            })?;
        let mut stream = response.into_inner();
        let result = stream
            .message()
            .await
            .map_err(|status| {
                StageError::retryable(format!("function {} stream: {status}", self.id))
            })?
            .ok_or_else(|| {
                StageError::retryable(format!("function {} returned no result", self.id))
            })?;
        if result.invocation_id != invocation_id {
            return Err(StageError::terminal(format!(
                "function {} returned mismatched invocation_id",
                self.id
            )));
        }
        Ok(result)
    }
}

#[derive(Debug, Default)]
pub struct PipelineOutcome {
    pub emitted: Vec<CdcRecord>,
    pub dropped: Vec<CdcRecord>,
    pub dead_letter: Vec<(CdcRecord, FunctionError)>,
}

pub struct ProcessingPipeline {
    spec: ProcessingPipelineSpec,
    stages: Vec<Arc<dyn BatchStage>>,
}

impl ProcessingPipeline {
    pub fn new(
        spec: ProcessingPipelineSpec,
        stages: Vec<Arc<dyn BatchStage>>,
    ) -> Result<Self, StageError> {
        spec.validate().map_err(StageError::terminal)?;
        let enabled_ids = spec
            .stages
            .iter()
            .filter(|stage| stage.enabled)
            .map(|stage| stage.id.as_str())
            .collect::<Vec<_>>();
        let executor_ids = stages.iter().map(|stage| stage.id()).collect::<Vec<_>>();
        if enabled_ids != executor_ids {
            return Err(StageError::terminal(
                "stage executors must match enabled pipeline stages in order",
            ));
        }
        Ok(Self { spec, stages })
    }

    pub fn spec(&self) -> &ProcessingPipelineSpec {
        &self.spec
    }

    pub async fn execute(&self, mut batch: RecordBatch) -> Result<PipelineOutcome, StageError> {
        let mut outcome = PipelineOutcome::default();
        if self.stages.is_empty() {
            outcome.emitted = batch.records;
            return Ok(outcome);
        }
        let batch_attributes = batch.attributes.clone();

        for (stage_index, executor) in self.stages.iter().enumerate() {
            if batch.records.is_empty() {
                break;
            }
            let stage_spec = self
                .spec
                .stages
                .iter()
                .filter(|stage| stage.enabled)
                .nth(stage_index)
                .ok_or_else(|| StageError::terminal("stage configuration missing"))?;
            batch.stage_id = stage_spec.id.clone();
            batch.attributes.clone_from(&batch_attributes);
            batch.attributes.extend(stage_spec.attributes.clone());
            let input = batch.records.clone();
            let result = self
                .execute_with_retry(executor.as_ref(), stage_spec, &batch)
                .await;
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    self.apply_failure_policy(&mut outcome, input, stage_spec, error)?;
                    batch.records = Vec::new();
                    break;
                }
            };

            if let Some(error) = result.error {
                let stage_error = StageError {
                    message: error.message,
                    retryable: error.retryable,
                };
                self.apply_failure_policy(&mut outcome, input, stage_spec, stage_error)?;
                batch.records = Vec::new();
                break;
            }

            let originals = input
                .into_iter()
                .map(|record| (record.record_id.clone(), record))
                .collect::<HashMap<_, _>>();
            let mut next = Vec::new();
            let result_count = result.results.len();
            for item in result.results {
                let Some(original) = originals.get(&item.record_id) else {
                    return Err(StageError::terminal(format!(
                        "stage {} returned unknown record_id {}",
                        stage_spec.id, item.record_id
                    )));
                };
                let record = merge_result(original, &item);
                match Decision::try_from(item.decision).unwrap_or(Decision::DeadLetter) {
                    Decision::Emit => next.push(record),
                    Decision::Drop => outcome.dropped.push(record),
                    Decision::DeadLetter | Decision::Retry | Decision::Unspecified => {
                        outcome.dead_letter.push((
                            record,
                            item.error.unwrap_or_else(|| FunctionError {
                                code: "FUNCTION_DECISION".into(),
                                message: format!(
                                    "stage {} returned {:?}",
                                    stage_spec.id,
                                    Decision::try_from(item.decision)
                                        .unwrap_or(Decision::DeadLetter)
                                ),
                                retryable: false,
                                details: HashMap::new(),
                            }),
                        ));
                    }
                }
            }
            if result_count != originals.len() {
                return Err(StageError::terminal(format!(
                    "stage {} returned {} results for {} records",
                    stage_spec.id,
                    result_count,
                    originals.len()
                )));
            }
            batch.records = next;
        }

        outcome.emitted.extend(batch.records);
        Ok(outcome)
    }

    async fn execute_with_retry(
        &self,
        executor: &dyn BatchStage,
        spec: &ProcessingStageSpec,
        batch: &RecordBatch,
    ) -> Result<BatchResult, StageError> {
        let attempts = spec.retry.max_attempts.max(1);
        let mut backoff = spec.retry.initial_backoff_ms;
        for attempt in 1..=attempts {
            let mut request = batch.clone();
            request.attempt = attempt;
            match executor.process(request).await {
                Ok(result) if !contains_retry(&result) => return Ok(result),
                Ok(_) if attempt < attempts => {}
                Ok(result) => return Ok(result),
                Err(error) if !error.retryable || attempt == attempts => return Err(error),
                Err(_) => {}
            }
            tokio::time::sleep(Duration::from_millis(backoff)).await;
            backoff = backoff.saturating_mul(2).min(spec.retry.max_backoff_ms);
        }
        Err(StageError::terminal("retry loop exhausted"))
    }

    fn apply_failure_policy(
        &self,
        outcome: &mut PipelineOutcome,
        records: Vec<CdcRecord>,
        spec: &ProcessingStageSpec,
        error: StageError,
    ) -> Result<(), StageError> {
        match spec.failure_policy {
            FailurePolicy::FailPipeline => Err(error),
            FailurePolicy::EmitUnchanged => {
                outcome.emitted.extend(records);
                Ok(())
            }
            FailurePolicy::Drop => {
                outcome.dropped.extend(records);
                Ok(())
            }
            FailurePolicy::DeadLetter => {
                for record in records {
                    outcome.dead_letter.push((
                        record,
                        FunctionError {
                            code: "STAGE_FAILED".into(),
                            message: error.message.clone(),
                            retryable: error.retryable,
                            details: HashMap::from([("stage_id".into(), spec.id.clone())]),
                        },
                    ));
                }
                Ok(())
            }
        }
    }
}

fn contains_retry(result: &BatchResult) -> bool {
    result.error.as_ref().is_some_and(|error| error.retryable)
        || result
            .results
            .iter()
            .any(|item| item.decision == Decision::Retry as i32)
}

fn merge_result(original: &CdcRecord, result: &RecordResult) -> CdcRecord {
    CdcRecord {
        record_id: original.record_id.clone(),
        key: result.key.clone().or_else(|| original.key.clone()),
        value: result.value.clone().or_else(|| original.value.clone()),
        headers: if result.headers.is_empty() {
            original.headers.clone()
        } else {
            result.headers.clone()
        },
        timestamp_ms: original.timestamp_ms,
        offset: original.offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::function_runtime::config::{
        BatchPolicy, ExecutionMode, ProcessingStageSpec, RetryPolicy, StageKind,
    };
    use uuid::Uuid;

    struct DropInactive;

    #[async_trait]
    impl BatchStage for DropInactive {
        fn id(&self) -> &str {
            "filter"
        }

        async fn process(&self, batch: RecordBatch) -> Result<BatchResult, StageError> {
            let results = batch
                .records
                .iter()
                .map(|record| {
                    let active = record
                        .value
                        .as_ref()
                        .and_then(|value| serde_json::from_slice::<serde_json::Value>(value).ok())
                        .and_then(|value| value.get("active").and_then(|v| v.as_bool()))
                        .unwrap_or(false);
                    RecordResult {
                        record_id: record.record_id.clone(),
                        decision: if active {
                            Decision::Emit as i32
                        } else {
                            Decision::Drop as i32
                        },
                        key: None,
                        value: None,
                        headers: vec![],
                        error: None,
                    }
                })
                .collect();
            Ok(BatchResult {
                invocation_id: batch.invocation_id,
                results,
                error: None,
            })
        }
    }

    #[tokio::test]
    async fn applies_filter_decisions() {
        let stage = ProcessingStageSpec {
            id: "filter".into(),
            name: "Filter inactive".into(),
            kind: StageKind::Filter,
            execution: ExecutionMode::Grpc,
            endpoint: Some("http://127.0.0.1:50051".into()),
            dataservice: None,
            timeout_ms: 1_000,
            retry: RetryPolicy::default(),
            failure_policy: FailurePolicy::DeadLetter,
            enabled: true,
            attributes: HashMap::new(),
        };
        let spec = ProcessingPipelineSpec {
            id: Uuid::new_v4(),
            name: "customers".into(),
            input_topic: "customers.raw".into(),
            output_topic: "customers.trusted".into(),
            dead_letter_topic: "customers.dlq".into(),
            consumer_group: "customers.processor".into(),
            batch: BatchPolicy::default(),
            stages: vec![stage],
            preserve_partition_order: true,
            enabled: true,
        };
        let pipeline = ProcessingPipeline::new(spec, vec![Arc::new(DropInactive)]).unwrap();
        let records = [true, false]
            .into_iter()
            .enumerate()
            .map(|(index, active)| CdcRecord {
                record_id: index.to_string(),
                key: None,
                value: Some(serde_json::to_vec(&serde_json::json!({ "active": active })).unwrap()),
                headers: vec![],
                timestamp_ms: 0,
                offset: index as i64,
            })
            .collect();
        let outcome = pipeline
            .execute(RecordBatch {
                pipeline_id: pipeline.spec.id.to_string(),
                stage_id: String::new(),
                invocation_id: Uuid::new_v4().to_string(),
                attempt: 0,
                topic: "customers.raw".into(),
                partition: 0,
                records,
                attributes: HashMap::new(),
            })
            .await
            .unwrap();
        assert_eq!(outcome.emitted.len(), 1);
        assert_eq!(outcome.dropped.len(), 1);
    }
}
