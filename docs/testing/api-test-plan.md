# API test plan & known-bug register

Last updated: 2026-11-01

## How to run

| Command | What it does | Needs a stack? |
|---|---|---|
| `npm run check` | OpenAPI route coverage + native-UI parity + Rust unit tests (orchestrator + MCP) | No |
| `npm run test:api` | Strict Playwright API suites — read-only or self-cleaning fixtures | Yes |
| `npm run test:api:e2e` | Same, plus pipelines that start connectors and write to the demo target | Yes + demo DBs |
| `npm run test:all` | `check` then `test:api` | Yes |

Environment knobs (see `tests/api/helpers.ts`):

| Variable | Default | Purpose |
|---|---|---|
| `ORCH_TEST_BASE_URL` | `http://localhost:9788` | Orchestrator |
| `ORCH_TEST_TOKEN` | — | Bearer when `ORCH_API_TOKEN` is set (`grep ^ORCH_API_TOKEN= docker/.env`) |
| `ORCH_TEST_DB_HOST` | `host.docker.internal` | How the orchestrator reaches demo Postgres |
| `ORCH_TEST_SOURCE_PORT` / `_TARGET_PORT` | `25432` / `25433` | Demo source / target |
| `ORCH_TEST_E2E` | off | Enable pipeline-starting tests |
| `ORCH_TEST_ALLOW_ENGINE` | off | Enable `engine/up|restart|down` (stops/bounces the stack) |

> The demo Postgres containers sit on `cdc_zone`, the orchestrator and jdbc-admin on
> `server_zone`, so the service names `cdc-postgres-source/target` do not resolve from the
> orchestrator. Tests (and the existing profiles) go through `host.docker.internal`. The MCP
> `cdc_pipeline_create` defaults still use the service names — see bug B-09.

## Safety rules

- Every fixture is prefixed `pw-<run>-` (profiles, compare profiles, topics) or `pw_<run>_`
  (schemas) and removed in `afterAll`. Suites never modify objects they did not create —
  shared stacks hold real customer profiles.
- E2E removes its connectors and drops its replication slot directly, because the
  orchestrator only pauses connectors on delete (B-02).
- Known bugs are encoded as `test.fail(...)`: the suite stays green while the bug exists and
  turns **red when it is fixed** ("expected to fail but passed") — then delete the annotation.

## Coverage

| Area | Suite | Notes |
|---|---|---|
| Health, engine, Connect, Kafka groups | `all-apis.spec.ts` | Legacy relaxed suite; engine mutations opt-in |
| Profiles: validate, catalog, inspect, trigger preview, CRUD, masking, enable, sub-resources, audit, input validation | `profiles.spec.ts` | |
| Real replication (sync_once) | `profiles.spec.ts` (E2E) | Capture verified; apply blocked by B-01/B-03 |
| Test initial sync: sample copy, stop, teardown, failure teardown, mode guard | `profiles.spec.ts` (E2E) | Incremental `bookings.flights` → scratch schema with extra `__deleted` column |
| Schema Compare: CRUD, counts, preview, schema counts, exec-ddl, injection | `compare.spec.ts` | |
| Topics: create, search, produce/consume, metadata, configs, guards, delete | `topics.spec.ts` | |
| ksqlDB (read-only) | `ksqldb.spec.ts` | |
| Capability, cluster, brokers, metrics, audit, pipeline-logs, notifications, logs | `ops.spec.ts` | |
| Migration tools ×4 (pgcopy, pgloader, redisshake, mongocopy) | `migration-tools.spec.ts` | Read-only |
| MCP: tool registry, dispatch, write guard, helpers, prompts | `mcp/src/*.rs` `#[cfg(test)]` | `cargo test` in `mcp/` |
| OpenAPI ↔ router drift | `scripts/check-openapi-routes.mjs` | All 108 `/cdc` routes documented |

Not covered yet: UI end-to-end (Nuxt), Connect connector CRUD in strict mode, ACLs / schema
registry writes, notification rules CRUD, lake (`source_to_lake`) and trigger-install flows,
migration-tool job runs.

## Known-bug register

