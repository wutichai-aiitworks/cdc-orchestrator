//! Discovery + contract for optional migration sidecars (pgcopydb, RedisShake).

use crate::config::AppConfig;
use crate::error::AppError;
use crate::mongocopy;
use crate::pgcopy;
use crate::pgloader;
use crate::redisshake;
use chrono::Utc;
use reqwest::Client;
use serde_json::{json, Value};

pub const CONTRACT_VERSION: &str = "1";

fn conn_fields(db_type: &str, default_port: &str, include_database: bool) -> Value {
    let mut fields = vec![
        json!({"key": "type", "kind": "string", "required": true, "default": db_type, "enum": [db_type]}),
        json!({"key": "host", "kind": "string", "required": true}),
        json!({"key": "port", "kind": "string", "required": false, "default": default_port}),
        json!({"key": "username", "kind": "string", "required": false, "default": ""}),
        json!({"key": "password", "kind": "string", "required": false, "default": "", "secret": true}),
    ];
    if include_database {
        fields.push(json!({"key": "database", "kind": "string", "required": true}));
        fields.push(json!({"key": "schema", "kind": "string", "required": false, "default": ""}));
    }
    json!(fields)
}

fn operations(tool: &str) -> Value {
    json!([
        {"method": "GET", "path": format!("/cdc/{tool}/discover"), "purpose": "runtime + wiring discovery"},
        {"method": "GET", "path": format!("/cdc/{tool}/contract"), "purpose": "profile schema, defaults, API map"},
        {"method": "GET", "path": format!("/cdc/{tool}/capability"), "purpose": "sidecar capability passthrough"},
        {"method": "GET", "path": format!("/cdc/{tool}/profiles"), "purpose": "list saved profiles"},
        {"method": "POST", "path": format!("/cdc/{tool}/profiles"), "purpose": "create profile"},
        {"method": "GET", "path": format!("/cdc/{tool}/profiles/{{id}}"), "purpose": "get profile"},
        {"method": "PUT", "path": format!("/cdc/{tool}/profiles/{{id}}"), "purpose": "update profile"},
        {"method": "DELETE", "path": format!("/cdc/{tool}/profiles/{{id}}"), "purpose": "delete profile"},
        {"method": "POST", "path": format!("/cdc/{tool}/profiles/{{id}}/run"), "purpose": "start job from profile"},
        {"method": "GET", "path": format!("/cdc/{tool}/jobs"), "purpose": "list jobs"},
        {"method": "GET", "path": format!("/cdc/{tool}/jobs/{{job_id}}"), "purpose": "job status"},
        {"method": "GET", "path": format!("/cdc/{tool}/jobs/{{job_id}}/logs"), "purpose": "job logs"},
        {"method": "POST", "path": format!("/cdc/{tool}/jobs/{{job_id}}/stop"), "purpose": "stop job"},
        {"method": "POST", "path": format!("/cdc/{tool}/validate"), "purpose": "lightweight connection field check"},
    ])
}

pub fn tools_catalog(cfg: &AppConfig) -> Value {
    json!({
        "contract_version": CONTRACT_VERSION,
        "timestamp": Utc::now(),
        "tools": [
            {
                "id": "pgcopy",
                "title": "Postgres copy (pgcopydb)",
                "enabled": cfg.pgcopy_url.is_some(),
                "discover_path": "/cdc/pgcopy/discover",
                "contract_path": "/cdc/pgcopy/contract",
                "ui_path": "/pgcopy",
                "orchestrator_env": ["ORCH_PGCOPY_URL", "ORCH_PGCOPY_TOKEN"],
            },
            {
                "id": "redisshake",
                "title": "Redis sync (RedisShake)",
                "enabled": cfg.redis_shake_url.is_some(),
                "discover_path": "/cdc/redisshake/discover",
                "contract_path": "/cdc/redisshake/contract",
                "ui_path": "/redisshake",
                "orchestrator_env": ["ORCH_REDIS_SHAKE_URL", "ORCH_REDIS_SHAKE_TOKEN"],
            },
            {
                "id": "pgloader",
                "title": "Load to Postgres (pgloader)",
                "enabled": cfg.pgloader_url.is_some(),
                "discover_path": "/cdc/pgloader/discover",
                "contract_path": "/cdc/pgloader/contract",
                "ui_path": "/pgloader",
                "orchestrator_env": ["ORCH_PGLOADER_URL", "ORCH_PGLOADER_TOKEN"],
            },
            {
                "id": "mongocopy",
                "title": "Mongo copy (mongodump/mongorestore)",
                "enabled": cfg.mongocopy_url.is_some(),
                "discover_path": "/cdc/mongocopy/discover",
                "contract_path": "/cdc/mongocopy/contract",
                "ui_path": "/mongocopy",
                "orchestrator_env": ["ORCH_MONGOCOPY_URL", "ORCH_MONGOCOPY_TOKEN"],
            },
        ],
    })
}

