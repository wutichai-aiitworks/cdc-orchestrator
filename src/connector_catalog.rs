//! Live schema/table catalog for connector databases.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::time::timeout;
use tokio_postgres::NoTls;

use crate::clients::JdbcAdminClient;
use crate::connector_meta::{ConnectorConnectionView, ConnectorInsight, ConnectorKind};
use crate::error::AppError;
use crate::models::{
    SchemaCountsResponse, SourceTableColumn, SourceTableInspectRequest, SourceTableInspectResponse,
    TableCountResponse, TablePreviewResponse,
};

const SYSTEM_SCHEMAS: &[&str] = &[
    "pg_catalog",
    "information_schema",
    "pg_toast",
    "pg_temp",
    "hyper_cdc",
    "mysql",
    "sys",
    "performance_schema",
];

#[derive(Debug, Clone)]
pub struct LiveTable {
    pub schema: String,
    pub name: String,
    pub row_count: Option<i64>,
    pub primary_key: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LiveCatalog {
    pub schemas: Vec<String>,
    pub tables: Vec<LiveTable>,
}

const EXACT_COUNT_TABLE_LIMIT: usize = 80;
const EXACT_COUNT_BUDGET: Duration = Duration::from_secs(6);
const EXACT_COUNT_PER_TABLE: Duration = Duration::from_secs(2);
const PREVIEW_ROW_LIMIT_DEFAULT: u32 = 50;
const PREVIEW_ROW_LIMIT_MAX: u32 = 100;

pub async fn load_live_catalog(insight: &ConnectorInsight) -> Result<LiveCatalog, AppError> {
    load_postgres_or_err(insight, &insight.configured_schemas).await
}

fn jdbc_admin() -> Option<JdbcAdminClient> {
    JdbcAdminClient::from_env()
}

fn require_jdbc_admin(db_type: &str) -> Result<JdbcAdminClient, AppError> {
    jdbc_admin().ok_or_else(|| {
        AppError::BadRequest(format!(
            "{db_type} control-plane access requires cdc-jdbc-admin — set ORCH_JDBC_ADMIN_URL"
        ))
    })
}

pub async fn test_connection(
    db_type: &str,
    connection: &HashMap<String, String>,
) -> Result<String, AppError> {
    if let Some(admin) = jdbc_admin() {
        return admin.test(db_type, connection).await;
    }
    match db_type.trim().to_ascii_lowercase().as_str() {
        "postgres" | "postgresql" => {
            let insight = insight_from_connection(db_type, connection);
            postgres_client(&insight).await?;
            let host = connection.get("host").map(|s| s.as_str()).unwrap_or("");
            let port = connection.get("port").map(|s| s.as_str()).unwrap_or("");
            Ok(format!("source connection successful: {host}:{port}"))
        }
        other => Err(AppError::BadRequest(format!(
            "{other} connection test requires cdc-jdbc-admin — set ORCH_JDBC_ADMIN_URL"
        ))),
    }
}

pub async fn load_live_catalog_from_connection(
    source_type: &str,
    connection: &HashMap<String, String>,
    schema_filter: &[String],
) -> Result<LiveCatalog, AppError> {
    if let Some(admin) = jdbc_admin() {
        let (schemas, tables) = admin
            .catalog(source_type, connection, schema_filter)
            .await?;
        return Ok(LiveCatalog { schemas, tables });
    }

    match source_type.trim().to_ascii_lowercase().as_str() {
        "postgres" | "postgresql" => {
            load_postgres_or_err(
                &insight_from_connection(source_type, connection),
                schema_filter,
            )
            .await
        }
        "mysql" | "mariadb" | "sqlserver" | "mssql" | "sqlite" => {
            let (schemas, tables) =
                crate::trigger_cdc::live_catalog(source_type, connection).await?;
            Ok(LiveCatalog {
                schemas,
                tables: tables
                    .into_iter()
                    .map(|(schema, name)| LiveTable {
                        schema,
                        name,
                        row_count: None,
                        primary_key: Vec::new(),
                    })
                    .collect(),
            })
        }
        other => Err(AppError::BadRequest(format!(
            "live catalog for '{other}' requires cdc-jdbc-admin — set ORCH_JDBC_ADMIN_URL"
        ))),
    }
}

/// Extract the schema name from a `CREATE TABLE [IF NOT EXISTS] <schema>.<table>` statement.
///
/// Handles double-quoted (`"s"`), backtick (`` `s` ``), bracket (`[s]`), and
/// bare identifiers.  Returns `None` when the statement is not a `CREATE TABLE`
/// or has no explicit `<schema>.<table>` qualifier.
fn unquote_ident(s: &str) -> &str {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"'))
        || (s.starts_with('`') && s.ends_with('`'))
        || (s.starts_with('[') && s.ends_with(']'))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn sql_tokens(statement: &str) -> Vec<&str> {
    let mut tokens: Vec<&str> = Vec::new();
    let mut chars = statement.char_indices().peekable();
    let mut start = 0usize;
    while let Some((i, c)) = chars.next() {
        match c {
            '"' | '`' => skip_until(&mut chars, c),
            '[' => skip_until(&mut chars, ']'),
            w if w.is_whitespace() => {
                push_token(&mut tokens, statement, start, i);
                start = i + w.len_utf8();
            }
            _ => {}
        }
    }
    push_token(&mut tokens, statement, start, statement.len());
    tokens
}

fn skip_until(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>, close: char) {
    for (_, ch) in chars.by_ref() {
        if ch == close {
            break;
        }
    }
}

fn push_token<'a>(tokens: &mut Vec<&'a str>, statement: &'a str, start: usize, end: usize) {
    let tok = statement[start..end].trim();
    if !tok.is_empty() {
        tokens.push(tok);
    }
}

fn create_table_name_index(tokens: &[&str]) -> Option<usize> {
    let upper: Vec<String> = tokens.iter().map(|t| t.to_ascii_uppercase()).collect();
    let mut pos = 0usize;
    if upper.get(pos).map(String::as_str) != Some("CREATE") {
        return None;
    }
    pos += 1;
    while matches!(
        upper.get(pos).map(String::as_str),
        Some("GLOBAL") | Some("TEMPORARY")
    ) {
        pos += 1;
    }
    if upper.get(pos).map(String::as_str) != Some("TABLE") {
        return None;
    }
    pos += 1;
    if upper.get(pos).map(String::as_str) == Some("IF") {
        pos += 3;
    }
    Some(pos)
}

fn schema_before_dot(name_tok: &str) -> Option<String> {
    let mut depth = 0u8;
    for (i, c) in name_tok.char_indices() {
        match c {
            '"' | '`' | '[' => depth += 1,
            '\'' | ']' => depth = depth.saturating_sub(1),
            '.' if depth == 0 => return Some(unquote_ident(&name_tok[..i]).to_string()),
            _ => {}
        }
    }
    None
}

fn extract_create_table_schema(statement: &str) -> Option<String> {
    let tokens = sql_tokens(statement);
    let skip = create_table_name_index(&tokens)?;
    schema_before_dot(tokens.get(skip)?)
}

/// Execute a DDL statement (CREATE SCHEMA / CREATE TABLE / ALTER TABLE …)
/// against a target connection.
///
/// Prefers `cdc-jdbc-admin` (JDBC Type 4) when `ORCH_JDBC_ADMIN_URL` is set.
/// Otherwise uses native Rust drivers for Postgres/MySQL/SQL Server/SQLite.
pub async fn exec_ddl_on_connection(
    db_type: &str,
    connection: &HashMap<String, String>,
    statement: &str,
) -> Result<(), AppError> {
    let schema_name = extract_create_table_schema(statement);
    let normalized = db_type.trim().to_ascii_lowercase();

    if let Some(admin) = jdbc_admin() {
        admin
            .ddl(db_type, connection, statement, schema_name.as_deref())
            .await?;
        return Ok(());
    }

    exec_ddl_native(
        db_type,
        connection,
        statement,
        schema_name.as_deref(),
        &normalized,
    )
    .await
}

async fn exec_ddl_native(
    db_type: &str,
    connection: &HashMap<String, String>,
    statement: &str,
    schema_name: Option<&str>,
    normalized: &str,
) -> Result<(), AppError> {
    match normalized {
        "postgres" | "postgresql" => {
            exec_postgres_ddl(db_type, connection, statement, schema_name).await
        }
        "mysql" | "mariadb" => {
            exec_vendor_ddl(db_type, connection, statement, schema_name, |s| {
                format!("CREATE SCHEMA IF NOT EXISTS `{s}`")
            })
            .await
        }
        "sqlserver" | "mssql" => {
            exec_vendor_ddl(db_type, connection, statement, schema_name, |s| {
                let safe = s.replace('\'', "''");
                format!(
                    "IF NOT EXISTS (SELECT 1 FROM sys.schemas WHERE name = N'{safe}') \
                     EXEC(N'CREATE SCHEMA [{safe}]')"
                )
            })
            .await
        }
        "sqlite" => {
            let vendor = crate::trigger_cdc::Vendor::parse(db_type)?;
            crate::trigger_cdc::exec_ddl(vendor, connection, statement).await
        }
        other => {
            require_jdbc_admin(other)?
                .ddl(db_type, connection, statement, schema_name)
                .await?;
            Ok(())
        }
    }
}

fn postgres_ddl_detail(err: &tokio_postgres::Error) -> String {
    let Some(db) = err.as_db_error() else {
        return err.to_string();
    };
    let mut msg = db.message().to_string();
    if let Some(d) = db.detail() {
        msg.push_str(&format!(" — {d}"));
    }
    if let Some(h) = db.hint() {
        msg.push_str(&format!(" (hint: {h})"));
    }
    msg
}

async fn exec_postgres_ddl(
    db_type: &str,
    connection: &HashMap<String, String>,
    statement: &str,
    schema_name: Option<&str>,
) -> Result<(), AppError> {
    let insight = insight_from_connection(db_type, connection);
    let client = postgres_client(&insight).await?;
    let batch = match schema_name {
        Some(s) => format!("CREATE SCHEMA IF NOT EXISTS \"{s}\";\n{statement}"),
        None => statement.to_string(),
    };
    client.batch_execute(&batch).await.map_err(|e| {
        AppError::BadRequest(format!(
            "PostgreSQL DDL failed: {}",
            postgres_ddl_detail(&e)
        ))
    })?;
    Ok(())
}

async fn exec_vendor_ddl(
    db_type: &str,
    connection: &HashMap<String, String>,
    statement: &str,
    schema_name: Option<&str>,
    guard_sql: impl FnOnce(&str) -> String,
) -> Result<(), AppError> {
    let vendor = crate::trigger_cdc::Vendor::parse(db_type)?;
    if let Some(s) = schema_name {
        let guard = guard_sql(s);
        crate::trigger_cdc::exec_ddl(vendor, connection, &guard).await?;
    }
    crate::trigger_cdc::exec_ddl(vendor, connection, statement).await
}

// ── Data Copy ─────────────────────────────────────────────────────────────────

/// Copy rows from a source table to a matching target table (SELECT → batch INSERT).
///
/// Source and target use the same schema.table name. Prefer
/// [`copy_mapped_table_data`] when the replica table is renamed.
#[allow(clippy::too_many_arguments)]
pub async fn copy_table_data(
    src_type: &str,
    src_conn: &HashMap<String, String>,
    tgt_type: &str,
    tgt_conn: &HashMap<String, String>,
    schema: &str,
    table: &str,
    batch_size: usize,
    truncate_before_copy: bool,
) -> Result<i64, AppError> {
    copy_mapped_table_data(
        src_type,
        src_conn,
        tgt_type,
        tgt_conn,
        schema,
        table,
        schema,
        table,
        batch_size,
        truncate_before_copy,
    )
    .await
}

/// Copy rows from `src_schema.src_table` into `tgt_schema.tgt_table`.
///
/// Supported paths:
/// - PostgreSQL → PostgreSQL: native tokio_postgres on both sides.
/// - Other JDBC sources → PostgreSQL: paged reads via cdc-jdbc-admin.
///
/// Returns the number of rows copied.
#[allow(clippy::too_many_arguments)]
pub async fn copy_mapped_table_data(
    src_type: &str,
    src_conn: &HashMap<String, String>,
    tgt_type: &str,
    tgt_conn: &HashMap<String, String>,
    src_schema: &str,
    src_table: &str,
    tgt_schema: &str,
    tgt_table: &str,
    batch_size: usize,
    truncate_before_copy: bool,
) -> Result<i64, AppError> {
    let src_norm = src_type.trim().to_ascii_lowercase();
    let tgt_norm = tgt_type.trim().to_ascii_lowercase();

    match (src_norm.as_str(), tgt_norm.as_str()) {
        ("postgres" | "postgresql", "postgres" | "postgresql") => {
            copy_postgres_to_postgres(
                src_conn,
                tgt_conn,
                src_schema,
                src_table,
                tgt_schema,
                tgt_table,
                batch_size,
                truncate_before_copy,
            )
            .await
        }

        (src, "postgres" | "postgresql") if src != "postgres" && src != "postgresql" => {
            copy_via_jdbc_admin_to_postgres(
                src_type,
                src_conn,
                tgt_conn,
                src_schema,
                src_table,
                tgt_schema,
                tgt_table,
                batch_size,
                truncate_before_copy,
            )
            .await
        }

        (src, tgt) => Err(AppError::BadRequest(format!(
            "direct data copy from '{src}' to '{tgt}' is not yet supported; \
             use a CDC pipeline with SyncOnce capture mode for this combination"
        ))),
    }
}

/// Fold JDBC preview columns to lowercase so `json_populate_record` matches
/// unquoted Postgres identifiers (Oracle reports `BOOK_ID`, the replica has `book_id`).
pub(crate) fn postgres_json_row(columns: &[String], row: &[Value]) -> Value {
    let mut obj = serde_json::Map::new();
    for (idx, col) in columns.iter().enumerate() {
        obj.insert(
            col.to_ascii_lowercase(),
            row.get(idx).cloned().unwrap_or(Value::Null),
        );
    }
    Value::Object(obj)
}

/// Copy rows from a JDBC-only source to PostgreSQL via cdc-jdbc-admin preview pages.
#[allow(clippy::too_many_arguments)]
async fn copy_via_jdbc_admin_to_postgres(
    src_type: &str,
    src_conn: &HashMap<String, String>,
    tgt_conn: &HashMap<String, String>,
    src_schema: &str,
    src_table: &str,
    tgt_schema: &str,
    tgt_table: &str,
    batch_size: usize,
    truncate_before_copy: bool,
) -> Result<i64, AppError> {
    let admin = require_jdbc_admin(src_type)?;
    let tgt_qualified = quote_qualified(tgt_schema, tgt_table)?;

    let tgt_insight = insight_from_connection("postgres", tgt_conn);
    let tgt_client = postgres_client(&tgt_insight)
        .await
        .map_err(|e| AppError::BadRequest(format!("target connection failed: {e}")))?;

    if truncate_before_copy {
        tgt_client
            .execute(&format!("TRUNCATE TABLE {tgt_qualified}"), &[])
            .await
            .map_err(|e| AppError::BadRequest(format!("TRUNCATE {tgt_qualified} failed: {e}")))?;
    }

    let insert_sql = format!(
        "INSERT INTO {tgt_qualified} \
         SELECT (json_populate_record(NULL::{tgt_qualified}, value)).* \
         FROM json_array_elements($1::text::json)"
    );

    let effective_batch = batch_size.clamp(1, 5000) as u64;
    let mut page: u64 = 0;
    let mut copied: i64 = 0;

    loop {
        let preview = admin
            .preview(
                src_type,
                src_conn,
                src_schema,
                src_table,
                page,
                effective_batch,
                None,
            )
            .await?;
        if preview.rows.is_empty() {
            break;
        }
        let objects: Vec<Value> = preview
            .rows
            .iter()
            .map(|row| postgres_json_row(&preview.columns, row))
            .collect();
        let batch_len = objects.len();
        let json_array_str = Value::Array(objects).to_string();
        tgt_client
            .execute(insert_sql.as_str(), &[&json_array_str])
            .await
            .map_err(|e| {
                AppError::BadRequest(format!("INSERT into {tgt_qualified} failed: {e}"))
            })?;
        copied += batch_len as i64;
        if !preview.has_more {
            break;
        }
        page += 1;
    }

    Ok(copied)
}

fn quote_qualified(schema: &str, table: &str) -> Result<String, AppError> {
    let q_schema = quote_ident(schema)
        .ok_or_else(|| AppError::BadRequest(format!("invalid identifier for copy: {schema}")))?;
    let q_table = quote_ident(table)
        .ok_or_else(|| AppError::BadRequest(format!("invalid identifier for copy: {table}")))?;
    Ok(format!("{q_schema}.{q_table}"))
}

/// Fastest path: PostgreSQL → PostgreSQL via the server-level COPY protocol.
///
/// `COPY … TO STDOUT (FORMAT CSV)` on the source produces a raw byte stream;
/// `COPY … FROM STDIN (FORMAT CSV)` on the target consumes it.  No JSON
/// serialisation, no INSERT overhead — throughput is typically 50k–500k rows/s
/// compared with ~3k–10k rows/s for the json_populate_record INSERT approach.
///
/// CSV format (not BINARY) is used so that minor type differences between the
/// two tables (e.g. VARCHAR vs TEXT, NUMERIC precision) are handled by
/// Postgres's input converters rather than failing at the binary level.
///
/// Implementation note — `CopyInSink` is `!Unpin`, so we:
///   1. `COUNT(*)` the source upfront (one extra query) to return the row count,
///   2. `tokio::pin!` both stream and sink so `SinkExt` + `StreamExt` work,
///   3. use `Sink::close()` (signals COPY EOF) instead of the owned `finish()`.
#[allow(clippy::too_many_arguments)]
async fn copy_postgres_to_postgres(
    src_conn: &HashMap<String, String>,
    tgt_conn: &HashMap<String, String>,
    src_schema: &str,
    src_table: &str,
    tgt_schema: &str,
    tgt_table: &str,
    _batch_size: usize, // not used for COPY — streaming is unbounded
    truncate_before_copy: bool,
) -> Result<i64, AppError> {
    let src_qualified = quote_qualified(src_schema, src_table)?;
    let tgt_qualified = quote_qualified(tgt_schema, tgt_table)?;

    let src_insight = insight_from_connection("postgres", src_conn);
    let tgt_insight = insight_from_connection("postgres", tgt_conn);

    let src_client = postgres_client(&src_insight)
        .await
        .map_err(|e| AppError::BadRequest(format!("source connection failed: {e}")))?;
    let tgt_client = postgres_client(&tgt_insight)
        .await
        .map_err(|e| AppError::BadRequest(format!("target connection failed: {e}")))?;

    // Prefetch row count — used as the return value since CopyInSink::close()
    // (the safe pinned variant) doesn't surface the Postgres row count.
    let row_count: i64 = src_client
        .query_one(&format!("SELECT COUNT(*) FROM {src_qualified}"), &[])
        .await
        .map_err(|e| AppError::BadRequest(format!("COUNT on {src_qualified} failed: {e}")))?
        .get(0);

    if truncate_before_copy {
        tgt_client
            .execute(&format!("TRUNCATE TABLE {tgt_qualified}"), &[])
            .await
            .map_err(|e| AppError::BadRequest(format!("TRUNCATE {tgt_qualified} failed: {e}")))?;
    }

    let copy_out_sql = format!("COPY {src_qualified} TO STDOUT WITH (FORMAT CSV, NULL '\\N')");
    let copy_in_sql = format!("COPY {tgt_qualified} FROM STDIN WITH (FORMAT CSV, NULL '\\N')");

    let byte_stream = src_client
        .copy_out(copy_out_sql.as_str())
        .await
        .map_err(|e| AppError::BadRequest(format!("COPY OUT setup on {src_qualified}: {e}")))?;

    // copy_in<T: ToStatement, U: Buf> — let Rust infer U = bytes::Bytes from usage below.
    let sink = tgt_client
        .copy_in(copy_in_sql.as_str())
        .await
        .map_err(|e| AppError::BadRequest(format!("COPY IN setup on {tgt_qualified}: {e}")))?;

    // `CopyInSink<T>` is `!Unpin`; pin both sides on the stack so that
    // `StreamExt::next` and `SinkExt::send` / `SinkExt::close` work.
    // `Pin<&mut T>: Unpin` always (because `&mut T: Unpin`), so after pinning
    // both sides all the `SinkExt` / `StreamExt` convenience methods are available.
    tokio::pin!(byte_stream);
    tokio::pin!(sink);

    // Pipe raw bytes from source directly to target — no deserialise/reserialise.
    while let Some(chunk) = byte_stream.next().await {
        let chunk = chunk
            .map_err(|e| AppError::BadRequest(format!("COPY OUT read on {src_qualified}: {e}")))?;
        sink.send(chunk)
            .await
            .map_err(|e| AppError::BadRequest(format!("COPY IN write on {tgt_qualified}: {e}")))?;
    }

    // Flush and signal EOF (COPY done message) to Postgres.
    sink.close()
        .await
        .map_err(|e| AppError::BadRequest(format!("COPY IN close on {tgt_qualified}: {e}")))?;

    Ok(row_count)
}

// ── Table Data Preview ────────────────────────────────────────────────────────

/// Preview live rows from a table using the appropriate driver.
///
/// - **PostgreSQL / MySQL / SQLServer / SQLite**: native Rust drivers — supports
///   server-side `search` (ILIKE / LIKE across every text-castable column).
/// - **Oracle / AS400 / DB2 / Informix** (JDBC-only): routes through cdc-jdbc-admin.
pub async fn preview_table_data(
    db_type: &str,
    conn: &HashMap<String, String>,
    schema: &str,
    table: &str,
    page: u64,
    page_size: u64,
    search: Option<&str>,
) -> Result<TablePreviewResponse, AppError> {
    let norm = db_type.trim().to_ascii_lowercase();
    let page_size = page_size.clamp(1, 200);

    if let Some(admin) = jdbc_admin() {
        return admin
            .preview(db_type, conn, schema, table, page, page_size, search)
            .await;
    }

    match norm.as_str() {
        "postgres" | "postgresql" => {
            preview_postgres(conn, schema, table, page, page_size, search).await
        }
        other => Err(AppError::BadRequest(format!(
            "table preview for '{other}' requires cdc-jdbc-admin — set ORCH_JDBC_ADMIN_URL"
        ))),
    }
}

/// Preview rows from a PostgreSQL table using native tokio_postgres.
async fn preview_postgres(
    conn: &HashMap<String, String>,
    schema: &str,
    table: &str,
    page: u64,
    page_size: u64,
    search: Option<&str>,
) -> Result<TablePreviewResponse, AppError> {
    let q_schema = format!("\"{}\"", schema.replace('"', "\"\""));
    let q_table = format!("\"{}\"", table.replace('"', "\"\""));
    let qualified = format!("{q_schema}.{q_table}");

    let insight = insight_from_connection("postgres", conn);
    let client = postgres_client(&insight)
        .await
        .map_err(|e| AppError::BadRequest(format!("connection failed: {e}")))?;

    // Fetch column names so we can output ordered rows.
    let col_rows = client
        .query(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_schema = $1 AND table_name = $2 \
             ORDER BY ordinal_position",
            &[&schema, &table],
        )
        .await
        .map_err(|e| AppError::BadRequest(format!("column query failed: {e}")))?;
    let columns: Vec<String> = col_rows.iter().map(|r| r.get::<_, String>(0)).collect();

    // Build WHERE clause for optional search (ILIKE on every column cast to text).
    let where_clause = match search.filter(|s| !s.is_empty()) {
        Some(term) => {
            let sanitised = term
                .replace('\'', "''")
                .replace('%', "\\%")
                .replace('_', "\\_");
            if columns.is_empty() {
                String::new()
            } else {
                let predicates = columns
                    .iter()
                    .map(|c| {
                        let qc = format!("\"{}\"", c.replace('"', "\"\""));
                        format!("CAST({qc} AS TEXT) ILIKE '%{sanitised}%' ESCAPE '\\'")
                    })
                    .collect::<Vec<_>>()
                    .join(" OR ");
                format!("WHERE {predicates}")
            }
        }
        None => String::new(),
    };

    let limit = page_size + 1; // fetch one extra to detect has_more
    let offset = page * page_size;
    let data_sql =
        format!("SELECT * FROM {qualified} {where_clause} LIMIT {limit} OFFSET {offset}");

    let data_rows = client
        .query(data_sql.as_str(), &[])
        .await
        .map_err(|e| AppError::BadRequest(format!("data query failed: {e}")))?;

    let has_more = data_rows.len() as u64 > page_size;
    let _rows_to_take = data_rows.len().min(page_size as usize);

    // Serialise each row as a JSON text string, then parse to serde_json::Value.
    // (tokio_postgres has no FromSql for serde_json::Value without extra features.)
    let jsonb_sql = format!(
        "SELECT to_jsonb(t)::text FROM ( \
           SELECT * FROM {qualified} {where_clause} LIMIT {page_size} OFFSET {offset} \
         ) t"
    );
    let jsonb_rows = client
        .query(jsonb_sql.as_str(), &[])
        .await
        .map_err(|e| AppError::BadRequest(format!("JSON serialise query failed: {e}")))?;

    let rows: Vec<Vec<serde_json::Value>> = jsonb_rows
        .iter()
        .map(|row| {
            let raw: String = row.try_get::<_, String>(0).unwrap_or_default();
            let jsonb: serde_json::Value =
                serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
            columns
                .iter()
                .map(|c| jsonb.get(c).cloned().unwrap_or(serde_json::Value::Null))
                .collect()
        })
        .collect();

    Ok(TablePreviewResponse {
        columns,
        rows,
        page,
        page_size,
        has_more,
    })
}

pub async fn inspect_source_table(
    payload: &SourceTableInspectRequest,
) -> Result<SourceTableInspectResponse, AppError> {
    let source = &payload.source;
    if source.source_type.trim().is_empty() {
        return Err(AppError::BadRequest("source_type is required".to_string()));
    }
    if let Some(admin) = jdbc_admin() {
        return admin
            .inspect(
                &source.source_type,
                &source.connection,
                payload.schema.trim(),
                payload.name.trim(),
            )
            .await;
    }
    match source.source_type.trim().to_ascii_lowercase().as_str() {
        "postgres" | "postgresql" => inspect_postgres_table(payload).await,
        other => Err(AppError::BadRequest(format!(
            "table inspect for '{other}' requires cdc-jdbc-admin — set ORCH_JDBC_ADMIN_URL"
        ))),
    }
}

pub(crate) fn insight_from_connection(
    source_type: &str,
    connection: &HashMap<String, String>,
) -> ConnectorInsight {
    ConnectorInsight {
        kind: ConnectorKind::Source,
        database_type: source_type.trim().to_ascii_lowercase(),
        database_label: source_type.trim().to_string(),
        class_name: String::new(),
        connection: ConnectorConnectionView {
            host: connection.get("host").cloned().unwrap_or_default(),
            port: connection.get("port").cloned().unwrap_or_default(),
            database: connection.get("database").cloned().unwrap_or_default(),
            username: connection.get("username").cloned().unwrap_or_default(),
            password: connection.get("password").cloned().unwrap_or_default(),
            jdbc_url: None,
        },
        configured_schemas: vec![],
        configured_tables: vec![],
    }
}

async fn load_postgres_or_err(
    insight: &ConnectorInsight,
    schema_filter: &[String],
) -> Result<LiveCatalog, AppError> {
    match insight.database_type.as_str() {
        "postgres" | "postgresql" => load_postgres_catalog(insight, schema_filter).await,
        other => Err(AppError::BadRequest(format!(
            "live catalog is available for PostgreSQL; {other} shows configured capture lists only"
        ))),
    }
}

pub(crate) async fn postgres_client(
    insight: &ConnectorInsight,
) -> Result<tokio_postgres::Client, AppError> {
    let host = insight.connection.host.trim();
    let port_raw = insight.connection.port.trim();
    let user = insight.connection.username.trim();
    let database = insight.connection.database.trim();
    if host.is_empty() || user.is_empty() || database.is_empty() {
        return Err(AppError::BadRequest(
            "PostgreSQL catalog needs host, username, and database".to_string(),
        ));
    }
    let port = port_raw
        .parse::<u16>()
        .map_err(|_| AppError::BadRequest(format!("invalid database port: {port_raw}")))?;

    let mut cfg = tokio_postgres::Config::new();
    cfg.host(host);
    cfg.port(port);
    cfg.user(user);
    if !insight.connection.password.is_empty() {
        cfg.password(&insight.connection.password);
    }
    cfg.dbname(database);
    cfg.connect_timeout(Duration::from_secs(5));

    let (client, connection) = timeout(Duration::from_secs(8), cfg.connect(NoTls))
        .await
        .map_err(|_| {
            AppError::BadRequest(format!(
                "catalog timed out connecting to {host}:{port}/{database}"
            ))
        })?
        .map_err(|e| {
            AppError::BadRequest(format!(
                "catalog connection failed to {host}:{port}/{database}: {e}"
            ))
        })?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(client)
}

async fn load_postgres_catalog(
    insight: &ConnectorInsight,
    schema_filter: &[String],
) -> Result<LiveCatalog, AppError> {
    let client = postgres_client(insight).await?;

    let schema_rows = client
        .query(
            "SELECT nspname FROM pg_namespace \
             WHERE nspname NOT LIKE 'pg_%' \
               AND nspname <> 'information_schema' \
             ORDER BY nspname",
            &[],
        )
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to list schemas: {e}")))?;
    let schemas: Vec<String> = schema_rows
        .into_iter()
        .filter_map(|row| row.try_get::<_, String>(0).ok())
        .filter(|name| !is_system_schema(name))
        .collect();

    let table_rows = client
        .query(
            "SELECT n.nspname, c.relname, \
                    CASE WHEN c.reltuples < 0 THEN NULL \
                         ELSE ROUND(c.reltuples)::bigint END \
             FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind IN ('r', 'p') \
               AND n.nspname NOT LIKE 'pg_%' \
               AND n.nspname <> 'information_schema' \
             ORDER BY n.nspname, c.relname \
             LIMIT 500",
            &[],
        )
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to list tables: {e}")))?;
    let mut tables: Vec<LiveTable> = table_rows
        .into_iter()
        .filter_map(|row| {
            let schema = row.try_get::<_, String>(0).ok()?;
            let name = row.try_get::<_, String>(1).ok()?;
            if is_system_schema(&schema) {
                return None;
            }
            let row_count = row.try_get::<_, Option<i64>>(2).ok().flatten();
            Some(LiveTable {
                schema,
                name,
                row_count,
                primary_key: Vec::new(),
            })
        })
        .collect();

    fill_primary_keys(&client, &mut tables).await;
    fill_exact_row_counts(&client, &mut tables, schema_filter).await;

    Ok(LiveCatalog { schemas, tables })
}

async fn inspect_postgres_table(
    payload: &SourceTableInspectRequest,
) -> Result<SourceTableInspectResponse, AppError> {
    let schema = payload.schema.trim();
    let table = payload.name.trim();
    let Some(quoted_schema) = quote_ident(schema) else {
        return Err(AppError::BadRequest(format!(
            "invalid schema name: {schema}"
        )));
    };
    let Some(quoted_table) = quote_ident(table) else {
        return Err(AppError::BadRequest(format!("invalid table name: {table}")));
    };
    let limit = payload
        .limit
        .unwrap_or(PREVIEW_ROW_LIMIT_DEFAULT)
        .clamp(1, PREVIEW_ROW_LIMIT_MAX);

    let insight = insight_from_connection(&payload.source.source_type, &payload.source.connection);
    let client = postgres_client(&insight).await?;

    let column_rows = client
        .query(
            "SELECT a.attname, \
                    pg_catalog.format_type(a.atttypid, a.atttypmod), \
                    NOT a.attnotnull, \
                    a.attnum, \
                    COALESCE(( \
                      SELECT x.ordinality::int \
                      FROM pg_index i \
                      JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS x(attnum, ordinality) ON true \
                      WHERE i.indrelid = a.attrelid AND i.indisprimary AND x.attnum = a.attnum \
                      LIMIT 1 \
                    ), 0), \
                    EXISTS ( \
                      SELECT 1 FROM pg_index i \
                      WHERE i.indrelid = a.attrelid AND i.indisunique AND a.attnum = ANY(i.indkey) \
                    ) \
             FROM pg_attribute a \
             JOIN pg_class c ON c.oid = a.attrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 \
               AND c.relname = $2 \
               AND a.attnum > 0 \
               AND NOT a.attisdropped \
             ORDER BY a.attnum",
            &[&schema, &table],
        )
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to load table structure: {e}")))?;

    if column_rows.is_empty() {
        return Err(AppError::BadRequest(format!(
            "table {schema}.{table} was not found"
        )));
    }

    let mut columns = Vec::new();
    let mut primary_key = Vec::new();
    for row in column_rows {
        let name: String = row.try_get(0).unwrap_or_default();
        let data_type: String = row.try_get(1).unwrap_or_default();
        let nullable: bool = row.try_get(2).unwrap_or(true);
        let position: i16 = row.try_get(3).unwrap_or(0);
        let pk_position: i32 = row.try_get(4).unwrap_or(0);
        let unique: bool = row.try_get(5).unwrap_or(false);
        if pk_position > 0 {
            primary_key.push((pk_position, name.clone()));
        }
        columns.push(SourceTableColumn {
            name,
            data_type,
            nullable,
            position: position as i32,
            primary_key: pk_position > 0,
            pk_position: if pk_position > 0 {
                Some(pk_position)
            } else {
                None
            },
            unique,
        });
    }
    primary_key.sort_by_key(|(position, _)| *position);
    let primary_key: Vec<String> = primary_key.into_iter().map(|(_, name)| name).collect();

    let mut row_count = None;
    let count_sql = format!("SELECT COUNT(*)::bigint FROM {quoted_schema}.{quoted_table}");
    if let Ok(Ok(row)) = timeout(Duration::from_secs(2), client.query_one(&count_sql, &[])).await {
        row_count = row.try_get::<_, i64>(0).ok().map(|n| n.max(0));
    }

    let preview_sql = format!(
        "SELECT to_jsonb(src)::text FROM {quoted_schema}.{quoted_table} AS src LIMIT {limit}"
    );
    let preview_rows = client
        .query(&preview_sql, &[])
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to preview table data: {e}")))?;
    let mut rows = Vec::new();
    for row in preview_rows {
        let raw: String = row.try_get(0).unwrap_or_else(|_| "{}".to_string());
        match serde_json::from_str::<Value>(&raw) {
            Ok(Value::Object(map)) => rows.push(Value::Object(map)),
            Ok(other) => rows.push(other),
            Err(_) => rows.push(Value::String(raw)),
        }
    }

    Ok(SourceTableInspectResponse {
        schema: schema.to_string(),
        name: table.to_string(),
        row_count,
        primary_key,
        columns,
        rows,
        limit,
        timestamp: chrono::Utc::now(),
    })
}

async fn fill_primary_keys(client: &tokio_postgres::Client, tables: &mut [LiveTable]) {
    let Ok(rows) = client
        .query(
            "SELECT n.nspname, c.relname, a.attname \
             FROM pg_index i \
             JOIN pg_class c ON c.oid = i.indrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS x(attnum, ordinality) ON true \
             JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = x.attnum \
             WHERE i.indisprimary \
               AND n.nspname NOT LIKE 'pg_%' \
               AND n.nspname <> 'information_schema' \
             ORDER BY n.nspname, c.relname, x.ordinality",
            &[],
        )
        .await
    else {
        return;
    };
    let grouped = group_primary_keys(
        rows.into_iter()
            .filter_map(|row| {
                Some((
                    row.try_get::<_, String>(0).ok()?,
                    row.try_get::<_, String>(1).ok()?,
                    row.try_get::<_, String>(2).ok()?,
                ))
            })
            .collect(),
    );
    for table in tables.iter_mut() {
        if let Some(keys) = grouped.get(&(table.schema.clone(), table.name.clone())) {
            table.primary_key = keys.clone();
        }
    }
}

fn group_primary_keys(
    rows: Vec<(String, String, String)>,
) -> HashMap<(String, String), Vec<String>> {
    let mut map: HashMap<(String, String), Vec<String>> = HashMap::new();
    for (schema, table, column) in rows {
        map.entry((schema, table)).or_default().push(column);
    }
    map
}

/// Read real PRIMARY KEY columns for every user table on a PostgreSQL target,
/// keyed by `(schema, table)` lower-cased for case-insensitive lookup.
///
/// jdbc-admin catalog includes primary keys when DatabaseMetaData reports them.
/// Trigger apply still queries the target directly so apply never depends on
/// catalog completeness.
pub async fn postgres_primary_keys(
    client: &tokio_postgres::Client,
) -> Result<HashMap<(String, String), Vec<String>>, AppError> {
    let rows = client
        .query(
            "SELECT n.nspname, c.relname, a.attname \
             FROM pg_index i \
             JOIN pg_class c ON c.oid = i.indrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS x(attnum, ordinality) ON true \
             JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = x.attnum \
             WHERE i.indisprimary \
               AND n.nspname NOT LIKE 'pg_%' \
               AND n.nspname <> 'information_schema' \
             ORDER BY n.nspname, c.relname, x.ordinality",
            &[],
        )
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to read target primary keys: {e}")))?;
    Ok(group_primary_keys(
        rows.into_iter()
            .filter_map(|row| {
                Some((
                    row.try_get::<_, String>(0).ok()?.to_ascii_lowercase(),
                    row.try_get::<_, String>(1).ok()?.to_ascii_lowercase(),
                    row.try_get::<_, String>(2).ok()?,
                ))
            })
            .collect(),
    ))
}

fn quote_ident(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.contains('\0') {
        return None;
    }
    Some(format!("\"{}\"", name.replace('"', "\"\"")))
}

fn tables_to_count<'a>(
    tables: &'a mut [LiveTable],
    schema_filter: &[String],
) -> Vec<&'a mut LiveTable> {
    let filter: Vec<String> = schema_filter
        .iter()
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect();
    if filter.is_empty() {
        if tables.len() > EXACT_COUNT_TABLE_LIMIT {
            return Vec::new();
        }
        return tables.iter_mut().collect();
    }
    tables
        .iter_mut()
        .filter(|table| {
            filter
                .iter()
                .any(|wanted| table.schema.eq_ignore_ascii_case(wanted))
        })
        .collect()
}

