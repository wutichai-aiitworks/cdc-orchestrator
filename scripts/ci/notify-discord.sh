#!/bin/sh
# Discord embed used by Woodpecker (same pattern as supertokens-gateway).
# Step outcome comes from .ci-status/<key>:
#   missing file -> skipped, "success" -> pass, anything else (including "running") -> fail.
# On a successful publish the digest-pinned images.env is attached to the message.
set -e

: "${DISCORD_WEBHOOK:?DISCORD_WEBHOOK is required}"
: "${REGISTRY_NAMESPACE:?REGISTRY_NAMESPACE is required}"

HERE="$(cd "$(dirname "$0")" && pwd)"
HEADING_NAME="${HEADING_NAME:-cdc-orchestrator}"
VERSION="${VERSION:-$(sh "$HERE/version.sh")}"
SHORT_SHA="$(echo "${CI_COMMIT_SHA}" | cut -c1-7)"
BUILD_NO="${CI_PIPELINE_NUMBER:-n/a}"
RUN_URL="${CI_PIPELINE_URL:-${CI_BUILD_LINK}}"
COMMIT_URL="${CI_COMMIT_LINK:-${CI_REPO_LINK}/commit/${CI_COMMIT_SHA}}"
TS="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
WORKFLOW="${CI_WORKFLOW_NAME:-main-ci}"

if [ -n "${CI_COMMIT_TAG:-}" ]; then
  REF_NAME="Tag"; REF_VALUE="${CI_COMMIT_TAG}"
  TAGS_MD='`latest`, `'"${VERSION}"'`'
else
  REF_NAME="Branch"; REF_VALUE="${CI_COMMIT_BRANCH:-unknown}"
  TAGS_MD='`'"${VERSION}"'`'
fi
PLATFORMS_MD='`linux/amd64`, `linux/arm64`'

icon_for() {
  file=".ci-status/$1"
  if [ ! -f "$file" ]; then
    printf '%s' "⏭️ skipped"
  elif [ "$(tr -d '[:space:]' < "$file")" = "success" ]; then
    printf '%s' "✅"
  else
    printf '%s' "❌"
  fi
}

line_for() {
  printf '%-10s %s' "$1" "$(icon_for "$2")"
}

add_line() {
  if [ -z "${FLOW:-}" ]; then
    FLOW="$1"
  else
    FLOW="${FLOW}\n$1"
  fi
}

add_line "$(printf '%-10s %s' checkout '✅')"
add_line "$(line_for rust rust)"
add_line "$(line_for mcp mcp)"
add_line "$(line_for webui webui)"
add_line "$(line_for sidecars sidecars)"
add_line "$(line_for wireguard wireguard)"
add_line "$(line_for jdbc-admin jdbc-admin)"
add_line "$(line_for sonar sonar)"
add_line "$(line_for build build)"
# One line per image (scripts/ci/images.txt), only when the build step ran:
#   ✅ pushed · ❌ failed while building/pushing · ⏭️ not reached (an earlier image failed).
if [ -f .ci-status/build ]; then
  while IFS='|' read -r _var name _rest; do
    case "$_var" in ''|'#'*) continue ;; esac
    add_line "$(printf '  %-17s %s' "$name" "$(icon_for "image-$name")")"
  done < "$HERE/images.txt"
fi
add_line "$(line_for publish publish)"

COLOR=5763719
HEADING="${HEADING_NAME} pipeline succeeded"
case "$FLOW" in
  *"❌"*)
    COLOR=15548997
    HEADING="${HEADING_NAME} pipeline failed"
    ;;
esac

# Published only when the publish line is green; then images.env exists and is attached.
ATTACH=""
state_of() { if [ -f ".ci-status/$1" ]; then tr -d '[:space:]' < ".ci-status/$1"; fi; }
BUILT="$(state_of build)"
PUBLISHED="$(state_of publish)"
if [ "$PUBLISHED" = "success" ] && [ -s .ci-status/images.env ]; then
  ATTACH=".ci-status/images.env"
  # Names are already in the per-image lines above; here just where and the attachment.
  COUNT="$(wc -l < "$ATTACH" | tr -d ' ')"
  IMAGES_NAME="Images (${COUNT})"
  IMAGES_MD='`'"${REGISTRY_NAMESPACE}"'/cdc-*` · digest-pinned `images.env` attached for docker/.env.production'
elif [ "$BUILT" = "success" ]; then
  # Every image was pushed under the version tag; only the manifest / 'latest' step failed.
  IMAGES_NAME="Images"
  IMAGES_MD='`'"${REGISTRY_NAMESPACE}"'/cdc-*` pushed as `'"${VERSION}"'`; publish step failed'
  TAGS_MD='`'"${VERSION}"'`'
  if [ -n "${CI_COMMIT_TAG:-}" ]; then TAGS_MD="${TAGS_MD} · \`latest\` unchanged"; fi
elif [ -n "$BUILT" ]; then
  # Build failed partway: images before the failing one are already pushed (version tag only).
  IMAGES_NAME="Images"
  IMAGES_MD='`'"${REGISTRY_NAMESPACE}"'/cdc-*` build failed; earlier images may be pushed as `'"${VERSION}"'`'
  if [ -n "${CI_COMMIT_TAG:-}" ]; then IMAGES_MD="${IMAGES_MD} · \`latest\` unchanged"; fi
  TAGS_MD='`'"${VERSION}"'` (incomplete)'
else
  IMAGES_NAME="Images"
  IMAGES_MD='`'"${REGISTRY_NAMESPACE}"'/cdc-*` not published'
  if [ -n "${CI_COMMIT_TAG:-}" ]; then IMAGES_MD="${IMAGES_MD} · \`latest\` unchanged"; fi
  TAGS_MD='`'"${VERSION}"'` (not pushed)'
fi

# curl image runs as a non-root user. .ci-status is created by an earlier
# root step, so the payload goes to /tmp, which this user can write.
cat > /tmp/discord-payload.json <<EOF
{
  "embeds": [
    {
      "title": "${HEADING}",
      "description": "\`\`\`\n${FLOW}\n\`\`\`",
      "color": ${COLOR},
      "fields": [
        { "name": "Project", "value": "${CI_REPO_NAME}", "inline": true },
        { "name": "${REF_NAME}", "value": "${REF_VALUE}", "inline": true },
        { "name": "Triggered by", "value": "${CI_COMMIT_AUTHOR}", "inline": true },
        { "name": "${IMAGES_NAME}", "value": "${IMAGES_MD}", "inline": false },
        { "name": "Tags", "value": "${TAGS_MD}", "inline": true },
        { "name": "Platforms", "value": "${PLATFORMS_MD}", "inline": true },
        { "name": "Commit", "value": "[\`${SHORT_SHA}\`](${COMMIT_URL})", "inline": true },
        { "name": "Pipeline", "value": "[#${BUILD_NO}](${RUN_URL}) · \`${WORKFLOW}\`", "inline": true }
      ],
      "footer": { "text": "Woodpecker CI" },
      "timestamp": "${TS}"
    }
  ]
}
EOF

if [ -n "$ATTACH" ]; then
  # multipart: embed + file in one message.
  curl -fsS -X POST "${DISCORD_WEBHOOK}" \
    -F "payload_json=</tmp/discord-payload.json" \
    -F "files[0]=@${ATTACH};filename=images-${VERSION}.env"
else
  curl -fsS -X POST "${DISCORD_WEBHOOK}" \
    -H "Content-Type: application/json" \
    --data-binary @/tmp/discord-payload.json
fi
