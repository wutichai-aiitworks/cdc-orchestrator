use std::collections::{HashMap, HashSet};
use std::time::Duration;

use mysql_async::prelude::{Query, Queryable, WithParams};
use mysql_async::{OptsBuilder, Pool};
use tokio::time::timeout;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use crate::clients::JdbcAdminClient;
use crate::error::AppError;
use crate::postgres_trigger_cdc::CapturedTable;

use super::sql::{install_statements, prune_sql};
use super::{changelog_owner, split_name, TriggerInstallResult, Vendor};

pub async fn install(
    vendor: Vendor,
    connection: &HashMap<String, String>,
    tables: &[CapturedTable],
) -> Result<TriggerInstallResult, AppError> {
    match vendor {
        Vendor::Mysql | Vendor::Mariadb => mysql_install(connection, tables).await,
        Vendor::SqlServer => mssql_install(connection, tables).await,
        Vendor::Sqlite => sqlite_install(connection, tables).await,
        other => Err(AppError::BadRequest(format!(
            "native trigger install is not available for {}",
            other.label()
        ))),
    }
}

pub async fn status(
    vendor: Vendor,
    connection: &HashMap<String, String>,
    requested: &[String],
) -> Result<(bool, Vec<(String, String, bool)>), AppError> {
    match vendor {
        Vendor::Mysql | Vendor::Mariadb => mysql_status(connection, requested).await,
        Vendor::SqlServer => mssql_status(connection, requested).await,
        Vendor::Sqlite => sqlite_status(connection, requested).await,
        other => Err(AppError::BadRequest(format!(
            "native trigger status is not available for {}",
            other.label()
        ))),
    }
}

fn require_jdbc_admin(vendor: Vendor) -> Result<JdbcAdminClient, AppError> {
    JdbcAdminClient::from_env().ok_or_else(|| {
        AppError::BadRequest(format!(
            "{} trigger control-plane access requires cdc-jdbc-admin — set ORCH_JDBC_ADMIN_URL",
            vendor.label()
        ))
    })
}

fn remote_db_type(vendor: Vendor) -> Result<&'static str, AppError> {
    match vendor {
        Vendor::Oracle => Ok("oracle"),
        other => Err(AppError::BadRequest(format!(
            "remote trigger install is not available for {}",
            other.label()
        ))),
    }
}

/// Install changelog + capture triggers through jdbc-admin (Oracle today).
pub async fn install_remote(
    vendor: Vendor,
    connection: &HashMap<String, String>,
    tables: &[CapturedTable],
) -> Result<TriggerInstallResult, AppError> {
    let db_type = remote_db_type(vendor)?;
    let admin = require_jdbc_admin(vendor)?;
    let (_change_log_present, installed_rows) = oracle_status(&admin, connection, &[]).await?;
    let mut before = HashSet::new();
    for (schema, table, present) in installed_rows {
        if present {
            before.insert(format!("{schema}.{table}").to_ascii_lowercase());
        }
    }
    let columns = preview_columns(vendor, connection, tables).await?;
    let owner = changelog_owner(vendor, connection);
    let statements = install_statements(vendor, tables, &columns, owner.as_deref())?;
    for sql in &statements {
        admin
            .ddl(db_type, connection, sql, None)
            .await
            .map_err(|e| {
                AppError::BadRequest(format!(
                    "failed to install {} changelog trigger: {e}",
                    vendor.label()
                ))
            })?;
    }
    Ok(result_from(
        vendor,
        tables,
        &before,
        statements.join("\n\n"),
    ))
}

/// Report which requested tables already have capture triggers (Oracle via jdbc-admin).
pub async fn status_remote(
    vendor: Vendor,
    connection: &HashMap<String, String>,
    requested: &[String],
) -> Result<(bool, Vec<(String, String, bool)>), AppError> {
    let admin = require_jdbc_admin(vendor)?;
    match vendor {
        Vendor::Oracle => oracle_status(&admin, connection, requested).await,
        other => Err(AppError::BadRequest(format!(
            "remote trigger status is not available for {}",
            other.label()
        ))),
    }
}

