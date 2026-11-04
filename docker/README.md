# docker/

Production **core** is `docker-compose.yaml` only. Local/full stacks add
`docker-compose.sidecars.yaml` and `docker-compose.ops.yaml` (default in `./install.sh`).
Demo databases, MinIO lake, Vector, MCP, and the native UI are compose profiles.

```bash
./install.sh --native-ui
./install.sh --demo --lake
./install.sh --mcp --demo
```

`./install.sh`, `./start.sh`, and `./redeploy.sh` at the repo root are thin wrappers
that forward here.

## Layout

| Path | Role |
|------|------|
| `docker-compose.yaml` | Production core (`cdc-stack` project name must stay `cdc-stack`) |
| `docker-compose.sidecars.yaml` | Copy & Sync sidecars, pgdump, rsync, Vector profile |
| `docker-compose.ops.yaml` | ksqlDB + Kafbat UI |
| `docker-compose.prod.yaml` | Production tuning overlay (memory, Connect batching) |
| `compose.inc.sh` | Shared `-f` list for install/redeploy/start |
| `.env` | Live config — **gitignored**, created from `.env.example` on first install |
| `.env.example` | Template; copy to `.env` and edit secrets |
| `Dockerfile` | Orchestrator + `cdc-processor` image; build context is the **repo root** |
| `connect/` | Custom Debezium Connect Dockerfile (Avro converter + JDBC source); **build context is the repo root** so it can compile `kafka-connect-jdbc/` |
| `jdbc-admin/` | JDBC Type 4 catalog/DDL/preview service (`cdc-jdbc-admin`) |
| `rsync/` | `cdc-rsync` sidecar: host-to-host file sync with rsync over SSH (Linux/macOS), direct or relay (`/cdc/rsync/*`). See below |
| `pgdump/` | `cdc-pgdump` sidecar: PostgreSQL remote dump to a downloadable file, or dump → restore into a target (`/cdc/pgdump/*`). See below |
| `pgcopydb/`, `pgloader/`, `mongocopy/`, `redisshake/` | Copy & Sync sidecars (`/cdc/pgcopy|pgloader|mongocopy|redisshake/*`). See below |
| `sidecar_common/` | Job store, HTTP contract and connection tests shared by those four; copied into each image via compose `additional_contexts: common` |
| `config/` | Files bind-mounted into containers (`vector.toml`) |
| `init/` | Demo Postgres bootstrap, mounted to `/docker-entrypoint-initdb.d` |
| `install.sh` | Stack installer |
| `start.sh` | Local cargo dev runner (`--stack` also brings up compose infra) |
| `redeploy.sh` | Rebuild orchestrator + native UI (default); checks rqlite health only (never recreates it); `--connect` / `--full` |

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
- `../cdc-native-ui` (compose service `cdc-native-ui`), `../kafbat-webui`, `../mcp` — build contexts
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

## Deploy shapes

| Goal | Compose files |
|------|----------------|
| Production CDC | `-f docker-compose.yaml` (+ `-f docker-compose.prod.yaml` after preflight) |
| + Copy & Sync / pgdump / rsync | add `-f docker-compose.sidecars.yaml` |
| + ksqlDB + Kafbat | add `-f docker-compose.ops.yaml` |
| Full local (install default) | all three base overlays |

```bash
# production core
cd docker
docker compose --env-file .env.production -f docker-compose.yaml -f docker-compose.prod.yaml up -d

# or
./install.sh --core
```

## Profiles

| Profile | Adds |
|---------|------|
| *(none)* | Core only (or core + overlays you pass with `-f`) |
| `demo` | Demo Postgres source + target |
| `lake` | MinIO warehouse + bucket init |
| `vector` | Vector consumer (requires `docker-compose.sidecars.yaml`) |
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

## Postgres dump sidecar (`cdc-pgdump`)

Runs `pg_dump` (and `pg_restore` for mode `dump_restore`) against profile connections. Orchestrator
routes: `/cdc/pgdump/*` (profiles, `run`, jobs, logs, stop, `GET jobs/{id}/file`, `DELETE jobs/{id}`);
UI page `/pgdump`.

