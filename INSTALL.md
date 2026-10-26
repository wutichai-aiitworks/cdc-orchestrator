# Install — hyper-cdc-orchestrator (complete Docker Compose package)

Single-host CDC stack: **Redpanda + Debezium Connect + ksqlDB + Kafbat + Redis + Orchestrator**.

| Ready? | Answer |
|--------|--------|
| Control plane ready to use? | **Yes** for single-host Docker (dev / hardened prod with `ORCH_ENV=prod`) |
| Needs Kafka Connect stack? | **Yes** — this compose file starts it |
| Multi-tenant / multi-node? | **No** (out of scope) |

## What you get

| Container | Role | Host port (default) |
|-----------|------|---------------------|
| `cdc-message` | Redpanda (Kafka API + Schema Registry + `rpk`) | `19092` / SR `18081` |
| `cdc-connect` | Debezium Kafka Connect (Avro default) | `28083` |
| `cdc-ksqldb` | ksqlDB (Avro + Schema Registry) | `28088` |
| `cdc-admin` | Kafbat UI | `4321` |
| `cdc-native-ui` *(optional)* | Level-3 native web UI | `4320` |
| `cdc-redis` | Redis | `26379` |
| `cdc-rqlite` | rqlite config store (pipelines, drafts, rules) | `14001` |
| `hyper-cdc-orchestrator` | CDC control plane API | `127.0.0.1:9788` |
| `cdc-vector` *(optional)* | Vector consumer | — |
| `cdc-postgres-source` *(optional)* | Demo Postgres source (`demo` / bookings dump) | `25432` |
| `cdc-postgres-target` *(optional)* | Demo Postgres target (`demo` / empty) | `25433` |
| `cdc-lake` *(optional)* | MinIO object storage for `source_to_lake` | `19000` / console `19001` |
| `mcp-cdc` *(optional)* | MCP agent gateway | `127.0.0.1:8410` |

Compose project name is **`cdc-stack`** (required for orchestrator engine lifecycle).

## Where the Docker files live

Everything Docker-only sits under `docker/`:

| Path | Role |
|------|------|
| `docker/docker-compose.yaml` | The stack definition |
| `docker/.env` | Live config — gitignored, created from `.env.example` on first install |
| `docker/.env.example` | Template |
| `docker/Dockerfile` | Orchestrator image (build context is the repo root) |
| `docker/connect/` | Custom Debezium Connect image (Avro converter + vendored JDBC source) |
| `kafka-connect-jdbc/` | Vendored Confluent JDBC source fork (10.9.7 + Oracle timezone patch); compiled by the Connect image |
| `docker/config/` | Files mounted into containers (`vector.toml`) |
| `docker/init/` | Demo Postgres bootstrap (`/docker-entrypoint-initdb.d`) |
| `docker/install.sh`, `docker/start.sh`, `docker/reload-dev.sh` | Real scripts; `./install.sh` etc. at the root are thin wrappers |

`.env` lives **next to** `docker-compose.yaml`, so Compose loads it automatically —
no `--env-file` needed. Only `.env_local` stays at the repo root, since it is used
solely by local cargo runs.

See [`docker/README.md`](docker/README.md) for the full layout and path conventions.

## Serialization (Avro default)

CDC source and JDBC sink connectors use **Avro** (`io.confluent.connect.avro.AvroConverter`) with Redpanda Schema Registry at `http://cdc-message:8081` (host `http://127.0.0.1:18081`). Connect is a custom image (`docker/connect/Dockerfile`) that adds the Confluent Avro converter to Debezium 2.7.

Existing pipelines keep their previous converter until you **Enable / Start / Restart** them so reconcile can rewrite connector config.

## ksqlDB

`cdc-ksqldb` listens on host `28088`. The native UI **KSQL DB** page talks to it through the orchestrator (`ORCH_KSQL_URL=http://cdc-ksqldb:8088`). Default value format is **Avro**. Example:

