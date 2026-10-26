//! Vendor dialects for trigger-based CDC.
//!
//! Native install runs on PostgreSQL, MySQL/MariaDB, SQL Server, and SQLite.
//! Oracle install / status / prune go through `cdc-jdbc-admin` (JDBC Type 4).
//! DB2 and Informix still generate SQL for the operator to apply.

use std::collections::HashMap;

use crate::error::AppError;
use crate::postgres_trigger_cdc::{
    self, captured_tables_from_names, CapturedTable, CAPTURE_SCHEMA, CHANGE_LOG,
};

mod exec;
mod sql;

pub use exec::exec_ddl;
pub use exec::preview_columns;
pub use sql::prune_sql;
pub use sql::source_install_sql;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Postgres,
    Mysql,
    Mariadb,
    SqlServer,
    Sqlite,
    Oracle,
    Db2,
    Informix,
}

impl Vendor {
    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" => Ok(Self::Postgres),
            "mysql" => Ok(Self::Mysql),
            "mariadb" => Ok(Self::Mariadb),
            "sqlserver" | "mssql" => Ok(Self::SqlServer),
            "sqlite" => Ok(Self::Sqlite),
            "oracle" => Ok(Self::Oracle),
            "db2" => Ok(Self::Db2),
            "informix" => Ok(Self::Informix),
            other => Err(AppError::BadRequest(format!(
                "unsupported database type for trigger capture: {other}"
            ))),
        }
    }

    pub fn native_install(self) -> bool {
        matches!(
            self,
            Self::Postgres | Self::Mysql | Self::Mariadb | Self::SqlServer | Self::Sqlite
        )
    }

    /// Vendors the orchestrator can install through `cdc-jdbc-admin` (no bundled Rust driver).
    pub fn remote_install(self) -> bool {
        matches!(self, Self::Oracle)
    }

    pub fn can_execute_install(self) -> bool {
        self.native_install()
            || (self.remote_install() && crate::clients::JdbcAdminClient::from_env().is_some())
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL",
            Self::Mysql => "MySQL",
            Self::Mariadb => "MariaDB",
            Self::SqlServer => "SQL Server",
            Self::Sqlite => "SQLite",
            Self::Oracle => "Oracle",
            Self::Db2 => "IBM DB2",
            Self::Informix => "Informix",
        }
    }

    pub fn changelog_table(self) -> String {
        match self {
            Self::Postgres | Self::SqlServer | Self::Db2 => {
                format!("{CAPTURE_SCHEMA}.{CHANGE_LOG}")
            }
            Self::Mysql | Self::Mariadb | Self::Sqlite | Self::Informix => {
                format!("hyper_cdc_{CHANGE_LOG}")
            }
            Self::Oracle => "HYPER_CDC_CHANGE_LOG".to_string(),
        }
    }

    /// Bare changelog table name, without any schema, in the casing the database stores it.
    ///
    /// Oracle folds unquoted DDL identifiers to upper case; the engines that keep the
    /// changelog in a dedicated schema store it lower case. This is the name the JDBC
    /// driver reports for the table, so it is also what regex matching sees.
    pub fn changelog_table_name(self) -> String {
        match self {
            Self::Postgres | Self::SqlServer | Self::Db2 => CHANGE_LOG.to_string(),
            Self::Mysql | Self::Mariadb | Self::Sqlite | Self::Informix => {
                format!("hyper_cdc_{CHANGE_LOG}")
            }
            Self::Oracle => "HYPER_CDC_CHANGE_LOG".to_string(),
        }
    }

    /// Name of the changelog surrogate key column, in the casing the database stores it.
    pub fn changelog_id_column(self) -> &'static str {
        match self {
            Self::Oracle => "ID",
            _ => "id",
        }
    }
}

/// Schema (Oracle: owner) that holds the changelog table, as the JDBC driver reports it.
///
/// Engines that support schemas get a dedicated `hyper_cdc` schema. The Oracle DDL
/// creates `HYPER_CDC_CHANGE_LOG` unqualified, so it lands in the connecting user's own
/// schema and Oracle folds that name to upper case. MySQL/MariaDB/SQLite/Informix have no
/// schema layer, so the changelog is a prefixed table in the database itself.
pub fn changelog_owner(vendor: Vendor, connection: &HashMap<String, String>) -> Option<String> {
    match vendor {
        Vendor::Postgres | Vendor::SqlServer | Vendor::Db2 => Some(CAPTURE_SCHEMA.to_string()),
        Vendor::Oracle => connection
            .get("username")
            .map(|u| u.trim().to_ascii_uppercase())
            .filter(|u| !u.is_empty()),
        Vendor::Mysql | Vendor::Mariadb | Vendor::Sqlite | Vendor::Informix => None,
    }
}

