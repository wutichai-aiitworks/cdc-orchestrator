use std::collections::HashMap;

use crate::error::AppError;
use crate::postgres_trigger_cdc::{self, CapturedTable};

use super::{sql_literal, Vendor};

/// Generate the changelog + trigger DDL for a source database.
///
/// `owner` names the schema that holds the changelog table and is only consulted for
/// Oracle, where an unqualified name resolves against whichever schema owns the running
/// trigger. See [`oracle_statements`].
pub fn source_install_sql(
    vendor: Vendor,
    tables: &[CapturedTable],
    columns: &HashMap<String, Vec<String>>,
    owner: Option<&str>,
) -> Result<String, AppError> {
    Ok(install_statements(vendor, tables, columns, owner)?.join("\n\n"))
}

pub fn install_statements(
    vendor: Vendor,
    tables: &[CapturedTable],
    columns: &HashMap<String, Vec<String>>,
    owner: Option<&str>,
) -> Result<Vec<String>, AppError> {
    match vendor {
        Vendor::Postgres => Ok(vec![postgres_trigger_cdc::source_install_sql(tables)?]),
        Vendor::Mysql | Vendor::Mariadb => mysql_statements(tables, columns),
        Vendor::SqlServer => sqlserver_statements(tables),
        Vendor::Sqlite => sqlite_statements(tables, columns),
        Vendor::Oracle => oracle_statements(tables, columns, owner),
        Vendor::Db2 => db2_statements(tables, columns),
        Vendor::Informix => informix_statements(tables, columns),
    }
}

fn cols_for(table: &CapturedTable, columns: &HashMap<String, Vec<String>>) -> Vec<String> {
    let key = format!("{}.{}", table.source_schema, table.source_table);
    columns
        .get(&key)
        .cloned()
        .or_else(|| columns.get(&key.to_ascii_lowercase()).cloned())
        .unwrap_or_else(|| table.pk_columns.clone())
}

fn mysql_quote(name: &str) -> Result<String, AppError> {
    let name = super::safe_ident(name)?;
    Ok(format!("`{}`", name.replace('`', "``")))
}

fn mssql_quote(name: &str) -> Result<String, AppError> {
    let name = super::safe_ident(name)?;
    Ok(format!("[{}]", name.replace(']', "]]")))
}

fn oracle_quote(name: &str) -> Result<String, AppError> {
    let name = super::safe_ident(name)?;
    Ok(format!(
        "\"{}\"",
        name.replace('"', "\"\"").to_ascii_uppercase()
    ))
}

fn ansi_quote(name: &str) -> Result<String, AppError> {
    let name = super::safe_ident(name)?;
    Ok(format!("\"{}\"", name.replace('"', "\"\"")))
}

fn trigger_suffix(table: &str, max: usize) -> String {
    let trimmed: String = table
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if trimmed.len() <= max {
        trimmed
    } else {
        trimmed.chars().take(max).collect()
    }
}

fn mysql_json(alias: &str, columns: &[String]) -> Result<String, AppError> {
    if columns.is_empty() {
        return Ok("NULL".into());
    }
    let mut parts = Vec::new();
    for column in columns {
        let ident = mysql_quote(column)?;
        parts.push(format!("{}, {alias}.{ident}", sql_literal(column)));
    }
    Ok(format!("JSON_OBJECT({})", parts.join(", ")))
}

