# Production CDC Phase 3B Platform Overlays

Date: 2026-09-06

## Platform policy

Phase 3B is platform-dependent:

- macOS with Docker Desktop: use `docker-compose.prod.yaml` only.
- Shared Linux host or contended VM: use `docker-compose.prod.yaml` only.
- Dedicated native Linux host: add `docker-compose.prod-linux.yaml`.

The portable production overlay retains `--overprovisioned`. The dedicated
Linux overlay replaces the Redpanda command without that flag. Both retain
`--check=false` until the separate Phase 3C host/filesystem validation.

Creating these overlays does not restart or modify the running stack.

## Mac Studio and shared Linux deployment

Run the production preflight:

```sh
cd docker
ENV_FILE=.env.production ./scripts/production-preflight.sh
```

Deploy:

```sh
docker compose \
  --env-file .env.production \
  -f docker-compose.yaml \
  -f docker-compose.prod.yaml \
  up -d
```

This is also the rollback configuration for Phase 3B.

## Dedicated native Linux deployment

Use this only when Redpanda has predictable CPU time and the host is not a
Docker Desktop VM, shared machine, or contended virtual machine.

Run the platform-aware preflight:

```sh
cd docker
ENV_FILE=.env.production \
PLATFORM_COMPOSE_FILE=docker-compose.prod-linux.yaml \
./scripts/production-preflight.sh
```

For an initial deployment:

```sh
docker compose \
  --env-file .env.production \
  -f docker-compose.yaml \
  -f docker-compose.prod.yaml \
  -f docker-compose.prod-linux.yaml \
  up -d
```

For a controlled Phase 3B rollout on an existing deployment, first take a
fresh Redpanda volume backup, then recreate only the broker:

```sh
docker compose \
  --env-file .env.production \
  -f docker-compose.yaml \
  -f docker-compose.prod.yaml \
  -f docker-compose.prod-linux.yaml \
  up -d --no-deps --force-recreate --no-build cdc-message
```

After the restart, verify cluster health, topics, schemas, connector tasks,
consumer lag, source/target row counts, CPU saturation, and latency.

## Dedicated Linux rollback

Recreate only Redpanda without the Linux overlay. The existing data volume is
reused:

```sh
docker compose \
  --env-file .env.production \
  -f docker-compose.yaml \
  -f docker-compose.prod.yaml \
  up -d --no-deps --force-recreate --no-build cdc-message
```

Do not delete Kafka topics, offsets, connector configurations, or the Redpanda
data volume during rollout or rollback.

## Current status

The overlay files and preflight support are implemented and render correctly.
They have not been applied to the current Mac Studio stack because that
deployment must retain `--overprovisioned`.

