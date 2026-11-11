//! Compile CDC profiles into Debezium Kafka Connect connector configs and reconcile lifecycle.

use std::collections::HashMap;

use reqwest::Method;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;
use crate::models::{
    CdcProfile, HistoryItem, ProfilePattern, ProfileStatus, SchemaSelectionMode, SinkErrorPolicy,
    StreamPlatform, TableSelectionMode,
};
use crate::AppState;
use chrono::Utc;
use tracing::info;

pub fn connector_name_for(profile: &CdcProfile) -> String {
    if let Some(ref n) = profile.connector_name {
        if !n.trim().is_empty() {
            return n.clone();
        }
    }
    format!("cdc-{}", slug(&profile.name))
}

pub fn sink_connector_name_for(profile: &CdcProfile) -> String {
    if let Some(ref n) = profile.sink_connector_name {
        if !n.trim().is_empty() {
            return n.clone();
        }
    }
    format!("{}-sink", connector_name_for(profile))
}

/// Connectors left behind by earlier trigger experiments. They are not created
/// by current reconcile, so start/stop must delete them or a failed leftover
/// task keeps showing up next to the live pipeline.
pub fn leftover_connector_names(profile: &CdcProfile) -> Vec<String> {
    let base = connector_name_for(profile);
    vec![
        format!("{base}-changelog-src"),
        format!("{base}-initial-load"),
        format!("{base}-join-src"),
        format!("{base}-join-sink"),
    ]
}

