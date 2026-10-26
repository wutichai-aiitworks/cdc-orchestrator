# Roadmap — Trigger-Based CDC

Status: Draft (planning only — nothing implemented from this document yet)
Owner: TBD
Last updated: 2026-08-29

## Why this roadmap

Many customer source databases **cannot enable log-based CDC** (PostgreSQL `wal_level=replica|minimal`, MySQL without binlog, SQL Server without CDC/agent, Oracle without LogMiner privileges). For these, `hyper-cdc-orchestrator` already ships a **trigger-based capture mode** (`capture_mode=trigger`) as an alternative to WAL/binlog/journal decoding.

This document captures the **current state**, the **known gaps**, and a **prioritized plan** to make trigger-based CDC a fully productized replica path.

### Target customer scenario (drives current priorities)

- **Sources:** PostgreSQL, Oracle, SQL Server — production databases **without** WAL / archived logs / journal, so **triggers are the only capture option**.
- **Targets:** **all PostgreSQL** — `postgres → postgres`, `oracle → postgres`, `mssql → postgres`.
- **Sync profile:** incremental, **configurable poll interval** (infrequent by design) to minimize load on production sources ("smart sync", micro-batch).

Because every target is PostgreSQL, the existing Postgres-only sink + apply path covers all three pipelines, and **non-Postgres sink work (Phase 2) is out of scope** for this customer set.

For **large tables** that do not need hard-delete capture, prefer `capture_mode=incremental` instead of installing triggers: JDBC polls `updated_at` / `id` on the live tables with no source DDL and no write amplification. See `src/incremental_cdc.rs`.

---

## Current state (as-is)

Trigger CDC is implemented end-to-end for the **capture** and **landing** stages, but the **apply** stage and multi-vendor support are incomplete.

Flow today:

```
Source tables
  → CREATE TRIGGER (AFTER INSERT/UPDATE/DELETE)
  → changelog table (hyper_cdc.change_log)
  → Confluent JdbcSourceConnector (mode=incrementing on id)
  → Redpanda (Avro)
  → Debezium JdbcSinkConnector
  → target hyper_cdc.inbox    ← STOPS HERE (no apply into real tables)
```

| Stage | Status | Location |
|-------|--------|----------|
| Source trigger DDL generation | Done (8 vendors) | `src/trigger_cdc/mod.rs`, `src/trigger_cdc/sql.rs`, `src/trigger_cdc/exec.rs` |
| Native trigger install | Done: Postgres, MySQL/MariaDB, SQL Server, SQLite | `src/postgres_trigger_cdc.rs`, `src/trigger_cdc/exec.rs` |
| SQL-only trigger DDL (operator applies) | Done: Oracle, DB2, Informix | `src/trigger_cdc/mod.rs` (`Vendor::native_install`) |
| Changelog JDBC source connector | Done | `src/postgres_trigger_cdc.rs:245` (`build_jdbc_source_config`) |
| Reconcile switch (skip Debezium) | Done | `src/reconcile.rs:600` |
| Sink → inbox | Done, **Postgres target only** | `src/postgres_trigger_cdc.rs:284` (`build_trigger_sink_config`) |
| Inbox → real tables (apply) | **Not wired** (code exists, unit-test only) | `src/postgres_trigger_cdc.rs:156` (`target_apply_sql`) |
| Trigger APIs (preview/install/status) | Done (source side) | `src/handlers.rs:1731+`, `docs/openapi.yaml:1161` |
| Profile source/target type allow-list | postgres/mysql/mariadb/sqlserver/oracle only | `src/handlers.rs:2536` (`is_supported_database_type`) |
| MCP trigger support | **Missing** (`cdc_pipeline_create` never sets trigger mode) | `mcp/src/tools.rs` |

### Known gaps (blocking full productization)

