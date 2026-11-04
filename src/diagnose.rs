//! Turn raw errors (Kafka Connect stack traces, JDBC / driver messages, HTTP failures)
//! into "where is the problem and how do I fix it".
//!
//! Every surface that shows an error — API error bodies, the table-status banner, the
//! per-table count / Kafka errors — attaches a [`Diagnosis`] so the operator sees the
//! failing component (source DB, target DB, network, Kafka Connect, …) and the next
//! step first, with the raw message kept as detail.
//!
//! Rules are ordered: specific database causes before generic connectivity, so a trace
//! such as "couldn't obtain encoding … caused by UnknownHostException" lands on the
//! actual cause (DNS) rather than on the outer symptom.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorArea {
    SourceDatabase,
    TargetDatabase,
    Network,
    KafkaConnect,
    Kafka,
    SchemaRegistry,
    PipelineConfig,
    Orchestrator,
}

/// Which side of the pipeline produced the error, when the caller knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Source,
    Target,
    Unknown,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Diagnosis {
    pub area: ErrorArea,
    /// Human label of the area, e.g. "Source database", "Network → target".
    pub area_label: String,
    /// `source` / `target` when the failure belongs to one end of the pipeline, so a
    /// UI can say "Source offline" without reading the technical text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<&'static str>,
    /// One-line statement of what is wrong.
    pub title: String,
    /// What to do about it.
    pub fix: String,
}

impl Side {
    fn db_area(self) -> ErrorArea {
        match self {
            Side::Target => ErrorArea::TargetDatabase,
            _ => ErrorArea::SourceDatabase,
        }
    }
    fn word(self) -> &'static str {
        match self {
            Side::Source => "source",
            Side::Target => "target",
            Side::Unknown => "database",
        }
    }
}

fn label(area: ErrorArea, side: Side) -> String {
    match area {
        ErrorArea::SourceDatabase => "Source database".into(),
        ErrorArea::TargetDatabase => "Target database".into(),
        ErrorArea::Network => match side {
            Side::Unknown => "Network".into(),
            s => format!("Network → {}", s.word()),
        },
        ErrorArea::KafkaConnect => "Kafka Connect".into(),
        ErrorArea::Kafka => "Kafka".into(),
        ErrorArea::SchemaRegistry => "Schema Registry".into(),
        ErrorArea::PipelineConfig => "Pipeline settings".into(),
        ErrorArea::Orchestrator => "Orchestrator".into(),
    }
}

fn d(area: ErrorArea, side: Side, title: impl Into<String>, fix: impl Into<String>) -> Diagnosis {
    Diagnosis {
        area,
        area_label: label(area, side),
        side: match (area, side) {
            (ErrorArea::SourceDatabase, _) => Some("source"),
            (ErrorArea::TargetDatabase, _) => Some("target"),
            (_, Side::Source) => Some("source"),
            (_, Side::Target) => Some("target"),
            _ => None,
        },
        title: title.into(),
        fix: fix.into(),
    }
}

/// Guess the side from wording when the caller does not know it.
fn infer_side(lower: &str) -> Side {
    let source = lower.contains("source");
    let target = lower.contains("target") || lower.contains("sink");
    match (source, target) {
        (true, false) => Side::Source,
        (false, true) => Side::Target,
        _ => Side::Unknown,
    }
}

fn any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| text.contains(n))
}

fn diagnose_postgres_replication(t: &str) -> Option<Diagnosis> {
    // ── PostgreSQL logical replication (always the source) ─────────────────────
    if any(t, &["publication autocreation is disabled"])
        || (t.contains("publication") && t.contains("does not exist"))
    {
        return Some(d(
            ErrorArea::SourceDatabase,
            Side::Source,
            "Publication is missing on the source database",
            "As the owner of the tables run: CREATE PUBLICATION dbz_publication FOR TABLE <schema.table>, …; then Restart the pipeline.",
        ));
    }
    if any(t, &["wal_level", "logical decoding requires"]) {
        return Some(d(
            ErrorArea::SourceDatabase,
            Side::Source,
            "Source PostgreSQL is not configured for logical replication",
            "Set wal_level = logical in postgresql.conf and RESTART PostgreSQL (a reload is not enough), then Restart the pipeline.",
        ));
    }
    if any(
        t,
        &["all replication slots are in use", "max_replication_slots"],
    ) {
        return Some(d(
            ErrorArea::SourceDatabase,
            Side::Source,
            "No free replication slot on the source",
            "Drop unused slots (SELECT pg_drop_replication_slot('<name>')) or raise max_replication_slots and restart PostgreSQL.",
        ));
    }
    if t.contains("replication slot")
        && any(t, &["is active for pid", "already active", "is in use"])
    {
        return Some(d(
            ErrorArea::SourceDatabase,
            Side::Source,
            "Replication slot is held by another connection",
            "Another connector or process is attached to the same slot. Stop it, or give this pipeline its own slot name.",
        ));
    }
    if any(
        t,
        &[
            "can no longer get changes from replication slot",
            "requested wal segment",
            "has been invalidated",
            "wal_status",
        ],
    ) || (t.contains("replication slot") && t.contains("does not exist"))
    {
        return Some(d(
            ErrorArea::SourceDatabase,
            Side::Source,
            "Replication slot was lost or invalidated",
            "WAL needed by the pipeline was removed (e.g. max_slot_wal_keep_size exceeded while it was down). Use Resync to reset the slot and take a fresh snapshot.",
        ));
    }
    if any(
        t,
        &[
            "must be superuser or replication role",
            "permission denied to start wal sender",
            "must have replication",
        ],
    ) {
        return Some(d(
            ErrorArea::SourceDatabase,
            Side::Source,
            "Source user lacks the REPLICATION privilege",
            "Run as a superuser: ALTER ROLE <pipeline user> REPLICATION;",
        ));
    }
    None
}

