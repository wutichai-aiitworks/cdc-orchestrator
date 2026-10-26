#!/usr/bin/env bash

# Usage:
#   ./start.sh                              # cargo run (env=docker/.env, then .env_local)
#   ./start.sh .env_local                   # extra env file last
#   ./start.sh --build .env_local           # force cargo rebuild
#   ./start.sh --build --stack .env_local   # also start Docker infra (Kafka/Connect/Redis/Kafbat)
#   ./start.sh debug .env_local
# 
#   log_level  — info (default) | debug | trace | warn | error
#   env_file   — extra env file sourced last (highest priority), e.g. .env_local
#   --build    — force cargo rebuild; with --stack also docker compose --build
#   --stack    — start compose infra only (not the orchestrator/native-ui containers)
# 
# Native UI: cd kafbat-webui && ./start.sh --build .env_local

set -euo pipefail

DOCKER_DIR="$(cd "$(dirname "$0")" && pwd)"
# The cargo workspace, target/, and .env_local live at the repo root; .env lives
# next to docker-compose.yaml.
DIR="$(cd "${DOCKER_DIR}/.." && pwd)"
cd "$DIR"

# Compose runs from docker/ so it picks up docker/.env automatically.
compose() {
  (cd "$DOCKER_DIR" && docker compose -f docker-compose.yaml "$@")
}

load_env_file() {
  local env_path="$1"
  [ -f "$env_path" ] || return 0
  while IFS= read -r line || [ -n "$line" ]; do
    [[ -z "${line// }" ]] && continue
    [[ "$line" =~ ^[[:space:]]*# ]] && continue
    if [[ "$line" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]; then
      local key="${line%%=*}"
      local val="${line#*=}"
      if [[ "$val" =~ ^\".*\"$ || "$val" =~ ^\'.*\'$ ]]; then
        val="${val:1:${#val}-2}"
      fi
      export "$key=$val"
    fi
  done < "$env_path"
}

usage() {
  sed -n '3,15p' "$0" | sed 's/^# //'
}

FORCE_BUILD=0
START_STACK=0
LOG_LEVEL="info"
ENV_FILE_ARG=""

while [ $# -gt 0 ]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --build)
      FORCE_BUILD=1
      shift
      ;;
    --stack)
      START_STACK=1
      shift
      ;;
    info|debug|trace|warn|error)
      LOG_LEVEL="$1"
      shift
      ;;
    --*)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
    *)
      if [ -n "$ENV_FILE_ARG" ]; then
        echo "Unexpected argument: $1 (env file already set to $ENV_FILE_ARG)" >&2
        exit 1
      fi
      ENV_FILE_ARG="$1"
      shift
      ;;
  esac
done

load_env_file "${DOCKER_DIR}/.env"
load_env_file "${DIR}/.env_local"
if [ -n "$ENV_FILE_ARG" ]; then
  ENV_FILE="$ENV_FILE_ARG"
  [[ "$ENV_FILE" != /* ]] && ENV_FILE="${DIR}/${ENV_FILE}"
  if [ -f "$ENV_FILE" ]; then
    load_env_file "$ENV_FILE"
    echo "Loaded env: $ENV_FILE"
  else
    echo "Warning: env file '$ENV_FILE' not found, skipping." >&2
  fi
else
  echo "Loaded env: docker/.env / .env_local (if present)"
fi

export ORCH_HOST="${ORCH_HOST:-0.0.0.0}"
export ORCH_PORT="${ORCH_PORT:-${HYPER_CDC_ORCHESTRATOR_PORT:-9788}}"
export ORCH_ENGINE_PROVIDER="${ORCH_ENGINE_PROVIDER:-docker_api}"
export ORCH_CONTAINER_PROJECT_NAME="${ORCH_CONTAINER_PROJECT_NAME:-cdc-stack}"
export ORCH_MANAGED_CONTAINERS="${ORCH_MANAGED_CONTAINERS:-cdc-message,cdc-admin,cdc-connect,cdc-redis,cdc-vector}"
export ORCH_CONNECT_BASE_URL="${ORCH_CONNECT_BASE_URL:-http://127.0.0.1:${CDC_CONNECT_HOST_PORT:-28083}}"
export ORCH_REDPANDA_CONTAINER="${ORCH_REDPANDA_CONTAINER:-cdc-message}"
export ORCH_COMMAND_TIMEOUT_SECS="${ORCH_COMMAND_TIMEOUT_SECS:-30}"
export RUST_LOG="${RUST_LOG:-$LOG_LEVEL}"
export ORCH_CORS_ORIGINS="${ORCH_CORS_ORIGINS:-http://127.0.0.1:${CDC_NATIVE_UI_HOST_PORT:-4320},http://localhost:${CDC_NATIVE_UI_HOST_PORT:-4320}}"

BIN="${DIR}/target/debug/hyper-cdc-orchestrator"

if [ "$START_STACK" -eq 1 ]; then
  echo "==> Starting CDC docker infra (project: cdc-stack)"
  COMPOSE_ARGS=(up -d)
  if [ "$FORCE_BUILD" -eq 1 ]; then
    COMPOSE_ARGS+=(--build)
  fi
  compose "${COMPOSE_ARGS[@]}" \
    cdc-message cdc-connect cdc-admin cdc-redis
fi

echo "Starting hyper-cdc-orchestrator on ${ORCH_HOST}:${ORCH_PORT}"
echo "Engine provider     : ${ORCH_ENGINE_PROVIDER}"
echo "Container project   : ${ORCH_CONTAINER_PROJECT_NAME}"
echo "Managed containers  : ${ORCH_MANAGED_CONTAINERS}"
echo "Connect base URL    : ${ORCH_CONNECT_BASE_URL}"
echo "Kafbat UI           : http://127.0.0.1:${CDC_ADMIN_HOST_PORT:-4321}"
echo "Native UI           : cd kafbat-webui && ./start.sh --build .env_local"
echo

if [ "$FORCE_BUILD" -eq 1 ] || [ ! -x "$BIN" ]; then
  exec cargo run --bin hyper-cdc-orchestrator
fi

echo "Using existing binary: ${BIN} (use --build to recompile)"
exec "$BIN"
