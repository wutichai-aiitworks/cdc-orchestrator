# ADR 0014 — Start position for log-based capture (LSN / binlog / SCN)

Status: Accepted (built; Postgres verified live against the demo pair; MySQL / SQL Server /
Oracle unit-tested only — see "Not verified")
Owner: Wutichai
Last updated: 2026-11-12

## Context

Every Debezium source used to start the same way: copy all rows, then stream
(`snapshot.mode=initial`). Operators also need to:

- skip the copy and stream from now (the target was loaded some other way, e.g. pg_dump);
- stream from the oldest change the source still holds;
- start at a known position — after restoring the target from a backup taken at LSN X, or
  when cutting over from another CDC tool that reports its last position;
- move a running pipeline (skip a poison range, or replay a window).

## Decision

1. **One field, dialect-neutral**: `source.start_position = {mode, position?}` with modes
   `snapshot_then_stream` (default, unchanged behaviour), `latest`, `earliest_retained`,
   `custom`. `position` is a typed `LogPosition` (`postgres_lsn`, `mysql_binlog` with
   optional `gtid_set`, `sql_server_lsn`, `oracle_scn`) whose `kind` must match the source.
   Only for `capture_mode=logical`; trigger / incremental / sync_once reject a non-default
   value (they track a changelog id or a watermark, not a log position).
2. **One-shot.** The position applies when the source connector has no stored offset (first
   Start) or when the value changes. A config marker `cdc.start_position` is written into the
   connector config; `offsets_incompatible_reason` treats a marker change like the existing
   "Sync once → ongoing" case: stop, delete offsets, re-seed. Otherwise the stored offset
   always wins, so Restart / Resume never rewind. The default writes no marker, so connectors
   created before this feature compare equal and keep their offsets.
3. **Seeding uses Kafka Connect, not Debezium options.** A new connector is created with
   `initial_state: STOPPED` (KIP-980), its offset is written with
   `PATCH /connectors/{n}/offsets` (KIP-875, Connect ≥ 3.6; the image ships 3.7), then it is
   resumed. It never runs from the wrong position first. The offset holds only the position
   fields (`{"lsn": <u64>}` for Postgres, partition `{"server": topic.prefix}`); Debezium
   fills the rest on its first commit. Verified against offsets written by live 2.7
   connectors.
4. **Snapshot mode.** `latest` → `no_data`. Seeded modes → `no_data` on Postgres (no schema
   history). MySQL / SQL Server / Oracle → `recovery`, because they cannot decode the log
   without a schema history and `recovery` rebuilds it from the current schema while keeping
   the stored offset.
5. **Validate before touching Connect** (400 with the reason):
   - Postgres: LSN ≤ `pg_current_wal_lsn()`; ≥ the slot's `confirmed_flush_lsn` when the slot
     exists; when it does not, any LSN older than now is refused, because a new slot starts
     at the current WAL position. `earliest_retained` = the slot's `confirmed_flush_lsn` and
     needs an existing slot. Postgres never streams from before `confirmed_flush_lsn`, even
     though WAL back to `restart_lsn` is still on disk, so "earliest" on Postgres really means
     "where the slot is".
   - SQL Server: within `[MAX(cdc.change_tables.start_lsn), fn_cdc_get_max_lsn()]`.
   - Oracle: within `[MIN(first_change#) of archived + online logs, v$database.current_scn]`.
   - MySQL: a `gtid_set` must contain `@@gtid_purged` (`GTID_SUBSET`). The binlog file itself
     is not checked: jdbc-admin `/v1/query` only accepts SELECT, and the file list needs
     `SHOW BINARY LOGS`. Debezium reports a missing file on its first poll. For the same
     reason `earliest_retained` (and reposition to `latest`) are 400 on MySQL for now.
   Non-Postgres checks go through jdbc-admin `/v1/query`, so jdbc-admin needs no change. The
   GTID set is checked against a strict character set before it is put into the SELECT.
6. **Reposition** (`POST /cdc/profiles/{id}/reposition {start_position, clear_target}`) moves
   a running pipeline: halt sink + source → wait STOPPED → optional truncate → DELETE offsets
   (so no stale `lsn_proc` / `lsn_commit` sits next to the new `lsn`) → PUT config with the
   new snapshot mode + marker → PATCH offset → resume. `latest` resolves to the source's
   current position. The new value is saved as `start_position`, so a later Start sees a
   matching marker and keeps the offsets. Audited as `profile.reposition`.
7. **Resync is refused** on non-default start positions: resync means "copy everything
   again", which `no_data` / `recovery` never do. The 400 points at Reposition, or at
   switching back to `snapshot_then_stream` (which resets and snapshots via rule 2).
8. **Read back**: `GET /cdc/profiles/{id}/position` returns the configured value and the
   committed position parsed from the connector offsets, in `LogPosition` form so it can be
   pasted back. It is null until Connect first flushes an offset (default 60 s).

## Consequences

- Forward moves (`latest`, a later custom position, a reposition forward) leave a **gap** in
  the target. That is the operator's call. The Verify Record tab (ADR 0013) is how you find
  out what is missing.
- Backward moves replay changes. The JDBC sink upserts by key, so a replay converges and
  does not duplicate rows.
- Postgres can only go back as far as the slot's `confirmed_flush_lsn`, which Debezium
  advances on every offset commit. Replaying a past window on Postgres therefore needs a
  slot held back on purpose; that is out of scope here.

## Not verified / follow-ups

- **MySQL / SQL Server / Oracle end-to-end.** Offset keys (`file`/`pos`/`gtids`,
  `commit_lsn` with partition `{server, database}`, `scn`) follow Debezium 2.7 source and
  are unit-tested, but no live connector of those types exists on the stack. Also, the
  profile path does not set `schema.history.internal.kafka.*` for these connectors at all,
  so they cannot start today regardless of start position. Fix that first, then check the
  seeded offsets live, including whether `recovery` should switch back to `no_data` after
  the first run.
- IBM i journal (receiver + sequence) and MongoDB resume tokens: add `LogPosition` kinds
  once those sources go through `build_debezium_config`.
- Native UI: "Start from" in the source step, current position + Reposition dialog on the
  pipeline page.

## Revisit when

- jdbc-admin gains a non-SELECT read path (MySQL earliest / binlog file check).
- Debezium changes its offset layout (major version bump of the Connect image).
