# ADR 0013 — Record diff: source vs target by key, per table, in the background

Status: Accepted (built; verified against the demo Postgres pair, not yet against Oracle)
Owner: Wutichai
Last updated: 2026-11-12

## Context

Row counts (`/tables/sync-status`) say *that* a replica drifted, not *which* rows. Operators
need, per pipeline table: rows present only in the source (target is missing them), rows
present only in the target (source deleted them, or they were never captured), and
optionally rows on both sides whose values differ. The tables can be millions of rows, on
different engines (Oracle → Postgres is the main case), while the pipeline is live.

## Decision

1. **One background job per requested table**: `POST /cdc/profiles/{id}/record-diff`
   with `{table, key_columns?, compare_columns? | compare_all_columns?, options}` → 202
   with a job id. Status / list / cancel / delete / rows page / download (NDJSON or CSV).
   One active job per (pipeline, table). `ORCH_RECORD_DIFF_CONCURRENCY` jobs run at once
   (default 2), the rest queue.
2. **Key resolution**: request (source names) → mapping `pk_columns` (target names, mapped
   back) → source PK → target PK; else 400. Keys must reach the target untransformed.
   Columns map through the pipeline's column mapping when it renames, else by
   case-insensitive name (covers Oracle → Postgres folding).
3. **Read once, hash-partition, compare per bucket.** Each side is read with one
   forward-only `SELECT key, compared… FROM t` through jdbc-admin `/v1/stream`
   (new `columns` projection), both sides concurrently. Every row's key is hashed into
   one of N bucket files (N from the row-count estimate, 200k rows/bucket, 16–512) on the
   orchestrator's disk (`ORCH_RECORD_DIFF_DIR`, default `<tmp>/cdc-record-diff`).
   Buckets are then compared one at a time in memory. Memory is bounded by bucket size;
   disk use is about one projected copy of both tables, freed bucket by bucket.
4. **Canonical values from jdbc-admin** (`/v1/stream` `canonical: true`): every value
   becomes an engine-neutral string where the JDBC type is known — plain decimals
   without trailing zeros (`10.50` ≡ `10.500` ≡ `1.05E+1`), ISO local timestamps, UTC
   instants for zoned timestamps, ISO dates/times, CLOB text, `sha256:` of BLOB/binary.
   The orchestrator then applies the request options: ignore trailing spaces (default on,
   `CHAR` padding) and case-insensitive (default off).
5. **Recheck by key.** A live pipeline changes both sides during a scan. Each candidate
   difference (first `recheck_limit` per kind, default 10 000) is re-read on both sides
   via the new jdbc-admin `/v1/lookup` (prepared `WHERE (k1=? AND k2=?) OR …`, 200 keys
   per statement, keys bound with the column's own JDBC type so the index is used).
   Rows that converged are dropped and counted as `resolved_in_flight`; the rest are
   `confirmed`. Over the limit (or recheck off) they are reported as `unverified`.
6. **Results on disk, summary survives restarts.** `source_only.ndjson`,
   `target_only.ndjson`, `mismatched.ndjson` (`{key, status, values? | differences?}`)
   plus `summary.json` per job. On startup finished jobs are reloaded; a job that was
   running is marked failed. The newest 20 finished jobs per pipeline are kept.
7. Duplicate keys (key not unique on a side) and NULL key components are counted and
   skipped, never reported as differences. Read-only on both databases. Audit:
   `profile.record_diff_start`, `profile.record_diff_cancel`, `profile.record_diff`.

## Rejected options

- **Sorted merge with `ORDER BY key` on both sides.** Streams in O(1) memory, but
  Oracle and Postgres collate text differently (`NLS_SORT`, ICU vs C), so a merge join
  across engines walks out of step and reports false differences. It also makes each
  database sort the whole table.
- **One in-memory `HashSet` of all keys.** Simple, but unbounded memory: tens of millions
  of composite keys can exhaust the orchestrator.
- **Per-chunk checksums (`md5` of ranges) pushed down to SQL.** Fast when few rows
  differ, but the hash and text casts differ per engine, so cross-engine values never
  agree. Worth revisiting for same-engine pairs.
- **Comparing raw JSON values in Rust.** `serde_json` without `arbitrary_precision`
  turns `NUMBER(38)` keys into `f64` and loses digits; normalising in jdbc-admin, where
  the real JDBC type is known, avoids that.

## Consequences

- Needs a rebuilt `cdc-jdbc-admin` image (`columns`, `canonical`, `/v1/lookup`). Against
  an old image the scan fails with "expected N columns — is cdc-jdbc-admin up to date?".
- In Docker the default directory is inside the orchestrator container. Mount a volume
  at `ORCH_RECORD_DIFF_DIR` to keep results across container recreation, sized for a
  projected copy of the largest table you diff.
- Known value-comparison noise (keys are unaffected): a Postgres `date` column vs an
  Oracle `DATE` holding midnight (`2024-01-02` vs `2024-01-02T00:00`), and booleans vs
  `NUMBER(1)`. Compare such columns only if you expect them to agree in form.
- Job state is per orchestrator instance (like initial load).

## Revisit when

- Same-engine pairs need diffs of very large, mostly-equal tables faster than a full read
  (chunk checksums, see above).
- Users want a **repair** step (insert missing / delete extra rows from the result).