pub(crate) fn slug(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

pub(crate) fn topic_prefix_for(profile: &CdcProfile) -> String {
    profile
        .name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .to_ascii_lowercase()
}

pub(crate) fn apply_avro_converters(cfg: &mut HashMap<String, String>, schema_registry_url: &str) {
    let registry = schema_registry_url.trim();
    cfg.insert(
        "key.converter".into(),
        "io.confluent.connect.avro.AvroConverter".into(),
    );
    cfg.insert(
        "value.converter".into(),
        "io.confluent.connect.avro.AvroConverter".into(),
    );
    cfg.insert("key.converter.schema.registry.url".into(), registry.into());
    cfg.insert(
        "value.converter.schema.registry.url".into(),
        registry.into(),
    );
    cfg.insert("key.converter.auto.register.schemas".into(), "true".into());
    cfg.insert(
        "value.converter.auto.register.schemas".into(),
        "true".into(),
    );
    cfg.insert(
        "key.converter.enhanced.avro.schema.support".into(),
        "true".into(),
    );
    cfg.insert(
        "value.converter.enhanced.avro.schema.support".into(),
        "true".into(),
    );
}

pub(crate) fn apply_topic_creation_defaults(cfg: &mut HashMap<String, String>) {
    cfg.insert(
        "topic.creation.default.replication.factor".into(),
        "1".into(),
    );
    cfg.insert("topic.creation.default.partitions".into(), "1".into());
    cfg.insert(
        "topic.creation.default.cleanup.policy".into(),
        "delete".into(),
    );
    cfg.insert(
        "topic.creation.default.retention.ms".into(),
        "604800000".into(),
    );
    cfg.insert(
        "topic.creation.default.segment.bytes".into(),
        "134217728".into(),
    );
}

pub(crate) fn apply_sink_error_policy(cfg: &mut HashMap<String, String>, profile: &CdcProfile) {
    let use_dlq = profile.effective_sink_error_policy() == SinkErrorPolicy::Dlq;
    if !use_dlq {
        return;
    }

    cfg.insert("errors.tolerance".into(), "all".into());
    cfg.insert(
        "errors.deadletterqueue.topic.name".into(),
        format!("{}-dlq", sink_connector_name_for(profile)),
    );
    cfg.insert(
        "errors.deadletterqueue.topic.replication.factor".into(),
        "1".into(),
    );
    cfg.insert(
        "errors.deadletterqueue.context.headers.enable".into(),
        "true".into(),
    );
    cfg.insert("errors.retry.timeout".into(), "60000".into());
    cfg.insert("errors.retry.delay.max.ms".into(), "5000".into());
}

fn schema_registry_url(state: &AppState) -> String {
    state
        .config
        .schema_registry_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("http://cdc-message:8081")
        .to_string()
}

pub fn build_debezium_config(
    profile: &CdcProfile,
    schema_registry_url: &str,
) -> Result<HashMap<String, String>, AppError> {
    let src = &profile.source;
    let db_type = src.source_type.trim().to_ascii_lowercase();
    let host = req(&src.connection, "host")?;
    let port = req(&src.connection, "port")?;
    let database = req(&src.connection, "database")?;
    let username = req(&src.connection, "username")?;
    let password = req(&src.connection, "password")?;
    let server_name = topic_prefix_for(profile);

    let mut cfg = HashMap::new();
    let connector_class = match db_type.as_str() {
        "postgres" | "postgresql" => "io.debezium.connector.postgresql.PostgresConnector",
        "mysql" | "mariadb" => "io.debezium.connector.mysql.MySqlConnector",
        "sqlserver" => "io.debezium.connector.sqlserver.SqlServerConnector",
        "oracle" => "io.debezium.connector.oracle.OracleConnector",
        other => {
            return Err(AppError::BadRequest(format!(
                "unsupported source_type for Debezium: {other}"
            )));
        }
    };
    cfg.insert("connector.class".into(), connector_class.into());
    cfg.insert("tasks.max".into(), "1".into());
    cfg.insert("database.hostname".into(), host.into());
    cfg.insert("database.port".into(), port.into());
    cfg.insert("database.user".into(), username.into());
    cfg.insert("database.password".into(), password.into());
    cfg.insert("topic.prefix".into(), server_name.clone());
    cfg.insert(
        "snapshot.mode".into(),
        if src.capture_mode == crate::models::CaptureMode::SyncOnce {
            "initial_only".into()
        } else {
            "initial".into()
        },
    );

    apply_debezium_dialect(&mut cfg, profile, &db_type, database, &server_name);
    apply_debezium_transforms(&mut cfg, profile);

    if profile.pattern == ProfilePattern::SourceToStream {
        let _platform = profile.stream_platform.unwrap_or(StreamPlatform::Redpanda);
        let _ = _platform;
    }

    apply_topic_creation_defaults(&mut cfg);
    apply_avro_converters(&mut cfg, schema_registry_url);
    Ok(cfg)
}

fn apply_debezium_dialect(
    cfg: &mut HashMap<String, String>,
    profile: &CdcProfile,
    db_type: &str,
    database: &str,
    server_name: &str,
) {
    match db_type {
        "postgres" | "postgresql" => apply_postgres_debezium(cfg, profile, database, server_name),
        "mysql" | "mariadb" => {
            cfg.insert("database.include.list".into(), database.into());
            insert_table_include(cfg, profile);
            cfg.insert(
                "database.server.id".into(),
                format!("{}", (Uuid::new_v4().as_u128() % 1_000_000) + 1),
            );
        }
        "sqlserver" => {
            cfg.insert("database.names".into(), database.into());
            insert_table_include(cfg, profile);
        }
        "oracle" => {
            cfg.insert("database.dbname".into(), database.into());
            insert_table_include(cfg, profile);
        }
        _ => {}
    }
}

fn apply_postgres_debezium(
    cfg: &mut HashMap<String, String>,
    profile: &CdcProfile,
    database: &str,
    server_name: &str,
) {
    let src = &profile.source;
    cfg.insert("database.dbname".into(), database.into());
    cfg.insert(
        "plugin.name".into(),
        src.connection
            .get("plugin")
            .cloned()
            .unwrap_or_else(|| "pgoutput".into()),
    );
    cfg.insert(
        "publication.name".into(),
        src.connection
            .get("publication")
            .cloned()
            .unwrap_or_else(|| "dbz_publication".into()),
    );
    cfg.insert(
        "slot.name".into(),
        src.connection
            .get("slot")
            .cloned()
            .unwrap_or_else(|| format!("cdc_{}", server_name.replace('-', "_"))),
    );
    cfg.insert("publication.autocreate.mode".into(), "disabled".into());
    if src.schema_mode == SchemaSelectionMode::SpecificSchemas && !src.schemas.is_empty() {
        cfg.insert("schema.include.list".into(), src.schemas.join(","));
    }
    insert_table_include(cfg, profile);
    // Explicit overrides only; `reconcile_start` re-applies with catalog-derived keys.
    let _ = apply_message_key_columns(cfg, profile, &HashMap::new());
}

/// Set Debezium `message.key.columns` for source tables without a PRIMARY KEY.
///
/// Without it Debezium emits a null record key and the `record_key` JDBC sink
/// fails the whole task ("primary key mode 'record_key' cannot have null schema").
/// `source.connection.message_key_columns` (Debezium syntax,
/// `schema.table:c1,c2;...`) wins per table; other keyless tables get the key
/// derived from the catalog. A Source→Target pipeline with a keyless table and
/// no derivable key is refused up front instead of crashing the sink later.
pub(crate) fn apply_message_key_columns(
    cfg: &mut HashMap<String, String>,
    profile: &CdcProfile,
    keyless: &HashMap<(String, String), crate::connector_catalog::KeylessTable>,
) -> Result<(), AppError> {
    let src = &profile.source;
    let mut entries: Vec<String> = Vec::new();
    let mut overridden: Vec<String> = Vec::new();
    if let Some(raw) = src.connection.get("message_key_columns") {
        for entry in raw.split(';').map(str::trim).filter(|e| !e.is_empty()) {
            if let Some((table, _)) = entry.split_once(':') {
                overridden.push(table.trim().replace("\\.", ".").to_ascii_lowercase());
            }
            entries.push(entry.to_string());
        }
    }

    let include: Vec<String> = source_table_include_list(profile)
        .iter()
        .map(|t| t.trim().to_ascii_lowercase())
        .collect();
    let schemas: Vec<String> = if src.schema_mode == SchemaSelectionMode::SpecificSchemas {
        src.schemas
            .iter()
            .map(|s| s.trim().to_ascii_lowercase())
            .collect()
    } else {
        Vec::new()
    };

    let mut candidates: Vec<_> = keyless
        .iter()
        .filter(|((schema, table), _)| {
            let qualified = format!("{schema}.{table}").to_ascii_lowercase();
            if !include.is_empty() {
                include.contains(&qualified)
            } else {
                schemas.is_empty() || schemas.contains(&schema.to_ascii_lowercase())
            }
        })
        .collect();
    candidates.sort_by(|a, b| a.0.cmp(b.0));

    let mut missing = Vec::new();
    for ((schema, table), facts) in candidates {
        let qualified = format!("{schema}.{table}");
        if overridden.contains(&qualified.to_ascii_lowercase()) {
            continue;
        }
        let key = facts.derived_key();
        if key.is_empty() {
            missing.push(qualified);
        } else {
            entries.push(format!(
                "{}\\.{}:{}",
                regex_escape(schema),
                regex_escape(table),
                key.join(",")
            ));
        }
    }

    if !missing.is_empty() && profile.pattern == ProfilePattern::SourceToTarget {
        return Err(AppError::BadRequest(format!(
            "source table(s) without a primary key or usable unique key: {}. \
             Add a PRIMARY KEY on the source, or set source connection \
             `message_key_columns` (e.g. `schema.table:col1,col2`).",
            missing.join(", ")
        )));
    }
    if entries.is_empty() {
        cfg.remove("message.key.columns");
    } else {
        cfg.insert("message.key.columns".into(), entries.join(";"));
    }
    Ok(())
}

fn insert_table_include(cfg: &mut HashMap<String, String>, profile: &CdcProfile) {
    let tables = source_table_include_list(profile);
    if !tables.is_empty() {
        cfg.insert("table.include.list".into(), tables.join(","));
    }
}

fn apply_debezium_transforms(cfg: &mut HashMap<String, String>, profile: &CdcProfile) {
    let enabled: Vec<_> = profile
        .flow
        .transforms
        .iter()
        .filter(|t| t.enabled)
        .collect();
    if enabled.is_empty() {
        return;
    }
    let names: Vec<String> = enabled.iter().map(|t| sanitize_smt_name(&t.name)).collect();
    cfg.insert("transforms".into(), names.join(","));
    for t in enabled {
        let alias = sanitize_smt_name(&t.name);
        cfg.insert(format!("transforms.{alias}.type"), smt_class_for(&t.name));
        for (k, v) in &t.config {
            cfg.insert(format!("transforms.{alias}.{k}"), v.clone());
        }
    }
}

fn resync_history_detail(snapshot: Option<&[(String, i64)]>, cleared: &[String]) -> String {
    match snapshot {
        Some(copied) => {
            let snap = crate::postgres_trigger_cdc::snapshot_history_detail(copied);
            if cleared.is_empty() {
                snap
            } else {
                format!(
                    "Cleared {} target table(s) ({}); {snap}",
                    cleared.len(),
                    cleared.join(", ")
                )
            }
        }
        None if cleared.is_empty() => {
            "Source offsets reset; pipeline replaying from the start".to_string()
        }
        None => format!(
            "Cleared {} target table(s) and reset source offsets: {}",
            cleared.len(),
            cleared.join(", ")
        ),
    }
}

pub fn build_jdbc_sink_config(
    profile: &CdcProfile,
    schema_registry_url: &str,
) -> Result<HashMap<String, String>, AppError> {
    let target = profile.target.as_ref().ok_or_else(|| {
        AppError::BadRequest("target is required for source_to_target sink".into())
    })?;
    let host = req(&target.connection, "host")?;
    let port = req(&target.connection, "port")?;
    let database = req(&target.connection, "database")?;
    let username = req(&target.connection, "username")?;
    let password = req(&target.connection, "password")?;
    let db_type = target.target_type.trim().to_ascii_lowercase();
    let prefix = topic_prefix_for(profile);

    let jdbc_url = match db_type.as_str() {
        "postgres" | "postgresql" => {
            let schema = jdbc_target_schema(profile);
            format!("jdbc:postgresql://{host}:{port}/{database}?currentSchema={schema}")
        }
        "mysql" | "mariadb" => format!("jdbc:mysql://{host}:{port}/{database}"),
        "sqlserver" => format!("jdbc:sqlserver://{host}:{port};databaseName={database}"),
        "oracle" => format!("jdbc:oracle:thin:@//{host}:{port}/{database}"),
        other => {
            return Err(AppError::BadRequest(format!(
                "unsupported target_type for JDBC sink: {other}"
            )));
        }
    };

    let mut cfg = HashMap::new();
    cfg.insert(
        "connector.class".into(),
        "io.debezium.connector.jdbc.JdbcSinkConnector".into(),
    );
    cfg.insert("tasks.max".into(), "1".into());
    let capture_tables = source_table_include_list(profile);
    let topics_regex = if !capture_tables.is_empty() {
        let parts: Vec<String> = capture_tables.iter().map(|t| regex_escape(t)).collect();
        format!("{prefix}\\.({})", parts.join("|"))
    } else {
        // `routes` in the demo dump has no PK; record_key sink cannot consume it.
        format!("{prefix}\\.(?!.*\\.routes$).*")
    };
    cfg.insert("topics.regex".into(), topics_regex);
    cfg.insert("connection.url".into(), jdbc_url);
    cfg.insert("connection.username".into(), username.into());
    cfg.insert("connection.password".into(), password.into());
    cfg.insert("insert.mode".into(), "upsert".into());
    cfg.insert("delete.enabled".into(), "true".into());
    cfg.insert("primary.key.mode".into(), "record_key".into());
    cfg.insert("schema.evolution".into(), "basic".into());
    cfg.insert("quote.identifiers".into(), "true".into());
    cfg.insert("table.name.format".into(), "${topic}".into());
    // Native Postgres `point` is mapped to PostGIS ST_GeomFromWKB; skip it so
    // auto-create works on stock postgres:17 (no PostGIS on arm64).
    cfg.insert("field.exclude.list".into(), "coordinates".into());
    apply_sink_mapping_transforms(&mut cfg, profile);
    apply_sink_error_policy(&mut cfg, profile);
    apply_avro_converters(&mut cfg, schema_registry_url);
    Ok(cfg)
}

fn sanitize_smt_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

pub(crate) fn source_table_include_list(profile: &CdcProfile) -> Vec<String> {
    if !profile.source.tables.is_empty()
        || profile.source.table_mode == TableSelectionMode::SpecificTables
    {
        if !profile.source.tables.is_empty() {
            return profile.source.tables.clone();
        }
        return profile
            .flow
            .mappings
            .iter()
            .map(|m| m.source_qualified())
            .filter(|t| !t.is_empty())
            .collect();
    }
    Vec::new()
}

pub(crate) fn jdbc_target_schema(profile: &CdcProfile) -> &str {
    profile
        .flow
        .mappings
        .iter()
        .map(|m| m.target_schema.trim())
        .find(|s| !s.is_empty())
        .or_else(|| {
            profile
                .source
                .schemas
                .first()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or("public")
}

fn regex_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if matches!(
            c,
            '.' | '+' | '*' | '?' | '^' | '$' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn apply_sink_mapping_transforms(cfg: &mut HashMap<String, String>, profile: &CdcProfile) {
    apply_sink_transforms(cfg, profile, true);
}

/// `TopicNameMatches` pattern that selects one mapped table's capture topic.
///
/// The topic shape depends on the capture engine, the same split as
/// `profile_table_status::capture_topic`:
/// - Debezium (Logical / SyncOnce): `{prefix}.{schema}.{table}` → match schema + table;
/// - JDBC source (Trigger / Incremental, table mode or per-table query mode):
///   `{prefix}.{TABLE}` with no schema segment → match the table name alone.
///
/// Emitting the Debezium shape for a JDBC pipeline never matched, so the rename SMT
/// was skipped, the fallback `route` SMT produced the raw source name and the sink
/// auto-created a second table (e.g. `"PAT_CARMODEL"` beside the mapped
/// `pat_carmodel`). The JDBC form is case-insensitive because dialects differ in how
/// they case the table name in the topic.
pub(crate) fn sink_topic_predicate(
    profile: &CdcProfile,
    mapping: &crate::models::TableMapping,
) -> String {
    use crate::models::CaptureMode;
    let table = regex_escape(mapping.source_table.trim());
    let schema = mapping.source_schema.trim();
    match profile.source.capture_mode {
        CaptureMode::Logical | CaptureMode::SyncOnce if !schema.is_empty() => {
            format!(r".*{}\.{table}$", regex_escape(schema))
        }
        CaptureMode::Logical | CaptureMode::SyncOnce => format!(r".*\.{table}$"),
        CaptureMode::Trigger | CaptureMode::Incremental => format!(r"(?i).*\.{table}$"),
    }
}

/// SMT chain for the JDBC sink. Debezium envelopes need `unwrap`; JDBC source
/// records are already flat, so incremental capture passes `unwrap=false`.
pub(crate) fn apply_sink_transforms(
    cfg: &mut HashMap<String, String>,
    profile: &CdcProfile,
    unwrap: bool,
) {
    let remaps: Vec<_> = profile
        .flow
        .mappings
        .iter()
        .filter(|m| m.is_remap())
        .collect();

    let mut transform_names = Vec::new();
    if unwrap {
        transform_names.push("unwrap".to_string());
    }
    let mut predicate_names = Vec::new();

    for (i, mapping) in remaps.iter().enumerate() {
        let pred = format!("p{i}");
        let route = format!("map{i}");
        predicate_names.push(pred.clone());

        let pattern = sink_topic_predicate(profile, mapping);
        cfg.insert(
            format!("predicates.{pred}.type"),
            "org.apache.kafka.connect.transforms.predicates.TopicNameMatches".into(),
        );
        cfg.insert(format!("predicates.{pred}.pattern"), pattern);

        cfg.insert(
            format!("transforms.{route}.type"),
            "org.apache.kafka.connect.transforms.RegexRouter".into(),
        );
        cfg.insert(format!("transforms.{route}.regex"), ".*".into());
        cfg.insert(
            format!("transforms.{route}.replacement"),
            mapping.target_table.trim().into(),
        );
        cfg.insert(format!("transforms.{route}.predicate"), pred.clone());
        transform_names.push(route);

        if !mapping.columns.is_empty() {
            let rename = format!("cols{i}");
            let include: Vec<String> = mapping
                .columns
                .iter()
                .map(|c| c.source.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let renames: Vec<String> = mapping
                .columns
                .iter()
                .filter(|c| {
                    !c.source.trim().is_empty()
                        && !c.target.trim().is_empty()
                        && c.source.trim() != c.target.trim()
                })
                .map(|c| format!("{}:{}", c.source.trim(), c.target.trim()))
                .collect();
            cfg.insert(
                format!("transforms.{rename}.type"),
                "org.apache.kafka.connect.transforms.ReplaceField$Value".into(),
            );
            cfg.insert(format!("transforms.{rename}.include"), include.join(","));
            if !renames.is_empty() {
                cfg.insert(format!("transforms.{rename}.renames"), renames.join(","));
            }
            cfg.insert(format!("transforms.{rename}.predicate"), pred);
            transform_names.push(rename);
        }
    }

    transform_names.push("route".to_string());
    cfg.insert("transforms".into(), transform_names.join(","));
    if !predicate_names.is_empty() {
        cfg.insert("predicates".into(), predicate_names.join(","));
    }
    if unwrap {
        cfg.insert(
            "transforms.unwrap.type".into(),
            "io.debezium.transforms.ExtractNewRecordState".into(),
        );
        cfg.insert("transforms.unwrap.drop.tombstones".into(), "false".into());
        cfg.insert(
            "transforms.unwrap.delete.handling.mode".into(),
            "rewrite".into(),
        );
    }
    cfg.insert(
        "transforms.route.type".into(),
        "org.apache.kafka.connect.transforms.RegexRouter".into(),
    );
    cfg.insert("transforms.route.regex".into(), ".*\\.([^.]+)$".into());
    cfg.insert("transforms.route.replacement".into(), "$1".into());
}

fn smt_class_for(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    if n.contains("mask") || n.contains("pii") {
        "org.apache.kafka.connect.transforms.ReplaceField$Value".into()
    } else if n.contains("route") {
        "org.apache.kafka.connect.transforms.RegexRouter".into()
    } else {
        "org.apache.kafka.connect.transforms.ExtractField$Value".into()
    }
}

fn req<'a>(m: &'a HashMap<String, String>, key: &str) -> Result<&'a str, AppError> {
    m.get(key)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest(format!("missing connection field: {key}")))
}

pub async fn connect_request(
    state: &AppState,
    method: Method,
    path: &str,
    payload: Option<Value>,
) -> Result<Value, AppError> {
    let url = format!(
        "{}{}",
        state.config.connect_base_url.trim_end_matches('/'),
        path
    );
    let mut req = state.http.request(method, &url);
    if let Some(p) = payload {
        req = req.json(&p);
    }
    let res = req
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("connect request failed: {e}")))?;
    let status = res.status();
    let bytes = res
        .bytes()
        .await
        .map_err(|e| AppError::Upstream(format!("connect response read failed: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "connect returned {status}: {}",
            String::from_utf8_lossy(&bytes)
        )));
    }
    if bytes.is_empty() {
        return Ok(json!({ "status": "ok" }));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| AppError::Upstream(format!("invalid connect json: {e}")))
}

