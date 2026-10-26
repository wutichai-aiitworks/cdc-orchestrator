# Production CDC Phase 4 Canary Result

Date: 2026-09-06

## Selected policy

- Normal CDC retention: 7 days (`604800000 ms`)
- DLQ retention: 30 days (`2592000000 ms`)
- Segment size: 128 MiB (`134217728` bytes)
- `retention.bytes`: deferred until production-like throughput is measured
- Canary: `test01_hyper_cdc.HYPER_CDC_CHANGE_LOG`

## Applied canary

The canary already used 7-day retention and unlimited `retention.bytes`.
Phase 4 made the 128 MiB segment size an explicit topic override without
shortening history:

- `retention.ms=604800000` (`DYNAMIC_TOPIC_CONFIG`)
- `segment.bytes=134217728` (`DYNAMIC_TOPIC_CONFIG`)
- `retention.bytes=-1` (unchanged)

The log-start offset remained `0` and the high watermark remained `3`, so all
three canary records were retained.

## Tooling

`docker/scripts/configure-topic-retention.sh` now:

- Defaults to dry-run mode.
- Requires explicit topic names.
- Rejects Kafka internal and platform topics.
- Applies 7-day retention to ordinary CDC topics.
- Applies 30-day retention to DLQ topics.
- Leaves `retention.bytes` untouched when no measured byte cap is configured.
- Reads settings back after an applied change.
- Is idempotent.

The Kafka Connect worker enables source topic creation, and logical,
incremental, and trigger source connector builders now provide default
creation settings for 7-day retention and 128 MiB segments. They intentionally
omit `topic.creation.default.retention.bytes`.

These Connect changes have not been deployed to the running worker. They take
effect after a future image rebuild, `cdc-connect` recreation, and connector
reconciliation.

## Validation

- Redpanda remained healthy.
- All four connector tasks remained `RUNNING`.
- Both active sink consumer groups remained at zero lag.
- The retention script's second applied run reported `UNCHANGED`.
- DLQ dry-run selected 30-day retention.
- Internal-topic protection rejected `_schemas`.
- Production preflight passed with no failures.
- Targeted Rust tests passed: 23 tests across logical, incremental, and trigger
  connector configuration.

The repository has pre-existing `rustfmt --check` differences in several Rust
files. The targeted tests and IDE diagnostics reported no new compile or lint
errors.

## Remaining work

1. Run production-like load long enough to measure peak bytes per second.
2. Calculate a byte cap from the replay window, peak rate, and at least a 1.5
   safety factor.
3. Rebuild and deploy the Connect/orchestrator changes during a maintenance
   window.
4. Observe the canary through a full seven-day retention cycle.
5. Migrate existing non-internal topics in reviewed batches.
6. Apply the 30-day policy to existing DLQ topics after confirming disk budget.
7. Keep trigger `changelog_retention_hours` longer than the supported maximum
   outage and Kafka lag.