```sql
-- After a pipeline is running with Avro (Enable / Start / Restart)
CREATE STREAM cdc_raw WITH (KAFKA_TOPIC='demo.bookings.airplanes_data', VALUE_FORMAT='AVRO');
SELECT * FROM cdc_raw EMIT CHANGES LIMIT 10;
```

Existing JSON connectors need a restart before Avro `CREATE STREAM` can infer columns from Schema Registry. `PRINT 'topic' FROM BEGINNING LIMIT 1;` works on any topic.

## Prerequisites

- Docker Engine 24+ with Compose v2
- ~4 GB RAM free for the full stack
- Ports free: `9788`, `28083`, `4321`, `19092`, Schema Registry `18081`, ksqlDB `28088`, rqlite `14001` (and optional native UI `4320`, demo Postgres source `25432` / target `25433`, MCP `8410`)

## Quick install

```bash
cd hyper-cdc-orchestrator
# install.sh creates docker/.env from docker/.env.example on first run;
# copy it up front if you want to edit secrets first.
cp docker/.env.example docker/.env
# Optional: set ORCH_API_TOKEN for auth even in dev
./install.sh
```

With demo Postgres source **and** target:

```bash
./install.sh --demo
```

With a MinIO datalake warehouse (Iceberg/Parquet landing):

```bash
./install.sh --demo --lake
```