1. **Apply step not wired** — changes land in `hyper_cdc.inbox` and stop; real target tables are never updated. `target_apply_sql()` exists but is only exercised by a unit test. (Target is always Postgres → this one fix covers all three pipelines.)
2. **Poll interval hardcoded** — `build_jdbc_source_config` sets `poll.interval.ms=1000` (1s). The `ScheduleConfig { delivery_mode, interval_seconds }` model exists but is **not wired** into the JDBC source, so "infrequent, configurable sync" isn't possible yet.
3. **Oracle JDBC driver missing** — `docker/connect/jdbc-pom.xml` bundles Postgres/MySQL/MSSQL/SQLite but **not `ojdbc`**, so the JDBC source cannot poll an Oracle changelog.
4. **Oracle→Postgres JSON key casing** — **Done** (apply-side lowercase keys + Oracle `JSON_OBJECT` KEY literals).
5. **No changelog retention** — **Done** (opt-in `changelog_retention_hours`; Oracle prune is DBA-scheduled).
6. **Sink is Postgres-only** — not a blocker for this customer set (all targets Postgres); relevant only if non-Postgres targets are added later.
7. **Source/target allow-list too narrow** — SQLite/DB2/Informix trigger DDL exists but those types can't be saved as a profile source. (Not needed for PG/Oracle/MSSQL.)
8. **No MCP/prompt path** — trigger workflows are UI/API only.
9. **No trigger-mode snapshot/backfill** — rows written before triggers are installed are not captured.

---

## Prioritized roadmap

Effort key: S (≤1d) · M (2–4d) · L (1–2wk), relative estimates.

> Priority reflects the target customer scenario (PG/Oracle/MSSQL → **Postgres**, interval-based sync). The critical path is **Phase 0 → Phase 1 → Phase 1b**. Phases 2, 3, 4 are deferred/optional for this customer set.

### Phase 0 — Foundations, guardrails & Oracle enablement · S · (critical path)

Small changes that de-risk the rest.

- **0.1** Reject unsupported trigger combos early with actionable errors (e.g. non-Postgres target while sink is Postgres-only). — `src/handlers.rs` (validate profile ~2199), `src/reconcile.rs`.
- **0.2** Surface `capture_mode` + resolved vendor in the profile validation summary for UI/MCP display. — `src/handlers.rs`.
- **0.3** Per-vendor unit tests for `build_jdbc_source_config` / `build_trigger_sink_config`.
- **0.4** Add the **Oracle JDBC driver** (`com.oracle.database.jdbc:ojdbc11`) to `docker/connect/jdbc-pom.xml` so the JDBC source can poll an Oracle changelog. (Blocker for the Oracle source.)

Acceptance: invalid trigger profiles fail fast with clear messages; per-vendor tests pass; Connect image contains `ojdbc`.

### Phase 1 — Complete the apply step · M–L · (HIGHEST priority)

Without this, trigger CDC is not a usable replica — data stops at the inbox. **Target is always Postgres, so this single fix covers all three pipelines (PG/Oracle/MSSQL → Postgres).**

- **1.1** Wire `target_apply_sql()` into the trigger-profile lifecycle using approach **(A) DB-side apply trigger** — install the inbox table + `apply_inbox()` BEFORE-INSERT trigger on the Postgres target at start; inbox inserts upsert/delete into real tables. Reuses existing, tested code. — `src/reconcile.rs` (`reconcile_start`, before sink upsert), `src/postgres_trigger_cdc.rs:156`.
- **1.2** Resolve **target PK columns** for each captured table (reuse `connector_catalog::fill_primary_keys`) and pass them into `target_apply_sql` so upsert/delete predicates are generated.
- **1.3** Verify DELETE propagation end-to-end (`op='d'` → inbox → apply trigger delete).
- **1.4** (Oracle source) Normalize **JSON key casing** so Oracle's UPPERCASE `JSON_OBJECT` keys map to Postgres column names in `jsonb_populate_record` (lowercase keys, or explicit column mapping). — `src/trigger_cdc/sql.rs` (`oracle_json`) or apply-side normalization. **Done:** apply-side `jsonb_object_agg(lower(kv.key))` plus source-side lowercase `KEY` in `oracle_json`.

Acceptance: `postgres → postgres`, `oracle → postgres`, `mssql → postgres` trigger pipelines replicate INSERT/UPDATE/DELETE into real target tables in the demo stack.

### Phase 1b — Smart interval-based sync · S · (critical path)

Customer requirement: infrequent, configurable sync to protect production sources.

- **1b.1** Wire `flow.schedule.interval_seconds` (when `delivery_mode = micro_batch`) → `poll.interval.ms` in `build_jdbc_source_config`; keep 1s default for `realtime`. — `src/postgres_trigger_cdc.rs:278`.
- **1b.2** Add a bounded `batch.max.rows` so a large backlog drains in predictable chunks (avoid load spikes after long intervals).
- **1b.3** (Optional) expose these in the profile validation summary / UI.

Acceptance: a micro-batch trigger profile with `interval_seconds=300` produces a JDBC source with `poll.interval.ms=300000`; realtime stays at 1s.

