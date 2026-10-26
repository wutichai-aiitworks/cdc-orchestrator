#!/usr/bin/env bash
# Restore Postgres Professional demo dump into the source POSTGRES_DB.
# Loads samples/demo-{3m,6m,1y,2y}.sql.gz (CDC_DEMO_DUMP_SIZE).
set -euo pipefail

SIZE="${CDC_DEMO_DUMP_SIZE:-3m}"
DUMP="/samples/demo-${SIZE}.sql.gz"
DB="${POSTGRES_DB:-demo}"
USER="${POSTGRES_USER:-demo}"

case "$SIZE" in
  3m|6m|1y|2y) ;;
  *)
    echo "CDC_DEMO_DUMP_SIZE must be 3m, 6m, 1y, or 2y (got: ${SIZE})" >&2
    exit 1
    ;;
esac

if [[ ! -f "$DUMP" ]]; then
  echo "Missing dump file: ${DUMP}" >&2
  echo "Put demo-${SIZE}.sql.gz in the samples/ folder." >&2
  exit 1
fi

echo "Restoring database '${DB}' from demo-${SIZE}.sql.gz ..."

# The dumps DROP/CREATE database demo and \\connect. Skip those so we load into
# the database created by POSTGRES_DB.
gunzip -c "$DUMP" | awk '
  /^DROP DATABASE/ { next }
  /^CREATE DATABASE/ { next }
  /^\\connect/ { next }
  { print }
' | psql -v ON_ERROR_STOP=1 --username "$USER" --dbname "$DB"

echo "Restore complete (${SIZE})."
