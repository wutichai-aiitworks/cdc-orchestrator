# Production CDC Phase 4 Baseline

Date: 2026-09-06

## Outcome

The read-only Phase 4 baseline is complete. No retention configuration was
changed and no Kafka records were deleted.

## Storage

- Docker filesystem: `291 GiB`
- Filesystem used: `171 GiB`
- Filesystem available: `106 GiB`
- Redpanda data directory: approximately `3.3 GiB`
- Redpanda cluster default retention: `604800000 ms` (7 days)
- Redpanda cluster default segment size: `134217728` bytes (128 MiB)
- Default and observed topic `retention.bytes`: unlimited (`-1`)

The filesystem figures include Docker data beyond Redpanda, so the full
`106 GiB` must not be assigned to Kafka retention.

## Topic inventory

- 22 topics were visible through the CDC topic inventory.
- All topics had one replica because this is a single-broker deployment.
- No topic had an out-of-sync replica.
- Kafka Connect internal topics use compaction and must be excluded from broad
  Phase 4 retention updates.
- `_schemas` is compacted and must remain excluded.
- `cdc-test01-dlq` and `cdc-test01-sink-dlq` exist and require a longer policy
  than ordinary CDC topics.
- The largest reported CDC topic segment was approximately `2 MiB`; the current
  demo workload is too small to represent production capacity requirements.

## Traffic and lag

A 30-second per-topic produced-byte sample recorded no CDC writes. Both active
sink consumer groups reported zero total lag.

This confirms the stack is caught up, but it does not provide a usable peak
bytes-per-second value. A production-like load sample is required before a
safe `retention.bytes` cap can be calculated.

## Decision required before canary

Before applying an irreversible canary policy, select:

1. Normal CDC replay/maximum-outage window.
2. DLQ replay window.
3. Disk budget or a production-like peak throughput measurement.
4. A confirmed noncritical canary topic.

Until those decisions are made, keep the existing 7-day time retention and do
not add a per-topic byte cap.

