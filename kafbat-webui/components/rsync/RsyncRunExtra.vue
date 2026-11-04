<script setup lang="ts">
// File sync's part of a run's detail: which route it took and why, the hosts it found, the
// preview's change list, and host keys seen on unpinned logins (pin in one click).
import { Lock } from "@lucide/vue"
import type { RsyncHostFacts, RsyncJob } from "~/composables/useRsync"
import { formatBytes } from "~/utils/bytes"

const props = defineProps<{ job: RsyncJob; pinned: { source: boolean; target: boolean } }>()
const emit = defineEmits<{ pin: [role: "source" | "target", fingerprint: string] }>()

const OP_LABEL: Record<string, string> = { create: "new", update: "changed", delete: "delete", attrs: "attributes" }
const OP_TONE: Record<string, string> = {
  create: "text-emerald-600 dark:text-emerald-400",
  update: "text-primary",
  delete: "text-red-600 dark:text-red-400",
  attrs: "text-muted-foreground",
}
const hostLine = (h?: RsyncHostFacts) =>
  h
    ? [h.os === "Darwin" ? "macOS" : h.os, h.arch, h.rsync_path && `${h.rsync_path}${h.gnu_rsync === false ? " (limited rsync)" : ""}`, h.free_bytes != null && `${formatBytes(h.free_bytes)} free`]
        .filter(Boolean)
        .join(", ")
    : ""
const items = computed(() => (props.job.changes?.items || []).filter((i) => i.op !== "attrs").slice(0, 200))
const roles = ["source", "target"] as const
</script>

<template>
  <div class="space-y-4">
    <dl class="grid gap-x-6 gap-y-2 text-sm sm:grid-cols-[8rem_minmax(0,1fr)]">
      <template v-if="job.transport">
        <dt class="text-muted-foreground">Route</dt>
        <dd>
          {{ job.transport === "direct" ? "Direct, source to target" : "Relay, through the sync service" }}
          <span v-if="job.transport_reason" class="block text-xs text-muted-foreground">{{ job.transport_reason }}</span>
        </dd>
      </template>
      <template v-for="r in roles" :key="r">
        <template v-if="job.hosts?.[r]">
          <dt class="capitalize text-muted-foreground">{{ r }}</dt>
          <dd class="min-w-0 truncate text-xs text-muted-foreground" :title="job.hosts[r]?.rsync_version">{{ hostLine(job.hosts[r]) }}</dd>
        </template>
      </template>
      <template v-if="job.stats_pull && job.transport === 'relay'">
        <dt class="text-muted-foreground">Pulled</dt>
        <dd class="text-xs tabular-nums text-muted-foreground">
          {{ job.stats_pull.number_of_regular_files_transferred ?? 0 }} files, {{ formatBytes(job.stats_pull.total_transferred_file_size) }} into the relay copy<template v-if="job.staging_discarded">, then removed</template>
        </dd>
      </template>
    </dl>

    <div v-if="job.dry_run && job.changes" class="space-y-2">
      <h4 class="text-sm font-medium text-foreground">
        What a sync would change
        <span class="ml-1 text-xs font-normal tabular-nums text-muted-foreground">
          {{ job.changes.counts.create || 0 }} new, {{ job.changes.counts.update || 0 }} changed, {{ job.changes.counts.delete || 0 }} deleted
        </span>
      </h4>
      <div class="max-h-72 overflow-auto rounded-lg border border-border">
        <div v-for="(i, n) in items" :key="n" class="flex gap-3 border-b border-border px-3 py-1.5 text-xs last:border-b-0">
          <span class="w-20 shrink-0 font-medium" :class="OP_TONE[i.op]">{{ OP_LABEL[i.op] || i.op }}</span>
          <span class="min-w-0 truncate font-mono" :title="i.path">{{ i.path }}</span>
        </div>
        <p v-if="!items.length" class="px-3 py-2 text-xs text-muted-foreground">Source and target already match.</p>
      </div>
      <p v-if="job.changes.truncated" class="text-xs text-muted-foreground">Only the first 500 entries are listed; the log has the rest.</p>
    </div>
    <p v-else-if="job.dry_run && job.status === 'succeeded'" class="text-sm text-muted-foreground">Preview: source and target already match.</p>

    <div v-if="job.host_keys" class="space-y-1">
      <template v-for="r in roles" :key="r">
        <div v-if="job.host_keys[r] && !job.host_keys[r].verified" class="flex flex-wrap items-center gap-2 text-xs">
          <span class="capitalize text-amber-600 dark:text-amber-400">{{ r }} host key not pinned:</span>
          <span class="break-all font-mono text-muted-foreground">{{ job.host_keys[r].fingerprint }}</span>
          <AppButton v-if="!pinned[r]" size="sm" title="Use this fingerprint; save to keep it" @click="emit('pin', r, job.host_keys[r].fingerprint)">
            <Lock class="h-3.5 w-3.5" :stroke-width="1.75" />Pin this key
          </AppButton>
        </div>
      </template>
    </div>
  </div>
</template>
