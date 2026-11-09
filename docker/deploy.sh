#!/usr/bin/env bash
# Rebuild and restart fast-moving app images without wiping stack data.
#
# Default: jdbc-admin + orchestrator + native UI only (does not touch cdc-rqlite — no recreate;
# Kafka topics, Connect offsets, demo DB volumes, Redis, etc. unchanged).
#
# Usage:
#   ./redeploy.sh              # jdbc-admin + orchestrator + native-ui (recommended)
#   ./redeploy.sh --connect    # also rebuild/restart Kafka Connect
#   ./redeploy.sh --full       # wider restart (message, connect, tools…)
set -euo pipefail
cd "$(dirname "$0")"
# shellcheck source=compose.inc.sh
source ./compose.inc.sh

WITH_CONNECT=0
FULL=0

usage() {
  sed -n '3,12p' "$0" | sed 's/^# \?//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --connect) WITH_CONNECT=1; shift ;;
    --full) FULL=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 1 ;;
  esac
done

compose() {
  COMPOSE_IGNORE_ORPHANS=1 docker compose "${COMPOSE_FILES[@]}" "$@"
}

# Orchestrator redeploy uses --no-deps; rqlite must be healthy but we only start it
# (never recreate — no `compose up --force-recreate` on cdc-rqlite).
ensure_rqlite_healthy() {
  echo "Checking cdc-rqlite (pipeline config store; not recreated by redeploy)…"
  if ! docker inspect cdc-rqlite >/dev/null 2>&1; then
    echo "cdc-rqlite missing — creating from compose (core file only)…"
    docker compose "${COMPOSE_CORE[@]}" up -d cdc-rqlite
  elif ! docker inspect -f '{{.State.Running}}' cdc-rqlite 2>/dev/null | grep -qx true; then
    echo "cdc-rqlite stopped — starting existing container…"
    docker start cdc-rqlite >/dev/null
  fi

  local health="" i
  for i in $(seq 1 30); do
    health="$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}unknown{{end}}' cdc-rqlite 2>/dev/null || echo missing)"
    if [[ "$health" == "healthy" ]]; then
      echo "cdc-rqlite healthy."
      return 0
    fi
    if [[ "$health" == "unhealthy" ]]; then
      break
    fi
    sleep 2
  done

  echo "ERROR: cdc-rqlite is not healthy (status=$health)." >&2
  echo "  docker compose ${COMPOSE_CORE[*]} logs cdc-rqlite" >&2
  exit 1
}

wait_container_healthy() {
  local name=$1
  local max_secs=${2:-120}
  local health="" i
  for ((i = 1; i <= max_secs; i++)); do
    if ! docker inspect "$name" >/dev/null 2>&1; then
      sleep 1
      continue
    fi
    health="$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}unknown{{end}}' "$name" 2>/dev/null || echo missing)"
    if [[ "$health" == "healthy" ]]; then
      return 0
    fi
    if [[ "$health" == "unhealthy" && "$i" -ge 25 ]]; then
      echo "ERROR: $name is unhealthy." >&2
      docker inspect "$name" --format '{{range .State.Health.Log}}{{.Start}} exit={{.ExitCode}} {{.Output}}{{println}}{{end}}' 2>/dev/null || true
      docker compose "${COMPOSE_FILES[@]}" logs --tail=60 "$name" 2>/dev/null || true
      return 1
    fi
    sleep 1
  done
  echo "ERROR: timed out after ${max_secs}s waiting for $name (last status=$health)." >&2
  return 1
}

if [[ "$FULL" -eq 1 ]]; then
  echo "Full redeploy (restarts message bus + Connect — Kafka/connectors may briefly reset)…"
  ensure_rqlite_healthy
  compose up -d --no-deps cdc-redis
  wait_container_healthy cdc-redis 30
  compose up -d --no-deps cdc-message
  wait_container_healthy cdc-message 90
  compose up -d --build --no-deps cdc-connect
  echo "Waiting for Kafka Connect (can take ~90s after message restart)…"
  wait_container_healthy cdc-connect 180
  #compose up -d --no-deps cdc-ksqldb
  compose up -d --build --force-recreate --no-deps cdc-jdbc-admin
  wait_container_healthy cdc-jdbc-admin 60
  compose up -d --build --no-deps cdc-wireguard cdc-pgcopydb cdc-redisshake cdc-pgloader cdc-mongocopy cdc-pgdump cdc-rsync
  compose up -d --build --force-recreate --no-deps cdc-orchestrator
  wait_container_healthy cdc-orchestrator 90
  #compose up -d --no-deps cdc-admin
  compose --profile vector up -d --no-deps cdc-vector
  compose --profile native-ui up -d --build --no-deps cdc-native-ui
  echo "Done (full)."
  exit 0
fi

echo "Redeploy: jdbc-admin + orchestrator + native UI (rqlite untouched; Kafka, demo DBs unchanged)…"
ensure_rqlite_healthy
compose up -d --no-deps cdc-redis
compose up -d --build --force-recreate --no-deps cdc-jdbc-admin
compose --profile native-ui up -d --build --force-recreate --no-deps cdc-native-ui
compose up -d --build --force-recreate --no-deps cdc-orchestrator
wait_container_healthy cdc-orchestrator 90

if [[ "$WITH_CONNECT" -eq 1 ]]; then
  echo "Also rebuilding Kafka Connect…"
  compose up -d --build --no-deps cdc-connect
fi

echo "Done."
