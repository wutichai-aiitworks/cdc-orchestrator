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

fn slug(value: &str) -> String {
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

fn jdbc_target_schema(profile: &CdcProfile) -> &str {
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

        let table = regex_escape(mapping.source_table.trim());
        let pattern = if mapping.source_schema.trim().is_empty() {
            format!(r".*\.{table}$")
        } else {
            format!(
                r".*{}\.{table}$",
                regex_escape(mapping.source_schema.trim())
            )
        };
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

async fn upsert_connector(
    state: &AppState,
    name: &str,
    config: &HashMap<String, String>,
) -> Result<(), AppError> {
    if connector_exists(state, name).await? {
        connect_request(
            state,
            Method::PUT,
            &format!("/connectors/{}/config", urlencoding::encode(name)),
            Some(serde_json::to_value(config).unwrap()),
        )
        .await?;
    } else {
        connect_request(
            state,
            Method::POST,
            "/connectors",
            Some(json!({ "name": name, "config": config })),
        )
        .await?;
    }
    Ok(())
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

async fn cleanup_leftover_connectors(state: &AppState, profile: &CdcProfile) {
    for leftover in leftover_connector_names(profile) {
        if let Err(err) = stop_connector(state, &leftover, true).await {
            tracing::warn!(connector = %leftover, error = %err, "failed to delete leftover connector");
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
    let config = match profile.source.capture_mode {
        crate::models::CaptureMode::Trigger => {
            crate::postgres_trigger_cdc::build_jdbc_source_config(profile, &registry)?
        }
        crate::models::CaptureMode::Incremental => {
            let columns = incremental_source_columns(profile).await?;
            crate::incremental_cdc::build_jdbc_source_config(profile, &registry, &columns)?
        }
        _ => build_debezium_config(profile, &registry)?,
    };
    cleanup_leftover_connectors(state, profile).await;
    upsert_connector(state, &name, &config).await?;
    profile.connector_name = Some(name.clone());

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

async fn wait_for_stopped(state: &AppState, name: &str) -> Result<(), AppError> {
    for _ in 0..40 {
        let status = connect_request(
            state,
            Method::GET,
            &format!("/connectors/{}/status", urlencoding::encode(name)),
            None,
        )
        .await?;
        let state_str = status
            .get("connector")
            .and_then(|c| c.get("state"))
            .and_then(|s| s.as_str())
            .unwrap_or("");
        if state_str == "STOPPED" {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err(AppError::Upstream(format!(
        "connector '{name}' did not reach STOPPED in time; resync aborted before touching offsets"
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
        .map_err(|e| AppError::BadRequest(format!("failed to clear target tables: {e}")))?;
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
pub async fn reconcile_resync(
    state: &AppState,
    profile: &mut CdcProfile,
    clear_target: bool,
) -> Result<(), AppError> {
    let source_name = connector_name_for(profile);
    if !connector_exists(state, &source_name).await? {
        return Err(AppError::BadRequest(format!(
            "connector '{source_name}' does not exist yet — start the pipeline before resyncing"
        )));
    }
    let sink_name = if profile.has_managed_sink() {
        let name = sink_connector_name_for(profile);
        connector_exists(state, &name).await?.then_some(name)
    } else {
        None
    };
    let trigger_snapshot = profile.source.capture_mode == crate::models::CaptureMode::Trigger
        && profile.pattern == ProfilePattern::SourceToTarget;

    // Stop the sink first so it cannot apply rows while the target is being cleared.
    if let Some(sink) = sink_name.as_deref() {
        halt_connector(state, sink).await?;
    }
    halt_connector(state, &source_name).await?;
    wait_for_stopped(state, &source_name).await?;

    let cleared = if clear_target {
        truncate_target_tables(profile).await?
    } else {
        Vec::new()
    };

    let snapshot = if trigger_snapshot {
        Some(crate::postgres_trigger_cdc::snapshot_source_tables(profile, !clear_target).await?)
    } else {
        None
    };

    // Changelog replay after a snapshot would overwrite current source rows with
    // leftover events. Incremental / logical pipelines still need the offset reset.
    if !trigger_snapshot {
        reset_connector_offsets(state, &source_name).await?;
    }

    resume_connector(state, &source_name).await?;
    if let Some(sink) = sink_name.as_deref() {
        let _ = resume_connector(state, sink).await;
    }

    profile.connector_name = Some(source_name);
    if sink_name.is_some() {
        profile.sink_connector_name = sink_name;
    }
    profile.status = ProfileStatus::Running;
    profile.updated_at = Utc::now();
    profile.last_error = None;
    profile.push_history(HistoryItem {
        timestamp: Utc::now(),
        level: "info".to_string(),
        event: "profile_resynced".to_string(),
        detail: resync_history_detail(snapshot.as_deref(), &cleared),
    });
    Ok(())
}

fn build_managed_sink_config(
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
        "resync" => reconcile_resync(state, profile, false).await,
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
