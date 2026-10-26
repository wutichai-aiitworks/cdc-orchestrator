use hyper_cdc_orchestrator::function_runtime::build_pipeline;
use hyper_cdc_orchestrator::function_runtime::config::ProcessingPipelineSpec;
use hyper_cdc_orchestrator::function_runtime::kafka::{KafkaProcessor, KafkaRuntimeConfig};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "cdc_processor=info,hyper_cdc_orchestrator=info".into()),
        )
        .init();

    let config_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("CDC_PIPELINE_CONFIG").map(PathBuf::from))
        .ok_or("pass a pipeline JSON path or set CDC_PIPELINE_CONFIG")?;
    let raw = tokio::fs::read_to_string(&config_path).await?;
    let spec: ProcessingPipelineSpec = serde_json::from_str(&raw)?;
    spec.validate()?;

    let pipeline = build_pipeline(spec).await?;
    let processor = KafkaProcessor::new(KafkaRuntimeConfig::from_env()?, pipeline)?;
    let cancellation = CancellationToken::new();
    let signal = cancellation.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        signal.cancel();
    });
    processor.run(cancellation).await?;
    Ok(())
}
