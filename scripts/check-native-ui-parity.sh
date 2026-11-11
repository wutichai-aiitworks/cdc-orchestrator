#!/usr/bin/env bash
# Fail if shared native-UI files drift between kafbat-webui/ and cdc-native-ui/.
# LEGACY (2026-11-07): UI work happens in cdc-native-ui only; kafbat-webui is frozen and this check
# is no longer part of `npm run check`. Kept for reference only.
#   scripts/check-native-ui-parity.sh          # check
#   scripts/check-native-ui-parity.sh --diff   # also print diffs
set -euo pipefail
cd "$(dirname "$0")/.."

A=kafbat-webui
B=cdc-native-ui
FILES=(
  composables/useTransferTest.ts
  composables/usePgCopy.ts
  composables/usePgLoader.ts
  composables/useMongoCopy.ts
  composables/useRedisShake.ts
  server/api/cdc/pgcopy/test-connection.post.ts
  server/api/cdc/pgloader/test-connection.post.ts
  server/api/cdc/mongocopy/test-connection.post.ts
  server/api/cdc/redisshake/test-connection.post.ts
  server/api/cdc/pgcopy/validate.post.ts
  server/api/cdc/pgcopy/profiles/index.post.ts
  server/api/cdc/pgcopy/profiles/[id]/index.put.ts
  server/api/cdc/pgloader/validate.post.ts
  server/api/cdc/pgloader/profiles/index.post.ts
  server/api/cdc/pgloader/profiles/[id]/index.put.ts
  server/api/cdc/mongocopy/validate.post.ts
  server/api/cdc/mongocopy/profiles/index.post.ts
  server/api/cdc/mongocopy/profiles/[id]/index.put.ts
  server/api/cdc/redisshake/validate.post.ts
  server/api/cdc/redisshake/profiles/index.post.ts
  server/api/cdc/redisshake/profiles/[id]/index.put.ts
  pages/pgcopy.vue
  pages/pgloader.vue
  pages/mongocopy.vue
  pages/redisshake.vue
  utils/transferView.ts
  utils/transferTools/types.ts
  utils/transferTools/pgcopy.ts
  utils/transferTools/pgloader.ts
  utils/transferTools/mongocopy.ts
  utils/transferTools/redisshake.ts
  components/transfer/TransferWorkspace.vue
  components/transfer/TransferConnection.vue
  components/transfer/TransferRunDetail.vue
  components/transfer/TransferEngineMark.vue
  components/pgdump/PgDumpRunExtra.vue
  pages/pgdump.vue
  composables/usePgDump.ts
  utils/pgDumpView.ts
  server/api/cdc/pgdump/test-connection.post.ts
  components/transfer/TransferRoute.vue
  components/transfer/TransferSection.vue
  components/transfer/TransferSwitch.vue
  components/transfer/TransferSegmented.vue
  components/transfer/TransferLog.vue
  components/transfer/TransferSaveBar.vue
  components/transfer/TransferRail.vue
  components/transfer/TransferTokens.vue
  components/pgdump/PgDumpDestination.vue
  composables/useRsync.ts
  utils/transferTools/rsync.ts
  pages/rsync.vue
  components/rsync/RsyncEndpoint.vue
  components/rsync/RsyncRunExtra.vue
  server/api/cdc/rsync/test-connection.post.ts
  server/api/cdc/rsync/test-route.post.ts
  server/api/cdc/rsync/profiles/[id]/run.post.ts
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
  components/ErrorDiagnosis.vue
  components/PipelineActivityTimeline.vue
  utils/pipelineEvents.ts
  utils/apiError.ts
  components/TargetTableDdlDialog.vue
  components/ui/AppCombobox.vue
  utils/targetDdl.ts
  utils/dbTypeTranslator.ts
  server/api/cdc/profiles/catalog/target/ddl.post.ts
  components/InitialLoadDialog.vue
  components/SplitPipelineDialog.vue
  server/api/cdc/profiles/[id]/split.post.ts
  components/TablePollEditor.vue
  server/api/cdc/profiles/[id]/initial-load.post.ts
  server/api/cdc/profiles/[id]/initial-load.get.ts
  server/api/cdc/profiles/[id]/initial-load.delete.ts
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
