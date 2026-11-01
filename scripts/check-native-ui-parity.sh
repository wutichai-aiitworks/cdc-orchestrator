#!/usr/bin/env bash
# Fail if shared native-UI files drift between kafbat-webui/ and cdc-native-ui/.
# Both trees are kept for now (ADR 0002); auth/UI-shell changes must land in both.
#   scripts/check-native-ui-parity.sh          # check
#   scripts/check-native-ui-parity.sh --diff   # also print diffs
set -euo pipefail
cd "$(dirname "$0")/.."

A=kafbat-webui
B=cdc-native-ui
FILES=(
  utils/authMode.ts
  utils/sso.ts
  nuxt.config.ts
  server/utils/sessionConfig.ts
  server/middleware/admin-auth.ts
  server/api/auth/login.post.ts
  server/api/auth/logout.post.ts
  server/api/auth/me.get.ts
  server/plugins/security-startup.ts
  composables/useAuth.ts
  plugins/auth-guard.client.ts
  middleware/auth.global.ts
  middleware/guest.ts
  pages/auth/login.vue
  components/AppUserMenu.vue
  composables/useNavReselect.ts
  composables/useProfilesM3.ts
  composables/useOperationsM3.ts
  components/PipelineDetail.vue
  pages/pipelines.vue
  components/AppActionMenu.vue
  components/ProfileTableSyncPanel.vue
  assets/css/main.css
  .env.example
)

drift=0
for f in "${FILES[@]}"; do
  if [[ ! -f "$A/$f" || ! -f "$B/$f" ]]; then
    echo "MISSING  $f"
    drift=1
  elif ! cmp -s "$A/$f" "$B/$f"; then
    echo "DIFFERS  $f"
    [[ "${1:-}" == "--diff" ]] && diff -u "$A/$f" "$B/$f" || true
    drift=1
  fi
done

if [[ $drift -eq 0 ]]; then
  echo "native UI parity OK (${#FILES[@]} files)"
fi
exit $drift
