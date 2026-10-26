use crate::error::AppError;
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use hyper_cdc_orchestrator::function_runtime::config::ProcessingPipelineSpec;
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
pub struct ValidationResponse {
    valid: bool,
    errors: Vec<String>,
}

pub async fn list_pipelines(
    State(state): State<AppState>,
) -> Result<Json<Vec<ProcessingPipelineSpec>>, AppError> {
    Ok(Json(
        state
            .processing_pipelines
            .list()
            .await
            .map_err(AppError::Upstream)?,
    ))
}

pub async fn get_pipeline(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ProcessingPipelineSpec>, AppError> {
    state
        .processing_pipelines
        .get(id)
        .await
        .map_err(AppError::Upstream)?
        .map(Json)
        .ok_or_else(|| AppError::NotFound(format!("processing pipeline {id} not found")))
}

pub async fn create_pipeline(
    State(state): State<AppState>,
    Json(spec): Json<ProcessingPipelineSpec>,
) -> Result<(StatusCode, Json<ProcessingPipelineSpec>), AppError> {
    if state
        .processing_pipelines
        .get(spec.id)
        .await
        .map_err(AppError::Upstream)?
        .is_some()
    {
        return Err(AppError::BadRequest(format!(
            "processing pipeline {} already exists",
            spec.id
        )));
    }
    let stored = state
        .processing_pipelines
        .upsert(spec)
        .await
        .map_err(AppError::BadRequest)?;
    Ok((StatusCode::CREATED, Json(stored)))
}

pub async fn update_pipeline(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(spec): Json<ProcessingPipelineSpec>,
) -> Result<Json<ProcessingPipelineSpec>, AppError> {
    if id != spec.id {
        return Err(AppError::BadRequest(
            "path id must match processing pipeline id".into(),
        ));
    }
    if state
        .processing_pipelines
        .get(id)
        .await
        .map_err(AppError::Upstream)?
        .is_none()
    {
        return Err(AppError::NotFound(format!(
            "processing pipeline {id} not found"
        )));
    }
    state
        .processing_pipelines
        .upsert(spec)
        .await
        .map(Json)
        .map_err(AppError::BadRequest)
}

pub async fn delete_pipeline(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    if state
        .processing_pipelines
        .delete(id)
        .await
        .map_err(AppError::Upstream)?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound(format!(
            "processing pipeline {id} not found"
        )))
    }
}

pub async fn validate_pipeline(
    Json(spec): Json<ProcessingPipelineSpec>,
) -> Json<ValidationResponse> {
    match spec.validate() {
        Ok(()) => Json(ValidationResponse {
            valid: true,
            errors: vec![],
        }),
        Err(error) => Json(ValidationResponse {
            valid: false,
            errors: vec![error],
        }),
    }
}
