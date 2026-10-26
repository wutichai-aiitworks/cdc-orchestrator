//! PostgreSQL trigger-based CDC when wal_level is not logical.
//!
//! Source triggers write row images into `hyper_cdc.change_log`. Kafka Connect
//! JDBC source tails that table; the JDBC sink lands `hyper_cdc.inbox` on the
//! target; a generated apply function upserts into the real tables.

use std::collections::HashMap;

use crate::error::AppError;
use crate::models::{CaptureMode, CdcProfile, TableMapping};
use crate::reconcile::{self, source_table_include_list, topic_prefix_for};

pub const CAPTURE_SCHEMA: &str = "hyper_cdc";
pub const CHANGE_LOG: &str = "change_log";
pub const INBOX: &str = "inbox";

/// Default poll cadence for realtime trigger sync (Confluent JDBC source default is 5s).
pub const TRIGGER_POLL_DEFAULT_MS: u64 = 1000;
/// Cap rows drained per poll so a large backlog does not spike the source in one query.
pub const TRIGGER_BATCH_MAX_ROWS: u64 = 500;

/// Resolve the JDBC source `poll.interval.ms` from the profile schedule.
///
/// `micro_batch` with `interval_seconds` polls infrequently to minimize load on a
/// production source; `realtime` (or a missing/zero interval) keeps the fast default.
pub fn trigger_poll_interval_ms(profile: &CdcProfile) -> u64 {
    match profile.flow.schedule.delivery_mode {
        crate::models::DeliveryMode::MicroBatch => profile
            .flow
            .schedule
            .interval_seconds
            .filter(|s| *s > 0)
            .map(|s| u64::from(s) * 1000)
            .unwrap_or(TRIGGER_POLL_DEFAULT_MS),
        crate::models::DeliveryMode::Realtime => TRIGGER_POLL_DEFAULT_MS,
    }
}

#[derive(Debug, Clone)]
pub struct CapturedTable {
    pub source_schema: String,
    pub source_table: String,
    pub target_schema: String,
    pub target_table: String,
    pub pk_columns: Vec<String>,
}

pub fn quote_ident(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > 63
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || name.starts_with(|c: char| c.is_ascii_digit())
    {
        return Err(AppError::BadRequest(format!(
            "invalid PostgreSQL identifier: {name}"
        )));
    }
    Ok(format!("\"{}\"", name.replace('"', "\"\"")))
}

