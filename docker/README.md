# docker/

Single-file deploy from `docker-compose.yaml`. Demo databases, MinIO lake, Vector, MCP,
and the native UI are compose profiles.

```bash
./install.sh --native-ui
./install.sh --demo --lake
./install.sh --mcp --demo
```

`./install.sh`, `./start.sh`, and `./reload-dev.sh` at the repo root are thin wrappers
that forward here.

## Layout

| Path | Role |
|------|------|
| `docker-compose.yaml` | The `cdc-stack` stack definition (project name must stay `cdc-stack`) |
| `.env` | Live config — **gitignored**, created from `.env.example` on first install |
| `.env.example` | Template; copy to `.env` and edit secrets |
| `Dockerfile` | Orchestrator + `cdc-processor` image; build context is the **repo root** |
| `connect/` | Custom Debezium Connect Dockerfile (Avro converter + JDBC source); **build context is the repo root** so it can compile `kafka-connect-jdbc/` |
| `jdbc-admin/` | JDBC Type 4 catalog/DDL/preview service (`cdc-jdbc-admin`) |
| `config/` | Files bind-mounted into containers (`vector.toml`) |
| `init/` | Demo Postgres bootstrap, mounted to `/docker-entrypoint-initdb.d` |
| `install.sh` | Stack installer |
| `start.sh` | Local cargo dev runner (`--stack` also brings up compose infra) |
| `reload-dev.sh` | Rebuild + restart the fast-moving services |

## Running compose directly

`.env` sits next to `docker-compose.yaml`, so Compose loads it automatically for
`${VAR}` interpolation — no `--env-file` needed. Run from this directory:

```bash
cd docker
docker compose ps
docker compose logs -f cdc-orchestrator
docker compose --profile demo up -d --build
docker compose down -v
```

From the repo root, point `-f` at the file; the project directory is still `docker/`,
so `docker/.env` is picked up the same way:

```bash
docker compose -f docker/docker-compose.yaml ps
```

Extra overlays are relative to this directory:

```bash
cd docker
docker compose -f docker-compose.yaml -f ../examples/docker-compose.faas.yml \
  --profile processing up -d
```

## Path conventions

Relative paths in `docker-compose.yaml` resolve against `docker/`:

- `./connect`, `./config/...`, `./init` — inside this directory
- `..` — repo root (orchestrator and Connect build context; Connect compiles `kafka-connect-jdbc/`)
- `../kafbat-webui`, `../mcp` — sibling source workspaces used as build contexts
- `../samples` — demo Postgres dumps

`dockerfile:` is relative to its own `context:`, which is why the orchestrator and
Connect services use `context: ..` with `dockerfile: docker/Dockerfile` and
`dockerfile: docker/connect/Dockerfile`.

## Env files

| File | Used by |
|------|---------|
| `docker/.env` | Compose (interpolation + orchestrator `env_file`), and `start.sh` |
| `docker/.env.example` | Template checked into git |
| `.env_local` *(repo root)* | Local cargo runs only, layered on top of `docker/.env` by `start.sh` |

## Profiles

| Profile | Adds |
|---------|------|
| *(none)* | Redpanda, Connect, ksqlDB, Kafbat, Redis, rqlite, orchestrator |
| `demo` | Demo Postgres source + target |
| `lake` | MinIO warehouse + bucket init |
| `vector` | Vector consumer |
| `mcp` | `mcp-cdc` agent gateway |
| `native-ui` | Level-3 Nuxt web UI |


# Rebuild the Connect image (compiles kafka-connect-jdbc/ from this repo)
cd docker
docker compose build cdc-connect && docker compose up -d cdc-connect


DROP TABLE HYPER_CDC_CHANGE_LOG;

CREATE TABLE HYPER_CDC_CHANGE_LOG (
    id            NUMBER(19,0)   NOT NULL,
    source_schema VARCHAR2(128)  NOT NULL,
    source_table  VARCHAR2(128)  NOT NULL,
    op            CHAR(1)        NOT NULL,
    pk            CLOB,
    row_data      CLOB,
    changed_at    TIMESTAMP      DEFAULT SYSTIMESTAMP NOT NULL,
    CONSTRAINT pk_hyper_cdc_change_log PRIMARY KEY (id)
);


DROP SEQUENCE HYPER_CDC_CHANGE_LOG_SEQ;

CREATE SEQUENCE HYPER_CDC_CHANGE_LOG_SEQ
    START WITH 1
    INCREMENT BY 1
    NOCACHE
    NOCYCLE;

CREATE OR REPLACE TRIGGER HYPER_CDC_CHANGE_LOG_BI
BEFORE INSERT ON HYPER_CDC_CHANGE_LOG
FOR EACH ROW
BEGIN
    SELECT HYPER_CDC_CHANGE_LOG_SEQ.NEXTVAL
    INTO :NEW.id
    FROM DUAL;
END;

ALTER TABLE HYPER_CDC_CHANGE_LOG MODIFY (id NUMBER(19,0));