#!/bin/sh
set -eu

BROKER_CONTAINER=${CDC_REDPANDA_CONTAINER:-cdc-message}
APPLY=${CDC_REDPANDA_RETENTION_APPLY:-false}
RETENTION_MS=${CDC_REDPANDA_RETENTION_MS:-604800000}
DLQ_RETENTION_MS=${CDC_REDPANDA_DLQ_RETENTION_MS:-2592000000}
SEGMENT_BYTES=${CDC_REDPANDA_SEGMENT_BYTES:-134217728}
RETENTION_BYTES=${CDC_REDPANDA_RETENTION_BYTES:-}

fail() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

is_positive_integer() {
  case "$1" in
    ''|*[!0-9]*|0) return 1 ;;
    *) return 0 ;;
  esac
}

config_entry() {
  topic=$1
  key=$2
  docker exec "$BROKER_CONTAINER" rpk topic describe "$topic" -c |
    awk -v wanted="$key" '$1 == wanted { print $2 "|" $3; exit }'
}

is_positive_integer "$RETENTION_MS" ||
  fail "CDC_REDPANDA_RETENTION_MS must be a positive integer"
is_positive_integer "$DLQ_RETENTION_MS" ||
  fail "CDC_REDPANDA_DLQ_RETENTION_MS must be a positive integer"
is_positive_integer "$SEGMENT_BYTES" ||
  fail "CDC_REDPANDA_SEGMENT_BYTES must be a positive integer"
if [ -n "$RETENTION_BYTES" ]; then
  is_positive_integer "$RETENTION_BYTES" ||
    fail "CDC_REDPANDA_RETENTION_BYTES must be empty or a positive integer"
fi

case "$APPLY" in
  true|TRUE|1|yes|YES) apply=true ;;
  false|FALSE|0|no|NO) apply=false ;;
  *) fail "CDC_REDPANDA_RETENTION_APPLY must be true or false" ;;
esac

[ "$#" -gt 0 ] ||
  fail "provide one or more explicit non-internal topic names"
docker inspect "$BROKER_CONTAINER" >/dev/null 2>&1 ||
  fail "Redpanda container is unavailable: $BROKER_CONTAINER"

for topic in "$@"; do
  case "$topic" in
    _*|cdc_ksql_processing_log)
      fail "internal or platform topic is excluded: $topic"
      ;;
  esac

  case "$topic" in
    *-dlq|*-DLQ|*.dlq|*.DLQ) topic_retention_ms=$DLQ_RETENTION_MS ;;
    *) topic_retention_ms=$RETENTION_MS ;;
  esac

  retention_entry=$(config_entry "$topic" retention.ms)
  segment_entry=$(config_entry "$topic" segment.bytes)
  retention_value=${retention_entry%%|*}
  retention_source=${retention_entry#*|}
  segment_value=${segment_entry%%|*}
  segment_source=${segment_entry#*|}

  needs_change=false
  if [ "$retention_value" != "$topic_retention_ms" ] ||
    [ "$retention_source" != DYNAMIC_TOPIC_CONFIG ]; then
    needs_change=true
  fi
  if [ "$segment_value" != "$SEGMENT_BYTES" ] ||
    [ "$segment_source" != DYNAMIC_TOPIC_CONFIG ]; then
    needs_change=true
  fi

  if [ -n "$RETENTION_BYTES" ]; then
    retention_bytes_entry=$(config_entry "$topic" retention.bytes)
    retention_bytes_value=${retention_bytes_entry%%|*}
    retention_bytes_source=${retention_bytes_entry#*|}
    if [ "$retention_bytes_value" != "$RETENTION_BYTES" ] ||
      [ "$retention_bytes_source" != DYNAMIC_TOPIC_CONFIG ]; then
      needs_change=true
    fi
  fi

  if [ "$needs_change" = false ]; then
    printf 'UNCHANGED: %s\n' "$topic"
    continue
  fi

  if [ -n "$RETENTION_BYTES" ]; then
    if [ "$apply" = true ]; then
      docker exec "$BROKER_CONTAINER" rpk topic alter-config "$topic" \
        --set "retention.ms=$topic_retention_ms" \
        --set "segment.bytes=$SEGMENT_BYTES" \
        --set "retention.bytes=$RETENTION_BYTES"
    else
      docker exec "$BROKER_CONTAINER" rpk topic alter-config "$topic" --dry \
        --set "retention.ms=$topic_retention_ms" \
        --set "segment.bytes=$SEGMENT_BYTES" \
        --set "retention.bytes=$RETENTION_BYTES"
    fi
  elif [ "$apply" = true ]; then
    docker exec "$BROKER_CONTAINER" rpk topic alter-config "$topic" \
      --set "retention.ms=$topic_retention_ms" \
      --set "segment.bytes=$SEGMENT_BYTES"
  else
    docker exec "$BROKER_CONTAINER" rpk topic alter-config "$topic" --dry \
      --set "retention.ms=$topic_retention_ms" \
      --set "segment.bytes=$SEGMENT_BYTES"
  fi

  if [ "$apply" = true ]; then
    [ "$(config_entry "$topic" retention.ms)" = "$topic_retention_ms|DYNAMIC_TOPIC_CONFIG" ] ||
      fail "retention.ms verification failed for $topic"
    [ "$(config_entry "$topic" segment.bytes)" = "$SEGMENT_BYTES|DYNAMIC_TOPIC_CONFIG" ] ||
      fail "segment.bytes verification failed for $topic"
    if [ -n "$RETENTION_BYTES" ]; then
      [ "$(config_entry "$topic" retention.bytes)" = "$RETENTION_BYTES|DYNAMIC_TOPIC_CONFIG" ] ||
        fail "retention.bytes verification failed for $topic"
    fi
    printf 'APPLIED: %s retention.ms=%s segment.bytes=%s' \
      "$topic" "$topic_retention_ms" "$SEGMENT_BYTES"
  else
    printf 'DRY-RUN: %s retention.ms=%s segment.bytes=%s' \
      "$topic" "$topic_retention_ms" "$SEGMENT_BYTES"
  fi
  if [ -n "$RETENTION_BYTES" ]; then
    printf ' retention.bytes=%s' "$RETENTION_BYTES"
  else
    printf ' retention.bytes=unchanged'
  fi
  printf '\n'
done
