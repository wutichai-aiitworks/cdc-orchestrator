# docker/production — production deploy (Woodpecker images, Docker secrets)

Pull-only stack for an AWS EC2 host (or any Linux VM). Woodpecker (`.woodpecker/main-ci.yml`)
builds and pushes thirteen images to Docker Hub under `wutichai/`; this folder runs them,
pinned by digest. The host needs only this folder (no source tree, no build toolchain),
Docker with Compose v2, and the external network `server_zone`.

| Path | Purpose | Committed |
|------|---------|-----------|
| `docker-compose.yaml` | All 17 services, no `build:`; project `cdc-stack` | yes |
| `.env.example` → `.env` | Non-secret config | example only |
| `secrets/` | One file per credential, mounted as Docker secrets | **no** |
| `images.env.example` → `images.env` | Release manifest (which image each service runs) | example only |
| `config/secret-env.sh` | Entrypoint shim: `/run/secrets/*` → env, then exec | yes |
| `config/vector.toml` | Copy-job logs → OpenObserve (no Kafka console tap) | yes |
| `deploy.sh` | secrets / images / preflight / up / rollback / status / logs / down | yes |
| `.deploy/` | Previous `images.env` files (rollback history) | no |

## Stack

| Group | Containers | Published on host |
|-------|-----------|-------------------|
| Core | `cdc-message` (Redpanda), `cdc-connect`, `cdc-jdbc-admin`, `cdc-rqlite`, `cdc-redis`, `cdc-orchestrator`, `cdc-native-ui` | UI `:4320`; everything else `127.0.0.1` |
| Sidecars | `cdc-pgcopydb`, `cdc-wireguard`, `cdc-redisshake`, `cdc-pgloader`, `cdc-mongocopy`, `cdc-pgdump`, `cdc-rsync` | none (server_zone only) |
| AI agent (profile `agent`) | `mcp-cdc`, `cdc-vector` | MCP `127.0.0.1:8410` |
| AI vector sync (`VECTORSYNC_ENABLED=true`) | `cdc-vectorsync` | none (server_zone only) |

## First install

```bash
sudo dnf install -y docker && sudo systemctl enable --now docker   # Amazon Linux 2023
#   (install the compose + buildx plugins too; Ubuntu: docker-ce + docker-compose-plugin)
docker network create server_zone
docker login -u wutichai                     # read-only Docker Hub token if the repos are private

cd docker/production
./deploy.sh secrets                          # creates .env + secrets/, lists what's left
printf '%s' '<value>' > secrets/sso_app_secret
printf '%s' '<value>' > secrets/o2_password
$EDITOR .env                                 # ADMIN_USERNAME, SSO_*, O2_*, public URLs
./deploy.sh images v1.0.0                    # or: up --images <Woodpecker images.env>
./deploy.sh up
```

## Images

Docker Hub `wutichai/<image>`: `cdc-orchestrator`, `cdc-connect`, `cdc-webui`, `cdc-mcp`,
`cdc-jdbc-admin`, `cdc-wireguard`, `cdc-pgcopydb`, `cdc-pgdump`, `cdc-pgloader`,
`cdc-redisshake`, `cdc-mongocopy`, `cdc-rsync`, `cdc-vectorsync`. Upstream images are pinned in compose
(Redpanda `v24.2.4`, rqlite `8.36.16`, Redis `7.4-alpine`, Vector `0.41.1-alpine`).

Tags come from `scripts/ci/version.sh`. A `v*` git tag publishes `vX.Y.Z`, and `latest`
moves to it once all 13 images are built. A branch push publishes `<branch>_<sha7>`
(e.g. `test_1bb2c3c`) and never moves `latest`.

Three ways to fill `images.env`, best first:

| Way | Command | Pinned |
|-----|---------|--------|
| Woodpecker release file (Discord attachment / `build-and-push` log) | `./deploy.sh up --images images-v1.2.3.env` | digest |
| Resolve a tag from Docker Hub now | `./deploy.sh images v1.2.3` then `./deploy.sh up` | digest |
| `images.env.example` as-is | `CDC_REGISTRY=wutichai`, `CDC_VERSION=<tag>` | tag only (preflight warns) |

## Secrets

Every credential is a Docker secret: one file in `secrets/` (mode 700 dir, 644 files),
mounted only into the services that use it, at `/run/secrets/<ENV_NAME>`. None of the
CDC apps read `*_FILE` variables, so `config/secret-env.sh` exports each mounted file as
its env var and then execs the image's original entrypoint. So:

