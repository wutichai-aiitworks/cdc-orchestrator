#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
DOCKER_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
ENV_FILE=${ENV_FILE:-"$DOCKER_DIR/.env.production"}
BASE_COMPOSE="$DOCKER_DIR/docker-compose.yaml"
PROD_COMPOSE="$DOCKER_DIR/docker-compose.prod.yaml"
PLATFORM_COMPOSE_FILE=${PLATFORM_COMPOSE_FILE:-}
if [ -n "$PLATFORM_COMPOSE_FILE" ]; then
  case "$PLATFORM_COMPOSE_FILE" in
    /*) ;;
    *) PLATFORM_COMPOSE_FILE="$DOCKER_DIR/$PLATFORM_COMPOSE_FILE" ;;
  esac
fi
MIN_DOCKER_MEMORY_BYTES=${CDC_PREFLIGHT_MIN_MEMORY_BYTES:-4294967296}
MIN_DISK_KB=${CDC_PREFLIGHT_MIN_DISK_KB:-20971520}
errors=0
warnings=0

pass() {
  printf 'PASS: %s\n' "$*"
}

warn() {
  warnings=$((warnings + 1))
  printf 'WARN: %s\n' "$*" >&2
}

fail() {
  errors=$((errors + 1))
  printf 'FAIL: %s\n' "$*" >&2
}

require_command() {
  if command -v "$1" >/dev/null 2>&1; then
    pass "command available: $1"
  else
    fail "required command is missing: $1"
  fi
}

env_value() {
  key=$1
  awk -v wanted="$key" '
    index($0, wanted "=") == 1 {
      sub(/^[^=]*=/, "")
      print
      exit
    }
  ' "$ENV_FILE"
}

is_positive_integer() {
  case "$1" in
    ''|*[!0-9]*) return 1 ;;
    0) return 1 ;;
    *) return 0 ;;
  esac
}

is_memory_value() {
  printf '%s\n' "$1" | awk '
    /^[1-9][0-9]*([KMGTP]i?[Bb]?|[kmgpt]i?[bB]?)?$/ { ok=1 }
    END { exit(ok ? 0 : 1) }
  '
}

production_compose() {
  if [ -n "$PLATFORM_COMPOSE_FILE" ]; then
    CDC_ENV_FILE="$ENV_FILE" docker compose \
      --env-file "$ENV_FILE" \
      -f "$BASE_COMPOSE" \
      -f "$PROD_COMPOSE" \
      -f "$PLATFORM_COMPOSE_FILE" \
      "$@"
  else
    CDC_ENV_FILE="$ENV_FILE" docker compose \
      --env-file "$ENV_FILE" \
      -f "$BASE_COMPOSE" \
      -f "$PROD_COMPOSE" \
      "$@"
  fi
}

printf 'Hyper CDC production preflight (read-only)\n'
printf 'Docker directory: %s\n' "$DOCKER_DIR"
printf 'Environment file: %s\n' "$ENV_FILE"
printf 'Platform overlay: %s\n\n' "${PLATFORM_COMPOSE_FILE:-portable (none)}"

require_command docker
require_command awk
require_command cmp
require_command df
require_command mktemp
require_command sort

if [ ! -f "$BASE_COMPOSE" ]; then
  fail "missing base Compose file: $BASE_COMPOSE"
fi
if [ ! -f "$PROD_COMPOSE" ]; then
  fail "missing production Compose file: $PROD_COMPOSE"
fi
if [ -n "$PLATFORM_COMPOSE_FILE" ] && [ ! -f "$PLATFORM_COMPOSE_FILE" ]; then
  fail "missing platform Compose file: $PLATFORM_COMPOSE_FILE"
fi
if [ ! -f "$ENV_FILE" ]; then
  fail "missing production environment file; copy .env.production.example to .env.production"
fi

if ! docker info >/dev/null 2>&1; then
  fail "Docker daemon is unavailable"
else
  pass "Docker daemon is available"
fi

if compose_version=$(docker compose version 2>/dev/null); then
  pass "$compose_version"
else
  fail "Docker Compose plugin is unavailable"
fi

if [ -f "$ENV_FILE" ]; then
  orch_env=$(env_value ORCH_ENV)
  api_token=$(env_value ORCH_API_TOKEN)
  mcp_token=$(env_value MCP_BEARER_TOKEN)
  allow_write=$(env_value ALLOW_WRITE_TOOLS)
  smp=$(env_value CDC_REDPANDA_SMP)
  memory=$(env_value CDC_REDPANDA_MEMORY)
  reserve=$(env_value CDC_REDPANDA_RESERVE_MEMORY)
  heap=$(env_value CDC_CONNECT_HEAP_OPTS)
  compression=$(env_value CDC_CONNECT_PRODUCER_COMPRESSION_TYPE)
  producer_batch=$(env_value CDC_CONNECT_PRODUCER_BATCH_SIZE)
  producer_linger=$(env_value CDC_CONNECT_PRODUCER_LINGER_MS)
  consumer_records=$(env_value CDC_CONNECT_CONSUMER_MAX_POLL_RECORDS)
  offset_flush=$(env_value CDC_CONNECT_OFFSET_FLUSH_INTERVAL_MS)
  offset_flush_timeout=$(env_value CDC_CONNECT_OFFSET_FLUSH_TIMEOUT_MS)
  retention_apply=$(env_value CDC_REDPANDA_RETENTION_APPLY)
  retention_ms=$(env_value CDC_REDPANDA_RETENTION_MS)
  dlq_retention_ms=$(env_value CDC_REDPANDA_DLQ_RETENTION_MS)
  segment_bytes=$(env_value CDC_REDPANDA_SEGMENT_BYTES)
  retention_bytes=$(env_value CDC_REDPANDA_RETENTION_BYTES)

  if [ "$orch_env" = prod ]; then
    pass "ORCH_ENV=prod"
  else
    fail "ORCH_ENV must be prod, got '${orch_env:-unset}'"
  fi

  case "$api_token" in
    *REPLACE_ME*) fail "ORCH_API_TOKEN still contains the example placeholder" ;;
    *)
      if [ "${#api_token}" -ge 24 ]; then
        pass "ORCH_API_TOKEN is configured (value not displayed)"
      else
        fail "ORCH_API_TOKEN must be at least 24 characters"
      fi
      ;;
  esac

  case "$mcp_token" in
    *REPLACE_ME*) fail "MCP_BEARER_TOKEN still contains the example placeholder" ;;
    *) ;;
  esac

  case "$allow_write" in
    false|FALSE|0|no|NO) pass "MCP write tools are disabled" ;;
    *) fail "ALLOW_WRITE_TOOLS must be false in production" ;;
  esac

  if is_positive_integer "$smp"; then
    pass "CDC_REDPANDA_SMP is a positive integer"
  else
    fail "CDC_REDPANDA_SMP must be a positive integer"
  fi

  if is_memory_value "$memory"; then
    pass "CDC_REDPANDA_MEMORY has valid syntax"
  else
    fail "invalid CDC_REDPANDA_MEMORY value: '${memory:-unset}'"
  fi

  if [ "$reserve" = 0M ] || [ "$reserve" = 0 ]; then
    fail "CDC_REDPANDA_RESERVE_MEMORY must be nonzero after Phase 3A"
  elif is_memory_value "$reserve"; then
    pass "CDC_REDPANDA_RESERVE_MEMORY has valid syntax"
  else
    fail "invalid CDC_REDPANDA_RESERVE_MEMORY value: '${reserve:-unset}'"
  fi

  case "$heap" in
    *-Xms*"-Xmx"*) pass "CDC_CONNECT_HEAP_OPTS contains -Xms and -Xmx" ;;
    *) fail "CDC_CONNECT_HEAP_OPTS must contain both -Xms and -Xmx" ;;
  esac

  case "$compression" in
    none|gzip|snappy|lz4|zstd) pass "Kafka Connect producer compression is supported" ;;
    *) fail "unsupported CDC_CONNECT_PRODUCER_COMPRESSION_TYPE: '${compression:-unset}'" ;;
  esac

  for setting in \
    "CDC_CONNECT_PRODUCER_BATCH_SIZE:$producer_batch" \
    "CDC_CONNECT_PRODUCER_LINGER_MS:$producer_linger" \
    "CDC_CONNECT_CONSUMER_MAX_POLL_RECORDS:$consumer_records" \
    "CDC_CONNECT_OFFSET_FLUSH_INTERVAL_MS:$offset_flush" \
    "CDC_CONNECT_OFFSET_FLUSH_TIMEOUT_MS:$offset_flush_timeout"
  do
    key=${setting%%:*}
    value=${setting#*:}
    if is_positive_integer "$value"; then
      pass "$key is a positive integer"
    else
      fail "$key must be a positive integer"
    fi
  done

  case "$retention_apply" in
    false|FALSE|0|no|NO) pass "Phase 4 retention apply mode is disabled by default" ;;
    true|TRUE|1|yes|YES) warn "Phase 4 retention apply mode is enabled" ;;
    *) fail "CDC_REDPANDA_RETENTION_APPLY must be true or false" ;;
  esac

  for setting in \
    "CDC_REDPANDA_RETENTION_MS:$retention_ms" \
    "CDC_REDPANDA_DLQ_RETENTION_MS:$dlq_retention_ms" \
    "CDC_REDPANDA_SEGMENT_BYTES:$segment_bytes"
  do
    key=${setting%%:*}
    value=${setting#*:}
    if is_positive_integer "$value"; then
      pass "$key is a positive integer"
    else
      fail "$key must be a positive integer"
    fi
  done

  if [ -z "$retention_bytes" ]; then
    pass "CDC_REDPANDA_RETENTION_BYTES is deferred pending load measurement"
  elif is_positive_integer "$retention_bytes"; then
    pass "CDC_REDPANDA_RETENTION_BYTES is a positive integer"
  else
    fail "CDC_REDPANDA_RETENTION_BYTES must be empty or a positive integer"
  fi
fi

docker_memory=$(docker info --format '{{.MemTotal}}' 2>/dev/null || printf '0')
if is_positive_integer "$docker_memory" && [ "$docker_memory" -ge "$MIN_DOCKER_MEMORY_BYTES" ]; then
  pass "Docker memory allocation is at least 4 GiB"
else
  fail "Docker memory allocation is below 4 GiB or could not be detected"
fi

available_disk_kb=$(df -Pk "$DOCKER_DIR" | awk 'NR==2 {print $4}')
if is_positive_integer "$available_disk_kb" && [ "$available_disk_kb" -ge "$MIN_DISK_KB" ]; then
  pass "filesystem has at least 20 GiB available"
else
  fail "filesystem has less than 20 GiB available or could not be detected"
fi

if docker network inspect server_zone >/dev/null 2>&1; then
  pass "external network server_zone exists"
else
  fail "external network server_zone does not exist"
fi

if [ -f "$ENV_FILE" ] && [ -f "$BASE_COMPOSE" ] && [ -f "$PROD_COMPOSE" ]; then
  rendered=$(mktemp)
  base_services=$(mktemp)
  prod_services=$(mktemp)
  base_volumes=$(mktemp)
  prod_volumes=$(mktemp)
  trap 'rm -f "$rendered" "$base_services" "$prod_services" "$base_volumes" "$prod_volumes"' EXIT HUP INT TERM

  if production_compose config >"$rendered"; then
    pass "production Compose renders successfully"
  else
    fail "production Compose rendering failed"
  fi

  docker compose --env-file "$ENV_FILE" -f "$BASE_COMPOSE" config --services | sort >"$base_services"
  production_compose config --services | sort >"$prod_services"
  if cmp -s "$base_services" "$prod_services"; then
    pass "production overlay preserves the service inventory"
  else
    fail "production overlay changes the service inventory"
  fi

  docker compose --env-file "$ENV_FILE" -f "$BASE_COMPOSE" config --volumes | sort >"$base_volumes"
  production_compose config --volumes | sort >"$prod_volumes"
  if cmp -s "$base_volumes" "$prod_volumes"; then
    pass "production overlay preserves named volumes"
  else
    fail "production overlay changes named volumes"
  fi

  if awk '
    /_STORAGE_REPLICATION_FACTOR:/ && $2 != "\"1\"" { bad=1 }
    END { exit(bad ? 0 : 1) }
  ' "$rendered"; then
    fail "Connect internal topic replication factor must remain 1 on a single broker"
  else
    pass "Connect internal topic replication factors are single-broker compatible"
  fi

  if [ -n "$PLATFORM_COMPOSE_FILE" ]; then
    if awk '/- --overprovisioned$/ { found=1 } END { exit(found ? 0 : 1) }' "$rendered"; then
      fail "dedicated-Linux Compose must remove --overprovisioned"
    else
      pass "dedicated-Linux Compose removes --overprovisioned"
    fi
  else
    if awk '/- --overprovisioned$/ { found=1 } END { exit(found ? 0 : 1) }' "$rendered"; then
      pass "portable Compose retains --overprovisioned"
    else
      fail "portable Compose must retain --overprovisioned"
    fi
  fi

  if awk '/- --check=false$/ { found=1 } END { exit(found ? 0 : 1) }' "$rendered"; then
    pass "Phase 3B Compose retains --check=false"
  else
    fail "Phase 3B must retain --check=false until Phase 3C"
  fi
fi

for service in cdc-message cdc-rqlite; do
  if docker inspect "$service" >/dev/null 2>&1; then
    owner=$(docker inspect "$service" --format '{{index .Config.Labels "com.docker.compose.project.config_files"}}')
    case "$owner" in
      *"/docker/docker-compose.yaml"*) pass "$service is owned by the canonical Compose file" ;;
      *) warn "$service is currently owned by legacy Compose metadata: $owner" ;;
    esac
  fi
done

printf '\nPreflight result: %s failure(s), %s warning(s)\n' "$errors" "$warnings"
if [ "$errors" -ne 0 ]; then
  exit 1
fi
