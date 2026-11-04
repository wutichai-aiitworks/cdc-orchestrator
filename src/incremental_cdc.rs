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
    TablePollRule, TablePollStrategy,
};
use crate::postgres_trigger_cdc::{trigger_poll_interval_ms, TRIGGER_BATCH_MAX_ROWS};
use crate::reconcile::{self, source_table_include_list, topic_prefix_for};

fn validate_incremental_replica_target(
    payload: &ProfileUpsertRequest,
    has_watermark_tables: bool,
    timestamp: &Option<&str>,
    incrementing: &Option<&str>,
    upsert_keys_early: &[String],
) -> Result<(), AppError> {
    let pattern = payload
        .pattern
        .clone()
        .unwrap_or(ProfilePattern::SourceToTarget);
    if pattern == ProfilePattern::SourceToLake || pattern == ProfilePattern::SourceToOlake {
        return Err(AppError::BadRequest(
            "incremental capture does not support source_to_lake or source_to_olake; use source_to_target or source_to_stream".into(),
        ));
    }
    if pattern != ProfilePattern::SourceToTarget {
        return Ok(());
    }
    if incrementing.is_none() && upsert_keys_early.is_empty() {
        return Err(AppError::BadRequest(
            "incremental replica upsert needs incrementing_column (numeric poll) and/or upsert_key_columns (e.g. BRANCH_NO,PRO_CODE). Hard deletes are not captured.".into(),
        ));
    }
    if has_watermark_tables && incrementing.is_none() && timestamp.is_none() {
        return Err(AppError::BadRequest(
            "incremental replica with text/composite upsert_key_columns requires timestamp_column (e.g. UPDATE_DATE) for JDBC poll".into(),
        ));
    }
    match payload.target.as_ref() {
        None => Err(AppError::BadRequest(
            "incremental capture requires a target for source_to_target".into(),
        )),
        Some(target)
            if !matches!(
                target.target_type.trim().to_ascii_lowercase().as_str(),
                "postgres" | "postgresql" | "mysql" | "mariadb" | "sqlserver" | "oracle"
            ) =>
        {
            Err(AppError::BadRequest(format!(
                "incremental capture does not support target_type '{}'",
                target.target_type
            )))
        }
        _ => Ok(()),
    }
}

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
    validate_table_poll(&payload.source)?;
    // Watermark columns only matter when at least one table still polls on them; a
    // pipeline made only of rolling-window / full-copy tables needs just upsert keys.
    let has_watermark_tables = payload.source.tables.iter().any(|t| {
        let t = t.trim();
        !t.is_empty()
            && !payload.source.table_poll.iter().any(|rule| {
                rule.strategy != TablePollStrategy::Watermark
                    && rule.table.trim().eq_ignore_ascii_case(t)
            })
    });
    let timestamp = opt_column(payload.source.timestamp_column.as_deref())?;
    let incrementing = opt_column(payload.source.incrementing_column.as_deref())?;
    let upsert_keys_early = normalized_upsert_keys(&payload.source);
    if timestamp.is_none() && incrementing.is_none() && upsert_keys_early.is_empty() {
        return Err(AppError::BadRequest(
            "incremental capture needs timestamp_column (e.g. updated_at) and/or incrementing_column (e.g. id)".into(),
        ));
    }
    validate_incremental_replica_target(
        payload,
        has_watermark_tables,
        &timestamp,
        &incrementing,
        &upsert_keys_early,
    )?;
    if payload.flow.schedule.delivery_mode == DeliveryMode::MicroBatch
        && payload.flow.schedule.interval_seconds.unwrap_or(0) == 0
    {
        return Err(AppError::BadRequest(
            "incremental capture in micro_batch mode requires interval_seconds > 0".into(),
        ));
    }
    Ok(())
}

/// Fill `source.upsert_key_columns` from `flow.mappings[].pk_columns` when the profile
/// was saved by an older UI/orchestrator pair that only persisted mapping PKs.
pub fn hydrate_upsert_keys_from_mappings(profile: &mut CdcProfile) {
    if !profile.source.upsert_key_columns.is_empty() {
        return;
    }
    if profile.source.capture_mode != CaptureMode::Incremental {
        return;
    }
    for mapping in &profile.flow.mappings {
        for key in &mapping.pk_columns {
            let key = key.trim();
            if key.is_empty() {
                continue;
            }
            if profile
                .source
                .upsert_key_columns
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(key))
            {
                continue;
            }
            profile.source.upsert_key_columns.push(key.to_string());
        }
    }
}

