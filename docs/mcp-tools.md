# mcp-cdc — tools for chat clients

`mcp-cdc` exposes **80 tools** (50 read-only, 30 write) and 4 prompts to MCP chat clients
(Claude, ChatGPT, Cursor, …). This list is generated from the server's own `tools/list`. Setup, keys and client
connection: [mcp/README.md](../mcp/README.md); OAuth for claude.ai / ChatGPT: ADR 0017; keys and scopes: ADR 0008.

## In the web UI

Sidebar **MCP** (ADR 0021):

| Page | Shows |
|---|---|
| **Overview** (`/mcp`) | mcp-cdc status (version, write switch, client keys by scope, sessions, rate limit, OAuth, public URL), connection snippets per client, how to manage keys, the daily check prompt |
| **Tools** (`/mcp/tools`) | This list, live from the server: search, Read/Write filter, arguments per tool, example prompts |
| **Activity** (`/mcp/activity`) | Actions MCP clients performed (orchestrator audit, actor `mcp:<client>`) |

The pages read mcp-cdc `GET /info` (non-secret) through the UI server, which needs `MCP_URL`
(default `http://mcp-cdc:8410`) and the same orchestrator token as mcp-cdc. Keys are never shown
or edited in the UI.

## How access works

| | Read tools (R) | Write tools (W) |
|---|---|---|
| Client key scope `read` | ✅ | ❌ refused |
| Client key scope `write` + `ALLOW_WRITE_TOOLS=true` | ✅ | ✅ |

- Any tool that takes a pipeline accepts its **name** or its id (e.g. `"CollabX Sync"`).
- Secrets are never returned (passwords, keys and tokens come back as `********`, or are left out).
- Write tools change things; the server tells the model to confirm destructive actions (delete, stop, resync, clear) with you first.
- Production recommendation: give chat clients a **read** key; use a write key only for operators.

## Example prompts

| Ask | Tools the model uses |
|---|---|
| "Is the source of Orders Sync healthy? Is the slot holding WAL?" | `cdc_source_health` |
| "What must the DBA fix before we start CDC on this source?" | `cdc_source_readiness` |
| "Are the SSH tunnels up? Which pipelines use them?" | `cdc_ssh_tunnels` |
| "Can the tunnel reach 127.0.0.1:5432?" | `cdc_ssh_tunnel_probe` |
| "Is the Redis real-time sync still running?" | `cdc_redis_sync_status` |
| "Which pipeline is slowest right now?" / "latency of X over 24h" | `cdc_latency`, `cdc_pipeline_latency` |
| "What failed in the last hour?" | `cdc_audit_events`, `cdc_pipeline_logs` |
| "Alert me on LINE when a connector fails" | `cdc_notification_channel_upsert`, `cdc_notification_rule_upsert` (write) |
| "Run a record diff for X" | `cdc_api_catalog` → `cdc_api_call` (write) |

### Daily rollout check (paste into the chat)

> Check production CDC: run cdc_source_health for every running pipeline, cdc_ssh_tunnels and
> cdc_redis_sync_status. List only warnings, with what to do. Do not change anything.

## Production monitoring (rollout)

Answers "is everything OK?" during and after a customer rollout. See production-rollout-guideline.md §5.

| Tool | R/W | What it does |
|---|:-:|---|
| `cdc_source_health` | R | One-call health of a pipeline's SOURCE for production monitoring: pipeline status and last error, current latency/backlog, committed log position, replication-slot and WAL-retention checks (slot active, WAL held by the slot, max_slot_wal_keep_size, free senders) and the SSH tunnel state when the source goes through one. |
| `cdc_source_readiness` | R | Live CDC readiness check of a pipeline's source (ADR 0019): wal_level, slots, WAL senders, REPLICATION login, publication coverage, replica identity, table SELECT (and MySQL/SQL Server/Oracle equivalents). |
| `cdc_ssh_tunnels` | R | SSH tunnels (ADR 0020) with live state: connected or not, last error, the stable forward ports (cdc-sshtunnel:22xxx → database as seen from the SSH server), open connections and which pipelines use each tunnel. |
| `cdc_ssh_tunnel_probe` | R | Dial a database address through a running SSH tunnel (host/port as seen from the SSH server, e.g. 127.0.0.1:5432). |
| `cdc_ssh_tunnel_reconnect` | **W** | Reconnect an SSH tunnel now instead of waiting for the backoff (up to 30 s). |
| `cdc_redis_sync_status` | R | Redis → Redis sync (RedisShake) status per profile: whether it is configured for real time (live AOF stream) or one-time, the latest run's status, start/finish time and error. |