| ID | Severity | Bug | Evidence | Test |
|---|---|---|---|---|
| B-01 | **Critical** | Sink writes to the wrong schema. `connection.url …?currentSchema=<target_schema>` + unqualified `table.name.format=${topic}`; the Debezium JDBC sink resolves an existing same-named table in another schema (here `bookings.airplanes_data`) and upserts into it. The mapped schema is never created. | Re-confirmed 2026-11-01 with a consistent mapping (`identical:false`, `target_schema=pw_b01`): `pw_b01.airplanes_data` never created after 450 s, `bookings.airplanes_data` `n_tup_upd` 10 → 20. (The first repro used `identical:true` + a different schema, which is contradictory — see B-12.) | `profiles.spec.ts` E2E "sink writes to the mapped target schema" |
| B-02 | **Critical** | Deleting a profile only **pauses** its connectors when `ORCH_STOP_DELETES_CONNECTOR=false` (default). Orphaned PAUSED connectors keep an **active replication slot** on the source → unbounded WAL retention on production sources. Errors are discarded (`let _ =`). | `handlers.rs::delete_profile` → `reconcile_stop` → `stop_connector(delete=false)` | E2E "deleting the profile removes its connectors" |
| B-03 | High | New pipelines wait up to **5 minutes** for first data: sink subscribes with `topics.regex` before the topics exist; Kafka re-evaluates regex only every `metadata.max.age.ms` (300 s). | Reproduced: offsets committed only after ~5 min | E2E "sink sees new topics without the 5-minute regex metadata delay" |
| B-04 | High | `POST /profiles/:id/enable {enabled:true}` always calls `start`, ignoring `autostart=false` and Draft status — re-enabling silently launches a pipeline. | `handlers.rs::set_profile_enabled` | `profiles.spec.ts` "enabling a draft … does not start it" |
| B-05 | High (security) | Compare profiles return **plaintext passwords** from create/list/get (CDC profiles mask `***`). | `GET /cdc/compare/profiles` | `compare.spec.ts` "compare profiles mask stored passwords" |
| B-06 | Medium | Duplicate topic create → **502** instead of 409. | `POST /cdc/topics` twice | `topics.spec.ts` |
| B-07 | Medium | Metadata for a missing topic → **200** with `UNKNOWN_TOPIC_OR_PARTITION` buried in `raw` instead of 404. | `GET /cdc/topics/:missing/metadata` | `topics.spec.ts` |
| B-08 | Low | Connect 404 (missing connector) surfaces as **502**. | `GET /connect/connectors/:missing/status` | `topics.spec.ts` |
| B-09 | Medium | MCP `cdc_pipeline_create` defaults hosts to `cdc-postgres-source/target`, unreachable from the orchestrator in the current compose network layout. | `mcp/src/tools.rs::connection_from` | — (deployment/config) |
| B-10 | Low | Body-parse errors (axum 400/422) are plain text, not the `{ "error": … }` envelope. | `POST /cdc/profiles` with bad body | `profiles.spec.ts` "body-parse errors use the JSON error envelope" |
| B-12 | Low | The API accepts a contradictory mapping (`identical:true` with a different `target_schema`/`target_table`). The snapshot path then uses the source names while the sink uses the target schema in `currentSchema`. Should reject or normalise `identical` when names differ. | `parse_qualified_tables` vs `build_jdbc_sink_config` | — |
| B-11 | Note | `POST /compare/target/exec-ddl` runs any SQL against any connection in the body (no allow-list). Intended for Schema Compare, but it is effectively a remote SQL runner — keep `ORCH_API_TOKEN` set. | Used by tests to drop slots/schemas | — |

### Fixed while building the suite

| ID | Fix |
|---|---|
| F-01 | Rust unit tests did not compile — `CdcProfile.autostart` missing in 4 test fixtures. |
| F-02 | MCP `urlencoding_simple` truncated non-ASCII (Thai) names (`c as u8`); now encodes UTF-8 bytes. |
| F-03 | MCP `cdc_connector_validate` was annotated as a write tool (`readOnlyHint=false`) though validation has no side effects. |
| F-05 | Test initial sync (`start?row_limit=N`) left the pipeline **running** as a live sync. It now copies the sample, resets source offsets, deletes both connectors and ends `stopped` (or `failed`, with connectors still removed). Verified: a later production Start re-reads the whole table. |
| F-06 | PG→PG snapshot copy used positional `SELECT *` / `COPY tgt FROM STDIN`, failing on any extra target column — including the sink's own `__deleted`. Now copies shared columns by name, skipping generated columns. |
| F-07 | Snapshot copy errors said only `db error`; COPY/TRUNCATE/COUNT failures now include the Postgres message, detail and hint. |
| F-04 | 29 `/cdc` path entries were missing from `docs/openapi.yaml` (compare, audit, pipeline-logs, profile sub-resources, migration tools, connector plugins/topics, consumer-group offsets, schema compatibility/versions). |

### Suggested fix order

1. **B-02** (WAL disk-fill risk on customer sources) — `delete_profile` must always delete
   connectors regardless of `stop_deletes_connector`, propagate errors, and drop the Debezium
   slot (`slot.drop.on.stop=true` on delete, or `pg_drop_replication_slot`).
2. **B-01** — qualify `table.name.format` with the mapped schema and `CREATE SCHEMA IF NOT
   EXISTS` on the target before upserting the sink.
3. **B-03** — use an explicit `topics=` list (names are deterministic: `prefix.schema.table`)
   or `consumer.override.metadata.max.age.ms=10000`.
4. **B-04**, **B-05**, then the status-code fixes.
After each fix, remove the matching `test.fail` and enable the `test.fixme` row-count check.