fn normalized_upsert_keys(source: &SourceConfig) -> Vec<String> {
    source
        .upsert_key_columns
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn sink_upsert_key_fields(profile: &CdcProfile) -> Result<Vec<String>, AppError> {
    let keys = normalized_upsert_keys(&profile.source);
    if !keys.is_empty() {
        return Ok(if needs_lowercase_fields(profile) {
            keys.iter().map(|k| k.to_ascii_lowercase()).collect()
        } else {
            keys
        });
    }
    let inc = opt_column(profile.source.incrementing_column.as_deref())?.ok_or_else(|| {
        AppError::BadRequest(
            "incremental sink upsert needs upsert_key_columns or incrementing_column".into(),
        )
    })?;
    Ok(vec![if needs_lowercase_fields(profile) {
        inc.to_ascii_lowercase()
    } else {
        inc.to_string()
    }])
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

/// Connection, cadence, type mapping, column-case and converter settings shared by every
/// JDBC source connector of an incremental pipeline (table mode and per-table query mode).
fn base_source_config(
    profile: &CdcProfile,
    schema_registry_url: &str,
    columns: &HashMap<String, Vec<String>>,
) -> Result<HashMap<String, String>, AppError> {
    if profile.source.capture_mode != CaptureMode::Incremental {
        return Err(AppError::BadRequest(
            "JDBC table poller is only used for capture_mode=incremental".into(),
        ));
    }
    let conn = &profile.source.connection;
    let field = |key: &str| conn.get(key).map(|s| s.trim()).unwrap_or("");
    let database = req(conn, "database")?;
    let vendor = crate::trigger_cdc::Vendor::parse(&profile.source.source_type)?;

    let mut cfg = HashMap::new();
    cfg.insert(
        "connector.class".into(),
        "io.confluent.connect.jdbc.JdbcSourceConnector".into(),
    );
    cfg.insert("tasks.max".into(), "1".into());
    cfg.insert(
        "connection.url".into(),
        crate::trigger_cdc::jdbc_source_url(vendor, field("host"), field("port"), database)?,
    );
    cfg.insert("connection.user".into(), field("username").into());
    cfg.insert(
        "connection.password".into(),
        conn.get("password")
            .map(|s| s.as_str())
            .unwrap_or("")
            .into(),
    );
    cfg.insert("numeric.mapping".into(), "best_fit".into());
    cfg.insert(
        "poll.interval.ms".into(),
        trigger_poll_interval_ms(profile).to_string(),
    );
    cfg.insert("batch.max.rows".into(), TRIGGER_BATCH_MAX_ROWS.to_string());
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

/// Table-mode JDBC source for the pipeline's **watermark** tables (every selected table
/// without a rolling-window / full-copy rule).
pub fn build_jdbc_source_config(
    profile: &CdcProfile,
    schema_registry_url: &str,
    columns: &HashMap<String, Vec<String>>,
) -> Result<HashMap<String, String>, AppError> {
    let tables = watermark_tables(profile);
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

    let mut cfg = base_source_config(profile, schema_registry_url, columns)?;
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
    cfg.insert("validate.non.null".into(), "true".into());
    // JDBC source topics are `{prefix}{table}` (no schema). Trailing dot matches
    // the Debezium-style `{prefix}.{table}` regex the sink already uses.
    cfg.insert("topic.prefix".into(), format!("{prefix}."));
    Ok(cfg)
}

/// Query-mode, `mode=bulk` JDBC source for one rolling-window / full-copy table.
///
/// Bulk mode keeps no offset: every poll re-runs the query and publishes its rows. The
/// topic is set to exactly what table mode would produce (`{prefix}.{TABLE}`), so the
/// sink, table status and topic tooling treat both strategies the same.
pub fn build_poll_connector_config(
    profile: &CdcProfile,
    schema_registry_url: &str,
    columns: &HashMap<String, Vec<String>>,
    qualified: &str,
    rule: &TablePollRule,
) -> Result<HashMap<String, String>, AppError> {
    let vendor = crate::trigger_cdc::Vendor::parse(&profile.source.source_type)?;
    let query = poll_query(vendor, qualified, rule)?;
    let (_, table) = split_qualified_table(qualified)?;
    let mut cfg = base_source_config(profile, schema_registry_url, columns)?;
    cfg.insert("mode".into(), "bulk".into());
    cfg.insert("query".into(), query);
    cfg.insert(
        "topic.prefix".into(),
        format!("{}.{table}", topic_prefix_for(profile)),
    );
    Ok(cfg)
}

/// One Kafka Connect source connector an incremental pipeline needs.
#[derive(Debug, Clone)]
pub struct SourceConnectorSpec {
    pub name: String,
    pub config: HashMap<String, String>,
}

/// Every source connector of an incremental pipeline, primary first.
///
/// The primary keeps the pipeline's connector name (`primary_name`), so status, pause,
/// stop and the watcher keep working on one well-known connector. It is the table-mode
/// watermark connector when any table uses a watermark; otherwise the first
/// rolling-window / full-copy table. Each other custom table gets
/// [`poll_connector_name`].
pub fn source_connector_plan(
    profile: &CdcProfile,
    primary_name: &str,
    schema_registry_url: &str,
    columns: &HashMap<String, Vec<String>>,
) -> Result<Vec<SourceConnectorSpec>, AppError> {
    let mut plan = Vec::new();
    if !watermark_tables(profile).is_empty() {
        plan.push(SourceConnectorSpec {
            name: primary_name.to_string(),
            config: build_jdbc_source_config(profile, schema_registry_url, columns)?,
        });
    }
    for (qualified, rule) in custom_poll_tables(profile) {
        let name = if plan.is_empty() {
            primary_name.to_string()
        } else {
            poll_connector_name(primary_name, &qualified)
        };
        plan.push(SourceConnectorSpec {
            name,
            config: build_poll_connector_config(
                profile,
                schema_registry_url,
                columns,
                &qualified,
                &rule,
            )?,
        });
    }
    if plan.is_empty() {
        return Err(AppError::BadRequest(
            "incremental capture needs an explicit table list (schema.table)".into(),
        ));
    }
    Ok(plan)
}

/// `{primary}-poll-{schema-table}` for an extra per-table source connector.
pub fn poll_connector_name(primary_name: &str, qualified: &str) -> String {
    format!(
        "{}{}",
        poll_connector_prefix(primary_name),
        reconcile::slug(qualified)
    )
}

/// Name prefix shared by a pipeline's extra per-table source connectors.
pub fn poll_connector_prefix(primary_name: &str) -> String {
    format!("{primary_name}-poll-")
}

/// The non-watermark rule for `qualified`, if any (case-insensitive table match).
pub fn table_poll_rule<'a>(profile: &'a CdcProfile, qualified: &str) -> Option<&'a TablePollRule> {
    let wanted = qualified.trim();
    profile.source.table_poll.iter().find(|rule| {
        rule.strategy != TablePollStrategy::Watermark
            && rule.table.trim().eq_ignore_ascii_case(wanted)
    })
}

/// Selected tables that poll on the watermark columns (no rolling-window / full-copy rule).
pub fn watermark_tables(profile: &CdcProfile) -> Vec<String> {
    source_table_include_list(profile)
        .into_iter()
        .filter(|t| table_poll_rule(profile, t).is_none())
        .collect()
}

/// Selected tables with a rolling-window / full-copy rule, in table-list order.
pub fn custom_poll_tables(profile: &CdcProfile) -> Vec<(String, TablePollRule)> {
    source_table_include_list(profile)
        .into_iter()
        .filter_map(|t| table_poll_rule(profile, &t).cloned().map(|rule| (t, rule)))
        .collect()
}

/// True when any selected table uses a rolling window, whose target holds rows the
/// connector never re-reads (anything older than the window).
pub fn has_rolling_window_tables(profile: &CdcProfile) -> bool {
    profile.source.capture_mode == CaptureMode::Incremental
        && custom_poll_tables(profile)
            .iter()
            .any(|(_, rule)| rule.strategy == TablePollStrategy::RollingWindow)
}

/// Default rolling window: today plus yesterday.
pub const DEFAULT_WINDOW_DAYS: u32 = 1;
/// Upper bound for `window_days`; longer windows re-read too much on every poll.
pub const MAX_WINDOW_DAYS: u32 = 31;

/// The `SELECT` a rolling-window / full-copy connector runs on every poll.
pub fn poll_query(
    vendor: crate::trigger_cdc::Vendor,
    qualified: &str,
    rule: &TablePollRule,
) -> Result<String, AppError> {
    let (schema, table) = split_qualified_table(qualified)?;
    let from = format!(
        "{}.{}",
        quote_source_ident(vendor, schema),
        quote_source_ident(vendor, table)
    );
    match rule.strategy {
        TablePollStrategy::FullCopy => Ok(format!("SELECT * FROM {from}")),
        TablePollStrategy::RollingWindow => {
            let column = opt_column(rule.window_column.as_deref())?.ok_or_else(|| {
                AppError::BadRequest(format!(
                    "rolling window on {qualified} needs window_column (e.g. UPDATE_DATE)"
                ))
            })?;
            let days = rule.window_days.unwrap_or(DEFAULT_WINDOW_DAYS);
            if days > MAX_WINDOW_DAYS {
                return Err(AppError::BadRequest(format!(
                    "window_days on {qualified} must be between 0 and {MAX_WINDOW_DAYS}"
                )));
            }
            let predicate = window_predicate(vendor, &quote_source_ident(vendor, column), days)?;
            Ok(format!("SELECT * FROM {from} WHERE {predicate}"))
        }
        TablePollStrategy::Watermark => Err(AppError::BadRequest(format!(
            "{qualified} uses the watermark strategy; it has no poll query"
        ))),
    }
}

/// `column >= start of (today - days)` in the source's own SQL dialect. Comparing a bare
/// column against a constant expression keeps an index on that column usable.
fn window_predicate(
    vendor: crate::trigger_cdc::Vendor,
    column: &str,
    days: u32,
) -> Result<String, AppError> {
    use crate::trigger_cdc::Vendor;
    Ok(match vendor {
        Vendor::Oracle => format!("{column} >= TRUNC(SYSDATE) - {days}"),
        Vendor::Postgres => format!("{column} >= CURRENT_DATE - {days}"),
        Vendor::Mysql | Vendor::Mariadb => {
            format!("{column} >= CURRENT_DATE - INTERVAL {days} DAY")
        }
        Vendor::SqlServer => {
            format!("{column} >= DATEADD(day, -{days}, CAST(GETDATE() AS date))")
        }
        Vendor::Db2 => format!("{column} >= CURRENT DATE - {days} DAYS"),
        Vendor::Sqlite | Vendor::Informix => {
            return Err(AppError::BadRequest(format!(
                "rolling window is not supported for {} sources",
                vendor.label()
            )));
        }
    })
}

fn quote_source_ident(vendor: crate::trigger_cdc::Vendor, ident: &str) -> String {
    use crate::trigger_cdc::Vendor;
    match vendor {
        Vendor::Mysql | Vendor::Mariadb => format!("`{ident}`"),
        Vendor::SqlServer => format!("[{ident}]"),
        _ => format!("\"{ident}\""),
    }
}

/// Split `SCHEMA.TABLE` and reject anything that is not a plain identifier, since both
/// parts are spliced into the poll query.
fn split_qualified_table(qualified: &str) -> Result<(&str, &str), AppError> {
    let (schema, table) = qualified
        .trim()
        .split_once('.')
        .map(|(s, t)| (s.trim(), t.trim()))
        .filter(|(s, t)| !s.is_empty() && !t.is_empty())
        .ok_or_else(|| AppError::BadRequest(format!("expected SCHEMA.TABLE, got '{qualified}'")))?;
    for part in [schema, table] {
        if part.len() > 128
            || !part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '#'))
        {
            return Err(AppError::BadRequest(format!(
                "invalid table identifier in '{qualified}'"
            )));
        }
    }
    Ok((schema, table))
}