pub async fn prune_changelog_remote(
    vendor: Vendor,
    connection: &HashMap<String, String>,
    older_than_hours: u32,
) -> Result<u64, AppError> {
    let db_type = remote_db_type(vendor)?;
    let admin = require_jdbc_admin(vendor)?;
    let owner = changelog_owner(vendor, connection);
    let sql = prune_sql(vendor, older_than_hours, owner.as_deref());
    let deleted = admin.ddl(db_type, connection, &sql, None).await?;
    Ok(deleted.max(0) as u64)
}

async fn oracle_status(
    admin: &JdbcAdminClient,
    connection: &HashMap<String, String>,
    requested: &[String],
) -> Result<(bool, Vec<(String, String, bool)>), AppError> {
    let owner = changelog_owner(Vendor::Oracle, connection);
    let change_log_present = oracle_changelog_present(admin, connection, owner.as_deref()).await?;
    let installed = oracle_installed(admin, connection).await?;
    Ok(merge_status(requested, &installed, change_log_present))
}

async fn oracle_changelog_present(
    admin: &JdbcAdminClient,
    connection: &HashMap<String, String>,
    owner: Option<&str>,
) -> Result<bool, AppError> {
    let sql = match owner {
        Some(owner) => {
            let owner = super::safe_ident(owner)?;
            format!(
                "SELECT table_name FROM all_tables \
                 WHERE owner = {owner_lit} AND table_name = 'HYPER_CDC_CHANGE_LOG'",
                owner_lit = super::sql_literal(&owner.to_ascii_uppercase())
            )
        }
        None => "SELECT table_name FROM user_tables WHERE table_name = 'HYPER_CDC_CHANGE_LOG'"
            .to_string(),
    };
    let result = admin.query("oracle", connection, &sql).await?;
    Ok(!result.rows.is_empty())
}

async fn oracle_installed(
    admin: &JdbcAdminClient,
    connection: &HashMap<String, String>,
) -> Result<HashSet<String>, AppError> {
    // Capture triggers are named HYPER_CDC_<table>. Ignore the changelog BEFORE-INSERT
    // trigger so status reports the business tables, not the log itself.
    let sql = "SELECT table_owner, table_name FROM all_triggers \
         WHERE trigger_name LIKE 'HYPER_CDC_%' \
           AND trigger_name <> 'HYPER_CDC_CHANGE_LOG_BI' \
           AND table_name <> 'HYPER_CDC_CHANGE_LOG'";
    let result = admin.query("oracle", connection, sql).await?;
    let mut installed = HashSet::new();
    for row in result.rows {
        let schema = row.first().and_then(|v| v.as_str()).unwrap_or("");
        let table = row.get(1).and_then(|v| v.as_str()).unwrap_or("");
        if !schema.is_empty() && !table.is_empty() {
            installed.insert(format!("{schema}.{table}").to_ascii_lowercase());
        }
    }
    Ok(installed)
}

fn conn_field<'a>(connection: &'a HashMap<String, String>, key: &str) -> Result<&'a str, AppError> {
    connection
        .get(key)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest(format!("missing connection field: {key}")))
}

fn merge_status(
    requested: &[String],
    installed: &HashSet<String>,
    change_log_present: bool,
) -> (bool, Vec<(String, String, bool)>) {
    let mut tables = Vec::new();
    if requested.is_empty() {
        for key in installed {
            if let Ok((schema, table)) = split_name(key) {
                tables.push((schema, table, true));
            }
        }
        return (change_log_present, tables);
    }
    for value in requested {
        if value.trim().is_empty() {
            continue;
        }
        if let Ok((schema, table)) = split_name(value) {
            let present = installed.contains(&format!("{schema}.{table}").to_ascii_lowercase());
            tables.push((schema, table, present));
        }
    }
    (change_log_present, tables)
}

