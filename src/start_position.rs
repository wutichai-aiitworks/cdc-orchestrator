//! Where a log-based source starts reading: snapshot + stream, from now, from the oldest
//! retained log, or from an operator-supplied LSN / binlog / SCN (ADR 0014).
//!
//! Pure translation (snapshot mode, offset JSON, position parsing) lives here so it can be
//! unit-tested without a stack; [`resolve`] is the only part that talks to the source.

use crate::error::AppError;
use crate::models::{CaptureMode, CdcProfile, LogPosition, SourceConfig, StartPosition};
use serde_json::{json, Value};

/// Connector config key that records which start position the stored offsets came from.
/// Kafka Connect passes unknown keys through; Debezium ignores them.
pub const MARKER_KEY: &str = "cdc.start_position";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Postgres,
    Mysql,
    SqlServer,
    Oracle,
}

impl Dialect {
    pub fn of(source_type: &str) -> Option<Self> {
        match source_type.trim().to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" => Some(Self::Postgres),
            "mysql" | "mariadb" => Some(Self::Mysql),
            "sqlserver" => Some(Self::SqlServer),
            "oracle" => Some(Self::Oracle),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL",
            Self::Mysql => "MySQL",
            Self::SqlServer => "SQL Server",
            Self::Oracle => "Oracle",
        }
    }

    /// The `LogPosition` kind this dialect accepts.
    fn accepts(self, pos: &LogPosition) -> bool {
        matches!(
            (self, pos),
            (Self::Postgres, LogPosition::PostgresLsn { .. })
                | (Self::Mysql, LogPosition::MysqlBinlog { .. })
                | (Self::SqlServer, LogPosition::SqlServerLsn { .. })
                | (Self::Oracle, LogPosition::OracleScn { .. })
        )
    }
}

fn bad(msg: impl Into<String>) -> AppError {
    AppError::BadRequest(msg.into())
}

// ---------------------------------------------------------------------------
// Static validation (profile save / Start, no I/O)
// ---------------------------------------------------------------------------

/// Reject start positions the source cannot honour, before touching anything.
pub fn validate_static(source: &SourceConfig) -> Result<(), AppError> {
    let sp = &source.start_position;
    if *sp == StartPosition::SnapshotThenStream {
        return Ok(());
    }
    if source.capture_mode != CaptureMode::Logical {
        return Err(bad(
            "start_position applies only to log-based capture (capture_mode=logical); \
             trigger, incremental and sync-once pipelines track their own position",
        ));
    }
    let dialect = Dialect::of(&source.source_type).ok_or_else(|| {
        bad(format!(
            "start_position is not supported for source_type '{}'",
            source.source_type
        ))
    })?;
    match sp {
        StartPosition::EarliestRetained if dialect == Dialect::Mysql => Err(bad(
            "earliest_retained is not supported for MySQL yet (the binlog list needs SHOW BINARY LOGS); \
             give a custom binlog file and position instead",
        )),
        StartPosition::Custom { position } => validate_position_shape(dialect, position),
        _ => Ok(()),
    }
}

/// Dialect match and format checks for a supplied position.
pub fn validate_position_shape(dialect: Dialect, pos: &LogPosition) -> Result<(), AppError> {
    if !dialect.accepts(pos) {
        return Err(bad(format!(
            "position kind does not match a {} source",
            dialect.label()
        )));
    }
    match pos {
        LogPosition::PostgresLsn { lsn } => parse_pg_lsn(lsn).map(|_| ()).map_err(bad),
        LogPosition::MysqlBinlog {
            file,
            pos,
            gtid_set,
        } => {
            validate_binlog_file(file).map_err(bad)?;
            if *pos < 4 {
                return Err(bad(
                    "binlog pos must be 4 or greater (4 is the first event)",
                ));
            }
            if let Some(set) = gtid_set {
                validate_gtid_set(set).map_err(bad)?;
            }
            Ok(())
        }
        LogPosition::SqlServerLsn { commit_lsn } => {
            parse_sqlserver_lsn(commit_lsn).map(|_| ()).map_err(bad)
        }
        LogPosition::OracleScn { scn } => parse_scn(scn).map(|_| ()).map_err(bad),
    }
}

// ---------------------------------------------------------------------------
// Connector config
// ---------------------------------------------------------------------------

