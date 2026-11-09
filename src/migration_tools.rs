//! Discovery + contract for optional migration sidecars (pgcopydb, RedisShake).

use crate::config::AppConfig;
use crate::error::AppError;
use crate::mongocopy;
use crate::pgcopy;
use crate::pgdump;
use crate::pgloader;
use crate::redisshake;
use crate::rsync;
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
            {
                "id": "pgdump",
                "title": "Postgres dump (pg_dump / pg_restore)",
                "enabled": cfg.pgdump_url.is_some(),
                "discover_path": "/cdc/pgdump/discover",
                "contract_path": "/cdc/pgdump/contract",
                "ui_path": "/pgdump",
                "orchestrator_env": ["ORCH_PGDUMP_URL", "ORCH_PGDUMP_TOKEN"],
            },
            {
                "id": "rsync",
                "title": "File sync (rsync over SSH)",
                "enabled": cfg.rsync_url.is_some(),
                "discover_path": "/cdc/rsync/discover",
                "contract_path": "/cdc/rsync/contract",
                "ui_path": "/rsync",
                "orchestrator_env": ["ORCH_RSYNC_URL", "ORCH_RSYNC_TOKEN"],
            },
        ],
    })
}

async fn discover_if_enabled(
    enabled: bool,
    discover: impl std::future::Future<Output = Result<Value, AppError>>,
) -> Result<Option<Value>, AppError> {
    if enabled {
        Ok(Some(discover.await?))
    } else {
        Ok(None)
    }
}

pub async fn sidecar_discover(
    cfg: &AppConfig,
    client: &Client,
    tool: &str,
) -> Result<Option<Value>, AppError> {
    match tool {
        "pgcopy" => {
            discover_if_enabled(cfg.pgcopy_url.is_some(), pgcopy::discover(cfg, client)).await
        }
        "redisshake" => {
            discover_if_enabled(
                cfg.redis_shake_url.is_some(),
                redisshake::discover(cfg, client),
            )
            .await
        }
        "pgloader" => {
            discover_if_enabled(cfg.pgloader_url.is_some(), pgloader::discover(cfg, client)).await
        }
        "mongocopy" => {
            discover_if_enabled(
                cfg.mongocopy_url.is_some(),
                mongocopy::discover(cfg, client),
            )
            .await
        }
        "pgdump" => {
            discover_if_enabled(cfg.pgdump_url.is_some(), pgdump::discover(cfg, client)).await
        }
        "rsync" => discover_if_enabled(cfg.rsync_url.is_some(), rsync::discover(cfg, client)).await,
        _ => Err(AppError::NotFound(format!("unknown migration tool {tool}"))),
    }
}

async fn migration_tool_discover(
    cfg: &AppConfig,
    client: &Client,
    tool: &str,
    contract_path: &str,
    enabled: bool,
    minimal_example: Value,
) -> Result<Value, AppError> {
    let sidecar = sidecar_discover(cfg, client, tool).await?;
    Ok(json!({
        "contract_version": CONTRACT_VERSION,
        "id": tool,
        "enabled": enabled,
        "contract_path": contract_path,
        "sidecar": sidecar,
        "minimal_profile_example": minimal_example,
    }))
}

pub async fn pgcopy_discover(cfg: &AppConfig, client: &Client) -> Result<Value, AppError> {
    migration_tool_discover(
        cfg,
        client,
        "pgcopy",
        "/cdc/pgcopy/contract",
        cfg.pgcopy_url.is_some(),
        pgcopy_contract_body()["minimal_profile_example"].clone(),
    )
    .await
}