async fn connector_exists(state: &AppState, name: &str) -> Result<bool, AppError> {
    let url = format!(
        "{}/connectors/{}",
        state.config.connect_base_url.trim_end_matches('/'),
        urlencoding::encode(name)
    );
    let res = state
        .http
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("connect probe failed: {e}")))?;
    Ok(res.status().is_success())
}

/// Create-or-update a connector with `PUT /connectors/{name}/config`.
///
/// Kafka Connect treats that PUT as an upsert (201 on create, 200 on update), so there is
/// no exists-probe followed by `POST /connectors` — a window in which a second Start saw
/// "no connector", posted too, and failed with 409 "Connector … already exists".
async fn upsert_connector(
    state: &AppState,
    name: &str,
    config: &HashMap<String, String>,
) -> Result<(), AppError> {
    connect_request(
        state,
        Method::PUT,
        &format!("/connectors/{}/config", urlencoding::encode(name)),
        Some(serde_json::to_value(config).unwrap()),
    )
    .await?;
    Ok(())
}

/// Pre-create the capture topics a Start is about to produce into.
///
/// Topics normally appear when the source writes its first record. Until then the
/// `topics.regex` sink has nothing to subscribe to and the Tables panel reports
/// "topic not found", which reads like a failure on a pipeline that started seconds
/// ago. Creating them up front (same settings as `topic.creation.default.*`) makes the
/// sink attach immediately and the panel show 0 messages instead. Best effort: the
/// connector's own topic creation still covers anything this misses.
async fn ensure_capture_topics(state: &AppState, profile: &CdcProfile) {
    for topic in crate::profile_table_status::capture_topics(profile) {
        if let Err(e) = crate::kafka_topics::ensure_topic(state, &topic).await {
            tracing::warn!(topic = %topic, error = %e, "could not pre-create capture topic");
        }
    }
}

async fn pause_connector(state: &AppState, name: &str) -> Result<(), AppError> {
    connect_request(
        state,
        Method::PUT,
        &format!("/connectors/{}/pause", urlencoding::encode(name)),
        None,
    )
    .await?;
    Ok(())
}

async fn resume_connector(state: &AppState, name: &str) -> Result<(), AppError> {
    connect_request(
        state,
        Method::PUT,
        &format!("/connectors/{}/resume", urlencoding::encode(name)),
        None,
    )
    .await?;
    Ok(())
}

/// Start must leave a connector RUNNING. A config PUT does not change Connect's target
/// state, so a connector paused by Stop stayed PAUSED after Start, and a FAILED task is
/// only retried when its config changes. Resume (no-op when running), then restart just
/// the failed tasks. Best effort: a rebalance can answer 409 right after a config PUT;
/// the table status then reports the connector instead of a silent "syncing".
async fn ensure_running(state: &AppState, name: &str) {
    if let Err(e) = resume_connector(state, name).await {
        tracing::warn!(connector = %name, error = %e, "resume after start failed");
    }
    let path = format!(
        "/connectors/{}/restart?includeTasks=true&onlyFailed=true",
        urlencoding::encode(name)
    );
    if let Err(e) = connect_request(state, Method::POST, &path, None).await {
        tracing::warn!(connector = %name, error = %e, "restart of failed tasks after start failed");
    }
}

/// Why the offsets stored for an existing source connector cannot carry over to `new`.
async fn existing_offsets_incompatible(
    state: &AppState,
    name: &str,
    new: &HashMap<String, String>,
) -> Option<String> {
    let path = format!("/connectors/{}/config", urlencoding::encode(name));
    let old: HashMap<String, String> = match connect_request(state, Method::GET, &path, None).await
    {
        Ok(v) => serde_json::from_value(v).ok()?,
        Err(_) => return None, // not created yet: nothing stored
    };
    offsets_incompatible_reason(&old, new)
}

/// Pure rule behind [`existing_offsets_incompatible`].
fn offsets_incompatible_reason(
    old: &HashMap<String, String>,
    new: &HashMap<String, String>,
) -> Option<String> {
    fn get<'a>(m: &'a HashMap<String, String>, k: &str) -> &'a str {
        m.get(k).map(String::as_str).unwrap_or("")
    }
    if get(old, "connector.class") != get(new, "connector.class") {
        return Some("capture connector type changed".into());
    }
    // A snapshot-only run stores "snapshot done" without ever streaming; a streaming
    // mode would resume after it and silently miss every change made since.
    if get(old, "snapshot.mode") == "initial_only" && get(new, "snapshot.mode") != "initial_only" {
        return Some("switched from Sync once to ongoing capture".into());
    }
    None
}

