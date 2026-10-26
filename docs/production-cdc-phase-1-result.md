# Production CDC Phase 1 Result

Completed: 2026-09-06  
Scope: Behavior-compatible production Compose overlay and preflight  
Runtime activation: Not performed

## Result

Phase 1 configuration is implemented and validated without restarting or recreating any running service.

Added:

- `docker/docker-compose.prod.yaml`
- `docker/.env.production.example`
- `docker/scripts/production-preflight.sh`

Updated:

- `docker/docker-compose.yaml`
- `docker/.env.example`
- `docker/.gitignore`
- `docs/production-cdc-hardening-plan.md`

## Current-feature impact

There was no runtime impact:

- No container restarted.
- No connector paused.
- No topic or offset changed.
- No named volume changed.
- No profile changed.
- Logical and trigger pipelines stayed running.
- The production override defaults render identically to the current base stack.

The only base Compose change makes the orchestrator's env file selectable:

```yaml
env_file:
  - ${CDC_ENV_FILE:-.env}
```

The default remains `.env`, so existing development behavior is unchanged.

## Production overlay

`docker/docker-compose.prod.yaml` introduces configuration points while retaining the current effective defaults:

```text
CDC_REDPANDA_SMP=1
CDC_REDPANDA_MEMORY=1G
CDC_REDPANDA_RESERVE_MEMORY=0M
CDC_CONNECT_HEAP_OPTS=-Xms512m -Xmx1024m
```

These are compatibility values, not final production tuning:

- Phase 2 tunes Kafka Connect.
- Phase 3 introduces a nonzero Redpanda reserve and removes development flags.
- Phase 4 owns retention activation.

## Environment handling

`docker/.env.production.example` documents:

- Required production orchestrator mode
- Authentication and persistence requirements
- Image overrides
- Host ports
- Phase 1 compatibility resources
- Phase 2 Connect tuning candidates
- Phase 4 retention values
- MCP write-tool restriction

Real `docker/.env.production` is ignored by Git.

The existing `.env.example` image/bind variable names were corrected from the unused `HYPER_CDC_ORCHESTRATOR_*` names to the `CDC_ORCHESTRATOR_*` names consumed by Compose.

## Preflight

The preflight is read-only. It checks:

- Required commands
- Docker daemon and Compose
- `ORCH_ENV=prod`
- Non-placeholder orchestrator token
- MCP writes disabled
- Redpanda SMP/memory syntax
- Connect heap syntax
- Docker memory
- Available disk
- External `cdc_zone` network
- Production Compose rendering
- Service inventory preservation
- Named volume preservation
- Single-broker Connect replication factors
- Legacy Compose ownership metadata

Validation result with temporary non-secret test tokens:

```text
0 failures
3 warnings
```

Expected warnings:

1. `CDC_REDPANDA_RESERVE_MEMORY=0M` remains until Phase 3.
2. `cdc-message` has legacy Compose ownership metadata.
3. `cdc-rqlite` has legacy Compose ownership metadata.

## Compose compatibility verification

The rendered base and base-plus-production configurations were compared using the current `.env`.

Result:

```text
Compatibility render: identical
```

This confirms that merely adding the production overlay does not alter current Redpanda or Connect behavior.

## Activation warning

A dry-run with every optional profile showed that a broad `up` could recreate:

- Demo PostgreSQL source
- Redis
- Vector

It did not propose recreating Redpanda, rqlite, Connect, or orchestrator.

Therefore, do not activate production configuration with an indiscriminate all-profile `up`. Later activation must:

1. Use only the profiles required in production.
2. Review `docker compose --dry-run up -d` output.
3. Recreate only explicitly approved services.
4. Preserve Phase 0 backups.

## rqlite identity risk

The current rqlite node ID is derived from its historical container hostname. Forcing a container recreation can give it a new node ID while its persisted Raft state still references the old one.

Phase 1 intentionally did not force ownership normalization because doing so could leave single-node rqlite without a leader. Before any rqlite recreation, add a tested stable-node-identity or logical migration procedure.

This is now a prerequisite for production activation, not a reason to disturb the healthy running stack during compatibility setup.

## How to prepare a real production environment

Create the private environment file:

```sh
cd docker
cp .env.production.example .env.production
chmod 600 .env.production
```

Replace every `REPLACE_ME` value, then run:

```sh
ENV_FILE=.env.production ./scripts/production-preflight.sh
```

Render and inspect without starting:

```sh
CDC_ENV_FILE=.env.production docker compose \
  --env-file .env.production \
  -f docker-compose.yaml \
  -f docker-compose.prod.yaml \
  config
```

Do not run `up` until the preflight passes and the requested profiles/services are reviewed.

## Phase 1 exit decision

Passed:

- Production override exists.
- Defaults preserve current behavior.
- Production env template exists.
- Private production env is ignored.
- Preflight script passes with valid temporary settings.
- Base and overlay renders are identical.
- Services and volumes are preserved.
- Running stack was not changed.

Deferred safely:

- Actual production activation
- Compose ownership normalization
- rqlite stable identity/migration
- Connect tuning
- Redpanda production tuning
- Retention changes
