//! HTTP client for the cdc-pgloader sidecar.

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::{PgLoaderProfile, SchemaCompareConn};
use crate::tool_sidecar::SidecarAccess;
use reqwest::Client;
use serde_json::{json, Value};

const SOURCE_TYPES: &[&str] = &["mysql", "mariadb", "mssql", "sqlserver", "sqlite", "csv"];

pub fn is_pgloader_source(db_type: &str) -> bool {
    SOURCE_TYPES.contains(&db_type.trim().to_ascii_lowercase().as_str())
}

pub fn is_postgres(db_type: &str) -> bool {
    matches!(
        db_type.trim().to_ascii_lowercase().as_str(),
        "postgres" | "postgresql" | "pg"
    )
}

pub fn require_pgloader_pair(
    source: &SchemaCompareConn,
    target: &SchemaCompareConn,
) -> Result<(), AppError> {
    if !is_pgloader_source(&source.db_type) {
        return Err(AppError::BadRequest(
            "pgloader source must be MySQL, SQL Server, SQLite, or CSV".into(),
        ));
    }
    if !is_postgres(&target.db_type) {
        return Err(AppError::BadRequest(
            "pgloader target must be PostgreSQL".into(),
        ));
    }
    if matches!(
        source.db_type.trim().to_ascii_lowercase().as_str(),
        "sqlite" | "csv"
    ) && source.database.trim().is_empty()
        && source.host.trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "SQLite/CSV source needs a file path in database".into(),
        ));
    }
    Ok(())
}

pub fn profile_run_payload(profile: &PgLoaderProfile) -> Value {
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
        base_url: cfg.pgloader_url.as_deref().ok_or_else(|| {
            AppError::BadRequest("pgloader sidecar is not configured (ORCH_PGLOADER_URL)".into())
        })?,
        bearer_token: cfg.pgloader_token.as_deref(),
        tool: "pgloader",
        token_env: "ORCH_PGLOADER_TOKEN",
        extra_bad_request: &[],
    })
}

crate::sidecar_v1_handlers!(access);

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(db_type: &str, database: &str) -> SchemaCompareConn {
        SchemaCompareConn {
            tunnel_id: None,
            db_type: db_type.into(),
            host: "src".into(),
            port: "3306".into(),
            database: database.into(),
            username: "u".into(),
            password: "p".into(),
            schema: String::new(),
        }
    }

    #[test]
    fn accepts_mysql_to_postgres() {
        let target = SchemaCompareConn {
            tunnel_id: None,
            db_type: "postgres".into(),
            host: "tgt".into(),
            port: "5432".into(),
            database: "demo".into(),
            username: "u".into(),
            password: "p".into(),
            schema: String::new(),
        };
        assert!(require_pgloader_pair(&conn("mysql", "shop"), &target).is_ok());
        assert!(require_pgloader_pair(&conn("postgres", "shop"), &target).is_err());
    }
}
