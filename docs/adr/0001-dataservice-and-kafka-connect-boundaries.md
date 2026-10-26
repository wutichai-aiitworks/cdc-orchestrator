# ADR 0001 — jdbc-admin for the control plane, Kafka Connect for the data plane

Status: Accepted (supersedes DataService control-plane dependency)
Owner: TBD
Last updated: 2026-09-05

## Context

`hyper-cdc-orchestrator` needs to touch customer databases for two very different reasons, and the two have almost nothing in common:

- **Control plane** — list schemas/tables/columns, test connections, count rows, preview data, execute DDL for Schema Compare. Low volume, request/response, run on operator demand. The hard requirement here is **breadth of database support**: customer sources include Oracle, DB2/AS400, and Informix.
- **Data plane** — continuous replication of every insert/update/delete. High volume, long-running, needs offset tracking, backpressure, restart/resume, and schema evolution.

Rust has no production-grade driver for Oracle, DB2/AS400, or Informix. Most of those engines do speak **JDBC Type 4**. The orchestrator still compiles native drivers for Postgres/MySQL/SQL Server/SQLite as a fallback when jdbc-admin is down.

Hyper DataService is a separate product (keystore, MyBatis mappers, txs). CDC no longer depends on it.

## Decision

Split by plane, and use a different tool for each.

**Control plane → `cdc-jdbc-admin`**, a narrow Java HTTP service using JDBC Type 4 + `DatabaseMetaData` (`docker/jdbc-admin/`). Catalog, connection test, inspect, preview, counts, and one-shot DDL. Configured through `ORCH_JDBC_ADMIN_URL` / `ORCH_JDBC_ADMIN_TOKEN`. It is a **separate Compose service**, not a process inside Connect and not kafka-connect-jdbc connector APIs.

**Data plane → Kafka Connect**, with the orchestrator acting purely as a config compiler: it builds connector JSON and pushes it to the Connect REST API (`reconcile_start()` in `src/reconcile.rs`). It never sits in the record path.

**Control-plane state → rqlite** (`src/persist.rs`), then in-memory. No DataService keystore fallback.

### Connector selection (data plane)

| Role | Capture mode | Connector class |
|------|--------------|-----------------|
| Source (default) | `logical`, `sync_once` | `io.debezium.connector.{postgresql,mysql,sqlserver,oracle}.*` (`src/reconcile.rs:117`) |
| Source (fallback) | `trigger`, `incremental` | `io.confluent.connect.jdbc.JdbcSourceConnector` |
| Sink (replica) | all | `io.debezium.connector.jdbc.JdbcSinkConnector` (`src/reconcile.rs:267`) |
| Sink (lake) | `source_to_lake` | `org.apache.iceberg.connect.IcebergSinkConnector` (`src/lake.rs`) |

**Note on naming:** Confluent's `kafka-connect-jdbc` is a **source-side fallback only**, used when WAL/binlog capture is unavailable. Delivery to relational targets is **Debezium's** JDBC sink. This matters for target DDL: table creation and evolution are governed by Debezium's `schema.evolution=basic`, **not** Confluent's `auto.create` / `auto.evolve`. The template id `jdbc-sink` in `src/handlers.rs` is misleading on this point.

## Consequences

Positive:

- New JDBC-capable engines need a Type 4 driver jar in `cdc-jdbc-admin`, not a new Rust crate.
- The orchestrator never handles CDC record volume, so it stays a small stateless control process.
- Connect gives offsets, restart/resume, and schema evolution for free.
- jdbc-admin is optional at startup; the orchestrator boots without it and degrades to native drivers.

Negative / accepted costs:

- Two connection paths when jdbc-admin is unset (native fallback) vs set (Type 4).
- Control-plane operations inherit jdbc-admin availability.
- Debugging spans orchestrator, jdbc-admin, and Connect.

## Known inconsistencies

### 1. Connector fork provenance (resolved)

The Confluent JDBC **source** connector used for `trigger` / `incremental` capture
is vendored at repo-root [`kafka-connect-jdbc/`](../../kafka-connect-jdbc/) (upstream
**v10.9.7** plus the Oracle `timezoneAsRegion=false` patch). The `cdc-connect` image
compiles that tree in [`docker/connect/Dockerfile`](../../docker/connect/Dockerfile);
there is no prebuilt `kafka-connect-jdbc-local.jar`.

The older sibling tree `kafka-connect-jdbc-10.9.2` (`DebeziumEnvelopeBuilder`,
`SchemaPublisher`, `SchemaReader`) is **not** in this repo and is not shipped.

## Alternatives considered

**Native Rust drivers for everything.** Rejected: no viable Oracle/DB2/AS400/Informix crates. Would mean maintaining FFI bindings against vendor C clients.

**Keep Hyper DataService as the JDBC gateway.** Rejected for CDC: that product also carries keystore, MyBatis, and txs. CDC only needs Type 4 catalog/DDL/preview.

**Same-container sidecar inside cdc-connect + kafka-connect-jdbc dialects.** Rejected: couples admin to Connect lifecycle/heap and pulls Kafka Connect APIs in for `DatabaseMetaData` work.

**Kafka Connect tasks for control-plane metadata.** Rejected: Connect has no synchronous query API.

**DataService in the replication path.** Rejected: no offset management, no backpressure, no resume semantics.

## References

- `AGENTS.md` — DataService and CDC agent rules
- `docs/architecture/hyper-cdc-orchestrator-design.html` — full design doc
- `docs/roadmap-trigger-cdc.md` — trigger capture mode roadmap
- `src/connector_catalog.rs` — all control-plane database access
- `src/reconcile.rs` — connector config compilation