/// Validate `source.table_poll` against the selected tables.
fn validate_table_poll(source: &SourceConfig) -> Result<(), AppError> {
    let vendor = crate::trigger_cdc::Vendor::parse(&source.source_type)?;
    let mut seen: Vec<String> = Vec::new();
    for rule in &source.table_poll {
        let table = rule.table.trim();
        if !source
            .tables
            .iter()
            .any(|t| t.trim().eq_ignore_ascii_case(table))
        {
            return Err(AppError::BadRequest(format!(
                "table_poll rule for '{table}' does not match a selected table"
            )));
        }
        let key = table.to_ascii_lowercase();
        if seen.contains(&key) {
            return Err(AppError::BadRequest(format!(
                "table_poll has more than one rule for '{table}'"
            )));
        }
        seen.push(key);
        if rule.strategy != TablePollStrategy::Watermark {
            // Builds the exact query the connector will run, so every failure mode
            // (identifier, column, window size, dialect) surfaces at save time.
            poll_query(vendor, table, rule)?;
        }
    }
    Ok(())
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
    let pk_fields = sink_upsert_key_fields(profile)?;

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
    cfg.insert("primary.key.fields".into(), pk_fields.join(","));
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
            autostart: true,
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
                upsert_key_columns: vec![],
                table_poll: vec![],
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
    fn composite_text_upsert_uses_timestamp_poll_only() {
        let mut p = profile();
        p.source.source_type = "oracle".into();
        p.source.tables = vec!["PANDORA.PM_PRODUCT".into()];
        p.source.timestamp_column = Some("UPDATE_DATE".into());
        p.source.incrementing_column = None;
        p.source.upsert_key_columns = vec!["BRANCH_NO".into(), "PRO_CODE".into()];
        assert_eq!(jdbc_mode(&p.source).unwrap(), "timestamp");
        let cfg = build_jdbc_source_config(&p, "http://cdc-message:8081", &HashMap::new()).unwrap();
        assert_eq!(cfg.get("mode").unwrap(), "timestamp");
        assert_eq!(
            cfg.get("timestamp.columns.mapping").unwrap(),
            ".*:[UPDATE_DATE]"
        );
        assert!(!cfg.contains_key("incrementing.column.mapping"));
        let sink = build_incremental_sink_config(&p, "http://cdc-message:8081").unwrap();
        assert_eq!(
            sink.get("primary.key.fields").unwrap(),
            "branch_no,pro_code"
        );
    }

    #[test]
    fn rejects_sql_in_column_names() {
        let mut p = profile();
        p.source.timestamp_column = Some("updated_at;drop".into());
        assert!(build_jdbc_source_config(&p, "http://cdc-message:8081", &HashMap::new()).is_err());
    }

    // ---- per-table poll strategies (rolling window / full copy) ----

    fn oracle_profile() -> CdcProfile {
        let mut p = profile();
        p.source.source_type = "oracle".into();
        p.source.schemas = vec!["PANDORA".into(), "CRM_STG".into()];
        p.source.tables = vec!["PANDORA.PM_PRODUCT".into(), "CRM_STG.PM_PROBRAND".into()];
        p.source.timestamp_column = Some("UPDATE_DATE".into());
        p.source.incrementing_column = None;
        p.source.upsert_key_columns = vec!["BRANCH_NO".into(), "PRO_CODE".into()];
        p
    }

    fn rolling(table: &str, column: &str, days: Option<u32>) -> TablePollRule {
        TablePollRule {
            table: table.into(),
            strategy: TablePollStrategy::RollingWindow,
            window_column: Some(column.into()),
            window_days: days,
        }
    }

    fn full_copy(table: &str) -> TablePollRule {
        TablePollRule {
            table: table.into(),
            strategy: TablePollStrategy::FullCopy,
            window_column: None,
            window_days: None,
        }
    }

    fn upsert_request(p: &CdcProfile) -> ProfileUpsertRequest {
        ProfileUpsertRequest {
            name: p.name.clone(),
            description: None,
            autostart: None,
            pattern: Some(p.pattern.clone()),
            stream_platform: None,
            transform_strategy: None,
            source: p.source.clone(),
            target: p.target.clone(),
            lake: None,
            governance: None,
            sink_error_policy: None,
            flow: p.flow.clone(),
        }
    }

    #[test]
    fn rolling_window_query_uses_each_dialect_and_defaults_to_one_day() {
        use crate::trigger_cdc::Vendor;
        let rule = rolling("PANDORA.PM_PRODUCT", "UPDATE_DATE", None);
        assert_eq!(
            poll_query(Vendor::Oracle, "PANDORA.PM_PRODUCT", &rule).unwrap(),
            "SELECT * FROM \"PANDORA\".\"PM_PRODUCT\" WHERE \"UPDATE_DATE\" >= TRUNC(SYSDATE) - 1"
        );
        let rule = rolling("app.orders", "updated_at", Some(3));
        assert_eq!(
            poll_query(Vendor::Postgres, "app.orders", &rule).unwrap(),
            "SELECT * FROM \"app\".\"orders\" WHERE \"updated_at\" >= CURRENT_DATE - 3"
        );
        assert!(poll_query(Vendor::Mysql, "app.orders", &rule)
            .unwrap()
            .ends_with("`updated_at` >= CURRENT_DATE - INTERVAL 3 DAY"));
        assert!(poll_query(Vendor::SqlServer, "dbo.orders", &rule)
            .unwrap()
            .contains("[updated_at] >= DATEADD(day, -3, CAST(GETDATE() AS date))"));
        assert!(poll_query(Vendor::Sqlite, "main.orders", &rule).is_err());
    }

    #[test]
    fn full_copy_query_reads_the_whole_table() {
        use crate::trigger_cdc::Vendor;
        assert_eq!(
            poll_query(
                Vendor::Oracle,
                "CRM_STG.PM_PROBRAND",
                &full_copy("CRM_STG.PM_PROBRAND")
            )
            .unwrap(),
            "SELECT * FROM \"CRM_STG\".\"PM_PROBRAND\""
        );
    }

    #[test]
    fn poll_query_rejects_injection_and_oversized_windows() {
        use crate::trigger_cdc::Vendor;
        let rule = rolling("PANDORA.PM_PRODUCT", "UPDATE_DATE", None);
        assert!(poll_query(Vendor::Oracle, "PANDORA.PM_PRODUCT; DROP TABLE X", &rule).is_err());
        assert!(poll_query(
            Vendor::Oracle,
            "PANDORA.PM_PRODUCT",
            &rolling("PANDORA.PM_PRODUCT", "UPDATE_DATE) OR (1=1", None)
        )
        .is_err());
        assert!(poll_query(
            Vendor::Oracle,
            "PANDORA.PM_PRODUCT",
            &rolling(
                "PANDORA.PM_PRODUCT",
                "UPDATE_DATE",
                Some(MAX_WINDOW_DAYS + 1)
            )
        )
        .is_err());
        let mut no_column = rolling("PANDORA.PM_PRODUCT", "", None);
        no_column.window_column = None;
        assert!(poll_query(Vendor::Oracle, "PANDORA.PM_PRODUCT", &no_column).is_err());
    }

    #[test]
    fn poll_connector_is_bulk_query_mode_on_the_table_mode_topic() {
        let mut p = oracle_profile();
        p.source.table_poll = vec![rolling("PANDORA.PM_PRODUCT", "UPDATE_DATE", None)];
        let cfg = build_poll_connector_config(
            &p,
            "http://registry:8081",
            &HashMap::new(),
            "PANDORA.PM_PRODUCT",
            &p.source.table_poll[0],
        )
        .unwrap();
        assert_eq!(cfg.get("mode").unwrap(), "bulk");
        assert!(cfg.get("query").unwrap().contains("TRUNC(SYSDATE) - 1"));
        assert_eq!(
            cfg.get("topic.prefix").unwrap(),
            &format!("{}.PM_PRODUCT", topic_prefix_for(&p))
        );
        // Query mode must not carry table-mode / watermark keys.
        for key in [
            "table.include.list",
            "timestamp.columns.mapping",
            "incrementing.column.mapping",
            "validate.non.null",
        ] {
            assert!(!cfg.contains_key(key), "unexpected {key}");
        }
    }

    #[test]
    fn plan_with_only_custom_tables_gives_the_primary_name_to_the_first() {
        let mut p = oracle_profile();
        p.source.table_poll = vec![
            rolling("PANDORA.PM_PRODUCT", "UPDATE_DATE", None),
            full_copy("crm_stg.pm_probrand"),
        ];
        let plan =
            source_connector_plan(&p, "cdc-draft---oracle", "http://r:8081", &HashMap::new())
                .unwrap();
        let names: Vec<&str> = plan.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "cdc-draft---oracle",
                "cdc-draft---oracle-poll-crm-stg-pm-probrand"
            ]
        );
        assert!(plan.iter().all(|s| s.config.get("mode").unwrap() == "bulk"));
        assert_eq!(
            plan[1].config.get("query").unwrap(),
            "SELECT * FROM \"CRM_STG\".\"PM_PROBRAND\""
        );
    }

    #[test]
    fn plan_keeps_watermark_tables_on_the_primary_table_mode_connector() {
        let mut p = oracle_profile();
        p.source.table_poll = vec![full_copy("CRM_STG.PM_PROBRAND")];
        let plan = source_connector_plan(&p, "cdc-x", "http://r:8081", &HashMap::new()).unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].name, "cdc-x");
        assert_eq!(plan[0].config.get("mode").unwrap(), "timestamp");
        assert_eq!(
            plan[0].config.get("table.include.list").unwrap(),
            "PANDORA\\.PM_PRODUCT"
        );
        assert_eq!(plan[1].name, "cdc-x-poll-crm-stg-pm-probrand");
    }

    #[test]
    fn watermark_rules_do_not_count_as_custom_tables() {
        let mut p = oracle_profile();
        p.source.table_poll = vec![TablePollRule {
            table: "PANDORA.PM_PRODUCT".into(),
            strategy: TablePollStrategy::Watermark,
            window_column: None,
            window_days: None,
        }];
        assert_eq!(watermark_tables(&p).len(), 2);
        assert!(custom_poll_tables(&p).is_empty());
        assert!(!has_rolling_window_tables(&p));
    }

    #[test]
    fn rolling_window_only_pipeline_validates_without_a_timestamp_column() {
        let mut p = oracle_profile();
        p.source.timestamp_column = None;
        p.source.table_poll = vec![
            rolling("PANDORA.PM_PRODUCT", "UPDATE_DATE", Some(1)),
            full_copy("CRM_STG.PM_PROBRAND"),
        ];
        validate_profile(&upsert_request(&p)).unwrap();
        assert!(has_rolling_window_tables(&p));

        // A remaining watermark table still needs the watermark column.
        p.source.table_poll.pop();
        let err = validate_profile(&upsert_request(&p))
            .unwrap_err()
            .to_string_message();
        assert!(err.contains("requires timestamp_column"), "{err}");
    }

    #[test]
    fn table_poll_rules_must_name_selected_tables_once() {
        let mut p = oracle_profile();
        p.source.table_poll = vec![full_copy("PANDORA.OTHER")];
        assert!(validate_profile(&upsert_request(&p)).is_err());
        p.source.table_poll = vec![
            full_copy("CRM_STG.PM_PROBRAND"),
            full_copy("crm_stg.pm_probrand"),
        ];
        assert!(validate_profile(&upsert_request(&p)).is_err());
    }

    #[test]
    fn table_poll_round_trips_through_json_and_defaults_to_empty() {
        let mut p = oracle_profile();
        p.source.table_poll = vec![rolling("PANDORA.PM_PRODUCT", "UPDATE_DATE", Some(2))];
        let json = serde_json::to_value(&p.source).unwrap();
        assert_eq!(json["table_poll"][0]["strategy"], "rolling_window");
        assert_eq!(json["table_poll"][0]["window_days"], 2);
        let mut legacy = json.clone();
        legacy.as_object_mut().unwrap().remove("table_poll");
        let back: SourceConfig = serde_json::from_value(legacy).unwrap();
        assert!(back.table_poll.is_empty());
    }
}