- **Client version:** `pg_dump` must be ≥ every source server's major. Build arg `PG_MAJOR`
  (`CDC_PGDUMP_PG_MAJOR`, default 18) picks the `postgres:<major>` base; a newer client dumps older
  servers back to 9.2.
- **Secrets:** passwords go into a per-job `0600` `.pgpass` (exact `host:port:*:user` lines), deleted
  when the job ends and on sidecar start. They never appear in argv, `job.json`, or logs.
- **Files:** volume `cdc-pgdump-data`; removed `PGDUMP_RETENTION_HOURS` (72) after the job finishes,
  or via `DELETE /cdc/pgdump/jobs/{id}`. Start is refused below `PGDUMP_MIN_FREE_MB` (1024).
- **Limits:** `PGDUMP_MAX_CONCURRENT` (2) jobs; one active job per profile; `PGDUMP_TIMEOUT_SECONDS`
  (0 = none).
- **dump_restore:** archive is always `-Fc` and kept after the restore (downloadable). When the
  profile uses `include_tables`, schemas listed in the archive TOC are created on the target first
  (`pg_dump --table` omits `CREATE SCHEMA`). `pg_restore` errors that it skips past end as
  `completed_with_errors` with `errors_ignored`, `first_error`, and a hint; set
  `restore_exit_on_error` to fail fast instead.