fn mysql_statements(
    tables: &[CapturedTable],
    columns: &HashMap<String, Vec<String>>,
) -> Result<Vec<String>, AppError> {
    let mut out = Vec::new();
    out.push(
        "CREATE TABLE IF NOT EXISTS `hyper_cdc_change_log` (\n\
         \tid BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,\n\
         \tsource_schema VARCHAR(128) NOT NULL,\n\
         \tsource_table VARCHAR(128) NOT NULL,\n\
         \top CHAR(1) NOT NULL,\n\
         \tpk JSON NULL,\n\
         \trow_data JSON NULL,\n\
         \tchanged_at TIMESTAMP(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),\n\
         \tKEY hyper_cdc_change_log_id (id)\n\
         )"
        .into(),
    );
    for table in tables {
        let schema = mysql_quote(&table.source_schema)?;
        let name = mysql_quote(&table.source_table)?;
        let cols = cols_for(table, columns);
        let json_new = mysql_json("NEW", &cols)?;
        let json_old = mysql_json("OLD", &cols)?;
        let suffix = trigger_suffix(&table.source_table, 40);
        let schema_lit = sql_literal(&table.source_schema);
        let table_lit = sql_literal(&table.source_table);
        for (kind, event, alias_json, op) in [
            ("ai", "INSERT", json_new.as_str(), "c"),
            ("au", "UPDATE", json_new.as_str(), "u"),
            ("ad", "DELETE", json_old.as_str(), "d"),
        ] {
            let trig = format!("hyper_cdc_{kind}_{suffix}");
            out.push(format!("DROP TRIGGER IF EXISTS `{trig}`"));
            let row = if op == "d" { "NULL" } else { alias_json };
            let pk = alias_json;
            out.push(format!(
                "CREATE TRIGGER `{trig}`\n\
                 AFTER {event} ON {schema}.{name}\n\
                 FOR EACH ROW\n\
                 INSERT INTO `hyper_cdc_change_log`(source_schema, source_table, op, pk, row_data)\n\
                 VALUES ({schema_lit}, {table_lit}, '{op}', {pk}, {row})"
            ));
        }
    }
    Ok(out)
}

fn sqlserver_statements(tables: &[CapturedTable]) -> Result<Vec<String>, AppError> {
    let mut out = Vec::new();
    out.push(
        "IF NOT EXISTS (SELECT 1 FROM sys.schemas WHERE name = 'hyper_cdc') EXEC('CREATE SCHEMA hyper_cdc')"
            .into(),
    );
    out.push(
        "IF OBJECT_ID('hyper_cdc.change_log', 'U') IS NULL\n\
         CREATE TABLE hyper_cdc.change_log (\n\
         \tid bigint IDENTITY(1,1) PRIMARY KEY,\n\
         \tsource_schema nvarchar(128) NOT NULL,\n\
         \tsource_table nvarchar(128) NOT NULL,\n\
         \top char(1) NOT NULL,\n\
         \tpk nvarchar(max) NULL,\n\
         \trow_data nvarchar(max) NULL,\n\
         \tchanged_at datetime2 NOT NULL CONSTRAINT df_hyper_cdc_changed_at DEFAULT SYSUTCDATETIME()\n\
         )"
        .into(),
    );
    for table in tables {
        let schema = mssql_quote(&table.source_schema)?;
        let name = mssql_quote(&table.source_table)?;
        let trig = format!("hyper_cdc_{}", trigger_suffix(&table.source_table, 80));
        let schema_lit = sql_literal(&table.source_schema);
        let table_lit = sql_literal(&table.source_table);
        out.push(format!(
            "IF OBJECT_ID(N'{schema}.[{trig}]', N'TR') IS NOT NULL DROP TRIGGER {schema}.[{trig}]"
        ));
        out.push(format!(
            "CREATE TRIGGER {schema}.[{trig}] ON {schema}.{name}\n\
             AFTER INSERT, UPDATE, DELETE\n\
             AS\n\
             BEGIN\n\
               SET NOCOUNT ON;\n\
               IF NOT EXISTS (SELECT 1 FROM inserted)\n\
               BEGIN\n\
                 INSERT INTO hyper_cdc.change_log(source_schema, source_table, op, pk, row_data)\n\
                 SELECT {schema_lit}, {table_lit}, 'd',\n\
                        (SELECT d.* FOR JSON PATH, WITHOUT_ARRAY_WRAPPER),\n\
                        NULL\n\
                 FROM deleted d;\n\
                 RETURN;\n\
               END\n\
               IF NOT EXISTS (SELECT 1 FROM deleted)\n\
               BEGIN\n\
                 INSERT INTO hyper_cdc.change_log(source_schema, source_table, op, pk, row_data)\n\
                 SELECT {schema_lit}, {table_lit}, 'c',\n\
                        (SELECT i.* FOR JSON PATH, WITHOUT_ARRAY_WRAPPER),\n\
                        (SELECT i.* FOR JSON PATH, WITHOUT_ARRAY_WRAPPER)\n\
                 FROM inserted i;\n\
                 RETURN;\n\
               END\n\
               INSERT INTO hyper_cdc.change_log(source_schema, source_table, op, pk, row_data)\n\
               SELECT {schema_lit}, {table_lit}, 'u',\n\
                      (SELECT i.* FOR JSON PATH, WITHOUT_ARRAY_WRAPPER),\n\
                      (SELECT i.* FOR JSON PATH, WITHOUT_ARRAY_WRAPPER)\n\
               FROM inserted i;\n\
             END"
        ));
    }
    Ok(out)
}

