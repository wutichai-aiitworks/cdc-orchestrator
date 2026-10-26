use futures_util::StreamExt;
use hyper_cdc_orchestrator::function_proto::cdc_function_server::{CdcFunction, CdcFunctionServer};
use hyper_cdc_orchestrator::function_proto::{
    BatchResult, CapabilityRequest, CapabilityResponse, Decision, FunctionError, RecordBatch,
    RecordResult,
};
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

enum RecordAction {
    Emit,
    Drop,
}

struct FunctionFailure {
    code: &'static str,
    message: String,
    retryable: bool,
}

trait RecordFunction: Send + Sync {
    fn apply(
        &self,
        value: &mut Value,
        attributes: &HashMap<String, String>,
    ) -> Result<RecordAction, FunctionFailure>;
}

struct NormalizeCustomer;

impl RecordFunction for NormalizeCustomer {
    fn apply(
        &self,
        value: &mut Value,
        _attributes: &HashMap<String, String>,
    ) -> Result<RecordAction, FunctionFailure> {
        let object = value.as_object_mut().ok_or_else(|| FunctionFailure {
            code: "INVALID_SHAPE",
            message: "normalize_customer requires a JSON object".into(),
            retryable: false,
        })?;
        if let Some(email) = object
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
        {
            object.insert("email".into(), Value::String(email));
        }
        let full_name = [
            object.get("first_name").and_then(Value::as_str),
            object.get("last_name").and_then(Value::as_str),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        if !full_name.is_empty() {
            object.insert("full_name".into(), Value::String(full_name));
        }
        Ok(RecordAction::Emit)
    }
}

struct FilterActive;

impl RecordFunction for FilterActive {
    fn apply(
        &self,
        value: &mut Value,
        attributes: &HashMap<String, String>,
    ) -> Result<RecordAction, FunctionFailure> {
        let field = attributes
            .get("active_field")
            .map(String::as_str)
            .unwrap_or("active");
        Ok(
            if value.get(field).and_then(Value::as_bool) == Some(false) {
                RecordAction::Drop
            } else {
                RecordAction::Emit
            },
        )
    }
}

struct EnrichRegion;

impl RecordFunction for EnrichRegion {
    fn apply(
        &self,
        value: &mut Value,
        attributes: &HashMap<String, String>,
    ) -> Result<RecordAction, FunctionFailure> {
        let country_field = attributes
            .get("country_field")
            .map(String::as_str)
            .unwrap_or("country");
        let region = match value.get(country_field).and_then(Value::as_str) {
            Some("TH" | "SG" | "JP" | "AU") => "apac",
            Some("DE" | "FR" | "GB") => "emea",
            Some("US" | "CA") => "americas",
            _ => "other",
        };
        let object = value.as_object_mut().ok_or_else(|| FunctionFailure {
            code: "INVALID_SHAPE",
            message: "enrich_region requires a JSON object".into(),
            retryable: false,
        })?;
        object.insert("region".into(), Value::String(region.into()));
        object.insert(
            "enriched_by".into(),
            Value::String("sample-customer-function".into()),
        );
        Ok(RecordAction::Emit)
    }
}

struct MaskEmail;

impl RecordFunction for MaskEmail {
    fn apply(
        &self,
        value: &mut Value,
        _attributes: &HashMap<String, String>,
    ) -> Result<RecordAction, FunctionFailure> {
        let Some(object) = value.as_object_mut() else {
            return Err(FunctionFailure {
                code: "INVALID_SHAPE",
                message: "mask_email requires a JSON object".into(),
                retryable: false,
            });
        };
        if object.get("email").is_some_and(Value::is_string) {
            object.insert("email".into(), Value::String("***@***".into()));
        }
        Ok(RecordAction::Emit)
    }
}

#[derive(Clone)]
struct FunctionRegistry {
    handlers: HashMap<&'static str, Arc<dyn RecordFunction>>,
}

impl Default for FunctionRegistry {
    fn default() -> Self {
        Self {
            handlers: HashMap::from([
                (
                    "normalize_customer",
                    Arc::new(NormalizeCustomer) as Arc<dyn RecordFunction>,
                ),
                ("filter_active", Arc::new(FilterActive)),
                ("enrich_region", Arc::new(EnrichRegion)),
                ("mask_email", Arc::new(MaskEmail)),
            ]),
        }
    }
}

impl FunctionRegistry {
    fn resolve(&self, batch: &RecordBatch) -> Option<&Arc<dyn RecordFunction>> {
        let operation = batch
            .attributes
            .get("function")
            .map(String::as_str)
            .unwrap_or_else(|| match batch.stage_id.as_str() {
                "transform" => "normalize_customer",
                "filter" => "filter_active",
                "enrich" => "enrich_region",
                other => other,
            });
        self.handlers.get(operation)
    }

    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .handlers
            .keys()
            .map(|name| (*name).to_string())
            .collect();
        names.sort();
        names
    }
}

#[derive(Clone, Default)]
struct SampleFunction {
    registry: FunctionRegistry,
}

#[tonic::async_trait]
impl CdcFunction for SampleFunction {
    type ProcessStream =
        Pin<Box<dyn tokio_stream::Stream<Item = Result<BatchResult, Status>> + Send>>;

