# Production CDC Hardening Plan

Status: Planning  
Deployment target: Single-host Docker Compose  
Scope: Redpanda and Kafka Connect stability, performance, retention, recovery, and connector error handling

## Objective

Harden the current CDC stack without breaking:

- Logical CDC
- Incremental capture
- Trigger capture
- Initial snapshot and resync
- Hard-delete propagation
- Existing profile JSON
- WebUI and MCP operations

This plan improves stability and bounded resource usage. A single-host, single-broker deployment is still not highly available because the host, Redpanda, Kafka Connect, and rqlite remain single failure domains.

## Compatibility rules

1. Keep `docker/docker-compose.yaml` working as it does now.
2. Put production settings in `docker/docker-compose.prod.yaml`.
3. Keep replication factor `1` while only one Redpanda broker exists.
4. Keep logical and incremental sinks fail-fast by default.
5. Keep the trigger source fail-fast.
6. Preserve the trigger sink's current DLQ behavior.
7. Do not enable finite retention until traffic and replay requirements are measured.
8. Configure Redpanda v24.2 cluster properties through validated `rpk cluster config` commands, not guessed `redpanda start` flags.
9. Never delete or recreate production volumes during rollout or rollback.

## Phase 0 — Baseline, backup, and recovery

Status: Completed on 2026-09-06. See [Production CDC Phase 0 Baseline](production-cdc-phase-0-baseline.md).

### Current-feature impact

- Read-only inventory has no impact.
- A consistent volume backup may require a maintenance window.
- Logical CDC pauses while Redpanda and Connect are stopped; source logs continue accumulating.
- Incremental capture resumes from Connect watermarks.
- Trigger capture continues writing to the source changelog, which must have enough capacity for the outage.
- WebUI and MCP are unavailable during a stopped-volume backup.

### Work

1. Record host CPU, RAM, disk, filesystem, Docker limits, and available capacity.
2. Export the rendered Compose configuration and running image digests.
3. Export profile definitions, connector configurations, connector offsets, topic configurations, consumer lag, and DLQ counts.
4. Record Redpanda throughput, topic sizes, disk growth, and under-replicated partitions.
5. Record Connect heap use, GC, task restarts, and source/sink lag.
6. Back up `cdc-redpanda-data` and `cdc-rqlite-data`.
7. Restore the backup into a separate Compose project.
8. Run baseline tests:
   - Logical: snapshot, insert, update, delete, restart, offset resume.
   - Incremental: initial load, update upsert, watermark resume, no duplicates.
   - Trigger: resync, insert, update, delete, changelog resume, target apply, DLQ.
   - Operations: WebUI, MCP reads, profile lifecycle, orchestrator restart.

### Exit gate

- Restore succeeds.
- Every current feature passes.
- Normal and peak change rates are measured.
- Source logs and trigger changelog can cover planned maintenance windows.

### Rollback

No runtime change is introduced. Restore the captured volumes and exported configurations if the recovery test identifies a problem.

## Phase 1 — Behavior-compatible production overlay

Status: Implemented and validated on 2026-09-06 without runtime activation. See [Production CDC Phase 1 Result](production-cdc-phase-1-result.md).

### Current-feature impact

- No effect when running only the base Compose file.
- Documentation and override files do not modify running containers.
- First activation of the production override may recreate services whose effective configuration changed.
- Service names, endpoints, networks, volumes, topics, offsets, and profile formats remain unchanged.

### Work

1. Add `docker/docker-compose.prod.yaml` containing only production overrides.
2. Add `docker/.env.production.example`.
3. Document shared optional settings in `docker/.env.example`.
4. Add configurable values:
   - `CDC_REDPANDA_SMP`
   - `CDC_REDPANDA_MEMORY`
   - `CDC_REDPANDA_RESERVE_MEMORY`
   - `CDC_CONNECT_HEAP_OPTS`
   - Connect batching and offset-flush values
   - Retention values, disabled until Phase 4
5. Add a read-only `docker/scripts/production-preflight.sh` that checks:
   - Docker and Compose versions
   - Host memory and disk
   - Required environment values
   - Compose rendering
   - Single-broker replication constraints
6. Compare rendered base and production configurations:

```sh
cd docker
docker compose -f docker-compose.yaml config
docker compose -f docker-compose.yaml -f docker-compose.prod.yaml config
```

### Exit gate

- The base stack remains unchanged without the override.
- Production Compose renders successfully.
- Named volumes, service names, networks, and ports are not unintentionally renamed.

### Rollback

Start the stack without `docker-compose.prod.yaml`.

## Phase 2 — Kafka Connect hardening

Status: Applied and validated on 2026-09-06. See [Production CDC Phase 2 Result](production-cdc-phase-2-result.md).

### Current-feature impact

- Recreating `cdc-connect` briefly pauses every connector.
- Logical, incremental, and trigger pipelines resume from persisted offsets.
- Trigger changes accumulate in the source changelog during the pause.
- LZ4 changes storage/network efficiency, not record content.
- Producer linger may add up to the configured delay to publication latency.
- Shorter offset flushing reduces crash replay but increases writes to the offset topic.
- At-least-once replay remains possible.

