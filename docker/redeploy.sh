#!/usr/bin/env bash
# Rebuild and restart fast-moving app images without wiping stack data.
#
# Default: orchestrator + native UI only (does not touch cdc-rqlite — no recreate;
# Kafka topics, Connect offsets, demo DB volumes, Redis, etc. unchanged).
#
# Usage:
#   ./redeploy.sh              # orchestrator + native-ui (recommended)
#   ./redeploy.sh --connect    # also rebuild/restart Kafka Connect
#   ./redeploy.sh --full       # wider restart (message, connect, tools…)
set -euo pipefail
cd "$(dirname "$0")"

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
  COMPOSE_IGNORE_ORPHANS=1 docker compose -f docker-compose.yaml "$@"
}

# redeploy uses --no-deps on the orchestrator; rqlite must already be healthy
# (we never run compose up on cdc-rqlite here — avoids accidental recreate).
assert_rqlite_healthy() {
  echo "Checking cdc-rqlite (pipeline config store; not recreated by redeploy)…"
  if ! docker inspect -f '{{.State.Running}}' cdc-rqlite 2>/dev/null | grep -qx true; then
    echo "ERROR: cdc-rqlite is not running. Start the existing container:" >&2
    echo "  docker start cdc-rqlite" >&2
    echo "  # first install only: cd docker && docker compose up -d cdc-rqlite" >&2
    exit 1
  fi
  local health
  health="$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}unknown{{end}}' cdc-rqlite 2>/dev/null || echo missing)"
  if [[ "$health" != "healthy" ]]; then
    echo "ERROR: cdc-rqlite is not healthy (status=$health). Fix before redeploy." >&2
    echo "  docker compose -f docker-compose.yaml logs cdc-rqlite" >&2
    echo "  docker start cdc-rqlite   # if stopped" >&2
    exit 1
  fi
}

if [[ "$FULL" -eq 1 ]]; then
  echo "Full redeploy (restarts message bus + Connect — Kafka/connectors may briefly reset)…"
  assert_rqlite_healthy
  compose up -d --no-deps cdc-message
  compose up -d --build --no-deps cdc-connect
  compose up -d --no-deps cdc-ksqldb
  compose up -d --build --no-deps --wait cdc-orchestrator
  compose up -d --no-deps cdc-admin
  compose up -d --build --no-deps cdc-pgcopydb
  compose up -d --build --no-deps cdc-redisshake
  compose up -d --build --no-deps cdc-pgloader
  compose up -d --build --no-deps cdc-mongocopy
  compose --profile vector up -d --no-deps cdc-vector
  compose --profile native-ui up -d --build --no-deps cdc-native-ui
  echo "Done (full)."
  exit 0
fi

echo "Redeploy: orchestrator + native UI (rqlite untouched; Kafka, demo DBs unchanged)…"
assert_rqlite_healthy
compose --profile native-ui up -d --build --no-deps cdc-native-ui
compose up -d --build --no-deps --wait cdc-orchestrator

if [[ "$WITH_CONNECT" -eq 1 ]]; then
  echo "Also rebuilding Kafka Connect…"
  compose up -d --build --no-deps cdc-connect
fi

echo "Done."