fn diagnose_credentials(t: &str, side: Side, db: ErrorArea, who: &str) -> Option<Diagnosis> {
    // ── Credentials / privileges (either side) ─────────────────────────────────
    if any(
        t,
        &[
            "password authentication failed",
            "ora-01017",
            "access denied for user",
            "login failed for user",
            "invalid username/password",
        ],
    ) {
        return Some(d(
            db,
            side,
            format!("Wrong username or password for the {who} database"),
            "Fix the credentials in Edit → connection, then Restart.",
        ));
    }
    if any(t, &["no pg_hba.conf entry"]) {
        return Some(d(
            db,
            side,
            format!("The {who} database rejects this client address (pg_hba.conf)"),
            "Allow the Kafka Connect / orchestrator host in pg_hba.conf and reload PostgreSQL.",
        ));
    }
    if any(
        t,
        &[
            "permission denied for",
            "ora-01031",
            "insufficient privileges",
            "command denied to user",
        ],
    ) {
        let fix = if side == Side::Target {
            "Grant INSERT, UPDATE, DELETE (and CREATE / TRUNCATE if the pipeline manages tables) on the target schema to the pipeline user."
        } else {
            "Grant SELECT on the captured tables (and USAGE on the schema) to the pipeline user."
        };
        return Some(d(
            db,
            side,
            format!("Missing privilege on the {who} database"),
            fix,
        ));
    }
    None
}

fn diagnose_connectivity(t: &str, side: Side, who: &str) -> Option<Diagnosis> {
    // ── Connectivity ───────────────────────────────────────────────────────────
    if any(
        t,
        &[
            "unknownhostexception",
            "could not translate host name",
            "name or service not known",
            "no such host",
            "nodename nor servname",
            "failed to lookup address",
        ],
    ) {
        return Some(d(
            ErrorArea::Network,
            side,
            format!("The {who} hostname cannot be resolved"),
            "The host name is unknown from inside Docker. Use a name the Kafka Connect / orchestrator containers can resolve (a compose service name or host.docker.internal) in the pipeline connection.",
        ));
    }
    if any(t, &["unable to determine dialect"]) {
        return Some(d(
            ErrorArea::Network,
            Side::Target,
            "The sink could not open a connection to the target database",
            "Check the target host, port, database and credentials in the pipeline; the sink needs a working JDBC connection before it can start.",
        ));
    }
    // pgjdbc words it "Connection to host:port refused."
    if (t.contains("connection to") && t.contains("refused"))
        || any(
            t,
            &[
                "connection refused",
                "the connection attempt failed",
                "communications link failure",
                "no route to host",
                "ora-12541",
                "ora-12514",
                "ora-12170",
                "network is unreachable",
                "connection reset",
                "could not connect to server",
            ],
        )
    {
        return Some(d(
            ErrorArea::Network,
            side,
            format!("The {who} database is not reachable"),
            "Check the database is running and the host/port are correct. If it goes through an SSH tunnel or port-forward (e.g. Termius), make sure the tunnel is connected.",
        ));
    }
    None
}

