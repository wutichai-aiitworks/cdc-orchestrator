//! Query-based incremental capture when WAL/binlog/journal is unavailable
//! and table triggers are too expensive (large tables).
//!
//! JDBC source polls selected tables on a timestamp and/or incrementing
//! watermark. The JDBC sink upserts into the target. Hard deletes are not
//! captured — use trigger or logical CDC for those tables.

use std::collections::HashMap;

use crate::error::AppError;
use crate::models::{
    CaptureMode, CdcProfile, DeliveryMode, ProfilePattern, ProfileUpsertRequest, SourceConfig,
};
use crate::postgres_trigger_cdc::{trigger_poll_interval_ms, TRIGGER_BATCH_MAX_ROWS};
use crate::reconcile::{self, source_table_include_list, topic_prefix_for};

pub fn validate_profile(payload: &ProfileUpsertRequest) -> Result<(), AppError> {
    if payload.source.capture_mode != CaptureMode::Incremental {
        return Ok(());
    }
    crate::trigger_cdc::Vendor::parse(&payload.source.source_type)?;
    if payload
        .source
        .tables
        .iter()
        .all(|item| item.trim().is_empty())
    {
        return Err(AppError::BadRequest(
            "incremental capture needs an explicit table list (schema.table)".into(),
        ));
    }
    let timestamp = opt_column(payload.source.timestamp_column.as_deref())?;
    let incrementing = opt_column(payload.source.incrementing_column.as_deref())?;
    if timestamp.is_none() && incrementing.is_none() {
        return Err(AppError::BadRequest(
            "incremental capture needs timestamp_column (e.g. updated_at) and/or incrementing_column (e.g. id)".into(),
        ));
    }
    let pattern = payload
        .pattern
        .clone()
        .unwrap_or(ProfilePattern::SourceToTarget);
    if pattern == ProfilePattern::SourceToLake || pattern == ProfilePattern::SourceToOlake {
        return Err(AppError::BadRequest(
            "incremental capture does not support source_to_lake or source_to_olake; use source_to_target or source_to_stream".into(),
        ));
    }
    if pattern == ProfilePattern::SourceToTarget {
        if incrementing.is_none() {
            return Err(AppError::BadRequest(
                "incremental replica upsert needs incrementing_column (usually the primary key, e.g. id). Hard deletes are not captured.".into(),
            ));
        }
        match payload.target.as_ref() {
            None => {
                return Err(AppError::BadRequest(
                    "incremental capture requires a target for source_to_target".into(),
                ));
            }
            Some(target)
                if !matches!(
                    target.target_type.trim().to_ascii_lowercase().as_str(),
                    "postgres" | "postgresql" | "mysql" | "mariadb" | "sqlserver" | "oracle"
                ) =>
            {
                return Err(AppError::BadRequest(format!(
                    "incremental capture does not support target_type '{}'",
                    target.target_type
                )));
            }
            _ => {}
        }
    }
    if payload.flow.schedule.delivery_mode == DeliveryMode::MicroBatch
        && payload.flow.schedule.interval_seconds.unwrap_or(0) == 0
    {
        return Err(AppError::BadRequest(
            "incremental capture in micro_batch mode requires interval_seconds > 0".into(),
        ));
    }
    Ok(())
}

pub fn jdbc_mode(source: &SourceConfig) -> Result<&'static str, AppError> {
    let timestamp = opt_column(source.timestamp_column.as_deref())?;
    let incrementing = opt_column(source.incrementing_column.as_deref())?;
    match (timestamp, incrementing) {
        (Some(_), Some(_)) => Ok("timestamp+incrementing"),
        (Some(_), None) => Ok("timestamp"),
        (None, Some(_)) => Ok("incrementing"),
        (None, None) => Err(AppError::BadRequest(
            "incremental capture needs timestamp_column and/or incrementing_column".into(),
        )),
    }
}

