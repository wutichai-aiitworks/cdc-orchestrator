#!/usr/bin/env bash
# Thin wrapper — the reload script lives in docker/reload-dev.sh.
set -euo pipefail
exec "$(cd "$(dirname "$0")" && pwd)/docker/reload-dev.sh" "$@"
