# mcp-cdc

HTTP MCP gateway for **hyper-cdc-orchestrator**. Agents operate CDC by prompting — status, tables, records, and pipeline configuration — without the WebUI.

Pair with orchestrator OpenAPI (`docs/openapi.yaml`) and setup notes (compendium Ch.15,
`docs/architecture/hyper-cdc-orchestrator-design.html#mcp`).

## Tools

Start with `cdc_help` or `cdc_overview`. Prefer prompt-friendly tools over raw orchestrator JSON.

### Read
- `cdc_help`, `cdc_overview`, `cdc_health`, `cdc_capability`, `cdc_engine_status`, `cdc_cluster`, `cdc_metrics`
- `cdc_profile_list`, `cdc_profile_get`, `cdc_profile_dlq`, `cdc_tables`
- `cdc_connector_list`, `cdc_connector_status`, `cdc_connector_timeline`, `cdc_connector_templates`
- `cdc_topics_list`, `cdc_kafka_topics`, `cdc_topic_messages`, `cdc_topic_metadata`
- `cdc_consumer_groups`, `cdc_kafka_lag`
- `cdc_source_validate`, `cdc_target_validate` (demo defaults: `cdc-postgres-source` / `cdc-postgres-target`, db/user/pass `demo`)
- `cdc_alert_history`, `cdc_notification_rules_list`, `cdc_logs_tail`
- `cdc_processing_pipelines_list`, `cdc_processing_pipeline_get`

### Write (gated by `ALLOW_WRITE_TOOLS`)
- `cdc_pipeline_create` — prompt-friendly create (host/db/user/pass/schemas/tables; optional `sink_error_policy`, `validate` + `start`)
- `cdc_engine_up` / `down` / `restart`
- `cdc_profile_upsert`, `cdc_profile_enable`, `cdc_profile_action`
- `cdc_connector_validate` / `create` / `update` / `action` / `delete`
- `cdc_topic_create` / `delete` / `recreate` / `clear_messages`
- `cdc_notification_rule_upsert` / `delete`
- `cdc_processing_pipeline_upsert` / `delete`

### Prompts
- `cdc_status` — engine, connectors, topics, alerts
- `cdc_tables` — captured schemas/tables
- `cdc_inspect_records` — sample topic/table records
- `cdc_configure_pipeline` — create (and optionally start) a pipeline from conversation

## Environment

| Variable | Default | Role |
|----------|---------|------|
| `MCP_BIND_ADDR` | `0.0.0.0:8410` | Listen address |
| `MCP_BEARER_TOKEN` | placeholder | MCP client auth (≥24 chars) |
| `CDC_ORCHESTRATOR_URL` | `http://127.0.0.1:9788` | Orchestrator base |
| `CDC_ORCHESTRATOR_TOKEN` | unset | Forwarded as `Authorization: Bearer` to orchestrator |
| `ALLOW_WRITE_TOOLS` | `false` | Enable mutating tools (set `true` for local prompting) |

See `docs/env.mcp.example` and `.env_local` (cargo). Compose uses the repo-root `.env`.

## Run

From **hyper-cdc-orchestrator** (this crate lives in `mcp/`):

```bash
# from hyper-cdc-orchestrator/
# .env: ALLOW_WRITE_TOOLS=true
./install.sh --mcp --demo
```

Or cargo from this directory:

```bash
./start.sh --build .env_local
```

### Cursor MCP snippet

```json
{
  "mcpServers": {
    "hyper-cdc": {
      "url": "http://127.0.0.1:8410/mcp",
      "headers": {
        "Authorization": "Bearer <MCP_BEARER_TOKEN>"
      }
    }
  }
}
```

Reload Cursor MCP after saving. Then prompt: “What’s the CDC status?”, “Which tables are captured?”, “Show recent records for bookings.airports_data”, “Create a source-to-target pipeline for schema bookings and start it.”
