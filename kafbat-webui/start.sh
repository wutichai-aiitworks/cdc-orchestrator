#!/usr/bin/env bash

# Usage:
#   ./start.sh                         # dev (default), env=.env then .env_local
#   ./start.sh .env_local              # dev, env=.env_local
#   ./start.sh --build .env_local      # dev (Nuxt rebuilds automatically)
#   ./start.sh prod                    # prod, env=.env
#   ./start.sh prod .env_local         # prod: build + start
#   ./start.sh prod --build .env_local # prod: force rebuild + start
#   ./start.sh --mode prod .env_local  # same as prod
# 
# Modes:
#   dev  — npm run dev (hot reload)
#   prod — npm run build (if needed/forced) then nuxt preview

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"

MODE="dev"
FORCE_BUILD=0
ENV_FILE=""

usage() {
  sed -n '3,14p' "$0" | sed 's/^# //'
}

while [ $# -gt 0 ]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    dev|prod)
      MODE="$1"
      shift
      ;;
    --mode)
      if [ $# -lt 2 ]; then
        echo "Error: --mode requires dev|prod" >&2
        exit 1
      fi
      MODE="$2"
      shift 2
      ;;
    --build)
      FORCE_BUILD=1
      shift
      ;;
    --no-build)
      FORCE_BUILD=0
      shift
      ;;
    -*)
      echo "Error: unknown option '$1'" >&2
      usage >&2
      exit 1
      ;;
    *)
      if [ -n "$ENV_FILE" ]; then
        echo "Error: unexpected argument '$1' (env file already set to '$ENV_FILE')" >&2
        usage >&2
        exit 1
      fi
      ENV_FILE="$1"
      shift
      ;;
  esac
done

if [ "$MODE" != "dev" ] && [ "$MODE" != "prod" ]; then
  echo "Error: mode must be 'dev' or 'prod' (got '$MODE')" >&2
  exit 1
fi

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

load_env_file "${DIR}/.env"
load_env_file "${DIR}/.env_local"
if [ -n "$ENV_FILE" ]; then
  ENV_PATH="$ENV_FILE"
  [[ "$ENV_PATH" != /* ]] && ENV_PATH="${DIR}/${ENV_PATH}"
  if [ -f "$ENV_PATH" ]; then
    load_env_file "$ENV_PATH"
    echo "Loaded env: $ENV_PATH"
  else
    echo "Warning: env file '$ENV_PATH' not found, using process environment."
  fi
else
  echo "Loaded env: .env / .env_local (if present)"
fi

HOST="${HOST:-0.0.0.0}"
PORT="${PORT:-${CDC_NATIVE_UI_HOST_PORT:-4320}}"
export APP_NAME="${APP_NAME:-Hyper CDC WebUI}"
export PUBLIC_ORCHESTRATOR_URL="${PUBLIC_ORCHESTRATOR_URL:-http://127.0.0.1:9788}"
export ORCHESTRATOR_URL="${ORCHESTRATOR_URL:-${PUBLIC_ORCHESTRATOR_URL}}"
export ORCHESTRATOR_TOKEN="${ORCHESTRATOR_TOKEN:-}"

echo "Mode: $MODE"
echo "Listen: ${HOST}:${PORT}"
echo "Orchestrator: ${ORCHESTRATOR_URL}"

if [ ! -d "${DIR}/node_modules" ]; then
  echo "==> Installing kafbat-webui dependencies"
  npm install
fi

if [ "$MODE" = "dev" ]; then
  if [ "$FORCE_BUILD" -eq 1 ]; then
    echo "Info: '--build' acknowledged (Nuxt dev handles rebuild automatically)."
  fi
  exec npx nuxt dev --host "$HOST" --port "$PORT"
fi

OUTPUT_ENTRY=".output/server/index.mjs"
NEED_BUILD=0
if [ "$FORCE_BUILD" -eq 1 ]; then
  NEED_BUILD=1
elif [ ! -f "$OUTPUT_ENTRY" ]; then
  echo "Info: production build missing ($OUTPUT_ENTRY); building..."
  NEED_BUILD=1
fi

if [ "$NEED_BUILD" -eq 1 ]; then
  echo "Building production bundle..."
  npm run build
fi

echo "Starting production server..."
export HOST PORT
exec npx nuxt preview --host "$HOST" --port "$PORT"
