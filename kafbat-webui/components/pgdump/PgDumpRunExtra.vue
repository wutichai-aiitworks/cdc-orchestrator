<script setup lang="ts">
// pgdump's part of a run's detail: the file it produced, where it was stored, host keys seen on
// unpinned uploads (pin in one click), and Download.
import { Copy, Download, Lock } from "@lucide/vue"
import { pgDumpFileUrl, type PgDumpJob } from "~/composables/usePgDump"
import { formatBytes } from "~/utils/bytes"
import { startedLabel } from "~/utils/transferView"

const props = defineProps<{
  job: PgDumpJob
  retentionHours?: number
  /** Destination names that already have a pinned fingerprint. */
  pinnedNames: string[]
}>()
const emit = defineEmits<{ pin: [name: string, fingerprint: string] }>()
const toast = useToast()

const keptUntil = computed(() => {
  const end = Date.parse(props.job.finished_at || "")
  if (!props.retentionHours || !Number.isFinite(end) || !props.job.download_path) return ""
  return startedLabel(new Date(end + props.retentionHours * 3600_000).toISOString())
})
const copy = async (text: string, what: string) => {
  try {
    await navigator.clipboard.writeText(text)
    toast.success(`${what} copied`)
  } catch {
    toast.error("Copy failed")
  }
}
const uploadTone = (s: string) =>
  s === "uploaded" ? "text-emerald-600 dark:text-emerald-400" : s === "failed" ? "text-red-600 dark:text-red-400" : "text-muted-foreground"
</script>

<template>
  <div class="space-y-4">
    <dl v-if="job.artifact || (job.mode === 'dump_restore' && job.target)" class="grid gap-x-6 gap-y-2 text-sm sm:grid-cols-[8rem_minmax(0,1fr)]">
      <template v-if="job.artifact">
        <dt class="text-muted-foreground">File</dt>
        <dd class="min-w-0">
          <div class="truncate font-mono text-xs" :title="job.artifact.name">{{ job.artifact.name }}</div>
          <div class="text-xs tabular-nums text-muted-foreground">
            {{ formatBytes(job.artifact.bytes) }}, {{ job.artifact.format }} format<template v-if="job.download_path">, on the dump volume<template v-if="keptUntil"> until {{ keptUntil }}</template></template><template v-else-if="job.artifact.local_deleted">, removed from the dump volume after upload</template>
          </div>
        </dd>
        <template v-if="job.artifact.sha256">
          <dt class="text-muted-foreground">SHA-256</dt>
          <dd class="flex min-w-0 items-center gap-1">
            <span class="truncate font-mono text-xs text-muted-foreground">{{ job.artifact.sha256 }}</span>
            <AppButton size="icon" variant="ghost" title="Copy checksum" @click="copy(job.artifact.sha256!, 'Checksum')"><Copy class="h-3.5 w-3.5" :stroke-width="1.75" /></AppButton>
          </dd>
        </template>
      </template>
      <template v-if="job.mode === 'dump_restore' && job.target">
        <dt class="text-muted-foreground">Restored into</dt>
        <dd class="truncate">{{ job.target }}<span v-if="job.tables_restored != null" class="text-muted-foreground">, {{ job.tables_restored }} tables</span></dd>
      </template>
    </dl>

    <div v-if="job.uploads?.length" class="space-y-2">
      <h4 class="text-sm font-medium text-foreground">Stored on</h4>
      <div v-for="u in job.uploads" :key="u.name" class="rounded-lg border border-border p-3 text-sm">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <span class="font-medium">{{ u.name }}</span>
          <span class="text-xs font-medium capitalize tabular-nums" :class="uploadTone(u.status)">
            {{ u.status === "uploading" && u.bytes_sent != null ? `uploading, ${formatBytes(u.bytes_sent)}` : u.status }}
          </span>
        </div>
        <div v-if="u.remote_path || u.remote_dir" class="mt-1 truncate font-mono text-xs text-muted-foreground">{{ u.remote_path || u.remote_dir }}</div>
        <div v-if="u.host_key_fingerprint" class="mt-2 flex flex-wrap items-center gap-2 text-xs">
          <span :class="u.host_key_verified ? 'text-emerald-600 dark:text-emerald-400' : 'text-amber-600 dark:text-amber-400'">
            {{ u.host_key_verified ? "Host key matched the pinned one" : "Host key not pinned:" }}
          </span>
          <span v-if="!u.host_key_verified" class="break-all font-mono text-muted-foreground">{{ u.host_key_fingerprint }}</span>
          <AppButton
            v-if="!u.host_key_verified && !pinnedNames.includes(u.name)"
            size="sm"
            title="Use this fingerprint for the server; save to keep it"
            @click="emit('pin', u.name, u.host_key_fingerprint!)"
          >
            <Lock class="h-3.5 w-3.5" :stroke-width="1.75" />Pin this key
          </AppButton>
        </div>
      </div>
    </div>

    <a v-if="job.download_path" :href="pgDumpFileUrl(job.id)" class="btn-primary !h-8 !min-h-8 !w-auto !px-3 !text-xs" download>
      <Download class="h-3.5 w-3.5" :stroke-width="1.75" />Download file
    </a>
  </div>
</template>
