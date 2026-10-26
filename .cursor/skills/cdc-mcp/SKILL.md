---
name: cdc-mcp
description: >-
  Operate hyper-cdc-orchestrator via MCP (mcp-cdc) without the WebUI — status,
  captured tables, topic records, and prompt-friendly pipeline create/start.
  Use when the user asks about CDC status, connectors, Kafka topics, lag,
  alerts, logs, sample records, or configuring a CDC pipeline from conversation.
---

# CDC MCP (no WebUI)

Prefer live **hyper-cdc** MCP tools over opening kafbat-webui or inventing `/cdc/*` JSON.

If MCP is not connected, tell the user to start `./install.sh --mcp --demo` and add Cursor MCP from the design compendium Ch.15 (`docs/architecture/hyper-cdc-orchestrator-design.html#mcp`).

## First tools

| User intent | Tool |
|-------------|------|
| What can I prompt? | `cdc_help` |
| Overall status | `cdc_overview` then `cdc_alert_history` |
| Captured schemas/tables | `cdc_tables` / `cdc_profile_list` |
| Sample records | `cdc_topics_list` (search) → `cdc_topic_messages` |
| Create pipeline from chat | `cdc_source_validate` → `cdc_pipeline_create` (`start: true` only for replica / `source_to_target`). |
| Start/pause/stop | `cdc_profile_action` |

## Configure from prompts

`cdc_pipeline_create` accepts host/db/user/pass/schemas/tables — not raw orchestrator bodies. Default pattern is **`source_to_target`** (database replica; this is the path that can Start).

| Intent | Pattern | Start now? |
|--------|---------|------------|
| Database replica | `source_to_target` | Yes |
| Event-first (Kafka topics) | `source_to_stream` | No (Phase 2) |
| Event-first + Iceberg from Kafka | `source_to_lake` | No (Phase 2b) |
| Analytics-first (OLake) | `source_to_olake` | No — save draft only (Phase 3) |

Never put OLake and Debezium (replica / Event-first) on the same source host/port/database. Iceberg-from-Kafka is `source_to_lake`, not OLake.

For sources **without WAL/binlog** and **large tables**, use `capture_mode=incremental` with `timestamp_column` (e.g. `updated_at`) and `incrementing_column` (e.g. `id`). Prefer `delivery_mode=micro_batch` and `interval_seconds`. Hard deletes are not captured — use `capture_mode=trigger` only on tables that must replicate deletes.

CDC payloads default to **Avro** via Schema Registry (`http://cdc-message:8081`). Topic sample tools decode Avro when registry is up.

Demo compose defaults (in-network hostnames):

- source `cdc-postgres-source:5432`, target `cdc-postgres-target:5432`
- database / username / password: `demo`
- typical schema: `bookings`

Ask for missing passwords. Do not print secrets in summaries.

Writes require `ALLOW_WRITE_TOOLS=true` on `mcp-cdc`.

## Do not

- Send users to the WebUI when MCP can answer
- Use `cdc_profile_upsert` unless they already have a full orchestrator body
- Reuse DataService MCP tools for CDC control-plane work
