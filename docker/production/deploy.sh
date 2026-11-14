#!/usr/bin/env bash
# Hyper CDC production deploy: pull-only, Woodpecker-built images, Docker secrets.
#
# Usage (from anywhere; operates on this directory):
#   ./deploy.sh secrets                 create missing ./secrets/* (random where possible)
#   ./deploy.sh secrets --rotate NAME   regenerate one secret (then: ./deploy.sh up)
#   ./deploy.sh images TAG [--registry NS]
#                                       write images.env pinned by digest for TAG (Docker Hub)
#   ./deploy.sh preflight               read-only checks (env, secrets, images, host)
#   ./deploy.sh up [--images FILE]      install manifest → preflight → pull → ordered up
#   ./deploy.sh rollback                re-deploy the previous images.env
#   ./deploy.sh pull | status | logs [svc...] | down
#
# FILE is the images.env from the Woodpecker release (Discord attachment / step log).
# Never runs `down -v`; never prints a secret value.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"
ENV_FILE="$DIR/.env"
IMAGES_FILE="$DIR/images.env"
SECRETS_DIR="$DIR/secrets"
HISTORY_DIR="$DIR/.deploy"
DEFAULT_REGISTRY=wutichai

# Woodpecker-built images: VAR|repo name (scripts/ci/images.txt).
BUILT=(
  CDC_ORCHESTRATOR_IMAGE\|cdc-orchestrator CDC_CONNECT_IMAGE\|cdc-connect
  CDC_NATIVE_UI_IMAGE\|cdc-webui MCP_CDC_IMAGE\|cdc-mcp
  CDC_JDBC_ADMIN_IMAGE\|cdc-jdbc-admin CDC_WIREGUARD_IMAGE\|cdc-wireguard
  CDC_PGCOPY_IMAGE\|cdc-pgcopydb CDC_PGDUMP_IMAGE\|cdc-pgdump
  CDC_PGLOADER_IMAGE\|cdc-pgloader CDC_REDIS_SHAKE_IMAGE\|cdc-redisshake
  CDC_MONGOCOPY_IMAGE\|cdc-mongocopy CDC_RSYNC_IMAGE\|cdc-rsync
)
# Entrypoint/Cmd that docker-compose.yaml re-states after the secret-env.sh shim.
# If an image changes its ENTRYPOINT/CMD, compose must follow — checked after pull.
EXPECTED_EP=(
  'CDC_ORCHESTRATOR_IMAGE|["/usr/local/bin/hyper-cdc-orchestrator"] null'
  'CDC_NATIVE_UI_IMAGE|["node","/app/start-ui.mjs"] ["./.output/server/index.mjs"]'
  'MCP_CDC_IMAGE|["/usr/local/bin/mcp-cdc"] null'
  'CDC_JDBC_ADMIN_IMAGE|["java","-Xms64m","-Xmx256m","-jar","/app/jdbc-admin.jar"] null'
  'CDC_WIREGUARD_IMAGE|["/usr/local/bin/cdc-wireguard"] null'
  'CDC_PGCOPY_IMAGE|null ["python3","/app/serve.py"]'
  'CDC_PGDUMP_IMAGE|null ["python3","/app/serve.py"]'
  'CDC_PGLOADER_IMAGE|null ["python3","/app/serve.py"]'
  'CDC_REDIS_SHAKE_IMAGE|["python3","/app/serve.py"] null'
  'CDC_MONGOCOPY_IMAGE|["docker-entrypoint.sh"] ["python3","/app/serve.py"]'
  'CDC_RSYNC_IMAGE|null ["python3","/app/serve.py"]'
)
# Secrets: name|kind. hex/b64 = generated; manual = you fill it; optional = may stay empty;
# mcpkeys = generated "ops:read:<hex>" (edit to add clients).
SECRETS=(
  orch_api_token\|hex orch_secret_key\|b64
  orch_jdbc_admin_token\|hex orch_pgcopy_token\|hex orch_redis_shake_token\|hex
  orch_pgloader_token\|hex orch_mongocopy_token\|hex orch_pgdump_token\|hex
  orch_rsync_token\|hex orch_wireguard_token\|hex
  admin_password\|hex session_secret\|hex admin_session_secret\|hex
  sso_app_secret\|manual authelia_client_secret\|optional
  mcp_client_keys\|mcpkeys mcp_bearer_token\|optional o2_password\|manual
  deploy_discord_webhook\|optional deploy_o2_auth_b64\|optional
)
SERVICE_TOKENS=(orch_api_token orch_jdbc_admin_token orch_pgcopy_token orch_redis_shake_token
  orch_pgloader_token orch_mongocopy_token orch_pgdump_token orch_rsync_token orch_wireguard_token)
