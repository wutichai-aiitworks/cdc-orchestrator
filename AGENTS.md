# AGENTS — hyper-cdc-orchestrator

Instructions for AI coding agents working with the CDC control plane.

## Always follow

1. **Orchestrator is the CDC gateway.** Do not talk to Kafka Connect, Redpanda, or Docker APIs directly from agents or business services.
2. **Prefer MCP over WebUI** when `mcp-cdc` is running (`http://127.0.0.1:8410/mcp`). Use tools for status, tables, records, and pipeline create/start.
3. **Prompt-friendly create:** `cdc_pipeline_create` (host/db/user/pass/schemas/tables). Default pattern is **`source_to_target`** (database replica). Avoid raw profile JSON unless the user supplies it.
4. **Writes** need `ALLOW_WRITE_TOOLS=true` on `mcp-cdc` and a write-scoped client key (`MCP_CLIENT_KEYS`, ADR 0008). Keep writes off in shared/prod unless needed. Copy & Sync: `cdc_pg_copy_create` saves only; `cdc_copy_run` starts.
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
| `docs/adr/0004-incremental-rolling-window-and-initial-load.md` | Incremental per-table poll strategy (rolling window / full copy), streaming initial load, why not COALESCE/triggers |
| `docs/adr/0007-rsync-file-sync-sidecar.md` | File sync (rsync over SSH): direct via forwarded key, relay fallback, host-key pinning, macOS rsync detection |
| `docs/adr/0012-rsync-daemon-over-wireguard.md` | File sync with `rsync://` daemon endpoints (module, RSYNC_PASSWORD, relay only), rsyncd.conf for a tunnel |
| `docs/adr/0015-notification-channels.md` | Alert channels (webhook/email/LINE/Slack/Discord/Teams), encrypted write-only secrets (`ORCH_SECRET_KEY`), queued delivery + log, conditions, cooldown |
| `docs/adr/0016-pipeline-latency.md` | Per-pipeline latency: estimate (Kafka/Connect), trigger inbox, opt-in heartbeat probe; `/cdc/latency`, rollups, `latency_above` alert |
| `docs/adr/0017-mcp-oauth-and-full-api-coverage.md` | mcp-cdc OAuth bridge for claude.ai / ChatGPT (consent with an MCP key), `cdc_api_catalog` / `_get` / `_call`, pipelines by name |
| `docs/roadmap/gluesync-gap-roadmap.md` | Competitive gap plan vs GlueSync (plan only, R0–R4) |
| `docs/adr/0008-mcp-per-client-keys-and-hardening.md` | mcp-cdc for other clients: per-client keys/scopes, error shape, sessions, redaction |
| `docs/adr/0002-native-ui-local-auth.md` | Native WebUI login (`AUTH_MODE=local`), threat model, decisions |
| `docs/adr/0011-app-mode.md` | `APP_MODE=Production\|Maintenance\|Enhanced` (the one mode variable), corner ribbon, Production-default hardening |
| `docs/patterns/deployment-mode/SKILL.md` | Reusable version of APP_MODE for other repos: contract, gotchas, templates (Rust/Python/Go/Nuxt/ops), agent prompt |
| `docs/patterns/break-glass-admin/SKILL.md` | Reusable break-glass admin port (`127.0.0.1:4321`, local login beside SSO, same container): contract, gotchas, templates, agent prompt |
| `docs/adr/0013-record-diff.md` | Record diff (source vs target by key, per table, async): hash-partitioned scan, canonical values, recheck by key, why not ORDER BY merge |
| `docs/adr/0014-cdc-start-position.md` | Start position for log-based capture (`latest` / `earliest_retained` / custom LSN·binlog·SCN), one-shot marker, seeding via Connect `PATCH /offsets`, Reposition, why resync is refused |
| `docs/testing/api-test-plan.md` | How to run tests, coverage map, **known-bug register** (B-xx) and fix order |

## Demo defaults (compose profile `demo`)

Source `cdc-postgres-source`, target `cdc-postgres-target`, db/user/pass `demo`, schema `bookings`.

## Native WebUI (Nuxt, `:4320`)

