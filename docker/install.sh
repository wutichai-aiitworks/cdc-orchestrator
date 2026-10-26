#!/usr/bin/env bash
# =============================================================================
# install.sh — complete single-host CDC stack (single docker-compose.yaml).
# Runs from docker/, where .env sits next to the compose file.
#
# Usage:
#   ./install.sh --native-ui
#   ./install.sh --demo --lake
# =============================================================================
set -euo pipefail
cd "$(dirname "$0")"

PROFILES=()
WITH_DEMO=0
WITH_LAKE=0
WITH_VECTOR=0
WITH_MCP=0
WITH_NATIVE_UI=0
DETACH="-d"

usage() {
  cat <<'EOF'
Usage: ./install.sh [options]        (or ./docker/install.sh)

Options:
  --demo          Include demo Postgres source + target (profile: demo)
  --lake          Include MinIO object storage for source_to_lake pipelines (profile: lake)
  --vector        Include Vector consumer (profile: vector)
  --mcp           Include mcp-cdc agent gateway (profile: mcp; crate in ../mcp)
                  Local prompting: ALLOW_WRITE_TOOLS=true in docker/.env
                  See design compendium Ch.15 (docs/architecture/)
  --native-ui     Include native Level-3 web UI (profile: native-ui)
  --foreground    Run compose in foreground
  -h, --help      Show help

Examples:
  ./install.sh --native-ui
  ./install.sh --demo
  ./install.sh --demo --lake --native-ui
  ./install.sh --demo --vector --native-ui
  ORCH_ENV=prod ORCH_API_TOKEN=... ORCH_DATASERVICE_URL=... ./install.sh
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --demo) WITH_DEMO=1; shift ;;
    --lake) WITH_LAKE=1; shift ;;
    --vector) WITH_VECTOR=1; shift ;;
    --mcp) WITH_MCP=1; shift ;;
    --native-ui) WITH_NATIVE_UI=1; shift ;;
    --foreground) DETACH=""; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage; exit 1 ;;
  esac
done

if [[ ! -f .env ]]; then
  cp .env.example .env
  echo "Created docker/.env from .env.example — edit secrets before production use."
fi

# shellcheck disable=SC1091
set -a
source ./.env
set +a

compose() {
  if [[ "${#PROFILES[@]}" -gt 0 ]]; then
    docker compose -f docker-compose.yaml "${PROFILES[@]}" "$@"
  else
    # macOS Bash 3.2 + `set -u` treats an empty array expansion as unbound.
    docker compose -f docker-compose.yaml "$@"
  fi
}

if [[ "$WITH_DEMO" -eq 1 ]]; then PROFILES+=(--profile demo); fi
if [[ "$WITH_LAKE" -eq 1 ]]; then PROFILES+=(--profile lake); fi
if [[ "$WITH_VECTOR" -eq 1 ]]; then PROFILES+=(--profile vector); fi
if [[ "$WITH_NATIVE_UI" -eq 1 ]]; then PROFILES+=(--profile native-ui); fi
if [[ "$WITH_MCP" -eq 1 ]]; then
  if [[ ! -f ../mcp/Cargo.toml ]]; then
    echo "ERROR: --mcp requires in-tree crate mcp/" >&2
    exit 1
  fi
  PROFILES+=(--profile mcp)
fi

echo "== Building and starting CDC stack (project: cdc-stack) =="
# shellcheck disable=SC2086
compose up --build $DETACH

echo
echo "== Waiting for orchestrator health =="
ORCH_URL="http://${HYPER_CDC_ORCHESTRATOR_BIND:-127.0.0.1}:${HYPER_CDC_ORCHESTRATOR_PORT:-9788}"
for i in $(seq 1 60); do
  if curl -fsS "${ORCH_URL}/health" >/dev/null 2>&1; then
    echo "Orchestrator healthy: ${ORCH_URL}/health"
    break
  fi
  if [[ "$i" -eq 60 ]]; then
    echo "ERROR: orchestrator did not become healthy in time" >&2
    compose ps
    exit 1
  fi
  sleep 2
done

echo
echo "== Waiting for Kafka Connect =="
CONNECT_URL="http://127.0.0.1:${CDC_CONNECT_HOST_PORT:-28083}"
for i in $(seq 1 60); do
  if curl -fsS "${CONNECT_URL}/" >/dev/null 2>&1; then
    echo "Connect ready: ${CONNECT_URL}/"
    break
  fi
  if [[ "$i" -eq 60 ]]; then
    echo "WARN: Connect not ready yet — check: (cd docker && docker compose logs cdc-connect)" >&2
    break
  fi
  sleep 2
done

echo
echo "Stack is up."
echo "  Orchestrator : ${ORCH_URL}"
echo "  Connect REST : ${CONNECT_URL}"
echo "  Kafbat UI    : http://127.0.0.1:${CDC_ADMIN_HOST_PORT:-4321}"
echo "  Kafbat image : ${CDC_ADMIN_IMAGE:-ghcr.io/kafbat/kafka-ui:latest}"
if [[ "$WITH_NATIVE_UI" -eq 1 ]]; then
  echo "  Native UI    : http://127.0.0.1:${CDC_NATIVE_UI_HOST_PORT:-4320}"
fi
if [[ "$WITH_DEMO" -eq 1 ]]; then
  echo "  Demo PG source: 127.0.0.1:${CDC_POSTGRES_HOST_PORT:-25432}  db=demo  dump=${CDC_DEMO_DUMP_SIZE:-3m}"
  echo "  Demo PG target: 127.0.0.1:${CDC_POSTGRES_TARGET_HOST_PORT:-25433}  db=demo  (empty, for CDC)"
  echo "                  user/password demo/demo  (in-compose: cdc-postgres-source|target:5432)"
fi
if [[ "$WITH_LAKE" -eq 1 ]]; then
  echo "  Lake MinIO   : http://127.0.0.1:${CDC_LAKE_API_PORT:-19000}  (console :${CDC_LAKE_CONSOLE_PORT:-19001})"
  echo "                  bucket=lake  user/password ${CDC_LAKE_ACCESS_KEY:-cdcadmin}/${CDC_LAKE_SECRET_KEY:-cdcadmincdcadmin}"
  echo "                  in-compose endpoint http://cdc-lake:9000"
fi
if [[ "$WITH_MCP" -eq 1 ]]; then
  echo "  MCP          : http://${MCP_CDC_BIND:-127.0.0.1}:${MCP_CDC_PORT:-8410}/mcp"
  echo "                  writes=${ALLOW_WRITE_TOOLS:-false}  (compendium Ch.15)"
fi
echo
echo "Next (from the repo root):"
echo "  curl ${ORCH_URL}/health"
echo "  ORCH_BASE_URL=${ORCH_URL} ORCH_API_TOKEN=\$ORCH_API_TOKEN ./scripts/soak-test.sh"
echo "  cd docker && docker compose logs -f cdc-orchestrator"
echo "  cd docker && docker compose down"