fn sqlite_json(alias: &str, columns: &[String]) -> Result<String, AppError> {
    if columns.is_empty() {
        return Ok("NULL".into());
    }
    let mut parts = Vec::new();
    for column in columns {
        let ident = ansi_quote(column)?;
        parts.push(format!("{}, {alias}.{ident}", sql_literal(column)));
    }
    Ok(format!("json_object({})", parts.join(", ")))
}

fn sqlite_statements(
    tables: &[CapturedTable],
    columns: &HashMap<String, Vec<String>>,
) -> Result<Vec<String>, AppError> {
    let mut out = Vec::new();
    out.push(
        "CREATE TABLE IF NOT EXISTS hyper_cdc_change_log (\n\
         \tid INTEGER PRIMARY KEY AUTOINCREMENT,\n\
         \tsource_schema TEXT NOT NULL,\n\
         \tsource_table TEXT NOT NULL,\n\
         \top TEXT NOT NULL,\n\
         \tpk TEXT,\n\
         \trow_data TEXT,\n\
         \tchanged_at TEXT NOT NULL DEFAULT (datetime('now'))\n\
         )"
        .into(),
    );
    for table in tables {
        let name = ansi_quote(&table.source_table)?;
        let cols = cols_for(table, columns);
        let json_new = sqlite_json("NEW", &cols)?;
        let json_old = sqlite_json("OLD", &cols)?;
        let suffix = trigger_suffix(&table.source_table, 40);
        let schema_lit = sql_literal(&table.source_schema);
        let table_lit = sql_literal(&table.source_table);
        for (kind, event, pk, row, op) in [
            ("ai", "INSERT", json_new.as_str(), json_new.as_str(), "c"),
            ("au", "UPDATE", json_new.as_str(), json_new.as_str(), "u"),
            ("ad", "DELETE", json_old.as_str(), "NULL", "d"),
        ] {
            let trig = format!("hyper_cdc_{kind}_{suffix}");
            out.push(format!("DROP TRIGGER IF EXISTS \"{trig}\""));
            out.push(format!(
                "CREATE TRIGGER \"{trig}\"\n\
                 AFTER {event} ON {name}\n\
                 FOR EACH ROW\n\
                 BEGIN\n\
                   INSERT INTO hyper_cdc_change_log(source_schema, source_table, op, pk, row_data)\n\
                   VALUES ({schema_lit}, {table_lit}, '{op}', {pk}, {row});\n\
                 END"
            ));
        }
    }
    Ok(out)
}

fn oracle_json(alias: &str, columns: &[String]) -> Result<String, AppError> {
    if columns.is_empty() {
        return Ok("NULL".into());
    }
    let mut parts = Vec::new();
    for column in columns {
        let ident = oracle_quote(column)?;
        // Emit lowercase JSON keys so Postgres jsonb_populate_record matches
        // folded-lowercase target columns (Oracle identifiers are UPPERCASE).
        parts.push(format!(
            "KEY {} VALUE {alias}.{ident}",
            sql_literal(&column.to_ascii_lowercase())
        ));
    }
    Ok(format!("JSON_OBJECT({} RETURNING CLOB)", parts.join(", ")))
}

