# ADR 0019 — Live CDC readiness check and database versions

Status: Accepted
Owner: TBD
Last updated: 2026-11-14

## Context

The pipeline wizard's readiness panel (`SourceCdcReadiness.vue`, `utils/sourceReadiness.ts`) is a
static guide: recommended settings and SQL for the operator to copy and run. Most failed first Starts
come from things the source could have reported itself: `wal_level` is not `logical`, the publication
is missing (the connector runs with `publication.autocreate.mode = disabled`), the login lacks
REPLICATION, the binlog is not ROW/FULL, SQL Server CDC or Agent is off, Oracle is not in ARCHIVELOG.
Operators also asked to see the database version of both source and target.

## Decision

1. **Facts in jdbc-admin, rules in the orchestrator.** jdbc-admin gets a generic `/v1/probe`: named
   statements, SELECT-only (`requireSelect`), on one read-only connection, with a 10 s timeout and 300 rows
   each, at most 40 per request. A failing probe returns its error and SQLSTATE, and the rest still run.
   jdbc-admin has no engine knowledge. `src/source_readiness.rs` holds the probe SQL and the rules per
   engine, unit-tested with hand-written probe results. A new engine or rule touches only Rust.
2. **Endpoint** `POST /cdc/profiles/catalog/source/readiness`: same body as `validate/source`
   (source config + `profile_id`, stored password merged). With `profile_id` it also knows the
   pipeline's slot name. It is read-only and never changes the source.
3. **Statuses.** `pass` / `warn` / `fail`: the value was read. `unknown`: the login may not read it
   (missing privilege, cloud-restricted view); the detail names the grant that would reveal it. Unknown
   never blocks `ready` and is never guessed. `info`: worth knowing (unbounded slot WAL, GTID off).
4. **Fixes per platform.** The platform is detected from what only it exposes: Postgres
   `rds.logical_replication` / `aurora_version()` / `cloudsql.*` / `azure.*`; MySQL `@@basedir`
   (`rdsdbbin`) / `@@aurora_version`; SQL Server `EngineEdition` 5/8/9. A fix is SQL (or console steps)
   for that platform, e.g. a parameter group change instead of `ALTER SYSTEM` on RDS. All fix and warn
   items are joined into `fix_script` with a "review with your DBA" header.
5. **Coverage (log-based capture).**
   - PostgreSQL: version ≥ 10, primary (or 16+ standby), `wal_level`, free slot and WAL sender, inactive
     slots holding > 1 GB WAL, `max_slot_wal_keep_size`, REPLICATION login (or cloud role), the
     publication exists and covers the tables and all DML, the tables exist, SELECT, replica identity
     for tables without a PK (it blocks the application's UPDATE/DELETE), this pipeline's slot.
   - MySQL/MariaDB: `log_bin`, `binlog_format=ROW`, `binlog_row_image=FULL`, `server_id`, retention
     (RDS `binlog retention hours`), GTID, global grants (warn only, since role grants are invisible),
     InnoDB, primary keys.
   - SQL Server: edition (Express fails, Standard < 2016 fails), database CDC, SQL Server Agent (Azure
     SQL DB has none), change-table access, per-table capture instance, primary keys.
   - Oracle: ARCHIVELOG, minimal and per-table ALL COLUMNS supplemental logging, database role,
     LogMiner/dictionary/flashback/CREATE TABLE (and SET CONTAINER in a CDB) privileges, redo size.
   Trigger, incremental and sync-once capture, and trigger-only engines, return `applicable: false`
   with a note.
6. **Database version on both sides.** jdbc-admin `/v1/test` adds `server {product, version, major,
   minor, driver}` from `DatabaseMetaData`, which any login can read. `validate/source` and
   `validate/target` return it as `ConnectionCheckResponse.server` (other tools keep
   `ValidationResponse`). The UI formats it per engine (`utils/cdcReadiness.ts`: "SQL Server 2022
   (16.0.4135)", "MariaDB 10.11.6", "Oracle Database 19c Enterprise Edition").
7. **UI:** a "Check source" button in the readiness panel (visible while the guide is collapsed).
   Results show problems first with their fix, then passed checks, the system parameters read, and a
   copyable full script. The source and target version appear under Test Connection.

8. **Versions kept per pipeline and refreshed.** `src/db_versions.rs` stores the last-read source and
   target version per pipeline in rqlite (kind `db_version`). It is a separate record from the profile,
   so a refresh never races a save or a lifecycle action. Refreshes happen in the background every
   `ORCH_DB_VERSION_REFRESH_SECS` (default 3600, minimum 60), right after create/save, on every Test
   Connection of a saved pipeline, and from the refresh button (`POST /cdc/profiles/{id}/db-versions/refresh`).
   A different version replaces the stored one; `previous` and `changed_at` only feed a tooltip and an
   "updated" mark for 7 days, with no alert. A failed check keeps the last known version and records
   `error`. The pipeline list proxy attaches `db_versions` to `/api/cdc/profiles`, so the cards and the
   detail page update on the existing poll without another route.

## Consequences

- The background refresh opens one short metadata-only connection per pipeline side per interval
  (the same login the pipeline uses). Raise `ORCH_DB_VERSION_REFRESH_SECS` for many pipelines.
- jdbc-admin must be rebuilt for both features; older images return no `server` and a 404 on
  `/v1/probe` (the check reports the error).
- The probes run as the pipeline's own login, so a least-privilege login sees more `unknown` checks.
  That is intended: it shows exactly what a DBA must confirm.
- The proxy routes for `catalog/source/table` and `.../readiness` share one Nitro file
  (`[action].post.ts`). One more typed route pushed `useFetch` in `app.vue` past TypeScript's
  instantiation depth (TS2589).

## Rejected

- **Probe SQL in Java per engine:** rules would be split across two languages and two images.
- **Orchestrator calls `/v1/query` once per probe:** one connection per statement (slow on Oracle) and
  no per-probe error isolation.
- **Running fixes from the UI:** they need DBA rights and often a restart. The check stays read-only.
