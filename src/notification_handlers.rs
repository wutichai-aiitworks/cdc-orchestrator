//! `/cdc/notifications/channels*` and `/cdc/notifications/deliveries` (ADR 0015).

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use hyper_cdc_orchestrator::persist::{KIND_CHANNEL, KIND_RULE};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit::log_action;
use crate::error::AppError;
use crate::models::AlertRule;
use crate::notify::channel::{prepare_for_store, NotificationChannel};
use crate::AppState;

fn rqlite(state: &AppState) -> Result<&hyper_cdc_orchestrator::persist::RqliteClient, AppError> {
    state
        .rqlite
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("notification channels require ORCH_RQLITE_URL".into()))
}

fn actor(headers: &HeaderMap) -> &str {
    headers
        .get("x-user")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("system")
}

async fn stored(state: &AppState, id: &str) -> Result<Option<NotificationChannel>, AppError> {
    Ok(rqlite(state)?
        .get_json::<NotificationChannel>(KIND_CHANNEL, id)
        .await?)
}

pub async fn list_channels(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let mut items: Vec<NotificationChannel> = rqlite(&state)?.list_json(KIND_CHANNEL).await?;
    items.sort_by_key(|a| a.name.to_lowercase());
    let items: Vec<Value> = items
        .iter()
        .map(|c| {
            let mut v = serde_json::to_value(c.redacted()).unwrap_or_default();
            v["target"] = json!(c.target_label());
            v
        })
        .collect();
    Ok(Json(json!({
        "items": items,
        "secretsAvailable": state.config.secret_key.is_some(),
        "timestamp": Utc::now(),
    })))
}

pub async fn get_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<NotificationChannel>, AppError> {
    stored(&state, &id)
        .await?
        .map(|c| Json(c.redacted()))
        .ok_or_else(|| AppError::NotFound(format!("notification channel not found: {id}")))
}

pub async fn create_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<NotificationChannel>,
) -> Result<Json<NotificationChannel>, AppError> {
    if body.id.trim().is_empty() {
        body.id = Uuid::new_v4().to_string();
    }
    if stored(&state, &body.id).await?.is_some() {
        return Err(AppError::Conflict(format!(
            "notification channel {} already exists",
            body.id
        )));
    }
    let channel = prepare_for_store(body, None, state.config.secret_key.as_ref())?;
    rqlite(&state)?
        .put_json(KIND_CHANNEL, &channel.id, &channel)
        .await?;
    log_action(
        actor(&headers),
        "notification.channel.create",
        "success",
        &channel.id,
    );
    Ok(Json(channel.redacted()))
}

pub async fn update_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(mut body): Json<NotificationChannel>,
) -> Result<Json<NotificationChannel>, AppError> {
    let existing = stored(&state, &id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("notification channel not found: {id}")))?;
    body.id = id.clone();
    let channel = prepare_for_store(body, Some(&existing), state.config.secret_key.as_ref())?;
    rqlite(&state)?
        .put_json(KIND_CHANNEL, &id, &channel)
        .await?;
    log_action(
        actor(&headers),
        "notification.channel.update",
        "success",
        &id,
    );
    Ok(Json(channel.redacted()))
}

pub async fn delete_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let rq = rqlite(&state)?;
    let users: Vec<String> = rq
        .list_json::<AlertRule>(KIND_RULE)
        .await?
        .into_iter()
        .filter(|r| r.channel_ids.iter().any(|c| c == &id))
        .map(|r| r.name)
        .collect();
    if !users.is_empty() {
        return Err(AppError::Conflict(format!(
            "channel is used by rule(s): {}. Remove it from those rules first",
            users.join(", ")
        )));
    }
    rq.delete(KIND_CHANNEL, &id).await?;
    log_action(
        actor(&headers),
        "notification.channel.delete",
        "success",
        &id,
    );
    Ok(Json(json!({ "status": true })))
}

/// Sends a sample through a draft channel (not saved). A draft with the id of a saved
/// channel may send `********` to use the stored secrets (same destination rules as save).
pub async fn test_draft_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<NotificationChannel>,
) -> Result<Json<Value>, AppError> {
    let existing = if body.id.trim().is_empty() || state.rqlite.is_none() {
        None
    } else {
        stored(&state, &body.id).await?
    };
    let mut draft = body;
    if draft.name.trim().is_empty() {
        draft.name = "Draft channel".into();
    }
    let channel = prepare_for_store(draft, existing.as_ref(), state.config.secret_key.as_ref())?;
    let result = crate::notify::send_test(&state, channel).await;
    log_action(
        actor(&headers),
        "notification.channel.test",
        &result.outcome,
        &result.channel_name,
    );
    Ok(Json(json!(result)))
}

pub async fn test_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let channel = stored(&state, &id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("notification channel not found: {id}")))?;
    let result = crate::notify::send_test(&state, channel).await;
    log_action(
        actor(&headers),
        "notification.channel.test",
        &result.outcome,
        &id,
    );
    Ok(Json(json!(result)))
}

#[derive(Debug, Deserialize)]
pub struct DeliveriesQuery {
    #[serde(default = "default_limit")]
    pub limit: usize,
    pub channel_id: Option<String>,
    pub rule_id: Option<String>,
    pub outcome: Option<String>,
}

fn default_limit() -> usize {
    200
}

pub async fn list_deliveries(
    State(state): State<AppState>,
    Query(q): Query<DeliveriesQuery>,
) -> Result<Json<Value>, AppError> {
    let f = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let items = crate::notify::list_deliveries(
        &state,
        q.limit,
        f(&q.channel_id).as_deref(),
        f(&q.rule_id).as_deref(),
        f(&q.outcome).as_deref(),
    )
    .await?;
    Ok(Json(
        json!({ "items": items, "count": items.len(), "timestamp": Utc::now() }),
    ))
}