/// Debezium `snapshot.mode` for this source.
///
/// Seeded positions skip the row copy. Postgres keeps no schema history, so `no_data`
/// streams straight from the seeded LSN. MySQL / SQL Server / Oracle need a schema
/// history before they can decode the log; `recovery` rebuilds it from the current
/// schema and keeps the stored offset.
pub fn snapshot_mode(source: &SourceConfig) -> &'static str {
    if source.capture_mode == CaptureMode::SyncOnce {
        return "initial_only";
    }
    match &source.start_position {
        StartPosition::SnapshotThenStream => "initial",
        StartPosition::Latest => "no_data",
        StartPosition::EarliestRetained | StartPosition::Custom { .. } => {
            match Dialect::of(&source.source_type) {
                Some(Dialect::Postgres) | None => "no_data",
                Some(_) => "recovery",
            }
        }
    }
}

/// Marker value stored in the connector config; `None` for the default (so existing
/// connectors created before this feature compare equal and keep their offsets).
pub fn marker(sp: &StartPosition) -> Option<String> {
    match sp {
        StartPosition::SnapshotThenStream => None,
        other => serde_json::to_string(other).ok(),
    }
}

/// `PATCH /connectors/{name}/offsets` body that makes the connector resume at `pos`.
///
/// Partition keys match what the connectors write themselves: `{"server": topic.prefix}`
/// (plus `"database"` for SQL Server). Only the position fields are seeded; Debezium
/// fills the rest on its first commit.
pub fn offset_body(
    dialect: Dialect,
    server: &str,
    database: &str,
    pos: &LogPosition,
) -> Result<Value, AppError> {
    validate_position_shape(dialect, pos)?;
    let (partition, offset) = match pos {
        LogPosition::PostgresLsn { lsn } => (
            json!({ "server": server }),
            json!({ "lsn": parse_pg_lsn(lsn).map_err(bad)? }),
        ),
        LogPosition::MysqlBinlog {
            file,
            pos,
            gtid_set,
        } => {
            let mut o = json!({ "file": file.trim(), "pos": pos });
            if let Some(set) = gtid_set.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                o["gtids"] = json!(set);
            }
            (json!({ "server": server }), o)
        }
        LogPosition::SqlServerLsn { commit_lsn } => (
            json!({ "server": server, "database": database }),
            // change_lsn left out: Debezium then replays the whole commit, which the
            // upserting sink absorbs, instead of guessing a row inside it.
            json!({ "commit_lsn": parse_sqlserver_lsn(commit_lsn).map_err(bad)? }),
        ),
        LogPosition::OracleScn { scn } => (
            json!({ "server": server }),
            json!({ "scn": parse_scn(scn).map_err(bad)?.to_string() }),
        ),
    };
    Ok(json!({ "offsets": [ { "partition": partition, "offset": offset } ] }))
}

/// Read the position a source connector last committed, from `GET /connectors/{n}/offsets`.
pub fn current_position(dialect: Dialect, offsets: &Value) -> Option<LogPosition> {
    let offset = offsets
        .get("offsets")?
        .as_array()?
        .iter()
        .find_map(|e| e.get("offset"))?;
    match dialect {
        Dialect::Postgres => {
            offset
                .get("lsn")
                .and_then(Value::as_u64)
                .map(|n| LogPosition::PostgresLsn {
                    lsn: format_pg_lsn(n),
                })
        }
        Dialect::Mysql => Some(LogPosition::MysqlBinlog {
            file: offset.get("file")?.as_str()?.to_string(),
            pos: offset.get("pos")?.as_u64()?,
            gtid_set: offset
                .get("gtids")
                .and_then(Value::as_str)
                .map(str::to_string),
        }),
        Dialect::SqlServer => {
            offset
                .get("commit_lsn")
                .and_then(Value::as_str)
                .map(|l| LogPosition::SqlServerLsn {
                    commit_lsn: l.to_string(),
                })
        }
        Dialect::Oracle => {
            let scn = offset.get("scn")?;
            let text = scn
                .as_str()
                .map(str::to_string)
                .or_else(|| scn.as_u64().map(|n| n.to_string()))?;
            Some(LogPosition::OracleScn { scn: text })
        }
    }
}

// ---------------------------------------------------------------------------
// Position formats
// ---------------------------------------------------------------------------