pub async fn redisshake_discover(cfg: &AppConfig, client: &Client) -> Result<Value, AppError> {
    migration_tool_discover(
        cfg,
        client,
        "redisshake",
        "/cdc/redisshake/contract",
        cfg.redis_shake_url.is_some(),
        redisshake_contract_body()["minimal_profile_example"].clone(),
    )
    .await
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
                "sync_rdb", "sync_aof", "prefer_replica", "try_diskless", "off_reply", "empty_target",
            ],
            "strings": ["rdb_restore_command_behavior"],
            "integer_lists": ["source_dbs"],
            "integers": ["target_db"],
            "enum": {
                "rdb_restore_command_behavior": ["panic", "rewrite", "skip"],
            },
            "defaults": {
                "sync_rdb": true,
                "sync_aof": true,
                "rdb_restore_command_behavior": "panic",
                "source_dbs": [],
                "target_db": null,
            },
            "hints": {
                "sync_aof": "Disable for one-time full copy; job exits after RDB phase.",
                "sync_rdb": "Full snapshot; usually leave enabled.",
                "source_dbs": "Source database numbers to sync (RedisShake allow_db). Empty = all. Cluster sources: only 0.",
                "target_db": "Write into this target database instead of the same number. Needs exactly one source_dbs entry.",
            },
            "run_modes": {
                "query": "POST /cdc/redisshake/profiles/{id}/run?mode=once|cdc[&snapshot=true|false]",
                "once": "One-time sync: full copy, then the run ends (sync_rdb=true, sync_aof=false).",
                "cdc": "CDC: follow source changes until stopped (sync_aof=true); snapshot=true (default from the profile) copies everything first, false streams changes from now only.",
                "default": "Without mode, the profile's sync_rdb/sync_aof decide.",
                "empty_target": "&empty_target=true|false: delete the target's keys right before the full copy (FLUSHDB on the synced databases, FLUSHALL when syncing all). Refused with mode=cdc&snapshot=false.",
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
    migration_tool_discover(
        cfg,
        client,
        "pgloader",
        "/cdc/pgloader/contract",
        cfg.pgloader_url.is_some(),
        pgloader_contract_body()["minimal_profile_example"].clone(),
    )
    .await
}

pub async fn mongocopy_discover(cfg: &AppConfig, client: &Client) -> Result<Value, AppError> {
    migration_tool_discover(
        cfg,
        client,
        "mongocopy",
        "/cdc/mongocopy/contract",
        cfg.mongocopy_url.is_some(),
        mongocopy_contract_body()["minimal_profile_example"].clone(),
    )
    .await
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
        "pgdump" => Ok(pgdump_contract(cfg)),
        "rsync" => Ok(rsync_contract(cfg)),
        _ => Err(AppError::NotFound(format!("unknown migration tool {tool}"))),
    }
}

fn pgdump_operations() -> Value {
    let mut ops = operations("pgdump");
    if let Some(list) = ops.as_array_mut() {
        list.push(json!({"method": "GET", "path": "/cdc/pgdump/jobs/{job_id}/file", "purpose": "download the dump file (streamed)"}));
        list.push(json!({"method": "DELETE", "path": "/cdc/pgdump/jobs/{job_id}", "purpose": "delete a finished job and its file"}));
        list.push(json!({"method": "POST", "path": "/cdc/pgdump/jobs/{job_id}/send", "purpose": "upload a finished job's file to one ad-hoc SFTP server (body {destination, profile_id?})"}));
    }
    ops
}

