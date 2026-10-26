#!/usr/bin/env bash
# Rebuild and restart the fast-moving services in place.
set -euo pipefail
cd "$(dirname "$0")"

compose() {
  # Olake overlay containers stay up; this script only reloads core services.
  COMPOSE_IGNORE_ORPHANS=1 docker compose -f docker-compose.yaml "$@"
}

echo "Reloading CDC message bus, Connect (Avro), ksqlDB, Orchestrator, and Native UI..."
compose up -d --no-deps cdc-message
compose up -d --build --no-deps cdc-connect
compose up -d --no-deps cdc-ksqldb
compose up -d --build --no-deps cdc-orchestrator
compose up -d --no-deps cdc-admin
compose up -d --build --no-deps cdc-pgcopydb
compose up -d --build --no-deps cdc-redisshake
compose up -d --build --no-deps cdc-pgloader
compose up -d --build --no-deps cdc-mongocopy
compose --profile vector up -d --no-deps cdc-vector
compose --profile native-ui up -d --build --no-deps cdc-native-ui
echo "Done!"
