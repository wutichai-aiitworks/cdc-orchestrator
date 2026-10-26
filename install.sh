#!/usr/bin/env bash
# Thin wrapper — the installer lives in docker/install.sh.
set -euo pipefail
exec "$(cd "$(dirname "$0")" && pwd)/docker/install.sh" "$@"