/// `X/Y` hex (as Postgres prints it) or a plain decimal u64 (as Debezium stores it).
pub fn parse_pg_lsn(raw: &str) -> Result<u64, String> {
    let s = raw.trim();
    if let Some((hi, lo)) = s.split_once('/') {
        let hi = u32::from_str_radix(hi.trim(), 16);
        let lo = u32::from_str_radix(lo.trim(), 16);
        return match (hi, lo) {
            (Ok(hi), Ok(lo)) => Ok(((hi as u64) << 32) | lo as u64),
            _ => Err(format!(
                "invalid PostgreSQL LSN '{s}' (expected e.g. 0/1A2B3C4D)"
            )),
        };
    }
    s.parse::<u64>()
        .map_err(|_| format!("invalid PostgreSQL LSN '{s}' (expected e.g. 0/1A2B3C4D)"))
}

pub fn format_pg_lsn(n: u64) -> String {
    format!("{:X}/{:X}", n >> 32, n & 0xFFFF_FFFF)
}

/// Normalise to Debezium's `xxxxxxxx:xxxxxxxx:xxxx` (lower-case hex). Accepts that form or
/// SQL Server's `0x` + 20 hex digits (`CONVERT(varchar, lsn, 1)`).
pub fn parse_sqlserver_lsn(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    let err = || {
        format!("invalid SQL Server LSN '{s}' (expected 0000002a:00000c10:0003 or 0x0000002A00000C100003)")
    };
    let hex: String = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        if h.len() != 20 {
            return Err(err());
        }
        h.to_string()
    } else {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 3 || parts[0].len() != 8 || parts[1].len() != 8 || parts[2].len() != 4 {
            return Err(err());
        }
        parts.concat()
    };
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(err());
    }
    let h = hex.to_ascii_lowercase();
    Ok(format!("{}:{}:{}", &h[0..8], &h[8..16], &h[16..20]))
}

pub fn parse_scn(raw: &str) -> Result<u128, String> {
    let s = raw.trim();
    s.parse::<u128>()
        .map_err(|_| format!("invalid Oracle SCN '{s}' (expected a decimal number)"))
}

fn validate_binlog_file(file: &str) -> Result<(), String> {
    let f = file.trim();
    if f.is_empty()
        || f.len() > 255
        || !f
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(format!(
            "invalid binlog file name '{f}' (e.g. mysql-bin.000042)"
        ));
    }
    Ok(())
}

