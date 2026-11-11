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

# WebUI build version, same rule as Woodpecker: a v* tag on HEAD → the tag (no commit id),
# otherwise <branch-slug>_<short-sha>. An explicit CDC_APP_VERSION wins.
if [[ -z "${CDC_APP_VERSION:-}" ]] && git rev-parse --git-dir >/dev/null 2>&1; then
  _tag="$(git describe --tags --exact-match --match 'v*' HEAD 2>/dev/null || true)"
  if [[ -n "$_tag" ]]; then
    CDC_APP_VERSION="$_tag"
  else
    _slug="$(git rev-parse --abbrev-ref HEAD 2>/dev/null | tr '[:upper:]' '[:lower:]' \
      | sed 's#[^a-z0-9_.-]#-#g; s#-\{2,\}#-#g; s#^-##; s#-$##')"
    [[ "$_slug" == "head" ]] && _slug=""   # detached HEAD
    CDC_APP_VERSION="${_slug:-sha}_$(git rev-parse --short=7 HEAD)"
  fi
  unset _tag _slug
fi
export CDC_APP_VERSION="${CDC_APP_VERSION:-dev}"

compose_cmd() {
  docker compose "${COMPOSE_FILES[@]}" "$@"
}

compose_core_cmd() {
  docker compose "${COMPOSE_CORE[@]}" "$@"
}