# Keys that must no longer be in .env (they moved to ./secrets).
LEGACY_ENV_SECRETS=(ORCH_API_TOKEN ORCH_SECRET_KEY ORCH_JDBC_ADMIN_TOKEN ORCH_PGCOPY_TOKEN
  ORCH_REDIS_SHAKE_TOKEN ORCH_PGLOADER_TOKEN ORCH_MONGOCOPY_TOKEN ORCH_PGDUMP_TOKEN ORCH_RSYNC_TOKEN
  ORCH_WIREGUARD_TOKEN ADMIN_PASSWORD SESSION_SECRET ADMIN_SESSION_SECRET SSO_APP_SECRET
  AUTHELIA_CLIENT_SECRET MCP_CLIENT_KEYS MCP_BEARER_TOKEN O2_PASSWORD DEPLOY_DISCORD_WEBHOOK
  DEPLOY_O2_AUTH_B64)

CORE_INFRA=(cdc-message cdc-rqlite cdc-redis)
CORE_ENGINE=(cdc-connect cdc-jdbc-admin)
SIDECARS=(cdc-wireguard cdc-pgcopydb cdc-redisshake cdc-pgloader cdc-mongocopy cdc-pgdump cdc-rsync)
AGENT=(mcp-cdc cdc-vector)

errors=0
warnings=0
pass() { printf '  \033[32mPASS\033[0m %s\n' "$*"; }
warn() { warnings=$((warnings + 1)); printf '  \033[33mWARN\033[0m %s\n' "$*" >&2; }
fail() { errors=$((errors + 1)); printf '  \033[31mFAIL\033[0m %s\n' "$*" >&2; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

# Read KEY from an env file without sourcing it (values may contain spaces).
env_get() {
  local key=$1 file=${2:-$ENV_FILE}
  [[ -f "$file" ]] || return 0
  awk -v k="$key" 'index($0, k "=") == 1 { sub(/^[^=]*=/, ""); v=$0 } END { print v }' "$file"
}
secret_get() { [[ -f "$SECRETS_DIR/$1" ]] && cat "$SECRETS_DIR/$1" || true; }
file_mode() { stat -c %a "$1" 2>/dev/null || stat -f %Lp "$1"; }

compose() {
  local args=(--env-file "$ENV_FILE")
  [[ -f "$IMAGES_FILE" ]] && args+=(--env-file "$IMAGES_FILE")
  COMPOSE_IGNORE_ORPHANS=1 docker compose --project-directory "$DIR" "${args[@]}" \
    -f "$DIR/docker-compose.yaml" "$@"
}

agent_enabled() { [[ ",$(env_get COMPOSE_PROFILES)," == *",agent,"* ]]; }
# SSO_* / AUTHELIA_* and their secrets matter only when the UI runs AUTH_MODE=sso.
sso_enabled() { [[ "$(env_get AUTH_MODE | tr '[:upper:]' '[:lower:]')" == sso ]]; }

# Effective image ref for a built-image VAR (images.env line, else registry/name:version).
image_ref() {
  local var=$1 name=$2 ref
  ref="$(env_get "$var" "$IMAGES_FILE")"
  if [[ -z "$ref" ]]; then
    local reg ver
    reg="$(env_get CDC_REGISTRY "$IMAGES_FILE")"; ver="$(env_get CDC_VERSION "$IMAGES_FILE")"
    ref="${reg:-$DEFAULT_REGISTRY}/$name:${ver:-latest}"
  fi
  printf '%s' "$ref"
}

# ── secrets ──────────────────────────────────────────────────────────
gen_secret() {
  case "$1" in
    hex) openssl rand -hex 24 ;;
    b64) openssl rand -base64 32 ;;
    mcpkeys) printf 'ops:read:%s' "$(openssl rand -hex 24)" ;;
  esac
}

