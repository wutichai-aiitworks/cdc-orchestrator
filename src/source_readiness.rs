//! Live CDC readiness check for a pipeline source (ADR 0019).
//!
//! The wizard's static guide (cdc-native-ui `utils/sourceReadiness.ts`) says what a source
//! needs; this module asks the source itself. jdbc-admin runs read-only probes (`/v1/probe`,
//! SELECT-only, one connection) and returns rows or the database's error per probe; the rules
//! here turn them into checks:
//!
//! * `pass` / `warn` / `fail` — the setting was readable and is (not) what CDC needs;
//! * `unknown` — the login may not read it (missing privilege, cloud-hidden view). The check
//!   says which grant would let it see the value; nothing is guessed.
//!
//! Every failing or warning check carries the fix for the detected platform (self-managed,
//! Amazon RDS / Aurora, Cloud SQL, Azure), and the report joins them into one reviewable script.
//! Nothing here changes the source.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::clients::{ProbeResult, ProbeSql, ServerInfo};
use crate::models::{CaptureMode, SourceConfig};

/// At most this many selected tables are checked one by one.
pub const MAX_TABLES: usize = 300;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Warn,
    Fail,
    Unknown,
    Info,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub title: String,
    pub status: Status,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    /// SQL or steps that make the check pass (for this platform).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Param {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Summary {
    pub pass: usize,
    pub warn: usize,
    pub fail: usize,
    pub unknown: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub engine: String,
    pub capture_mode: CaptureMode,
    /// false = this engine/capture mode has no log-based requirements to check.
    pub applicable: bool,
    pub server: Option<ServerInfo>,
    /// self_managed | amazon_rds | amazon_aurora | google_cloud_sql | azure | unknown
    pub platform: String,
    pub checks: Vec<Check>,
    /// Raw system parameters that were readable (for the "System parameters" table).
    pub parameters: Vec<Param>,
    pub fix_script: String,
    pub summary: Summary,
    /// No failing check (unknown ones may still need a DBA to confirm).
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub checked_at: DateTime<Utc>,
}

/// What the rules need besides the probe results.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub engine: String,
    pub capture: CaptureMode,
    pub username: String,
    pub database: String,
    /// (schema/owner/database, table) the pipeline captures; empty = not chosen yet.
    pub tables: Vec<(String, String)>,
    pub tables_truncated: bool,
    /// Postgres publication the connector expects (`connection.publication`, default dbz_publication).
    pub publication: String,
    /// Postgres slot this pipeline uses, when known (saved pipeline or `connection.slot`).
    pub slot: Option<String>,
}

pub fn engine_of(source_type: &str) -> String {
    match source_type.trim().to_ascii_lowercase().as_str() {
        "postgresql" => "postgres".into(),
        "mssql" => "sqlserver".into(),
        other => other.into(),
    }
}

fn default_schema(engine: &str, ctx_db: &str, user: &str) -> String {
    match engine {
        "postgres" => "public".into(),
        "sqlserver" => "dbo".into(),
        "mysql" | "mariadb" => ctx_db.into(),
        "oracle" => user.to_ascii_uppercase(),
        _ => String::new(),
    }
}