### Phase 1c — Changelog retention (production-disk safety) · S–M · (pulled forward) — **Done**

- **1c.1** Prune the source changelog by age (`changed_at`) so production disk doesn't grow unbounded. Native prune for Postgres, MySQL/MariaDB, SQL Server, SQLite. Oracle/DB2/Informix: generated DELETE is logged for a DBA job (`trigger_cdc::prune_sql`). Only **running** trigger pipelines are pruned so paused pipelines keep unconsumed events.
- **1c.2** Retention window is `source.changelog_retention_hours` (omit/`0` = disabled). Sweep cadence: `ORCH_CHANGELOG_PRUNE_INTERVAL_SECS` (default 3600).

Acceptance: set `changelog_retention_hours` on a running trigger profile; aged changelog rows are deleted. Choose a window larger than expected lag.

### Phase 2 — Generalize the trigger sink beyond PostgreSQL · L · (DEFERRED — not needed while all targets are Postgres)

- **2.1** Parameterize `build_trigger_sink_config` JDBC URL + dialect by `target_type`. — `src/postgres_trigger_cdc.rs:307`.
- **2.2** Port inbox schema + apply logic to MySQL/MariaDB and SQL Server (Oracle later).
- **2.3** Fallback: inbox-only "staging only" mode for targets without a native apply path.

### Phase 3 — Expand supported source/target types · S–M · (DEFERRED — PG/Oracle/MSSQL already allowed)

- **3.1** Split `is_supported_database_type` into logical vs trigger-source variants; allow SQLite/DB2/Informix when `capture_mode=trigger`. — `src/handlers.rs:2536`.
- **3.2** Ensure catalog/probe handles new types or degrades gracefully.

### Phase 4 — MCP + UX parity · M · (optional)

- **4.1** Extend `cdc_pipeline_create` with `capture_mode` + optional `install_triggers` (behind `ALLOW_WRITE_TOOLS`). — `mcp/src/tools.rs`.
- **4.2** Add read tools: `cdc_trigger_status`, `cdc_trigger_preview`.
- **4.3** UI: target-apply install step + interval selector. — `kafbat-webui/pages/pipelines.vue`.

### Phase 5 — Further operational hardening · M

- **5.1** Trigger-mode snapshot/backfill (initial load before triggers go live).
- **5.2** Poll-lag metrics + alerts for the JDBC source. — `src/watcher/`.
- **5.3** Docs: update `docs/architecture/hyper-cdc-orchestrator-design.html` Ch.6 + `docs/openapi.yaml`; fix §7.3 create-vs-list wording.

---

## Sequencing & dependencies

```
Phase 0 ─▶ Phase 1 ─▶ Phase 1b ─▶ Phase 1c   (critical path for PG/Oracle/MSSQL → Postgres)
                 └────────────────▶ Phase 5 (hardening, in parallel)

Phase 2 / 3 / 4  = deferred / optional for this customer set
```

- Phase 0 → Phase 1 → Phase 1b is the critical path (trigger CDC isn't a usable, low-impact replica until these land).
- Phase 1c runs right after (or alongside) Phase 1.
- Phases 2, 3, 4 are deferred while all targets are Postgres.

## Resolved decisions (from customer requirements)

1. **Targets:** all PostgreSQL (`postgres/oracle/mssql → postgres`) → Phase 2 deferred.
2. **Apply strategy:** **1.1-A DB-side apply trigger** (recommended, reuses existing code).
3. **Sync cadence:** micro-batch with configurable `interval_seconds` (Phase 1b).

## Reference: key source locations

| Concern | Path |
|---------|------|
| Capture mode enum | `src/models.rs:550` (`CaptureMode`) |
| Reconcile switch | `src/reconcile.rs:600`, sink branch `:721` |
| JDBC source (capture) | `src/postgres_trigger_cdc.rs:245` |
| Trigger sink (landing) | `src/postgres_trigger_cdc.rs:284` |
| Apply SQL (unwired) | `src/postgres_trigger_cdc.rs:156` |
| Vendor matrix | `src/trigger_cdc/mod.rs` |
| Source trigger handlers | `src/handlers.rs:1731+` |
| Type allow-list | `src/handlers.rs:2536` |
| Trigger APIs (OpenAPI) | `docs/openapi.yaml:1161` |
| MCP tools | `mcp/src/tools.rs` |
| UI | `kafbat-webui/pages/pipelines.vue`, `kafbat-webui/components/TriggerPreviewDialog.vue` |
