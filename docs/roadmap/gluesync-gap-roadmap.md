# Roadmap — closing the gap with GlueSync (MOLO17)

Status: **Plan only — not scheduled.** Each item gets its own ADR and plan before work starts.
Owner: Wutichai · Written: 2026-11-12

## Why

Customers compare hyper-cdc-orchestrator with GlueSync. This page lists what GlueSync
offers, where we stand (Built / Partial / Missing, with evidence in this repo), and the order
in which to close the gaps. Agreed priority after the credibility fixes: **sources & targets**.

## What GlueSync offers (public material, 2.x)

- Agent per source/target: IBM i (Db2 for i journal API), four Oracle capture methods
  (LogMiner, XStream, triggers, …), SQL Server, MySQL, PostgreSQL WAL, and NoSQL
  (Couchbase, Aerospike, DynamoDB, MongoDB …); snapshot + change capture.
- Core Hub control plane and REST admin API.
- Bidirectional replication with conflict resolution, mainly RDBMS ↔ Couchbase.
- Monitoring: Control Plane UI, Prometheus metrics, ready-made Grafana dashboards with read
  and write latency per pipeline; latency / stall alerting (rebuilt in 2.2.8).

Sources: [Monitoring (v2.1)](https://docs.molo17.com/gluesync/v2.1/alerting-logging-monitoring/monitoring.html) ·
[Agents 101](https://docs.molo17.com/gluesync/v2.0/agents/agents-101.html) ·
[Core Hub](https://docs.molo17.com/gluesync/v2.1/latest-releases/gluesync-core-hub.html) ·
[CDC streaming](https://molo17.com/solutions/cdc-streaming/) ·
[Database offload](https://molo17.com/solutions/database-offload/) ·
[2.2.8 alerting](https://molo17.com/blog/gluesync-2-2-8-smart-alerting-rebuilt-ota-updates-and-rich-technical-metadata/index.md) ·
[IBM i source (v1.4)](https://docs.molo17.com/gluesync/v1.4/sql-to-nosql-source-ibmi-as400/index.html) ·
[Bidirectional with Couchbase](https://www.couchbase.com/blog/molo17-gluesync-bidirectional-data-replication)

Public material does not describe GlueSync's conflict rules in detail; confirm before claiming parity.

## Where we are stronger

- **Verify:** record diff source vs target by key, with CSV/NDJSON export (ADR 0013).
- **Start position:** snapshot-then-stream / latest / earliest / custom LSN-binlog-SCN, plus reposition (ADR 0014).
- **Schema compare** and re-runnable target DDL for 4 engines (ADR 0003).
- **Copy & Sync toolbox:** pgcopydb, pg_dump, pgloader, mongocopy, redis-shake, rsync (ADRs 0005–0007, 0009, 0012).
- **WireGuard site tunnels** to reach private sites (ADR 0010).
- **MCP server** for AI-assisted operations (ADR 0008).
- **Notifications:** webhook (signed), email, LINE, Slack, Discord and Teams, with a delivery log (ADR 0015).
- **Per-pipeline latency:** estimate everywhere, exact with a heartbeat (in progress).
- **Open stack:** Kafka Connect, Debezium, Redpanda; no per-agent licence.

## Gap table

| Area | GlueSync | Us | Evidence |
|---|---|---|---|
| Postgres log CDC | yes | **Built** (verified) | `src/reconcile.rs:183` |
| MySQL / SQL Server / Oracle log CDC | yes | **Partial**: connectors don't start (schema history unset) | ADR 0014 "Not verified" |
| IBM i journal CDC | yes (flagship) | **Missing** | design doc Ch.5.1 / Ch.21 |
| Oracle alt. methods (XStream, triggers) | yes | **Partial** (trigger via jdbc-admin) | `src/trigger_cdc/mod.rs` |
| Db2 LUW / Informix | partly | **Partial** (SQL generated, manual install) | `src/trigger_cdc/mod.rs` |
| NoSQL targets (Couchbase, MongoDB) | yes | **Missing** (Mongo only via copy sidecar) | `reconcile.rs:435-448` JDBC only |
| Warehouse / lake targets | yes | **Partial** (S3/Iceberg wired, pattern deferred) | `src/lake.rs` |
| Schema evolution | yes | **Missing** | design doc Ch.5.1 |
| Row filters / masking | yes | **Partial** (`mask_pii` = ReplaceField) | `src/handlers.rs:3643` |
| Bidirectional + conflicts | yes | **Missing** | — |
| Latency per pipeline | yes | **In progress** (plan Part 2) | — |
| Prometheus + Grafana dashboards | yes | **Partial** (`/metrics`, no dashboards/JMX) | `src/ops.rs:609` |
| Alerting | yes | **Built** | ADR 0015 |
| HA / multi-instance | yes | **Missing** (single instance, in-memory watcher state) | design doc Ch.21 |
| Kubernetes / Helm | yes | **Missing** (`engine.rs` K8s stub) | `src/engine.rs:31-57` |
| RBAC | yes | **Missing** | design doc Ch.14.5 |

## Order of work

### R0 — Credibility fixes (before any comparison claim)
- B-01: the sink writes into the wrong schema.
- B-02: deleting a profile orphans the replication slot, so WAL grows.
- B-03: up to 5 minutes before first data arrives (`docs/testing/api-test-plan.md:61-77`).
- Set `schema.history.internal.kafka.*` so MySQL, SQL Server and Oracle sources start, then live-verify each one.

### R1 — Sources & targets (agreed focus)
1. **IBM i journal CDC** via upstream `debezium-connector-ibmi` (`As400RpcConnector`; the
   `debezium-connector-ibmi-main` copy in the workspace is deprecated).
   - Add it to `docker/connect/Dockerfile`.
   - Add `source_type=db2i` in `src/models.rs` and a config builder in `reconcile.rs`.
   - Add a journaling/permissions preflight.
   - Live test on PUB400 or a customer LPAR.
2. **Oracle parity:** verify LogMiner live, harden the trigger path, document XStream as later.
3. **Db2 LUW / Informix:** install triggers automatically instead of generating SQL for a person to run.
4. **Targets beyond JDBC:**
   - MongoDB and Couchbase. Couchbase is GlueSync's flagship pair.
   - Snowflake and ClickHouse.
   - Kafka/stream: finish `source_to_stream`.
   - S3/Iceberg: finish `source_to_lake`.
5. **Schema evolution:** propagate ADD/ALTER COLUMN, using the sink's `schema.evolution` plus a guarded DDL path that reuses the target DDL generator. Write a runbook.
6. **Declarative row filters and column masking** per table (hash, redact, null).

### R2 — Observability
- Grafana dashboards and a Prometheus scrape config in `docker/`, under the ops profile.
- Connect JMX exporter, for Debezium `MilliSecondsBehindSource` and the sink task metrics.
- Stall detection as an alert condition: no progress while the source has changes.

### R3 — Enterprise operations
- HA: leader lease in rqlite for the watcher and sampler, shared alert state, 3-node rqlite.
- Helm chart and the Kubernetes engine provider.
- RBAC (viewer, operator, admin) mapped to SSO groups; hashed local passwords.
- Docker socket proxy; request rate and body-size limits.

### R4 — Differentiators and bidirectional
- Bidirectional sync: origin tagging for loop prevention, and a conflict policy (last writer wins or source priority).
  Start with Postgres ↔ Postgres and IBM i ↔ Postgres.
- Auto-repair from record diff: generate and apply fix batches.
- AI operations through MCP, for questions like "why is pipeline X slow?" (latency plus diagnose).
