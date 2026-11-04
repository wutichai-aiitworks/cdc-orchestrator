//! HTTP client for the cdc-redisshake sidecar.

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::{RedisShakeProfile, SchemaCompareConn};
use crate::tool_sidecar::SidecarAccess;
use reqwest::Client;
use serde_json::{json, Value};

pub fn is_redis(db_type: &str) -> bool {
    matches!(
        db_type.trim().to_ascii_lowercase().as_str(),
        "redis" | "valkey" | "tair"
    )
}

pub fn require_redis_pair(
    source: &SchemaCompareConn,
    target: &SchemaCompareConn,
) -> Result<(), AppError> {
    if !is_redis(&source.db_type) {
        return Err(AppError::BadRequest(
            "RedisShake source must be Redis (or Valkey/Tair)".into(),
        ));
    }
    if !is_redis(&target.db_type) {
        return Err(AppError::BadRequest(
            "RedisShake target must be Redis (or Valkey/Tair)".into(),
        ));
    }
    Ok(())
}

pub fn profile_run_payload(profile: &RedisShakeProfile) -> Value {
    json!({
        // One job per run: the sidecar assigns the job id; this ties the run to its transfer.
        "profile_id": profile.id.to_string(),
        "name": profile.name,
        "source": profile.source,
        "target": profile.target,
        "options": profile.options,
    })
}

fn access<'a>(cfg: &'a AppConfig, client: &'a Client) -> Result<SidecarAccess<'a>, AppError> {
    Ok(SidecarAccess {
        client,
        base_url: cfg.redis_shake_url.as_deref().ok_or_else(|| {
            AppError::BadRequest(
                "RedisShake sidecar is not configured (ORCH_REDIS_SHAKE_URL)".into(),
            )
        })?,
        bearer_token: cfg.redis_shake_token.as_deref(),
        tool: "redisshake",
        token_env: "ORCH_REDIS_SHAKE_TOKEN",
        extra_bad_request: &[],
    })
}

crate::sidecar_v1_handlers!(access);