### Work

1. Measure current heap, GC, worker properties, and connector lag.
2. Make heap configurable.
3. Start with `-Xms1g -Xmx2g` only if host capacity allows heap plus native memory.
4. Apply one tuning group at a time:
   - Heap sizing
   - LZ4 producer compression
   - Producer batch size `131072`
   - Producer linger `20ms`
   - Consumer max-poll records `500`
   - Configurable offset flush
5. Preserve the current effective offset-flush value initially.
6. Trial `10000ms` offset flush in staging after heap and batching are stable.
7. Keep Connect internal topics compacted with replication factor `1`.
8. Add connector-builder tests in:
   - `src/reconcile.rs`
   - `src/incremental_cdc.rs`
   - `src/postgres_trigger_cdc.rs`
9. Do not add blanket `errors.tolerance=all`.

### Validation

- Every connector task returns to `RUNNING`.
- Logical insert/update/delete continues.
- Incremental watermarks resume and updates remain upserts.
- Trigger changelog offsets resume and hard deletes apply.
- Resync still converges.
- Lag drains after peak traffic.
- No OOM, sustained GC pressure, or rebalance loop occurs.
- Replayed records do not create duplicate target rows.

### Exit gate

All Phase 0 tests pass with expected peak load and restart recovery.

### Rollback

Restore prior Connect environment values and recreate only `cdc-connect`. Do not delete internal topics or reset offsets.

## Phase 3 — Redpanda runtime hardening

**Status:** Phase 3A completed on 2026-09-06. Redpanda now runs with `2G`
process memory and `256M` reserve in the production overlay. Phase 3B platform
overlays are available but have not been applied to the current Mac deployment;
see [`production-cdc-phase-3b-platform-overlays.md`](production-cdc-phase-3b-platform-overlays.md).
The compatibility flags remain in place on Mac and shared Linux hosts; see
[`production-cdc-phase-3a-result.md`](production-cdc-phase-3a-result.md).
Phase 3B runtime rollout on a dedicated Linux host and Phase 3C
(`--check=false`) remain pending.

### Current-feature impact

- Restarting Redpanda interrupts Kafka, Schema Registry, Connect, ksqlDB, topic UI operations, and all pipelines.
- Source-side capture state is not intentionally reset.
- Source-log and trigger-changelog backlogs grow while Kafka is unavailable.
- Removing `--check=false` can block startup on an unsuitable host.
- Removing `--overprovisioned` changes CPU scheduling expectations.
- Increasing Redpanda memory reduces RAM available to other containers and the OS.

### Work

1. Schedule a maintenance window.
2. Confirm source log and trigger changelog capacity.
3. Take fresh backups and exports.
4. Configure production `--smp`, memory, and nonzero reserve.
5. Use `2G` process memory and `256M` reserve only when host measurements permit.
6. Apply memory settings first while retaining current development flags.
7. Restart and verify the complete stack.
8. Keep `--overprovisioned` on Docker Desktop, shared Linux hosts, and
   contended VMs. On dedicated native Linux only, add
   `docker-compose.prod-linux.yaml` to remove it as a separate rollout.
9. Remove `--check=false` only after platform-specific production preflight
   passes; do not remove it from the portable overlay.
10. Add `docker/scripts/configure-redpanda-production.sh` that:
    - Uses `rpk cluster config set`
    - Reads values back with `rpk cluster config get`
    - Runs `rpk cluster config status`
    - Records old and new values
    - Is safe to run repeatedly
    - Leaves retention unchanged until Phase 4

### Validation

- Redpanda reports healthy.
- Kafka and Schema Registry respond.
- Existing topics, schemas, offsets, and consumer groups remain.
- Connect tasks return to `RUNNING`.
- Backlog drains without missing data.
- Host swapping and memory pressure remain absent.

### Exit gate

The stack survives a clean Redpanda restart and fully recovers at peak backlog.

### Rollback

Restore the prior Redpanda command and recorded cluster properties. Reuse the existing data volume.

## Phase 4 — Retention and disk protection

**Status:** Baseline and zero-deletion canary completed on 2026-09-06; see
[`production-cdc-phase-4-baseline.md`](production-cdc-phase-4-baseline.md).
The selected policy is 7 days for CDC and 30 days for DLQ. Per-topic byte caps
remain deferred until production-like throughput is measured. See
[`production-cdc-phase-4-canary-result.md`](production-cdc-phase-4-canary-result.md).
No records were deleted.

### Current-feature impact

- Ongoing replication is unaffected while consumers stay within retention.
- Expired Kafka messages are permanently removed.
- Topic-browser history becomes shorter.
- Consumers cannot replay from offsets older than the retained log.
- A sink offline longer than retention may require resync.
- Kafka retention and source trigger-changelog retention are independent.
- This is the only intentionally irreversible data-lifecycle phase.

### Capacity calculation

```text
required_bytes =
  peak_bytes_per_second
  × replay_window_seconds
  × safety_factor
```

Use a safety factor of at least `1.5`, plus capacity for compaction, segments awaiting deletion, internal topics, and recovery bursts.

### Work