pub async fn sidecar_discover(
    cfg: &AppConfig,
    client: &Client,
    tool: &str,
) -> Result<Option<Value>, AppError> {
    let out = match tool {
        "pgcopy" => {
            if cfg.pgcopy_url.is_none() {
                None
            } else {
                Some(pgcopy::discover(cfg, client).await?)
            }
        }
        "redisshake" => {
            if cfg.redis_shake_url.is_none() {
                None
            } else {
                Some(redisshake::discover(cfg, client).await?)
            }
        }
        "pgloader" => {
            if cfg.pgloader_url.is_none() {
                None
            } else {
                Some(pgloader::discover(cfg, client).await?)
            }
        }
        "mongocopy" => {
            if cfg.mongocopy_url.is_none() {
                None
            } else {
                Some(mongocopy::discover(cfg, client).await?)
            }
        }
        _ => return Err(AppError::NotFound(format!("unknown migration tool {tool}"))),
    };
    Ok(out)
}

pub async fn pgcopy_discover(cfg: &AppConfig, client: &Client) -> Result<Value, AppError> {
    let sidecar = sidecar_discover(cfg, client, "pgcopy").await?;
    Ok(json!({
        "contract_version": CONTRACT_VERSION,
        "id": "pgcopy",
        "enabled": cfg.pgcopy_url.is_some(),
        "contract_path": "/cdc/pgcopy/contract",
        "sidecar": sidecar,
        "minimal_profile_example": pgcopy_contract_body()["minimal_profile_example"],
    }))
}

pub async fn redisshake_discover(cfg: &AppConfig, client: &Client) -> Result<Value, AppError> {
    let sidecar = sidecar_discover(cfg, client, "redisshake").await?;
    Ok(json!({
        "contract_version": CONTRACT_VERSION,
        "id": "redisshake",
        "enabled": cfg.redis_shake_url.is_some(),
        "contract_path": "/cdc/redisshake/contract",
        "sidecar": sidecar,
        "minimal_profile_example": redisshake_contract_body()["minimal_profile_example"],
    }))
}

fn pgcopy_contract_body() -> Value {
    json!({
        "contract_version": CONTRACT_VERSION,
        "id": "pgcopy",
        "title": "Postgres copy (pgcopydb clone)",
        "source_db_types": ["postgres", "postgresql", "pg"],
        "target_db_types": ["postgres", "postgresql", "pg"],
        "connection": {
            "source": conn_fields("postgres", "5432", true),
            "target": conn_fields("postgres", "5432", true),
        },
        "profile_fields": [
            {"key": "name", "kind": "string", "required": true},
            {"key": "source", "kind": "connection", "required": true},
            {"key": "target", "kind": "connection", "required": true},
            {"key": "include_tables", "kind": "string[]", "required": false, "default": [], "hint": "schema.table per line; empty = full database"},
            {"key": "options", "kind": "object", "required": false, "default": {}},
        ],
        "options": {
            "booleans": [
                "drop_if_exists", "roles", "no_role_passwords", "no_owner", "no_acl", "no_comments",
                "no_tablespaces", "skip_large_objects", "skip_extensions", "skip_ext_comments",
                "skip_collations", "skip_vacuum", "skip_analyze", "skip_db_properties",
                "skip_split_by_ctid", "fail_fast", "restart", "resume", "not_consistent", "follow",
                "create_slot", "wal2json_numeric_as_string", "use_copy_binary", "all_databases",
                "estimate_table_sizes",
            ],
            "integers": [
                "table_jobs", "index_jobs", "restore_jobs", "large_objects_jobs",
                "split_tables_larger_than", "split_max_parts",
            ],
            "strings": [
                "plugin", "slot_name", "origin", "endpos", "snapshot", "requirements", "filters_file",
            ],
            "defaults": {
                "table_jobs": 4,
                "index_jobs": 2,
                "drop_if_exists": false,
                "follow": false,
                "create_slot": false,
            },
        },
        "operations": operations("pgcopy"),
        "minimal_profile_example": {
            "name": "demo-clone",
            "source": {"type": "postgres", "host": "cdc-postgres-source", "port": "5432", "database": "demo", "username": "demo", "password": "demo"},
            "target": {"type": "postgres", "host": "cdc-postgres-target", "port": "5432", "database": "demo", "username": "demo", "password": "demo"},
            "include_tables": [],
            "options": {"table_jobs": 4, "index_jobs": 2},
        },
    })
}

