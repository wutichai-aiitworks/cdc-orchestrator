# hyper-cdc-orchestrator

Unified Rust CDC control plane for the Debezium/Kafka Connect stack. Powers `hyper-dataservice-webui` (`/cdc/*`) and the in-tree MCP gateway (`mcp/`).

## Is it ready to use?

**Yes for single-host Docker** (dev immediately; production after `ORCH_ENV=prod` + soak test).

Not for multi-tenant or multi-node HA.

## Complete install (Docker Compose)

One command starts Redpanda, Debezium Connect, Kafbat, Redis, and the orchestrator:

```bash
cp docker/.env.example docker/.env
./install.sh --native-ui
# optional: ./install.sh --native-ui --demo   # demo DB source :25432 + target :25433
# optional: ./install.sh --mcp --demo         # agent MCP at :8410 (compendium Ch.15)
```

Docker assets live in **[`docker/`](docker)** — `docker-compose.yaml`, `Dockerfile`,
`connect/`, mounted `config/`, Postgres `init/`, `.env` / `.env.example`, and the
install/dev scripts. `./install.sh` and friends at the root are thin wrappers. For
ad-hoc compose commands, `cd docker && docker compose ...` (`.env` is loaded
automatically because it sits next to the compose file).

Full guide: **[INSTALL.md](INSTALL.md)**  
Production hardening: **[compendium Ch.18](docs/architecture/hyper-cdc-orchestrator-design.html#security)**

| Endpoint | URL |
|----------|-----|
| Orchestrator health | http://127.0.0.1:9788/health |
| Connect REST | http://127.0.0.1:28083/ |
| Kafbat UI | http://127.0.0.1:4321 |
| Native Level-3 UI *(optional profile)* | http://127.0.0.1:4320 |
| Demo Postgres source *(optional `--demo`)* | `127.0.0.1:25432` (`cdc-postgres-source:5432`) |
| Demo Postgres target *(optional `--demo`)* | `127.0.0.1:25433` (`cdc-postgres-target:5432`) |
| Capability | http://127.0.0.1:9788/cdc/capability |
| MCP gateway *(optional `--mcp`)* | http://127.0.0.1:8410/mcp |

### Level 2: Forked Kafbat UX

`cdc-admin` can run your own Kafbat fork image without compose edits:

```bash
# .env
CDC_ADMIN_IMAGE=ghcr.io/<your-org>/kafka-ui:ux-v1
```

Then restart:

```bash
cd docker && docker compose up -d
```

Detailed workflow: [compendium Ch.14](docs/architecture/hyper-cdc-orchestrator-design.html#webui)

## Scope

- Engine lifecycle: `up`, `down`, `restart` (Docker API)
- Kafka Connect connector CRUD + task restart
- CDC **profiles** (including drafts) persisted in rqlite, with Connect reconcile
- Kafka topics / consumer groups / lag (Redpanda `rpk`)
- Docker log snapshot + SSE, Connect loggers
- Alert rules (rqlite), in-memory history, WebSocket fan-out, webhooks
- Prometheus `/cdc/metrics`
- Kafka-native Transform → Filter → Enrich pipeline runtime with batched gRPC functions,
  DataService mapper enrichment, bounded retries, DLQ, and replay by consumer group

## API

- Canonical OpenAPI: [`docs/openapi.yaml`](docs/openapi.yaml)
- Bundled OpenAPI JSON: [`docs/openapi.json`](docs/openapi.json)
- Native Scalar reference: `http://127.0.0.1:4320/api-docs` when the `native-ui` profile is enabled
- Prefer **`/cdc/*`** for all new clients

## Level 3 Native WebUI

- **Design compendium — every document in one file, with diagrams:**
  [`docs/architecture/hyper-cdc-orchestrator-design.html`](docs/architecture/hyper-cdc-orchestrator-design.html)
- Frontend workspace: [`kafbat-webui/`](kafbat-webui)
- Milestone plan, cutover checklist, and Level-2 Kafbat fork workflow:
  compendium [Ch.14](docs/architecture/hyper-cdc-orchestrator-design.html#webui)
- Production readiness assessment: compendium [Ch.21](docs/architecture/hyper-cdc-orchestrator-design.html#readiness)
- Native UI architecture and operational flows: compendium
  [Ch.3](docs/architecture/hyper-cdc-orchestrator-design.html#context) and
  [Ch.9](docs/architecture/hyper-cdc-orchestrator-design.html#observability)
- API contracts and request/response examples: compendium
  [Ch.13](docs/architecture/hyper-cdc-orchestrator-design.html#http-api)
- Kafka-native function runtime and six-phase acceptance plan: compendium
  [Ch.10](docs/architecture/hyper-cdc-orchestrator-design.html#function-runtime)
- Function authoring, testing, FaaS packaging, and release methodology: compendium
  [Ch.11](docs/architecture/hyper-cdc-orchestrator-design.html#function-methodology)
- CI: `.woodpecker/` holds one workflow per file — `test` (both Rust crates), `webui`
  (`kafbat-webui` typecheck + build), `build` (all four images multi-arch, no push),
  `sonar` (quality gate on push), and `main-ci` (multi-arch publish, gated on the others)

## Kafka-native function runtime

The processor is a separate data-plane binary; the orchestrator remains the control plane.

```bash
# Remote Transform / Filter / Enrich FaaS
cargo run --example sample_function

# All-gRPC Transform → Filter → Enrich sample
export CDC_BOOTSTRAP_SERVERS=127.0.0.1:19092
cargo run --bin cdc-processor -- examples/customer-pipeline-faas.json

# Processor: raw Kafka topic → stages → trusted topic / DLQ
export CDC_BOOTSTRAP_SERVERS=127.0.0.1:9092
export DATA_SERVICE_API_KEY='replace-me'
cargo run --bin cdc-processor -- examples/customer-pipeline.json
```

- Protobuf contract: [`proto/cdc_function.proto`](proto/cdc_function.proto)
- Pipeline sample: [`examples/customer-pipeline.json`](examples/customer-pipeline.json)
- Standalone FaaS image: [`examples/Dockerfile.sample-function`](examples/Dockerfile.sample-function)
- FaaS Compose demo: [`examples/docker-compose.faas.yml`](examples/docker-compose.faas.yml)
- DataService mapper sample:
  [`examples/customer-enrichment-mapper.json`](examples/customer-enrichment-mapper.json)
- Configuration API: `/cdc/processing-pipelines`
- Native WebUI stage editor: `/processing`

The current store for processing-pipeline definitions is in-memory. Persistent definitions,
supervised deployment/replay jobs, mTLS, autoscaling, and canary rollout are explicit production
acceptance work documented in the runtime guide.

## Testing

### Local pre-build (`testops.sh`)

Same checks as the Woodpecker `test` workflow.

```bash
./testops.sh
RUN_SMOKE=1 ./testops.sh
RUN_SMOKE=1 SMOKE_BASE_URL=http://127.0.0.1:9788 ./testops.sh
```

`./testops.sh` runs `cargo fmt --check`, `cargo test --locked`, and `cargo clippy -D warnings` for the root crate and `mcp/`. `RUN_SMOKE=1` needs the orchestrator up and checks `GET /health`.

### Local SonarQube (`sonar.sh`)

```bash
export SONAR_LOCAL_TOKEN=squ_...
./sonar.sh
ALL_ISSUES=1 ./sonar.sh
```

Starts the shared local server on port 9900 when needed (`scripts/sonar-local-up.sh`) and analyses as `hyper-cdc-orchestrator-local`, so the CI project is left alone. CI scans `hyper-cdc-orchestrator` on push.

## Soak test

```bash
export ORCH_BASE_URL=http://127.0.0.1:9788
export ORCH_API_TOKEN='your-token'   # if set
./scripts/soak-test.sh
```

## Docker Compose test (recommended)

Compose reads **`docker/.env`**, not `.env_local`. The root `.env_local` is cargo-only —
`./start.sh` layers it on top of `docker/.env`.

Stop any local `./start.sh` first (it previously bound 4320), then:

```bash
./install.sh --native-ui
```

Equivalent:

```bash
cd docker && docker compose --profile native-ui up -d --build
```

Your Kafka/Connect stack is already on the compose host ports, so this mainly builds and starts `hyper-cdc-orchestrator` and `cdc-native-ui`.

| Endpoint | URL |
|----------|-----|
| Orchestrator | http://127.0.0.1:9788/health |
| Native UI (kafbat-webui) | http://127.0.0.1:4320 |
| Connect REST | http://127.0.0.1:28083 |
| Kafbat UI | http://127.0.0.1:4321 |
| Kafka (Redpanda) | 127.0.0.1:19092 |
| MCP gateway *(profile `mcp`)* | http://127.0.0.1:8410/mcp |

```bash
cd docker
docker compose logs -f cdc-orchestrator cdc-native-ui
docker compose --profile native-ui down   # stop; add -v to wipe volumes
```

## Local cargo + kafbat-webui (optional)

Use this only when iterating on Rust/Nuxt without rebuilding images. Do not run cargo `start.sh` while Compose native UI is on **4320**. Connect stays on **28083**.

```bash
./start.sh --build .env_local
cd kafbat-webui && ./start.sh --build .env_local
```

## Guardrails

- Compose project name must be `cdc-stack`
- Managed containers: `cdc-message`, `cdc-admin`, `cdc-connect`, `cdc-redis`, `cdc-vector`
- Mutating routes emit audit logs with `audit_id`
- Profile GET responses redact secrets (`***`)
