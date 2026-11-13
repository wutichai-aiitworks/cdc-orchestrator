# mcp-cdc

HTTP MCP gateway for **hyper-cdc-orchestrator**. Agents operate CDC by prompting — status, tables, records, and pipeline configuration — without the WebUI.

Pair with orchestrator OpenAPI (`docs/openapi.yaml`) and setup notes (compendium Ch.15,
`docs/architecture/hyper-cdc-orchestrator-design.html#mcp`).

## Tools

Start with `cdc_help` or `cdc_overview`. Prefer prompt-friendly tools over raw orchestrator JSON.
Every tool that takes a pipeline accepts its **id or its name** (e.g. `"CollabX Sync"`).

### Health, latency, alerts (ADR 0015, 0016)
- `cdc_latency` — every running pipeline's end-to-end latency, backlog, throughput, status
- `cdc_pipeline_latency` — one pipeline over 1h…30d (p50 / p95 / max)
- `cdc_audit_events`, `cdc_pipeline_logs` — what happened and what failed
- `cdc_notification_channels_list`, `cdc_notification_deliveries`
- Write: `cdc_latency_probe` (exact heartbeat latency; `restart=true` applies it),
  `cdc_notification_channel_upsert` / `_test` / `_delete`

### Anything else (ADR 0017)
- `cdc_api_catalog` — search all documented `/cdc` operations (embedded from `docs/openapi.yaml`)
- `cdc_api_get` — call any `/cdc` GET
- `cdc_api_call` — call any `/cdc` POST / PUT / PATCH / DELETE (write-gated)

### Read
- `cdc_help`, `cdc_overview`, `cdc_health`, `cdc_capability`, `cdc_engine_status`, `cdc_cluster`, `cdc_metrics`
- `cdc_profile_list`, `cdc_profile_get`, `cdc_profile_dlq`, `cdc_tables`
- `cdc_connector_list`, `cdc_connector_status`, `cdc_connector_timeline`, `cdc_connector_templates`
- `cdc_topics_list`, `cdc_kafka_topics`, `cdc_topic_messages`, `cdc_topic_metadata`
- `cdc_consumer_groups`, `cdc_kafka_lag`
- `cdc_source_validate`, `cdc_target_validate` (demo defaults: `cdc-postgres-source` / `cdc-postgres-target`, db/user/pass `demo`)
- `cdc_alert_history`, `cdc_notification_rules_list`, `cdc_logs_tail`
- `cdc_processing_pipelines_list`, `cdc_processing_pipeline_get`
- Copy & Sync: `cdc_copy_profile_list`, `cdc_copy_profile_get`, `cdc_copy_test_connection`, `cdc_copy_jobs`, `cdc_copy_job_logs`

### Write (gated by `ALLOW_WRITE_TOOLS`)
- `cdc_pipeline_create` — prompt-friendly create (host/db/user/pass/schemas/tables; optional `sink_error_policy`, `validate` + `start`)
- `cdc_engine_up` / `down` / `restart`
- `cdc_profile_upsert`, `cdc_profile_enable`, `cdc_profile_action`
- `cdc_pipeline_position` (read) / `cdc_pipeline_reposition` (write) — source log position (LSN / binlog / SCN); `cdc_pipeline_create` also takes `start_position` (ADR 0014)
- `cdc_connector_validate` / `create` / `update` / `action` / `delete`
- `cdc_topic_create` / `delete` / `recreate` / `clear_messages`
- `cdc_notification_rule_upsert` / `delete`
- `cdc_processing_pipeline_upsert` / `delete`
- `cdc_pg_copy_create` — save a Postgres → Postgres copy (pgcopydb) profile and test both sides; never starts it
- `cdc_copy_run` — start a Copy & Sync run (only when the user asks), `cdc_copy_job_stop`, `cdc_copy_profile_delete`

### Prompts
- `cdc_status` — engine, connectors, topics, alerts
- `cdc_tables` — captured schemas/tables
- `cdc_inspect_records` — sample topic/table records
- `cdc_configure_pipeline` — create (and optionally start) a pipeline from conversation

## Environment