/// Oracle changelog + trigger DDL, all of it qualified by `owner`.
///
/// Qualification is not cosmetic. An unqualified `HYPER_CDC_CHANGE_LOG` inside a trigger
/// resolves against the schema that owns *the trigger*, while the JDBC source reads the
/// changelog in the schema of the *connecting user*. When a captured table lives outside
/// the capture user's own schema those are two different tables, and changes are written
/// to one while the connector tails the other — a silent, total loss of capture. Naming
/// the owner everywhere makes install and read provably agree.
fn oracle_statements(
    tables: &[CapturedTable],
    columns: &HashMap<String, Vec<String>>,
    owner: Option<&str>,
) -> Result<Vec<String>, AppError> {
    let log = match owner {
        Some(owner) => format!("{}.HYPER_CDC_CHANGE_LOG", oracle_quote(owner)?),
        None => "HYPER_CDC_CHANGE_LOG".to_string(),
    };
    let seq = match owner {
        Some(owner) => format!("{}.HYPER_CDC_CHANGE_LOG_SEQ", oracle_quote(owner)?),
        None => "HYPER_CDC_CHANGE_LOG_SEQ".to_string(),
    };
    let mut out = Vec::new();
    // Oracle 10g compatible: no GENERATED ALWAYS AS IDENTITY (12c+ only).
    // Use NUMBER(19,0) + sequence + BEFORE INSERT trigger for the surrogate key.
    // Each DDL is wrapped so re-running is idempotent (ORA-955 = name already used).
    out.push(format!(
        "BEGIN\n\
         \tEXECUTE IMMEDIATE 'CREATE TABLE {log} (\n\
         \tid NUMBER(19,0) NOT NULL,\n\
         \tsource_schema VARCHAR2(128) NOT NULL,\n\
         \tsource_table VARCHAR2(128) NOT NULL,\n\
         \top CHAR(1) NOT NULL,\n\
         \tpk CLOB,\n\
         \trow_data CLOB,\n\
         \tchanged_at TIMESTAMP DEFAULT SYSTIMESTAMP NOT NULL,\n\
         \tCONSTRAINT pk_hyper_cdc_change_log PRIMARY KEY (id))';\n\
         EXCEPTION WHEN OTHERS THEN\n\
         \tIF SQLCODE != -955 THEN RAISE; END IF;\n\
         END;"
    ));
    out.push(format!(
        "BEGIN\n\
         \tEXECUTE IMMEDIATE 'CREATE SEQUENCE {seq} START WITH 1 INCREMENT BY 1 NOCACHE NOCYCLE';\n\
         EXCEPTION WHEN OTHERS THEN\n\
         \tIF SQLCODE != -955 THEN RAISE; END IF;\n\
         END;"
    ));
    let bi_trigger = match owner {
        Some(owner) => format!("{}.HYPER_CDC_CHANGE_LOG_BI", oracle_quote(owner)?),
        None => "HYPER_CDC_CHANGE_LOG_BI".to_string(),
    };
    out.push(format!(
        "CREATE OR REPLACE TRIGGER {bi_trigger}\n\
         BEFORE INSERT ON {log}\n\
         FOR EACH ROW\n\
         BEGIN\n\
         \tSELECT {seq}.NEXTVAL INTO :NEW.id FROM DUAL;\n\
         END;"
    ));
    for table in tables {
        let schema = oracle_quote(&table.source_schema)?;
        let name = oracle_quote(&table.source_table)?;
        let cols = cols_for(table, columns);
        let json_new = oracle_json(":NEW", &cols)?;
        let json_old = oracle_json(":OLD", &cols)?;
        let trig = format!(
            "HYPER_CDC_{}",
            trigger_suffix(&table.source_table, 20).to_ascii_uppercase()
        );
        let schema_lit = sql_literal(&table.source_schema);
        let table_lit = sql_literal(&table.source_table);
        out.push(format!(
            "CREATE OR REPLACE TRIGGER {schema}.\"{trig}\"\n\
             AFTER INSERT OR UPDATE OR DELETE ON {schema}.{name}\n\
             FOR EACH ROW\n\
             BEGIN\n\
               IF DELETING THEN\n\
                 INSERT INTO {log}(source_schema, source_table, op, pk, row_data)\n\
                 VALUES ({schema_lit}, {table_lit}, 'd', {json_old}, NULL);\n\
               ELSIF UPDATING THEN\n\
                 INSERT INTO {log}(source_schema, source_table, op, pk, row_data)\n\
                 VALUES ({schema_lit}, {table_lit}, 'u', {json_new}, {json_new});\n\
               ELSE\n\
                 INSERT INTO {log}(source_schema, source_table, op, pk, row_data)\n\
                 VALUES ({schema_lit}, {table_lit}, 'c', {json_new}, {json_new});\n\
               END IF;\n\
             END;"
        ));
    }
    Ok(out)
}

