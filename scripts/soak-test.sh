#!/usr/bin/env bash
# Single-host CDC soak test against a running orchestrator.
# Usage:
#   ORCH_BASE_URL=http://127.0.0.1:9788 ORCH_API_TOKEN=... ./scripts/soak-test.sh
# Optional:
#   ORCH_SOAK_GROUP=vector-cdc-consumer
#   ORCH_SOAK_LOOPS=5
#   ORCH_SOAK_SLEEP_SECS=3
set -euo pipefail

BASE="${ORCH_BASE_URL:-http://127.0.0.1:9788}"
BASE="${BASE%/}"
TOKEN="${ORCH_API_TOKEN:-}"
GROUP="${ORCH_SOAK_GROUP:-vector-cdc-consumer}"
LOOPS="${ORCH_SOAK_LOOPS:-5}"
SLEEP_SECS="${ORCH_SOAK_SLEEP_SECS:-3}"

auth_hdr=()
if [[ -n "$TOKEN" ]]; then
  auth_hdr=(-H "Authorization: Bearer ${TOKEN}")
fi

json_get() {
  local path="$1"
  curl -fsS "${auth_hdr[@]}" "${BASE}${path}"
}

json_post() {
  local path="$1"
  local body="${2:-}"
  if [[ -n "$body" ]]; then
    curl -fsS "${auth_hdr[@]}" -H "Content-Type: application/json" \
      -X POST "${BASE}${path}" -d "$body"
  else
    curl -fsS "${auth_hdr[@]}" -X POST "${BASE}${path}"
  fi
}

fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "OK: $*"; }

echo "== CDC soak against ${BASE} =="

# 1) Health (public)
status="$(curl -fsS "${BASE}/health" | sed -n 's/.*"status"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
[[ "$status" == "ok" ]] || fail "health status not ok (got '${status}')"
ok "health"

# 2) Auth gate (when token configured)
if [[ -n "$TOKEN" ]]; then
  code="$(curl -sS -o /dev/null -w '%{http_code}' "${BASE}/cdc/capability" || true)"
  [[ "$code" == "401" ]] || fail "expected 401 without bearer for /cdc/capability, got ${code}"
  ok "auth rejects unauthenticated /cdc/capability"
fi

# 3) Capability + persistence flags
cap="$(json_get /cdc/capability)"
echo "$cap" | grep -q '"api_prefix"' || fail "capability missing api_prefix"
echo "$cap" | grep -q '"profile_reconcile"' || fail "capability missing profile_reconcile"
if echo "$cap" | grep -q '"persistence"[[:space:]]*:[[:space:]]*false'; then
  echo "WARN: persistence=false — profiles will not survive restart" >&2
else
  ok "capability reports persistence"
fi
ok "capability"

# 4) Engine status
json_get /cdc/engine/status >/dev/null
ok "engine status"

# 5) Connectors list
json_get /cdc/connectors >/dev/null
ok "connectors list"

# 6) Profiles list (persistence path)
json_get /cdc/profiles >/dev/null
ok "profiles list"

# 7) Kafka visibility
json_get /cdc/kafka/topics >/dev/null || echo "WARN: kafka topics failed (stack down?)" >&2
if ! json_get "/cdc/kafka/groups/${GROUP}/lag" >/dev/null 2>&1; then
  echo "WARN: lag for group '${GROUP}' unavailable (ok if consumer not running)" >&2
else
  ok "kafka lag group=${GROUP}"
fi

# 8) Metrics scrape
curl -fsS "${auth_hdr[@]}" "${BASE}/cdc/metrics" | grep -q 'cdc_connectors_total' \
  || fail "metrics missing cdc_connectors_total"
ok "metrics"

# 9) Loop: engine status + connectors + profiles (control-plane stability)
for i in $(seq 1 "$LOOPS"); do
  json_get /cdc/engine/status >/dev/null
  json_get /cdc/connectors >/dev/null
  json_get /cdc/profiles >/dev/null
  json_get /cdc/notifications/history?limit=10 >/dev/null || true
  echo "  soak loop ${i}/${LOOPS}"
  sleep "$SLEEP_SECS"
done
ok "soak loops x${LOOPS}"

echo
echo "SOAK PASSED"
echo "Manual follow-ups before production:"
echo "  1) Enable a real profile (wizard) and confirm Connect connector RUNNING"
echo "  2) Pause / resume / restart profile actions"
echo "  3) Restart orchestrator container and confirm profiles still listed (persistence)"
echo "  4) Trigger a FAILED connector and confirm /cdc/notifications/history + WebUI alerts"