/// Build the `table.include.list` regex that selects exactly the changelog table.
///
/// Confluent's JDBC source *full-matches* this pattern against the identifier it
/// discovered, which is `[catalog.][schema.]table`. Whether the catalog part is present
/// depends on the driver: Oracle and PostgreSQL report none, MySQL and SQL Server report
/// the database. A bare table name therefore matches nothing on Oracle, and the task dies
/// with "Task is being killed because it was not assigned a table nor a query to execute".
/// The optional leading group absorbs the catalog so one pattern works on every driver.
pub fn changelog_include_regex(
    vendor: Vendor,
    connection: &HashMap<String, String>,
) -> Result<String, AppError> {
    let table = vendor.changelog_table_name();
    safe_ident(&table)?;
    match changelog_owner(vendor, connection) {
        Some(owner) => {
            safe_ident(&owner)?;
            Ok(format!(r"(?:[^.]+\.)?{owner}\.{table}"))
        }
        None => Ok(format!(r"(?:[^.]+\.)?{table}")),
    }
}

#[derive(Debug, Clone)]
pub struct TriggerInstallResult {
    pub installed: Vec<String>,
    pub already_present: Vec<String>,
    pub change_log: String,
    pub sql: String,
    pub executed: bool,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct TriggerPreviewResult {
    pub tables: Vec<String>,
    pub change_log: String,
    pub sql: String,
    pub will_execute: bool,
    pub vendor: String,
    pub message: String,
}

pub fn jdbc_source_url(
    vendor: Vendor,
    host: &str,
    port: &str,
    database: &str,
) -> Result<String, AppError> {
    match vendor {
        Vendor::Postgres => Ok(format!("jdbc:postgresql://{host}:{port}/{database}")),
        Vendor::Mysql | Vendor::Mariadb => Ok(format!("jdbc:mysql://{host}:{port}/{database}")),
        Vendor::SqlServer => Ok(format!(
            "jdbc:sqlserver://{host}:{port};databaseName={database}"
        )),
        Vendor::Oracle => Ok(format!("jdbc:oracle:thin:@//{host}:{port}/{database}")),
        Vendor::Sqlite => Ok(format!("jdbc:sqlite:{database}")),
        Vendor::Db2 => Ok(format!("jdbc:db2://{host}:{port}/{database}")),
        Vendor::Informix => Ok(format!(
            "jdbc:informix-sqli://{host}:{port}/{database}:INFORMIXSERVER=ol_informix"
        )),
    }
}

/// Prune the source changelog table for a trigger-capture profile.
///
/// Returns `Some(rows_deleted)` for vendors the orchestrator can reach
/// (Postgres, MySQL/MariaDB, SQL Server, SQLite, and Oracle via jdbc-admin).
/// Returns `None` for DB2/Informix — those must be pruned by a DBA-scheduled job.
pub async fn prune_changelog(
    source_type: &str,
    connection: &HashMap<String, String>,
    older_than_hours: u32,
) -> Result<Option<u64>, AppError> {
    if older_than_hours == 0 {
        return Ok(Some(0));
    }
    let vendor = Vendor::parse(source_type)?;
    match vendor {
        Vendor::Postgres => Ok(Some(
            crate::postgres_trigger_cdc::prune_changelog_postgres(connection, older_than_hours)
                .await?,
        )),
        Vendor::Mysql | Vendor::Mariadb | Vendor::SqlServer | Vendor::Sqlite => Ok(Some(
            exec::prune_changelog(vendor, connection, older_than_hours).await?,
        )),
        Vendor::Oracle => Ok(Some(
            exec::prune_changelog_remote(vendor, connection, older_than_hours).await?,
        )),
        Vendor::Db2 | Vendor::Informix => Ok(None),
    }
}

/// Resolve the column list used to build each trigger's JSON payload.
///
/// Only vendors whose payload SQL names columns explicitly need this. Postgres and
/// SQL Server capture whole rows implicitly, so an empty map is correct for them.
/// Getting an empty map for a vendor that *does* need it silently degrades the
/// generated DDL to `NULL` payloads, so preview and install must both go through here.
async fn resolve_capture_columns(
    vendor: Vendor,
    connection: &HashMap<String, String>,
    tables: &[CapturedTable],
) -> Result<HashMap<String, Vec<String>>, AppError> {
    if matches!(
        vendor,
        Vendor::Mysql | Vendor::Mariadb | Vendor::Sqlite | Vendor::Oracle
    ) {
        exec::preview_columns(vendor, connection, tables).await
    } else {
        Ok(HashMap::new())
    }
}

pub async fn preview_source_triggers(
    source_type: &str,
    connection: &HashMap<String, String>,
    tables: &[CapturedTable],
) -> Result<TriggerPreviewResult, AppError> {
    let vendor = Vendor::parse(source_type)?;
    let change_log = vendor.changelog_table();
    let columns = resolve_capture_columns(vendor, connection, tables).await?;
    let owner = changelog_owner(vendor, connection);
    let sql = source_install_sql(vendor, tables, &columns, owner.as_deref())?;
    let names: Vec<String> = tables
        .iter()
        .map(|table| format!("{}.{}", table.source_schema, table.source_table))
        .collect();
    let will_execute = vendor.can_execute_install();
    let message = if will_execute {
        format!(
            "Review the {} CREATE statements. Confirming will install changelog triggers on {} table(s) and write to {}.",
            vendor.label(),
            names.len(),
            change_log
        )
    } else if vendor.remote_install() {
        format!(
            "Review the {} CREATE statements. Set ORCH_JDBC_ADMIN_URL so the orchestrator can install them, or copy and run this as a user that can CREATE TRIGGER.",
            vendor.label()
        )
    } else {
        format!(
            "Review the {} CREATE statements. Native install is not bundled for this engine — copy and run this as a user that can CREATE TRIGGER.",
            vendor.label()
        )
    };
    Ok(TriggerPreviewResult {
        tables: names,
        change_log,
        sql,
        will_execute,
        vendor: vendor.label().to_string(),
        message,
    })
}

pub async fn install_source_triggers(
    source_type: &str,
    connection: &HashMap<String, String>,
    tables: &[CapturedTable],
) -> Result<TriggerInstallResult, AppError> {
    let vendor = Vendor::parse(source_type)?;
    let change_log = vendor.changelog_table();
    let columns = resolve_capture_columns(vendor, connection, tables).await?;
    let owner = changelog_owner(vendor, connection);
    let sql = source_install_sql(vendor, tables, &columns, owner.as_deref())?;
    if vendor == Vendor::Postgres {
        let (installed, already_present) =
            postgres_trigger_cdc::install_source_triggers(source_type, connection, tables).await?;
        let count = installed.len() + already_present.len();
        return Ok(TriggerInstallResult {
            installed,
            already_present,
            change_log,
            sql,
            executed: true,
            message: format!("Changelog triggers ready on {count} table(s)."),
        });
    }
    if vendor.remote_install() {
        if crate::clients::JdbcAdminClient::from_env().is_some() {
            return exec::install_remote(vendor, connection, tables).await;
        }
        return Ok(TriggerInstallResult {
            installed: Vec::new(),
            already_present: Vec::new(),
            change_log,
            sql,
            executed: false,
            message: format!(
                "{} trigger SQL is ready. Set ORCH_JDBC_ADMIN_URL so the orchestrator can install it, or run the script as a user that can CREATE TRIGGER.",
                vendor.label()
            ),
        });
    }
    if !vendor.native_install() {
        return Ok(TriggerInstallResult {
            installed: Vec::new(),
            already_present: Vec::new(),
            change_log,
            sql,
            executed: false,
            message: format!(
                "{} trigger SQL is ready. Native install is not bundled for this engine — run the script as a user that can CREATE TRIGGER.",
                vendor.label()
            ),
        });
    }
    exec::install(vendor, connection, tables).await
}

pub async fn source_trigger_status(
    source_type: &str,
    connection: &HashMap<String, String>,
    requested: &[String],
) -> Result<(bool, Vec<(String, String, bool)>), AppError> {
    let vendor = Vendor::parse(source_type)?;
    if vendor == Vendor::Postgres {
        return postgres_trigger_cdc::source_trigger_status(source_type, connection, requested)
            .await;
    }
    if vendor.remote_install() {
        return exec::status_remote(vendor, connection, requested).await;
    }
    if !vendor.native_install() {
        let mut tables = Vec::new();
        for value in requested {
            if let Ok((schema, table)) = split_name(value) {
                tables.push((schema, table, false));
            }
        }
        return Ok((false, tables));
    }
    exec::status(vendor, connection, requested).await
}

pub fn tables_from_names(names: &[String]) -> Result<Vec<CapturedTable>, AppError> {
    captured_tables_from_names(names)
}

pub async fn live_catalog(
    source_type: &str,
    connection: &HashMap<String, String>,
) -> Result<(Vec<String>, Vec<(String, String)>), AppError> {
    let vendor = Vendor::parse(source_type)?;
    exec::catalog(vendor, connection).await
}

pub(crate) fn split_name(qualified: &str) -> Result<(String, String), AppError> {
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

pub(crate) fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(crate) fn safe_ident(name: &str) -> Result<&str, AppError> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > 128
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || name.starts_with(|c: char| c.is_ascii_digit())
    {
        return Err(AppError::BadRequest(format!(
            "invalid database identifier: {name}"
        )));
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(schema: &str, name: &str) -> CapturedTable {
        CapturedTable {
            source_schema: schema.into(),
            source_table: name.into(),
            target_schema: schema.into(),
            target_table: name.into(),
            pk_columns: vec!["id".into()],
        }
    }

    #[test]
    fn mysql_sql_creates_three_triggers() {
        let sql = source_install_sql(
            Vendor::Mysql,
            &[table("shop", "orders")],
            &HashMap::from([("shop.orders".into(), vec!["id".into(), "status".into()])]),
            None,
        )
        .unwrap();
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS `hyper_cdc_change_log`"));
        assert!(sql.contains("hyper_cdc_ai_orders"));
        assert!(sql.contains("AFTER INSERT ON `shop`.`orders`"));
        assert!(sql.contains("JSON_OBJECT('id', NEW.`id`"));
    }

    #[test]
    fn sqlserver_sql_uses_inserted_deleted() {
        let sql = source_install_sql(
            Vendor::SqlServer,
            &[table("dbo", "orders")],
            &HashMap::new(),
            None,
        )
        .unwrap();
        assert!(sql.contains("CREATE SCHEMA hyper_cdc"));
        assert!(sql.contains("AFTER INSERT, UPDATE, DELETE"));
        assert!(sql.contains("FOR JSON PATH"));
    }

    #[test]
    fn oracle_and_db2_sql_are_generated() {
        let oracle = source_install_sql(
            Vendor::Oracle,
            &[table("SALES", "ORDERS")],
            &HashMap::from([(
                "SALES.ORDERS".into(),
                vec!["CUSTOMER_ID".into(), "STATUS".into()],
            )]),
            None,
        )
        .unwrap();
        assert!(oracle.contains("HYPER_CDC_CHANGE_LOG"));
        assert!(oracle.contains("AFTER INSERT OR UPDATE OR DELETE"));
        assert!(oracle.contains("KEY 'customer_id'"));
        assert!(oracle.contains(":NEW.\"CUSTOMER_ID\""));
        let db2 = source_install_sql(
            Vendor::Db2,
            &[table("SALES", "ORDERS")],
            &HashMap::new(),
            None,
        )
        .unwrap();
        assert!(db2.contains("hyper_cdc.change_log"));
        assert!(db2.contains("AFTER INSERT"));
    }

    #[test]
    fn prune_sql_is_age_based_and_vendor_specific() {
        assert!(prune_sql(Vendor::Postgres, 72, None).contains("make_interval(hours => 72)"));
        assert!(prune_sql(Vendor::Oracle, 24, None).contains("NUMTODSINTERVAL(24, 'HOUR')"));
        assert!(prune_sql(Vendor::SqlServer, 48, None).contains("DATEADD(HOUR, -48"));
    }

    /// A scheduled prune that is not owner-qualified deletes from whichever changelog the
    /// DBA's own schema happens to hold, not the one the triggers write to.
    #[test]
    fn oracle_prune_sql_targets_the_owner_qualified_changelog() {
        let sql = prune_sql(Vendor::Oracle, 24, Some("bcs"));
        assert!(
            sql.contains("DELETE FROM \"BCS\".HYPER_CDC_CHANGE_LOG"),
            "{sql}"
        );
    }

    #[tokio::test]
    async fn preview_postgres_sql_does_not_need_a_live_connection() {
        let result =
            preview_source_triggers("postgres", &HashMap::new(), &[table("bookings", "tickets")])
                .await
                .unwrap();
        assert!(result.will_execute);
        assert_eq!(result.vendor, "PostgreSQL");
        assert_eq!(result.tables, vec!["bookings.tickets".to_string()]);
        assert!(result.sql.contains("CREATE TRIGGER hyper_cdc_capture"));
        assert!(result.sql.contains("bookings"));
        assert!(result.sql.contains("tickets"));
        assert!(result.change_log.contains("change_log"));
    }

    #[tokio::test]
    async fn preview_oracle_requires_a_column_source() {
        // Oracle payloads name every column explicitly, so preview must resolve them from
        // the live table. Failing loudly beats emitting DDL whose payload is a literal NULL.
        let err = preview_source_triggers("oracle", &HashMap::new(), &[table("SALES", "ORDERS")])
            .await
            .unwrap_err();
        assert!(
            format!("{err:?}").contains("jdbc-admin"),
            "expected a jdbc-admin hint, got: {err:?}"
        );
    }

    #[test]
    fn oracle_trigger_payload_carries_row_data_not_null() {
        let sql = source_install_sql(
            Vendor::Oracle,
            &[table("SALES", "ORDERS")],
            &HashMap::from([(
                "SALES.ORDERS".into(),
                vec!["CUSTOMER_ID".into(), "STATUS".into()],
            )]),
            None,
        )
        .unwrap();
        // An empty column list degrades JSON_OBJECT to `NULL`, which silently replicates
        // operations with no row data and leaves every target column unset.
        assert!(
            !sql.contains("'c', NULL, NULL"),
            "insert payload was NULL: {sql}"
        );
        assert!(
            !sql.contains("'u', NULL, NULL"),
            "update payload was NULL: {sql}"
        );
        assert!(sql.contains("JSON_OBJECT"));
        assert!(sql.contains(":OLD.\"CUSTOMER_ID\""));
    }

    /// Unqualified names inside an Oracle trigger resolve against the trigger owner's
    /// schema, which is the captured table's schema — not necessarily the capture user's.
    /// When those differ, the triggers write to a changelog the connector never reads.
    #[test]
    fn oracle_ddl_qualifies_every_changelog_reference_with_the_owner() {
        let sql = source_install_sql(
            Vendor::Oracle,
            &[table("SALES", "ORDERS")],
            &HashMap::from([("SALES.ORDERS".into(), vec!["CUSTOMER_ID".into()])]),
            Some("bcs"),
        )
        .unwrap();
        assert!(
            sql.contains("CREATE TABLE \"BCS\".HYPER_CDC_CHANGE_LOG"),
            "{sql}"
        );
        assert!(
            sql.contains("CREATE SEQUENCE \"BCS\".HYPER_CDC_CHANGE_LOG_SEQ"),
            "{sql}"
        );
        assert!(
            sql.contains("BEFORE INSERT ON \"BCS\".HYPER_CDC_CHANGE_LOG"),
            "{sql}"
        );
        assert!(
            sql.contains("\"BCS\".HYPER_CDC_CHANGE_LOG_SEQ.NEXTVAL"),
            "{sql}"
        );
        // The capture trigger lives in SALES but must still write to the BCS changelog.
        assert!(
            sql.contains("INSERT INTO \"BCS\".HYPER_CDC_CHANGE_LOG(source_schema"),
            "{sql}"
        );
        assert!(
            !sql.contains("INSERT INTO HYPER_CDC_CHANGE_LOG("),
            "an unqualified insert would resolve against the trigger owner: {sql}"
        );
    }

    /// The owner the DDL writes to must be the one the JDBC include list reads from.
    #[test]
    fn oracle_owner_is_the_capture_user_and_matches_the_include_list() {
        let connection = HashMap::from([("username".into(), "bcs".into())]);
        assert_eq!(
            changelog_owner(Vendor::Oracle, &connection).as_deref(),
            Some("BCS")
        );
        let pattern = changelog_include_regex(Vendor::Oracle, &connection).unwrap();
        assert!(pattern.contains("BCS"));
        // Vendors without a schema layer keep the changelog in the database itself.
        assert_eq!(changelog_owner(Vendor::Mysql, &connection), None);
        assert_eq!(
            changelog_owner(Vendor::Postgres, &connection).as_deref(),
            Some("hyper_cdc")
        );
    }

    #[test]
    fn oracle_is_remote_install_and_needs_jdbc_admin_to_execute() {
        assert!(Vendor::Oracle.remote_install());
        assert!(!Vendor::Oracle.native_install());
        assert!(!Vendor::Db2.remote_install());
        // Unit tests do not set ORCH_JDBC_ADMIN_URL, so install stays a review/copy step.
        std::env::remove_var("ORCH_JDBC_ADMIN_URL");
        assert!(!Vendor::Oracle.can_execute_install());
    }
}