/// True when the source names columns in a different case than the target expects.
///
/// Oracle reports every unquoted identifier in UPPERCASE, while this project's Postgres
/// targets use folded-lowercase columns — the same convention the trigger changelog
/// follows when it emits lowercase JSON keys. Without a rename the sink quotes `"ISBN"`,
/// fails to match `isbn`, and tries to ALTER the table to add the column.
pub fn needs_lowercase_fields(profile: &CdcProfile) -> bool {
    let source_is_oracle = profile
        .source
        .source_type
        .trim()
        .eq_ignore_ascii_case("oracle");
    let target_is_postgres = profile
        .target
        .as_ref()
        .map(|t| {
            let t = t.target_type.trim().to_ascii_lowercase();
            t == "postgres" || t == "postgresql"
        })
        .unwrap_or(false);
    source_is_oracle && target_is_postgres
}

/// Build the `ReplaceField$Value` rename list that folds source columns to lowercase.
/// Columns already lowercase are skipped so the transform stays a no-op for them.
fn lowercase_renames(columns: &HashMap<String, Vec<String>>) -> String {
    let mut renames: Vec<String> = Vec::new();
    for names in columns.values() {
        for name in names {
            let lower = name.to_ascii_lowercase();
            if lower == *name {
                continue;
            }
            let pair = format!("{name}:{lower}");
            if !renames.contains(&pair) {
                renames.push(pair);
            }
        }
    }
    renames.sort();
    renames.join(",")
}

pub fn build_jdbc_source_config(
    profile: &CdcProfile,
    schema_registry_url: &str,
    columns: &HashMap<String, Vec<String>>,
) -> Result<HashMap<String, String>, AppError> {
    if profile.source.capture_mode != CaptureMode::Incremental {
        return Err(AppError::BadRequest(
            "JDBC table poller is only used for capture_mode=incremental".into(),
        ));
    }
    let host = profile
        .source
        .connection
        .get("host")
        .map(|s| s.trim())
        .unwrap_or("");
    let port = profile
        .source
        .connection
        .get("port")
        .map(|s| s.trim())
        .unwrap_or("");
    let database = req(&profile.source.connection, "database")?;
    let username = profile
        .source
        .connection
        .get("username")
        .map(|s| s.trim())
        .unwrap_or("");
    let password = profile
        .source
        .connection
        .get("password")
        .map(|s| s.as_str())
        .unwrap_or("");
    let vendor = crate::trigger_cdc::Vendor::parse(&profile.source.source_type)?;
    let tables = source_table_include_list(profile);
    if tables.is_empty() {
        return Err(AppError::BadRequest(
            "incremental capture needs an explicit table list (schema.table)".into(),
        ));
    }
    let include: Vec<String> = tables.iter().map(|t| regex_escape(t.trim())).collect();
    let timestamp = opt_column(profile.source.timestamp_column.as_deref())?;
    let incrementing = opt_column(profile.source.incrementing_column.as_deref())?;
    let mode = jdbc_mode(&profile.source)?;
    let prefix = topic_prefix_for(profile);

    let mut cfg = HashMap::new();
    cfg.insert(
        "connector.class".into(),
        "io.confluent.connect.jdbc.JdbcSourceConnector".into(),
    );
    cfg.insert("tasks.max".into(), "1".into());
    cfg.insert(
        "connection.url".into(),
        crate::trigger_cdc::jdbc_source_url(vendor, host, port, database)?,
    );
    cfg.insert("connection.user".into(), username.into());
    cfg.insert("connection.password".into(), password.into());
    cfg.insert("mode".into(), mode.into());
    cfg.insert("table.include.list".into(), include.join(","));
    if let Some(column) = timestamp {
        cfg.insert("timestamp.columns.mapping".into(), format!(".*:[{column}]"));
    }
    if let Some(column) = incrementing {
        cfg.insert("incrementing.column.mapping".into(), format!(".*:{column}"));
    }
    if let Some(schema) = single_schema(&tables) {
        cfg.insert("schema.pattern".into(), schema.into());
    }
    cfg.insert("numeric.mapping".into(), "best_fit".into());
    cfg.insert("validate.non.null".into(), "true".into());
    cfg.insert(
        "poll.interval.ms".into(),
        trigger_poll_interval_ms(profile).to_string(),
    );
    cfg.insert("batch.max.rows".into(), TRIGGER_BATCH_MAX_ROWS.to_string());
    // JDBC source topics are `{prefix}{table}` (no schema). Trailing dot matches
    // the Debezium-style `{prefix}.{table}` regex the sink already uses.
    cfg.insert("topic.prefix".into(), format!("{prefix}."));
    if needs_lowercase_fields(profile) {
        let renames = lowercase_renames(columns);
        if !renames.is_empty() {
            cfg.insert("transforms".into(), "lowercaseFields".into());
            cfg.insert(
                "transforms.lowercaseFields.type".into(),
                "org.apache.kafka.connect.transforms.ReplaceField$Value".into(),
            );
            cfg.insert("transforms.lowercaseFields.renames".into(), renames);
        }
    }
    reconcile::apply_topic_creation_defaults(&mut cfg);
    reconcile::apply_avro_converters(&mut cfg, schema_registry_url);
    Ok(cfg)
}