async fn fill_exact_row_counts(
    client: &tokio_postgres::Client,
    tables: &mut [LiveTable],
    schema_filter: &[String],
) {
    let targets = tables_to_count(tables, schema_filter);
    if targets.is_empty() {
        return;
    }
    let _ = client.batch_execute("SET statement_timeout = '2000'").await;
    let deadline = Instant::now() + EXACT_COUNT_BUDGET;
    for table in targets {
        if Instant::now() >= deadline {
            break;
        }
        let Some(schema) = quote_ident(&table.schema) else {
            continue;
        };
        let Some(name) = quote_ident(&table.name) else {
            continue;
        };
        let sql = format!("SELECT COUNT(*)::bigint FROM {schema}.{name}");
        let counted = timeout(EXACT_COUNT_PER_TABLE, client.query_one(&sql, &[])).await;
        if let Ok(Ok(row)) = counted {
            if let Ok(count) = row.try_get::<_, i64>(0) {
                table.row_count = Some(count.max(0));
            }
        }
    }
}

fn is_system_schema(name: &str) -> bool {
    SYSTEM_SCHEMAS.iter().any(|s| name.eq_ignore_ascii_case(s)) || name.starts_with("pg_")
}

/// Return row counts for every table in a schema using catalog/statistics tables.
/// One network round-trip per call; values are estimates (last ANALYZE) for Postgres/MySQL.
pub async fn count_schema_tables(
    db_type: &str,
    conn: &HashMap<String, String>,
    schema: &str,
) -> Result<SchemaCountsResponse, AppError> {
    if let Some(admin) = jdbc_admin() {
        return admin.count_schema(db_type, conn, schema).await;
    }
    let norm = db_type.trim().to_ascii_lowercase();
    let mut counts: HashMap<String, i64> = HashMap::new();
    let estimated;

    match norm.as_str() {
        // ── PostgreSQL: pg_stat_user_tables — one query for all tables ──────
        "postgres" | "postgresql" => {
            estimated = true;
            let insight = insight_from_connection("postgres", conn);
            let client = postgres_client(&insight)
                .await
                .map_err(|e| AppError::BadRequest(format!("connection failed: {e}")))?;
            // n_live_tup is updated after DML by autovacuum; reltuples is stale until ANALYZE.
            // Use n_live_tup when available (> 0 or table was vacuumed), fall back to reltuples.
            let rows = client.query(
                "SELECT c.relname, \
                    GREATEST( \
                      COALESCE(NULLIF(s.n_live_tup, 0), NULLIF(GREATEST(c.reltuples::bigint, 0), 0), 0), \
                      0 \
                    )::bigint \
                 FROM pg_class c \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 LEFT JOIN pg_stat_user_tables s ON s.relid = c.oid \
                 WHERE n.nspname = $1 AND c.relkind = 'r'",
                &[&schema],
            ).await.map_err(|e| AppError::BadRequest(format!("count query failed: {e}")))?;
            for row in rows {
                let name: String = row.get(0);
                let cnt: i64 = row.get(1);
                counts.insert(name, cnt);
            }
        }
        // ── MySQL / MariaDB: information_schema.TABLES — one query ──────────
        "mysql" | "mariadb" => {
            estimated = true;
            use mysql_async::{prelude::Queryable, Opts, OptsBuilder, Pool};
            let host = conn
                .get("host")
                .cloned()
                .unwrap_or_else(|| "localhost".into());
            let port: u16 = conn
                .get("port")
                .and_then(|p| p.parse().ok())
                .unwrap_or(3306);
            let db = conn.get("database").cloned().unwrap_or_default();
            let user = conn.get("username").cloned().unwrap_or_default();
            let pass = conn.get("password").cloned().unwrap_or_default();
            let opts = OptsBuilder::default()
                .ip_or_hostname(host)
                .tcp_port(port)
                .db_name(Some(db.clone()))
                .user(Some(user))
                .pass(Some(pass));
            let pool = Pool::new(Opts::from(opts));
            let mut mc = pool
                .get_conn()
                .await
                .map_err(|e| AppError::BadRequest(format!("MySQL connection failed: {e}")))?;
            let rows: Vec<(String, Option<u64>)> = mc
                .query(format!(
                    "SELECT TABLE_NAME, TABLE_ROWS \
                 FROM information_schema.TABLES \
                 WHERE TABLE_SCHEMA = '{schema}' AND TABLE_TYPE = 'BASE TABLE'"
                ))
                .await
                .map_err(|e| AppError::BadRequest(format!("count query failed: {e}")))?;
            for (name, cnt) in rows {
                counts.insert(name, cnt.unwrap_or(0) as i64);
            }
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "{other} schema counts require cdc-jdbc-admin — set ORCH_JDBC_ADMIN_URL"
            )));
        }
    }

    Ok(SchemaCountsResponse {
        schema: schema.to_string(),
        estimated,
        counts,
    })
}