fn result_from(
    vendor: Vendor,
    tables: &[CapturedTable],
    before: &HashSet<String>,
    sql: String,
) -> TriggerInstallResult {
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
    let count = installed.len() + already_present.len();
    TriggerInstallResult {
        installed,
        already_present,
        change_log: vendor.changelog_table(),
        sql,
        executed: true,
        message: format!("Changelog triggers ready on {count} table(s)."),
    }
}

async fn mysql_pool(connection: &HashMap<String, String>) -> Result<Pool, AppError> {
    let host = conn_field(connection, "host")?;
    let port = conn_field(connection, "port")?
        .parse::<u16>()
        .map_err(|_| AppError::BadRequest("invalid MySQL port".into()))?;
    let opts = OptsBuilder::default()
        .ip_or_hostname(host)
        .tcp_port(port)
        .user(Some(conn_field(connection, "username")?))
        .pass(Some(
            connection.get("password").map(|s| s.as_str()).unwrap_or(""),
        ))
        .db_name(Some(conn_field(connection, "database")?))
        .stmt_cache_size(0);
    Ok(Pool::new(opts))
}

async fn mysql_columns(
    conn: &mut mysql_async::Conn,
    schema: &str,
    table: &str,
) -> Result<Vec<String>, AppError> {
    let rows: Vec<(String,)> = "SELECT COLUMN_NAME FROM information_schema.COLUMNS \
         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION"
        .with((schema, table))
        .fetch(conn)
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to list MySQL columns: {e}")))?;
    Ok(rows.into_iter().map(|row| row.0).collect())
}

async fn mysql_installed(conn: &mut mysql_async::Conn) -> Result<HashSet<String>, AppError> {
    let rows: Vec<(String, String)> = "SELECT EVENT_OBJECT_SCHEMA, EVENT_OBJECT_TABLE \
         FROM information_schema.TRIGGERS WHERE TRIGGER_NAME LIKE 'hyper_cdc_%'"
        .fetch(conn)
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to list MySQL triggers: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|(schema, table)| format!("{schema}.{table}").to_ascii_lowercase())
        .collect())
}

async fn mysql_install(
    connection: &HashMap<String, String>,
    tables: &[CapturedTable],
) -> Result<TriggerInstallResult, AppError> {
    let pool = mysql_pool(connection).await?;
    let mut conn = timeout(Duration::from_secs(8), pool.get_conn())
        .await
        .map_err(|_| AppError::BadRequest("MySQL catalog timed out".into()))?
        .map_err(|e| AppError::BadRequest(format!("MySQL connection failed: {e}")))?;
    let before = mysql_installed(&mut conn).await?;
    let mut columns = HashMap::new();
    for table in tables {
        let cols = mysql_columns(&mut conn, &table.source_schema, &table.source_table).await?;
        columns.insert(
            format!("{}.{}", table.source_schema, table.source_table),
            cols,
        );
    }
    let statements = install_statements(Vendor::Mysql, tables, &columns, None)?;
    for sql in &statements {
        sql.as_str()
            .ignore(&mut conn)
            .await
            .map_err(|e| AppError::BadRequest(format!("failed to create MySQL trigger: {e}")))?;
    }
    let _ = conn.disconnect().await;
    Ok(result_from(
        Vendor::Mysql,
        tables,
        &before,
        statements.join(";\n\n"),
    ))
}

async fn mysql_status(
    connection: &HashMap<String, String>,
    requested: &[String],
) -> Result<(bool, Vec<(String, String, bool)>), AppError> {
    let pool = mysql_pool(connection).await?;
    let mut conn = pool
        .get_conn()
        .await
        .map_err(|e| AppError::BadRequest(format!("MySQL connection failed: {e}")))?;
    let present: bool = "SELECT COUNT(*) FROM information_schema.TABLES \
         WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'hyper_cdc_change_log'"
        .first(&mut conn)
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to check MySQL changelog: {e}")))?
        .map(|count: i64| count > 0)
        .unwrap_or(false);
    let installed = mysql_installed(&mut conn).await?;
    let _ = conn.disconnect().await;
    Ok(merge_status(requested, &installed, present))
}