1. Measure peak topic bytes per second.
2. Agree on the required replay and maximum-outage window.
3. Inventory every topic's cleanup and retention settings.
4. Exclude Connect internal topics from broad retention changes.
5. Select one noncritical CDC topic as a canary.
6. Apply topic-level:
   - `segment.bytes`
   - `retention.ms`
   - `retention.bytes`
7. Use `rpk topic alter-config`, not unverified broker startup flags.
8. Observe segment rolling and deletion through a full retention cycle.
9. Verify active and delayed consumers remain within retained offsets.
10. Roll out in topic batches.
11. Add `topic.creation.*` defaults to source connector builders for new topics.
12. Add idempotent migration for existing topics through `src/kafka_topics.rs`.
13. Keep DLQ retention longer than normal CDC retention.
14. Keep `changelog_retention_hours` longer than the maximum outage and lag.

### Exit gate

- Disk growth stabilizes.
- The promised replay window remains available.
- Internal Connect topics keep valid configs and offsets.
- No consumer is outside retention.

### Rollback

Reset topic overrides. Already expired records require backup restore or source resync.

## Phase 5 — Explicit error and DLQ policy

Status: implementation and local runtime rollout completed on 2026-09-06. Existing
connector configurations remained byte-for-byte equivalent. Staging malformed-record
injection remains an exit-gate exercise and was not run against the active stack. See
`docs/production-cdc-phase-5-result.md`.

### Current-feature impact

- Adding a defaulted model does not change existing profiles.
- Logical and incremental sinks continue failing fast by default.
- Trigger source continues failing fast.
- Trigger sink preserves its existing DLQ behavior.
- Explicit DLQ mode changes semantics: rejected rows do not stop the sink, so the target can remain incomplete until correction and replay.

### Work

1. Add a profile-level `sink_error_policy`:
   - `fail_fast` as the default
   - `dlq` as explicit opt-in
2. Preserve backward compatibility through serde defaults.
3. Update `src/models.rs`, `docs/openapi.yaml`, MCP types, and WebUI profile forms.
4. Add `apply_sink_error_policy` in `src/reconcile.rs`.
5. Use the helper from logical, incremental, and trigger sink builders.
6. Generate no tolerance or DLQ properties for default logical/incremental profiles.
7. Preserve trigger sink's current effective policy.
8. Require DLQ topic naming and error-context headers.
9. Surface DLQ topic and message count through API and UI.
10. Alert whenever DLQ high-watermark increases.
11. Document correction and replay.
12. Inject malformed records in staging to test both modes.

### Exit gate

- Existing profiles generate equivalent connector configurations.
- Fail-fast still stops malformed logical/incremental records.
- Explicit DLQ mode continues and exposes error context.
- Trigger source never silently skips events.
- Corrected records can be replayed without duplicates.

### Rollback

Return profiles to `fail_fast` and restore the previous connector configuration. Do not delete DLQ topics until rejected records have been resolved.

## Phase 6 — Soak testing and production rollout

### Current-feature impact

- Failure injection and destructive testing run only in staging.
- Production rollout needs planned Connect and Redpanda maintenance windows.
- Retention activation occurs last.

### Soak tests

1. Use production-like data volume, connector count, and change rate.
2. Run long enough to cross several offset-flush and retention cycles.
3. Restart Connect during logical, incremental, and trigger traffic.
4. Restart Redpanda with queued source changes.
5. Interrupt source and target database connections.
6. Build a trigger backlog larger than one 500-row batch.
7. Inject a malformed sink record.
8. Exercise disk warning and retention cleanup.
9. Restart orchestrator and rqlite.
10. Run resync with and without clearing target tables.
11. Reconcile source and target rows after every failure.

### Production rollout

1. Freeze profile changes.
2. Export configurations and offsets.
3. Take fresh backups.
4. Apply the Phase 1 override.
5. Apply and verify Connect tuning.
6. Apply and verify Redpanda runtime tuning.
7. Observe a complete business cycle.
8. Activate retention only after explicit approval.
9. Keep prior images, environment, and cluster settings ready for rollback.

### Final acceptance

- No source changes are missing from the target.
- No unexpected duplicate target rows exist.
- Trigger hard deletes propagate.
- Lag returns to zero after peak load and restart.
- Disk growth is bounded after retention activation.
- DLQ growth is visible and actionable.
- Restore and rollback procedures have been exercised.
- Development Compose still works without the production override.

## Planned files

- `docker/docker-compose.prod.yaml`
- `docker/.env.example`
- `docker/.env.production.example`
- `docker/scripts/production-preflight.sh`
- `docker/scripts/configure-redpanda-production.sh`
- `src/reconcile.rs`
- `src/incremental_cdc.rs`
- `src/postgres_trigger_cdc.rs`
- `src/models.rs`
- `docs/openapi.yaml`
- WebUI and MCP profile types for explicit error policy

## Recommended implementation order

Implement and approve each phase independently:

1. Phase 0
2. Phase 1
3. Phase 2
4. Phase 3
5. Phase 4
6. Phase 5
7. Phase 6

Do not combine Phases 2, 3, and 4 into one production deployment. Separate changes make their effects measurable and rollback practical.
