//! `/cdc/latency`, `/cdc/profiles/{id}/latency`, `/cdc/profiles/{id}/latency-probe` (ADR 0016).

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit::log_action;
use crate::error::AppError;
use crate::latency::probe;
use crate::models::ProfileStatus;
use crate::AppState;

pub async fn list_latency(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "items": state.latency.current_all().await, "timestamp": Utc::now() }))
}

#[derive(Debug, Deserialize)]
pub struct RangeQuery {
    #[serde(default = "default_range")]
    pub range: String,
}

fn default_range() -> String {
    "1h".into()
}

pub async fn profile_latency(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<RangeQuery>,
) -> Result<Json<Value>, AppError> {
    let profile = state
        .profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("profile not found: {id}")))?;
    let range = match q.range.as_str() {
        r @ ("1h" | "6h" | "24h" | "7d" | "30d") => r,
        other => {
            return Err(AppError::BadRequest(format!(
                "range must be 1h, 6h, 24h, 7d or 30d (got {other})"
            )))
        }
    };
    let (points, resolution) = crate::latency::history(&state, id, range).await;
    Ok(Json(json!({
        "profileId": id,
        "name": profile.name,
        "range": range,
        "resolution": resolution,
        "current": state.latency.current(id).await,
        "probe": {
            "enabled": profile.latency_probe.enabled,
            "intervalSecs": profile.latency_probe.interval_secs,
            "unsupportedReason": probe::unsupported_reason(&profile),
        },
        "points": points,
        "timestamp": Utc::now(),
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeRequest {
    pub enabled: bool,
    #[serde(default)]
    pub interval_secs: Option<u32>,
}

/// Turns the heartbeat probe on or off. On: creates the source table and publication entry
/// first. A running pipeline must be restarted so the connectors capture and route the
/// heartbeat table (`restartRequired`).
pub async fn set_latency_probe(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<ProbeRequest>,
) -> Result<Json<Value>, AppError> {
    let mut profile = state
        .profiles
        .get(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("profile not found: {id}")))?;
    let was = profile.latency_probe.enabled;
    let mut notes = Vec::new();
    if body.enabled {
        notes = probe::setup_source(&profile).await?;
    } else if was {
        probe::remove_row(&profile).await;
    }
    profile.latency_probe.enabled = body.enabled;
    if let Some(s) = body.interval_secs {
        profile.latency_probe.interval_secs = s.clamp(5, 300);
    }
    profile.updated_at = Utc::now();
    state.profiles.upsert(&profile).await?;
    let actor = headers
        .get("x-user")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("system");
    log_action(
        actor,
        if body.enabled {
            "profile.latency_probe.enable"
        } else {
            "profile.latency_probe.disable"
        },
        "success",
        &id.to_string(),
    );
    let restart_required = was != body.enabled && profile.status == ProfileStatus::Running;
    Ok(Json(json!({
        "enabled": profile.latency_probe.enabled,
        "intervalSecs": profile.latency_probe.interval_secs,
        "restartRequired": restart_required,
        "notes": notes,
    })))
}