pub fn ctx_from_source(src: &SourceConfig, slot: Option<String>) -> Ctx {
    let engine = engine_of(&src.source_type);
    let get = |k: &str| {
        src.connection
            .get(k)
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    let username = {
        let u = get("username");
        if u.is_empty() {
            get("user")
        } else {
            u
        }
    };
    let database = {
        let d = get("database");
        if d.is_empty() {
            get("dbname")
        } else {
            d
        }
    };
    let def = default_schema(&engine, &database, &username);
    let mut seen = HashSet::new();
    let mut tables = Vec::new();
    for t in &src.tables {
        let t = t.trim().trim_matches('"');
        if t.is_empty() {
            continue;
        }
        let (schema, table) = match t.split_once('.') {
            Some((s, n)) => (
                s.trim_matches('"').to_string(),
                n.trim_matches('"').to_string(),
            ),
            None => (def.clone(), t.to_string()),
        };
        if seen.insert((schema.clone(), table.clone())) {
            tables.push((schema, table));
        }
    }
    let tables_truncated = tables.len() > MAX_TABLES;
    tables.truncate(MAX_TABLES);
    let publication = {
        let p = get("publication");
        if p.is_empty() {
            "dbz_publication".into()
        } else {
            p
        }
    };
    let slot = {
        let s = get("slot");
        if s.is_empty() {
            slot
        } else {
            Some(s)
        }
    };
    Ctx {
        engine,
        capture: src.capture_mode.clone(),
        username,
        database,
        tables,
        tables_truncated,
        publication,
        slot,
    }
}

// ── SQL helpers ─────────────────────────────────────────────────────────────────────────

/// SQL string literal ('' escaping). Identifiers from the form only ever go through this.
fn lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn pair_list(tables: &[(String, String)], upper: bool) -> String {
    tables
        .iter()
        .map(|(s, t)| {
            if upper {
                format!(
                    "({}, {})",
                    lit(&s.to_ascii_uppercase()),
                    lit(&t.to_ascii_uppercase())
                )
            } else {
                format!("({}, {})", lit(s), lit(t))
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn p(id: &str, sql: impl Into<String>) -> ProbeSql {
    ProbeSql {
        id: id.into(),
        sql: sql.into(),
    }
}

fn quote_pg(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn quote_ms(ident: &str) -> String {
    format!("[{}]", ident.replace(']', "]]"))
}

// ── probes per engine ───────────────────────────────────────────────────────────────────

/// Probes to run, or None when this engine/capture mode has nothing log-based to check.
pub fn probes(ctx: &Ctx) -> Option<Vec<ProbeSql>> {
    if ctx.capture != CaptureMode::Logical {
        return None;
    }
    Some(match ctx.engine.as_str() {
        "postgres" => pg_probes(ctx),
        "mysql" | "mariadb" => my_probes(ctx),
        "sqlserver" => ms_probes(ctx),
        "oracle" => ora_probes(ctx),
        _ => return None,
    })
}

fn pg_probes(ctx: &Ctx) -> Vec<ProbeSql> {
    let mut v = vec![
        p("info", "SELECT current_setting('server_version_num')::int AS version_num, pg_is_in_recovery() AS in_recovery, current_database() AS db, current_user AS usr, has_database_privilege(current_user, current_database(), 'CREATE') AS can_create"),
        p("settings", "SELECT name, setting, unit FROM pg_settings WHERE name IN ('wal_level','max_replication_slots','max_wal_senders','max_slot_wal_keep_size','wal_sender_timeout','wal_keep_size','server_encoding','rds.logical_replication','cloudsql.logical_decoding','azure.extensions','track_commit_timestamp') ORDER BY name"),
        p("aurora", "SELECT aurora_version() AS v"),
        p("role", "SELECT r.rolsuper AS superuser, r.rolreplication AS replication, EXISTS (SELECT 1 FROM pg_auth_members m JOIN pg_roles g ON g.oid = m.roleid WHERE m.member = r.oid AND g.rolname IN ('rds_replication','rds_superuser','cloudsqlsuperuser','azure_pg_admin')) AS cloud_replication FROM pg_roles r WHERE r.rolname = current_user"),
        p("slots", "SELECT slot_name, plugin, active, database, CASE WHEN pg_is_in_recovery() THEN NULL ELSE pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn)::bigint END AS retained_bytes FROM pg_replication_slots ORDER BY slot_name"),
        p("senders", "SELECT count(*) AS n FROM pg_stat_replication"),
        p("publication", format!("SELECT pubname, puballtables, pubinsert, pubupdate, pubdelete FROM pg_publication WHERE pubname = {}", lit(&ctx.publication))),
        p("pub_tables", format!("SELECT schemaname, tablename FROM pg_publication_tables WHERE pubname = {}", lit(&ctx.publication))),
    ];
    if !ctx.tables.is_empty() {
        v.push(p("tables", format!(
            "SELECT n.nspname AS schema, c.relname AS name, c.relreplident AS replident, \
             EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = c.oid AND i.indisprimary) AS has_pk, \
             has_table_privilege(c.oid, 'SELECT') AS can_select, pg_get_userbyid(c.relowner) AS owner \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind IN ('r','p') AND (n.nspname, c.relname) IN ({})",
            pair_list(&ctx.tables, false)
        )));
    }
    v
}

fn my_probes(ctx: &Ctx) -> Vec<ProbeSql> {
    let mut v = vec![
        p("vars", "SELECT @@version AS version, @@version_comment AS comment, @@log_bin AS log_bin, @@binlog_format AS binlog_format, @@binlog_row_image AS binlog_row_image, @@server_id AS server_id, @@basedir AS basedir"),
        p("gtid", "SELECT @@gtid_mode AS gtid_mode, @@enforce_gtid_consistency AS enforce_gtid_consistency"),
        p("retention", "SELECT @@binlog_expire_logs_seconds AS seconds"),
        p("retention_days", "SELECT @@expire_logs_days AS days"),
        p("aurora", "SELECT @@aurora_version AS v"),
        p("grants", "SELECT PRIVILEGE_TYPE AS privilege FROM information_schema.USER_PRIVILEGES WHERE GRANTEE = CONCAT('''', SUBSTRING_INDEX(CURRENT_USER(), '@', 1), '''@''', SUBSTRING_INDEX(CURRENT_USER(), '@', -1), '''')"),
        p("user", "SELECT CURRENT_USER() AS u"),
    ];
    if !ctx.tables.is_empty() {
        v.push(p("tables", format!(
            "SELECT t.TABLE_SCHEMA AS `schema`, t.TABLE_NAME AS name, t.ENGINE AS engine, \
             EXISTS (SELECT 1 FROM information_schema.TABLE_CONSTRAINTS c WHERE c.TABLE_SCHEMA = t.TABLE_SCHEMA AND c.TABLE_NAME = t.TABLE_NAME AND c.CONSTRAINT_TYPE = 'PRIMARY KEY') AS has_pk \
             FROM information_schema.TABLES t WHERE (t.TABLE_SCHEMA, t.TABLE_NAME) IN ({})",
            pair_list(&ctx.tables, false)
        )));
    }
    v
}

fn ms_probes(ctx: &Ctx) -> Vec<ProbeSql> {
    let mut v = vec![
        p("server", "SELECT CAST(SERVERPROPERTY('ProductVersion') AS nvarchar(64)) AS version, CAST(SERVERPROPERTY('Edition') AS nvarchar(128)) AS edition, CAST(SERVERPROPERTY('EngineEdition') AS int) AS engine_edition, DB_NAME() AS db, SUSER_SNAME() AS login"),
        p("db", "SELECT is_cdc_enabled, recovery_model_desc AS recovery_model, snapshot_isolation_state_desc AS snapshot_isolation FROM sys.databases WHERE name = DB_NAME()"),
        p("agent", "SELECT status_desc AS status FROM sys.dm_server_services WHERE servicename LIKE 'SQL Server Agent%'"),
        p("perms", "SELECT IS_SRVROLEMEMBER('sysadmin') AS sysadmin, IS_ROLEMEMBER('db_owner') AS db_owner, HAS_PERMS_BY_NAME('cdc', 'SCHEMA', 'SELECT') AS cdc_select"),
    ];
    if !ctx.tables.is_empty() {
        v.push(p("tables", format!(
            "SELECT s.name AS [schema], t.name AS name, t.is_tracked_by_cdc AS tracked, OBJECTPROPERTY(t.object_id, 'TableHasPrimaryKey') AS has_pk \
             FROM sys.tables t JOIN sys.schemas s ON s.schema_id = t.schema_id WHERE {}",
            ctx.tables
                .iter()
                .map(|(s, t)| format!("(s.name = {} AND t.name = {})", lit(s), lit(t)))
                .collect::<Vec<_>>()
                .join(" OR ")
        )));
    }
    v
}

fn ora_probes(ctx: &Ctx) -> Vec<ProbeSql> {
    let mut v = vec![
        p("db", "SELECT log_mode, supplemental_log_data_min AS supp_min, supplemental_log_data_all AS supp_all, supplemental_log_data_pk AS supp_pk, force_logging, cdb, open_mode, database_role FROM v$database"),
        p("version", "SELECT version FROM product_component_version WHERE ROWNUM = 1"),
        p("session", "SELECT SYS_CONTEXT('USERENV','CON_NAME') AS con_name, SYS_CONTEXT('USERENV','SESSION_USER') AS session_user FROM dual"),
        p("privs", "SELECT privilege FROM session_privs"),
        p("roles", "SELECT role FROM session_roles"),
        p("redo", "SELECT COUNT(*) AS groups_count, ROUND(MIN(bytes) / 1048576) AS min_mb FROM v$log"),
    ];
    if !ctx.tables.is_empty() {
        v.push(p("table_logs", format!(
            "SELECT owner, table_name, log_group_type FROM all_log_groups WHERE log_group_type = 'ALL COLUMN LOGGING' AND (owner, table_name) IN ({})",
            pair_list(&ctx.tables, true)
        )));
    }
    v
}

// ── reading results ─────────────────────────────────────────────────────────────────────

struct Res<'a>(HashMap<&'a str, &'a ProbeResult>);

impl<'a> Res<'a> {
    fn new(results: &'a [ProbeResult]) -> Self {
        Res(results.iter().map(|r| (r.id.as_str(), r)).collect())
    }
    fn err(&self, id: &str) -> Option<&str> {
        self.0.get(id).and_then(|r| r.error.as_deref())
    }
    fn ok(&self, id: &str) -> bool {
        self.0.get(id).is_some_and(|r| r.error.is_none())
    }
    fn rows(&self, id: &str) -> Vec<HashMap<String, Value>> {
        let Some(r) = self.0.get(id).filter(|r| r.error.is_none()) else {
            return vec![];
        };
        r.rows
            .iter()
            .map(|row| r.columns.iter().cloned().zip(row.iter().cloned()).collect())
            .collect()
    }
    fn first(&self, id: &str) -> Option<HashMap<String, Value>> {
        self.rows(id).into_iter().next()
    }
}

fn s(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(x)) => x.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(other) => other.to_string(),
    }
}

fn b(v: Option<&Value>) -> Option<bool> {
    match v {
        Some(Value::Bool(x)) => Some(*x),
        Some(Value::Number(n)) => n.as_i64().map(|i| i != 0),
        Some(Value::String(x)) => match x.trim().to_ascii_lowercase().as_str() {
            "1" | "t" | "true" | "on" | "yes" => Some(true),
            "0" | "f" | "false" | "off" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn n(v: Option<&Value>) -> Option<i64> {
    match v {
        Some(Value::Number(x)) => x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)),
        Some(Value::String(x)) => x.trim().parse().ok(),
        _ => None,
    }
}

fn first_line(e: &str) -> String {
    e.lines().next().unwrap_or(e).chars().take(220).collect()
}

fn check(id: &'static str, title: &str, status: Status, detail: impl Into<String>) -> Check {
    Check {
        id,
        title: title.into(),
        status,
        detail: detail.into(),
        value: None,
        expected: None,
        fix: None,
    }
}

impl Check {
    fn val(mut self, v: impl Into<String>) -> Self {
        let v = v.into();
        if !v.is_empty() {
            self.value = Some(v);
        }
        self
    }
    fn want(mut self, e: impl Into<String>) -> Self {
        self.expected = Some(e.into());
        self
    }
    fn fix(mut self, f: impl Into<String>) -> Self {
        self.fix = Some(f.into());
        self
    }
}

fn unknown(id: &'static str, title: &str, err: Option<&str>, grant: &str) -> Check {
    let why = err
        .map(first_line)
        .unwrap_or_else(|| "no rows returned".into());
    check(
        id,
        title,
        Status::Unknown,
        format!("The login could not read this ({why}). {grant}"),
    )
}

fn fmt_bytes(n: i64) -> String {
    let n = n.max(0) as f64;
    for (unit, size) in [("TB", 1e12), ("GB", 1e9), ("MB", 1e6), ("kB", 1e3)] {
        if n >= size {
            return format!("{:.1} {unit}", n / size);
        }
    }
    format!("{n} B")
}

// ── evaluate ────────────────────────────────────────────────────────────────────────────

pub fn not_applicable(ctx: &Ctx, server: Option<ServerInfo>) -> Report {
    let note = match ctx.capture {
        CaptureMode::Logical => format!(
            "{} has no log-based capture to check; it uses trigger capture (see the setup guide).",
            ctx.engine
        ),
        CaptureMode::Trigger => "Trigger capture writes changes to a changelog table; it needs no log settings. Use “Install triggers” to check the triggers.".into(),
        CaptureMode::Incremental => "Incremental capture polls the tables with SELECT; it needs no log settings, only read access and an index on the watermark column.".into(),
        CaptureMode::SyncOnce => "Sync once copies the rows a single time; it needs no log settings.".into(),
    };
    finish(
        ctx,
        server,
        "unknown".into(),
        vec![],
        vec![],
        false,
        Some(note),
    )
}

pub fn evaluate(ctx: &Ctx, server: Option<ServerInfo>, results: &[ProbeResult]) -> Report {
    let r = Res::new(results);
    let (platform, checks, params) = match ctx.engine.as_str() {
        "postgres" => eval_pg(ctx, &r),
        "mysql" | "mariadb" => eval_my(ctx, &r),
        "sqlserver" => eval_ms(ctx, &r),
        "oracle" => eval_ora(ctx, &r),
        _ => ("unknown".into(), vec![], vec![]),
    };
    let mut note = None;
    if ctx.tables.is_empty() {
        note = Some(
            "Pick tables to also check them one by one (keys, privileges, per-table CDC settings)."
                .into(),
        );
    } else if ctx.tables_truncated {
        note = Some(format!("Checked the first {MAX_TABLES} selected tables."));
    }
    finish(ctx, server, platform, checks, params, true, note)
}

fn finish(
    ctx: &Ctx,
    server: Option<ServerInfo>,
    platform: String,
    checks: Vec<Check>,
    parameters: Vec<Param>,
    applicable: bool,
    note: Option<String>,
) -> Report {
    let mut summary = Summary::default();
    for c in &checks {
        match c.status {
            Status::Pass => summary.pass += 1,
            Status::Warn => summary.warn += 1,
            Status::Fail => summary.fail += 1,
            Status::Unknown => summary.unknown += 1,
            Status::Info => {}
        }
    }
    let fixes: Vec<String> = checks
        .iter()
        .filter(|c| matches!(c.status, Status::Fail | Status::Warn))
        .filter_map(|c| {
            c.fix.as_ref().map(|f| {
                format!(
                    "-- {} ({})\n{}",
                    c.title,
                    if c.status == Status::Fail {
                        "required"
                    } else {
                        "recommended"
                    },
                    f
                )
            })
        })
        .collect();
    let fix_script = if fixes.is_empty() {
        String::new()
    } else {
        format!(
            "-- CDC setup for {} ({}). Review with your DBA before running; nothing here was run.\n\n{}\n",
            if ctx.database.is_empty() { "this source" } else { ctx.database.as_str() },
            platform_label(&platform),
            fixes.join("\n\n")
        )
    };
    Report {
        engine: ctx.engine.clone(),
        capture_mode: ctx.capture.clone(),
        applicable,
        server,
        platform,
        ready: applicable && summary.fail == 0,
        checks,
        parameters,
        fix_script,
        summary,
        note,
        checked_at: Utc::now(),
    }
}

pub fn platform_label(p: &str) -> &'static str {
    match p {
        "amazon_rds" => "Amazon RDS",
        "amazon_aurora" => "Amazon Aurora",
        "google_cloud_sql" => "Google Cloud SQL",
        "azure" => "Azure",
        "self_managed" => "self-managed",
        _ => "platform unknown",
    }
}

// ── PostgreSQL ──────────────────────────────────────────────────────────────────────────

fn eval_pg(ctx: &Ctx, r: &Res) -> (String, Vec<Check>, Vec<Param>) {
    let mut checks = Vec::new();
    let mut params = Vec::new();
    let settings: HashMap<String, String> = r
        .rows("settings")
        .into_iter()
        .map(|row| {
            let unit = s(row.get("unit"));
            let val = s(row.get("setting"));
            let shown = if unit.is_empty() || val == "-1" {
                val.clone()
            } else {
                format!("{val} {unit}")
            };
            params.push(Param {
                name: s(row.get("name")),
                value: shown,
            });
            (s(row.get("name")), val)
        })
        .collect();
    let platform = if r.ok("aurora") {
        "amazon_aurora"
    } else if settings.contains_key("rds.logical_replication") {
        "amazon_rds"
    } else if settings.contains_key("cloudsql.logical_decoding") {
        "google_cloud_sql"
    } else if settings.contains_key("azure.extensions") {
        "azure"
    } else if r.ok("settings") {
        "self_managed"
    } else {
        "unknown"
    };
    let user = quote_pg(if ctx.username.is_empty() {
        "cdc_user"
    } else {
        &ctx.username
    });

    // version / role of the server
    let info = r.first("info");
    let ver = info.as_ref().and_then(|i| n(i.get("version_num")));
    let in_recovery = info
        .as_ref()
        .and_then(|i| b(i.get("in_recovery")))
        .unwrap_or(false);
    if let Some(v) = ver {
        params.insert(
            0,
            Param {
                name: "server_version_num".into(),
                value: v.to_string(),
            },
        );
        params.insert(
            1,
            Param {
                name: "pg_is_in_recovery".into(),
                value: in_recovery.to_string(),
            },
        );
        // server_version_num: 10+ is major*10000 + minor (170007 = 17.7); 9.x is 90624 = 9.6.24.
        let human = if v >= 100000 {
            format!("{}.{}", v / 10000, v % 10000)
        } else {
            format!("{}.{}.{}", v / 10000, v % 10000 / 100, v % 100)
        };
        if v < 100000 {
            checks.push(
                check(
                    "pg.version",
                    "PostgreSQL version",
                    Status::Fail,
                    format!("PostgreSQL {human} has no pgoutput plugin; CDC needs 10 or later."),
                )
                .val(human)
                .want(">= 10"),
            );
        } else if in_recovery && v < 160000 {
            checks.push(check("pg.version", "Primary server", Status::Fail, "This is a standby. Logical decoding runs on the primary (or on a PostgreSQL 16+ standby). Point the pipeline at the primary.").val(format!("{human} standby")));
        } else {
            checks.push(
                check(
                    "pg.version",
                    "PostgreSQL version",
                    Status::Pass,
                    format!(
                        "PostgreSQL {human}{} supports logical decoding.",
                        if in_recovery { " standby" } else { "" }
                    ),
                )
                .val(human),
            );
        }
    } else {
        checks.push(unknown(
            "pg.version",
            "PostgreSQL version",
            r.err("info"),
            "",
        ));
    }

    // wal_level
    match settings.get("wal_level") {
        Some(w) if w == "logical" => checks.push(
            check(
                "pg.wal_level",
                "wal_level = logical",
                Status::Pass,
                "Write-ahead log carries the row data logical decoding needs.",
            )
            .val(w)
            .want("logical"),
        ),
        Some(w) => {
            let fix = match platform {
                "amazon_rds" | "amazon_aurora" => "-- Amazon RDS / Aurora: in the DB (cluster) parameter group set\n--   rds.logical_replication = 1\n-- then reboot the instance (or writer). wal_level becomes logical.".to_string(),
                "google_cloud_sql" => "-- Cloud SQL: set the database flag cloudsql.logical_decoding = on (restarts the instance).".into(),
                "azure" => "-- Azure Database for PostgreSQL: Server parameters → wal_level = LOGICAL, then restart.".into(),
                _ => "ALTER SYSTEM SET wal_level = logical;\n-- restart PostgreSQL (wal_level only changes at server start)".into(),
            };
            checks.push(check("pg.wal_level", "wal_level = logical", Status::Fail, format!("wal_level is '{w}'. Logical replication slots cannot be created until it is 'logical' (needs a restart).")).val(w).want("logical").fix(fix));
        }
        None => checks.push(unknown(
            "pg.wal_level",
            "wal_level = logical",
            r.err("settings"),
            "",
        )),
    }

    // slots and senders
    let slots = r.rows("slots");
    let max_slots = settings
        .get("max_replication_slots")
        .and_then(|v| v.parse::<i64>().ok());
    let our_slot_exists = ctx
        .slot
        .as_ref()
        .is_some_and(|name| slots.iter().any(|row| s(row.get("slot_name")) == *name));
    if let Some(max) = max_slots {
        let used = slots.len() as i64;
        let free = max - used;
        let status = if free >= 1 || our_slot_exists {
            Status::Pass
        } else {
            Status::Fail
        };
        let mut c = check(
            "pg.slots",
            "Free replication slot",
            status,
            format!(
                "{used} of {max} replication slots in use{}.",
                if our_slot_exists {
                    " (one is this pipeline's)"
                } else {
                    ""
                }
            ),
        )
        .val(format!("{used}/{max}"))
        .want("at least 1 free");
        if status == Status::Fail {
            c = c.fix(if matches!(platform, "amazon_rds" | "amazon_aurora") { "-- Parameter group: raise max_replication_slots (and max_wal_senders), then reboot.\n-- Or drop an unused slot: SELECT pg_drop_replication_slot('<name>');".to_string() } else { "ALTER SYSTEM SET max_replication_slots = 10;\nALTER SYSTEM SET max_wal_senders = 10;\n-- restart PostgreSQL\n-- or drop an unused slot: SELECT pg_drop_replication_slot('<name>');".into() });
        }
        checks.push(c);
    } else if r.err("slots").is_some() || !settings.contains_key("max_replication_slots") {
        checks.push(unknown(
            "pg.slots",
            "Free replication slot",
            r.err("slots"),
            "Grant pg_read_all_settings / pg_monitor to read it.",
        ));
    }
    if let (Some(max), Some(row)) = (
        settings
            .get("max_wal_senders")
            .and_then(|v| v.parse::<i64>().ok()),
        r.first("senders"),
    ) {
        let active = n(row.get("n")).unwrap_or(0);
        let status = if max - active >= 1 {
            Status::Pass
        } else {
            Status::Warn
        };
        checks.push(
            check(
                "pg.senders",
                "Free WAL sender",
                status,
                format!("{active} of {max} WAL senders busy."),
            )
            .val(format!("{active}/{max}"))
            .want("at least 1 free"),
        );
    }
    // WAL kept by inactive slots
    let lagging: Vec<String> = slots
        .iter()
        .filter(|row| b(row.get("active")) == Some(false))
        .filter_map(|row| {
            let bytes = n(row.get("retained_bytes"))?;
            (bytes > 1_000_000_000)
                .then(|| format!("{} ({})", s(row.get("slot_name")), fmt_bytes(bytes)))
        })
        .collect();
    if !lagging.is_empty() {
        checks.push(
            check(
                "pg.slot_lag",
                "Inactive slots holding WAL",
                Status::Warn,
                format!(
                    "Inactive slot(s) keep WAL on disk: {}. An abandoned slot can fill the disk.",
                    lagging.join(", ")
                ),
            )
            .fix("-- If a slot is no longer used:\nSELECT pg_drop_replication_slot('<name>');"),
        );
    }
    if settings
        .get("max_slot_wal_keep_size")
        .is_some_and(|v| v == "-1")
    {
        checks.push(check("pg.wal_keep", "WAL kept for slots is unbounded", Status::Info, "max_slot_wal_keep_size = -1: a stopped pipeline keeps WAL until it resumes. Consider a cap (PostgreSQL 13+) with disk alerts.").val("-1"));
    }

    // login privileges
    match r.first("role") {
        Some(row) => {
            let sup = b(row.get("superuser")).unwrap_or(false);
            let repl = b(row.get("replication")).unwrap_or(false);
            let cloud = b(row.get("cloud_replication")).unwrap_or(false);
            if sup || repl || cloud {
                let how = if sup {
                    "superuser"
                } else if repl {
                    "REPLICATION attribute"
                } else {
                    "cloud replication role"
                };
                checks.push(
                    check(
                        "pg.login",
                        "Login may replicate",
                        Status::Pass,
                        format!("The login has the {how}."),
                    )
                    .val(how),
                );
            } else {
                let fix = match platform {
                    "amazon_rds" | "amazon_aurora" => format!("GRANT rds_replication TO {user};"),
                    "google_cloud_sql" => format!("ALTER USER {user} WITH REPLICATION;"),
                    "azure" => format!("ALTER ROLE {user} WITH REPLICATION;"),
                    _ => format!("ALTER ROLE {user} WITH REPLICATION;"),
                };
                checks.push(check("pg.login", "Login may replicate", Status::Fail, "The login has neither the REPLICATION attribute nor superuser, so it cannot open a replication slot.").fix(fix));
            }
        }
        None => checks.push(unknown(
            "pg.login",
            "Login may replicate",
            r.err("role"),
            "",
        )),
    }

    // publication (the connector never creates it: publication.autocreate.mode = disabled)
    let pub_name = quote_pg(&ctx.publication);
    let wanted: Vec<String> = ctx
        .tables
        .iter()
        .map(|(sc, t)| format!("{}.{}", quote_pg(sc), quote_pg(t)))
        .collect();
    match (r.first("publication"), r.err("publication")) {
        (Some(row), _) => {
            let all = b(row.get("puballtables")).unwrap_or(false);
            let dml_ok = b(row.get("pubupdate")).unwrap_or(true)
                && b(row.get("pubdelete")).unwrap_or(true)
                && b(row.get("pubinsert")).unwrap_or(true);
            let in_pub: HashSet<(String, String)> = r
                .rows("pub_tables")
                .into_iter()
                .map(|row| (s(row.get("schemaname")), s(row.get("tablename"))))
                .collect();
            let missing: Vec<&(String, String)> = if all {
                vec![]
            } else {
                ctx.tables.iter().filter(|t| !in_pub.contains(*t)).collect()
            };
            if !missing.is_empty() {
                let list = missing
                    .iter()
                    .map(|(sc, t)| format!("{}.{}", quote_pg(sc), quote_pg(t)))
                    .collect::<Vec<_>>()
                    .join(", ");
                checks.push(check("pg.publication", "Publication covers the tables", Status::Fail, format!("Publication {} exists but lacks {} of the selected table(s); their changes would not be captured.", ctx.publication, missing.len())).val(format!("{} table(s) missing", missing.len())).fix(format!("ALTER PUBLICATION {pub_name} ADD TABLE {list};")));
            } else if !dml_ok {
                checks.push(check("pg.publication", "Publication publishes all changes", Status::Warn, format!("Publication {} skips some of INSERT/UPDATE/DELETE.", ctx.publication)).fix(format!("ALTER PUBLICATION {pub_name} SET (publish = 'insert, update, delete, truncate');")));
            } else {
                checks.push(
                    check(
                        "pg.publication",
                        "Publication ready",
                        Status::Pass,
                        if all {
                            format!("Publication {} covers all tables.", ctx.publication)
                        } else if ctx.tables.is_empty() {
                            format!(
                                "Publication {} exists ({} table(s)).",
                                ctx.publication,
                                in_pub.len()
                            )
                        } else {
                            format!(
                                "Publication {} includes every selected table.",
                                ctx.publication
                            )
                        },
                    )
                    .val(ctx.publication.clone()),
                );
            }
        }
        (None, None) => {
            let target = if wanted.is_empty() {
                "<schema.table>, …".to_string()
            } else {
                wanted.join(", ")
            };
            checks.push(check("pg.publication", "Publication exists", Status::Fail, format!("Publication {} does not exist. The connector does not create it (autocreate is off), so Start would fail.", ctx.publication)).want(ctx.publication.clone()).fix(format!("-- As the owner of the tables (or a superuser):\nCREATE PUBLICATION {pub_name} FOR TABLE {target};")));
        }
        (None, Some(e)) => {
            checks.push(unknown("pg.publication", "Publication exists", Some(e), ""))
        }
    }

    // tables
    if !ctx.tables.is_empty() {
        let rows = r.rows("tables");
        if rows.is_empty() && r.err("tables").is_some() {
            checks.push(unknown("pg.tables", "Selected tables", r.err("tables"), ""));
        } else {
            let found: HashSet<(String, String)> = rows
                .iter()
                .map(|row| (s(row.get("schema")), s(row.get("name"))))
                .collect();
            let absent: Vec<String> = ctx
                .tables
                .iter()
                .filter(|t| !found.contains(*t))
                .map(|(sc, t)| format!("{sc}.{t}"))
                .collect();
            if !absent.is_empty() {
                checks.push(check(
                    "pg.tables_exist",
                    "Tables exist",
                    Status::Fail,
                    format!(
                        "Not found (or not visible to the login): {}",
                        absent.join(", ")
                    ),
                ));
            }
            let no_select: Vec<String> = rows
                .iter()
                .filter(|row| b(row.get("can_select")) == Some(false))
                .map(|row| {
                    format!(
                        "{}.{}",
                        quote_pg(&s(row.get("schema"))),
                        quote_pg(&s(row.get("name")))
                    )
                })
                .collect();
            if no_select.is_empty() {
                checks.push(check(
                    "pg.select",
                    "Read access to tables",
                    Status::Pass,
                    format!(
                        "The login can SELECT all {} table(s) (needed for the snapshot).",
                        rows.len()
                    ),
                ));
            } else {
                checks.push(
                    check(
                        "pg.select",
                        "Read access to tables",
                        Status::Fail,
                        format!("No SELECT on: {}", no_select.join(", ")),
                    )
                    .fix(format!(
                        "GRANT SELECT ON {} TO {user};",
                        no_select.join(", ")
                    )),
                );
            }
            let no_identity: Vec<String> = rows
                .iter()
                .filter(|row| {
                    !b(row.get("has_pk")).unwrap_or(true)
                        && matches!(s(row.get("replident")).as_str(), "d" | "n")
                })
                .map(|row| {
                    format!(
                        "{}.{}",
                        quote_pg(&s(row.get("schema"))),
                        quote_pg(&s(row.get("name")))
                    )
                })
                .collect();
            if no_identity.is_empty() {
                checks.push(check(
                    "pg.identity",
                    "Row identity for updates/deletes",
                    Status::Pass,
                    "Every table has a primary key or a replica identity.",
                ));
            } else {
                checks.push(check("pg.identity", "Row identity for updates/deletes", Status::Fail, format!("No primary key and no replica identity: {}. Once these tables are in a publication, UPDATE and DELETE on them fail in your application.", no_identity.join(", "))).fix(no_identity.iter().map(|t| format!("ALTER TABLE {t} REPLICA IDENTITY FULL;  -- or add a primary key")).collect::<Vec<_>>().join("\n")));
            }
        }
    }

    // existing slot for this pipeline
    if let Some(name) = &ctx.slot {
        if let Some(row) = slots.iter().find(|row| s(row.get("slot_name")) == *name) {
            let plugin = s(row.get("plugin"));
            let st = if plugin.is_empty() || plugin == "pgoutput" {
                Status::Info
            } else {
                Status::Warn
            };
            checks.push(
                check(
                    "pg.own_slot",
                    "This pipeline's slot",
                    st,
                    format!(
                        "Slot {name} exists ({}, plugin {plugin}). Start resumes from it.",
                        if b(row.get("active")) == Some(true) {
                            "active"
                        } else {
                            "inactive"
                        }
                    ),
                )
                .val(name.clone()),
            );
        }
    }
    (platform.into(), checks, params)
}

// ── MySQL / MariaDB ─────────────────────────────────────────────────────────────────────

fn eval_my(ctx: &Ctx, r: &Res) -> (String, Vec<Check>, Vec<Param>) {
    let mut checks = Vec::new();
    let mut params = Vec::new();
    let maria = ctx.engine == "mariadb";
    let Some(v) = r.first("vars") else {
        checks.push(unknown("my.vars", "Binary log settings", r.err("vars"), ""));
        return ("unknown".into(), checks, params);
    };
    for k in [
        "version",
        "comment",
        "log_bin",
        "binlog_format",
        "binlog_row_image",
        "server_id",
    ] {
        params.push(Param {
            name: if k == "comment" {
                "version_comment".into()
            } else {
                k.into()
            },
            value: s(v.get(k)),
        });
    }
    let basedir = s(v.get("basedir"));
    let platform = if r.ok("aurora") {
        "amazon_aurora"
    } else if basedir.contains("rdsdbbin") {
        "amazon_rds"
    } else if s(v.get("comment")).to_ascii_lowercase().contains("google") {
        "google_cloud_sql"
    } else {
        "self_managed"
    };
    let rds = matches!(platform, "amazon_rds" | "amazon_aurora");
    let user_lit = match r.first("user").map(|u| s(u.get("u"))) {
        Some(u) if u.contains('@') => {
            let (n, h) = u.split_once('@').unwrap();
            format!("'{}'@'{}'", n.replace('\'', "''"), h.replace('\'', "''"))
        }
        _ => format!(
            "'{}'@'%'",
            if ctx.username.is_empty() {
                "cdc_user"
            } else {
                &ctx.username
            }
        ),
    };
    let cnf = |line: &str| {
        if rds {
            format!("-- RDS/Aurora: set {line} in the DB (cluster) parameter group, then reboot.")
        } else {
            format!(
                "-- my.cnf [mysqld]: {line}  (restart)\nSET PERSIST {};",
                line.replace(" = ", " = '").to_string() + "'"
            )
        }
    };

    match b(v.get("log_bin")) {
        Some(true) => checks.push(check("my.log_bin", "Binary log on", Status::Pass, "The server writes a binary log.").val("ON")),
        Some(false) => checks.push(check("my.log_bin", "Binary log on", Status::Fail, "The binary log is off; CDC reads changes from it.").val("OFF").want("ON").fix(if rds { "-- RDS: turn on automated backups (retention ≥ 1 day); that enables the binary log.\n-- Aurora: set binlog_format in the cluster parameter group and reboot the writer.".to_string() } else { "-- my.cnf [mysqld]:\n--   log_bin = mysql-bin\n--   server_id = 1\n-- restart MySQL".into() })),
        None => checks.push(unknown("my.log_bin", "Binary log on", None, "")),
    }
    let fmt = s(v.get("binlog_format")).to_ascii_uppercase();
    checks.push(if fmt == "ROW" {
        check(
            "my.format",
            "binlog_format = ROW",
            Status::Pass,
            "Row events carry the changed values.",
        )
        .val(fmt)
    } else {
        check(
            "my.format",
            "binlog_format = ROW",
            Status::Fail,
            format!("binlog_format is {fmt}; statement/mixed events do not carry row values."),
        )
        .val(fmt)
        .want("ROW")
        .fix(cnf("binlog_format = ROW"))
    });
    let img = s(v.get("binlog_row_image")).to_ascii_uppercase();
    checks.push(if img == "FULL" {
        check(
            "my.row_image",
            "binlog_row_image = FULL",
            Status::Pass,
            "Events carry every column.",
        )
        .val(img)
    } else {
        check(
            "my.row_image",
            "binlog_row_image = FULL",
            Status::Fail,
            format!("binlog_row_image is {img}; updates would miss unchanged columns."),
        )
        .val(img)
        .want("FULL")
        .fix(cnf("binlog_row_image = FULL"))
    });
    match n(v.get("server_id")) {
        Some(0) => checks.push(
            check(
                "my.server_id",
                "server_id set",
                Status::Fail,
                "server_id = 0 disables replication clients.",
            )
            .val("0")
            .fix(cnf("server_id = 1")),
        ),
        Some(id) => checks.push(
            check(
                "my.server_id",
                "server_id set",
                Status::Pass,
                "Replication clients can connect.",
            )
            .val(id.to_string()),
        ),
        None => {}
    }
    // retention
    let secs = r.first("retention").and_then(|x| n(x.get("seconds")));
    let days = r.first("retention_days").and_then(|x| n(x.get("days")));
    let ret = secs
        .filter(|s| *s > 0)
        .or_else(|| days.filter(|d| *d > 0).map(|d| d * 86400));
    match ret {
        Some(s) if s < 86400 => checks.push(check("my.retention", "Binlog retention", Status::Warn, format!("Binary logs are kept {}h. A pipeline stopped longer than that must resnapshot.", s / 3600)).val(format!("{}h", s / 3600)).want(">= 24h").fix(if rds { "CALL mysql.rds_set_configuration('binlog retention hours', 72);".to_string() } else { "SET PERSIST binlog_expire_logs_seconds = 259200;  -- 3 days".into() })),
        Some(s) => checks.push(check("my.retention", "Binlog retention", Status::Pass, format!("Binary logs are kept {} day(s).", s / 86400)).val(format!("{}d", s / 86400))),
        None if rds => checks.push(check("my.retention", "Binlog retention", Status::Warn, "On RDS the binlog is purged quickly unless 'binlog retention hours' is set.").fix("CALL mysql.rds_set_configuration('binlog retention hours', 72);")),
        None => {}
    }
    if let Some(g) = r.first("gtid") {
        let mode = s(g.get("gtid_mode"));
        params.push(Param {
            name: "gtid_mode".into(),
            value: mode.clone(),
        });
        checks.push(
            check(
                "my.gtid",
                "GTID mode",
                Status::Info,
                if mode.eq_ignore_ascii_case("ON") {
                    "GTIDs on: the pipeline can fail over between replicas."
                } else {
                    "GTIDs off: fine for a single server; turn on for replica failover."
                },
            )
            .val(mode),
        );
    }
    // privileges
    let privs: HashSet<String> = r
        .rows("grants")
        .into_iter()
        .map(|row| s(row.get("privilege")).to_ascii_uppercase())
        .collect();
    if r.ok("grants") {
        let mut needed = vec![
            "SELECT",
            "RELOAD",
            "SHOW DATABASES",
            "REPLICATION SLAVE",
            "REPLICATION CLIENT",
        ];
        if maria {
            needed.retain(|p| *p != "REPLICATION CLIENT");
        }
        let has = |p: &str| {
            privs.contains("ALL PRIVILEGES")
                || privs.contains(p)
                || (maria && p == "REPLICATION CLIENT" && privs.contains("BINLOG MONITOR"))
        };
        let missing: Vec<&str> = needed.into_iter().filter(|p| !has(p)).collect();
        if missing.is_empty() {
            checks.push(check(
                "my.grants",
                "Replication privileges",
                Status::Pass,
                "The login has SELECT, RELOAD, SHOW DATABASES and replication privileges.",
            ));
        } else {
            let mut list = missing.clone();
            if maria && !has("BINLOG MONITOR") {
                list.push("BINLOG MONITOR");
            }
            checks.push(check("my.grants", "Replication privileges", Status::Warn, format!("Global grants do not show: {}. (Privileges given through roles are not visible here — confirm with SHOW GRANTS.)", missing.join(", "))).fix(format!("GRANT {} ON *.* TO {user_lit};", list.join(", "))));
        }
    } else {
        checks.push(unknown(
            "my.grants",
            "Replication privileges",
            r.err("grants"),
            "Run SHOW GRANTS as the login to confirm.",
        ));
    }
    // tables
    if !ctx.tables.is_empty() {
        let rows = r.rows("tables");
        let found: HashSet<(String, String)> = rows
            .iter()
            .map(|row| (s(row.get("schema")), s(row.get("name"))))
            .collect();
        let absent: Vec<String> = ctx
            .tables
            .iter()
            .filter(|t| !found.contains(*t))
            .map(|(sc, t)| format!("{sc}.{t}"))
            .collect();
        if !absent.is_empty() && r.ok("tables") {
            checks.push(check(
                "my.tables_exist",
                "Tables exist",
                Status::Fail,
                format!(
                    "Not found (or not visible to the login): {}",
                    absent.join(", ")
                ),
            ));
        }
        let non_innodb: Vec<String> = rows
            .iter()
            .filter(|row| !s(row.get("engine")).eq_ignore_ascii_case("InnoDB"))
            .map(|row| {
                format!(
                    "{}.{} ({})",
                    s(row.get("schema")),
                    s(row.get("name")),
                    s(row.get("engine"))
                )
            })
            .collect();
        if !non_innodb.is_empty() {
            checks.push(check(
                "my.engine",
                "Transactional tables",
                Status::Warn,
                format!(
                    "Not InnoDB: {}. The snapshot is not consistent for them.",
                    non_innodb.join(", ")
                ),
            ));
        }
        let no_pk: Vec<String> = rows
            .iter()
            .filter(|row| b(row.get("has_pk")) == Some(false))
            .map(|row| format!("{}.{}", s(row.get("schema")), s(row.get("name"))))
            .collect();
        if !no_pk.is_empty() {
            checks.push(check("my.pk", "Primary keys", Status::Warn, format!("No primary key: {}. Events have no key, so the target cannot upsert or delete them.", no_pk.join(", "))));
        } else if !rows.is_empty() {
            checks.push(check(
                "my.pk",
                "Primary keys",
                Status::Pass,
                "Every selected table has a primary key.",
            ));
        }
    }
    (platform.into(), checks, params)
}

// ── SQL Server ──────────────────────────────────────────────────────────────────────────

fn eval_ms(ctx: &Ctx, r: &Res) -> (String, Vec<Check>, Vec<Param>) {
    let mut checks = Vec::new();
    let mut params = Vec::new();
    let srv = r.first("server");
    let engine_edition = srv.as_ref().and_then(|x| n(x.get("engine_edition")));
    if let Some(x) = &srv {
        for (k, name) in [
            ("version", "ProductVersion"),
            ("edition", "Edition"),
            ("engine_edition", "EngineEdition"),
        ] {
            params.push(Param {
                name: name.into(),
                value: s(x.get(k)),
            });
        }
    }
    let platform = match engine_edition {
        Some(5) | Some(8) | Some(9) => "azure",
        Some(_) => "self_managed",
        None => "unknown",
    };
    let db = if ctx.database.is_empty() {
        srv.as_ref().map(|x| s(x.get("db"))).unwrap_or_default()
    } else {
        ctx.database.clone()
    };
    match engine_edition {
        Some(4) => checks.push(
            check(
                "ms.edition",
                "Edition supports CDC",
                Status::Fail,
                "SQL Server Express has no CDC (no SQL Server Agent).",
            )
            .val(
                srv.as_ref()
                    .map(|x| s(x.get("edition")))
                    .unwrap_or_default(),
            ),
        ),
        Some(e) => {
            let major = srv
                .as_ref()
                .map(|x| s(x.get("version")))
                .and_then(|v| v.split('.').next().and_then(|m| m.parse::<i64>().ok()))
                .unwrap_or(0);
            let ed = srv
                .as_ref()
                .map(|x| s(x.get("edition")))
                .unwrap_or_default();
            if e == 2 && major > 0 && major < 13 {
                checks.push(
                    check(
                        "ms.edition",
                        "Edition supports CDC",
                        Status::Fail,
                        "CDC on Standard edition needs SQL Server 2016 SP1 or later.",
                    )
                    .val(ed),
                );
            } else {
                checks.push(
                    check(
                        "ms.edition",
                        "Edition supports CDC",
                        Status::Pass,
                        format!("{ed} supports change data capture."),
                    )
                    .val(ed),
                );
            }
        }
        None => checks.push(unknown(
            "ms.edition",
            "Edition supports CDC",
            r.err("server"),
            "",
        )),
    }
    match r.first("db") {
        Some(row) => {
            params.push(Param {
                name: "recovery_model".into(),
                value: s(row.get("recovery_model")),
            });
            params.push(Param {
                name: "snapshot_isolation".into(),
                value: s(row.get("snapshot_isolation")),
            });
            if b(row.get("is_cdc_enabled")) == Some(true) {
                checks.push(
                    check(
                        "ms.db_cdc",
                        "CDC enabled on the database",
                        Status::Pass,
                        format!("Change data capture is on for {db}."),
                    )
                    .val("on"),
                );
            } else {
                let fix = if platform == "azure" {
                    format!(
                        "USE {};\nEXEC sys.sp_cdc_enable_db;  -- as db_owner",
                        quote_ms(&db)
                    )
                } else {
                    format!(
                        "USE {};\nEXEC sys.sp_cdc_enable_db;  -- as sysadmin",
                        quote_ms(&db)
                    )
                };
                checks.push(
                    check(
                        "ms.db_cdc",
                        "CDC enabled on the database",
                        Status::Fail,
                        format!("Change data capture is off for {db}."),
                    )
                    .val("off")
                    .want("on")
                    .fix(fix),
                );
            }
        }
        None => checks.push(unknown(
            "ms.db_cdc",
            "CDC enabled on the database",
            r.err("db"),
            "",
        )),
    }
    // capture job runs under SQL Server Agent (Azure SQL Database has its own scheduler)
    if engine_edition == Some(5) {
        checks.push(check(
            "ms.agent",
            "Capture job scheduler",
            Status::Info,
            "Azure SQL Database runs the CDC capture itself (no SQL Server Agent).",
        ));
    } else {
        match r.first("agent") {
            Some(row) => {
                let st = s(row.get("status"));
                if st.eq_ignore_ascii_case("Running") {
                    checks.push(check("ms.agent", "SQL Server Agent running", Status::Pass, "The CDC capture and cleanup jobs can run.").val(st));
                } else {
                    checks.push(check("ms.agent", "SQL Server Agent running", Status::Fail, format!("SQL Server Agent is {st}; the capture job does not run, so no changes reach the change tables.")).val(st).want("Running").fix("-- Start SQL Server Agent and set it to start automatically\n-- (SQL Server Configuration Manager, or: net start SQLSERVERAGENT)"));
                }
            }
            None => checks.push(unknown("ms.agent", "SQL Server Agent running", r.err("agent").or(Some("no service row")), "Needs VIEW SERVER STATE to read sys.dm_server_services; or check in SSMS that SQL Server Agent runs.")),
        }
    }
    if let Some(row) = r.first("perms") {
        let sysadmin = b(row.get("sysadmin")) == Some(true);
        let owner = b(row.get("db_owner")) == Some(true);
        let cdc_sel = b(row.get("cdc_select"));
        if sysadmin || owner || cdc_sel == Some(true) {
            checks.push(check(
                "ms.perms",
                "Read the change tables",
                Status::Pass,
                if sysadmin {
                    "The login is sysadmin."
                } else if owner {
                    "The login is db_owner."
                } else {
                    "The login can SELECT the cdc schema."
                },
            ));
        } else {
            let login = if ctx.username.is_empty() {
                "cdc_user".to_string()
            } else {
                ctx.username.clone()
            };
            checks.push(check("ms.perms", "Read the change tables", Status::Warn, "The login is neither db_owner nor allowed to SELECT the cdc schema; it may not read the change tables (unless a gating role grants it).").fix(format!("USE {};\nGRANT SELECT ON SCHEMA::cdc TO {};\nGRANT SELECT ON SCHEMA::dbo TO {};  -- each captured schema", quote_ms(&db), quote_ms(&login), quote_ms(&login))));
        }
    }
    if !ctx.tables.is_empty() {
        let rows = r.rows("tables");
        if rows.is_empty() && r.err("tables").is_some() {
            checks.push(unknown(
                "ms.tables",
                "Tables tracked by CDC",
                r.err("tables"),
                "",
            ));
        } else {
            let found: HashSet<(String, String)> = rows
                .iter()
                .map(|row| {
                    (
                        s(row.get("schema")).to_lowercase(),
                        s(row.get("name")).to_lowercase(),
                    )
                })
                .collect();
            let absent: Vec<String> = ctx
                .tables
                .iter()
                .filter(|(a, t)| !found.contains(&(a.to_lowercase(), t.to_lowercase())))
                .map(|(a, t)| format!("{a}.{t}"))
                .collect();
            if !absent.is_empty() {
                checks.push(check(
                    "ms.tables_exist",
                    "Tables exist",
                    Status::Fail,
                    format!(
                        "Not found (or not visible to the login): {}",
                        absent.join(", ")
                    ),
                ));
            }
            let untracked: Vec<(String, String)> = rows
                .iter()
                .filter(|row| b(row.get("tracked")) == Some(false))
                .map(|row| (s(row.get("schema")), s(row.get("name"))))
                .collect();
            if untracked.is_empty() {
                checks.push(check(
                    "ms.tables",
                    "Tables tracked by CDC",
                    Status::Pass,
                    format!("All {} table(s) have a capture instance.", rows.len()),
                ));
            } else {
                let fix = format!("USE {};\n{}", quote_ms(&db), untracked.iter().map(|(a, t)| format!("EXEC sys.sp_cdc_enable_table @source_schema = N{}, @source_name = N{}, @role_name = NULL, @supports_net_changes = 0;", lit(a), lit(t))).collect::<Vec<_>>().join("\n"));
                checks.push(
                    check(
                        "ms.tables",
                        "Tables tracked by CDC",
                        Status::Fail,
                        format!(
                            "No capture instance: {}",
                            untracked
                                .iter()
                                .map(|(a, t)| format!("{a}.{t}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    )
                    .fix(fix),
                );
            }
            let no_pk: Vec<String> = rows
                .iter()
                .filter(|row| b(row.get("has_pk")) == Some(false))
                .map(|row| format!("{}.{}", s(row.get("schema")), s(row.get("name"))))
                .collect();
            if !no_pk.is_empty() {
                checks.push(check("ms.pk", "Primary keys", Status::Warn, format!("No primary key: {}. Events have no key, so the target cannot upsert or delete them.", no_pk.join(", "))));
            }
        }
    }
    (platform.into(), checks, params)
}

// ── Oracle ──────────────────────────────────────────────────────────────────────────────

fn eval_ora(ctx: &Ctx, r: &Res) -> (String, Vec<Check>, Vec<Param>) {
    let mut checks = Vec::new();
    let mut params = Vec::new();
    let user = if ctx.username.is_empty() {
        "C##DBZUSER".to_string()
    } else {
        ctx.username.to_ascii_uppercase()
    };
    const DICT_GRANT: &str =
        "Grant SELECT_CATALOG_ROLE (or SELECT ON V_$DATABASE) to let the check read it.";
    if let Some(v) = r.first("version") {
        params.push(Param {
            name: "version".into(),
            value: s(v.get("version")),
        });
    }
    let sess = r.first("session");
    if let Some(x) = &sess {
        params.push(Param {
            name: "con_name".into(),
            value: s(x.get("con_name")),
        });
    }
    let db = r.first("db");
    let cdb = db.as_ref().map(|x| s(x.get("cdb"))).unwrap_or_default() == "YES";
    match &db {
        Some(row) => {
            for k in [
                "log_mode",
                "supp_min",
                "supp_all",
                "force_logging",
                "cdb",
                "open_mode",
                "database_role",
            ] {
                params.push(Param {
                    name: k.into(),
                    value: s(row.get(k)),
                });
            }
            let mode = s(row.get("log_mode"));
            checks.push(if mode == "ARCHIVELOG" {
                check("ora.archivelog", "ARCHIVELOG mode", Status::Pass, "Redo is archived, so LogMiner can read past changes.").val(mode)
            } else {
                check("ora.archivelog", "ARCHIVELOG mode", Status::Fail, format!("The database runs in {mode}; LogMiner needs ARCHIVELOG (requires a short restart).")).val(mode).want("ARCHIVELOG").fix("-- As SYSDBA (brief outage):\nSHUTDOWN IMMEDIATE;\nSTARTUP MOUNT;\nALTER DATABASE ARCHIVELOG;\nALTER DATABASE OPEN;\n-- Amazon RDS: enable automated backups instead.")
            });
            let min = s(row.get("supp_min"));
            let all = s(row.get("supp_all")) == "YES";
            checks.push(if min == "YES" || min == "IMPLICIT" {
                check("ora.supp_min", "Minimal supplemental logging", Status::Pass, "Database-level supplemental logging is on.").val(min)
            } else {
                check("ora.supp_min", "Minimal supplemental logging", Status::Fail, "Supplemental logging is off; LogMiner cannot reconstruct row changes.").val(min).want("YES").fix("ALTER DATABASE ADD SUPPLEMENTAL LOG DATA;\n-- Amazon RDS: EXEC rdsadmin.rdsadmin_util.alter_supplemental_logging(p_action => 'ADD');")
            });
            let role = s(row.get("database_role"));
            if !role.is_empty() && role != "PRIMARY" {
                checks.push(check("ora.role", "Primary database", Status::Warn, format!("The database role is {role}. Capture from the primary unless you run LogMiner on an Active Data Guard standby on purpose.")).val(role));
            }
            // per-table ALL COLUMNS logging (not needed when the whole database logs all columns)
            if !ctx.tables.is_empty() {
                if all {
                    checks.push(check(
                        "ora.supp_tables",
                        "All-column logging for tables",
                        Status::Pass,
                        "The database logs all columns for every table.",
                    ));
                } else if r.ok("table_logs") {
                    let have: HashSet<(String, String)> = r
                        .rows("table_logs")
                        .into_iter()
                        .map(|row| (s(row.get("owner")), s(row.get("table_name"))))
                        .collect();
                    let missing: Vec<(String, String)> = ctx
                        .tables
                        .iter()
                        .map(|(o, t)| (o.to_ascii_uppercase(), t.to_ascii_uppercase()))
                        .filter(|k| !have.contains(k))
                        .collect();
                    if missing.is_empty() {
                        checks.push(check(
                            "ora.supp_tables",
                            "All-column logging for tables",
                            Status::Pass,
                            "Every selected table logs all columns.",
                        ));
                    } else {
                        checks.push(check("ora.supp_tables", "All-column logging for tables", Status::Fail, format!("{} table(s) without ALL COLUMNS supplemental logging; updates would miss unchanged columns.", missing.len())).fix(missing.iter().map(|(o, t)| format!("ALTER TABLE {o}.{t} ADD SUPPLEMENTAL LOG DATA (ALL) COLUMNS;")).collect::<Vec<_>>().join("\n")));
                    }
                } else {
                    checks.push(unknown(
                        "ora.supp_tables",
                        "All-column logging for tables",
                        r.err("table_logs"),
                        "",
                    ));
                }
            }
        }
        None => {
            checks.push(unknown(
                "ora.archivelog",
                "ARCHIVELOG mode",
                r.err("db"),
                DICT_GRANT,
            ));
            checks.push(unknown(
                "ora.supp_min",
                "Minimal supplemental logging",
                r.err("db"),
                DICT_GRANT,
            ));
        }
    }
    // privileges of the session
    if r.ok("privs") {
        let privs: HashSet<String> = r
            .rows("privs")
            .into_iter()
            .map(|row| s(row.get("privilege")))
            .collect();
        let roles: HashSet<String> = r
            .rows("roles")
            .into_iter()
            .map(|row| s(row.get("role")))
            .collect();
        let dba = roles.contains("DBA");
        let has = |p: &str| dba || privs.contains(p);
        let mut missing = Vec::new();
        for p in [
            "CREATE SESSION",
            "SELECT ANY TRANSACTION",
            "FLASHBACK ANY TABLE",
            "CREATE TABLE",
        ] {
            if !has(p) {
                missing.push(p.to_string());
            }
        }
        if !(has("LOGMINING") || roles.contains("EXECUTE_CATALOG_ROLE")) {
            missing.push("LOGMINING".into());
        }
        if !(has("SELECT ANY DICTIONARY") || roles.contains("SELECT_CATALOG_ROLE")) {
            missing.push("SELECT_CATALOG_ROLE".into());
        }
        if cdb && !has("SET CONTAINER") {
            missing.push("SET CONTAINER".into());
        }
        if missing.is_empty() {
            checks.push(check(
                "ora.privs",
                "LogMiner privileges",
                Status::Pass,
                if dba {
                    "The login has the DBA role."
                } else {
                    "The login has the LogMiner, dictionary and snapshot privileges."
                },
            ));
        } else {
            let container = if cdb { " CONTAINER=ALL" } else { "" };
            checks.push(
                check(
                    "ora.privs",
                    "LogMiner privileges",
                    Status::Fail,
                    format!("Missing: {}", missing.join(", ")),
                )
                .fix(
                    missing
                        .iter()
                        .map(|p| format!("GRANT {p} TO {user}{container};"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            );
        }
    } else {
        checks.push(unknown(
            "ora.privs",
            "LogMiner privileges",
            r.err("privs"),
            "",
        ));
    }
    if let Some(row) = r.first("redo") {
        let mb = n(row.get("min_mb")).unwrap_or(0);
        params.push(Param {
            name: "redo_groups".into(),
            value: s(row.get("groups_count")),
        });
        params.push(Param {
            name: "redo_min_mb".into(),
            value: mb.to_string(),
        });
        if mb > 0 && mb < 500 {
            checks.push(check("ora.redo", "Redo log size", Status::Warn, format!("Smallest redo log is {mb} MB. Small logs switch often and make LogMiner fall behind under load; 1–2 GB is typical.")).val(format!("{mb} MB")));
        }
    }
    (
        if params.iter().any(|p| p.value.contains("RDS")) {
            "amazon_rds"
        } else {
            "self_managed"
        }
        .into(),
        checks,
        params,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx(engine: &str, tables: &[(&str, &str)]) -> Ctx {
        Ctx {
            engine: engine.into(),
            capture: CaptureMode::Logical,
            username: "cdc".into(),
            database: "app".into(),
            tables: tables
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
            tables_truncated: false,
            publication: "dbz_publication".into(),
            slot: Some("cdc_orders".into()),
        }
    }

    fn ok(id: &str, cols: &[&str], rows: Vec<Vec<Value>>) -> ProbeResult {
        ProbeResult {
            id: id.into(),
            columns: cols.iter().map(|c| c.to_string()).collect(),
            rows,
            error: None,
            sqlstate: None,
        }
    }

    fn err(id: &str, e: &str) -> ProbeResult {
        ProbeResult {
            id: id.into(),
            error: Some(e.into()),
            ..Default::default()
        }
    }

    fn status(rep: &Report, id: &str) -> Status {
        rep.checks
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| {
                panic!(
                    "no check {id}: {:?}",
                    rep.checks.iter().map(|c| c.id).collect::<Vec<_>>()
                )
            })
            .status
    }

    fn pg_results(
        wal: &str,
        publication: bool,
        pub_tables: Vec<Vec<Value>>,
        rds: bool,
    ) -> Vec<ProbeResult> {
        let mut settings = vec![
            vec![json!("wal_level"), json!(wal), json!(null)],
            vec![json!("max_replication_slots"), json!("10"), json!(null)],
            vec![json!("max_wal_senders"), json!("10"), json!(null)],
        ];
        if rds {
            settings.push(vec![
                json!("rds.logical_replication"),
                json!("off"),
                json!(null),
            ]);
        }
        vec![
            ok(
                "info",
                &["version_num", "in_recovery", "db", "usr", "can_create"],
                vec![vec![
                    json!(160004),
                    json!(false),
                    json!("app"),
                    json!("cdc"),
                    json!(true),
                ]],
            ),
            ok("settings", &["name", "setting", "unit"], settings),
            err("aurora", "function aurora_version() does not exist"),
            ok(
                "role",
                &["superuser", "replication", "cloud_replication"],
                vec![vec![json!(false), json!(true), json!(false)]],
            ),
            ok(
                "slots",
                &[
                    "slot_name",
                    "plugin",
                    "active",
                    "database",
                    "retained_bytes",
                ],
                vec![vec![
                    json!("old"),
                    json!("pgoutput"),
                    json!(false),
                    json!("app"),
                    json!(5_000_000_000i64),
                ]],
            ),
            ok("senders", &["n"], vec![vec![json!(0)]]),
            ok(
                "publication",
                &[
                    "pubname",
                    "puballtables",
                    "pubinsert",
                    "pubupdate",
                    "pubdelete",
                ],
                if publication {
                    vec![vec![
                        json!("dbz_publication"),
                        json!(false),
                        json!(true),
                        json!(true),
                        json!(true),
                    ]]
                } else {
                    vec![]
                },
            ),
            ok("pub_tables", &["schemaname", "tablename"], pub_tables),
            ok(
                "tables",
                &[
                    "schema",
                    "name",
                    "replident",
                    "has_pk",
                    "can_select",
                    "owner",
                ],
                vec![
                    vec![
                        json!("public"),
                        json!("orders"),
                        json!("d"),
                        json!(true),
                        json!(true),
                        json!("app"),
                    ],
                    vec![
                        json!("public"),
                        json!("audit"),
                        json!("d"),
                        json!(false),
                        json!(false),
                        json!("app"),
                    ],
                ],
            ),
        ]
    }

    #[test]
    fn postgres_ready_and_not_ready() {
        let c = ctx("postgres", &[("public", "orders"), ("public", "audit")]);
        let rep = evaluate(
            &c,
            None,
            &pg_results(
                "logical",
                true,
                vec![
                    vec![json!("public"), json!("orders")],
                    vec![json!("public"), json!("audit")],
                ],
                false,
            ),
        );
        assert_eq!(rep.platform, "self_managed");
        assert_eq!(
            rep.checks
                .iter()
                .find(|c| c.id == "pg.version")
                .unwrap()
                .value
                .as_deref(),
            Some("16.4")
        );
        assert_eq!(status(&rep, "pg.wal_level"), Status::Pass);
        assert_eq!(status(&rep, "pg.publication"), Status::Pass);
        assert_eq!(
            status(&rep, "pg.select"),
            Status::Fail,
            "audit has no SELECT"
        );
        assert_eq!(
            status(&rep, "pg.identity"),
            Status::Fail,
            "audit: no pk, default identity"
        );
        assert_eq!(status(&rep, "pg.slot_lag"), Status::Warn);
        assert!(!rep.ready);
        assert!(rep.fix_script.contains("REPLICA IDENTITY FULL"));
        assert!(rep
            .fix_script
            .contains("GRANT SELECT ON \"public\".\"audit\" TO \"cdc\";"));
    }

    #[test]
    fn postgres_rds_fix_uses_parameter_group_and_missing_publication_fails() {
        let c = ctx("postgres", &[("public", "orders")]);
        let rep = evaluate(&c, None, &pg_results("replica", false, vec![], true));
        assert_eq!(rep.platform, "amazon_rds");
        let wal = rep.checks.iter().find(|c| c.id == "pg.wal_level").unwrap();
        assert_eq!(wal.status, Status::Fail);
        assert!(wal
            .fix
            .as_ref()
            .unwrap()
            .contains("rds.logical_replication = 1"));
        let p = rep
            .checks
            .iter()
            .find(|c| c.id == "pg.publication")
            .unwrap();
        assert_eq!(p.status, Status::Fail);
        assert!(p
            .fix
            .as_ref()
            .unwrap()
            .contains("CREATE PUBLICATION \"dbz_publication\" FOR TABLE \"public\".\"orders\";"));
    }

    #[test]
    fn postgres_publication_missing_tables_and_unreadable_settings() {
        let c = ctx("postgres", &[("public", "orders"), ("sales", "x")]);
        let rep = evaluate(
            &c,
            None,
            &pg_results(
                "logical",
                true,
                vec![vec![json!("public"), json!("orders")]],
                false,
            ),
        );
        let p = rep
            .checks
            .iter()
            .find(|c| c.id == "pg.publication")
            .unwrap();
        assert_eq!(p.status, Status::Fail);
        assert!(p
            .fix
            .as_ref()
            .unwrap()
            .contains("ADD TABLE \"sales\".\"x\""));

        let mut res = pg_results("logical", true, vec![], false);
        res[1] = err("settings", "permission denied for view pg_settings");
        let rep = evaluate(&c, None, &res);
        assert_eq!(status(&rep, "pg.wal_level"), Status::Unknown);
        assert_eq!(rep.platform, "unknown");
    }

    #[test]
    fn mysql_rules() {
        let c = ctx("mysql", &[("app", "orders")]);
        let res = vec![
            ok(
                "vars",
                &[
                    "version",
                    "comment",
                    "log_bin",
                    "binlog_format",
                    "binlog_row_image",
                    "server_id",
                    "basedir",
                ],
                vec![vec![
                    json!("8.0.36"),
                    json!("MySQL Community"),
                    json!(1),
                    json!("MIXED"),
                    json!("FULL"),
                    json!(1),
                    json!("/rdsdbbin/mysql/"),
                ]],
            ),
            ok(
                "gtid",
                &["gtid_mode", "enforce_gtid_consistency"],
                vec![vec![json!("ON"), json!("ON")]],
            ),
            ok("retention", &["seconds"], vec![vec![json!(3600)]]),
            err("retention_days", "unknown"),
            err("aurora", "Unknown system variable"),
            ok(
                "grants",
                &["privilege"],
                vec![vec![json!("SELECT")], vec![json!("REPLICATION SLAVE")]],
            ),
            ok("user", &["u"], vec![vec![json!("cdc@%")]]),
            ok(
                "tables",
                &["schema", "name", "engine", "has_pk"],
                vec![vec![
                    json!("app"),
                    json!("orders"),
                    json!("MyISAM"),
                    json!(0),
                ]],
            ),
        ];
        let rep = evaluate(&c, None, &res);
        assert_eq!(rep.platform, "amazon_rds");
        assert_eq!(status(&rep, "my.log_bin"), Status::Pass);
        assert_eq!(status(&rep, "my.format"), Status::Fail);
        assert!(rep
            .checks
            .iter()
            .find(|c| c.id == "my.format")
            .unwrap()
            .fix
            .as_ref()
            .unwrap()
            .contains("parameter group"));
        assert_eq!(status(&rep, "my.retention"), Status::Warn);
        assert_eq!(status(&rep, "my.grants"), Status::Warn);
        assert!(rep
            .fix_script
            .contains("GRANT RELOAD, SHOW DATABASES, REPLICATION CLIENT ON *.* TO 'cdc'@'%';"));
        assert_eq!(status(&rep, "my.engine"), Status::Warn);
        assert_eq!(status(&rep, "my.pk"), Status::Warn);
    }

    #[test]
    fn sqlserver_rules() {
        let c = ctx("sqlserver", &[("dbo", "Orders")]);
        let res = vec![
            ok(
                "server",
                &["version", "edition", "engine_edition", "db", "login"],
                vec![vec![
                    json!("16.0.4135.4"),
                    json!("Developer Edition (64-bit)"),
                    json!(3),
                    json!("app"),
                    json!("cdc"),
                ]],
            ),
            ok(
                "db",
                &["is_cdc_enabled", "recovery_model", "snapshot_isolation"],
                vec![vec![json!(false), json!("FULL"), json!("OFF")]],
            ),
            err("agent", "VIEW SERVER STATE permission was denied"),
            ok(
                "perms",
                &["sysadmin", "db_owner", "cdc_select"],
                vec![vec![json!(0), json!(1), json!(null)]],
            ),
            ok(
                "tables",
                &["schema", "name", "tracked", "has_pk"],
                vec![vec![json!("dbo"), json!("Orders"), json!(false), json!(1)]],
            ),
        ];
        let rep = evaluate(&c, None, &res);
        assert_eq!(status(&rep, "ms.edition"), Status::Pass);
        assert_eq!(status(&rep, "ms.db_cdc"), Status::Fail);
        assert_eq!(status(&rep, "ms.agent"), Status::Unknown);
        assert_eq!(status(&rep, "ms.perms"), Status::Pass);
        assert_eq!(status(&rep, "ms.tables"), Status::Fail);
        assert!(rep
            .fix_script
            .contains("sp_cdc_enable_table @source_schema = N'dbo', @source_name = N'Orders'"));
    }

    #[test]
    fn oracle_rules_and_unreadable_dictionary() {
        let c = ctx("oracle", &[("app", "orders")]);
        let res = vec![
            ok(
                "db",
                &[
                    "log_mode",
                    "supp_min",
                    "supp_all",
                    "supp_pk",
                    "force_logging",
                    "cdb",
                    "open_mode",
                    "database_role",
                ],
                vec![vec![
                    json!("NOARCHIVELOG"),
                    json!("NO"),
                    json!("NO"),
                    json!("NO"),
                    json!("NO"),
                    json!("YES"),
                    json!("READ WRITE"),
                    json!("PRIMARY"),
                ]],
            ),
            ok("version", &["version"], vec![vec![json!("19.0.0.0.0")]]),
            ok("privs", &["privilege"], vec![vec![json!("CREATE SESSION")]]),
            ok("roles", &["role"], vec![]),
            ok(
                "table_logs",
                &["owner", "table_name", "log_group_type"],
                vec![],
            ),
        ];
        let rep = evaluate(&c, None, &res);
        assert_eq!(status(&rep, "ora.archivelog"), Status::Fail);
        assert_eq!(status(&rep, "ora.supp_min"), Status::Fail);
        assert_eq!(status(&rep, "ora.supp_tables"), Status::Fail);
        assert!(rep
            .fix_script
            .contains("ALTER TABLE APP.ORDERS ADD SUPPLEMENTAL LOG DATA (ALL) COLUMNS;"));
        assert!(rep
            .fix_script
            .contains("GRANT SET CONTAINER TO CDC CONTAINER=ALL;"));

        let res = vec![
            err("db", "ORA-00942: table or view does not exist"),
            ok("privs", &["privilege"], vec![]),
            ok("roles", &["role"], vec![vec![json!("DBA")]]),
        ];
        let rep = evaluate(&c, None, &res);
        assert_eq!(status(&rep, "ora.archivelog"), Status::Unknown);
        assert!(rep
            .checks
            .iter()
            .find(|c| c.id == "ora.archivelog")
            .unwrap()
            .detail
            .contains("SELECT_CATALOG_ROLE"));
        assert_eq!(status(&rep, "ora.privs"), Status::Pass);
        assert!(rep.ready, "unknown does not block");
    }

    #[test]
    fn probes_escape_literals_and_skip_non_logical() {
        let mut c = ctx("postgres", &[("public", "o'brien")]);
        c.publication = "pub'x".into();
        let sql = probes(&c)
            .unwrap()
            .into_iter()
            .map(|p| p.sql)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(sql.contains("'o''brien'"));
        assert!(sql.contains("'pub''x'"));
        c.capture = CaptureMode::Incremental;
        assert!(probes(&c).is_none());
        assert!(probes(&ctx("db2", &[])).is_none());
    }

    #[test]
    fn ctx_parses_tables_and_defaults() {
        let src: SourceConfig = serde_json::from_value(json!({
            "source_type": "PostgreSQL",
            "connection": {"host": "h", "username": "u", "database": "d", "publication": "mypub"},
            "schema_mode": "all_schemas", "table_mode": "specific_tables",
            "schemas": [], "tables": ["sales.orders", "items", "sales.orders"]
        }))
        .unwrap();
        let c = ctx_from_source(&src, Some("cdc_x".into()));
        assert_eq!(c.engine, "postgres");
        assert_eq!(
            c.tables,
            vec![
                ("sales".into(), "orders".into()),
                ("public".into(), "items".into())
            ]
        );
        assert_eq!(c.publication, "mypub");
        assert_eq!(c.slot.as_deref(), Some("cdc_x"));
    }
}