async fn cleanup_leftover_connectors(state: &AppState, profile: &CdcProfile) {
    for leftover in leftover_connector_names(profile) {
        if let Err(err) = stop_connector(state, &leftover, true).await {
            tracing::warn!(connector = %leftover, error = %err, "failed to delete leftover connector");
        }
    }
}

/// Extra per-table source connectors (`{primary}-poll-*`) that currently exist in Connect.
///
/// Read from Connect rather than derived from the profile, so connectors left behind by a
/// table whose strategy changed are still paused, stopped and removed with the pipeline.
async fn existing_poll_connectors(state: &AppState, profile: &CdcProfile) -> Vec<String> {
    let prefix = crate::incremental_cdc::poll_connector_prefix(&connector_name_for(profile));
    match connect_request(state, Method::GET, "/connectors", None).await {
        Ok(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|name| name.starts_with(&prefix))
            .map(str::to_string)
            .collect(),
        Ok(_) => Vec::new(),
        Err(e) => {
            tracing::warn!(error = %e, "could not list per-table poll connectors");
            Vec::new()
        }
    }
}

async fn stop_connector(state: &AppState, name: &str, delete: bool) -> Result<(), AppError> {
    if !connector_exists(state, name).await? {
        return Ok(());
    }
    if delete {
        connect_request(
            state,
            Method::DELETE,
            &format!("/connectors/{}", urlencoding::encode(name)),
            None,
        )
        .await?;
    } else {
        let _ = pause_connector(state, name).await;
    }
    Ok(())
}

/// Resolve source columns for incremental capture, but only when the connector actually
/// needs them to rename fields. Sources whose case already matches the target skip the
/// catalog round-trip entirely.
async fn incremental_source_columns(
    profile: &CdcProfile,
) -> Result<HashMap<String, Vec<String>>, AppError> {
    if !crate::incremental_cdc::needs_lowercase_fields(profile) {
        return Ok(HashMap::new());
    }
    let vendor = crate::trigger_cdc::Vendor::parse(&profile.source.source_type)?;
    let tables = crate::postgres_trigger_cdc::parse_qualified_tables(profile)?;
    crate::trigger_cdc::preview_columns(vendor, &profile.source.connection, &tables).await
}

pub async fn reconcile_start(state: &AppState, profile: &mut CdcProfile) -> Result<(), AppError> {
    if profile.pattern == ProfilePattern::SourceToOlake {
        return Err(AppError::BadRequest(
            "Analytics-first (OLake) cannot start yet. Save a draft; the OLake stack is Phase 3."
                .into(),
        ));
    }
    let registry = schema_registry_url(state);
    let name = connector_name_for(profile);
    // Incremental pipelines may need one source connector per rolling-window / full-copy
    // table on top of the table-mode watermark connector; the first one keeps `name`.
    let sources: Vec<(String, HashMap<String, String>)> = match profile.source.capture_mode {
        crate::models::CaptureMode::Trigger => vec![(
            name.clone(),
            crate::postgres_trigger_cdc::build_jdbc_source_config(profile, &registry)?,
        )],
        crate::models::CaptureMode::Incremental => {
            let columns = incremental_source_columns(profile).await?;
            crate::incremental_cdc::source_connector_plan(profile, &name, &registry, &columns)?
                .into_iter()
                .map(|spec| (spec.name, spec.config))
                .collect()
        }
        _ => {
            let mut cfg = build_debezium_config(profile, &registry)?;
            if matches!(
                profile
                    .source
                    .source_type
                    .trim()
                    .to_ascii_lowercase()
                    .as_str(),
                "postgres" | "postgresql"
            ) {
                let insight = crate::connector_catalog::insight_from_connection(
                    "postgres",
                    &profile.source.connection,
                );
                // Best effort: if the orchestrator can't reach the source, start with
                // explicit overrides only (Connect may still reach it on its network).
                let keyless = match crate::connector_catalog::postgres_client(&insight).await {
                    Ok(client) => crate::connector_catalog::postgres_keyless_tables(&client).await,
                    Err(e) => Err(e),
                };
                match keyless {
                    Ok(keyless) => apply_message_key_columns(&mut cfg, profile, &keyless)?,
                    Err(err) => tracing::warn!(
                        connector = %name,
                        error = %err,
                        "could not read source keys; keyless tables will emit null record keys"
                    ),
                }
            }
            vec![(name.clone(), cfg)]
        }
    };
    cleanup_leftover_connectors(state, profile).await;
    for stale in existing_poll_connectors(state, profile).await {
        if !sources.iter().any(|(n, _)| *n == stale) {
            if let Err(err) = stop_connector(state, &stale, true).await {
                tracing::warn!(connector = %stale, error = %err, "failed to delete stale poll connector");
            }
        }
    }
    for (source_name, config) in &sources {
        if let Some(reason) = existing_offsets_incompatible(state, source_name, config).await {
            // Stored offsets would make the new connector skip rows (e.g. Sync once →
            // Change capture: Debezium sees a finished snapshot and streams only from
            // now, losing everything written in between). Reset so it snapshots again;
            // the sink upserts by key, so re-sent rows do not duplicate.
            halt_connector(state, source_name).await?;
            wait_for_stopped(state, source_name).await?;
            reset_connector_offsets(state, source_name).await?;
            profile.push_history(HistoryItem {
                timestamp: Utc::now(),
                level: "warn".to_string(),
                event: "source_offsets_reset".to_string(),
                detail: format!(
                    "{reason}: cleared stored offsets of {source_name}; a full snapshot runs before streaming"
                ),
            });
        }
        upsert_connector(state, source_name, config).await?;
        ensure_running(state, source_name).await;
    }
    profile.connector_name = Some(name.clone());
    ensure_capture_topics(state, profile).await;

    // Trigger capture lands in hyper_cdc.inbox on the Postgres target; install the
    // inbox + apply trigger before the sink starts so changes reach the real tables.
    if profile.source.capture_mode == crate::models::CaptureMode::Trigger
        && profile.pattern == ProfilePattern::SourceToTarget
    {
        crate::postgres_trigger_cdc::install_target_apply(profile).await?;
    }

    if profile.has_managed_sink() {
        let sink_name = sink_connector_name_for(profile);
        let sink_config = build_managed_sink_config(profile, &registry)?;
        upsert_connector(state, &sink_name, &sink_config).await?;
        ensure_running(state, &sink_name).await;
        profile.sink_connector_name = Some(sink_name);
    }

    profile.status = ProfileStatus::Running;
    profile.last_error = None;
    profile.updated_at = Utc::now();
    profile.push_history(HistoryItem {
        timestamp: Utc::now(),
        level: "info".to_string(),
        event: "profile_started".to_string(),
        detail: "CDC pipeline reconciled and running".to_string(),
    });
    Ok(())
}

pub async fn reconcile_pause(state: &AppState, profile: &mut CdcProfile) -> Result<(), AppError> {
    let name = connector_name_for(profile);
    pause_connector(state, &name).await?;
    for extra in existing_poll_connectors(state, profile).await {
        let _ = pause_connector(state, &extra).await;
    }
    profile.connector_name = Some(name);
    if profile.has_managed_sink() {
        let sink_name = sink_connector_name_for(profile);
        let _ = pause_connector(state, &sink_name).await;
        profile.sink_connector_name = Some(sink_name);
    }
    profile.status = ProfileStatus::Paused;
    profile.updated_at = Utc::now();
    profile.push_history(HistoryItem {
        timestamp: Utc::now(),
        level: "info".to_string(),
        event: "profile_paused".to_string(),
        detail: "CDC pipeline paused".to_string(),
    });
    Ok(())
}

pub async fn reconcile_resume(state: &AppState, profile: &mut CdcProfile) -> Result<(), AppError> {
    let name = connector_name_for(profile);
    resume_connector(state, &name).await?;
    for extra in existing_poll_connectors(state, profile).await {
        let _ = resume_connector(state, &extra).await;
    }
    profile.connector_name = Some(name);
    if profile.has_managed_sink() {
        let sink_name = sink_connector_name_for(profile);
        let _ = resume_connector(state, &sink_name).await;
        profile.sink_connector_name = Some(sink_name);
    }
    profile.status = ProfileStatus::Running;
    profile.updated_at = Utc::now();
    profile.push_history(HistoryItem {
        timestamp: Utc::now(),
        level: "info".to_string(),
        event: "profile_resumed".to_string(),
        detail: "CDC pipeline resumed".to_string(),
    });
    Ok(())
}

pub async fn reconcile_restart(state: &AppState, profile: &mut CdcProfile) -> Result<(), AppError> {
    // Re-apply generated config so a restart picks up include-list / converter
    // changes instead of bouncing a stale connector.
    cleanup_leftover_connectors(state, profile).await;
    reconcile_start(state, profile).await?;
    if let Some(last) = profile.history.last_mut() {
        if last.event == "profile_started" {
            last.event = "profile_restarted".into();
            last.detail = "CDC pipeline reconciled and restarted".into();
        }
    }
    Ok(())
}

