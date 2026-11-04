//! HTTP client for the cdc-mongocopy sidecar.

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::{MongoCopyProfile, SchemaCompareConn};
use crate::tool_sidecar::SidecarAccess;
use reqwest::Client;
use serde_json::{json, Value};

pub fn is_mongo(db_type: &str) -> bool {
    matches!(
        db_type.trim().to_ascii_lowercase().as_str(),
        "mongo" | "mongodb"
    )
}

pub fn require_mongo_pair(
    source: &SchemaCompareConn,
    target: &SchemaCompareConn,
) -> Result<(), AppError> {
    if !is_mongo(&source.db_type) {
        return Err(AppError::BadRequest(
            "mongocopy source must be MongoDB".into(),
        ));
    }
    if !is_mongo(&target.db_type) {
        return Err(AppError::BadRequest(
            "mongocopy target must be MongoDB".into(),
        ));
    }
    if source.host.trim().is_empty() || target.host.trim().is_empty() {
        return Err(AppError::BadRequest(
            "host is required on source and target".into(),
        ));
    }
    Ok(())
}

pub fn profile_run_payload(profile: &MongoCopyProfile) -> Value {
    json!({
        // One job per run: the sidecar assigns the job id; this ties the run to its transfer.
        "profile_id": profile.id.to_string(),
        "name": profile.name,
        "source": profile.source,
        "target": profile.target,
        "include_collections": profile.include_collections,
        "options": profile.options,
    })
}

fn access<'a>(cfg: &'a AppConfig, client: &'a Client) -> Result<SidecarAccess<'a>, AppError> {
    Ok(SidecarAccess {
        client,
        base_url: cfg.mongocopy_url.as_deref().ok_or_else(|| {
            AppError::BadRequest("mongocopy sidecar is not configured (ORCH_MONGOCOPY_URL)".into())
        })?,
        bearer_token: cfg.mongocopy_token.as_deref(),
        tool: "mongocopy",
        token_env: "ORCH_MONGOCOPY_TOKEN",
        extra_bad_request: &[],
    })
}

crate::sidecar_v1_handlers!(access);
