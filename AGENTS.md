# AGENTS — hyper-cdc-orchestrator

Instructions for AI coding agents working with the CDC control plane.

## Always follow

1. **Orchestrator is the CDC gateway.** Do not talk to Kafka Connect, Redpanda, or Docker APIs directly from agents or business services.
2. **Prefer MCP over WebUI** when `mcp-cdc` is running (`http://127.0.0.1:8410/mcp`). Use tools for status, tables, records, and pipeline create/start.
3. **Prompt-friendly create:** `cdc_pipeline_create` (host/db/user/pass/schemas/tables). Default pattern is **`source_to_target`** (database replica). Avoid raw profile JSON unless the user supplies it.
4. **Writes** need `ALLOW_WRITE_TOOLS=true` on `mcp-cdc`. Keep that off in shared/prod.
5. **Docker assets live in `docker/`** (`docker-compose.yaml`, `Dockerfile`, `connect/`,
 mounted `config/`, Postgres `init/`, `.env` / `.env.example`, install/dev scripts).
 Run compose from that directory (`cd docker && docker compose ...`) so `.env` loads
 automatically. Relative paths resolve against `docker/`, so the repo root is `..`.
 Only `.env_local` stays at the repo root (cargo-only). The JDBC source connector
 fork lives at repo-root `kafka-connect-jdbc/` (Connect image compiles it).

## Read these first

| Doc | When |
|-----|------|
| `docs/architecture/hyper-cdc-orchestrator-design.html` | **Start here.** Every design doc in one file: architecture, APIs, deployment, readiness |
| ↳ Ch.6 `#source-to-target` | Current path: Postgres/MySQL/… replica, no Connect JSON |
| ↳ Ch.5 `#patterns` | Delivery order: replica now; stream and lake later |
| ↳ Ch.12 `#datalake` | Lakehouse CDC contract (deferred until replica is solid) |
| ↳ Ch.15 `#mcp` | Cursor MCP + local prompting |
| `.cursor/skills/cdc-mcp/SKILL.md` | Compact agent skill |
| `.cursor/rules/cdc-implementation.mdc` | Always-apply implementation standards (errors, state, env, auth, persistence, audit, CDC lifecycle) |
| `.cursor/rules/rust-patterns.mdc` | Rust conventions auto-loaded when editing `src/**/*.rs` or `mcp/src/**/*.rs` |
| `docs/openapi.yaml` | Full `/cdc/*` API |
| `mcp/README.md` | MCP crate / tool list |
| `INSTALL.md` | Compose install (`--mcp --demo`) + `docker/` layout |
| `.woodpecker/README.md` | CI workflows, published images, required secrets |

## Demo defaults (compose profile `demo`)

Source `cdc-postgres-source`, target `cdc-postgres-target`, db/user/pass `demo`, schema `bookings`.
