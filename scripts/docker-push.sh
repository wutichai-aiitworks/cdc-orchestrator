#!/usr/bin/env bash
# Build and push a multi-platform Docker image with automatic semantic versioning.
#
# Usage:
#   ./scripts/docker-push.sh              → push :latest only (no git tag)
#   ./scripts/docker-push.sh patch        → bump patch:  v1.2.3 → v1.2.4
#   ./scripts/docker-push.sh minor        → bump minor:  v1.2.3 → v1.3.0
#   ./scripts/docker-push.sh major        → bump major:  v1.2.3 → v2.0.0
#   ./scripts/docker-push.sh v1.5.0       → push explicit tag (no auto-bump)
#
# Auto-bump (patch/minor/major):
#   - Reads the latest git tag matching vMAJOR.MINOR.PATCH.
#   - Increments the requested part (resets lower parts to 0).
#   - Creates and pushes the new git tag.
#   - Builds and pushes Docker image tagged :vX.Y.Z + :latest.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

IMAGE="${IMAGE:-wutichai/hyper-cdc-orchestrator}"
PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64}"
BUILDER="${BUILDER:-multiarch-builder}"
ARG="${1:-latest}"

# Check if we're inside a git repo
IS_GIT_REPO=false
git rev-parse --git-dir &>/dev/null && IS_GIT_REPO=true || true

# ── version bump helper ────────────────────────────────────────────────────────
# Minimum version floor — tags below this are ignored and the baseline is used.
MIN_MAJOR=1; MIN_MINOR=0; MIN_PATCH=0

semver_gte() {
  # Returns 0 (true) when vA.B.C >= vX.Y.Z (floor)
  local a_maj a_min a_pat
  a_maj=$(echo "$1" | cut -d. -f1 | tr -d 'v')
  a_min=$(echo "$1" | cut -d. -f2)
  a_pat=$(echo "$1" | cut -d. -f3)
  [[ $a_maj -gt $MIN_MAJOR ]] && return 0
  [[ $a_maj -lt $MIN_MAJOR ]] && return 1
  [[ $a_min -gt $MIN_MINOR ]] && return 0
  [[ $a_min -lt $MIN_MINOR ]] && return 1
  [[ $a_pat -ge $MIN_PATCH ]]
}

bump_version() {
  local bump="$1"

  local latest=""
  if [[ "$IS_GIT_REPO" == "true" ]]; then
    latest=$(git tag --list 'v*.*.*' \
      | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
      | sort -t. -k1,1V -k2,2n -k3,3n \
      | tail -1)
  fi

  local major minor patch
  if [[ -z "$latest" ]] || ! semver_gte "$latest"; then
    major=$MIN_MAJOR; minor=$MIN_MINOR; patch=$MIN_PATCH
  else
    major=$(echo "$latest" | cut -d. -f1 | tr -d 'v')
    minor=$(echo "$latest" | cut -d. -f2)
    patch=$(echo "$latest" | cut -d. -f3)
  fi

  case "$bump" in
    major) major=$((major + 1)); minor=0; patch=0 ;;
    minor) minor=$((minor + 1)); patch=0 ;;
    patch) patch=$((patch + 1)) ;;
  esac

  echo "v${major}.${minor}.${patch}"
}

latest_tag() {
  if [[ "$IS_GIT_REPO" == "true" ]]; then
    git tag --list 'v*.*.*' \
      | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
      | sort -t. -k1,1V -k2,2n -k3,3n \
      | tail -1
  fi
}

# ── resolve TAG ────────────────────────────────────────────────────────────────
case "$ARG" in
  patch|minor|major)
    TAG=$(bump_version "$ARG")
    PREV=$(latest_tag || true)
    echo "▶ Bump     : ${ARG}  (${PREV:-none} → ${TAG})"

    if [[ "$IS_GIT_REPO" == "true" ]]; then
      git tag "$TAG"
      git push origin "$TAG"
      echo "▶ Git tag  : ${TAG} pushed"
    else
      echo "▶ Git tag  : skipped (not a git repository)"
    fi
    ;;
  latest)
    TAG="latest"
    ;;
  v*.*.*)
    TAG="$ARG"
    ;;
  *)
    echo "ERROR: unknown argument '${ARG}'. Use: patch | minor | major | vX.Y.Z | latest" >&2
    exit 1
    ;;
esac

# ── docker build args ──────────────────────────────────────────────────────────
TAG_ARGS="--tag ${IMAGE}:latest"
if [[ "${TAG}" != "latest" ]]; then
  TAG_ARGS="${TAG_ARGS} --tag ${IMAGE}:${TAG}"
  echo "▶ Image    : ${IMAGE}:${TAG} + ${IMAGE}:latest"
else
  echo "▶ Image    : ${IMAGE}:latest"
fi
echo "▶ Platforms: ${PLATFORMS}"
echo

# ── buildx setup ───────────────────────────────────────────────────────────────
if ! docker buildx inspect "${BUILDER}" &>/dev/null; then
  echo "Creating buildx builder '${BUILDER}'..."
  docker buildx create --name "${BUILDER}" --driver docker-container --bootstrap
fi
docker buildx use "${BUILDER}"

# ── build & push ───────────────────────────────────────────────────────────────
echo "Building and pushing ..."
# shellcheck disable=SC2086
docker buildx build \
  --platform "${PLATFORMS}" \
  --file "${ROOT}/docker/Dockerfile" \
  ${TAG_ARGS} \
  --push \
  "${ROOT}"

echo
if [[ "${TAG}" != "latest" ]]; then
  echo "✓ Done — pushed ${IMAGE}:${TAG} and ${IMAGE}:latest"
else
  echo "✓ Done — pushed ${IMAGE}:latest"
fi
