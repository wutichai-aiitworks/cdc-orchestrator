#!/usr/bin/env bash
# Local pre-build/release checks for hyper-cdc-orchestrator.
#
#   ./testops.sh
#   RUN_SMOKE=1 ./testops.sh
#   RUN_SMOKE=1 SMOKE_BASE_URL=http://127.0.0.1:9788 ./testops.sh
#
# Mirrors .woodpecker/test.yml. rdkafka's cmake-build feature needs cmake
# and a C toolchain, same as that workflow. Smoke needs a running orchestrator.
set -euo pipefail
cd "$(dirname "$0")"

step() { printf '\n==> %s\n' "$1"; }

crate() {
  local dir="$1"
  local label="$2"
  step "cargo fmt --check ($label)"
  (cd "$dir" && cargo fmt --all -- --check)
  step "cargo test --locked ($label)"
  (cd "$dir" && cargo test --locked --all-targets)
  step "cargo clippy -D warnings ($label)"
  (cd "$dir" && cargo clippy --all-targets -- -D warnings)
}

crate . orchestrator
crate mcp mcp

if [[ "${RUN_SMOKE:-}" == "1" ]]; then
  step "discovery smoke (GET /health)"
  base="${SMOKE_BASE_URL:-http://127.0.0.1:9788}"
  base="${base%/}"
  body="$(mktemp)"
  status="$(curl -sS -o "$body" -w '%{http_code}' "$base/health" || true)"
  printf 'GET /health %s\n' "$status"
  if [[ "$status" != "200" ]]; then
    echo "body: $(head -c 200 "$body")" >&2
    rm -f "$body"
    exit 1
  fi
  python3 -c '
import json, sys
d = json.load(sys.stdin)
assert d.get("status") == "ok", d
assert d.get("service") == "hyper-cdc-orchestrator", d
print("health", d.get("status"), d.get("service"))
' <"$body"
  rm -f "$body"
fi

printf '\nhyper-cdc-orchestrator testops: all checks passed\n'
