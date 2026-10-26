# Production CDC Phase 3A Result

Date: 2026-09-06

## Outcome

Phase 3A completed successfully. The production overlay now starts Redpanda
with:

- `--smp 1`
- `--memory 2G` (previously `1G`)
- `--reserve-memory 256M` (previously `0M`)

`--overprovisioned` and `--check=false` were intentionally retained. Removing
them remains separate Phase 3B and Phase 3C work.

## Applied changes

- Updated `docker/docker-compose.prod.yaml` defaults.
- Updated `docker/.env.example` and `docker/.env.production.example`.
- Changed production preflight to reject a zero Redpanda reserve.
- Recreated only `cdc-message`, reusing
  `cdc-stack_cdc-redpanda-data`.
- Normalized `cdc-message` Compose ownership to the canonical base and
  production overlay files.

No Kafka Connect worker or connector configuration was changed.

## Maintenance result

- Restart began: `2026-09-06T10:33:46Z`
- All four connector tasks were `RUNNING`: `2026-09-06T10:33:53Z`
- Observed recovery window: 7 seconds
- Automatic rollback protection was armed and was not needed.

## Verification

The following checks passed:

- Redpanda cluster health is healthy.
- No leaderless or under-replicated partitions exist.
- Kafka topics before and after are identical.
- Schema Registry subjects before and after are identical.
- Connector configuration hashes before and after are identical.
- `cdc-demo`, `cdc-demo-sink`, `cdc-test01`, and `cdc-test01-sink` and all
  their tasks are `RUNNING`.
- Both active sink consumer groups report zero total lag.
- Logical source and target row counts match and did not change.
- Trigger target row counts did not change.
- `log_retention_ms` remains `604800000` (7 days).
- `log_segment_size` remains `134217728` (128 MiB).
- The stopped-volume backup checksum and archive readability checks passed.
- Production preflight passes with zero failures and one pre-existing rqlite
  ownership warning.
- No linter diagnostics were reported for the changed Docker files.

Immediately after recovery, Redpanda used approximately `483 MiB` resident
container memory. Docker had approximately `63.66 GiB` available, so the new
allocation did not create host memory pressure.

Redpanda logged one non-fatal 192 KiB oversized-allocation warning while the
administrative verification commands were running. There was no startup
failure, out-of-memory event, unhealthy partition, lag, or connector failure.
This warning should be watched during the soak period.

## Backup and evidence

The restricted backup and before/after evidence are stored outside the
repository at:

`/Users/wutichai/workspace/hyper/backups/hyper-cdc-orchestrator/phase3a-20260906-173333`

The backup contains the stopped Redpanda volume archive, checksum, connector
configuration hashes and status, topic and schema inventories, consumer lag,
row counts, cluster health, runtime commands, and restart timing.

## Remaining Phase 3 risks

Phase 3 is not complete:

1. Phase 3B keeps `--overprovisioned` on Mac/shared hosts. Its dedicated Linux
   overlay removes the flag and still requires runtime CPU and latency
   validation on the target Linux host.
2. Phase 3C must run Redpanda host/filesystem checks before removing
   `--check=false`; startup can fail if the host is unsuitable.
3. Cluster properties still need the planned idempotent configuration script.
4. Retention remains unchanged until Phase 4.
5. The single-broker deployment still has no broker high availability.