pub async fn reconcile_stop(state: &AppState, profile: &mut CdcProfile) -> Result<(), AppError> {
    let name = connector_name_for(profile);
    let delete = state.config.stop_deletes_connector;
    cleanup_leftover_connectors(state, profile).await;
    stop_connector(state, &name, delete).await?;
    for extra in existing_poll_connectors(state, profile).await {
        if let Err(err) = stop_connector(state, &extra, delete).await {
            tracing::warn!(connector = %extra, error = %err, "failed to stop poll connector");
        }
    }
    profile.connector_name = Some(name);
    if profile.has_managed_sink() {
        let sink_name = sink_connector_name_for(profile);
        let _ = stop_connector(state, &sink_name, delete).await;
        profile.sink_connector_name = Some(sink_name);
    }
    profile.status = ProfileStatus::Stopped;
    profile.updated_at = Utc::now();
    profile.push_history(HistoryItem {
        timestamp: Utc::now(),
        level: "info".to_string(),
        event: "profile_stopped".to_string(),
        detail: "CDC pipeline stopped".to_string(),
    });
    Ok(())
}

/// `PUT /connectors/{name}/stop`.
///
/// Distinct from pause: Kafka Connect only releases a connector's stored offsets while
/// it is STOPPED, so a paused connector cannot be reset.
async fn halt_connector(state: &AppState, name: &str) -> Result<(), AppError> {
    connect_request(
        state,
        Method::PUT,
        &format!("/connectors/{}/stop", urlencoding::encode(name)),
        None,
    )
    .await?;
    Ok(())
}

/// `GET /connectors/{name}/status` → the connector state (`RUNNING`, `STOPPED`, …).
///
/// `Ok(None)` when Connect answers 404: a connector that was just created (or just
/// re-targeted) has its config committed before any worker writes a status record, so
/// "No status found" is a normal, short-lived state rather than an error.
async fn connector_state(state: &AppState, name: &str) -> Result<Option<String>, AppError> {
    let url = format!(
        "{}/connectors/{}/status",
        state.config.connect_base_url.trim_end_matches('/'),
        urlencoding::encode(name)
    );
    let res = state
        .http
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("connect request failed: {e}")))?;
    let status = res.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| AppError::Upstream(format!("connect response read failed: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "connect returned {status}: {}",
            String::from_utf8_lossy(&bytes)
        )));
    }
    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|e| AppError::Upstream(format!("invalid connect json: {e}")))?;
    Ok(body
        .get("connector")
        .and_then(|c| c.get("state"))
        .and_then(|s| s.as_str())
        .map(str::to_string))
}