fn redisshake_contract_body() -> Value {
    json!({
        "contract_version": CONTRACT_VERSION,
        "id": "redisshake",
        "title": "Redis sync (RedisShake sync_reader → redis_writer)",
        "source_db_types": ["redis", "valkey", "tair"],
        "target_db_types": ["redis", "valkey", "tair"],
        "connection": {
            "source": conn_fields("redis", "6379", false),
            "target": conn_fields("redis", "6379", false),
        },
        "profile_fields": [
            {"key": "name", "kind": "string", "required": true},
            {"key": "source", "kind": "connection", "required": true},
            {"key": "target", "kind": "connection", "required": true},
            {"key": "options", "kind": "object", "required": false, "default": {}},
        ],
        "options": {
            "booleans": [
                "source_cluster", "target_cluster", "source_tls", "target_tls",
                "sync_rdb", "sync_aof", "prefer_replica", "try_diskless", "off_reply",
            ],
            "strings": ["rdb_restore_command_behavior"],
            "enum": {
                "rdb_restore_command_behavior": ["panic", "rewrite", "skip"],
            },
            "defaults": {
                "sync_rdb": true,
                "sync_aof": true,
                "rdb_restore_command_behavior": "panic",
            },
            "hints": {
                "sync_aof": "Disable for one-time full copy; job exits after RDB phase.",
                "sync_rdb": "Full snapshot; usually leave enabled.",
            },
        },
        "sidecar_limits_env": [
            "REDIS_SHAKE_MAX_CONCURRENT_JOBS",
            "REDIS_SHAKE_RETAIN_JOBS",
            "REDIS_SHAKE_CLEANUP_STAGING",
        ],
        "operations": operations("redisshake"),
        "minimal_profile_example": {
            "name": "demo-redis-sync",
            "source": {"type": "redis", "host": "redis-source", "port": "6379", "username": "", "password": ""},
            "target": {"type": "redis", "host": "redis-target", "port": "6379", "username": "", "password": ""},
            "options": {"sync_rdb": true, "sync_aof": false, "rdb_restore_command_behavior": "rewrite"},
        },
    })
}

pub fn pgcopy_contract(_cfg: &AppConfig) -> Value {
    pgcopy_contract_body()
}

pub fn redisshake_contract(_cfg: &AppConfig) -> Value {
    redisshake_contract_body()
}

pub async fn pgloader_discover(cfg: &AppConfig, client: &Client) -> Result<Value, AppError> {
    let sidecar = sidecar_discover(cfg, client, "pgloader").await?;
    Ok(json!({
        "contract_version": CONTRACT_VERSION,
        "id": "pgloader",
        "enabled": cfg.pgloader_url.is_some(),
        "contract_path": "/cdc/pgloader/contract",
        "sidecar": sidecar,
        "minimal_profile_example": pgloader_contract_body()["minimal_profile_example"],
    }))
}

