<script setup lang="ts">
// pgdump's part of a run's detail: the file it produced, where it was stored, host keys seen on
// unpinned uploads (pin in one click), Download, and Send via SFTP (ad hoc, not saved).
import { Copy, Download, Lock, Send, Square } from "@lucide/vue"
import { pgDumpFileUrl, usePgDump, type PgDumpDestination, type PgDumpJob } from "~/composables/usePgDump"
import { formatBytes } from "~/utils/bytes"
import { startedLabel } from "~/utils/transferView"

const props = defineProps<{
  job: PgDumpJob
  retentionHours?: number
  /** Destination names that already have a pinned fingerprint. */
  pinnedNames: string[]
  /** Saved profile and its servers, offered in the send dialog. */
  profileId?: string
  savedDestinations?: PgDumpDestination[]
}>()
const emit = defineEmits<{ pin: [name: string, fingerprint: string] }>()
const toast = useToast()
const { fetchJob, stopJob } = usePgDump()

// The workspace polls slowly once a run has finished, so a send is followed here.
const live = ref<PgDumpJob | null>(null)
const job = computed(() => (live.value && live.value.id === props.job.id ? live.value : props.job))
const sendOpen = ref(false)
let timer: ReturnType<typeof setTimeout> | null = null
const follow = async () => {
  timer = null
  try {
    live.value = await fetchJob(props.job.id)
  } catch {
    return
  }
  if (live.value?.sending) timer = setTimeout(follow, 1500)
  else {
    const last = live.value?.uploads?.at(-1)
    if (last?.status === "uploaded") toast.success(`Sent to ${last.name}`)
    else if (last?.status === "failed") toast.error(`Send to ${last.name} failed`)
  }
}
const onSent = (j: PgDumpJob) => {
  live.value = j
  if (!timer) timer = setTimeout(follow, 1000)
}
const stopSend = async () => {
  try {
    await stopJob(props.job.id)
  } catch {
    toast.error("Could not stop the send")
  }
}
onMounted(() => {
  if (props.job.sending) follow()
})
onBeforeUnmount(() => timer && clearTimeout(timer))

const keptUntil = computed(() => {
  const end = Date.parse(job.value.finished_at || "")
  if (!props.retentionHours || !Number.isFinite(end) || !job.value.download_path) return ""
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
      <div v-for="(u, i) in job.uploads" :key="i" class="rounded-lg border border-border p-3 text-sm">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <span class="font-medium">{{ u.name }}<span v-if="u.adhoc" class="ml-2 rounded bg-muted px-1.5 py-0.5 text-xs font-normal text-muted-foreground" title="Sent with Send via SFTP; not a saved server">ad hoc</span></span>
          <span class="text-xs font-medium capitalize tabular-nums" :class="uploadTone(u.status)">
            {{ u.status === "uploading" && u.bytes_sent != null ? `uploading, ${formatBytes(u.bytes_sent)}` : u.status }}
          </span>
        </div>
        <div v-if="u.remote_path || u.remote_dir" class="mt-1 truncate font-mono text-xs text-muted-foreground">{{ u.remote_path || u.remote_dir }}</div>
        <div v-if="u.status === 'failed' && u.error" class="mt-1 text-xs text-red-600 dark:text-red-400">{{ u.error }}<span v-if="u.hint" class="block text-muted-foreground">{{ u.hint }}</span></div>
        <div v-if="u.host_key_fingerprint" class="mt-2 flex flex-wrap items-center gap-2 text-xs">
          <span :class="u.host_key_verified ? 'text-emerald-600 dark:text-emerald-400' : 'text-amber-600 dark:text-amber-400'">
            {{ u.host_key_verified ? "Host key matched the pinned one" : "Host key not pinned:" }}
          </span>
          <span v-if="!u.host_key_verified" class="break-all font-mono text-muted-foreground">{{ u.host_key_fingerprint }}</span>
          <AppButton
            v-if="!u.host_key_verified && !u.adhoc && !pinnedNames.includes(u.name)"
            size="sm"
            title="Use this fingerprint for the server; save to keep it"
            @click="emit('pin', u.name, u.host_key_fingerprint!)"
          >
            <Lock class="h-3.5 w-3.5" :stroke-width="1.75" />Pin this key
          </AppButton>
        </div>
      </div>
    </div>

    <div v-if="job.download_path" class="flex flex-wrap gap-2">
      <a :href="pgDumpFileUrl(job.id)" class="btn-primary !h-8 !min-h-8 !w-auto !px-3 !text-xs" download>
        <Download class="h-3.5 w-3.5" :stroke-width="1.75" />Download file
      </a>
      <AppButton v-if="!job.sending" size="sm" title="Upload this file to an SFTP server now, without saving the server" @click="sendOpen = true">
        <Send class="h-3.5 w-3.5" :stroke-width="1.75" />Send via SFTP
      </AppButton>
      <AppButton v-else size="sm" @click="stopSend"><Square class="h-3.5 w-3.5" :stroke-width="1.75" />Stop sending</AppButton>
    </div>

    <PgDumpSendDialog
      v-model:open="sendOpen"
      :job="job"
      :profile-id="profileId"
      :saved="savedDestinations || []"
      @sent="onSent"
    />
  </div>
</template>