fn pgdump_contract_body() -> Value {
    json!({
        "contract_version": CONTRACT_VERSION,
        "id": "pgdump",
        "title": "Postgres dump (pg_dump / pg_restore)",
        "source_db_types": ["postgres", "postgresql", "pg"],
        "target_db_types": ["postgres", "postgresql", "pg"],
        "modes": {
            "dump": "pg_dump the source to a file kept on the sidecar volume; download via /jobs/{job_id}/file",
            "dump_restore": "pg_dump -Fc the source, then pg_restore into target (seed a replica before CDC)",
        },
        "connection": {
            "source": conn_fields("postgres", "5432", true),
            "target": conn_fields("postgres", "5432", true),
        },
        "profile_fields": [
            {"key": "name", "kind": "string", "required": true},
            {"key": "mode", "kind": "string", "required": false, "default": "dump", "enum": ["dump", "dump_restore"]},
            {"key": "source", "kind": "connection", "required": true},
            {"key": "target", "kind": "connection", "required": false, "note": "required for dump_restore"},
            {"key": "include_tables", "kind": "string[]", "required": false, "default": [],
             "note": "pg_dump --table patterns; schemas they need are created on the target before restore"},
            {"key": "options", "kind": "object", "required": false, "default": {}},
            {"key": "destinations", "kind": "sftp_destination[]", "required": false, "default": [], "max": 5,
             "note": "uploaded after the dump: <file>.part then rename, plus <file>.sha256; local copy removed when all succeed unless options.keep_local"},
        ],
        "sftp_destination_fields": [
            {"key": "name", "kind": "string", "required": false},
            {"key": "host", "kind": "string", "required": true},
            {"key": "port", "kind": "string", "required": false, "default": "22"},
            {"key": "username", "kind": "string", "required": true},
            {"key": "auth", "kind": "string", "required": false, "enum": ["key", "password"]},
            {"key": "password", "kind": "string", "required": false, "secret": true},
            {"key": "private_key", "kind": "string", "required": false, "secret": true, "note": "OpenSSH/PEM text"},
            {"key": "passphrase", "kind": "string", "required": false, "secret": true},
            {"key": "remote_dir", "kind": "string", "required": true, "note": "created if missing; relative = under SFTP home"},
            {"key": "host_key_fingerprint", "kind": "string", "required": false, "note": "SHA256:… pin (ssh-keygen -lf); mismatch aborts"},
        ],
        "options": {
            "strings": {"format": ["custom", "plain", "tar"], "source_sslmode": "libpq sslmode", "target_sslmode": "libpq sslmode"},
            "integers": {"compress": "0-9", "lock_wait_timeout_ms": ">=1", "restore_jobs": "1-32"},
            "lists": ["schemas", "exclude_schemas", "exclude_tables", "exclude_table_data"],
            "booleans": ["schema_only", "data_only", "no_owner", "no_privileges", "no_comments",
                         "no_publications", "no_subscriptions", "clean", "if_exists", "create",
                         "serializable_deferrable", "restore_clean", "restore_if_exists",
                         "restore_no_owner", "restore_no_privileges", "restore_single_transaction",
                         "restore_exit_on_error", "keep_local"],
            "defaults": {"format": "custom"},
        },
        "job_statuses": ["starting", "running", "succeeded", "completed_with_errors", "failed", "stopped"],
        "operations": pgdump_operations(),
        "minimal_profile_example": {
            "name": "seed-bookings",
            "mode": "dump_restore",
            "source": {"type": "postgres", "host": "cdc-postgres-source", "port": "5432", "database": "demo", "username": "demo", "password": "demo"},
            "target": {"type": "postgres", "host": "cdc-postgres-target", "port": "5432", "database": "demo", "username": "demo", "password": "demo"},
            "include_tables": [],
            "options": {"schemas": ["bookings"], "no_owner": true, "restore_jobs": 2},
        },
    })
}

pub fn pgdump_contract(_cfg: &AppConfig) -> Value {
    pgdump_contract_body()
}

pub async fn pgdump_discover(cfg: &AppConfig, client: &Client) -> Result<Value, AppError> {
    migration_tool_discover(
        cfg,
        client,
        "pgdump",
        "/cdc/pgdump/contract",
        cfg.pgdump_url.is_some(),
        pgdump_contract_body()["minimal_profile_example"].clone(),
    )
    .await
}

fn rsync_operations() -> Value {
    let mut ops = operations("rsync");
    if let Some(list) = ops.as_array_mut() {
        list.push(json!({"method": "POST", "path": "/cdc/rsync/profiles/{id}/run?dry_run=true", "purpose": "preview: itemized changes, nothing written"}));
        list.push(json!({"method": "POST", "path": "/cdc/rsync/test-route", "purpose": "would a run go direct (source reaches target)?"}));
        list.push(json!({"method": "DELETE", "path": "/cdc/rsync/jobs/{job_id}", "purpose": "delete a finished job"}));
        list.push(json!({"method": "GET", "path": "/cdc/rsync/staging", "purpose": "relay staging copies and sizes"}));
        list.push(json!({"method": "DELETE", "path": "/cdc/rsync/staging/{id}", "purpose": "free a profile's relay staging copy"}));
    }
    ops
}