| Variable | Default | Role |
|----------|---------|------|
| `MCP_BIND_ADDR` | `127.0.0.1:8410` (compose: `0.0.0.0:8410` in-container) | Listen address |
| `MCP_CLIENT_KEYS` | unset | Per-client keys `name:scope:token,...`, scope `read` \| `write` (ADR 0008) |
| `MCP_BEARER_TOKEN` | unset | Legacy single key = client `default`, write scope. `install.sh` generates it |
| `MCP_ALLOW_WEAK_TOKENS` | `false` | Local dev only: allow keys < 24 chars / placeholders (otherwise startup fails) |
| `CDC_ORCHESTRATOR_URL` | `http://127.0.0.1:9788` | Orchestrator base |
| `CDC_ORCHESTRATOR_TOKEN` | unset | Forwarded as `Authorization: Bearer` to orchestrator |
| `ALLOW_WRITE_TOOLS` | `false` | Global write switch; a client also needs a `write`-scoped key |
| `MCP_RATE_LIMIT_RPM` | `300` | Requests per minute **per client** |
| `MCP_SESSION_TTL_SECS` / `MCP_MAX_SESSIONS` | `86400` / `1000` | Idle session expiry / cap (oldest evicted) |
| `MCP_CORS_ORIGINS` | unset | Browser origins allowed (comma list) |

| `MCP_OAUTH` | `true` | OAuth bridge for claude.ai / ChatGPT connectors (ADR 0017) |
| `MCP_PUBLIC_URL` | unset | Public https base URL, e.g. `https://cdc-mcp.example.com` (needed behind a proxy) |
| `MCP_OAUTH_STORE` | unset (compose: `/data/oauth-store.json`) | Persist OAuth clients + token hashes across restarts |
| `MCP_OAUTH_ACCESS_TTL_SECS` / `MCP_OAUTH_REFRESH_TTL_SECS` | `3600` / `2592000` | Token lifetimes |

## Connect a client

Every client uses the same endpoint, `https://<your-host>/mcp`. Give each client its own key in
`MCP_CLIENT_KEYS` (`name:read|write:token`, `openssl rand -hex 24`), so the audit log and
revocation are per client.

**Claude Code** (header key):

```bash
claude mcp add --transport http cdc https://cdc-mcp.example.com/mcp \
  --header "Authorization: Bearer <key>"
```

Leave out `--header` to use the OAuth flow instead (`/mcp` in Claude Code → authenticate).

**claude.ai and Claude Desktop** (OAuth): Settings → Connectors → Add custom connector →
URL `https://cdc-mcp.example.com/mcp` → Connect. A page from this server asks for an MCP client
key; paste one to approve. The connector then acts as that client and with its scope.

**ChatGPT** (OAuth): with developer mode enabled (Settings → Apps & Connectors → Advanced),
create a connector with the same URL and OAuth authentication, then approve with a key as above.
Availability depends on the ChatGPT plan.

**Cursor, VS Code, Windsurf and other clients** that accept headers:

```json
{ "mcpServers": { "cdc": { "url": "https://cdc-mcp.example.com/mcp",
  "headers": { "Authorization": "Bearer <key>" } } } }
```

stdio-only clients can bridge with `npx mcp-remote https://cdc-mcp.example.com/mcp --header "Authorization: Bearer <key>"`.

**Exposing the server.** claude.ai and ChatGPT connect from the internet, so put mcp-cdc behind
https: for example a Cloudflare tunnel hostname → `mcp-cdc:8410`, then set
`MCP_PUBLIC_URL=https://<that hostname>`. Header clients on the LAN can use `http://host:8410/mcp`.

### Production notes (ADR 0008)

- Give each client its own key (`openssl rand -hex 24`); read-only clients get `read`. Rotate by editing `MCP_CLIENT_KEYS` and restarting.
- Orchestrator audit entries show the client as `actor=mcp:<name>`.
- Tool failures come back as results with `isError: true`; protocol errors are JSON-RPC errors.
- All tool output and error text is secret-redacted before it leaves the server.
- Plain HTTP: terminate TLS at a proxy. Sessions are in memory — with several replicas, route sticky on `mcp-session-id` (clients re-initialize on 404).

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