async fn mssql_client(
    connection: &HashMap<String, String>,
) -> Result<tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>, AppError> {
    let host = conn_field(connection, "host")?;
    let port = conn_field(connection, "port")?
        .parse::<u16>()
        .map_err(|_| AppError::BadRequest("invalid SQL Server port".into()))?;
    let mut config = tiberius::Config::new();
    config.host(host);
    config.port(port);
    config.database(conn_field(connection, "database")?);
    config.authentication(tiberius::AuthMethod::sql_server(
        conn_field(connection, "username")?,
        connection.get("password").map(|s| s.as_str()).unwrap_or(""),
    ));
    config.trust_cert();
    config.encryption(tiberius::EncryptionLevel::NotSupported);
    let tcp = timeout(
        Duration::from_secs(8),
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    .map_err(|_| AppError::BadRequest(format!("SQL Server timed out connecting to {host}:{port}")))?
    .map_err(|e| AppError::BadRequest(format!("SQL Server connection failed: {e}")))?;
    tcp.set_nodelay(true)
        .map_err(|e| AppError::BadRequest(format!("SQL Server TCP nodelay failed: {e}")))?;
    tiberius::Client::connect(config, tcp.compat_write())
        .await
        .map_err(|e| AppError::BadRequest(format!("SQL Server handshake failed: {e}")))
}

async fn mssql_installed(
    client: &mut tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>,
) -> Result<HashSet<String>, AppError> {
    let rows = client
        .simple_query(
            "SELECT s.name, t.name \
             FROM sys.triggers tr \
             JOIN sys.tables t ON tr.parent_id = t.object_id \
             JOIN sys.schemas s ON t.schema_id = s.schema_id \
             WHERE tr.name LIKE 'hyper_cdc_%'",
        )
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to list SQL Server triggers: {e}")))?
        .into_first_result()
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to read SQL Server triggers: {e}")))?;
    let mut installed = HashSet::new();
    for row in rows {
        let schema: Option<&str> = row.get(0);
        let table: Option<&str> = row.get(1);
        if let (Some(schema), Some(table)) = (schema, table) {
            installed.insert(format!("{schema}.{table}").to_ascii_lowercase());
        }
    }
    Ok(installed)
}

async fn mssql_install(
    connection: &HashMap<String, String>,
    tables: &[CapturedTable],
) -> Result<TriggerInstallResult, AppError> {
    let mut client = mssql_client(connection).await?;
    let before = mssql_installed(&mut client).await?;
    let statements = install_statements(Vendor::SqlServer, tables, &HashMap::new(), None)?;
    for sql in &statements {
        client
            .simple_query(sql)
            .await
            .map_err(|e| AppError::BadRequest(format!("failed to create SQL Server trigger: {e}")))?
            .into_results()
            .await
            .map_err(|e| {
                AppError::BadRequest(format!("failed to apply SQL Server trigger: {e}"))
            })?;
    }
    Ok(result_from(
        Vendor::SqlServer,
        tables,
        &before,
        statements.join(";\nGO\n"),
    ))
}

async fn mssql_status(
    connection: &HashMap<String, String>,
    requested: &[String],
) -> Result<(bool, Vec<(String, String, bool)>), AppError> {
    let mut client = mssql_client(connection).await?;
    let present = client
        .simple_query(
            "SELECT CASE WHEN OBJECT_ID('hyper_cdc.change_log', 'U') IS NULL THEN 0 ELSE 1 END",
        )
        .await
        .ok()
        .is_some();
    let installed = mssql_installed(&mut client).await.unwrap_or_default();
    Ok(merge_status(requested, &installed, present))
}

fn sqlite_path(connection: &HashMap<String, String>) -> Result<String, AppError> {
    let database = conn_field(connection, "database")?;
    Ok(database.to_string())
}

fn sqlite_installed(conn: &rusqlite::Connection) -> Result<HashSet<String>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'trigger' AND name LIKE 'hyper_cdc_%'",
        )
        .map_err(|e| AppError::BadRequest(format!("failed to list SQLite triggers: {e}")))?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| AppError::BadRequest(format!("failed to read SQLite triggers: {e}")))?;
    let mut installed = HashSet::new();
    for name in names.flatten() {
        if let Some(table) = name.rsplit_once('_').map(|(_, table)| table) {
            installed.insert(format!("main.{table}").to_ascii_lowercase());
        }
    }
    Ok(installed)
}

