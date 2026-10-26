#!/bin/bash
# Local SonarQube check with the same tests and quality gate as Woodpecker CI.
#
# Runs ./testops.sh (fmt, test, clippy for the root crate and mcp/), then
# @sonar/scan uploads the analysis and waits for the quality gate.
# -Dsonar.projectKey overrides sonar-project.properties, so the CI project is never touched.
#
# Rust issue rules run on the SonarQube server. Community Build accepts the scan
# and will not raise Rust-specific issues.
#
# Usage:
#   export SONAR_LOCAL_TOKEN=squ_...     # token from the local SonarQube
#   ./sonar.sh                           # start server if needed, test, analyse, report
#   ALL_ISSUES=1 ./sonar.sh              # also list every open issue
#   SONAR_SKIP_UP=1 ./sonar.sh           # do not try to start the local container
set -euo pipefail

cd "$(dirname "$0")"

export SONAR_HOST_URL="${SONAR_HOST_URL:-http://localhost:9900}"
export SONAR_PROJECT_KEY="${SONAR_PROJECT_KEY:-hyper-cdc-orchestrator-local}"
export SONAR_PROJECT_NAME="${SONAR_PROJECT_NAME:-$SONAR_PROJECT_KEY}"
: "${SONAR_LOCAL_TOKEN:?export SONAR_LOCAL_TOKEN first (see scripts/sonar-local-up.sh)}"

if [ "${SONAR_SKIP_UP:-0}" != "1" ]; then
  scripts/sonar-local-up.sh
fi

gate_status=0
# Smoke hits a live process; the CI test step does not.
RUN_SMOKE= ./testops.sh || gate_status=$?

if [ "$gate_status" -eq 0 ]; then
  npx --yes @sonar/scan \
    -Dsonar.host.url="$SONAR_HOST_URL" \
    -Dsonar.token="$SONAR_LOCAL_TOKEN" \
    -Dsonar.projectKey="$SONAR_PROJECT_KEY" \
    -Dsonar.projectName="$SONAR_PROJECT_NAME" \
    -Dsonar.qualitygate.wait=true || gate_status=$?
else
  echo "Tests failed; skipping the scan."
fi

echo
if [ "$gate_status" -ne 0 ]; then
  echo "Scan failed (exit $gate_status). If tests failed, the report below is from the previous analysis."
fi
scripts/sonar-local-report.sh || gate_status=1
exit "$gate_status"