/// Return the exact row count for a single table.
pub async fn count_table_rows(
    db_type: &str,
    conn: &HashMap<String, String>,
    schema: &str,
    table: &str,
) -> Result<TableCountResponse, AppError> {
    if let Some(admin) = jdbc_admin() {
        return admin.count_table(db_type, conn, schema, table).await;
    }
    let norm = db_type.trim().to_ascii_lowercase();
    let q_schema = format!("\"{}\"", schema.replace('"', "\"\""));
    let q_table = format!("\"{}\"", table.replace('"', "\"\""));
    let sql = format!("SELECT COUNT(*) FROM {q_schema}.{q_table}");

    let count: i64 = match norm.as_str() {
        "postgres" | "postgresql" => {
            let insight = insight_from_connection("postgres", conn);
            let client = postgres_client(&insight)
                .await
                .map_err(|e| AppError::BadRequest(format!("connection failed: {e}")))?;
            let row = client
                .query_one(sql.as_str(), &[])
                .await
                .map_err(|e| AppError::BadRequest(format!("count query failed: {e}")))?;
            row.get::<_, i64>(0)
        }
        "mysql" | "mariadb" => {
            use mysql_async::{prelude::Queryable, Opts, OptsBuilder, Pool};
            let host = conn
                .get("host")
                .cloned()
                .unwrap_or_else(|| "localhost".into());
            let port: u16 = conn
                .get("port")
                .and_then(|p| p.parse().ok())
                .unwrap_or(3306);
            let database = conn.get("database").cloned().unwrap_or_default();
            let user = conn.get("username").cloned().unwrap_or_default();
            let pass = conn.get("password").cloned().unwrap_or_default();
            let opts = OptsBuilder::default()
                .ip_or_hostname(host)
                .tcp_port(port)
                .db_name(Some(database))
                .user(Some(user))
                .pass(Some(pass));
            let pool = Pool::new(Opts::from(opts));
            let mut mysql_conn = pool
                .get_conn()
                .await
                .map_err(|e| AppError::BadRequest(format!("MySQL connection failed: {e}")))?;
            let row: Option<(i64,)> = mysql_conn
                .query_first(sql)
                .await
                .map_err(|e| AppError::BadRequest(format!("count query failed: {e}")))?;
            row.map(|(n,)| n).unwrap_or(0)
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "{other} row count requires cdc-jdbc-admin — set ORCH_JDBC_ADMIN_URL"
            )));
        }
    };

    Ok(TableCountResponse {
        schema: schema.to_string(),
        table: table.to_string(),
        count,
    })
}