pub fn build_incremental_sink_config(
    profile: &CdcProfile,
    schema_registry_url: &str,
) -> Result<HashMap<String, String>, AppError> {
    let target = profile.target.as_ref().ok_or_else(|| {
        AppError::BadRequest("target is required for incremental capture sink".into())
    })?;
    let host = req(&target.connection, "host")?;
    let port = req(&target.connection, "port")?;
    let database = req(&target.connection, "database")?;
    let username = req(&target.connection, "username")?;
    let password = req(&target.connection, "password")?;
    let db_type = target.target_type.trim().to_ascii_lowercase();
    let prefix = topic_prefix_for(profile);
    let pk = opt_column(profile.source.incrementing_column.as_deref())?.ok_or_else(|| {
        AppError::BadRequest(
            "incremental replica upsert needs incrementing_column (usually the primary key)".into(),
        )
    })?;

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
                "unsupported target_type for incremental JDBC sink: {other}"
            )));
        }
    };

    let table_names = table_names_only(profile);
    let topics_regex = if table_names.is_empty() {
        format!("{prefix}\\..*")
    } else {
        let parts: Vec<String> = table_names.iter().map(|t| regex_escape(t)).collect();
        format!("{prefix}\\.({})", parts.join("|"))
    };

    let mut cfg = HashMap::new();
    cfg.insert(
        "connector.class".into(),
        "io.debezium.connector.jdbc.JdbcSinkConnector".into(),
    );
    cfg.insert("tasks.max".into(), "1".into());
    cfg.insert("topics.regex".into(), topics_regex);
    cfg.insert("connection.url".into(), jdbc_url);
    cfg.insert("connection.username".into(), username.into());
    cfg.insert("connection.password".into(), password.into());
    cfg.insert("insert.mode".into(), "upsert".into());
    cfg.insert("delete.enabled".into(), "false".into());
    cfg.insert("primary.key.mode".into(), "record_value".into());
    // The source renames columns to lowercase before publishing, so the key field the
    // sink looks up in the record must be folded the same way.
    let pk_field = if needs_lowercase_fields(profile) {
        pk.to_ascii_lowercase()
    } else {
        pk.to_string()
    };
    cfg.insert("primary.key.fields".into(), pk_field);
    cfg.insert("schema.evolution".into(), "basic".into());
    cfg.insert("quote.identifiers".into(), "true".into());
    cfg.insert("table.name.format".into(), "${topic}".into());
    reconcile::apply_sink_transforms(&mut cfg, profile, false);
    reconcile::apply_sink_error_policy(&mut cfg, profile);
    reconcile::apply_avro_converters(&mut cfg, schema_registry_url);
    Ok(cfg)
}

fn table_names_only(profile: &CdcProfile) -> Vec<String> {
    let mut out = Vec::new();
    for qualified in source_table_include_list(profile) {
        let name = qualified
            .rsplit_once('.')
            .map(|(_, table)| table.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| qualified.trim().to_string());
        if !name.is_empty() && !out.iter().any(|existing: &String| existing == &name) {
            out.push(name);
        }
    }
    out
}