fn sqlite_columns(conn: &rusqlite::Connection, table: &str) -> Result<Vec<String>, AppError> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| AppError::BadRequest(format!("failed to inspect SQLite table: {e}")))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| AppError::BadRequest(format!("failed to read SQLite columns: {e}")))?;
    Ok(rows.flatten().collect())
}

async fn sqlite_install(
    connection: &HashMap<String, String>,
    tables: &[CapturedTable],
) -> Result<TriggerInstallResult, AppError> {
    let path = sqlite_path(connection)?;
    let tables = tables.to_vec();
    tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open(&path)
            .map_err(|e| AppError::BadRequest(format!("SQLite open failed: {e}")))?;
        let before = sqlite_installed(&conn)?;
        let mut columns = HashMap::new();
        for table in &tables {
            columns.insert(
                format!("{}.{}", table.source_schema, table.source_table),
                sqlite_columns(&conn, &table.source_table)?,
            );
        }
        let statements = install_statements(Vendor::Sqlite, &tables, &columns, None)?;
        for sql in &statements {
            conn.execute_batch(sql).map_err(|e| {
                AppError::BadRequest(format!("failed to create SQLite trigger: {e}"))
            })?;
        }
        Ok(result_from(
            Vendor::Sqlite,
            &tables,
            &before,
            statements.join(";\n\n"),
        ))
    })
    .await
    .map_err(|e| AppError::Internal(format!("SQLite install task failed: {e}")))?
}

pub async fn preview_columns(
    vendor: Vendor,
    connection: &HashMap<String, String>,
    tables: &[CapturedTable],
) -> Result<HashMap<String, Vec<String>>, AppError> {
    match vendor {
        Vendor::Mysql | Vendor::Mariadb => mysql_preview_columns(connection, tables).await,
        Vendor::Sqlite => sqlite_preview_columns(connection, tables).await,
        Vendor::Oracle => oracle_preview_columns(connection, tables).await,
        _ => Ok(HashMap::new()),
    }
}

/// Resolve the full column list for each captured Oracle table.
///
/// Oracle triggers build their payload with `JSON_OBJECT(KEY ... VALUE ...)`, which needs
/// every column named explicitly — unlike Postgres (`to_jsonb`) or SQL Server
/// (`FOR JSON PATH`), which capture the whole row implicitly. Without this the generated
/// DDL falls back to `NULL` and the changelog records operations with no data.
async fn oracle_preview_columns(
    connection: &HashMap<String, String>,
    tables: &[CapturedTable],
) -> Result<HashMap<String, Vec<String>>, AppError> {
    let mut columns = HashMap::new();
    for table in tables {
        let request = crate::models::SourceTableInspectRequest {
            source: crate::models::SourceConfig {
                source_type: "oracle".to_string(),
                connection: connection.clone(),
                capture_mode: crate::models::CaptureMode::default(),
                schema_mode: crate::models::SchemaSelectionMode::SpecificSchemas,
                table_mode: crate::models::TableSelectionMode::SpecificTables,
                schemas: vec![table.source_schema.clone()],
                tables: vec![format!("{}.{}", table.source_schema, table.source_table)],
                changelog_retention_hours: None,
                timestamp_column: None,
                incrementing_column: None,
            },
            schema: table.source_schema.clone(),
            name: table.source_table.clone(),
            // Metadata only — no sample rows needed to build the trigger payload.
            limit: Some(0),
            profile_id: None,
        };
        let detail = crate::connector_catalog::inspect_source_table(&request).await?;
        if detail.columns.is_empty() {
            return Err(AppError::BadRequest(format!(
                "could not read columns for Oracle table {}.{}; trigger payload would be empty",
                table.source_schema, table.source_table
            )));
        }
        columns.insert(
            format!("{}.{}", table.source_schema, table.source_table),
            detail.columns.into_iter().map(|c| c.name).collect(),
        );
    }
    Ok(columns)
}