    async fn capabilities(
        &self,
        _request: Request<CapabilityRequest>,
    ) -> Result<Response<CapabilityResponse>, Status> {
        Ok(Response::new(CapabilityResponse {
            function_name: "sample-customer-function".into(),
            function_version: "1.0.0".into(),
            supported_stages: self.registry.names(),
            max_batch_records: 1_000,
            max_batch_bytes: 8 * 1024 * 1024,
        }))
    }

    async fn process(
        &self,
        request: Request<Streaming<RecordBatch>>,
    ) -> Result<Response<Self::ProcessStream>, Status> {
        let mut inbound = request.into_inner();
        let (sender, receiver) = mpsc::channel(8);
        let registry = self.registry.clone();
        tokio::spawn(async move {
            while let Some(item) = inbound.next().await {
                let batch = match item {
                    Ok(batch) => batch,
                    Err(error) => {
                        let _ = sender.send(Err(error)).await;
                        break;
                    }
                };
                let result = process_batch(&registry, batch);
                if sender.send(Ok(result)).await.is_err() {
                    break;
                }
            }
        });
        Ok(Response::new(
            Box::pin(ReceiverStream::new(receiver)) as Self::ProcessStream
        ))
    }
}

fn process_batch(registry: &FunctionRegistry, batch: RecordBatch) -> BatchResult {
    let Some(function) = registry.resolve(&batch) else {
        return BatchResult {
            invocation_id: batch.invocation_id,
            results: batch
                .records
                .iter()
                .map(|record| {
                    dead_letter(
                        &record.record_id,
                        "UNSUPPORTED_FUNCTION",
                        &format!(
                            "no function registered for stage {} or attribute function",
                            batch.stage_id
                        ),
                        false,
                    )
                })
                .collect(),
            error: None,
        };
    };
    let results = batch
        .records
        .iter()
        .map(|record| {
            let parsed = record
                .value
                .as_ref()
                .and_then(|value| serde_json::from_slice::<Value>(value).ok());
            let Some(mut value) = parsed else {
                return dead_letter(
                    &record.record_id,
                    "INVALID_JSON",
                    "record value is not JSON",
                    false,
                );
            };

            let decision = match function.apply(&mut value, &batch.attributes) {
                Ok(RecordAction::Emit) => Decision::Emit,
                Ok(RecordAction::Drop) => Decision::Drop,
                Err(error) => {
                    return dead_letter(
                        &record.record_id,
                        error.code,
                        &error.message,
                        error.retryable,
                    );
                }
            };
            RecordResult {
                record_id: record.record_id.clone(),
                decision: decision as i32,
                key: None,
                value: serde_json::to_vec(&value).ok(),
                headers: vec![],
                error: None,
            }
        })
        .collect();
    BatchResult {
        invocation_id: batch.invocation_id,
        results,
        error: None,
    }
}

fn dead_letter(record_id: &str, code: &str, message: &str, retryable: bool) -> RecordResult {
    RecordResult {
        record_id: record_id.into(),
        decision: Decision::DeadLetter as i32,
        key: None,
        value: None,
        headers: vec![],
        error: Some(FunctionError {
            code: code.into(),
            message: message.into(),
            retryable,
            details: HashMap::new(),
        }),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "sample_function=info".into()),
        )
        .init();
    let address = std::env::var("CDC_FUNCTION_ADDRESS")
        .unwrap_or_else(|_| "127.0.0.1:50051".into())
        .parse()?;
    tracing::info!(%address, "sample CDC function listening");
    tonic::transport::Server::builder()
        .add_service(CdcFunctionServer::new(SampleFunction::default()))
        .serve(address)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper_cdc_orchestrator::function_proto::CdcRecord;

    #[test]
    fn filters_inactive_customer() {
        let result = process_batch(
            &FunctionRegistry::default(),
            RecordBatch {
                pipeline_id: "customers".into(),
                stage_id: "filter".into(),
                invocation_id: "test".into(),
                attempt: 1,
                topic: "customers.raw".into(),
                partition: 0,
                records: vec![CdcRecord {
                    record_id: "0".into(),
                    key: None,
                    value: Some(br#"{"active":false}"#.to_vec()),
                    headers: vec![],
                    timestamp_ms: 0,
                    offset: 0,
                }],
                attributes: HashMap::new(),
            },
        );
        assert_eq!(result.results[0].decision, Decision::Drop as i32);
    }

    #[test]
    fn dispatches_custom_function_from_stage_attributes() {
        let result = process_batch(
            &FunctionRegistry::default(),
            RecordBatch {
                pipeline_id: "customers".into(),
                stage_id: "custom-transform".into(),
                invocation_id: "test".into(),
                attempt: 1,
                topic: "customers.raw".into(),
                partition: 0,
                records: vec![CdcRecord {
                    record_id: "0".into(),
                    key: None,
                    value: Some(br#"{"email":"person@example.com"}"#.to_vec()),
                    headers: vec![],
                    timestamp_ms: 0,
                    offset: 0,
                }],
                attributes: HashMap::from([("function".into(), "mask_email".into())]),
            },
        );
        let value: Value =
            serde_json::from_slice(result.results[0].value.as_ref().unwrap()).unwrap();
        assert_eq!(value["email"], "***@***");
    }
}