async fn wait_for_stopped(state: &AppState, name: &str) -> Result<(), AppError> {
    let mut last_state: Option<String> = None;
    for _ in 0..40 {
        last_state = connector_state(state, name).await?;
        if last_state.as_deref() == Some("STOPPED") {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let seen = last_state.unwrap_or_else(|| "no status reported by Connect".to_string());
    Err(AppError::Upstream(format!(
        "connector '{name}' did not reach STOPPED within 20s (last state: {seen}); aborted before touching offsets"
    )))
}

async fn reset_connector_offsets(state: &AppState, name: &str) -> Result<(), AppError> {
    connect_request(
        state,
        Method::DELETE,
        &format!("/connectors/{}/offsets", urlencoding::encode(name)),
        None,
    )
    .await?;
    Ok(())
}

/// Empty the profile's target tables so a replay lands in a clean target.
///
/// Every table is truncated in a single statement so that foreign keys among them do
/// not block the operation. Returns the qualified names that were cleared.
async fn truncate_target_tables(profile: &CdcProfile) -> Result<Vec<String>, AppError> {
    let target = profile
        .target
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("pipeline has no target to clear".into()))?;
    if !matches!(
        target.target_type.trim().to_ascii_lowercase().as_str(),
        "postgres" | "postgresql"
    ) {
        return Err(AppError::BadRequest(format!(
            "clearing the target is only supported for PostgreSQL targets, not '{}'",
            target.target_type
        )));
    }
    let tables = crate::postgres_trigger_cdc::parse_qualified_tables(profile)?;
    if tables.is_empty() {
        return Ok(Vec::new());
    }
    let mut quoted = Vec::new();
    let mut names = Vec::new();
    for table in &tables {
        quoted.push(format!(
            "{}.{}",
            crate::postgres_trigger_cdc::quote_ident(&table.target_schema)?,
            crate::postgres_trigger_cdc::quote_ident(&table.target_table)?
        ));
        names.push(format!("{}.{}", table.target_schema, table.target_table));
    }
    let insight = crate::connector_catalog::insight_from_connection("postgres", &target.connection);
    let client = crate::connector_catalog::postgres_client(&insight).await?;
    client
        .batch_execute(&format!("TRUNCATE TABLE {}", quoted.join(", ")))
        .await
        .map_err(|e| {
            AppError::BadRequest(format!(
                "failed to clear target tables ({}): {}",
                names.join(", "),
                crate::connector_catalog::postgres_ddl_detail(&e)
            ))
        })?;
    Ok(names)
}

/// Replay the whole pipeline from the start of the source.
///
/// Emptying the target is invisible to the source connector, which tracks its own
/// position in the *source* database and will happily report "nothing new" against an
/// empty target. A resync therefore stops both connectors, clears that stored position,
/// and lets the source republish every row.
///
/// Trigger capture is different: the JDBC source only tails the changelog, so
/// resetting offsets just replays leftover change events. A trigger resync copies
/// the live source tables into the target and leaves the changelog offset where it
/// is so stale events cannot overwrite that snapshot.
fn table_snapshot_resync_path(profile: &CdcProfile, row_limit: Option<u64>) -> bool {
    if profile.pattern != ProfilePattern::SourceToTarget {
        return false;
    }
    match profile.source.capture_mode {
        crate::models::CaptureMode::Trigger => true,
        crate::models::CaptureMode::Incremental => row_limit.is_some(),
        _ => false,
    }
}

fn require_row_limit_for_profile(
    profile: &CdcProfile,
    row_limit: Option<u64>,
) -> Result<(), AppError> {
    if row_limit.is_none() {
        return Ok(());
    }
    if profile.pattern != ProfilePattern::SourceToTarget {
        return Err(AppError::BadRequest(
            "row_limit applies only to database replica (source_to_target) pipelines".into(),
        ));
    }
    if !matches!(
        profile.source.capture_mode,
        crate::models::CaptureMode::Trigger | crate::models::CaptureMode::Incremental
    ) {
        return Err(AppError::BadRequest(
            "row_limit applies only to trigger or incremental replica pipelines (live table snapshot copy)".into(),
        ));
    }
    Ok(())
}

/// (cleared target tables, rows copied per table) from a live table snapshot.
type SnapshotOutcome = (Vec<String>, Vec<(String, i64)>);

/// Halt connectors, optionally truncate targets, copy live source rows, then resume them
/// when `resume_after` (resync). The test initial sync leaves them STOPPED for teardown.
/// Does not reset Kafka Connect offsets (safe after Start or for resync).
async fn resume_snapshot_connectors(
    state: &AppState,
    source_name: &str,
    sink_name: Option<&str>,
    extras: &[String],
    snapshot_result: &Result<SnapshotOutcome, AppError>,
) -> Result<(), AppError> {
    if let Err(e) = resume_halted_connectors(state, source_name, sink_name).await {
        if snapshot_result.is_ok() {
            return Err(e);
        }
        tracing::warn!(error = %e, "failed to resume connectors after failed table snapshot");
    }
    for extra in extras {
        if let Err(e) = resume_connector(state, extra).await {
            if snapshot_result.is_ok() {
                return Err(e);
            }
            tracing::warn!(connector = %extra, error = %e, "failed to resume poll connector");
        }
    }
    Ok(())
}

async fn run_trigger_table_snapshot(
    state: &AppState,
    profile: &CdcProfile,
    clear_target: bool,
    row_limit: Option<u64>,
    resume_after: bool,
) -> Result<SnapshotOutcome, AppError> {
    let source_name = connector_name_for(profile);
    let sink_name = if profile.has_managed_sink() {
        Some(sink_connector_name_for(profile))
    } else {
        None
    };
    if let Some(sink) = sink_name.as_deref() {
        halt_connector(state, sink).await?;
    }
    halt_connector(state, &source_name).await?;
    let extras = existing_poll_connectors(state, profile).await;
    for extra in &extras {
        halt_connector(state, extra).await?;
    }
    wait_for_stopped(state, &source_name).await?;

    let snapshot_result: Result<SnapshotOutcome, AppError> = async {
        let cleared = if clear_target {
            truncate_target_tables(profile).await?
        } else {
            Vec::new()
        };
        let copied =
            crate::postgres_trigger_cdc::snapshot_source_tables(profile, !clear_target, row_limit)
                .await?;
        Ok((cleared, copied))
    }
    .await;

    if !resume_after {
        return snapshot_result;
    }
    resume_snapshot_connectors(
        state,
        &source_name,
        sink_name.as_deref(),
        &extras,
        &snapshot_result,
    )
    .await?;
    snapshot_result
}

async fn resume_halted_connectors(
    state: &AppState,
    source_name: &str,
    sink_name: Option<&str>,
) -> Result<(), AppError> {
    resume_connector(state, source_name).await?;
    if let Some(sink) = sink_name {
        resume_connector(state, sink).await?;
    }
    Ok(())
}

/// Test initial sync: verify connectivity and configuration, then stop.
///
/// Starts the pipeline (Connect accepts the config and reaches source/target), copies up to
/// `row_limit` rows per table into the target (read, write, mapping, permissions), then tears
/// the connectors down — offsets reset first so a later production Start begins clean, then
/// deleted. The profile ends `Stopped`; this is never a production sync. Teardown also runs
/// when any step fails, so a failed test never leaves a pipeline running.
pub async fn reconcile_start_with_test_sync(
    state: &AppState,
    profile: &mut CdcProfile,
    action: &str,
    clear_target: bool,
    row_limit: Option<u64>,
) -> Result<(), AppError> {
    require_row_limit_for_profile(profile, row_limit)?;
    let Some(limit) = row_limit else {
        return Err(AppError::BadRequest(
            "test initial sync requires row_limit".into(),
        ));
    };
    if !matches!(action, "start" | "restart") {
        return Err(AppError::BadRequest(format!(
            "test initial sync is not supported for action '{action}'"
        )));
    }

    let outcome: Result<SnapshotOutcome, AppError> = async {
        match action {
            "start" => reconcile_start(state, profile).await?,
            _ => reconcile_restart(state, profile).await?,
        }
        run_trigger_table_snapshot(state, profile, clear_target, Some(limit), false).await
    }
    .await;

    let teardown = teardown_test_connectors(state, profile).await;
    profile.connector_name = None;
    profile.sink_connector_name = None;
    profile.updated_at = Utc::now();

    let (cleared, copied) = match outcome {
        Ok(result) => result,
        Err(e) => {
            if let Err(t) = teardown {
                tracing::warn!(error = %t, "test initial sync teardown failed after error");
            }
            profile.status = ProfileStatus::Stopped;
            profile.push_history(HistoryItem {
                timestamp: Utc::now(),
                level: "error".to_string(),
                event: "profile_test_initial_sync_failed".to_string(),
                detail: format!(
                    "Test initial sync failed; connectors removed, nothing left running. {}",
                    e.to_string_message()
                ),
            });
            return Err(e);
        }
    };
    teardown?;

    profile.push_history(HistoryItem {
        timestamp: Utc::now(),
        level: "info".to_string(),
        event: "profile_test_initial_sync".to_string(),
        detail: test_sync_history_detail(limit, &copied, &cleared),
    });
    profile.status = ProfileStatus::Stopped;
    profile.last_error = None;
    Ok(())
}

/// Remove the connectors a test run created: stop, reset source offsets, delete.
async fn teardown_test_connectors(state: &AppState, profile: &CdcProfile) -> Result<(), AppError> {
    let source_name = connector_name_for(profile);
    let mut first_error = None;
    if profile.has_managed_sink() {
        let sink = sink_connector_name_for(profile);
        if let Err(e) = stop_connector(state, &sink, true).await {
            first_error.get_or_insert(e);
        }
    }
    if connector_exists(state, &source_name).await? {
        let _ = halt_connector(state, &source_name).await;
        let reset = async {
            wait_for_stopped(state, &source_name).await?;
            reset_connector_offsets(state, &source_name).await
        }
        .await;
        if let Err(e) = reset {
            first_error.get_or_insert(e);
        }
        if let Err(e) = stop_connector(state, &source_name, true).await {
            first_error.get_or_insert(e);
        }
    }
    // Rolling-window / full-copy connectors run in bulk mode and keep no offsets: delete.
    for extra in existing_poll_connectors(state, profile).await {
        if let Err(e) = stop_connector(state, &extra, true).await {
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn test_sync_history_detail(limit: u64, copied: &[(String, i64)], cleared: &[String]) -> String {
    let total: i64 = copied.iter().map(|(_, n)| n).sum();
    let tables = copied
        .iter()
        .map(|(t, n)| format!("{t}={n}"))
        .collect::<Vec<_>>()
        .join(", ");
    let cleared = if cleared.is_empty() {
        String::new()
    } else {
        format!(" Cleared {} target table(s) first.", cleared.len())
    };
    format!(
        "Test initial sync passed (row_limit={limit}): copied {total} row(s) across {} table(s) [{tables}].{cleared} \
         Pipeline stopped and connectors removed — not a production sync. Use Start or Resync to run it.",
        copied.len()
    )
}

/// Restart pipelines marked autostart after orchestrator boot (docker redeploy, crash, etc.).
pub async fn reconcile_autostart_profiles(state: &AppState) {
    let Ok(profiles) = state.profiles.list().await else {
        tracing::warn!("autostart skipped: could not list profiles");
        return;
    };
    for mut profile in profiles {
        if !profile.autostart || !profile.enabled {
            continue;
        }
        if !matches!(
            profile.status,
            ProfileStatus::Running | ProfileStatus::Failed
        ) {
            continue;
        }
        if profile.pattern == ProfilePattern::SourceToOlake {
            continue;
        }
        let name = profile.name.clone();
        match reconcile_start(state, &mut profile).await {
            Ok(()) => {
                profile.push_history(HistoryItem {
                    timestamp: Utc::now(),
                    level: "info".to_string(),
                    event: "profile_autostarted".to_string(),
                    detail: "Pipeline reconciled on orchestrator startup".to_string(),
                });
                if let Err(e) = state.profiles.upsert(&profile).await {
                    tracing::warn!(profile = %name, error = %e, "autostart upsert failed");
                } else {
                    info!(profile = %name, "autostart reconciled pipeline");
                }
            }
            Err(e) => {
                tracing::warn!(profile = %name, error = %e, "autostart reconcile failed");
                profile.last_error = Some(e.to_string_message());
                profile.status = ProfileStatus::Failed;
                let _ = state.profiles.upsert(&profile).await;
            }
        }
    }
}

/// Refuse to empty the target of a pipeline that polls any table with a rolling window.
///
/// A rolling window only ever re-reads recent rows, so after `TRUNCATE` every older row
/// (for example legacy rows with no `UPDATE_DATE`) would stay missing until someone runs
/// an initial load. Fail loudly instead of silently shrinking the replica — this also
/// covers the scheduled full refresh, which always clears the target.
fn guard_clear_target_for_rolling_window(
    profile: &CdcProfile,
    clear_target: bool,
) -> Result<(), AppError> {
    if clear_target && crate::incremental_cdc::has_rolling_window_tables(profile) {
        return Err(AppError::BadRequest(
            "clear_target is not allowed on a pipeline with rolling-window tables: the window \
             re-reads only recent rows, so older rows would stay missing. Resync without \
             clearing, or use Initial load (streaming) to rebuild the target."
                .into(),
        ));
    }
    Ok(())
}

pub async fn reconcile_resync(
    state: &AppState,
    profile: &mut CdcProfile,
    clear_target: bool,
    row_limit: Option<u64>,
) -> Result<(), AppError> {
    let source_name = connector_name_for(profile);
    require_row_limit_for_profile(profile, row_limit)?;
    guard_clear_target_for_rolling_window(profile, clear_target)?;
    if !connector_exists(state, &source_name).await? {
        return resync_fresh_start(state, profile, clear_target, row_limit).await;
    }
    if table_snapshot_resync_path(profile, row_limit) {
        let (cleared, copied) =
            run_trigger_table_snapshot(state, profile, clear_target, row_limit, true).await?;
        profile.connector_name = Some(source_name.clone());
        if profile.has_managed_sink() {
            profile.sink_connector_name = Some(sink_connector_name_for(profile));
        }
        profile.status = ProfileStatus::Running;
        profile.updated_at = Utc::now();
        profile.last_error = None;
        profile.push_history(HistoryItem {
            timestamp: Utc::now(),
            level: "info".to_string(),
            event: "profile_resynced".to_string(),
            detail: resync_history_detail(Some(&copied), &cleared),
        });
        return Ok(());
    }

    if let Some(sink) = profile
        .has_managed_sink()
        .then(|| sink_connector_name_for(profile))
    {
        halt_connector(state, &sink).await?;
    }
    halt_connector(state, &source_name).await?;
    let extras = existing_poll_connectors(state, profile).await;
    for extra in &extras {
        halt_connector(state, extra).await?;
    }
    wait_for_stopped(state, &source_name).await?;
    let cleared = if clear_target {
        truncate_target_tables(profile).await?
    } else {
        Vec::new()
    };
    reset_connector_offsets(state, &source_name).await?;
    resume_connector(state, &source_name).await?;
    // Bulk-mode poll connectors keep no offsets; resuming makes them re-read their window.
    for extra in &extras {
        resume_connector(state, extra).await?;
    }
    if profile.has_managed_sink() {
        let sink = sink_connector_name_for(profile);
        let _ = resume_connector(state, &sink).await;
        profile.sink_connector_name = Some(sink);
    }
    profile.connector_name = Some(source_name);
    profile.status = ProfileStatus::Running;
    profile.updated_at = Utc::now();
    profile.last_error = None;
    profile.push_history(HistoryItem {
        timestamp: Utc::now(),
        level: "info".to_string(),
        event: "profile_resynced".to_string(),
        detail: resync_history_detail(None, &cleared),
    });
    Ok(())
}

/// Resync of a pipeline whose connectors do not exist: never started, or removed by a
/// test initial sync (or a Stop configured to delete).
///
/// There are no offsets to reset — brand-new connectors already read every row on their
/// first poll / snapshot — so this is "clear the target if asked, then Start". Refusing
/// with "start the pipeline before resyncing" used to leave a fresh pipeline with no
/// working button: Start was the only way forward and a resync from the UI just failed.
async fn resync_fresh_start(
    state: &AppState,
    profile: &mut CdcProfile,
    clear_target: bool,
    row_limit: Option<u64>,
) -> Result<(), AppError> {
    let cleared = if clear_target {
        truncate_target_tables(profile).await?
    } else {
        Vec::new()
    };
    reconcile_start(state, profile).await?;
    // Trigger capture and capped runs copy the live tables directly, as a normal resync.
    let copied = if table_snapshot_resync_path(profile, row_limit) {
        let (_, copied) =
            run_trigger_table_snapshot(state, profile, false, row_limit, true).await?;
        Some(copied)
    } else {
        None
    };
    profile.status = ProfileStatus::Running;
    profile.updated_at = Utc::now();
    profile.last_error = None;
    let detail = match copied.as_deref() {
        Some(copied) => resync_history_detail(Some(copied), &cleared),
        None if cleared.is_empty() => {
            "Pipeline had not started yet: connectors created, full read from the source running"
                .to_string()
        }
        None => format!(
            "Cleared {} target table(s) ({}); pipeline had not started yet: connectors created, full read from the source running",
            cleared.len(),
            cleared.join(", ")
        ),
    };
    profile.push_history(HistoryItem {
        timestamp: Utc::now(),
        level: "info".to_string(),
        event: "profile_resynced".to_string(),
        detail,
    });
    Ok(())
}

/// How often a `topics.regex` sink re-reads topic metadata (ms).
///
/// The sink starts in the same Start call as the source, before the source has created
/// any capture topic. A regex consumer only notices new matching topics on a metadata
/// refresh — `metadata.max.age.ms`, 5 min by default — so a finished snapshot could sit in
/// Kafka for up to five minutes with the target still empty. Connect ≥ 3.0 allows this
/// per-connector override by default (`connector.client.config.override.policy=All`).
const SINK_METADATA_MAX_AGE_MS: &str = "10000";

fn build_managed_sink_config(
    profile: &CdcProfile,
    schema_registry_url: &str,
) -> Result<HashMap<String, String>, AppError> {
    let mut cfg = build_managed_sink_config_inner(profile, schema_registry_url)?;
    if cfg.contains_key("topics.regex") {
        cfg.entry("consumer.override.metadata.max.age.ms".into())
            .or_insert_with(|| SINK_METADATA_MAX_AGE_MS.into());
    }
    Ok(cfg)
}

fn build_managed_sink_config_inner(
    profile: &CdcProfile,
    schema_registry_url: &str,
) -> Result<HashMap<String, String>, AppError> {
    match profile.pattern {
        ProfilePattern::SourceToTarget
            if profile.source.capture_mode == crate::models::CaptureMode::Trigger =>
        {
            crate::postgres_trigger_cdc::build_trigger_sink_config(profile, schema_registry_url)
        }
        ProfilePattern::SourceToTarget
            if profile.source.capture_mode == crate::models::CaptureMode::Incremental =>
        {
            crate::incremental_cdc::build_incremental_sink_config(profile, schema_registry_url)
        }
        ProfilePattern::SourceToTarget => build_jdbc_sink_config(profile, schema_registry_url),
        ProfilePattern::SourceToLake => {
            crate::lake::build_lake_sink_config(profile, schema_registry_url)
        }
        ProfilePattern::SourceToStream => Err(AppError::BadRequest(
            "source_to_stream pipelines do not manage a sink connector".into(),
        )),
        ProfilePattern::SourceToOlake => Err(AppError::BadRequest(
            "source_to_olake pipelines do not manage a Kafka Connect sink".into(),
        )),
    }
}

pub async fn apply_profile_action(
    state: &AppState,
    profile: &mut CdcProfile,
    action: &str,
) -> Result<(), AppError> {
    if !profile.enabled && action != "stop" {
        return Err(AppError::BadRequest(
            "profile is disabled; enable it before running actions".to_string(),
        ));
    }
    match action {
        "start" => reconcile_start(state, profile).await,
        "pause" => reconcile_pause(state, profile).await,
        "resume" => reconcile_resume(state, profile).await,
        "restart" => reconcile_restart(state, profile).await,
        "stop" => reconcile_stop(state, profile).await,
        // Clearing the target is destructive, so the plain action never does it; callers
        // that want it go through reconcile_resync directly with an explicit opt-in.
        "resync" => reconcile_resync(state, profile, false, None).await,
        _ => Err(AppError::BadRequest(
            "unsupported profile action".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        ColumnMapping, DeliveryMode, FlowConfig, ScheduleConfig, SourceConfig, TableMapping,
        TargetConfig, TransformStrategy,
    };
    use chrono::Utc;
    use uuid::Uuid;

    fn cfg(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn sync_once_to_streaming_resets_offsets() {
        let dbz = "io.debezium.connector.postgresql.PostgresConnector";
        let once = cfg(&[("connector.class", dbz), ("snapshot.mode", "initial_only")]);
        let cdc = cfg(&[("connector.class", dbz), ("snapshot.mode", "initial")]);
        assert!(offsets_incompatible_reason(&once, &cdc).is_some());
        // Same mode again, or streaming → streaming: keep offsets (no re-snapshot).
        assert!(offsets_incompatible_reason(&cdc, &cdc).is_none());
        assert!(offsets_incompatible_reason(&once, &once).is_none());
        let jdbc = cfg(&[(
            "connector.class",
            "io.confluent.connect.jdbc.JdbcSourceConnector",
        )]);
        assert!(offsets_incompatible_reason(&jdbc, &cdc).is_some());
    }

    #[test]
    fn test_sync_history_detail_reports_counts_and_stop() {
        let copied = vec![("app.orders".to_string(), 5), ("app.items".to_string(), 3)];
        let detail = test_sync_history_detail(5, &copied, &["app.orders".to_string()]);
        assert!(detail.contains("row_limit=5"));
        assert!(detail.contains("copied 8 row(s) across 2 table(s)"));
        assert!(detail.contains("app.orders=5, app.items=3"));
        assert!(detail.contains("Cleared 1 target table(s) first."));
        assert!(detail.contains("not a production sync"));
        assert!(!test_sync_history_detail(5, &copied, &[]).contains("Cleared"));
    }

    fn conn() -> std::collections::HashMap<String, String> {
        HashMap::from([
            ("host".into(), "localhost".into()),
            ("port".into(), "5432".into()),
            ("database".into(), "demo".into()),
            ("username".into(), "demo".into()),
            ("password".into(), "demo".into()),
        ])
    }

    fn profile_with_mapping(mapping: TableMapping) -> CdcProfile {
        CdcProfile {
            autostart: true,
            id: Uuid::new_v4(),
            name: "Orders CDC".into(),
            description: "demo".into(),
            pattern: ProfilePattern::SourceToTarget,
            stream_platform: None,
            transform_strategy: TransformStrategy::None,
            source: SourceConfig {
                source_type: "postgres".into(),
                connection: conn(),
                capture_mode: crate::models::CaptureMode::Logical,
                schema_mode: SchemaSelectionMode::SpecificSchemas,
                table_mode: TableSelectionMode::SpecificTables,
                schemas: vec!["bookings".into()],
                tables: vec!["bookings.a".into()],
                changelog_retention_hours: None,
                timestamp_column: None,
                incrementing_column: None,
                upsert_key_columns: vec![],
                table_poll: vec![],
            },
            target: Some(TargetConfig {
                name: "warehouse".into(),
                target_type: "postgres".into(),
                connection: conn(),
                schemas: vec![],
            }),
            lake: None,
            governance: crate::models::GovernanceConfig::default(),
            sink_error_policy: crate::models::SinkErrorPolicy::FailFast,
            flow: FlowConfig {
                transforms: vec![],
                schedule: ScheduleConfig {
                    delivery_mode: DeliveryMode::Realtime,
                    interval_seconds: None,
                    full_refresh_minutes: None,
                },
                mappings: vec![mapping],
            },
            status: ProfileStatus::Draft,
            enabled: true,
            last_error: None,
            history: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tags: vec![],
            connector_name: None,
            sink_connector_name: None,
        }
    }

    #[test]
    fn source_and_sink_default_to_avro() {
        let profile = profile_with_mapping(TableMapping {
            source_schema: "bookings".into(),
            source_table: "a".into(),
            target_schema: "bookings".into(),
            target_table: "a".into(),
            identical: true,
            columns: vec![],
            pk_columns: vec![],
        });
        let src = build_debezium_config(&profile, "http://cdc-message:8081").unwrap();
        let sink = build_jdbc_sink_config(&profile, "http://cdc-message:8081").unwrap();
        for cfg in [&src, &sink] {
            assert_eq!(
                cfg.get("key.converter").unwrap(),
                "io.confluent.connect.avro.AvroConverter"
            );
            assert_eq!(
                cfg.get("value.converter").unwrap(),
                "io.confluent.connect.avro.AvroConverter"
            );
            assert_eq!(
                cfg.get("value.converter.schema.registry.url").unwrap(),
                "http://cdc-message:8081"
            );
        }
        assert_eq!(src.get("snapshot.mode").unwrap(), "initial");
        assert_eq!(
            src.get("topic.creation.default.retention.ms").unwrap(),
            "604800000"
        );
        assert_eq!(
            src.get("topic.creation.default.segment.bytes").unwrap(),
            "134217728"
        );
        assert!(!src.contains_key("topic.creation.default.retention.bytes"));
        assert!(!sink.contains_key("errors.tolerance"));
        assert!(!sink.contains_key("errors.deadletterqueue.topic.name"));
    }

    #[test]
    fn logical_sink_uses_dlq_only_when_explicitly_selected() {
        let mut profile = profile_with_mapping(TableMapping {
            source_schema: "bookings".into(),
            source_table: "a".into(),
            target_schema: "bookings".into(),
            target_table: "a".into(),
            identical: true,
            columns: vec![],
            pk_columns: vec![],
        });
        profile.sink_error_policy = SinkErrorPolicy::Dlq;

        let sink = build_jdbc_sink_config(&profile, "http://cdc-message:8081").unwrap();
        assert_eq!(sink.get("errors.tolerance").unwrap(), "all");
        assert_eq!(
            sink.get("errors.deadletterqueue.topic.name").unwrap(),
            "cdc-orders-cdc-sink-dlq"
        );
        assert_eq!(
            sink.get("errors.deadletterqueue.context.headers.enable")
                .unwrap(),
            "true"
        );
    }

    fn bookings_a_profile() -> CdcProfile {
        profile_with_mapping(TableMapping {
            source_schema: "bookings".into(),
            source_table: "a".into(),
            target_schema: "bookings".into(),
            target_table: "a".into(),
            identical: true,
            columns: vec![],
            pk_columns: vec![],
        })
    }

    fn keyless(
        table: &str,
        facts: crate::connector_catalog::KeylessTable,
    ) -> HashMap<(String, String), crate::connector_catalog::KeylessTable> {
        HashMap::from([(("bookings".to_string(), table.to_string()), facts)])
    }

    #[test]
    fn keyless_partition_gets_identity_plus_partition_key() {
        let profile = bookings_a_profile();
        let mut cfg = build_debezium_config(&profile, "http://cdc-message:8081").unwrap();
        assert!(!cfg.contains_key("message.key.columns"));
        let facts = crate::connector_catalog::KeylessTable {
            unique_key: vec![],
            identity: vec!["id".into()],
            partition_key: vec!["created_at".into(), "id".into()],
        };
        apply_message_key_columns(&mut cfg, &profile, &keyless("a", facts)).unwrap();
        assert_eq!(
            cfg.get("message.key.columns").unwrap(),
            "bookings\\.a:id,created_at"
        );
    }

    #[test]
    fn keyless_unique_index_wins_and_unselected_tables_ignored() {
        let profile = bookings_a_profile();
        let mut cfg = HashMap::new();
        let mut map = keyless(
            "a",
            crate::connector_catalog::KeylessTable {
                unique_key: vec!["code".into()],
                identity: vec!["id".into()],
                partition_key: vec![],
            },
        );
        map.insert(
            ("bookings".into(), "other".into()),
            crate::connector_catalog::KeylessTable::default(),
        );
        apply_message_key_columns(&mut cfg, &profile, &map).unwrap();
        assert_eq!(cfg.get("message.key.columns").unwrap(), "bookings\\.a:code");
    }

    #[test]
    fn keyless_without_derivable_key_is_refused_for_target_sink() {
        let profile = bookings_a_profile();
        let mut cfg = HashMap::new();
        let err = apply_message_key_columns(
            &mut cfg,
            &profile,
            &keyless("a", crate::connector_catalog::KeylessTable::default()),
        )
        .unwrap_err();
        assert!(format!("{err:?}").contains("bookings.a"));
    }

    #[test]
    fn explicit_message_key_columns_override_wins() {
        let mut profile = bookings_a_profile();
        profile
            .source
            .connection
            .insert("message_key_columns".into(), "bookings.a:x,y".into());
        let mut cfg = build_debezium_config(&profile, "http://cdc-message:8081").unwrap();
        assert_eq!(cfg.get("message.key.columns").unwrap(), "bookings.a:x,y");
        apply_message_key_columns(
            &mut cfg,
            &profile,
            &keyless("a", crate::connector_catalog::KeylessTable::default()),
        )
        .unwrap();
        assert_eq!(cfg.get("message.key.columns").unwrap(), "bookings.a:x,y");
    }

    #[test]
    fn sync_once_uses_initial_only_snapshot() {
        let mut profile = profile_with_mapping(TableMapping {
            source_schema: "bookings".into(),
            source_table: "a".into(),
            target_schema: "bookings".into(),
            target_table: "a".into(),
            identical: true,
            columns: vec![],
            pk_columns: vec![],
        });
        profile.source.capture_mode = crate::models::CaptureMode::SyncOnce;
        let cfg = build_debezium_config(&profile, "http://cdc-message:8081").unwrap();
        assert_eq!(cfg.get("snapshot.mode").unwrap(), "initial_only");
    }

    #[test]
    fn incremental_uses_jdbc_source_not_debezium() {
        let mut profile = profile_with_mapping(TableMapping {
            source_schema: "bookings".into(),
            source_table: "a".into(),
            target_schema: "bookings".into(),
            target_table: "a".into(),
            identical: true,
            columns: vec![],
            pk_columns: vec![],
        });
        profile.source.capture_mode = crate::models::CaptureMode::Incremental;
        profile.source.tables = vec!["bookings.a".into()];
        profile.source.timestamp_column = Some("updated_at".into());
        profile.source.incrementing_column = Some("id".into());
        let src = crate::incremental_cdc::build_jdbc_source_config(
            &profile,
            "http://cdc-message:8081",
            &HashMap::new(),
        )
        .unwrap();
        let sink = crate::incremental_cdc::build_incremental_sink_config(
            &profile,
            "http://cdc-message:8081",
        )
        .unwrap();
        assert_eq!(
            src.get("connector.class").unwrap(),
            "io.confluent.connect.jdbc.JdbcSourceConnector"
        );
        assert_ne!(
            src.get("connector.class").unwrap(),
            "io.debezium.connector.postgresql.PostgresConnector"
        );
        assert_eq!(sink.get("delete.enabled").unwrap(), "false");
        assert!(!sink.get("transforms").unwrap().contains("unwrap"));
    }

    #[test]
    fn identical_tables_keep_default_route() {
        let profile = profile_with_mapping(TableMapping {
            source_schema: "bookings".into(),
            source_table: "a".into(),
            target_schema: "bookings".into(),
            target_table: "a".into(),
            identical: true,
            columns: vec![],
            pk_columns: vec![],
        });
        let cfg = build_jdbc_sink_config(&profile, "http://cdc-message:8081").unwrap();
        assert_eq!(cfg.get("transforms").unwrap(), "unwrap,route");
        assert!(!cfg.contains_key("predicates"));
        assert_eq!(
            cfg.get("value.converter").unwrap(),
            "io.confluent.connect.avro.AvroConverter"
        );
        assert_eq!(
            cfg.get("value.converter.schema.registry.url").unwrap(),
            "http://cdc-message:8081"
        );
    }

    #[test]
    fn remap_table_and_columns_adds_predicates() {
        let profile = profile_with_mapping(TableMapping {
            source_schema: "bookings".into(),
            source_table: "a".into(),
            target_schema: "public".into(),
            target_table: "b".into(),
            identical: false,
            columns: vec![ColumnMapping {
                source: "id".into(),
                sources: vec![],
                target: "ticket_id".into(),
                data_type: "int".into(),
                transform: None,
                transform_args: None,
            }],
            pk_columns: vec![],
        });
        let cfg = build_jdbc_sink_config(&profile, "http://cdc-message:8081").unwrap();
        assert_eq!(cfg.get("transforms").unwrap(), "unwrap,map0,cols0,route");
        assert_eq!(cfg.get("predicates").unwrap(), "p0");
        // Debezium topic shape: {prefix}.{schema}.{table}
        assert_eq!(cfg.get("predicates.p0.pattern").unwrap(), r".*bookings\.a$");
        assert_eq!(cfg.get("transforms.map0.replacement").unwrap(), "b");
        assert_eq!(cfg.get("transforms.cols0.include").unwrap(), "id");
        assert_eq!(cfg.get("transforms.cols0.renames").unwrap(), "id:ticket_id");
        assert!(cfg
            .get("connection.url")
            .unwrap()
            .contains("currentSchema=public"));
    }

    #[test]
    fn leftover_trigger_connectors_use_the_profile_base_name() {
        let mut profile = profile_with_mapping(TableMapping {
            source_schema: "bookings".into(),
            source_table: "a".into(),
            target_schema: "bookings".into(),
            target_table: "a".into(),
            identical: true,
            columns: vec![],
            pk_columns: vec![],
        });
        profile.name = "test01".into();
        profile.connector_name = Some("cdc-test01".into());
        let leftover = leftover_connector_names(&profile);
        assert_eq!(
            leftover,
            vec![
                "cdc-test01-changelog-src".to_string(),
                "cdc-test01-initial-load".to_string(),
                "cdc-test01-join-src".to_string(),
                "cdc-test01-join-sink".to_string(),
            ]
        );
    }
}
