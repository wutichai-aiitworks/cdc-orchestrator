#!/bin/bash
# Starts the local SonarQube used by sonar.sh (container "sonar-local", http://localhost:9900)
# and waits until it is ready. Creates the container on first run.
# Same container name as hyper-dataservice, so one local server is shared.
#
# First run only: log in at http://localhost:9900 (admin/admin, then change the password),
# create a token under My Account -> Security, and export it:
#   export SONAR_LOCAL_TOKEN=squ_...
set -euo pipefail

CONTAINER="${SONAR_CONTAINER:-sonar-local}"
PORT="${SONAR_PORT:-9900}"
IMAGE="${SONAR_IMAGE:-sonarqube:community}"
WAIT_SECONDS="${SONAR_WAIT_SECONDS:-300}"

if docker inspect "$CONTAINER" >/dev/null 2>&1; then
  if [ "$(docker inspect -f '{{.State.Running}}' "$CONTAINER")" != "true" ]; then
    echo "Starting $CONTAINER ..."
    docker start "$CONTAINER" >/dev/null
  fi
else
  echo "Creating $CONTAINER from $IMAGE on port $PORT ..."
  docker run -d --name "$CONTAINER" -p "$PORT:9000" \
    -v "${CONTAINER}-data:/opt/sonarqube/data" \
    -v "${CONTAINER}-extensions:/opt/sonarqube/extensions" \
    "$IMAGE" >/dev/null
fi

echo -n "Waiting for SonarQube on http://localhost:$PORT "
deadline=$((SECONDS + WAIT_SECONDS))
until curl -fs "http://localhost:$PORT/api/system/status" | grep -q '"status":"UP"'; do
  if [ "$SECONDS" -ge "$deadline" ]; then
    echo
    echo "SonarQube did not become ready within ${WAIT_SECONDS}s (docker logs $CONTAINER)" >&2
    exit 1
  fi
  echo -n "."
  sleep 3
done
echo " UP"

if [ -z "${SONAR_LOCAL_TOKEN:-}" ]; then
  echo "SONAR_LOCAL_TOKEN is not set. Create a token at http://localhost:$PORT/account/security"
fi