1. **One UI tree: `cdc-native-ui/`.** All UI work goes there only (decided 2026-11-07). `kafbat-webui/` is frozen: do **not** copy changes into it, and do not delete it unless asked. `scripts/check-native-ui-parity.sh` is legacy (no longer part of `npm run check`).
2. **Auth modes:** `AUTH_MODE = none | local | sso`. `local` = env-only `ADMIN_USERNAME` / `ADMIN_PASSWORD` (default `admin` / `admin`). Decisions and rejected options live in ADR 0002 — don't re-propose forced password change, hashing, or public exposure unless the deployment model changes.
3. **Break-glass admin (`127.0.0.1:4321`):** second process in the `cdc-native-ui` container (`start-ui.mjs`, `ADMIN_PORT=3001`), `AUTH_MODE=local` fixed, loopback only, own cookie `cdc-admin-session`. Use when SSO is down; remote via `ssh -L 4321:127.0.0.1:4321`. Never publish it on `0.0.0.0` (ADR 0002 amendment).
3a. **Threat model:** internal/VPN, single operator, direct `:4320`. Lockout keys on socket IP; `TRUST_PROXY=true` only behind a proxy that overwrites `X-Forwarded-For`.
4. **Orchestrator token:** `docker/install.sh` generates `ORCH_API_TOKEN` into `docker/.env` if missing. Host-side callers of `:9788` need `Authorization: Bearer $ORCH_API_TOKEN`.
5. **UI conventions:** icons `class="h-4 w-4" :stroke-width="1.75"`; brand chips `bg-brand-500/10 text-primary`; focus `ring-1 ring-ring`; popovers teleport to body (see `AppInfoHint`, `AppUserMenu`); reuse `.icon-button`, `.avatar`, `.menu-panel`, `.menu-item`, `.auth-input`, `.btn-primary`, `.alert-*` instead of ad-hoc classes.
6. **Verify:** `npm run build` then run `.output` with `AUTH_MODE=local node map-env.mjs ./.output/server/index.mjs` and exercise `/api/auth/login`, `/api/auth/me`, `/api/auth/logout`. Don't commit unless asked.
6a. **List → detail pages:** a row/card opens a detail *page* via the URL (`/pipelines?pipeline=<id>`), not a panel under the list. Pages add trailing breadcrumb crumbs with `usePageCrumbs()` (clear on unmount); menu crumbs in the top bar link to the page's landing view and fire `useNavReselect` on same-path clicks. Client-side lists page with `components/ui/AppListPager.vue` (`v-model:page`, `v-model:page-size`).
6b. **Verify Record tab** (`components/RecordDiffPanel.vue`, replica pipelines; API/code name stays "record diff"): `POST /cdc/profiles/{id}/record-diff` per table, polls while active, pages results with `AppListPager`, downloads stream through `server/api/.../record-diff/[jobId]/download.get.ts`. Engine: `src/record_diff.rs`; values come canonical from jdbc-admin (`/v1/stream` `columns`+`canonical`, `/v1/lookup`) — rebuild `cdc-jdbc-admin` when changing either side. Rules in ADR 0013.
7. **Target DDL ("Create table" in the mapping step):** one generator, `utils/targetDdl.ts` (types via `utils/dbTypeTranslator.ts`) in `cdc-native-ui`. Runs through `POST /cdc/profiles/catalog/target/ddl` (create-only guard, one statement per call, audited as `target.create_tables`). Rules (ADR 0003): Oracle → Postgres lowercases every name like `fold_oracle_postgres_target`; identity/sequence defaults dropped; no foreign keys; expression/partial indexes become `-- NOTE` lines, never guesses. New source metadata (size/scale/default/indexes) needs a rebuilt `cdc-jdbc-admin` image.

## Testing

1. **Offline gate:** `npm run check` — OpenAPI route coverage (`scripts/check-openapi-routes.mjs`), UI unit tests (`npm run test:unit`: `node --test` on `tests/unit/*.test.ts`, no framework), Rust unit tests (orchestrator + `mcp/`). jdbc-admin: `cd docker/jdbc-admin && mvn -q test` (H2). Must pass before finishing any change.
2. **Live API:** `npm run test:api` (strict, self-cleaning `pw-` fixtures). `npm run test:api:e2e` also starts pipelines against demo DBs. Engine up/restart/down only with `ORCH_TEST_ALLOW_ENGINE=true` — never on a shared stack.
3. **New `/cdc` route ⇒ document it** in `docs/openapi.yaml` (the route check fails otherwise), then `scripts/generate-openapi.sh` (rewrites `docs/openapi.json` + `cdc-native-ui/public/openapi.json`).
4. **Known bugs are `test.fail(...)`** tied to IDs in `docs/testing/api-test-plan.md`. When you fix one, remove its annotation (the test turns red until you do) and update the register.
5. Never point tests at objects you didn't create; the stack may hold real customer profiles.
6. **Testing undeployed backend changes:** run an isolated orchestrator — memory-only store (no `ORCH_RQLITE_URL` ⇒ zero profiles, nothing to reconcile), Connect/ksql/registry on a dead port, no Docker socket — plus `java -jar docker/jdbc-admin/target/cdc-jdbc-admin-1.0.0.jar` with `JDBC_ADMIN_HOST=127.0.0.1 JDBC_ADMIN_PORT=18084`. Launch from a **bash** script with explicit `env -i VAR=…` (zsh does not word-split `$vars`, so a packed env string silently falls back to the real `localhost:28083` Connect). Check the startup line shows `connect_base_url=http://127.0.0.1:9` before sending requests. Then `ORCH_TEST_BASE_URL=http://127.0.0.1:<port> ORCH_TEST_DB_HOST=localhost npx playwright test <spec>`.

### Short prompts that work

| Say | Agent does |
|-----|-----------|
| "change admin password / username" | Set `ADMIN_PASSWORD` / `ADMIN_USERNAME` in `docker/.env`, recreate `cdc-native-ui` |
| "SSO is down / break-glass login" | Tunnel `ssh -L 4321:127.0.0.1:4321 <host>`, open `http://127.0.0.1:4321`, log in with `ADMIN_*`; if refused, check `docker logs cdc-native-ui \| grep '\[admin\]'` |
| "enable local login" | Set `AUTH_MODE=local` in `docker/.env`, recreate `cdc-native-ui` |
| "native ui behind proxy" | Set `TRUST_PROXY=true`; re-check ADR 0002 "Revisit when" |
| "grill native ui auth" | Re-read ADR 0002, interview only on what changed since |
| "run all tests" | `npm run check` then `npm run test:api`; report pass/fail + any `test.fail` that flipped |
| "run e2e" | `npm run test:api:e2e`, then verify no `pw-` leftovers (profiles, connectors, slots, topics, schemas) |
| "fix B-02" (any B-xx) | Read the register row, fix, remove its `test.fail`, run `check` + `test:api:e2e`, update register |
| "what's not tested" | Read "Not covered yet" in the test plan and re-check against current routes |
