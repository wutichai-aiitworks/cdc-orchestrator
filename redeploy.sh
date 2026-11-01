#!/usr/bin/env bash
# Thin wrapper — the redeploy script lives in docker/redeploy.sh.
set -euo pipefail
exec "$(cd "$(dirname "$0")" && pwd)/docker/redeploy.sh" "$@"