fn single_schema(tables: &[String]) -> Option<&str> {
    let mut found: Option<&str> = None;
    for qualified in tables {
        let schema = qualified.split_once('.')?.0.trim();
        if schema.is_empty() {
            return None;
        }
        match found {
            None => found = Some(schema),
            Some(prev) if !prev.eq_ignore_ascii_case(schema) => return None,
            _ => {}
        }
    }
    found
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

fn opt_column(value: Option<&str>) -> Result<Option<&str>, AppError> {
    let Some(raw) = value.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if raw.len() > 128
        || !raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || raw.starts_with(|c: char| c.is_ascii_digit())
    {
        return Err(AppError::BadRequest(format!(
            "invalid incremental column name: {raw}"
        )));
    }
    Ok(Some(raw))
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

fn req<'a>(m: &'a HashMap<String, String>, key: &str) -> Result<&'a str, AppError> {
    m.get(key)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest(format!("missing connection field: {key}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        DeliveryMode, FlowConfig, ProfileStatus, ScheduleConfig, SchemaSelectionMode,
        TableSelectionMode, TargetConfig, TransformStrategy,
    };
    use chrono::Utc;
    use uuid::Uuid;

    fn profile() -> CdcProfile {
        CdcProfile {
            id: Uuid::new_v4(),
            name: "Large Tables".into(),
            description: String::new(),
            pattern: ProfilePattern::SourceToTarget,
            stream_platform: None,
            transform_strategy: TransformStrategy::None,
            source: SourceConfig {
                source_type: "postgres".into(),
                connection: HashMap::from([
                    ("host".into(), "src".into()),
                    ("port".into(), "5432".into()),
                    ("database".into(), "demo".into()),
                    ("username".into(), "demo".into()),
                    ("password".into(), "demo".into()),
                ]),
                capture_mode: CaptureMode::Incremental,
                schema_mode: SchemaSelectionMode::SpecificSchemas,
                table_mode: TableSelectionMode::SpecificTables,
                schemas: vec!["bookings".into()],
                tables: vec!["bookings.tickets".into(), "bookings.airports_data".into()],
                changelog_retention_hours: None,
                timestamp_column: Some("updated_at".into()),
                incrementing_column: Some("id".into()),
            },
            target: Some(TargetConfig {
                name: "replica".into(),
                target_type: "postgres".into(),
                connection: HashMap::from([
                    ("host".into(), "dst".into()),
                    ("port".into(), "5432".into()),
                    ("database".into(), "demo".into()),
                    ("username".into(), "demo".into()),
                    ("password".into(), "demo".into()),
                ]),
                schemas: vec![],
            }),
            lake: None,
            governance: crate::models::GovernanceConfig::default(),
            sink_error_policy: crate::models::SinkErrorPolicy::FailFast,
            flow: FlowConfig {
                transforms: vec![],
                schedule: ScheduleConfig {
                    delivery_mode: DeliveryMode::MicroBatch,
                    interval_seconds: Some(300),
                    full_refresh_minutes: None,
                },
                mappings: vec![],
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
    fn timestamp_plus_incrementing_is_default_when_both_set() {
        let p = profile();
        assert_eq!(jdbc_mode(&p.source).unwrap(), "timestamp+incrementing");
        let cfg = build_jdbc_source_config(&p, "http://cdc-message:8081", &HashMap::new()).unwrap();
        assert_eq!(
            cfg.get("connector.class").unwrap(),
            "io.confluent.connect.jdbc.JdbcSourceConnector"
        );
        assert_eq!(cfg.get("mode").unwrap(), "timestamp+incrementing");
        assert_eq!(
            cfg.get("table.include.list").unwrap(),
            "bookings\\.tickets,bookings\\.airports_data"
        );
        assert_eq!(
            cfg.get("timestamp.columns.mapping").unwrap(),
            ".*:[updated_at]"
        );
        assert_eq!(cfg.get("incrementing.column.mapping").unwrap(), ".*:id");
        assert_eq!(cfg.get("poll.interval.ms").unwrap(), "300000");
        assert_eq!(cfg.get("batch.max.rows").unwrap(), "500");
        assert_eq!(cfg.get("schema.pattern").unwrap(), "bookings");
        assert_eq!(cfg.get("topic.prefix").unwrap(), "large_tables.");
        assert!(!cfg.contains_key("timestamp.column.name"));
        assert!(!cfg.contains_key("incrementing.column.name"));
    }

    #[test]
    fn incrementing_only_is_append_mode() {
        let mut p = profile();
        p.source.timestamp_column = None;
        assert_eq!(jdbc_mode(&p.source).unwrap(), "incrementing");
        let cfg = build_jdbc_source_config(&p, "http://cdc-message:8081", &HashMap::new()).unwrap();
        assert_eq!(cfg.get("mode").unwrap(), "incrementing");
        assert!(!cfg.contains_key("timestamp.columns.mapping"));
        assert_eq!(cfg.get("incrementing.column.mapping").unwrap(), ".*:id");
    }

    #[test]
    fn oracle_source_folds_uppercase_columns_to_lowercase() {
        let mut p = profile();
        p.source.source_type = "oracle".into();
        p.source.tables = vec!["BCS.G_BOOKS".into()];
        p.source.incrementing_column = Some("BOOK_ID".into());
        p.source.timestamp_column = None;
        let columns = HashMap::from([(
            "BCS.G_BOOKS".to_string(),
            vec!["BOOK_ID".to_string(), "ISBN".to_string()],
        )]);
        let src = build_jdbc_source_config(&p, "http://cdc-message:8081", &columns).unwrap();
        assert_eq!(src.get("transforms").unwrap(), "lowercaseFields");
        assert_eq!(
            src.get("transforms.lowercaseFields.renames").unwrap(),
            "BOOK_ID:book_id,ISBN:isbn"
        );
        // The sink reads the key out of the already-renamed record.
        let sink = build_incremental_sink_config(&p, "http://cdc-message:8081").unwrap();
        assert_eq!(sink.get("primary.key.fields").unwrap(), "book_id");
    }

    #[test]
    fn postgres_source_keeps_column_case_untouched() {
        let cfg = build_jdbc_source_config(&profile(), "http://cdc-message:8081", &HashMap::new())
            .unwrap();
        assert!(!cfg.contains_key("transforms"));
    }

    #[test]
    fn sink_upserts_without_unwrap_or_deletes() {
        let cfg = build_incremental_sink_config(&profile(), "http://cdc-message:8081").unwrap();
        assert_eq!(
            cfg.get("connector.class").unwrap(),
            "io.debezium.connector.jdbc.JdbcSinkConnector"
        );
        assert_eq!(cfg.get("insert.mode").unwrap(), "upsert");
        assert_eq!(cfg.get("delete.enabled").unwrap(), "false");
        assert_eq!(cfg.get("primary.key.mode").unwrap(), "record_value");
        assert_eq!(cfg.get("primary.key.fields").unwrap(), "id");
        assert_eq!(
            cfg.get("topics.regex").unwrap(),
            "large_tables\\.(tickets|airports_data)"
        );
        let transforms = cfg.get("transforms").unwrap();
        assert!(!transforms.contains("unwrap"));
        assert!(transforms.contains("route"));
        assert!(!cfg.contains_key("transforms.unwrap.type"));
        assert_eq!(
            cfg.get("value.converter").unwrap(),
            "io.confluent.connect.avro.AvroConverter"
        );
        assert!(!cfg.contains_key("errors.tolerance"));
    }

    #[test]
    fn sink_dlq_policy_is_explicit_opt_in() {
        let mut p = profile();
        p.sink_error_policy = crate::models::SinkErrorPolicy::Dlq;
        let cfg = build_incremental_sink_config(&p, "http://cdc-message:8081").unwrap();
        assert_eq!(cfg.get("errors.tolerance").unwrap(), "all");
        assert_eq!(
            cfg.get("errors.deadletterqueue.topic.name").unwrap(),
            "cdc-large-tables-sink-dlq"
        );
    }

    #[test]
    fn rejects_sql_in_column_names() {
        let mut p = profile();
        p.source.timestamp_column = Some("updated_at;drop".into());
        assert!(build_jdbc_source_config(&p, "http://cdc-message:8081", &HashMap::new()).is_err());
    }
}