fn diagnose_kafka_stack(t: &str) -> Option<Diagnosis> {
    // ── Kafka Connect / Kafka / Schema Registry ────────────────────────────────
    if any(
        t,
        &[
            "cdc-connect",
            "connect request failed",
            "connect probe failed",
        ],
    ) {
        return Some(d(
            ErrorArea::KafkaConnect,
            Side::Unknown,
            "Kafka Connect is not reachable",
            "Check the cdc-connect container is running and healthy, then retry.",
        ));
    }
    if any(
        t,
        &[
            "no status found for connector",
            "did not reach stopped",
            "rebalance",
        ],
    ) {
        return Some(d(
            ErrorArea::KafkaConnect,
            Side::Unknown,
            "Kafka Connect is still applying a change to the connector",
            "Wait a few seconds and retry; Connect updates connector status asynchronously.",
        ));
    }
    if any(
        t,
        &[
            "schema registry",
            "schemaregistry",
            "error retrieving avro",
            "subject not found",
            ":8081",
        ],
    ) {
        return Some(d(
            ErrorArea::SchemaRegistry,
            Side::Unknown,
            "Schema Registry request failed",
            "Check Schema Registry is running and reachable from Kafka Connect (schema.registry.url), and that the subject's schema is compatible.",
        ));
    }
    if any(
        t,
        &[
            "topic not found or no partitions",
            "unknowntopicorpartition",
            "unknown_topic_or_partition",
        ],
    ) {
        return Some(d(
            ErrorArea::Kafka,
            Side::Unknown,
            "Capture topic does not exist yet",
            "Normal until the source writes its first record for this table. If the pipeline has been running a while, check the source connector.",
        ));
    }
    if (t.contains("timeoutexception") && any(t, &["metadata", "broker", "kafka"]))
        || any(
            t,
            &[
                "disconnected from node",
                "bootstrap broker",
                "leader not available",
            ],
        )
    {
        return Some(d(
            ErrorArea::Kafka,
            Side::Unknown,
            "Kafka broker is not reachable",
            "Check the Kafka / Redpanda container (cdc-message) is running and healthy.",
        ));
    }
    None
}

fn is_missing_table_error(t: &str) -> bool {
    any(t, &["ora-00942"])
        || ((t.contains("relation") || t.contains("table")) && t.contains("does not exist"))
        || t.contains("doesn't exist")
}

fn diagnose_missing_table(t: &str, side: Side, db: ErrorArea, who: &str) -> Option<Diagnosis> {
    if !is_missing_table_error(t) {
        return None;
    }
    let fix = if side == Side::Target {
        "Create the target table (Configuration → target DDL), or fix the mapping's target schema/table name."
    } else {
        "Check the schema and table name in the pipeline, and that the source user can see the table."
    };
    Some(d(
        db,
        side,
        format!("Table not found on the {who} database"),
        fix,
    ))
}

enum TargetIntegrityIssue {
    DuplicateKey,
    NotNull,
    TypeMismatch,
}

fn target_integrity_issue(t: &str) -> Option<TargetIntegrityIssue> {
    if t.contains("duplicate key value") || t.contains("ora-00001") {
        Some(TargetIntegrityIssue::DuplicateKey)
    } else if t.contains("violates not-null constraint") || t.contains("ora-01400") {
        Some(TargetIntegrityIssue::NotNull)
    } else if any(
        t,
        &[
            "value too long",
            "out of range",
            "invalid input syntax",
            "numeric field overflow",
        ],
    ) {
        Some(TargetIntegrityIssue::TypeMismatch)
    } else {
        None
    }
}

fn diagnose_target_integrity(t: &str) -> Option<Diagnosis> {
    Some(match target_integrity_issue(t)? {
        TargetIntegrityIssue::DuplicateKey => d(
            ErrorArea::TargetDatabase,
            Side::Target,
            "Duplicate key on the target",
            "The pipeline's upsert / primary key columns do not match the target table's primary key. Align them in the mapping.",
        ),
        TargetIntegrityIssue::NotNull => d(
            ErrorArea::TargetDatabase,
            Side::Target,
            "NOT NULL constraint violated on the target",
            "A source column arrives empty (or is excluded) but the target column is NOT NULL. Relax the constraint or map a value.",
        ),
        TargetIntegrityIssue::TypeMismatch => d(
            ErrorArea::TargetDatabase,
            Side::Target,
            "Data type mismatch on the target",
            "A target column's type or length is smaller than the source value. Widen the column or adjust the mapping.",
        ),
    })
}

fn diagnose_tables_and_data(t: &str, side: Side, db: ErrorArea, who: &str) -> Option<Diagnosis> {
    if any(t, &["because all of these columns are nullable"]) {
        return Some(d(
            ErrorArea::PipelineConfig,
            Side::Source,
            "Incremental timestamp column is nullable",
            "Pick a NOT NULL timestamp column, or set this table's poll strategy to Rolling window or Full copy.",
        ));
    }
    if let Some(found) = diagnose_missing_table(t, side, db, who) {
        return Some(found);
    }
    if (t.contains("column") && t.contains("does not exist")) || t.contains("ora-00904") {
        return Some(d(
            db,
            side,
            format!("Column mismatch on the {who} database"),
            "Source and target columns differ. Add the column to the target or adjust the column mapping.",
        ));
    }
    if let Some(found) = diagnose_target_integrity(t) {
        return Some(found);
    }
    if t.contains("ora-01555") || t.contains("snapshot too old") {
        return Some(d(
            ErrorArea::SourceDatabase,
            Side::Source,
            "Source read was too long for the undo retention (ORA-01555)",
            "Re-run off-peak, or ask the DBA to raise UNDO_RETENTION. Rows already copied are kept.",
        ));
    }
    if t.contains("jdbc-admin") {
        return Some(d(
            ErrorArea::Orchestrator,
            side,
            "The jdbc-admin helper failed",
            "Check the cdc-jdbc-admin container is running; the detail shows the database error it received.",
        ));
    }
    None
}