- **SFTP destinations** (`destinations[]` on a profile, max 5; UI "Store on SFTP servers"): after the
  dump, the file is uploaded to each server as `<file>.part`, size-checked, renamed atomically, and a
  `<file>.sha256` is written (`sha256sum -c` compatible). `remote_dir` is created if missing (relative
  = under the SFTP user's home). Auth is `key` (OpenSSH/PEM private key, optional passphrase) or
  `password`; secrets are redacted in API responses, kept when the editor sends `********` back, and
  never written to the sidecar disk. Pin `host_key_fingerprint` (`SHA256:…`, from
  `ssh-keyscan <host> | ssh-keygen -lf -`); a mismatch aborts that upload. Unpinned uploads report the
  observed fingerprint so it can be pinned. The local copy is removed once every upload succeeds unless
  `options.keep_local`; any failed upload makes the job `completed_with_errors` and keeps the file.
  The sidecar must reach each SFTP host on its port (default 22); `PGDUMP_SFTP_TIMEOUT_SECONDS` (30)
  bounds connect/auth.
- **Tests:** `cd docker/pgdump && python3 -m unittest -v` (also part of `npm run check`).

## File sync sidecar (`cdc-rsync`)

Copies a directory from one SSH host to another (Ubuntu, other Linux, macOS) with rsync. Orchestrator
routes: `/cdc/rsync/*` (profiles, `run`, `run?dry_run=true`, jobs, logs, stop, `test-connection`,
`test-route`, `staging`); UI page `/rsync` ("File sync"). Decisions: `docs/adr/0007-rsync-file-sync-sidecar.md`.

- **Route (`options.transport`):** `auto` (default) checks per run whether the source can log in to the
  target; if so the run is **direct** (sidecar → `ssh` source → rsync on the source pushes to the target),
  else **relay** (pull source → `cdc-rsync-data`, push → target) and the job records `transport_reason`.
  `direct` fails instead of relaying; `relay` always relays.
- **Keys:** direct mode forwards only the target key, from a per-job `ssh-agent`, and never writes it
  on the source, so a **password target always relays**. Keys may be encrypted (passphrase). The source
  needs an `ssh` client and agent forwarding allowed (`AllowAgentForwarding yes`, the default).
- **Target address from the source:** `target.reach_host` / `reach_port` when the source must use
  another name or LAN address than the sidecar (the pinned host key still applies).
- **Host keys:** pin `host_key_fingerprint` (`SHA256:…`, `ssh-keyscan -p 22 host | ssh-keygen -lf -`) on
  each host; unpinned runs and tests report the key for one-click pinning.
- **macOS:** rsync is auto-detected (Homebrew `/opt/homebrew/bin/rsync` or `/usr/local/bin/rsync` first).
  The built-in openrsync / 2.6.9 works without live progress, ACLs or xattrs; `brew install rsync` is
  recommended. Enable Remote Login on the Mac. `rsync_path` overrides detection.
- **Paths:** source trailing `/` copies the contents, without it the folder itself; the target path
  is a directory and is created if missing; relative paths start in the login's home.
- **Safety:** `delete` (mirror) is off by default; `max_delete` stops a run after N deletions (exit 25,
  `completed_with_errors`). Preview (`dry_run`) lists creates/updates/deletes without writing.
- **Relay copies:** one per profile in `cdc-rsync-data`, kept so the next relay run only moves changes
  (`discard_staging` removes it each run); unused copies expire after `RSYNC_STAGING_RETENTION_DAYS` (14);
  deleting the profile removes it. Relay runs refuse to start below `RSYNC_MIN_FREE_MB` (1024).
- **Limits:** `RSYNC_MAX_CONCURRENT` (2); one active run per profile; `RSYNC_TIMEOUT_SECONDS` (0 = none);
  job records kept `RSYNC_RETENTION_HOURS` (168).
- **Tests:** `cd docker/rsync && python3 -m unittest -v` (also part of `npm run check`).

### Demo hosts for testing

`docker-compose.rsync-demo.yaml` starts two throwaway Ubuntu 24.04 SSH hosts on `server_zone`
(own compose project `cdc-rsync-demo`, so `down -v` never touches `cdc-stack`):

```bash
docker compose -f docker-compose.rsync-demo.yaml up -d --build
docker compose -f docker-compose.rsync-demo.yaml down -v      # remove hosts and their data
```

| Host (from `cdc-rsync`) | From the Mac | Path | Key |
|---|---|---|---|
| `cdc-rsync-source:22` | `localhost:2221` | `data/` (sample files, spaces, symlink, 5 MB blob, `*.tmp`) | `rsync/demo-host/keys/source_ed25519` |
| `cdc-rsync-target:22` | `localhost:2222` | any, e.g. `backup` (starts empty) | `rsync/demo-host/keys/target_ed25519` |

User `syncuser`; password login is also on (`RSYNC_DEMO_PASSWORD`, default `sync-demo`). Keys are
created on first start (gitignored). Key → key runs go **direct**; a password target goes **relay**.
Host keys are per container, so pins change when the containers are recreated.

## Copy & Sync sidecars (`cdc-pgcopydb`, `cdc-pgloader`, `cdc-mongocopy`, `cdc-redisshake`)

Shared behaviour lives in `sidecar_common/` (decisions: `docs/adr/0006-copy-sync-sidecars.md`):

- **One job per run.** Each run gets its own id and records `profile_id`; the UI shows run history.
  The newest `*_RETAIN_RUNS` runs (10; Redis: `REDIS_SHAKE_RETAIN_JOBS`, 15) per transfer are kept.
  A transfer has at most one active run (409 → 400 in the API).
- **Test connection** (`POST /v1/test-connection`, API `/cdc/<tool>/test-connection`) runs inside the
  sidecar, so it proves the network path and login a run will use.
- **Secrets** never reach argv, `job.json` or logs: pgcopydb uses a per-run 0600 `.pgpass`; pgloader's
  load file, mongocopy's `--config` files and RedisShake's `shake.toml` are 0600 and deleted when the
  run ends (mongocopy also deletes the dump archive).
- **pgcopydb image** is `postgres:<CDC_PGCOPY_PG_MAJOR>` (default 18) + pgcopydb from the PostgreSQL
  apt repo. Its pg_dump must be ≥ every source server (it was `dimitri/pgcopydb:latest`, a PG 16 client).
  pgcopydb cannot parse column comments in some schemas: turn on "Leave out comments" if a run fails
  with "Failed to parse Archive TOC comment".
- **Build** needs the shared context: `docker compose build cdc-pgcopydb` (compose passes it), or
  `docker build --build-context common=docker/sidecar_common docker/pgcopydb`.
- **Tests:** `npm run test:sidecars` (part of `npm run check`) runs every sidecar's unittest suite.