cmd_secrets() {
  command -v openssl >/dev/null || die "openssl is required"
  [[ -f "$ENV_FILE" ]] || { cp "$DIR/.env.example" "$ENV_FILE"; echo "Created .env from .env.example"; }
  chmod 600 "$ENV_FILE"
  mkdir -p "$SECRETS_DIR"
  # Directory 0700 keeps other host users out; files 0644 so each container user
  # (node, postgres, wg, rsync, …) can read its bind-mounted secret.
  chmod 700 "$SECRETS_DIR"
  umask 022

  if [[ "${1:-}" == --rotate ]]; then
    local name=${2:?--rotate needs a secret name} spec kind=""
    for spec in "${SECRETS[@]}"; do [[ "${spec%%|*}" == "$name" ]] && kind="${spec#*|}"; done
    [[ -n "$kind" ]] || die "unknown secret: $name"
    case "$kind" in hex|b64|mcpkeys) ;; *) die "$name is $kind — edit secrets/$name by hand" ;; esac
    if [[ "$name" == orch_secret_key && "${3:-}" != --force ]]; then
      die "rotating orch_secret_key makes stored notification secrets unreadable (ADR 0015); add --force"
    fi
    gen_secret "$kind" >"$SECRETS_DIR/$name.tmp" && mv "$SECRETS_DIR/$name.tmp" "$SECRETS_DIR/$name"
    chmod 644 "$SECRETS_DIR/$name"
    echo "Rotated $name (value not displayed). Apply with: ./deploy.sh up"
    return
  fi

  local spec name kind made=0 todo=()
  for spec in "${SECRETS[@]}"; do
    name="${spec%%|*}"; kind="${spec#*|}"
    if [[ -s "$SECRETS_DIR/$name" ]]; then continue; fi
    case "$kind" in
      hex|b64|mcpkeys) gen_secret "$kind" >"$SECRETS_DIR/$name"; made=$((made + 1)) ;;
      manual) : >"$SECRETS_DIR/$name"; todo+=("$name") ;;
      optional) [[ -f "$SECRETS_DIR/$name" ]] || : >"$SECRETS_DIR/$name" ;;
    esac
    chmod 644 "$SECRETS_DIR/$name"
  done
  echo "Generated $made secret file(s) in secrets/ (values not displayed; existing files untouched)."
  [[ ${#todo[@]} -eq 0 ]] || { echo "Fill by hand (printf '%s' 'value' > secrets/<name>):"; printf '  - %s\n' "${todo[@]}"; }
  local left; left="$(grep -o '^[A-Z0-9_]*=REPLACE_ME[A-Z_]*' "$ENV_FILE" | sed 's/=.*//' || true)"
  [[ -z "$left" ]] || { echo "Fill in .env:"; sed 's/^/  - /' <<<"$left"; }
}

# ── images: resolve a tag to digest-pinned refs ──────────────────────
resolve_digest() {
  local ref=$1 d=""
  d="$(docker buildx imagetools inspect "$ref" --format '{{.Manifest.Digest}}' 2>/dev/null || true)"
  if [[ "$d" != sha256:* ]]; then
    docker pull -q "$ref" >/dev/null 2>&1 || return 1
    d="$(docker image inspect -f '{{range .RepoDigests}}{{println .}}{{end}}' "$ref" \
      | sed -n "s#^${ref%:*}@##p" | head -1)"
  fi
  [[ "$d" == sha256:* ]] && printf '%s' "$d"
}

cmd_images() {
  local tag="" reg=$DEFAULT_REGISTRY
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --registry) reg="${2:?}"; shift 2 ;;
      -*) die "unknown option: $1" ;;
      *) tag=$1; shift ;;
    esac
  done
  [[ -n "$tag" ]] || die "usage: ./deploy.sh images TAG [--registry NS]"
  local tmp spec var name ref digest
  tmp="$(mktemp)"
  {
    echo "# Resolved $(date -u +%Y-%m-%dT%H:%M:%SZ) from $reg/*:$tag by deploy.sh images"
    echo "CDC_REGISTRY=$reg"
    echo "CDC_VERSION=$tag"
  } >"$tmp"
  for spec in "${BUILT[@]}"; do
    var="${spec%%|*}"; name="${spec#*|}"; ref="$reg/$name:$tag"
    if digest="$(resolve_digest "$ref")"; then
      echo "$var=$ref@$digest" >>"$tmp"; printf '  %-22s %s\n' "$name" "${digest:0:19}…"
    else
      rm -f "$tmp"; die "cannot resolve $ref (tag pushed? docker login done for a private repo?)"
    fi
  done
  mkdir -p "$HISTORY_DIR"
  [[ -f "$IMAGES_FILE" ]] && ! cmp -s "$tmp" "$IMAGES_FILE" && cp "$IMAGES_FILE" "$HISTORY_DIR/images-$(date +%Y%m%d-%H%M%S).env"
  mv "$tmp" "$IMAGES_FILE"
  echo "Wrote images.env ($tag, digest-pinned). Deploy with: ./deploy.sh up"
}

