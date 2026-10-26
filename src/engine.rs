use crate::command::run_docker;
use crate::config::{AppConfig, EngineProvider};
use crate::error::AppError;
use crate::models::ServiceStatus;
use serde_json::Value;

pub async fn engine_status(cfg: &AppConfig) -> Result<(String, Vec<ServiceStatus>), AppError> {
    match cfg.engine_provider {
        EngineProvider::DockerApi => {
            let label_filter = format!(
                "label=com.docker.compose.project={}",
                cfg.container_project_name
            );
            let raw = run_docker(
                cfg,
                &[
                    "ps",
                    "-a",
                    "--filter",
                    label_filter.as_str(),
                    "--format",
                    "{{json .}}",
                ],
            )
            .await?;
            Ok((
                format!("docker_api:{}", cfg.container_project_name),
                parse_docker_ps_json_lines(&raw),
            ))
        }
        EngineProvider::Kubernetes => Err(AppError::BadRequest(
            "engine provider kubernetes is planned but not implemented yet".to_string(),
        )),
    }
}

pub async fn engine_up(cfg: &AppConfig) -> Result<String, AppError> {
    match cfg.engine_provider {
        EngineProvider::DockerApi => {
            let args = with_managed_containers("start", cfg);
            run_docker(cfg, &args).await?;
            Ok("docker start executed for managed containers".to_string())
        }
        EngineProvider::Kubernetes => Err(AppError::BadRequest(
            "engine provider kubernetes is planned but not implemented yet".to_string(),
        )),
    }
}

pub async fn engine_down(cfg: &AppConfig) -> Result<String, AppError> {
    match cfg.engine_provider {
        EngineProvider::DockerApi => {
            let args = with_managed_containers("stop", cfg);
            run_docker(cfg, &args).await?;
            Ok("docker stop executed for managed containers".to_string())
        }
        EngineProvider::Kubernetes => Err(AppError::BadRequest(
            "engine provider kubernetes is planned but not implemented yet".to_string(),
        )),
    }
}

pub async fn engine_restart(cfg: &AppConfig) -> Result<String, AppError> {
    match cfg.engine_provider {
        EngineProvider::DockerApi => {
            let args = with_managed_containers("restart", cfg);
            run_docker(cfg, &args).await?;
            Ok("docker restart executed for managed containers".to_string())
        }
        EngineProvider::Kubernetes => Err(AppError::BadRequest(
            "engine provider kubernetes is planned but not implemented yet".to_string(),
        )),
    }
}

fn with_managed_containers<'a>(op: &'a str, cfg: &'a AppConfig) -> Vec<&'a str> {
    let mut args = vec![op];
    for container in &cfg.managed_containers {
        args.push(container.as_str());
    }
    args
}

fn parse_docker_ps_json_lines(raw: &str) -> Vec<ServiceStatus> {
    let mut out = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            out.push(ServiceStatus {
                name: v
                    .get("Names")
                    .or_else(|| v.get("Name"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                state: v
                    .get("State")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                health: v
                    .get("Status")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            });
        }
    }
    out
}
