pub mod config;
pub mod executor;
pub mod kafka;
pub mod store;

use crate::function_runtime::config::{ExecutionMode, ProcessingPipelineSpec};
use crate::function_runtime::executor::{
    BatchStage, GrpcBatchStage, ProcessingPipeline, StageError,
};
use std::sync::Arc;

pub async fn build_pipeline(
    spec: ProcessingPipelineSpec,
) -> Result<Arc<ProcessingPipeline>, StageError> {
    let mut stages: Vec<Arc<dyn BatchStage>> = Vec::new();
    for stage in spec.stages.iter().filter(|stage| stage.enabled) {
        let executor: Arc<dyn BatchStage> = match stage.execution {
            ExecutionMode::Grpc => Arc::new(GrpcBatchStage::connect(stage).await?),
            ExecutionMode::DataService => {
                return Err(StageError::terminal(format!(
                    "stage {} uses removed DataService enrichment; use a gRPC stage",
                    stage.id
                )));
            }
        };
        stages.push(executor);
    }
    Ok(Arc::new(ProcessingPipeline::new(spec, stages)?))
}