#[cfg(test)]
mod tests {
    use super::{group_primary_keys, postgres_json_row, quote_ident, tables_to_count, LiveTable};
    use serde_json::json;

    fn table(schema: &str, name: &str) -> LiveTable {
        LiveTable {
            schema: schema.into(),
            name: name.into(),
            row_count: None,
            primary_key: Vec::new(),
        }
    }

    #[test]
    fn quotes_schema_and_table_for_count() {
        assert_eq!(quote_ident("bookings").as_deref(), Some("\"bookings\""));
        assert_eq!(quote_ident("tickets").as_deref(), Some("\"tickets\""));
        assert_eq!(quote_ident("odd\"name").as_deref(), Some("\"odd\"\"name\""));
        assert_eq!(quote_ident(""), None);
    }

    #[test]
    fn exact_count_targets_selected_schema() {
        let mut tables = vec![table("bookings", "tickets"), table("public", "users")];
        let targets = tables_to_count(&mut tables, &["bookings".into()]);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "tickets");
    }

    #[test]
    fn oracle_preview_keys_fold_to_postgres_columns() {
        let row = postgres_json_row(
            &["BOOK_ID".into(), "TITLE".into()],
            &[json!(1004), json!("a16")],
        );
        assert_eq!(row["book_id"], 1004);
        assert_eq!(row["title"], "a16");
        assert!(row.get("BOOK_ID").is_none());
    }

    #[test]
    fn groups_compound_primary_keys_in_order() {
        let grouped = group_primary_keys(vec![
            ("bookings".into(), "tickets".into(), "airline_id".into()),
            ("bookings".into(), "tickets".into(), "ticket_no".into()),
            ("bookings".into(), "airports".into(), "airport_code".into()),
        ]);
        assert_eq!(
            grouped.get(&("bookings".into(), "tickets".into())).unwrap(),
            &vec!["airline_id".to_string(), "ticket_no".to_string()]
        );
        assert_eq!(
            grouped
                .get(&("bookings".into(), "airports".into()))
                .unwrap(),
            &vec!["airport_code".to_string()]
        );
    }
}
