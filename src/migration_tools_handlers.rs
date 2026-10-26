use axum::extract::{Path, State};
use axum::Json;
use serde_json::Value;

use crate::error::AppError;
use crate::migration_tools;
use crate::AppState;

pub async fn migration_tools_discover(State(state): State<AppState>) -> Json<Value> {
    Json(migration_tools::tools_catalog(&state.config))
}

pub async fn pgcopy_discover(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    migration_tools::pgcopy_discover(&state.config, &state.http)
        .await
        .map(Json)
}

pub async fn pgcopy_contract(State(state): State<AppState>) -> Json<Value> {
    Json(migration_tools::pgcopy_contract(&state.config))
}

pub async fn redisshake_discover(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    migration_tools::redisshake_discover(&state.config, &state.http)
        .await
        .map(Json)
}

pub async fn redisshake_contract(State(state): State<AppState>) -> Json<Value> {
    Json(migration_tools::redisshake_contract(&state.config))
}

pub async fn migration_tool_contract(
    State(state): State<AppState>,
    Path(tool_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    migration_tools::contract_for(&tool_id, &state.config).map(Json)
}

pub async fn migration_tool_discover(
    State(state): State<AppState>,
    Path(tool_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    match tool_id.as_str() {
        "pgcopy" => migration_tools::pgcopy_discover(&state.config, &state.http)
            .await
            .map(Json),
        "redisshake" => migration_tools::redisshake_discover(&state.config, &state.http)
            .await
            .map(Json),
        "pgloader" => migration_tools::pgloader_discover(&state.config, &state.http)
            .await
            .map(Json),
        "mongocopy" => migration_tools::mongocopy_discover(&state.config, &state.http)
            .await
            .map(Json),
        _ => Err(AppError::NotFound(format!(
            "unknown migration tool {tool_id}"
        ))),
    }
}

pub async fn pgloader_discover(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    migration_tools::pgloader_discover(&state.config, &state.http)
        .await
        .map(Json)
}

pub async fn pgloader_contract(State(state): State<AppState>) -> Json<Value> {
    Json(migration_tools::pgloader_contract(&state.config))
}

pub async fn mongocopy_discover(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    migration_tools::mongocopy_discover(&state.config, &state.http)
        .await
        .map(Json)
}

pub async fn mongocopy_contract(State(state): State<AppState>) -> Json<Value> {
    Json(migration_tools::mongocopy_contract(&state.config))
}