## Start here / overview

| Tool | R/W | What it does |
|---|:-:|---|
| `cdc_help` | R | Catalog of CDC MCP tools and example prompts. |
| `cdc_overview` | R | One-shot status: health, capability, engine, connectors, and topic summary. |
| `cdc_health` | R | Orchestrator health check (GET /health). |
| `cdc_capability` | R | Feature flags and stack capability. |
| `cdc_engine_status` | R | Redpanda/Connect/engine container status. |
| `cdc_cluster` | R | Kafka/Redpanda cluster info. |
| `cdc_metrics` | R | Orchestrator metrics snapshot. |

## Latency, logs, audit

| Tool | R/W | What it does |
|---|:-:|---|
| `cdc_latency` | R | Current end-to-end latency of every running pipeline: e2e/capture/sink ms, backlog, throughput, method (heartbeat = exact, trigger_inbox = exact, estimate = upper bound with ~60 s resolution) and status (ok \| behind \| idle \| stopped). |
| `cdc_pipeline_latency` | R | Latency of one pipeline over a range: current value, p50/p95/max, and up to 60 points. |
| `cdc_latency_probe` | **W** | Turn the exact-latency heartbeat probe on or off for a pipeline (Postgres → Postgres, log-based). |
| `cdc_pipeline_logs` | R | Pipeline lifecycle log (start, sync, errors) from the orchestrator's store, newest first. |
| `cdc_audit_events` | R | Audit trail: who did what and whether it failed (pipeline actions, connector state changes, config edits). |
| `cdc_logs_tail` | R | Tail logs for a stack service (cdc-message, cdc-connect, cdc-orchestrator, ...). |
| `cdc_alert_history` | R | Recent CDC alerts. |

## Pipelines

| Tool | R/W | What it does |
|---|:-:|---|
| `cdc_profile_list` | R | List CDC pipelines/profiles. |
| `cdc_profile_get` | R | Get one CDC profile by id, including captured schemas/tables. |
| `cdc_profile_dlq` | R | Get a profile sink DLQ topic, policy, and current rejected-message count. |
| `cdc_tables` | R | List captured schemas/tables from profiles plus Kafka topics that represent those tables. |
| `cdc_source_validate` | R | Probe a source database connection before creating a pipeline. |
| `cdc_target_validate` | R | Probe a target database connection. |
| `cdc_pipeline_create` | **W** | Create a CDC pipeline from prompt-friendly fields (no web UI). |
| `cdc_profile_upsert` | **W** | Create a profile with a raw orchestrator body (write). |
| `cdc_profile_enable` | **W** | Enable or disable a profile (write). |
| `cdc_profile_action` | **W** | start/pause/resume/restart/stop a profile (write). |
| `cdc_pipeline_position` | R | Configured start position and the source log position (LSN / binlog / SCN) a pipeline last committed. |
| `cdc_pipeline_reposition` | **W** | Move a running log-based pipeline to another source position (write). |

## Copy & Sync (pg_dump, pgcopydb, pgloader, Mongo, Redis, files)

| Tool | R/W | What it does |
|---|:-:|---|
| `cdc_copy_profile_list` | R | List saved Copy & Sync profiles for a tool (passwords redacted). |
| `cdc_copy_profile_get` | R | Get one saved Copy & Sync profile (passwords redacted). |
| `cdc_copy_test_connection` | R | Test a database/Redis/SSH connection from the tool's sidecar (where the copy runs) — proves network path, login, server version. |
| `cdc_copy_jobs` | R | List runs for a Copy & Sync tool, or one run when job_id is given (status, progress, errors). |
| `cdc_copy_job_logs` | R | Log output of one Copy & Sync run. |
| `cdc_pg_copy_create` | **W** | Configure (save) a PostgreSQL → PostgreSQL copy profile (pgcopydb clone) from prompt-friendly fields. |
| `cdc_copy_run` | **W** | START a copy/sync run from a saved profile (write, can overwrite target data). |
| `cdc_copy_job_stop` | **W** | Stop a running Copy & Sync job (write). |
| `cdc_copy_profile_delete` | **W** | Delete a saved Copy & Sync profile (write). |
| `cdc_migration_tools_discover` | R | Catalog Copy & Sync tools: pgcopy, pgdump, pgloader, mongocopy, RedisShake, rsync. |
| `cdc_migration_tool_contract` | R | Profile schema, option defaults, and API map for a Copy & Sync tool. |