pub fn parse_qualified_tables(profile: &CdcProfile) -> Result<Vec<CapturedTable>, AppError> {
    let mut out = Vec::new();
    let include = source_table_include_list(profile);
    if include.is_empty() {
        return Err(AppError::BadRequest(
            "trigger capture needs an explicit table list (table_mode=specific_tables)".to_string(),
        ));
    }
    let mappings: HashMap<String, &TableMapping> = profile
        .flow
        .mappings
        .iter()
        .map(|m| (m.source_qualified().to_ascii_lowercase(), m))
        .collect();
    for qualified in include {
        let (schema, table) = split_qualified(&qualified)?;
        let mapping = mappings.get(&qualified.to_ascii_lowercase());
        let (target_schema, target_table) = match mapping {
            Some(m) if !m.identical && !m.target_table.trim().is_empty() => (
                nonempty_or(&m.target_schema, &schema),
                m.target_table.trim().to_string(),
            ),
            _ => (schema.clone(), table.clone()),
        };
        quote_ident(&schema)?;
        quote_ident(&table)?;
        quote_ident(&target_schema)?;
        quote_ident(&target_table)?;
        // Honour any logical PK the user declared in the mapping (used when the target
        // table has no PRIMARY KEY constraint but upsert semantics are still needed).
        let pk_columns = mapping
            .map(|m| {
                m.pk_columns
                    .iter()
                    .map(|c| c.trim().to_string())
                    .filter(|c| !c.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        out.push(CapturedTable {
            source_schema: schema,
            source_table: table,
            target_schema,
            target_table,
            pk_columns,
        });
    }
    Ok(out)
}

fn split_qualified(qualified: &str) -> Result<(String, String), AppError> {
    let value = qualified.trim();
    match value.rsplit_once('.') {
        Some((schema, table)) if !schema.trim().is_empty() && !table.trim().is_empty() => {
            Ok((schema.trim().to_string(), table.trim().to_string()))
        }
        _ => Err(AppError::BadRequest(format!(
            "trigger capture table must be schema.table: {value}"
        ))),
    }
}

fn nonempty_or(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn source_install_sql(tables: &[CapturedTable]) -> Result<String, AppError> {
    let schema = quote_ident(CAPTURE_SCHEMA)?;
    let log = quote_ident(CHANGE_LOG)?;
    let mut sql = String::new();
    sql.push_str("-- Hyper CDC trigger capture (source). Run as a user that can CREATE TRIGGER.\n");
    sql.push_str(
        "-- Use this when wal_level is not logical. Prefer logical decoding when allowed.\n\n",
    );
    sql.push_str(&format!("CREATE SCHEMA IF NOT EXISTS {schema};\n\n"));
    sql.push_str(&format!(
        "CREATE TABLE IF NOT EXISTS {schema}.{log} (\n\
         \tid bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,\n\
         \tsource_schema text NOT NULL,\n\
         \tsource_table text NOT NULL,\n\
         \top text NOT NULL,\n\
         \tpk jsonb,\n\
         \trow_data jsonb,\n\
         \tchanged_at timestamptz NOT NULL DEFAULT now()\n\
         );\n\n"
    ));
    sql.push_str(&format!(
        "CREATE INDEX IF NOT EXISTS change_log_changed_at_idx ON {schema}.{log} (id);\n\n"
    ));
    sql.push_str(&format!(
        "CREATE OR REPLACE FUNCTION {schema}.capture() RETURNS trigger\n\
         LANGUAGE plpgsql AS $$\n\
         BEGIN\n\
           IF TG_OP = 'DELETE' THEN\n\
             INSERT INTO {schema}.{log}(source_schema, source_table, op, pk, row_data)\n\
             VALUES (TG_TABLE_SCHEMA, TG_TABLE_NAME, 'd', to_jsonb(OLD), NULL);\n\
             RETURN OLD;\n\
           ELSIF TG_OP = 'UPDATE' THEN\n\
             INSERT INTO {schema}.{log}(source_schema, source_table, op, pk, row_data)\n\
             VALUES (TG_TABLE_SCHEMA, TG_TABLE_NAME, 'u', to_jsonb(NEW), to_jsonb(NEW));\n\
             RETURN NEW;\n\
           ELSE\n\
             INSERT INTO {schema}.{log}(source_schema, source_table, op, pk, row_data)\n\
             VALUES (TG_TABLE_SCHEMA, TG_TABLE_NAME, 'c', to_jsonb(NEW), to_jsonb(NEW));\n\
             RETURN NEW;\n\
           END IF;\n\
         END;\n\
         $$;\n\n"
    ));
    for table in tables {
        let src_schema = quote_ident(&table.source_schema)?;
        let src_table = quote_ident(&table.source_table)?;
        sql.push_str(&format!(
            "DROP TRIGGER IF EXISTS hyper_cdc_capture ON {src_schema}.{src_table};\n\
             CREATE TRIGGER hyper_cdc_capture\n\
             AFTER INSERT OR UPDATE OR DELETE ON {src_schema}.{src_table}\n\
             FOR EACH ROW EXECUTE PROCEDURE {schema}.capture();\n\n"
        ));
    }
    Ok(sql)
}

pub fn target_apply_sql(tables: &[CapturedTable]) -> Result<String, AppError> {
    let schema = quote_ident(CAPTURE_SCHEMA)?;
    let inbox = quote_ident(INBOX)?;
    let mut sql = String::new();
    sql.push_str("-- Hyper CDC trigger apply (target). Run before starting the pipeline.\n\n");
    sql.push_str(&format!("CREATE SCHEMA IF NOT EXISTS {schema};\n\n"));
    // `pk` / `row_data` are text, not jsonb: the changelog carries them as a JSON *string*
    // (Oracle builds them with JSON_OBJECT into a CLOB), and the JDBC sink binds that as
    // varchar. Postgres will not implicitly cast varchar to jsonb, so a jsonb column here
    // fails every insert. The apply function casts once it has the value.
    sql.push_str(&format!(
        "CREATE TABLE IF NOT EXISTS {schema}.{inbox} (\n\
         \tid bigint PRIMARY KEY,\n\
         \tsource_schema text NOT NULL,\n\
         \tsource_table text NOT NULL,\n\
         \top text NOT NULL,\n\
         \tpk text,\n\
         \trow_data text,\n\
         \tchanged_at timestamptz,\n\
         \tapplied_at timestamptz\n\
         );\n\n"
    ));
    // Inboxes created before the column types were corrected still have jsonb; widen them
    // in place so an existing deployment starts working without dropping the table.
    sql.push_str(&format!(
        "ALTER TABLE {schema}.{inbox} ALTER COLUMN pk TYPE text USING pk::text;\n\
         ALTER TABLE {schema}.{inbox} ALTER COLUMN row_data TYPE text USING row_data::text;\n\n"
    ));
    // Normalize top-level JSON keys to lowercase so source identifier casing
    // (e.g. Oracle emits UPPERCASE keys via JSON_OBJECT) matches the Postgres
    // target's folded-lowercase column names in jsonb_populate_record.
    sql.push_str(&format!(
        "CREATE OR REPLACE FUNCTION {schema}.apply_inbox() RETURNS trigger\n\
         LANGUAGE plpgsql AS $$\n\
         DECLARE\n\
           raw_payload jsonb := COALESCE(NULLIF(NEW.row_data, ''), NULLIF(NEW.pk, ''), '{{}}')::jsonb;\n\
           payload jsonb := COALESCE(\n\
             (SELECT jsonb_object_agg(lower(kv.key), kv.value) FROM jsonb_each(raw_payload) AS kv),\n\
             '{{}}'::jsonb\n\
           );\n\
         BEGIN\n"
    ));
    if tables.is_empty() {
        sql.push_str("           RAISE EXCEPTION 'hyper_cdc.apply_inbox: no captured tables';\n");
    }
    for (i, table) in tables.iter().enumerate() {
        let keyword = if i == 0 { "IF" } else { "ELSIF" };
        sql.push_str(&format!(
            "           {keyword} NEW.source_schema = {} AND NEW.source_table = {} THEN\n",
            sql_literal(&table.source_schema),
            sql_literal(&table.source_table),
        ));
        let tgt_schema = quote_ident(&table.target_schema)?;
        let tgt_table = quote_ident(&table.target_table)?;
        if table.pk_columns.is_empty() {
            sql.push_str(&format!(
                "             RAISE EXCEPTION 'table {}.{} has no primary key; trigger apply cannot upsert/delete';\n",
                table.source_schema, table.source_table
            ));
        } else {
            let pk_pred = pk_where(&table.pk_columns, "payload")?;
            sql.push_str(&format!(
                "             IF NEW.op = 'd' THEN\n\
                 \t             DELETE FROM {tgt_schema}.{tgt_table} t WHERE {pk_pred};\n\
                 \t           ELSE\n\
                 \t             DELETE FROM {tgt_schema}.{tgt_table} t WHERE {pk_pred};\n\
                 \t             INSERT INTO {tgt_schema}.{tgt_table}\n\
                 \t             SELECT * FROM jsonb_populate_record(NULL::{tgt_schema}.{tgt_table}, payload);\n\
                 \t           END IF;\n"
            ));
        }
    }
    sql.push_str(
        "           ELSE\n\
         \t           RAISE EXCEPTION 'hyper_cdc.apply_inbox: unmapped table %.%', NEW.source_schema, NEW.source_table;\n\
         \t         END IF;\n\
         \t         NEW.applied_at := now();\n\
         \t         RETURN NEW;\n\
         END;\n\
         $$;\n\n",
    );
    sql.push_str(&format!(
        "DROP TRIGGER IF EXISTS hyper_cdc_apply ON {schema}.{inbox};\n\
         CREATE TRIGGER hyper_cdc_apply\n\
         BEFORE INSERT ON {schema}.{inbox}\n\
         FOR EACH ROW EXECUTE PROCEDURE {schema}.apply_inbox();\n"
    ));
    Ok(sql)
}

fn pk_where(columns: &[String], json_alias: &str) -> Result<String, AppError> {
    let mut parts = Vec::new();
    for column in columns {
        let ident = quote_ident(column)?;
        // JSON keys are normalized to lowercase in apply_inbox(); look them up
        // case-folded while still comparing against the real target column.
        parts.push(format!(
            "t.{ident}::text = ({json_alias}->>{})",
            sql_literal(&column.to_ascii_lowercase())
        ));
    }
    Ok(parts.join(" AND "))
}

fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Kafka topic that carries the changelog rows, for both the source and the sink.
///
/// The JDBC source names its topic `topic.prefix + <bare table name>`, which differs per
/// vendor (`change_log` on PostgreSQL, `HYPER_CDC_CHANGE_LOG` on Oracle). Publishing under
/// that raw name would make the sink's subscription vendor-dependent, so the source rewrites
/// it to this canonical name and both sides call this function instead of rebuilding it.
pub fn changelog_topic(profile: &CdcProfile) -> String {
    format!(
        "{}_{CAPTURE_SCHEMA}.{CHANGE_LOG}",
        topic_prefix_for(profile)
    )
}

pub fn build_jdbc_source_config(
    profile: &CdcProfile,
    schema_registry_url: &str,
) -> Result<HashMap<String, String>, AppError> {
    if profile.source.capture_mode != CaptureMode::Trigger {
        return Err(AppError::BadRequest(
            "JDBC changelog source is only used for capture_mode=trigger".into(),
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
    cfg.insert("mode".into(), "incrementing".into());
    cfg.insert(
        "incrementing.column.mapping".into(),
        format!(".*:{}", vendor.changelog_id_column()),
    );
    cfg.insert(
        "table.include.list".into(),
        crate::trigger_cdc::changelog_include_regex(vendor, &profile.source.connection)?,
    );
    cfg.insert("numeric.mapping".into(), "best_fit".into());
    cfg.insert("validate.non.null".into(), "false".into());
    cfg.insert(
        "poll.interval.ms".into(),
        trigger_poll_interval_ms(profile).to_string(),
    );
    cfg.insert("batch.max.rows".into(), TRIGGER_BATCH_MAX_ROWS.to_string());
    cfg.insert("topic.prefix".into(), format!("{prefix}_"));

    // Oracle folds unquoted changelog columns to UPPERCASE. The Postgres inbox and
    // apply function use lowercase names, so rename before the record is published.
    // Incrementing still reads `ID` from the table — this SMT only changes Kafka fields.
    let mut transforms = Vec::new();
    if vendor == crate::trigger_cdc::Vendor::Oracle {
        transforms.push("lowercaseFields");
        cfg.insert(
            "transforms.lowercaseFields.type".into(),
            "org.apache.kafka.connect.transforms.ReplaceField$Value".into(),
        );
        cfg.insert(
            "transforms.lowercaseFields.renames".into(),
            "ID:id,SOURCE_SCHEMA:source_schema,SOURCE_TABLE:source_table,OP:op,PK:pk,ROW_DATA:row_data,CHANGED_AT:changed_at"
                .into(),
        );
    }

    // The include list pins the source to the single changelog table, so rewriting every
    // topic to one canonical name is unambiguous and keeps the sink subscription the same
    // whatever the source vendor calls its changelog table.
    transforms.push("changelogTopic");
    cfg.insert("transforms".into(), transforms.join(","));
    cfg.insert(
        "transforms.changelogTopic.type".into(),
        "org.apache.kafka.connect.transforms.RegexRouter".into(),
    );
    cfg.insert("transforms.changelogTopic.regex".into(), ".*".into());
    cfg.insert(
        "transforms.changelogTopic.replacement".into(),
        changelog_topic(profile),
    );

    // A trigger changelog is the only record of a change, so a conversion failure must stop
    // the connector rather than skip the row. Only transient connection errors are retried.
    cfg.insert("errors.retry.timeout".into(), "60000".into());
    cfg.insert("errors.retry.delay.max.ms".into(), "5000".into());

    // Oracle reports TIMESTAMP without a zone; pin the session zone when the profile knows
    // it so `changed_at` is not reinterpreted against the Connect worker's default.
    if let Some(tz) = profile
        .source
        .connection
        .get("timezone")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        cfg.insert("db.timezone".into(), tz.to_string());
    }

    reconcile::apply_topic_creation_defaults(&mut cfg);
    reconcile::apply_avro_converters(&mut cfg, schema_registry_url);
    Ok(cfg)
}

pub fn build_trigger_sink_config(
    profile: &CdcProfile,
    schema_registry_url: &str,
) -> Result<HashMap<String, String>, AppError> {
    let target = profile.target.as_ref().ok_or_else(|| {
        AppError::BadRequest("target is required for trigger capture sink".into())
    })?;
    let host = req(&target.connection, "host")?;
    let port = req(&target.connection, "port")?;
    let database = req(&target.connection, "database")?;
    let username = req(&target.connection, "username")?;
    let password = req(&target.connection, "password")?;
    let topic = changelog_topic(profile);
    let mut cfg = HashMap::new();
    cfg.insert(
        "connector.class".into(),
        "io.debezium.connector.jdbc.JdbcSinkConnector".into(),
    );
    cfg.insert("tasks.max".into(), "1".into());
    cfg.insert("topics".into(), topic);
    cfg.insert(
        "connection.url".into(),
        format!("jdbc:postgresql://{host}:{port}/{database}?currentSchema={CAPTURE_SCHEMA}"),
    );
    cfg.insert("connection.username".into(), username.into());
    cfg.insert("connection.password".into(), password.into());
    cfg.insert("insert.mode".into(), "upsert".into());
    cfg.insert("primary.key.mode".into(), "record_value".into());
    cfg.insert("primary.key.fields".into(), "id".into());
    cfg.insert("delete.enabled".into(), "false".into());
    // Inbox DDL is owned by install_target_apply. Do not let the sink ADD
    // uppercase Oracle fields (OP, SOURCE_SCHEMA, …) onto the lowercase table.
    cfg.insert("schema.evolution".into(), "none".into());
    cfg.insert("quote.identifiers".into(), "true".into());
    cfg.insert("table.name.format".into(), INBOX.into());

    // Preserve the trigger sink's existing DLQ behavior even when the profile uses
    // the backward-compatible fail_fast default.
    reconcile::apply_sink_error_policy(&mut cfg, profile);
    reconcile::apply_avro_converters(&mut cfg, schema_registry_url);
    Ok(cfg)
}

pub const TRIGGER_NAME: &str = "hyper_cdc_capture";

/// Install the target-side apply path (Postgres) for a trigger-capture profile.
///
/// Creates `hyper_cdc.inbox` and the `apply_inbox()` BEFORE-INSERT trigger so that
/// rows landed by the JDBC sink are upserted/deleted into the real replica tables.
/// Target primary keys are resolved from the target catalog. Idempotent: the
/// generated DDL uses `CREATE ... IF NOT EXISTS` / `CREATE OR REPLACE`.
pub async fn install_target_apply(profile: &CdcProfile) -> Result<(), AppError> {
    let target = profile.target.as_ref().ok_or_else(|| {
        AppError::BadRequest("target is required for trigger capture apply".into())
    })?;
    let mut tables = parse_qualified_tables(profile)?;

    let insight = crate::connector_catalog::insight_from_connection("postgres", &target.connection);
    let client = crate::connector_catalog::postgres_client(&insight).await?;

    // Query the target directly rather than going through the generic catalog so
    // primary keys are always available for inbox apply.
    let pk_map = crate::connector_catalog::postgres_primary_keys(&client).await?;

    for table in &mut tables {
        // An explicit `pk_columns` in the mapping is an override and always wins. Only fall
        // back to the catalog PK when the user did not declare one, and never overwrite with
        // an empty vector (which is what a table without a PRIMARY KEY reports).
        if table.pk_columns.is_empty() {
            if let Some(pk) = pk_map.get(&(
                table.target_schema.to_ascii_lowercase(),
                table.target_table.to_ascii_lowercase(),
            )) {
                table.pk_columns = pk.clone();
            }
        }
        if table.pk_columns.is_empty() {
            return Err(AppError::BadRequest(format!(
                "target table {}.{} has no primary key; trigger apply needs a PK to upsert/delete. \
                 Fix: either add a PRIMARY KEY constraint to the target table, or set \
                 'pk_columns' in the table mapping to declare which columns form the logical key.",
                table.target_schema, table.target_table
            )));
        }
    }

    let sql = target_apply_sql(&tables)?;
    client
        .batch_execute(&format!("BEGIN;\n{sql}\nCOMMIT;"))
        .await
        .map_err(|e| {
            AppError::BadRequest(format!(
                "failed to install target apply (hyper_cdc.inbox + apply_inbox trigger): {e}"
            ))
        })?;
    Ok(())
}

/// Copy the current contents of every captured source table into the target replica.
///
/// Trigger capture only tails `HYPER_CDC_CHANGE_LOG` / `hyper_cdc.change_log`, so rows
/// written before triggers (or not present in the leftover changelog) never reach the
/// replica. A resync therefore snapshots the live tables. The changelog offset is left
/// alone so stale changelog events cannot overwrite this copy.
pub async fn snapshot_source_tables(
    profile: &CdcProfile,
    truncate_each: bool,
) -> Result<Vec<(String, i64)>, AppError> {
    let target = profile.target.as_ref().ok_or_else(|| {
        AppError::BadRequest("pipeline has no target for a source-table snapshot".into())
    })?;
    if !matches!(
        target.target_type.trim().to_ascii_lowercase().as_str(),
        "postgres" | "postgresql"
    ) {
        return Err(AppError::BadRequest(format!(
            "trigger snapshot is only supported for PostgreSQL targets, not '{}'",
            target.target_type
        )));
    }
    let tables = parse_qualified_tables(profile)?;
    let mut copied = Vec::new();
    for table in &tables {
        let rows = crate::connector_catalog::copy_mapped_table_data(
            &profile.source.source_type,
            &profile.source.connection,
            &target.target_type,
            &target.connection,
            &table.source_schema,
            &table.source_table,
            &table.target_schema,
            &table.target_table,
            TRIGGER_BATCH_MAX_ROWS as usize,
            truncate_each,
        )
        .await?;
        copied.push((
            format!("{}.{}", table.target_schema, table.target_table),
            rows,
        ));
    }
    Ok(copied)
}

pub fn snapshot_history_detail(copied: &[(String, i64)]) -> String {
    if copied.is_empty() {
        return "No captured tables to snapshot".to_string();
    }
    let total: i64 = copied.iter().map(|(_, n)| *n).sum();
    let tables = copied
        .iter()
        .map(|(name, n)| format!("{name}={n}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Copied {total} source row(s) into {} target table(s): {tables}",
        copied.len()
    )
}

fn req<'a>(m: &'a HashMap<String, String>, key: &str) -> Result<&'a str, AppError> {
    m.get(key)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest(format!("missing connection field: {key}")))
}

/// Delete changelog rows older than `older_than_hours` on a PostgreSQL source.
/// Returns the number of rows removed. A no-op when `older_than_hours == 0`.
pub async fn prune_changelog_postgres(
    connection: &HashMap<String, String>,
    older_than_hours: u32,
) -> Result<u64, AppError> {
    if older_than_hours == 0 {
        return Ok(0);
    }
    let insight = crate::connector_catalog::insight_from_connection("postgres", connection);
    let client = crate::connector_catalog::postgres_client(&insight).await?;
    let schema = quote_ident(CAPTURE_SCHEMA)?;
    let log = quote_ident(CHANGE_LOG)?;
    let stmt =
        format!("DELETE FROM {schema}.{log} WHERE changed_at < now() - make_interval(hours => $1)");
    let deleted = client
        .execute(stmt.as_str(), &[&(older_than_hours as i32)])
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to prune changelog: {e}")))?;
    Ok(deleted)
}

pub fn captured_tables_from_names(names: &[String]) -> Result<Vec<CapturedTable>, AppError> {
    let mut out = Vec::new();
    for value in names {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (schema, table) = split_qualified(trimmed)?;
        quote_ident(&schema)?;
        quote_ident(&table)?;
        out.push(CapturedTable {
            source_schema: schema.clone(),
            source_table: table.clone(),
            target_schema: schema,
            target_table: table,
            pk_columns: Vec::new(),
        });
    }
    if out.is_empty() {
        return Err(AppError::BadRequest(
            "select at least one table (schema.table) to install triggers".to_string(),
        ));
    }
    Ok(out)
}

pub async fn list_capture_triggers(
    client: &tokio_postgres::Client,
) -> Result<Vec<(String, String)>, AppError> {
    let rows = client
        .query(
            "SELECT n.nspname::text, c.relname::text \
             FROM pg_trigger t \
             JOIN pg_class c ON c.oid = t.tgrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE NOT t.tgisinternal AND t.tgname = $1 \
             ORDER BY 1, 2",
            &[&TRIGGER_NAME],
        )
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to list capture triggers: {e}")))?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some((
                row.try_get::<_, String>(0).ok()?,
                row.try_get::<_, String>(1).ok()?,
            ))
        })
        .collect())
}

