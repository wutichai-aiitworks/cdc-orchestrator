#!/bin/sh
# Docker secrets → environment, then exec the image's real command.
#
# Compose mounts each secret at /run/secrets/<TARGET>; the target is the env var name the
# app reads (e.g. PGCOPY_API_TOKEN). None of the CDC apps read *_FILE variables, so this
# shim bridges the two without rebuilding images. Values never pass through compose,
# so `docker inspect` / `docker compose config` don't show them.
#
# Only names matching [A-Z_][A-Z0-9_]* are exported; empty files are skipped (= unset).
set -eu
if [ -d /run/secrets ]; then
  for f in /run/secrets/*; do
    [ -f "$f" ] || continue
    n=${f##*/}
    case "$n" in
      [!A-Z_]* | *[!A-Z0-9_]*) continue ;;
    esac
    v=$(cat "$f")
    [ -n "$v" ] || continue
    export "$n=$v"
  done
fi
exec "$@"