/// GTID sets are interpolated into a SELECT for validation, so allow only their alphabet.
fn validate_gtid_set(set: &str) -> Result<(), String> {
    let s = set.trim();
    if s.is_empty()
        || !s
            .chars()
            .all(|c| c.is_ascii_hexdigit() || matches!(c, ':' | ',' | '-' | ' ' | '\n'))
    {
        return Err(format!(
            "invalid GTID set '{s}' (e.g. 3E11FA47-71CA-11E1-9E33-C80AA9429562:1-5)"
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Resolution against the live source
// ---------------------------------------------------------------------------

/// The concrete position to seed for `sp`, checked against what the source still holds.
///
/// `Ok(None)` for positions that need no seed (`snapshot_then_stream`; `latest` on a
/// first Start, where Debezium itself starts from now).
pub async fn resolve(
    profile: &CdcProfile,
    sp: &StartPosition,
) -> Result<Option<LogPosition>, AppError> {
    let dialect = Dialect::of(&profile.source.source_type).ok_or_else(|| {
        bad("start_position needs a PostgreSQL, MySQL, SQL Server or Oracle source")
    })?;
    match sp {
        StartPosition::SnapshotThenStream | StartPosition::Latest => Ok(None),
        StartPosition::EarliestRetained => earliest(profile, dialect).await.map(Some),
        StartPosition::Custom { position } => {
            validate_position_shape(dialect, position)?;
            check_custom(profile, dialect, position).await?;
            Ok(Some(position.clone()))
        }
    }
}

/// The source's current write position (used to reposition a running pipeline to "now").
pub async fn current_source_position(profile: &CdcProfile) -> Result<LogPosition, AppError> {
    let dialect = Dialect::of(&profile.source.source_type)
        .ok_or_else(|| bad("unsupported source for repositioning"))?;
    match dialect {
        Dialect::Postgres => {
            let p = pg_state(profile).await?;
            Ok(LogPosition::PostgresLsn {
                lsn: format_pg_lsn(p.current),
            })
        }
        Dialect::SqlServer => {
            let (_, max) = sqlserver_range(profile).await?;
            Ok(LogPosition::SqlServerLsn { commit_lsn: max })
        }
        Dialect::Oracle => {
            let (_, current) = oracle_range(profile).await?;
            Ok(LogPosition::OracleScn {
                scn: current.to_string(),
            })
        }
        Dialect::Mysql => Err(bad(
            "repositioning a MySQL source to 'latest' is not supported yet (needs SHOW MASTER STATUS); \
             give a custom binlog file and position",
        )),
    }
}

async fn earliest(profile: &CdcProfile, dialect: Dialect) -> Result<LogPosition, AppError> {
    match dialect {
        Dialect::Postgres => {
            let p = pg_state(profile).await?;
            let confirmed = p.slot_confirmed.ok_or_else(|| {
                bad(format!(
                    "earliest_retained needs an existing replication slot '{}': a new slot starts at the current WAL position. \
                     Use latest, or snapshot_then_stream to copy existing rows",
                    p.slot
                ))
            })?;
            // Postgres never streams from before confirmed_flush_lsn, even though WAL back
            // to restart_lsn is still on disk.
            Ok(LogPosition::PostgresLsn {
                lsn: format_pg_lsn(confirmed),
            })
        }
        Dialect::SqlServer => {
            let (min, _) = sqlserver_range(profile).await?;
            Ok(LogPosition::SqlServerLsn { commit_lsn: min })
        }
        Dialect::Oracle => {
            let (min, _) = oracle_range(profile).await?;
            Ok(LogPosition::OracleScn {
                scn: min.to_string(),
            })
        }
        Dialect::Mysql => Err(bad("earliest_retained is not supported for MySQL yet")),
    }
}

async fn check_custom(
    profile: &CdcProfile,
    dialect: Dialect,
    pos: &LogPosition,
) -> Result<(), AppError> {
    match (dialect, pos) {
        (Dialect::Postgres, LogPosition::PostgresLsn { lsn }) => {
            let want = parse_pg_lsn(lsn).map_err(bad)?;
            let p = pg_state(profile).await?;
            pg_check_lsn(want, &p)
        }
        (Dialect::SqlServer, LogPosition::SqlServerLsn { commit_lsn }) => {
            let want = parse_sqlserver_lsn(commit_lsn).map_err(bad)?;
            let (min, max) = sqlserver_range(profile).await?;
            // Fixed-width lower-case hex: lexical order is numeric order.
            if want < min {
                return Err(bad(format!(
                    "LSN {want} is older than the oldest change still in the CDC tables ({min})"
                )));
            }
            if want > max {
                return Err(bad(format!(
                    "LSN {want} is ahead of the source (current {max})"
                )));
            }
            Ok(())
        }
        (Dialect::Oracle, LogPosition::OracleScn { scn }) => {
            let want = parse_scn(scn).map_err(bad)?;
            let (min, current) = oracle_range(profile).await?;
            if want < min {
                return Err(bad(format!(
                    "SCN {want} is older than the oldest retained redo/archive log (first SCN {min})"
                )));
            }
            if want > current {
                return Err(bad(format!(
                    "SCN {want} is ahead of the source (current {current})"
                )));
            }
            Ok(())
        }
        (Dialect::Mysql, LogPosition::MysqlBinlog { gtid_set, .. }) => {
            // The binlog file list is not reachable through a SELECT; Debezium reports a
            // missing file on its first poll. A GTID set can be checked against what the
            // server has already purged.
            if let Some(set) = gtid_set.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                validate_gtid_set(set).map_err(bad)?;
                let sql = format!("SELECT GTID_SUBSET(@@GLOBAL.gtid_purged, '{set}') AS ok");
                let ok = scalar(profile, &sql).await?;
                if !matches!(ok.as_ref().and_then(value_as_i128), Some(1)) {
                    return Err(bad(
                        "GTID set does not include every transaction the server has already purged; \
                         those changes are gone and the connector would fail",
                    ));
                }
            }
            Ok(())
        }
        _ => validate_position_shape(dialect, pos),
    }
}

struct PgState {
    slot: String,
    slot_confirmed: Option<u64>,
    current: u64,
}

fn pg_check_lsn(want: u64, p: &PgState) -> Result<(), AppError> {
    if want > p.current {
        return Err(bad(format!(
            "LSN {} is ahead of the source (current {})",
            format_pg_lsn(want),
            format_pg_lsn(p.current)
        )));
    }
    match p.slot_confirmed {
        Some(confirmed) if want < confirmed => Err(bad(format!(
            "LSN {} is older than replication slot '{}' (confirmed {}); Postgres no longer serves changes before it",
            format_pg_lsn(want),
            p.slot,
            format_pg_lsn(confirmed)
        ))),
        None if want < p.current => Err(bad(format!(
            "replication slot '{}' does not exist yet, and a new slot starts at the current WAL position ({}); \
             an older LSN ({}) cannot be replayed",
            p.slot,
            format_pg_lsn(p.current),
            format_pg_lsn(want)
        ))),
        _ => Ok(()),
    }
}

async fn pg_state(profile: &CdcProfile) -> Result<PgState, AppError> {
    let insight =
        crate::connector_catalog::insight_from_connection("postgres", &profile.source.connection);
    let client = crate::connector_catalog::postgres_client(&insight).await?;
    let slot = crate::reconcile::postgres_slot_name(profile);
    let current: String = client
        .query_one("SELECT pg_current_wal_lsn()::text", &[])
        .await
        .map_err(|e| bad(format!("could not read the source WAL position: {e}")))?
        .get(0);
    let confirmed: Option<String> = client
        .query_opt(
            "SELECT confirmed_flush_lsn::text FROM pg_replication_slots WHERE slot_name = $1",
            &[&slot],
        )
        .await
        .map_err(|e| bad(format!("could not read replication slot '{slot}': {e}")))?
        .and_then(|row| row.get::<_, Option<String>>(0));
    Ok(PgState {
        current: parse_pg_lsn(&current).map_err(AppError::Upstream)?,
        slot_confirmed: confirmed
            .as_deref()
            .map(parse_pg_lsn)
            .transpose()
            .map_err(AppError::Upstream)?,
        slot,
    })
}

/// (oldest, newest) LSN still in the SQL Server CDC tables, Debezium form.
async fn sqlserver_range(profile: &CdcProfile) -> Result<(String, String), AppError> {
    let min = scalar_text(
        profile,
        "SELECT CONVERT(VARCHAR(22), MAX(start_lsn), 1) FROM cdc.change_tables",
    )
    .await?
    .ok_or_else(|| {
        bad("no CDC capture instances on the source (is CDC enabled for the tables?)")
    })?;
    let max = scalar_text(
        profile,
        "SELECT CONVERT(VARCHAR(22), sys.fn_cdc_get_max_lsn(), 1)",
    )
    .await?
    .ok_or_else(|| bad("could not read the source's current CDC LSN"))?;
    Ok((
        parse_sqlserver_lsn(&min).map_err(AppError::Upstream)?,
        parse_sqlserver_lsn(&max).map_err(AppError::Upstream)?,
    ))
}

/// (oldest retained, current) SCN on an Oracle source.
async fn oracle_range(profile: &CdcProfile) -> Result<(u128, u128), AppError> {
    let min = scalar_text(
        profile,
        "SELECT MIN(first_change#) FROM (\
           SELECT first_change# FROM v$archived_log WHERE deleted = 'NO' AND status = 'A' \
           UNION ALL SELECT first_change# FROM v$log)",
    )
    .await?
    .ok_or_else(|| bad("could not read the oldest retained SCN (v$archived_log / v$log)"))?;
    let current = scalar_text(profile, "SELECT current_scn FROM v$database")
        .await?
        .ok_or_else(|| bad("could not read the source's current SCN"))?;
    Ok((
        parse_scn(&min).map_err(AppError::Upstream)?,
        parse_scn(&current).map_err(AppError::Upstream)?,
    ))
}

async fn scalar(profile: &CdcProfile, sql: &str) -> Result<Option<Value>, AppError> {
    let admin = crate::clients::JdbcAdminClient::from_env().ok_or_else(|| {
        bad("checking a start position on this source needs cdc-jdbc-admin — set ORCH_JDBC_ADMIN_URL")
    })?;
    let db_type = profile.source.source_type.trim().to_ascii_lowercase();
    let res = admin
        .query(&db_type, &profile.source.connection, sql)
        .await
        .map_err(|e| {
            bad(format!(
                "could not read the source log position: {}",
                e.to_string_message()
            ))
        })?;
    Ok(res
        .rows
        .into_iter()
        .next()
        .and_then(|r| r.into_iter().next())
        .filter(|v| !v.is_null()))
}

async fn scalar_text(profile: &CdcProfile, sql: &str) -> Result<Option<String>, AppError> {
    Ok(scalar(profile, sql).await?.map(|v| match v {
        Value::String(s) => s,
        other => other.to_string(),
    }))
}

fn value_as_i128(v: &Value) -> Option<i128> {
    match v {
        Value::Number(n) => n.as_i64().map(i128::from),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(*b as i128),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(source_type: &str, sp: StartPosition) -> SourceConfig {
        let mut s: SourceConfig = serde_json::from_value(json!({
            "source_type": source_type,
            "connection": {},
            "schema_mode": "all_schemas",
            "schemas": [],
            "tables": [],
        }))
        .unwrap();
        s.start_position = sp;
        s
    }

    fn custom(position: LogPosition) -> StartPosition {
        StartPosition::Custom { position }
    }

    #[test]
    fn pg_lsn_roundtrip() {
        assert_eq!(parse_pg_lsn("0/1A2B3C4D").unwrap(), 0x1A2B3C4D);
        assert_eq!(
            parse_pg_lsn("16/B374D848").unwrap(),
            (0x16u64 << 32) | 0xB374D848
        );
        assert_eq!(parse_pg_lsn("115878256").unwrap(), 115_878_256);
        assert_eq!(format_pg_lsn(115_878_256), "0/6E82970");
        let n = parse_pg_lsn("A/FF").unwrap();
        assert_eq!(parse_pg_lsn(&format_pg_lsn(n)).unwrap(), n);
        assert!(parse_pg_lsn("x/1").is_err());
        assert!(parse_pg_lsn("").is_err());
    }

    #[test]
    fn sqlserver_lsn_forms_normalise() {
        assert_eq!(
            parse_sqlserver_lsn("0x0000002A00000C100003").unwrap(),
            "0000002a:00000c10:0003"
        );
        assert_eq!(
            parse_sqlserver_lsn("0000002A:00000C10:0003").unwrap(),
            "0000002a:00000c10:0003"
        );
        assert!(parse_sqlserver_lsn("0x12").is_err());
        assert!(parse_sqlserver_lsn("zzzzzzzz:00000c10:0003").is_err());
    }

    #[test]
    fn snapshot_modes() {
        let pg = |sp| snapshot_mode(&source("postgres", sp));
        assert_eq!(pg(StartPosition::SnapshotThenStream), "initial");
        assert_eq!(pg(StartPosition::Latest), "no_data");
        assert_eq!(pg(StartPosition::EarliestRetained), "no_data");
        assert_eq!(
            snapshot_mode(&source(
                "oracle",
                custom(LogPosition::OracleScn { scn: "1".into() })
            )),
            "recovery"
        );
        let mut once = source("postgres", StartPosition::Latest);
        once.capture_mode = CaptureMode::SyncOnce;
        assert_eq!(snapshot_mode(&once), "initial_only");
    }

    #[test]
    fn static_validation() {
        assert!(validate_static(&source("postgres", StartPosition::SnapshotThenStream)).is_ok());
        assert!(validate_static(&source("postgres", StartPosition::Latest)).is_ok());
        // Wrong dialect for the position kind.
        assert!(validate_static(&source(
            "postgres",
            custom(LogPosition::OracleScn { scn: "1".into() })
        ))
        .is_err());
        // Non-log capture modes reject a start position.
        let mut trig = source("postgres", StartPosition::Latest);
        trig.capture_mode = CaptureMode::Trigger;
        assert!(validate_static(&trig).is_err());
        // MySQL earliest is not available in v1.
        assert!(validate_static(&source("mysql", StartPosition::EarliestRetained)).is_err());
        // Injection-shaped GTID sets are refused before any SQL is built.
        assert!(validate_static(&source(
            "mysql",
            custom(LogPosition::MysqlBinlog {
                file: "mysql-bin.000001".into(),
                pos: 4,
                gtid_set: Some("x') OR 1=1 --".into()),
            })
        ))
        .is_err());
        assert!(validate_static(&source(
            "mysql",
            custom(LogPosition::MysqlBinlog {
                file: "mysql-bin.000001".into(),
                pos: 0,
                gtid_set: None,
            })
        ))
        .is_err());
    }

    #[test]
    fn offsets_match_connector_format() {
        let pg = offset_body(
            Dialect::Postgres,
            "demo",
            "demo",
            &LogPosition::PostgresLsn {
                lsn: "0/6E82970".into(),
            },
        )
        .unwrap();
        assert_eq!(
            pg,
            json!({"offsets":[{"partition":{"server":"demo"},"offset":{"lsn":115878256u64}}]})
        );
        let ms = offset_body(
            Dialect::SqlServer,
            "s",
            "shop",
            &LogPosition::SqlServerLsn {
                commit_lsn: "0x0000002A00000C100003".into(),
            },
        )
        .unwrap();
        assert_eq!(
            ms["offsets"][0],
            json!({"partition":{"server":"s","database":"shop"},"offset":{"commit_lsn":"0000002a:00000c10:0003"}})
        );
        let my = offset_body(
            Dialect::Mysql,
            "m",
            "db",
            &LogPosition::MysqlBinlog {
                file: "mysql-bin.000042".into(),
                pos: 154,
                gtid_set: Some("3E11FA47-71CA-11E1-9E33-C80AA9429562:1-5".into()),
            },
        )
        .unwrap();
        assert_eq!(
            my["offsets"][0]["offset"]["gtids"],
            "3E11FA47-71CA-11E1-9E33-C80AA9429562:1-5"
        );
        assert_eq!(my["offsets"][0]["offset"]["pos"], 154);
        let ora = offset_body(
            Dialect::Oracle,
            "o",
            "ORCL",
            &LogPosition::OracleScn {
                scn: "9876543210".into(),
            },
        )
        .unwrap();
        assert_eq!(ora["offsets"][0]["offset"]["scn"], "9876543210");
    }

    #[test]
    fn current_position_reads_live_pg_offset() {
        // Shape copied from a live Debezium 2.7 Postgres connector.
        let live = json!({"offsets":[{"partition":{"server":"cdc_supertokens"},"offset":{
            "lsn_proc":115878256u64,"messageType":"INSERT","lsn_commit":115878256u64,
            "lsn":115878256u64,"txId":30889,"ts_usec":1794144610719620u64}}]});
        assert_eq!(
            current_position(Dialect::Postgres, &live),
            Some(LogPosition::PostgresLsn {
                lsn: "0/6E82970".into()
            })
        );
        assert_eq!(
            current_position(Dialect::Postgres, &json!({"offsets":[]})),
            None
        );
    }

    #[test]
    fn pg_lsn_bounds() {
        let with_slot = PgState {
            slot: "s".into(),
            slot_confirmed: Some(100),
            current: 500,
        };
        assert!(pg_check_lsn(99, &with_slot).is_err());
        assert!(pg_check_lsn(100, &with_slot).is_ok());
        assert!(pg_check_lsn(300, &with_slot).is_ok());
        assert!(pg_check_lsn(501, &with_slot).is_err());
        let no_slot = PgState {
            slot: "s".into(),
            slot_confirmed: None,
            current: 500,
        };
        assert!(pg_check_lsn(499, &no_slot).is_err());
        assert!(pg_check_lsn(500, &no_slot).is_ok());
    }

    #[test]
    fn marker_only_for_non_default() {
        assert_eq!(marker(&StartPosition::SnapshotThenStream), None);
        assert_eq!(
            marker(&StartPosition::Latest).as_deref(),
            Some(r#"{"mode":"latest"}"#)
        );
    }

    #[test]
    fn serde_shape() {
        let sp: StartPosition = serde_json::from_value(json!({
            "mode": "custom",
            "position": {"kind": "postgres_lsn", "lsn": "0/16B3748"}
        }))
        .unwrap();
        assert!(sp.is_seeded());
        let s = source("postgres", StartPosition::default());
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["start_position"], json!({"mode": "snapshot_then_stream"}));
    }
}