# ── preflight ────────────────────────────────────────────────────────
cmd_preflight() {
  errors=0; warnings=0
  echo "Hyper CDC production preflight ($DIR)"

  echo "Host"
  command -v docker >/dev/null && pass "docker present" || fail "docker missing"
  if docker info >/dev/null 2>&1; then pass "docker daemon reachable"; else fail "docker daemon unreachable"; fi
  if docker compose version >/dev/null 2>&1; then pass "compose $(docker compose version --short 2>/dev/null)"; else fail "docker compose plugin missing"; fi
  if docker network inspect server_zone >/dev/null 2>&1; then pass "network server_zone exists"
  else fail "network server_zone missing (docker network create server_zone)"; fi
  local mem disk
  mem="$(docker info --format '{{.MemTotal}}' 2>/dev/null || echo 0)"
  if [[ "$mem" =~ ^[0-9]+$ && "$mem" -ge 6442450944 ]]; then pass "docker memory >= 6 GiB"
  else warn "docker memory < 6 GiB (Redpanda 2G + Connect 2G heap + JVMs need headroom)"; fi
  disk="$(df -Pk "$DIR" | awk 'NR==2 {print $4}')"
  if [[ "$disk" =~ ^[0-9]+$ && "$disk" -ge 20971520 ]]; then pass "disk free >= 20 GiB"
  else fail "disk free < 20 GiB"; fi

  echo "Config (.env)"
  if [[ ! -f "$ENV_FILE" ]]; then
    fail ".env missing (./deploy.sh secrets creates it from .env.example)"
  else
    local perm; perm="$(file_mode "$ENV_FILE")"
    [[ "$perm" == 600 || "$perm" == 400 ]] && pass ".env mode $perm" || warn ".env mode $perm (chmod 600 .env)"
    local placeholders
    placeholders="$(grep -o '^[A-Z0-9_]*=.*REPLACE_ME' "$ENV_FILE" | sed 's/=.*//' | tr '\n' ' ' || true)"
    agent_enabled || placeholders="$(tr ' ' '\n' <<<"$placeholders" | grep -Ev '^(MCP_|O2_)' | tr '\n' ' ' || true)"
    sso_enabled || placeholders="$(tr ' ' '\n' <<<"$placeholders" | grep -Ev '^(SSO_|AUTHELIA_)' | tr '\n' ' ' || true)"
    [[ -z "${placeholders// /}" ]] && pass "no REPLACE_ME placeholders" || fail "placeholders left in .env: $placeholders"
    local k legacy=""
    for k in "${LEGACY_ENV_SECRETS[@]}"; do [[ -n "$(env_get "$k")" ]] && legacy+="$k "; done
    [[ -z "$legacy" ]] && pass "no credentials in .env" \
      || fail "credentials still in .env (move to secrets/, then delete the line): $legacy"
    case "$(env_get APP_MODE | tr '[:upper:]' '[:lower:]')" in
      production) pass "APP_MODE=Production" ;;
      maintenance|enhanced) warn "APP_MODE=$(env_get APP_MODE) (ribbon shown; hardening still enforced below)" ;;
      *) fail "APP_MODE must be Production|Maintenance|Enhanced" ;;
    esac
    local v
    for v in ORCH_REQUIRE_AUTH ORCH_REQUIRE_PERSISTENCE ORCH_STRICT_PERSISTENCE; do
      [[ "$(env_get "$v")" == true ]] && pass "$v=true" || fail "$v must be true"
    done
    case "$(env_get AUTH_MODE | tr '[:upper:]' '[:lower:]')" in
      sso) pass "AUTH_MODE=sso" ;;
      local) pass "AUTH_MODE=local (SSO_* not checked; login = ADMIN_USERNAME + secrets/admin_password)" ;;
      none|"") fail "AUTH_MODE=none not allowed in production (use sso or local)" ;;
      *) fail "AUTH_MODE=$(env_get AUTH_MODE): must be sso or local" ;;
    esac
    [[ "$(env_get CDC_HOST_BIND)" == 127.0.0.1 ]] && pass "infra ports bound to loopback" \
      || warn "CDC_HOST_BIND=$(env_get CDC_HOST_BIND): Redis/rqlite/Kafka exposed without auth"
    if agent_enabled; then
      [[ "$(env_get ALLOW_WRITE_TOOLS)" == false ]] && pass "MCP write tools off" || fail "ALLOW_WRITE_TOOLS must be false in production"
      if [[ "$(env_get MCP_OAUTH)" == true && "$(env_get MCP_PUBLIC_URL)" != https://* ]]; then
        fail "MCP_OAUTH=true needs an https MCP_PUBLIC_URL"
      fi
    fi
  fi

  echo "Secrets (./secrets)"
  if [[ ! -d "$SECRETS_DIR" ]]; then
    fail "secrets/ missing (./deploy.sh secrets)"
  else
    local dmode; dmode="$(file_mode "$SECRETS_DIR")"
    [[ "$dmode" == 700 ]] && pass "secrets/ mode 700" || fail "secrets/ mode $dmode (chmod 700 secrets)"
    local spec name kind val missing="" short=""
    for spec in "${SECRETS[@]}"; do
      name="${spec%%|*}"; kind="${spec#*|}"
      [[ -f "$SECRETS_DIR/$name" ]] || { missing+="$name "; continue; }
      val="$(secret_get "$name")"
      case "$name" in mcp_*|o2_*) agent_enabled || continue ;; esac
      case "$name" in sso_*|authelia_*) sso_enabled || continue ;; esac
      case "$kind" in
        optional) ;;
        manual) [[ -n "$val" ]] || missing+="$name " ;;
        *) [[ ${#val} -ge 24 ]] || short+="$name " ;;
      esac
    done
    [[ -z "$missing" ]] && pass "all secret files present" || fail "empty/missing secrets: $missing"
    [[ -z "$short" ]] && pass "generated secrets >= 24 chars (values not displayed)" || fail "too short: $short"
    for name in session_secret admin_session_secret; do
      val="$(secret_get "$name")"; [[ ${#val} -ge 32 ]] || fail "$name must be >= 32 chars"
    done
    local key_len; key_len="$(secret_get orch_secret_key | { openssl base64 -d -A 2>/dev/null || true; } | wc -c | tr -d ' ')"
    [[ "$key_len" -eq 32 ]] && pass "orch_secret_key decodes to 32 bytes" || fail "orch_secret_key must be 32 bytes base64"
    local dup; dup="$(for name in "${SERVICE_TOKENS[@]}"; do secret_get "$name"; echo; done | grep . | sort | uniq -d | grep -c . || true)"
    [[ "$dup" -eq 0 ]] && pass "service tokens are distinct" || warn "$dup service token value(s) reused"
    [[ "$(secret_get admin_password)" != admin ]] && pass "admin_password is not 'admin'" || fail "admin_password is 'admin'"
    if agent_enabled; then
      [[ -n "$(secret_get mcp_client_keys)$(secret_get mcp_bearer_token)" ]] && pass "MCP client key(s) configured" \
        || fail "agent profile on but mcp_client_keys and mcp_bearer_token are empty"
    fi
  fi

  echo "Release (images.env)"
  if [[ ! -f "$IMAGES_FILE" ]]; then
    fail "images.env missing (./deploy.sh images TAG, or up --images <woodpecker file>)"
  else
    local s var unpinned=0 ver=""
    for s in "${BUILT[@]}"; do
      var="${s%%|*}"
      local ref; ref="$(env_get "$var" "$IMAGES_FILE")"
      if [[ -z "$ref" || "$ref" != *@sha256:* ]]; then unpinned=$((unpinned + 1)); fi
      [[ -z "$ver" && "$ref" =~ :([^:@/]+)@ ]] && ver="${BASH_REMATCH[1]}"
    done
    if [[ "$unpinned" -eq 0 ]]; then
      pass "all ${#BUILT[@]} Woodpecker images pinned by digest${ver:+ (release $ver)}"
    else
      warn "$unpinned image(s) not digest-pinned → $(image_ref CDC_ORCHESTRATOR_IMAGE cdc-orchestrator | sed 's#/cdc-orchestrator#/<image>#') (run ./deploy.sh images TAG)"
    fi
  fi

  echo "Compose"
  local err; err="$(mktemp)"
  if [[ -f "$ENV_FILE" ]] && compose config -q 2>"$err"; then pass "compose renders"
  else fail "compose config failed: $(head -3 "$err")"; fi
  rm -f "$err"
  if [[ -f "$ENV_FILE" ]] && compose config 2>/dev/null | grep -qE '^\s+build:'; then
    fail "rendered compose contains build: (production must be pull-only)"
  fi
  local c owner
  for c in cdc-message cdc-rqlite cdc-orchestrator; do
    if docker inspect "$c" >/dev/null 2>&1; then
      owner="$(docker inspect "$c" --format '{{index .Config.Labels "com.docker.compose.project.config_files"}}')"
      [[ "$owner" == "$DIR/docker-compose.yaml" ]] && pass "$c owned by production compose" \
        || warn "$c currently owned by $owner (will be adopted on up; volumes kept)"
    fi
  done

  printf '\nPreflight: %d failure(s), %d warning(s)\n' "$errors" "$warnings"
  [[ "$errors" -eq 0 ]]
}

# After pull: the compose entrypoint overrides must still match each image.
check_entrypoints() {
  local spec var exp name ref insp got bad=0 s
  for spec in "${EXPECTED_EP[@]}"; do
    var="${spec%%|*}"; exp="${spec#*|}"
    for s in "${BUILT[@]}"; do [[ "${s%%|*}" == "$var" ]] && name="${s#*|}"; done
    ref="$(image_ref "$var" "$name")"
    [[ "$ref" == *@* ]] && insp="${ref%%:*}@${ref##*@}" || insp="$ref"
    got="$(docker image inspect -f '{{json .Config.Entrypoint}} {{json .Config.Cmd}}' "$insp" 2>/dev/null || echo missing)"
    if [[ "$got" != "$exp" ]]; then
      echo "  entrypoint drift: $name expected '$exp' got '$got'" >&2; bad=1
    fi
  done
  [[ "$bad" -eq 0 ]] || die "update entrypoint/command in docker-compose.yaml and EXPECTED_EP in deploy.sh"
  echo "  entrypoints match the compose shim overrides"
}

# ── health wait ──────────────────────────────────────────────────────
wait_healthy() {
  local timeout=$1; shift
  local name health start=$SECONDS
  for name in "$@"; do
    while :; do
      health="$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "$name" 2>/dev/null || echo missing)"
      case "$health" in healthy|running) printf '  %-18s %s\n' "$name" "$health"; break ;; esac
      if (( SECONDS - start > timeout )); then
        printf '  %-18s %s (timeout %ss)\n' "$name" "$health" "$timeout" >&2
        docker logs --tail 40 "$name" 2>&1 | sed 's/^/    | /' >&2 || true
        return 1
      fi
      sleep 3
    done
  done
}

# ── notifications (optional; payload holds host/version/status only) ─
notify() {
  local status=$1 detail=$2 version host hook o2 o2auth
  version="$(image_ref CDC_ORCHESTRATOR_IMAGE cdc-orchestrator | sed -n 's/.*:\([^:@/]*\)\(@.*\)\{0,1\}$/\1/p')"
  host="$(hostname -s 2>/dev/null || hostname)"
  hook="$(secret_get deploy_discord_webhook)"
  if [[ -n "$hook" ]]; then
    local color=3066993; [[ "$status" == success ]] || color=15158332
    curl -fsS -m 10 -H 'Content-Type: application/json' -X POST "$hook" -d "$(cat <<JSON
{"embeds":[{"title":"cdc-stack deploy ${status}","color":${color},"fields":[
 {"name":"Host","value":"${host}","inline":true},
 {"name":"Version","value":"${version:-unknown}","inline":true},
 {"name":"Detail","value":"${detail}"}]}]}
JSON
)" >/dev/null || echo "WARN: Discord notify failed" >&2
  fi
  o2="$(env_get DEPLOY_O2_URL)"; o2auth="$(secret_get deploy_o2_auth_b64)"
  if [[ -n "$o2" ]]; then
    curl -fsS -m 10 -H 'Content-Type: application/json' ${o2auth:+-H "Authorization: Basic $o2auth"} -X POST "$o2" \
      -d "[{\"stack\":\"cdc-stack\",\"host\":\"${host}\",\"version\":\"${version:-unknown}\",\"status\":\"${status}\",\"detail\":\"${detail}\"}]" \
      >/dev/null || echo "WARN: OpenObserve notify failed" >&2
  fi
}

# ── up ───────────────────────────────────────────────────────────────
install_images() {
  local src=$1
  [[ -f "$src" ]] || die "images file not found: $src"
  grep -q '^CDC_ORCHESTRATOR_IMAGE=' "$src" || die "$src does not look like a Woodpecker images.env"
  mkdir -p "$HISTORY_DIR"
  if [[ -f "$IMAGES_FILE" ]] && ! cmp -s "$src" "$IMAGES_FILE"; then
    cp "$IMAGES_FILE" "$HISTORY_DIR/images-$(date +%Y%m%d-%H%M%S).env"
  fi
  cp "$src" "$IMAGES_FILE"
  echo "Installed release manifest from $src"
}

do_rollout() {
  echo "==> pull"
  compose pull --quiet
  check_entrypoints
  echo "==> core infra (stateful; recreated only if config changed)"
  compose up -d --no-build "${CORE_INFRA[@]}"
  wait_healthy 180 "${CORE_INFRA[@]}"
  echo "==> engine"
  compose up -d --no-build "${CORE_ENGINE[@]}"
  wait_healthy 300 "${CORE_ENGINE[@]}"
  echo "==> sidecars"
  compose up -d --no-build "${SIDECARS[@]}"
  wait_healthy 180 "${SIDECARS[@]}"
  echo "==> control plane + UI"
  compose up -d --no-build cdc-orchestrator
  wait_healthy 180 cdc-orchestrator
  compose up -d --no-build cdc-native-ui
  wait_healthy 120 cdc-native-ui
  if agent_enabled; then
    echo "==> agent (mcp-cdc, cdc-vector)"
    compose up -d --no-build "${AGENT[@]}"
    wait_healthy 120 "${AGENT[@]}"
  fi
}

cmd_up() {
  local images=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --images) images="${2:?--images needs a file}"; shift 2 ;;
      *) die "unknown option: $1" ;;
    esac
  done
  [[ -n "$images" ]] && install_images "$images"
  cmd_preflight || die "preflight failed — nothing changed on the running stack"
  if do_rollout; then
    echo; cmd_status
    notify success "rollout complete"
  else
    notify failure "rollout stopped at an unhealthy service; run ./deploy.sh status / rollback"
    die "rollout stopped; previous manifests are in $HISTORY_DIR (./deploy.sh rollback)"
  fi
}

cmd_rollback() {
  local prev
  prev="$(ls -1t "$HISTORY_DIR"/images-2*.env 2>/dev/null | head -1 || true)"
  [[ -n "$prev" ]] || die "no previous manifest in $HISTORY_DIR"
  echo "Rolling back to $prev"
  cp "$IMAGES_FILE" "$HISTORY_DIR/rolledback-$(date +%Y%m%d-%H%M%S).env"
  cp "$prev" "$IMAGES_FILE"
  rm -f "$prev"
  cmd_preflight || die "preflight failed"
  if do_rollout; then notify success "rollback to $(basename "$prev")"; else notify failure "rollback failed"; exit 1; fi
}

cmd_status() {
  docker ps -a --filter label=com.docker.compose.project=cdc-stack \
    --format 'table {{.Names}}\t{{.Status}}\t{{.Image}}' | sort
}

case "${1:-}" in
  secrets) shift; cmd_secrets "$@" ;;
  images) shift; cmd_images "$@" ;;
  preflight) cmd_preflight ;;
  up) shift; cmd_up "$@" ;;
  rollback) cmd_rollback ;;
  pull) compose pull ;;
  status) cmd_status ;;
  logs) shift; compose logs -f --tail 200 "$@" ;;
  down) compose down ;;
  ""|-h|--help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//' ;;
  *) die "unknown command: $1 (see --help)" ;;
esac
