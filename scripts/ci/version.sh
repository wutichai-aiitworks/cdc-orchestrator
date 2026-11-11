#!/bin/sh
# Prints the build version used as the image tag and on the WebUI login page.
#   v* tag  -> the tag as-is (v1.2.3), no commit id
#   branch  -> <branch-slug>_<short-sha>   (e.g. feature-capture_view_31d8e3f)
#   neither -> sha_<short-sha>
# Inputs: Woodpecker's CI_COMMIT_TAG, CI_COMMIT_BRANCH, CI_COMMIT_SHA. POSIX sh (busybox ok).
set -e

if [ -n "${CI_COMMIT_TAG:-}" ]; then
  echo "${CI_COMMIT_TAG}"
  exit 0
fi

SHORT_SHA="$(echo "${CI_COMMIT_SHA:?CI_COMMIT_SHA is required}" | cut -c1-7)"
SLUG="$(echo "${CI_COMMIT_BRANCH:-}" | tr '[:upper:]' '[:lower:]' \
  | sed 's#[^a-z0-9_.-]#-#g; s#-\{2,\}#-#g; s#^-##; s#-$##')"
echo "${SLUG:-sha}_${SHORT_SHA}"