pub async fn change_log_present(client: &tokio_postgres::Client) -> Result<bool, AppError> {
    let row = client
        .query_one(
            "SELECT EXISTS ( \
               SELECT 1 FROM pg_class c \
               JOIN pg_namespace n ON n.oid = c.relnamespace \
               WHERE n.nspname = $1 AND c.relname = $2 \
             )",
            &[&CAPTURE_SCHEMA, &CHANGE_LOG],
        )
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to check change_log: {e}")))?;
    Ok(row.try_get::<_, bool>(0).unwrap_or(false))
}

pub async fn install_source_triggers(
    source_type: &str,
    connection: &HashMap<String, String>,
    tables: &[CapturedTable],
) -> Result<(Vec<String>, Vec<String>), AppError> {
    let insight = crate::connector_catalog::insight_from_connection(source_type, connection);
    let client = crate::connector_catalog::postgres_client(&insight).await?;
    let before: std::collections::HashSet<String> = list_capture_triggers(&client)
        .await?
        .into_iter()
        .map(|(schema, table)| format!("{schema}.{table}").to_ascii_lowercase())
        .collect();
    let sql = source_install_sql(tables)?;
    client
        .batch_execute(&format!("BEGIN;\n{sql}\nCOMMIT;"))
        .await
        .map_err(|e| {
            AppError::BadRequest(format!(
                "failed to create changelog triggers (need CREATE TRIGGER on each table): {e}"
            ))
        })?;
    let mut installed = Vec::new();
    let mut already_present = Vec::new();
    for table in tables {
        let qualified = format!("{}.{}", table.source_schema, table.source_table);
        if before.contains(&qualified.to_ascii_lowercase()) {
            already_present.push(qualified);
        } else {
            installed.push(qualified);
        }
    }
    Ok((installed, already_present))
}

