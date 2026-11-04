# Shared Compose file list for scripts in docker/.
# shellcheck shell=bash
#
# Core (production CDC path):
#   COMPOSE_CORE=( -f docker-compose.yaml )
#
# Full local stack (default for install.sh):
#   COMPOSE_FILES=( "${COMPOSE_CORE[@]}" -f docker-compose.sidecars.yaml -f docker-compose.ops.yaml )

COMPOSE_CORE=( -f docker-compose.yaml )
COMPOSE_SIDECARS=( -f docker-compose.sidecars.yaml )
COMPOSE_OPS=( -f docker-compose.ops.yaml )
COMPOSE_FILES=( "${COMPOSE_CORE[@]}" "${COMPOSE_SIDECARS[@]}" "${COMPOSE_OPS[@]}" )

compose_cmd() {
  docker compose "${COMPOSE_FILES[@]}" "$@"
}

compose_core_cmd() {
  docker compose "${COMPOSE_CORE[@]}" "$@"
}
