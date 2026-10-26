# Production CDC Phase 2 Result

Completed: 2026-09-06  
Scope: Kafka Connect heap, producer batching/compression, consumer batch size, and offset flush  
Runtime activation: Applied to `cdc-connect`

## Result

Phase 2 was applied by recreating only Kafka Connect.

- Redpanda was not restarted.
- rqlite was not restarted.
- Orchestrator was not restarted.
- Source and target databases were not restarted.
- Topics and Connect internal offsets were not reset.
- All four connector tasks recovered to `RUNNING`.
- Connector configurations are byte-semantically identical before and after the worker restart.
- Logical source and target row counts remain identical.
- Trigger target counts remain unchanged.
- Both active sink consumer groups have zero lag.
- No fatal, OOM, or failed-task message was found after startup.

## Applied worker settings

```text
HEAP_OPTS=-Xms1g -Xmx2g
producer.compression.type=lz4
producer.batch.size=131072
producer.linger.ms=20
consumer.max.poll.records=500
offset.flush.interval.ms=60000
offset.flush.timeout.ms=5000
```

The offset-flush interval intentionally remains at the previous effective value of 60 seconds. Shortening it is a separate measured change because it affects offset-topic write frequency and crash replay bounds.

## Runtime impact

Only `cdc-connect` was recreated.

```text
Restart started:       2026-09-06T07:03:31Z
All tasks reverified:  2026-09-06T07:04:26Z
```

The 55-second interval includes validation and a shell-script correction; connector tasks were observed running before final verification completed.

During restart:

- Logical changes could queue in source WAL.
- Trigger changes could queue in the Oracle changelog.
- Incremental capture would retain its stored watermark.
- Kafka topics and offsets remained available because Redpanda stayed running.

No new source mutation was injected during the restart.

## Before and after

Kafka Connect heap:

```text
Before: -Xms512m -Xmx1024m
After:  -Xms1g -Xmx2g
```

Observed container memory:

```text
Before: approximately 708 MiB
After:  approximately 1,001 MiB
```

The increase is expected from the larger initial heap. Docker has approximately 63.66 GiB available, so the current worker remains well inside the host allocation.

## Connector verification

Recovered connectors:

- `cdc-demo` — logical source
- `cdc-demo-sink` — logical sink
- `cdc-test01` — trigger changelog source
- `cdc-test01-sink` — trigger apply sink

Every connector and task reports `RUNNING`.

Canonical SHA-256 hashes of each connector configuration matched before and after restart. Worker-level tuning did not rewrite profile-generated connector properties.

## Data verification

Logical source and target exact counts matched before and after:

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

Trigger target counts were unchanged:

```text
BCS.G_BOOKS       1,004
hyper_cdc.inbox      16
```

Sink group lag:

```text
connect-cdc-demo-sink    0
connect-cdc-test01-sink  0
```

## Effective-property verification

The Debezium image translates `CONNECT_*` environment variables into Kafka Connect worker properties.

Offset flushing is different: the image initializes it from `OFFSET_FLUSH_INTERVAL_MS` and exports `CONNECT_OFFSET_FLUSH_INTERVAL_MS` internally. The production overlay uses the correct non-`CONNECT_` input name.

The generated `/kafka/config/connect-distributed.properties` contains all six expected Phase 2 properties.

## Preflight updates

`docker/scripts/production-preflight.sh` now validates:

- Supported producer compression
- Positive producer batch size
- Positive producer linger
- Positive consumer max-poll records
- Positive offset-flush interval
- Positive offset-flush timeout

Updated preflight result:

```text
0 failures
3 expected warnings
```

Warnings remain unchanged:

- Redpanda reserve is still Phase 1-compatible `0M`.
- Redpanda has legacy Compose ownership metadata.
- rqlite has legacy Compose ownership metadata.

## Recovery evidence

Private before/after artifacts:

```text
/Users/wutichai/workspace/hyper/backups/hyper-cdc-orchestrator/
  phase2-20260906-140308/
```

The directory contains:

- Private connector configurations before and after
- Canonical configuration hashes
- Connector statuses
- Logical source/target counts
- Trigger target counts
- Consumer lag summaries
- Effective worker properties
- Container identities
- SHA-256 manifests

Permissions are restricted. The private connector exports contain credentials and must not be committed or shared.

## Validation-script note

The first post-start validation command used `status`, which is a reserved zsh variable. The command stopped validation after the tuned container was already healthy. No rollback or second recreation occurred. A portable variable name was used for the remaining checks.

## Rollback

To return Kafka Connect to Phase 1 defaults without touching Redpanda or offsets:

```sh
cd docker
CDC_ENV_FILE=.env docker compose \
  --env-file .env \
  -f docker-compose.yaml \
  up -d --no-deps --force-recreate --no-build cdc-connect
```

Then wait for all tasks to return to `RUNNING` and repeat count/lag checks.

Do not delete `_cdc_connect_configs`, `_cdc_connect_offsets`, or `_cdc_connect_statuses`.

## Phase 2 exit decision

Passed:

- Tuned worker properties are active.
- Connector configurations did not change.
- Connector tasks recovered.
- Logical exact counts match.
- Trigger target counts are unchanged.
- Active sink lag is zero.
- Redpanda remains healthy.
- No fatal worker errors were found.
- Updated preflight passes.

Remaining:

- Long-duration peak-load soak belongs to Phase 6.
- Shorter offset flush remains deliberately deferred.
- No active incremental runtime pipeline exists; incremental restart behavior remains a staging acceptance gap.
