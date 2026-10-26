#!/usr/bin/env bash
# Thin wrapper — the dev runner lives in docker/start.sh.
set -euo pipefail
exec "$(cd "$(dirname "$0")" && pwd)/docker/start.sh" "$@"
