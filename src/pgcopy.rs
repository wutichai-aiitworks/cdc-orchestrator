//! HTTP client for the cdc-pgcopydb sidecar.

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::{PgCopyProfile, SchemaCompareConn};
use crate::tool_sidecar::SidecarAccess;
use reqwest::Client;
use serde_json::{json, Value};

pub fn is_postgres(db_type: &str) -> bool {
    matches!(
        db_type.trim().to_ascii_lowercase().as_str(),
        "postgres" | "postgresql" | "pg"
    )
}

pub fn require_postgres_pair(
    source: &SchemaCompareConn,
    target: &SchemaCompareConn,
) -> Result<(), AppError> {
    if !is_postgres(&source.db_type) {
        return Err(AppError::BadRequest(
            "pgcopydb source must be PostgreSQL".into(),
        ));
    }
    if !is_postgres(&target.db_type) {
        return Err(AppError::BadRequest(
            "pgcopydb target must be PostgreSQL".into(),
        ));
    }
    Ok(())
}

pub fn profile_run_payload(profile: &PgCopyProfile) -> Value {
    json!({
        // One job per run: the sidecar assigns the job id; this ties the run to its transfer.
        "profile_id": profile.id.to_string(),
        "name": profile.name,
        "source": profile.source,
        "target": profile.target,
        "include_tables": profile.include_tables,
        "options": profile.options,
    })
}

fn access<'a>(cfg: &'a AppConfig, client: &'a Client) -> Result<SidecarAccess<'a>, AppError> {
    Ok(SidecarAccess {
        client,
        base_url: cfg.pgcopy_url.as_deref().ok_or_else(|| {
            AppError::BadRequest("pgcopydb sidecar is not configured (ORCH_PGCOPY_URL)".into())
        })?,
        bearer_token: cfg.pgcopy_token.as_deref(),
        tool: "pgcopy",
        token_env: "ORCH_PGCOPY_TOKEN",
        extra_bad_request: &[],
    })
}

crate::sidecar_v1_handlers!(access);
