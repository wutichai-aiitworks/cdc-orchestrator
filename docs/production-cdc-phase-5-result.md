# Phase 5 result — Explicit sink errors and DLQ

Date: 2026-09-06

## Outcome

Phase 5 is implemented and running locally. Existing profiles deserialize with
`sink_error_policy=fail_fast`. Logical and incremental sinks emit no Kafka Connect
`errors.*` overrides unless DLQ is explicitly selected. Trigger sources still fail
loudly, while trigger sinks retain their previous DLQ behavior.

No Kafka Connect worker or connector was restarted during rollout.

## Contract and behavior

- Profile/API policy: `fail_fast` (default) or `dlq`.
- Legacy `PUT` clients that omit `sink_error_policy` preserve the profile's
  stored policy; omission defaults to `fail_fast` only when creating a profile.
- `dlq` is accepted only for `source_to_target` profiles.
- Trigger capture reports `configured_policy=fail_fast` and
  `effective_policy=dlq` for legacy profiles, making the compatibility override
  explicit.
- DLQ connector settings include:
  - `errors.tolerance=all`
  - deterministic `<sink-connector>-dlq` topic naming
  - DLQ replication factor `1` for this single-broker deployment
  - error-context headers
  - bounded retries
- `GET /cdc/profiles/{id}/dlq` reports the topic, existence, current retained
  message count, and monotonic high watermark.
- The watcher emits an alert-history/WebSocket event whenever an observed DLQ
  high watermark increases. Initial observation establishes a baseline and does
  not replay old alerts after every orchestrator restart.
- MCP exposes `cdc_profile_dlq` and accepts `sink_error_policy` in
  `cdc_pipeline_create`.
- The pipeline wizard exposes fail-fast versus DLQ for database replicas and
  shows effective DLQ status and rejected-record count.

## Runtime rollout

The orchestrator, native UI, and MCP images were rebuilt and recreated with a
health-gated rollback guard.

- Orchestrator: healthy, image
  `sha256:05e93f831067491d4dae9377eac63e081c201622532238b5538a860a2f811b18`
- Native UI: healthy, image
  `sha256:e27d35efab1ce4b18f9afc1b46534268acd051749f5000429bd81c3bf82bb3b2`
- MCP: running with `/healthz` passing, image
  `sha256:1ee89ab1aa3f94e560d54f5af80bdc455c2cd8ab0af2e7a930a32616fce1eea0`

All four pre-existing connector configuration hashes remained unchanged.
`cdc-demo`, `cdc-demo-sink`, and `cdc-test01-sink` retained their prior states.
`cdc-test01` was already failed before rollout because
`vpn.patanayont.com` did not resolve; Phase 5 did not cause or alter that failure.

## Validation

- Main Rust tests: 87 passed.
- Library tests: 4 passed.
- MCP tests/build: passed.
- Native UI production build: passed.
- OpenAPI JSON generation and freshness check: passed.
- MCP tool discovery includes `cdc_profile_dlq` and `sink_error_policy`.
- Direct orchestrator and native UI proxy DLQ responses match, excluding their
  independently generated timestamps.
- Existing connector config SHA-256 values are unchanged after rollout.

The repository-wide Nuxt typecheck still reports pre-existing errors in consumer
groups, profile catalog request types, schema comparison, schemas, and topic
metric tones. The production build passes, and no Phase 5 file introduced a new
reported type error.

## Correction and replay runbook

1. Query `/cdc/profiles/{id}/dlq` or MCP `cdc_profile_dlq`. Record the DLQ topic,
   retained count, and high watermark.
2. Consume and archive rejected records with their Kafka Connect context headers.
   Important headers include the original topic, partition, offset, connector,
   processing stage, exception class, and exception message.
3. Correct the target-side cause first: schema drift, missing table/column,
   invalid conversion, constraint, permissions, or connectivity.
4. Replay with a dedicated, checkpointed replay consumer and an idempotent
   producer. Publish the original key and serialized value to the original input
   topic identified by the DLQ headers—not back to the DLQ topic. Preserve Avro
   bytes/schema IDs and per-key ordering.
5. Use one replay consumer group and checkpoint each DLQ partition. Logical,
   incremental, and trigger target writes are key-based upserts/deletes, so
   replaying the same key is idempotent at the target row level.
6. Verify the target row, sink task state, sink lag, and replay checkpoint before
   advancing or resolving the alert.
7. Keep the DLQ topic until every rejected offset is accounted for. Do not clear
   it merely because the connector is running.

## Remaining exit gate

Malformed-record tests for both `fail_fast` and `dlq` must be run in staging with
production-equivalent converters and target constraints. Confirm stopping versus
continuation, context headers, alert emission, correction, and checkpointed replay
before enabling DLQ on a production logical or incremental profile.