- values never pass through Compose interpolation, and `docker inspect` / `docker compose config` don't show them;
- each container sees only its own secrets (pgdump gets one token, not the orchestrator's set);
- rotating one secret means editing one file and running `./deploy.sh up`.

| Secret file | Mounted into (as) | Kind |
|-------------|-------------------|------|
| `orch_api_token` | orchestrator (`ORCH_API_TOKEN`), native-ui (`ORCHESTRATOR_TOKEN`), mcp-cdc (`CDC_ORCHESTRATOR_TOKEN`) | generated |
| `orch_secret_key` | orchestrator | generated, 32 B base64, **keep stable** (ADR 0015) |
| `orch_jdbc_admin_token` | orchestrator, jdbc-admin (`ORCH_JDBC_ADMIN_TOKEN`) | generated |
| `orch_pgcopy_token` | orchestrator, pgcopydb (`PGCOPY_API_TOKEN`) | generated |
| `orch_redis_shake_token` | orchestrator, redisshake (`REDIS_SHAKE_API_TOKEN`) | generated |
| `orch_pgloader_token` | orchestrator, pgloader (`PGLOADER_API_TOKEN`) | generated |
| `orch_mongocopy_token` | orchestrator, mongocopy (`MONGOCOPY_API_TOKEN`) | generated |
| `orch_pgdump_token` | orchestrator, pgdump (`PGDUMP_API_TOKEN`) | generated |
| `orch_rsync_token` | orchestrator, rsync (`RSYNC_API_TOKEN`) | generated |
| `orch_wireguard_token` | orchestrator, wireguard (`WIREGUARD_API_TOKEN`) | generated |
| `orch_vectorsync_token` | orchestrator, vectorsync (`VECTORSYNC_API_TOKEN`) | generated (used only when vector sync is on) |
| `admin_password` | native-ui | generated (break-glass login) |
| `session_secret`, `admin_session_secret` | native-ui | generated |
| `sso_app_secret` | native-ui | **you fill** |
| `authelia_client_secret` | native-ui | optional |
| `mcp_client_keys` | mcp-cdc | generated `ops:read:<hex>`; add clients as `name:scope:token,…` |
| `mcp_bearer_token` | mcp-cdc | optional (legacy single key) |
| `o2_password` | cdc-vector | **you fill** |
| `deploy_discord_webhook`, `deploy_o2_auth_b64` | deploy.sh only (notifications) | optional |

Empty files are skipped by the shim (the variable stays unset). Rotate a generated one
with `./deploy.sh secrets --rotate orch_pgdump_token && ./deploy.sh up`. Rotating
`orch_secret_key` needs `--force`, because it orphans stored notification secrets.

**Coupling to know about:** the shim means `docker-compose.yaml` restates each image's
ENTRYPOINT/CMD. After every pull, `deploy.sh` compares them with the pulled images and
stops before rollout on any drift. If a Dockerfile's ENTRYPOINT changes, update compose
and `EXPECTED_EP` in `deploy.sh` together. Long term, adding native `*_FILE` support to
the apps removes this.

## Release

1. Push a `v*` tag. Woodpecker builds all 13 images, moves `latest`, and posts
   `images-<version>.env` to Discord.
2. On the host: `./deploy.sh up --images images-v1.2.3.env` (or `./deploy.sh images v1.2.3 && ./deploy.sh up`).

`up` saves the current manifest to `.deploy/` and runs preflight (it stops before
touching anything if a check fails). It then pulls, checks entrypoints, and rolls out in
order, waiting for each group to be healthy:

```
cdc-message, cdc-rqlite, cdc-redis → cdc-connect, cdc-jdbc-admin → 7 sidecars
  → cdc-vectorsync (if VECTORSYNC_ENABLED) → cdc-orchestrator → cdc-native-ui → mcp-cdc, cdc-vector (if profile agent)
```

Containers are recreated only when their image or config changed. `./deploy.sh rollback`
re-deploys the previous manifest.

## AI vector sync (optional, ADR 0018)

Keeps pgvector collections in step with CDC tables (pydebeziumai document builder in the
`cdc-vectorsync` sidecar, reading the pipelines' existing topics). Off by default.

```bash
# .env
VECTORSYNC_ENABLED=true
./deploy.sh up            # starts cdc-vectorsync, the UI shows "AI Vector Sync"
```

`deploy.sh` adds the compose profile `vectorsync` from that one flag. `false` + `up` removes the
container (its volume and the vectors in Postgres stay). Needs:

- a release that contains `cdc-vectorsync` (preflight warns if `images.env` has no pin for it;
  older releases still deploy while the flag is off);
- an embedding server reachable from the container: Ollama (`http://host.docker.internal:11434`
  on the same host, `ollama pull bge-m3`) or any OpenAI-compatible `/v1/embeddings`;
- a Postgres with the `vector` extension (e.g. RDS for PostgreSQL 15+ or `pgvector/pgvector`).
  Store it in a database the sync user may `CREATE EXTENSION` in, or enable it beforehand.

Sync passwords and API keys are encrypted at rest with `orch_secret_key`. Use the wizard's
Test buttons to check both connections from inside the sidecar. Tuning in `.env`:
`VECTORSYNC_DLQ_MAX`, `VECTORSYNC_MAX_BACKOFF_SECONDS`, and `VECTORSYNC_KAFKA_*` for extra
librdkafka settings. Every sync is in the orchestrator's rqlite, so backups need nothing new.

## EC2 notes

- Instance: `m7g.xlarge` (Graviton, 16 GiB) or `m7i.xlarge`; the images are multi-arch.
  Avoid T-family instances, which throttle when CPU credits run out.
- EBS gp3 ≥ 100 GB for Docker's data directory; DLM snapshots (rqlite volume first).
- Security group: inbound 4320 from the ALB/VPN only. Use SSM Session Manager instead of port 22.
- Behind an ALB: `TRUST_PROXY=true`; keep `SESSION_COOKIE_SECURE=true` (HTTPS at the ALB).
- `SSO_EXCHANGE_URL` and `O2_URL` must be reachable URLs: the goldmine containers
  `proxy-supertokens-gateway` and `openobserve-core` don't exist on EC2.
- Redpanda keeps `--overprovisioned` (right for any VM).

## How it differs from `docker/` (dev)

- No `build:`; credentials in Docker secrets, not `.env`.
- Infra ports bind to `127.0.0.1` (Redis and rqlite have no auth).
- The orchestrator gets an explicit env list, not `env_file`.
- Vector does not tail Kafka topics to stdout (that would put row data in `docker logs`).
- json-file log rotation. No ksqlDB, Kafbat UI, demo databases or lake.

## Ops

```bash
./deploy.sh status
./deploy.sh logs cdc-orchestrator cdc-connect
# break-glass admin when SSO is down (ADR 0002), via SSM port forwarding:
aws ssm start-session --target <instance-id> \
  --document-name AWS-StartPortForwardingSession --parameters portNumber=4321,localPortNumber=4321
```