async fn mysql_preview_columns(
    connection: &HashMap<String, String>,
    tables: &[CapturedTable],
) -> Result<HashMap<String, Vec<String>>, AppError> {
    let pool = mysql_pool(connection).await?;
    let mut conn = timeout(Duration::from_secs(8), pool.get_conn())
        .await
        .map_err(|_| AppError::BadRequest("MySQL catalog timed out".into()))?
        .map_err(|e| AppError::BadRequest(format!("MySQL connection failed: {e}")))?;
    let mut columns = HashMap::new();
    for table in tables {
        let cols = mysql_columns(&mut conn, &table.source_schema, &table.source_table).await?;
        columns.insert(
            format!("{}.{}", table.source_schema, table.source_table),
            cols,
        );
    }
    let _ = conn.disconnect().await;
    Ok(columns)
}

async fn sqlite_preview_columns(
    connection: &HashMap<String, String>,
    tables: &[CapturedTable],
) -> Result<HashMap<String, Vec<String>>, AppError> {
    let path = sqlite_path(connection)?;
    let tables = tables.to_vec();
    tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open(&path)
            .map_err(|e| AppError::BadRequest(format!("SQLite open failed: {e}")))?;
        let mut columns = HashMap::new();
        for table in &tables {
            columns.insert(
                format!("{}.{}", table.source_schema, table.source_table),
                sqlite_columns(&conn, &table.source_table)?,
            );
        }
        Ok(columns)
    })
    .await
    .map_err(|e| AppError::Internal(format!("SQLite preview task failed: {e}")))?
}

pub async fn catalog(
    vendor: Vendor,
    connection: &HashMap<String, String>,
) -> Result<(Vec<String>, Vec<(String, String)>), AppError> {
    match vendor {
        Vendor::Mysql | Vendor::Mariadb => mysql_catalog(connection).await,
        Vendor::SqlServer => mssql_catalog(connection).await,
        Vendor::Sqlite => sqlite_catalog(connection).await,
        other => Err(AppError::BadRequest(format!(
            "live catalog is not bundled for {}",
            other.label()
        ))),
    }
}

async fn mysql_catalog(
    connection: &HashMap<String, String>,
) -> Result<(Vec<String>, Vec<(String, String)>), AppError> {
    let pool = mysql_pool(connection).await?;
    let mut conn = pool
        .get_conn()
        .await
        .map_err(|e| AppError::BadRequest(format!("MySQL connection failed: {e}")))?;
    let schemas: Vec<(String,)> = "SELECT SCHEMA_NAME FROM information_schema.SCHEMATA \
         WHERE SCHEMA_NAME NOT IN ('mysql','information_schema','performance_schema','sys','hyper_cdc') \
         ORDER BY SCHEMA_NAME"
        .fetch(&mut conn)
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to list MySQL schemas: {e}")))?;
    let tables: Vec<(String, String)> =
        "SELECT TABLE_SCHEMA, TABLE_NAME FROM information_schema.TABLES \
         WHERE TABLE_TYPE = 'BASE TABLE' \
           AND TABLE_SCHEMA NOT IN ('mysql','information_schema','performance_schema','sys') \
           AND TABLE_NAME <> 'hyper_cdc_change_log' \
         ORDER BY TABLE_SCHEMA, TABLE_NAME"
            .fetch(&mut conn)
            .await
            .map_err(|e| AppError::BadRequest(format!("failed to list MySQL tables: {e}")))?;
    let _ = conn.disconnect().await;
    Ok((schemas.into_iter().map(|row| row.0).collect(), tables))
}

