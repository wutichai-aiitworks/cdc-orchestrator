#!/usr/bin/env bash

# Usage:
#   ./start.sh                     # cargo run (env=.env then .env_local)
#   ./start.sh .env_local          # extra env file last
#   ./start.sh --build .env_local  # force cargo rebuild
#
# Compose mcp-cdc already binds 127.0.0.1:8410. Stop it before cargo:
#   (cd ../docker && docker compose --profile mcp stop mcp-cdc)

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"

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
  sed -n '3,10p' "$0" | sed 's/^# //'
}

FORCE_BUILD=0
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

load_env_file "${DIR}/.env"
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
  echo "Loaded env: .env / .env_local (if present)"
fi

export MCP_BIND_ADDR="${MCP_BIND_ADDR:-127.0.0.1:8410}"
export CDC_ORCHESTRATOR_URL="${CDC_ORCHESTRATOR_URL:-http://127.0.0.1:9788}"
export ALLOW_WRITE_TOOLS="${ALLOW_WRITE_TOOLS:-true}"
export RUST_LOG="${RUST_LOG:-info,mcp_cdc=info}"

BIN="${DIR}/target/debug/mcp-cdc"

echo "Starting mcp-cdc on ${MCP_BIND_ADDR}"
echo "Orchestrator : ${CDC_ORCHESTRATOR_URL}"
echo "Write tools  : ${ALLOW_WRITE_TOOLS}"
echo

if [ "$FORCE_BUILD" -eq 1 ] || [ ! -x "$BIN" ]; then
  exec cargo run
fi

echo "Using existing binary: ${BIN} (use --build to recompile)"
exec "$BIN"