Create a lake pipeline after the stack is up (`pattern=source_to_lake`, endpoint `http://cdc-lake:9000`, bucket `lake`). See [compendium Ch.12](docs/architecture/hyper-cdc-orchestrator-design.html#datalake).

With Vector + demo:

```bash
./install.sh --demo --vector
```

With MCP (in-tree crate `mcp/`):

```bash
./install.sh --mcp
```

With native Level-3 web UI:

```bash
./install.sh --native-ui
```

## Manual compose

```bash
cd docker
cp .env.example .env
docker compose up -d --build

# optional profiles
docker compose --profile demo --profile vector up -d --build
```

Or from the repo root, without changing directory:

```bash
docker compose -f docker/docker-compose.yaml up -d --build
```

## Verify

```bash
curl -s http://127.0.0.1:9788/health
curl -s http://127.0.0.1:28083/ | head
curl -s http://127.0.0.1:9788/cdc/capability   # needs Bearer if token set

ORCH_BASE_URL=http://127.0.0.1:9788 \
ORCH_API_TOKEN="$ORCH_API_TOKEN" \
./scripts/soak-test.sh
```

## Wire WebUI

In Hyper CDC WebUI (`cdc-native-ui`) `.env` / compose:

```bash
ORCHESTRATOR_URL=http://127.0.0.1:9788
ORCHESTRATOR_TOKEN=<same as ORCH_API_TOKEN>
```

Open CDC WebUI → **CDC** nav.

## Wire MCP (Cursor / agents, no WebUI)

Uses in-tree `mcp/`. Local prompting needs writes:

```bash
# .env
ALLOW_WRITE_TOOLS=true
./install.sh --mcp --demo
```

Add to `~/.cursor/mcp.json` or project `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "hyper-cdc": {
      "url": "http://127.0.0.1:8410/mcp",
      "headers": {
        "Authorization": "Bearer change-me-mcp-cdc-token-min24chars"
      }
    }
  }
}
```

Use the same bearer as `MCP_BEARER_TOKEN`. Full guide: [compendium Ch.15](docs/architecture/hyper-cdc-orchestrator-design.html#mcp).

Then prompt: CDC status, captured tables, topic records, or “create a pipeline for schema bookings and start it.”


## Level 2: fork Kafbat for custom UX

Use your own Kafbat fork image for `cdc-admin`:

```bash
# .env
CDC_ADMIN_IMAGE=ghcr.io/<your-org>/kafka-ui:ux-v1
```

Deploy / refresh:

```bash
cd docker && docker compose up -d
```

Verify running image:

```bash
cd docker && docker compose ps --format json | jq -r '.[] | select(.Service=="cdc-admin") | .Image'
```

Full fork workflow: [compendium Ch.14](docs/architecture/hyper-cdc-orchestrator-design.html#webui)

## Production (single host)

```bash
# .env
ORCH_ENV=prod
ORCH_API_TOKEN=<random ≥24 chars>
ORCH_RQLITE_URL=http://cdc-rqlite:4001
ORCH_JDBC_ADMIN_URL=http://cdc-jdbc-admin:8084
HYPER_CDC_ORCHESTRATOR_BIND=127.0.0.1
```

See [compendium Ch.18](docs/architecture/hyper-cdc-orchestrator-design.html#security).

## Demo source → target (optional)

`--demo` starts two Postgres `demo` databases (user/password `demo` / `demo`). Source loads `samples/demo-{3m,6m,1y,2y}.sql.gz` (default `3m`). Target is an empty `demo` database for CDC to fill.

| Role | In-compose hostname | Host (from laptop) |
|------|---------------------|--------------------|
| Source | `cdc-postgres-source:5432` / db `demo` | `127.0.0.1:25432` |
| Target | `cdc-postgres-target:5432` / db `demo` | `127.0.0.1:25433` |

Pick a dump size in `.env` **before the first start** (init runs only on an empty volume):

```bash
CDC_DEMO_DUMP_SIZE=3m   # or 6m, 1y, 2y
```

To reload the **source** dump after a previous demo volume:

```bash
cd docker
docker compose --profile demo stop cdc-postgres-source
docker compose --profile demo rm -f cdc-postgres-source
docker volume rm cdc-stack_cdc-postgres-data
docker compose --profile demo up -d cdc-postgres-source
```

Source connector:

```bash
curl -s -X POST http://127.0.0.1:28083/connectors \
  -H 'Content-Type: application/json' \
  -d '{
    "name": "demo-bookings",
    "config": {
      "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
      "database.hostname": "cdc-postgres-source",
      "database.port": "5432",
      "database.user": "demo",
      "database.password": "demo",
      "database.dbname": "demo",
      "topic.prefix": "demo",
      "schema.include.list": "bookings",
      "plugin.name": "pgoutput",
      "publication.name": "dbz_publication",
      "slot.name": "debezium_demo"
    }
  }'
```

JDBC sink (writes `demo.bookings.*` into the target):

```bash
curl -s -X POST http://127.0.0.1:28083/connectors \
  -H 'Content-Type: application/json' \
  -d '{
    "name": "demo-bookings-sink",
    "config": {
      "connector.class": "io.debezium.connector.jdbc.JdbcSinkConnector",
      "tasks.max": "1",
      "topics.regex": "demo\\.bookings\\..*",
      "connection.url": "jdbc:postgresql://cdc-postgres-target:5432/demo?currentSchema=bookings",
      "connection.username": "demo",
      "connection.password": "demo",
      "insert.mode": "upsert",
      "delete.enabled": "true",
      "primary.key.mode": "record_key",
      "schema.evolution": "basic"
    }
  }'
```

Or use the native UI wizard / orchestrator connector templates (`postgres-source`, `jdbc-sink`).

## Stop / reset

```bash
cd docker
docker compose down
# wipe volumes (destroys Kafka/Redis/Postgres data)
docker compose down -v
```

## Troubleshooting

| Symptom | Check |
|---------|--------|
| Orchestrator unhealthy | `cd docker && docker compose logs cdc-orchestrator` |
| Connect 502 from API | `cd docker && docker compose logs cdc-connect`; wait for health |
| Engine status empty | Confirm project name `cdc-stack` and container names |
| Prod won't start | Token ≥24 chars + `ORCH_RQLITE_URL` required |
| MCP build fails | Confirm `mcp/` is present, or omit `--mcp` |