async fn mssql_catalog(
    connection: &HashMap<String, String>,
) -> Result<(Vec<String>, Vec<(String, String)>), AppError> {
    let mut client = mssql_client(connection).await?;
    let schema_rows = client
        .simple_query(
            "SELECT name FROM sys.schemas \
             WHERE name NOT IN ('sys','INFORMATION_SCHEMA','guest','hyper_cdc') \
               AND name NOT LIKE 'db_%' ORDER BY name",
        )
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to list SQL Server schemas: {e}")))?
        .into_first_result()
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to read SQL Server schemas: {e}")))?;
    let table_rows = client
        .simple_query(
            "SELECT s.name, t.name FROM sys.tables t \
             JOIN sys.schemas s ON t.schema_id = s.schema_id \
             WHERE s.name NOT IN ('sys','INFORMATION_SCHEMA','hyper_cdc') \
             ORDER BY s.name, t.name",
        )
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to list SQL Server tables: {e}")))?
        .into_first_result()
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to read SQL Server tables: {e}")))?;
    let schemas = schema_rows
        .iter()
        .filter_map(|row| row.get::<&str, usize>(0).map(|s| s.to_string()))
        .collect();
    let tables = table_rows
        .iter()
        .filter_map(|row| {
            Some((
                row.get::<&str, usize>(0)?.to_string(),
                row.get::<&str, usize>(1)?.to_string(),
            ))
        })
        .collect();
    Ok((schemas, tables))
}

async fn sqlite_catalog(
    connection: &HashMap<String, String>,
) -> Result<(Vec<String>, Vec<(String, String)>), AppError> {
    let path = sqlite_path(connection)?;
    tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open(&path)
            .map_err(|e| AppError::BadRequest(format!("SQLite open failed: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' \
                 AND name NOT LIKE 'sqlite_%' AND name <> 'hyper_cdc_change_log' ORDER BY name",
            )
            .map_err(|e| AppError::BadRequest(format!("failed to list SQLite tables: {e}")))?;
        let tables = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| AppError::BadRequest(format!("failed to read SQLite tables: {e}")))?
            .flatten()
            .map(|name| ("main".to_string(), name))
            .collect::<Vec<_>>();
        Ok((vec!["main".to_string()], tables))
    })
    .await
    .map_err(|e| AppError::Internal(format!("SQLite catalog task failed: {e}")))?
}

async fn sqlite_status(
    connection: &HashMap<String, String>,
    requested: &[String],
) -> Result<(bool, Vec<(String, String, bool)>), AppError> {
    let path = sqlite_path(connection)?;
    let requested = requested.to_vec();
    tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open(&path)
            .map_err(|e| AppError::BadRequest(format!("SQLite open failed: {e}")))?;
        let present = conn
            .prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'hyper_cdc_change_log'")
            .and_then(|mut stmt| stmt.exists([]))
            .unwrap_or(false);
        let installed = sqlite_installed(&conn)?;
        Ok(merge_status(&requested, &installed, present))
    })
    .await
    .map_err(|e| AppError::Internal(format!("SQLite status task failed: {e}")))?
}

pub async fn prune_changelog(
    vendor: Vendor,
    connection: &HashMap<String, String>,
    older_than_hours: u32,
) -> Result<u64, AppError> {
    if older_than_hours == 0 {
        return Ok(0);
    }
    // Only Oracle needs an owner, and this path never runs for Oracle.
    let sql = super::sql::prune_sql(vendor, older_than_hours, None);
    match vendor {
        Vendor::Mysql | Vendor::Mariadb => mysql_prune(connection, &sql).await,
        Vendor::SqlServer => mssql_prune(connection, &sql).await,
        Vendor::Sqlite => sqlite_prune(connection, &sql).await,
        other => Err(AppError::BadRequest(format!(
            "native changelog prune is not available for {}",
            other.label()
        ))),
    }
}

