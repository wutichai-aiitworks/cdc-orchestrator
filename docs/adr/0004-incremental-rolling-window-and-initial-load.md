# ADR 0004 — Rolling-window / full-copy polling and streaming initial load

Status: Accepted (built, not yet run against a live source)
Owner: Wutichai
Last updated: 2026-11-02

## Context

An Oracle production source (`PANDORA.PM_PRODUCT`, ~3.2M rows; `CRM_STG.PM_PROBRAND`,
~300 rows) must be replicated to PostgreSQL with **no changes on the source**: no
triggers, views, indexes, supplemental logging or LogMiner grants. Only read-only
polling is possible.

The JDBC watermark (`mode=timestamp` on `UPDATE_DATE`) cannot replicate it correctly:

- `UPDATE_DATE` / `INSERT_DATE` are **date-only** (always `00:00:00`); the time lives
  in a separate `UPDATE_TIME` text column. After a poll stores "today 00:00" as its
  offset, every later change made the same day has the same value, is not `>` the
  offset, and is **silently skipped forever**.
- ~1.2M legacy rows have a NULL `UPDATE_DATE` (old design: insert-only rows used
  `INSERT_DATE`). `ts > ?` never matches NULL. New rows always set `UPDATE_DATE`.
- Both columns are nullable, so `validate.non.null=true` refuses to start the task.

Separately, the existing direct copy (test initial sync / trigger resync) paged through
jdbc-admin `/v1/preview`, which re-runs `SELECT *` and skips `page × size` rows per page
— quadratic, ~16k ever-longer queries for 3.2M rows. Unsafe on production.

## Decision

1. **Per-table poll strategy** on incremental pipelines: `source.table_poll[]` of
   `{table, strategy, window_column?, window_days?}`. Strategies:
   - `watermark` (default, unchanged behaviour);
   - `rolling_window`: every poll re-reads `WHERE col >= <start of today − N days>`
     (dialect-specific; bare column vs constant so an index on it stays usable).
     No offset is kept, so date-only / non-unique timestamps cannot skip changes;
     the sink upsert on `upsert_key_columns` makes re-reads idempotent. Default N=1
     (today + yesterday) covers midnight; max 31.
   - `full_copy`: every poll re-reads the whole table (small tables only).
2. **One connector per custom table.** Kafka Connect JDBC query mode takes one query
   per connector, so each rolling-window / full-copy table runs a query-mode,
   `mode=bulk` source named `{connector}-poll-{schema-table}`. Its `topic.prefix` is
   the exact table-mode topic (`{prefix}.{TABLE}`), so the sink, table status and
   topic tooling are unchanged. The **first** connector of the plan always takes the
   pipeline connector name (the watermark connector when any table uses one), so
   status, pause, stop and the watcher keep a single primary. Extra connectors are
   discovered from Connect by name prefix for pause/resume/stop/teardown/resync, and
   stale ones (strategy changed back) are deleted on Start.
3. **Streaming initial load** (`POST/GET/DELETE /cdc/profiles/{id}/initial-load`):
   a background job per pipeline that reads each table with **one** forward-only
   `SELECT *` via new jdbc-admin `POST /v1/stream` (NDJSON: columns line, row arrays,
   `{"done":true,"rows":N}` trailer; a stream without a trailer is a failure) and
   inserts batches of 2000 with `ON CONFLICT DO NOTHING`. Safe beside a running
   pipeline (fresher window upserts win) and re-runnable after failure/cancel.
   Optional `clear_target` truncates each table only after its source read is open.
   Job state is in memory per orchestrator instance; the outcome goes to profile
   history (`profile_initial_load*`) and audit (`profile.initial_load`).
4. **Guards.**
   - The paged preview copy is gone: every jdbc-admin → Postgres copy now uses
     `/v1/stream` (row-capped for test sync), so reads are linear.
   - `clear_target` Resync (and therefore the scheduled full refresh) is rejected on
     pipelines with any rolling-window table: the window never re-reads older rows.
   - jdbc-admin now serves requests on a thread pool (`setExecutor(null)` ran every
     request on one thread, so a long stream would have blocked catalog/preview/count).
   - The orchestrator streams with a separate HTTP client: no total timeout, 15 s
     connect, 600 s per-read.

Recommended setup for the PANDORA case: `PM_PRODUCT` → `rolling_window` on
`UPDATE_DATE`, 1 day; `PM_PROBRAND` → `full_copy`; upsert key `BRANCH_NO, PRO_CODE`;
one initial load off-peak; poll interval 5 min if `UPDATE_DATE` is indexed, else
15–30 min.

## Consequences

- Hard deletes are still not captured by any strategy. `PRO_CANCELDATE` suggests soft
  deletes; if physical deletes happen, a keys-only reconcile is the next step (not built).
- Rows with **both** dates NULL are reached only by the initial load.
- Bulk connectors re-publish their window every poll: topic volume grows with
  (rows changed in window × polls/day); the table panel hides "Processing" for them.
- If the pipeline is down longer than the window, re-run the initial load (or widen
  the window temporarily).
- A long Oracle read can hit `ORA-01555 snapshot too old` on a busy table; the load
  reports it, keeps copied rows, and can be re-run.
- Initial-load job state is lost on orchestrator restart (the copy stops with it; the
  rows already copied stay). Multi-instance deployments must route the three
  initial-load calls to the same instance.

## Rejected

- `timestamp.columns.mapping=[UPDATE_DATE|INSERT_DATE]` (COALESCE): fixes NULLs only;
  date-only values still skip same-day changes; still fails `validate.non.null`.
- Query-mode timestamp on `UPDATE_DATE + UPDATE_TIME`: needs a computed expression
  (no index → full scan every poll), trusts a free-text time column, and still misses
  late-committing transactions.
- Triggers, views, LogMiner: require changing the production source.
- Initial load as a one-shot bulk Connect connector: no completion signal, and any
  worker restart would re-read the whole table from production.