fn ssh_endpoint_fields() -> Value {
    json!([
        {"key": "name", "kind": "string", "required": false},
        {"key": "host", "kind": "string", "required": true},
        {"key": "port", "kind": "string", "required": false, "default": "22"},
        {"key": "username", "kind": "string", "required": true},
        {"key": "auth", "kind": "string", "required": false, "enum": ["key", "password"]},
        {"key": "password", "kind": "string", "required": false, "secret": true},
        {"key": "private_key", "kind": "string", "required": false, "secret": true, "note": "OpenSSH/PEM text"},
        {"key": "passphrase", "kind": "string", "required": false, "secret": true},
        {"key": "path", "kind": "string", "required": true, "note": "relative = under the login's home; source trailing / copies the contents"},
        {"key": "host_key_fingerprint", "kind": "string", "required": false, "note": "SHA256:… pin (ssh-keygen -lf); mismatch aborts"},
        {"key": "rsync_path", "kind": "string", "required": false, "note": "auto-detected; Homebrew rsync preferred on macOS"},
        {"key": "reach_host", "kind": "string", "required": false, "note": "target only: address the source uses in direct mode"},
        {"key": "reach_port", "kind": "string", "required": false, "note": "target only"},
    ])
}

fn rsync_contract_body() -> Value {
    json!({
        "contract_version": CONTRACT_VERSION,
        "id": "rsync",
        "title": "File sync (rsync over SSH)",
        "source_host_os": ["linux", "darwin"],
        "target_host_os": ["linux", "darwin"],
        "transports": {
            "auto": "direct when the source can log in to the target with the forwarded key, otherwise relay (reason recorded on the job)",
            "direct": "the sidecar SSHes into the source (agent forwarding) and rsync there pushes to the target; data goes source → target once",
            "relay": "the sidecar pulls source → staging volume (kept per profile, incremental), then pushes staging → target",
        },
        "connection": {"source": ssh_endpoint_fields(), "target": ssh_endpoint_fields()},
        "profile_fields": [
            {"key": "name", "kind": "string", "required": true},
            {"key": "source", "kind": "ssh_endpoint", "required": true},
            {"key": "target", "kind": "ssh_endpoint", "required": true, "note": "path is always a directory; created if missing. Direct mode needs auth=key"},
            {"key": "options", "kind": "object", "required": false, "default": {}},
        ],
        "options": {
            "strings": {"transport": ["auto", "direct", "relay"]},
            "integers": {"max_delete": ">=0 (needs delete)", "bwlimit_kbps": ">=1", "io_timeout_seconds": "10-86400"},
            "lists": ["excludes"],
            "booleans": ["delete", "dry_run", "checksum", "compress", "no_owner", "hard_links", "acls", "xattrs",
                         "numeric_ids", "one_file_system", "discard_staging"],
            "defaults": {"transport": "auto", "delete": false},
        },
        "job_statuses": ["starting", "running", "succeeded", "completed_with_errors", "failed", "stopped"],
        "operations": rsync_operations(),
        "minimal_profile_example": {
            "name": "web-assets",
            "source": {"host": "web-01", "username": "deploy", "auth": "key", "private_key": "-----BEGIN OPENSSH PRIVATE KEY-----…", "path": "/srv/www/"},
            "target": {"host": "mac-mini.lan", "username": "backup", "auth": "key", "private_key": "-----BEGIN OPENSSH PRIVATE KEY-----…", "path": "backups/www"},
            "options": {"transport": "auto", "excludes": ["*.log", ".cache/"]},
        },
    })
}

pub fn rsync_contract(_cfg: &AppConfig) -> Value {
    rsync_contract_body()
}

pub async fn rsync_discover(cfg: &AppConfig, client: &Client) -> Result<Value, AppError> {
    migration_tool_discover(
        cfg,
        client,
        "rsync",
        "/cdc/rsync/contract",
        cfg.rsync_url.is_some(),
        rsync_contract_body()["minimal_profile_example"].clone(),
    )
    .await
}