async fn mysql_prune(connection: &HashMap<String, String>, sql: &str) -> Result<u64, AppError> {
    let pool = mysql_pool(connection).await?;
    let mut conn = timeout(Duration::from_secs(8), pool.get_conn())
        .await
        .map_err(|_| AppError::BadRequest("MySQL prune timed out".into()))?
        .map_err(|e| AppError::BadRequest(format!("MySQL connection failed: {e}")))?;
    sql.ignore(&mut conn)
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to prune MySQL changelog: {e}")))?;
    let deleted: u64 = "SELECT ROW_COUNT()"
        .first(&mut conn)
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to read MySQL prune count: {e}")))?
        .unwrap_or(0);
    let _ = conn.disconnect().await;
    Ok(deleted)
}

async fn mssql_prune(connection: &HashMap<String, String>, sql: &str) -> Result<u64, AppError> {
    let mut client = mssql_client(connection).await?;
    let result = client
        .execute(sql, &[])
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to prune SQL Server changelog: {e}")))?;
    Ok(result.rows_affected().iter().copied().sum::<u64>())
}

async fn sqlite_prune(connection: &HashMap<String, String>, sql: &str) -> Result<u64, AppError> {
    let path = sqlite_path(connection)?;
    let sql = sql.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open(&path)
            .map_err(|e| AppError::BadRequest(format!("SQLite open failed: {e}")))?;
        let deleted = conn
            .execute(&sql, [])
            .map_err(|e| AppError::BadRequest(format!("failed to prune SQLite changelog: {e}")))?;
        Ok(deleted as u64)
    })
    .await
    .map_err(|e| AppError::Internal(format!("SQLite prune task failed: {e}")))?
}

/// Execute a single DDL statement (e.g. CREATE SCHEMA, CREATE TABLE) against a
/// non-Postgres, non-Oracle target.  Postgres and Oracle are handled by the
/// caller in `connector_catalog::exec_ddl_on_connection`.
pub async fn exec_ddl(
    vendor: Vendor,
    connection: &HashMap<String, String>,
    statement: &str,
) -> Result<(), AppError> {
    match vendor {
        Vendor::Mysql | Vendor::Mariadb => {
            let pool = mysql_pool(connection).await?;
            let mut conn = timeout(Duration::from_secs(8), pool.get_conn())
                .await
                .map_err(|_| AppError::BadRequest("MySQL connection timed out".into()))?
                .map_err(|e| AppError::BadRequest(format!("MySQL connection failed: {e}")))?;
            conn.query_drop(statement)
                .await
                .map_err(|e| AppError::BadRequest(format!("MySQL DDL failed: {e}")))?;
            let _ = conn.disconnect().await;
            Ok(())
        }
        Vendor::SqlServer => {
            let mut client = mssql_client(connection).await?;
            client
                .simple_query(statement)
                .await
                .map_err(|e| AppError::BadRequest(format!("SQL Server DDL failed: {e}")))?;
            Ok(())
        }
        Vendor::Sqlite => {
            let path = sqlite_path(connection)?;
            let stmt = statement.to_string();
            tokio::task::spawn_blocking(move || {
                let conn = rusqlite::Connection::open(&path)
                    .map_err(|e| AppError::BadRequest(format!("SQLite open failed: {e}")))?;
                conn.execute_batch(&stmt)
                    .map_err(|e| AppError::BadRequest(format!("SQLite DDL failed: {e}")))?;
                Ok::<_, AppError>(())
            })
            .await
            .map_err(|e| AppError::Internal(format!("SQLite task failed: {e}")))??;
            Ok(())
        }
        other => Err(AppError::BadRequest(format!(
            "exec_ddl not supported for {}",
            other.label()
        ))),
    }
}