/// Diagnose a raw error. `None` when no rule recognises it (plain validation messages
/// such as "name is required" need no diagnosis).
pub fn diagnose(raw: &str, side: Side) -> Option<Diagnosis> {
    let t = raw.to_ascii_lowercase();
    let side = if side == Side::Unknown {
        infer_side(&t)
    } else {
        side
    };
    let db = side.db_area();
    let who = side.word();
    diagnose_postgres_replication(&t)
        .or_else(|| diagnose_credentials(&t, side, db, who))
        .or_else(|| diagnose_connectivity(&t, side, who))
        .or_else(|| diagnose_kafka_stack(&t))
        .or_else(|| diagnose_tables_and_data(&t, side, db, who))
}

/// Like [`diagnose`], but always returns something, pointing at the side the caller
/// knows produced the error.
pub fn diagnose_or_side(raw: &str, side: Side) -> Diagnosis {
    diagnose(raw, side).unwrap_or_else(|| {
        let area = match side {
            Side::Unknown => ErrorArea::Orchestrator,
            s => s.db_area(),
        };
        d(
            area,
            side,
            "Unrecognised error",
            "Read the detail below — it is the original message from the failing component.",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(raw: &str, side: Side) -> ErrorArea {
        diagnose(raw, side).expect("should diagnose").area
    }

    #[test]
    fn postgres_replication_setup() {
        let pub_missing = "org.apache.kafka.connect.errors.ConnectException: Publication autocreation is disabled, please create one and restart the connector.";
        assert_eq!(area(pub_missing, Side::Source), ErrorArea::SourceDatabase);
        assert!(diagnose(pub_missing, Side::Source)
            .unwrap()
            .fix
            .contains("CREATE PUBLICATION"));
        assert_eq!(
            area(
                "ERROR: logical decoding requires wal_level >= logical",
                Side::Source
            ),
            ErrorArea::SourceDatabase
        );
    }

    #[test]
    fn root_cause_beats_outer_symptom() {
        let trace = "io.debezium.DebeziumException: Couldn't obtain encoding for database demo (caused by java.net.UnknownHostException: cdc-postgres-source)";
        let dx = diagnose(trace, Side::Source).unwrap();
        assert_eq!(dx.area, ErrorArea::Network);
        assert_eq!(dx.area_label, "Network → source");
        assert_eq!(dx.side, Some("source"));
        assert!(dx.title.contains("hostname"));
    }

    #[test]
    fn side_controls_area_and_wording() {
        let refused = "jdbc-admin /v1/count failed [502 Bad Gateway]: Connection to host.docker.internal:11324 refused.";
        let dx = diagnose(refused, Side::Source).unwrap();
        assert_eq!(dx.area, ErrorArea::Network);
        assert!(dx.title.contains("source"));
        assert!(dx.fix.contains("tunnel"));
        assert_eq!(
            area(
                "ERROR: relation \"pandora.pm_probrand\" does not exist",
                Side::Target
            ),
            ErrorArea::TargetDatabase
        );
        assert_eq!(
            area(
                "failed to clear target tables: permission denied for table x",
                Side::Unknown
            ),
            ErrorArea::TargetDatabase
        );
    }

    #[test]
    fn kafka_and_connect() {
        assert_eq!(
            area(
                "Unable to determine Dialect without JDBC metadata",
                Side::Target
            ),
            ErrorArea::Network
        );
        assert_eq!(
            area("connect request failed: error sending request for url (http://cdc-connect:8083/connectors)", Side::Unknown),
            ErrorArea::KafkaConnect
        );
        assert_eq!(
            area("topic not found or no partitions", Side::Unknown),
            ErrorArea::Kafka
        );
        assert_eq!(
            area("Cannot make incremental queries using timestamp columns UPDATE_DATE on \"CRM_STG\".\"PM_PROBRAND\" because all of these columns are nullable.", Side::Source),
            ErrorArea::PipelineConfig
        );
    }

    #[test]
    fn plain_validation_has_no_diagnosis() {
        assert!(diagnose("name is required", Side::Unknown).is_none());
        assert_eq!(
            diagnose_or_side("weird", Side::Target).area,
            ErrorArea::TargetDatabase
        );
    }
}