pub async fn source_trigger_status(
    source_type: &str,
    connection: &HashMap<String, String>,
    requested: &[String],
) -> Result<(bool, Vec<(String, String, bool)>), AppError> {
    let insight = crate::connector_catalog::insight_from_connection(source_type, connection);
    let client = crate::connector_catalog::postgres_client(&insight).await?;
    let present = change_log_present(&client).await?;
    let installed: std::collections::HashSet<String> = list_capture_triggers(&client)
        .await?
        .into_iter()
        .map(|(schema, table)| format!("{schema}.{table}").to_ascii_lowercase())
        .collect();
    let tables = trigger_status_rows(&installed, requested)?;
    Ok((present, tables))
}

fn trigger_status_rows(
    installed: &std::collections::HashSet<String>,
    requested: &[String],
) -> Result<Vec<(String, String, bool)>, AppError> {
    let mut tables = Vec::new();
    if requested.is_empty() {
        push_installed_rows(&mut tables, installed);
        return Ok(tables);
    }
    for value in requested {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (schema, table) = split_qualified(trimmed)?;
        let installed_here = installed.contains(&format!("{schema}.{table}").to_ascii_lowercase());
        tables.push((schema, table, installed_here));
    }
    for key in installed {
        if requested
            .iter()
            .any(|wanted| wanted.trim().eq_ignore_ascii_case(key))
        {
            continue;
        }
        if let Ok((schema, table)) = split_qualified(key) {
            tables.push((schema, table, true));
        }
    }
    Ok(tables)
}