fn db2_json(alias: &str, columns: &[String]) -> Result<String, AppError> {
    if columns.is_empty() {
        return Ok("NULL".into());
    }
    let mut parts = Vec::new();
    for column in columns {
        let ident = ansi_quote(column)?;
        parts.push(format!("KEY {} VALUE {alias}.{ident}", sql_literal(column)));
    }
    Ok(format!("JSON_OBJECT({})", parts.join(", ")))
}

fn db2_statements(
    tables: &[CapturedTable],
    columns: &HashMap<String, Vec<String>>,
) -> Result<Vec<String>, AppError> {
    let mut out = Vec::new();
    out.push("CREATE SCHEMA hyper_cdc".into());
    out.push(
        "CREATE TABLE hyper_cdc.change_log (\n\
         \tid BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,\n\
         \tsource_schema VARCHAR(128) NOT NULL,\n\
         \tsource_table VARCHAR(128) NOT NULL,\n\
         \top CHAR(1) NOT NULL,\n\
         \tpk CLOB,\n\
         \trow_data CLOB,\n\
         \tchanged_at TIMESTAMP NOT NULL DEFAULT CURRENT TIMESTAMP\n\
         )"
        .into(),
    );
    for table in tables {
        let schema = ansi_quote(&table.source_schema)?;
        let name = ansi_quote(&table.source_table)?;
        let cols = cols_for(table, columns);
        let json_new = db2_json("N", &cols)?;
        let json_old = db2_json("O", &cols)?;
        let suffix = trigger_suffix(&table.source_table, 40);
        let schema_lit = sql_literal(&table.source_schema);
        let table_lit = sql_literal(&table.source_table);
        for (kind, event, ref_clause, pk, row, op) in [
            (
                "ai",
                "INSERT",
                "REFERENCING NEW AS N",
                json_new.as_str(),
                json_new.as_str(),
                "c",
            ),
            (
                "au",
                "UPDATE",
                "REFERENCING NEW AS N OLD AS O",
                json_new.as_str(),
                json_new.as_str(),
                "u",
            ),
            (
                "ad",
                "DELETE",
                "REFERENCING OLD AS O",
                json_old.as_str(),
                "NULL",
                "d",
            ),
        ] {
            let trig = format!("hyper_cdc_{kind}_{suffix}");
            out.push(format!(
                "CREATE OR REPLACE TRIGGER {schema}.{trig}\n\
                 AFTER {event} ON {schema}.{name}\n\
                 {ref_clause}\n\
                 FOR EACH ROW\n\
                 INSERT INTO hyper_cdc.change_log(source_schema, source_table, op, pk, row_data)\n\
                 VALUES ({schema_lit}, {table_lit}, '{op}', {pk}, {row})"
            ));
        }
    }
    Ok(out)
}

