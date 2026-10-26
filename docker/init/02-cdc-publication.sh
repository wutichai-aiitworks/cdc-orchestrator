#!/usr/bin/env bash
# Logical replication publication for Debezium (source only).
set -euo pipefail

psql -v ON_ERROR_STOP=1 --username "${POSTGRES_USER:-demo}" --dbname "${POSTGRES_DB:-demo}" <<'SQL'
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_publication WHERE pubname = 'dbz_publication') THEN
    CREATE PUBLICATION dbz_publication FOR ALL TABLES;
  END IF;
END $$;
SQL

echo "Publication dbz_publication is ready."