fn push_installed_rows(
    tables: &mut Vec<(String, String, bool)>,
    installed: &std::collections::HashSet<String>,
) {
    for key in installed {
        if let Ok((schema, table)) = split_qualified(key) {
            tables.push((schema, table, true));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        DeliveryMode, FlowConfig, ProfilePattern, ProfileStatus, ScheduleConfig,
        SchemaSelectionMode, SourceConfig, TableSelectionMode, TargetConfig, TransformStrategy,
    };
    use chrono::Utc;
    use uuid::Uuid;

    fn profile() -> CdcProfile {
        CdcProfile {
            id: Uuid::new_v4(),
            name: "Pg Replica".into(),
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
                capture_mode: CaptureMode::Trigger,
                schema_mode: SchemaSelectionMode::SpecificSchemas,
                table_mode: TableSelectionMode::SpecificTables,
                schemas: vec!["bookings".into()],
                tables: vec!["bookings.tickets".into()],
                changelog_retention_hours: None,
                timestamp_column: None,
                incrementing_column: None,
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
                    delivery_mode: DeliveryMode::Realtime,
                    interval_seconds: None,
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
    fn source_sql_installs_generic_trigger() {
        let tables = parse_qualified_tables(&profile()).unwrap();
        let sql = source_install_sql(&tables).unwrap();
        assert!(sql.contains("CREATE SCHEMA IF NOT EXISTS \"hyper_cdc\""));
        assert!(sql.contains("EXECUTE PROCEDURE \"hyper_cdc\".capture()"));
        assert!(sql.contains("ON \"bookings\".\"tickets\""));
    }

    #[test]
    fn target_sql_requires_pk_for_apply() {
        let mut tables = parse_qualified_tables(&profile()).unwrap();
        tables[0].pk_columns = vec!["id".into()];
        let sql = target_apply_sql(&tables).unwrap();
        assert!(sql.contains("jsonb_populate_record"));
        assert!(sql.contains("DELETE FROM \"bookings\".\"tickets\""));
        assert!(sql.contains("BEFORE INSERT ON \"hyper_cdc\".\"inbox\""));
        // Keys are lowercased so Oracle's UPPERCASE JSON keys map to PG columns.
        assert!(sql.contains("jsonb_object_agg(lower(kv.key)"));
    }

    #[test]
    fn apply_pk_lookup_is_case_folded() {
        let mut tables = parse_qualified_tables(&profile()).unwrap();
        tables[0].pk_columns = vec!["CUSTOMER_ID".into()];
        let sql = target_apply_sql(&tables).unwrap();
        assert!(sql.contains("payload->>'customer_id'"));
        assert!(sql.contains("t.\"CUSTOMER_ID\"::text"));
    }

    /// Model Confluent's filter, which full-matches the pattern against the identifier
    /// the driver discovered (`TableCollectionUtils.filterTables` -> `Pattern.matches`).
    fn include_matches(pattern: &str, discovered: &str) -> bool {
        regex::Regex::new(&format!("^(?:{pattern})$"))
            .expect("generated pattern must be a valid regex")
            .is_match(discovered)
    }

    fn oracle_profile() -> CdcProfile {
        let mut p = profile();
        p.source.source_type = "oracle".into();
        p.source.connection.insert("username".into(), "bcs".into());
        p.source.connection.insert("port".into(), "1521".into());
        p.source
            .connection
            .insert("database".into(), "PRODORCL".into());
        p.source.schemas = vec!["BCS".into()];
        p.source.tables = vec!["BCS.G_BOOKS".into()];
        p
    }

    #[test]
    fn jdbc_source_tails_changelog() {
        let cfg = build_jdbc_source_config(&profile(), "http://cdc-message:8081").unwrap();
        assert_eq!(
            cfg.get("connector.class").unwrap(),
            "io.confluent.connect.jdbc.JdbcSourceConnector"
        );
        assert_eq!(cfg.get("incrementing.column.mapping").unwrap(), ".*:id");
        assert!(
            !cfg.contains_key("incrementing.column.name"),
            "must not mix legacy key with table.include.list"
        );
    }

    /// The include list is full-matched against `[catalog.][schema.]table`, so a bare
    /// table name selects nothing and the task dies with "not assigned a table nor a
    /// query to execute". Both the two-part and three-part forms must match.
    #[test]
    fn include_list_matches_discovered_identifier() {
        let pg = build_jdbc_source_config(&profile(), "http://cdc-message:8081").unwrap();
        let pg_pattern = pg.get("table.include.list").unwrap();
        assert!(include_matches(pg_pattern, "hyper_cdc.change_log"));
        assert!(include_matches(pg_pattern, "demo.hyper_cdc.change_log"));
        assert!(!include_matches(pg_pattern, "public.change_log"));

        let oracle =
            build_jdbc_source_config(&oracle_profile(), "http://cdc-message:8081").unwrap();
        let oracle_pattern = oracle.get("table.include.list").unwrap();
        assert!(include_matches(oracle_pattern, "BCS.HYPER_CDC_CHANGE_LOG"));
        assert!(!include_matches(oracle_pattern, "HYPER_CDC_CHANGE_LOG"));
        assert!(!include_matches(
            oracle_pattern,
            "OTHER.HYPER_CDC_CHANGE_LOG"
        ));
    }

    #[test]
    fn oracle_changelog_id_column_keeps_database_casing() {
        let cfg = build_jdbc_source_config(&oracle_profile(), "http://cdc-message:8081").unwrap();
        assert_eq!(cfg.get("incrementing.column.mapping").unwrap(), ".*:ID");
        assert!(cfg
            .get("transforms")
            .unwrap()
            .starts_with("lowercaseFields"));
        assert!(cfg
            .get("transforms.lowercaseFields.renames")
            .unwrap()
            .contains("OP:op"));
    }

    /// The JDBC source names topics `topic.prefix + <bare table>`, which is vendor
    /// specific, while the sink subscribes to one fixed name. Without the router the two
    /// never meet and the replica silently receives nothing.
    #[test]
    fn source_and_sink_agree_on_the_changelog_topic() {
        for p in [profile(), oracle_profile()] {
            let src = build_jdbc_source_config(&p, "http://cdc-message:8081").unwrap();
            let sink = build_trigger_sink_config(&p, "http://cdc-message:8081").unwrap();
            let routed = src.get("transforms.changelogTopic.replacement").unwrap();
            assert_eq!(
                routed,
                sink.get("topics").unwrap(),
                "source publishes to a topic the sink does not consume"
            );
            assert_eq!(routed, "pg_replica_hyper_cdc.change_log");
            assert!(src.get("transforms").unwrap().contains("changelogTopic"));
        }
    }

    #[test]
    fn db_timezone_is_only_set_when_the_profile_declares_one() {
        let cfg = build_jdbc_source_config(&oracle_profile(), "http://cdc-message:8081").unwrap();
        assert!(!cfg.contains_key("db.timezone"));

        let mut p = oracle_profile();
        p.source
            .connection
            .insert("timezone".into(), "Asia/Bangkok".into());
        let cfg = build_jdbc_source_config(&p, "http://cdc-message:8081").unwrap();
        assert_eq!(cfg.get("db.timezone").unwrap(), "Asia/Bangkok");
    }

    /// A changelog row is the only record of the change, so the source must not skip
    /// records it cannot convert, while the sink routes apply failures to a DLQ.
    #[test]
    fn source_fails_loudly_and_sink_uses_a_dlq() {
        let src = build_jdbc_source_config(&profile(), "http://cdc-message:8081").unwrap();
        assert!(!src.contains_key("errors.tolerance"));

        let sink = build_trigger_sink_config(&profile(), "http://cdc-message:8081").unwrap();
        assert_eq!(sink.get("errors.tolerance").unwrap(), "all");
        assert_eq!(
            sink.get("errors.deadletterqueue.topic.name").unwrap(),
            "cdc-pg-replica-sink-dlq"
        );
    }

    /// Oracle builds the payload with JSON_OBJECT into a CLOB, so the sink binds it as
    /// varchar. Postgres will not implicitly cast varchar to jsonb.
    #[test]
    fn inbox_lands_payloads_as_text_and_casts_on_apply() {
        let mut tables = parse_qualified_tables(&profile()).unwrap();
        tables[0].pk_columns = vec!["id".into()];
        let sql = target_apply_sql(&tables).unwrap();
        assert!(
            sql.contains("\trow_data text,\n"),
            "row_data must be text: {sql}"
        );
        assert!(sql.contains("ALTER COLUMN row_data TYPE text"));
        assert!(sql.contains("NULLIF(NEW.row_data, '')"));
        assert!(sql.contains("'{}')::jsonb"));
    }

    #[test]
    fn realtime_uses_default_poll_interval() {
        let cfg = build_jdbc_source_config(&profile(), "http://cdc-message:8081").unwrap();
        assert_eq!(cfg.get("poll.interval.ms").unwrap(), "1000");
        assert_eq!(cfg.get("batch.max.rows").unwrap(), "500");
    }

    #[test]
    fn micro_batch_interval_drives_poll_ms() {
        let mut p = profile();
        p.flow.schedule.delivery_mode = DeliveryMode::MicroBatch;
        p.flow.schedule.interval_seconds = Some(300);
        assert_eq!(trigger_poll_interval_ms(&p), 300_000);
        let cfg = build_jdbc_source_config(&p, "http://cdc-message:8081").unwrap();
        assert_eq!(cfg.get("poll.interval.ms").unwrap(), "300000");
    }

    #[test]
    fn micro_batch_without_interval_falls_back_to_default() {
        let mut p = profile();
        p.flow.schedule.delivery_mode = DeliveryMode::MicroBatch;
        p.flow.schedule.interval_seconds = None;
        assert_eq!(trigger_poll_interval_ms(&p), 1000);
    }

    #[test]
    fn snapshot_history_lists_each_table() {
        let detail =
            snapshot_history_detail(&[("BCS.G_BOOKS".into(), 12), ("BCS.G_CART".into(), 3)]);
        assert!(detail.contains("Copied 15 source row(s) into 2 target table(s)"));
        assert!(detail.contains("BCS.G_BOOKS=12"));
        assert!(detail.contains("BCS.G_CART=3"));
    }

    #[test]
    fn rejects_unsafe_identifiers() {
        assert!(quote_ident("tickets;drop").is_err());
        assert!(quote_ident("ok_table").is_ok());
    }

    #[test]
    fn names_become_qualified_tables() {
        let tables = captured_tables_from_names(&["bookings.tickets".into()]).unwrap();
        assert_eq!(tables[0].source_schema, "bookings");
        assert_eq!(tables[0].source_table, "tickets");
        assert!(captured_tables_from_names(&["tickets".into()]).is_err());
    }
}
