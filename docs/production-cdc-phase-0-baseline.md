# Production CDC Phase 0 Baseline

Captured: 2026-09-06  
Scope: Current single-host CDC stack  
Result: Baseline and recovery checks passed, with production blockers and test-coverage gaps listed below

## Executive result

Phase 0 completed without changing application code, connector configurations, offsets, topics, or profile definitions.

- Secure recovery exports were created.
- Redpanda and rqlite received consistent stopped-volume backups.
- The maintenance window was 33 seconds.
- Both backups passed SHA-256 verification.
- Both backups were restored into isolated disposable volumes.
- Restored Redpanda started healthy and exposed the expected topics.
- Restored rqlite became leader/ready and accepted a database query.
- The original stack restarted healthy.
- All four active Kafka Connect connector tasks returned to `RUNNING`.
- The active logical replica has exact source/target row-count equality for all eight subscribed tables.
- The active trigger topic has zero consumer lag and both source/sink tasks are running.
- The full Rust test suite passed: 88 tests, 0 failures.

Phase 1 should not begin until the production blockers in this report are accepted and prioritized.

## Secure backup

The backup is outside the repository:

```text
/Users/wutichai/workspace/hyper/backups/hyper-cdc-orchestrator/
  phase0-20260906-131657/
```

Permissions:

- Backup directory: `0700`
- Private exports: `0600`
- Files containing connection credentials are intentionally marked `private`.
- Do not commit, publish, or attach this directory to support tickets.

Contents:

- `exports/compose-private.yaml`
- `exports/connectors-private.json`
- `exports/profiles-private.json`
- `exports/docker-env-private`
- `exports/images.json`
- `exports/redpanda-cluster-config.yaml`
- `exports/redpanda-health.txt`
- `exports/topics.txt`
- `volumes/cdc-redpanda-data.tar.gz`
- `volumes/cdc-rqlite-data.tar.gz`
- SHA-256 manifests
- Restore verification results

Backup sizes:

- Redpanda volume apparent size: approximately 3.3 GiB
- Redpanda compressed archive: approximately 361 KiB
- rqlite volume apparent size: approximately 17 MiB
- rqlite compressed archive: approximately 1.2 MiB

The large Redpanda difference is consistent with sparse/preallocated segment files. The isolated Redpanda restore verified that the archive contains usable broker state and topic metadata.

## Maintenance and recovery

Maintenance window:

```text
Start: 2026-09-06T06:17:25Z
End:   2026-09-06T06:17:58Z
Total: 33 seconds
```

Services stopped:

- MCP
- Native WebUI
- Kafka UI
- ksqlDB
- Orchestrator
- Kafka Connect
- Redpanda
- rqlite

The source and target databases remained running.

Recovery result:

- Redpanda: healthy
- rqlite: ready, leader established
- Kafka Connect: healthy
- Orchestrator: healthy
- ksqlDB: healthy
- Native WebUI: healthy
- No volume was deleted or recreated
- No connector offset was reset

Expected shutdown/restart messages were observed:

- Redpanda broken-condition/disconnect message during shutdown
- Connect broker disconnect during Redpanda shutdown
- Orchestrator watcher warning while Connect was unavailable
- rqlite raft transport close while stepping down
- Connect internal-topic `already exists` warnings during recovery

These were confined to the maintenance interval. No continuing fatal, OOM, or failed-task condition was found after recovery.

## Host capacity baseline

Host:

- CPU: 16 logical CPUs
- Physical memory: 128 GiB
- Docker-visible memory: approximately 63.66 GiB
- Host filesystem free space: approximately 334 GiB
- Docker local volumes: approximately 76.41 GiB total
- Docker reclaimable volume data: approximately 46.52 GiB
- Docker images: approximately 66.06 GiB
- Docker build cache: approximately 35.69 GiB

The host has adequate capacity for conservative Redpanda and Connect increases, but Docker's approximately 64 GiB allocation—not total host RAM—must be used for Phase 2 and Phase 3 sizing.

Settled CDC memory baseline:

- Redpanda: approximately 360 MiB
- Kafka Connect: approximately 708 MiB
- ksqlDB: approximately 897 MiB
- JDBC admin: approximately 199 MiB
- Native WebUI: approximately 43 MiB
- Orchestrator: approximately 20 MiB
- rqlite: approximately 16 MiB

Current effective Connect heap:

```text
-Xms512m -Xmx1024m
```

Current effective Redpanda runtime:

```text
--overprovisioned
--smp 1
--memory 1G
--reserve-memory 0M
--check=false
```

## Redpanda baseline

Cluster:

- One node
- Healthy
- No leaderless partitions
- No under-replicated partitions
- All topics have replication factor `1`

Persisted cluster configuration already contains:

```text
log_retention_ms = 604800000
log_segment_size = 134217728
```

This means a seven-day default retention and 128 MiB log segments are already active even though those settings are not declared in the current Compose command. Phase 3 and Phase 4 must preserve and explicitly reconcile this pre-existing state instead of blindly applying new defaults.

Current inventory:

- 22 visible topics in the orchestrator result
- Connect config, offset, and status topics are compacted
- Schema Registry topic is compacted
- Trigger DLQ topics currently contain zero messages
- No under-replicated topic was reported

## Connector baseline

Active connectors:

- `cdc-demo`: logical PostgreSQL source, `RUNNING`
- `cdc-demo-sink`: logical JDBC sink, `RUNNING`
- `cdc-test01`: Oracle trigger-changelog JDBC source, `RUNNING`
- `cdc-test01-sink`: trigger apply JDBC sink, `RUNNING`

Trigger connector configuration:

- Poll interval: 10 seconds
- Batch maximum: 500 rows
- Source conversion errors: fail fast after retry
- Sink errors: retry, then DLQ
- Active changelog topic: `test01_hyper_cdc.change_log`
- Active sink consumer lag on the changelog topic: `0`
- Trigger sink DLQ messages: `0`

The lag API also reports old or deliberately unsubscribed topics under a consumer group:

- `demo.bookings.routes` is excluded because it has no supported primary key.
- `test01.G_BOOKS` is an old/non-active topic for the current trigger sink.

Those entries make group-level total lag appear nonzero, but the actively subscribed logical and trigger topics are at zero lag. Production monitoring must calculate lag from the connector's actual subscription rather than every historical topic listed under the group.

## Logical replica verification

Exact source and target counts matched after restart:

```text
airplanes_data   10
airports_data    5,501
boarding_passes  2,463,832
bookings         1,292,893
flights          21,758
seats            1,741
segments         3,941,249
tickets          2,973,937
```

`routes` is intentionally excluded from the sink.

Result:

- Connector offsets survived the maintenance restart.
- Sink lag is zero for all eight subscribed topics.
- Exact row counts match.

## Trigger replica verification

Current target state:

```text
BCS.G_BOOKS       1,004 rows
hyper_cdc.inbox      16 rows
```

Current operational result:

- Trigger source connector is `RUNNING`.
- Trigger sink connector is `RUNNING`.
- Active changelog-topic consumer lag is zero.
- DLQ message count is zero.
- Profile `test01` remains enabled and running.
- Automatic full refresh is disabled.

No production Oracle row was modified during Phase 0. Therefore, insert/update/delete behavior was not reinjected during this baseline; the check proves restart and offset recovery, not a new source mutation.

## Incremental verification

No active incremental profile exists in this stack, so runtime restart and watermark behavior could not be verified without creating a new pipeline.

Code-level coverage passed:

- Timestamp plus incrementing mode
- Incrementing-only mode
- Oracle-to-Postgres lowercase field mapping
- Upsert behavior with deletes disabled
- Invalid watermark-column rejection
- Incremental profile validation
- Reconciliation selects JDBC source instead of Debezium

This is a Phase 0 coverage gap. Before production approval for incremental mode, create an isolated staging incremental profile and verify:

1. Initial load
2. Updated-row upsert
3. Connector restart
4. Watermark resume
5. No duplicate target rows
6. Explicit behavior for hard deletes

## Automated tests

Command:

```sh
cargo test
```

Result:

- Library tests: 4 passed
- Main binary tests: 84 passed
- Total: 88 passed
- Failures: 0

Three existing compiler warnings remain:

- Unread `CountBody.table`
- Unused `parse_rpk_group_describe`
- Unused `restart_connector`

No warning was introduced by Phase 0 because Phase 0 changed no Rust code.

## Production blockers discovered

### 1. Mixed Compose ownership

Container labels show two different historical Compose files:

- `cdc-connect` and `cdc-orchestrator` reference `docker/docker-compose.yaml`.
- `cdc-message` and `cdc-rqlite` reference a former repository-root `docker-compose.yml`.
- The former root Compose file no longer exists.

This can cause inconsistent recreation behavior and configuration drift. Phase 1 must normalize ownership to `docker/docker-compose.yaml` while preserving existing named volumes.

### 2. Production authentication is not enabled

Current environment is development mode and no orchestrator API token is configured.

Additional observations:

- MCP write tools are enabled.
- Several infrastructure ports are published on all interfaces.
- Kafka UI authentication is disabled.

Although the requested hardening scope focuses on Redpanda and Connect, this blocks a claim of overall production readiness. It requires a separate security-hardening decision.

### 3. Mutable and old images

- Kafka UI uses the mutable `latest` tag.
- ksqlDB image is old and runs an amd64 image on an arm64 host.
- Several images are local builds without immutable registry digests.

Production deployment should pin immutable tags or digests and retain a tested rollback image set.

### 4. No active incremental runtime baseline

Incremental implementation tests pass, but there is no running incremental pipeline to verify operational recovery.

### 5. Lag aggregation includes unrelated topics

The current group-lag view can include topics the connector does not subscribe to, producing a misleading nonzero total. Alerting should use actual connector subscriptions.

### 6. Resource limits are absent

The stack relies on Docker Desktop's global memory allocation and process-level Redpanda/Connect settings. Per-service CPU and memory boundaries are not defined.

## Phase 0 decision

Passed:

- Backup
- Checksum verification
- Isolated restore
- Stack restart
- Connector restart recovery
- Logical data equality
- Trigger offset recovery
- Full automated test suite

Conditional:

- Trigger source DML was not modified during this phase.
- Incremental runtime validation requires a staging pipeline.

Before Phase 1:

1. Accept the 33-second recovery baseline.
2. Preserve the existing seven-day Redpanda retention during normalization.
3. Normalize mixed Compose ownership without renaming volumes.
4. Decide whether security hardening is added to the production-readiness scope.
5. Create an isolated incremental staging profile for runtime acceptance.