## Kafka Connect connectors

| Tool | R/W | What it does |
|---|:-:|---|
| `cdc_connector_list` | R | List Kafka Connect connectors. |
| `cdc_connector_status` | R | Connector + task state. |
| `cdc_connector_timeline` | R | Recent connector state transitions and task errors. |
| `cdc_connector_templates` | R | Starter connector config templates. |
| `cdc_connector_validate` | R | Validate a connector config before create/update. |
| `cdc_connector_create` | **W** | Create a Kafka Connect connector (write). |
| `cdc_connector_update` | **W** | Update connector config (write). |
| `cdc_connector_action` | **W** | pause/resume/restart a connector (write). |
| `cdc_connector_delete` | **W** | Delete a connector (write). |

## Kafka topics and consumers

| Tool | R/W | What it does |
|---|:-:|---|
| `cdc_topics_list` | R | Paged Kafka topics with partitions, replica health, message counts, and size. |
| `cdc_kafka_topics` | R | Simple topic name list (legacy). |
| `cdc_topic_messages` | R | Sample CDC/Kafka records from a topic (table stream). |
| `cdc_topic_metadata` | R | Topic partitions, high watermarks, and configs. |
| `cdc_consumer_groups` | R | Paged consumer groups. |
| `cdc_kafka_lag` | R | Consumer group lag by topic/partition. |
| `cdc_topic_create` | **W** | Create a Kafka topic (write). |
| `cdc_topic_delete` | **W** | Delete a Kafka topic (write). |
| `cdc_topic_recreate` | **W** | Recreate a Kafka topic (write). |
| `cdc_topic_clear_messages` | **W** | Clear messages on a DELETE-policy topic (write). |

## Alerts and notifications

| Tool | R/W | What it does |
|---|:-:|---|
| `cdc_notification_channels_list` | R | Notification channels (webhook, email, line, slack, discord, teams) with their destination; secrets show as ********. |
| `cdc_notification_channel_upsert` | **W** | Create (no id) or update (id) a notification channel. |
| `cdc_notification_channel_test` | **W** | Send a sample message through a saved channel and report delivered/failed with the error. |
| `cdc_notification_channel_delete` | **W** | Delete a notification channel (refused while a rule still uses it). |
| `cdc_notification_deliveries` | R | Notification delivery log, newest first: channel, rule, event, outcome (delivered \| failed \| dropped), attempts, error. |
| `cdc_notification_rules_list` | R | Notification rules. |
| `cdc_notification_rule_upsert` | **W** | Create or update a notification rule (write). |
| `cdc_notification_rule_delete` | **W** | Delete a notification rule (write). |

## Event processing

| Tool | R/W | What it does |
|---|:-:|---|
| `cdc_processing_pipelines_list` | R | List event-processing pipelines. |
| `cdc_processing_pipeline_get` | R | Get one processing pipeline. |
| `cdc_processing_pipeline_upsert` | **W** | Create or replace an event-processing pipeline (write). |
| `cdc_processing_pipeline_delete` | **W** | Delete an event-processing pipeline (write). |

## Engine (stack lifecycle)

| Tool | R/W | What it does |
|---|:-:|---|
| `cdc_engine_up` | **W** | Start the CDC engine stack (write). |
| `cdc_engine_down` | **W** | Stop the CDC engine stack (write). |
| `cdc_engine_restart` | **W** | Restart the CDC engine stack (write). |

## Anything else (full API)

Every documented `/cdc` operation, including tunnels, readiness, record diff and initial load.

| Tool | R/W | What it does |
|---|:-:|---|
| `cdc_api_catalog` | R | List documented orchestrator operations (method, path, summary, read/write) to find one no dedicated tool covers. |
| `cdc_api_get` | R | GET any documented /cdc path (read-only), e.g. /cdc/profiles/{id}/tables/sync-status. |
| `cdc_api_call` | **W** | POST / PUT / PATCH / DELETE any documented /cdc path (write). |

## Prompts

| Prompt | Use |
|---|---|
| `cdc_status` | Engine, connectors, topics, alerts in one answer |
| `cdc_tables` | Which schemas/tables are captured |
| `cdc_inspect_records` | Sample records from a topic/table |
| `cdc_configure_pipeline` | Create (and optionally start) a pipeline from conversation |

## Keeping this list current

Regenerate after adding tools from `tools/list` (or `GET /info`) on a running mcp-cdc. The web UI Tools page is always live.
The test `every_tool_is_dispatched` (mcp/src/main.rs) fails if a tool is advertised without a handler.