fn informix_statements(
    tables: &[CapturedTable],
    columns: &HashMap<String, Vec<String>>,
) -> Result<Vec<String>, AppError> {
    let mut out = Vec::new();
    out.push(
        "CREATE TABLE IF NOT EXISTS hyper_cdc_change_log (\n\
         \tid SERIAL PRIMARY KEY,\n\
         \tsource_schema VARCHAR(128) NOT NULL,\n\
         \tsource_table VARCHAR(128) NOT NULL,\n\
         \top CHAR(1) NOT NULL,\n\
         \tpk LVARCHAR(32739),\n\
         \trow_data LVARCHAR(32739),\n\
         \tchanged_at DATETIME YEAR TO FRACTION(5) DEFAULT CURRENT YEAR TO FRACTION(5)\n\
         )"
        .into(),
    );
    for table in tables {
        let name = ansi_quote(&table.source_table)?;
        let cols = cols_for(table, columns);
        let json_new = db2_json("n", &cols)?;
        let json_old = db2_json("o", &cols)?;
        let suffix = trigger_suffix(&table.source_table, 40);
        let schema_lit = sql_literal(&table.source_schema);
        let table_lit = sql_literal(&table.source_table);
        for (kind, event, refs, pk, row, op) in [
            (
                "ai",
                "INSERT",
                "REFERENCING NEW AS n",
                json_new.as_str(),
                json_new.as_str(),
                "c",
            ),
            (
                "au",
                "UPDATE",
                "REFERENCING NEW AS n OLD AS o",
                json_new.as_str(),
                json_new.as_str(),
                "u",
            ),
            (
                "ad",
                "DELETE",
                "REFERENCING OLD AS o",
                json_old.as_str(),
                "NULL",
                "d",
            ),
        ] {
            let trig = format!("hyper_cdc_{kind}_{suffix}");
            out.push(format!(
                "CREATE TRIGGER {trig} {event} ON {name}\n\
                 {refs}\n\
                 FOR EACH ROW\n\
                 (\n\
                   INSERT INTO hyper_cdc_change_log(source_schema, source_table, op, pk, row_data)\n\
                   VALUES ({schema_lit}, {table_lit}, '{op}', {pk}, {row})\n\
                 )"
            ));
        }
    }
    Ok(out)
}

/// Age-based DELETE for the vendor changelog table. `hours` is interpolated as
/// a u32 so it cannot inject SQL. Operators for Oracle/DB2/Informix can run
/// this on a schedule; the orchestrator runs it natively for the other vendors.
///
/// `owner` must be the same schema the install DDL used, or the scheduled job prunes a
/// different table than the one the triggers write to. Only Oracle consults it.
pub fn prune_sql(vendor: Vendor, hours: u32, owner: Option<&str>) -> String {
    match vendor {
        Vendor::Postgres => format!(
            "DELETE FROM hyper_cdc.change_log WHERE changed_at < now() - make_interval(hours => {hours})"
        ),
        Vendor::Mysql | Vendor::Mariadb => format!(
            "DELETE FROM `hyper_cdc_change_log` WHERE `changed_at` < DATE_SUB(CURRENT_TIMESTAMP(6), INTERVAL {hours} HOUR)"
        ),
        Vendor::SqlServer => format!(
            "DELETE FROM hyper_cdc.change_log WHERE changed_at < DATEADD(HOUR, -{hours}, SYSUTCDATETIME())"
        ),
        Vendor::Sqlite => format!(
            "DELETE FROM hyper_cdc_change_log WHERE datetime(changed_at) < datetime('now', '-{hours} hours')"
        ),
        Vendor::Oracle => {
            let log = match owner.and_then(|o| oracle_quote(o).ok()) {
                Some(owner) => format!("{owner}.HYPER_CDC_CHANGE_LOG"),
                None => "HYPER_CDC_CHANGE_LOG".to_string(),
            };
            format!(
                "DELETE FROM {log} WHERE changed_at < SYSTIMESTAMP - NUMTODSINTERVAL({hours}, 'HOUR')"
            )
        }
        Vendor::Db2 => format!(
            "DELETE FROM hyper_cdc.change_log WHERE changed_at < CURRENT TIMESTAMP - {hours} HOURS"
        ),
        Vendor::Informix => format!(
            "DELETE FROM hyper_cdc_change_log WHERE changed_at < CURRENT YEAR TO FRACTION(5) - {hours} UNITS HOUR"
        ),
    }
}