pub async fn mongocopy_discover(cfg: &AppConfig, client: &Client) -> Result<Value, AppError> {
    let sidecar = sidecar_discover(cfg, client, "mongocopy").await?;
    Ok(json!({
        "contract_version": CONTRACT_VERSION,
        "id": "mongocopy",
        "enabled": cfg.mongocopy_url.is_some(),
        "contract_path": "/cdc/mongocopy/contract",
        "sidecar": sidecar,
        "minimal_profile_example": mongocopy_contract_body()["minimal_profile_example"],
    }))
}

fn pgloader_contract_body() -> Value {
    json!({
        "contract_version": CONTRACT_VERSION,
        "id": "pgloader",
        "title": "Load to Postgres (pgloader)",
        "source_db_types": ["mysql", "mariadb", "mssql", "sqlserver", "sqlite", "csv"],
        "target_db_types": ["postgres", "postgresql", "pg"],
        "connection": {
            "source": conn_fields("mysql", "3306", true),
            "target": conn_fields("postgres", "5432", true),
        },
        "profile_fields": [
            {"key": "name", "kind": "string", "required": true},
            {"key": "source", "kind": "connection", "required": true},
            {"key": "target", "kind": "connection", "required": true},
            {"key": "include_tables", "kind": "string[]", "required": false, "default": []},
            {"key": "options", "kind": "object", "required": false, "default": {}},
        ],
        "options": {
            "booleans": ["drop_if_exists", "create_tables", "create_indexes", "reset_sequences", "truncate"],
            "integers": ["workers", "concurrency", "csv_skip_header"],
            "strings": ["target_table", "csv_delimiter"],
            "defaults": {
                "create_tables": true,
                "create_indexes": true,
                "reset_sequences": true,
                "drop_if_exists": false,
                "csv_delimiter": ",",
                "csv_skip_header": 1,
            },
        },
        "operations": operations("pgloader"),
        "minimal_profile_example": {
            "name": "mysql-to-pg",
            "source": {"type": "mysql", "host": "mysql-source", "port": "3306", "database": "shop", "username": "root", "password": ""},
            "target": {"type": "postgres", "host": "cdc-postgres-target", "port": "5432", "database": "demo", "username": "demo", "password": "demo"},
            "include_tables": [],
            "options": {"create_tables": true, "create_indexes": true},
        },
    })
}

fn mongocopy_contract_body() -> Value {
    json!({
        "contract_version": CONTRACT_VERSION,
        "id": "mongocopy",
        "title": "Mongo copy (mongodump → mongorestore)",
        "source_db_types": ["mongo", "mongodb"],
        "target_db_types": ["mongo", "mongodb"],
        "connection": {
            "source": conn_fields("mongo", "27017", true),
            "target": conn_fields("mongo", "27017", true),
        },
        "profile_fields": [
            {"key": "name", "kind": "string", "required": true},
            {"key": "source", "kind": "connection", "required": true},
            {"key": "target", "kind": "connection", "required": true},
            {"key": "include_collections", "kind": "string[]", "required": false, "default": []},
            {"key": "options", "kind": "object", "required": false, "default": {}},
        ],
        "options": {
            "booleans": ["drop", "gzip"],
            "strings": ["auth_source"],
            "defaults": {"drop": false, "gzip": true},
        },
        "operations": operations("mongocopy"),
        "minimal_profile_example": {
            "name": "mongo-clone",
            "source": {"type": "mongo", "host": "mongo-source", "port": "27017", "database": "app", "username": "", "password": ""},
            "target": {"type": "mongo", "host": "mongo-target", "port": "27017", "database": "app", "username": "", "password": ""},
            "include_collections": [],
            "options": {"gzip": true, "drop": false},
        },
    })
}

pub fn pgloader_contract(_cfg: &AppConfig) -> Value {
    pgloader_contract_body()
}

pub fn mongocopy_contract(_cfg: &AppConfig) -> Value {
    mongocopy_contract_body()
}

pub fn contract_for(tool: &str, cfg: &AppConfig) -> Result<Value, AppError> {
    match tool {
        "pgcopy" => Ok(pgcopy_contract(cfg)),
        "redisshake" => Ok(redisshake_contract(cfg)),
        "pgloader" => Ok(pgloader_contract(cfg)),
        "mongocopy" => Ok(mongocopy_contract(cfg)),
        _ => Err(AppError::NotFound(format!("unknown migration tool {tool}"))),
    }
}
