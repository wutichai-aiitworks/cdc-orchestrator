<script setup lang="ts">
// "Send via SFTP" for one finished run: upload the file already on the dump volume to a server
// typed in here (or a saved one, pre-filled). Nothing is saved to the dump job.
import { Send } from "@lucide/vue"
import { emptyDestination, PGDUMP_REDACTED, usePgDump, type PgDumpDestination, type PgDumpJob } from "~/composables/usePgDump"
import { formatBytes } from "~/utils/bytes"
import { formatApiError } from "~/utils/apiError"

const props = defineProps<{
  open: boolean
  job: PgDumpJob
  /** Saved profile: its servers can be picked; their `********` secrets are filled server-side (same login only). */
  profileId?: string
  saved: PgDumpDestination[]
}>()
const emit = defineEmits<{ "update:open": [boolean]; sent: [PgDumpJob] }>()
const { sendFile } = usePgDump()
const toast = useToast()

const NEW = "new"
const choice = ref<string>(NEW)
const dest = ref<PgDumpDestination>(emptyDestination())
const sending = ref(false)

const options = computed(() => [
  ...props.saved.map((d, i) => ({ value: String(i), label: d.name || d.host })),
  { value: NEW, label: "Another server" },
])
const pick = (v: string) => {
  const d = v === NEW ? undefined : props.saved[Number(v)]
  dest.value = d ? { ...emptyDestination(), ...d } : emptyDestination()
}
watch(choice, pick)
watch(
  () => props.open,
  (o) => {
    if (!o) return
    choice.value = props.saved.length ? "0" : NEW
    pick(choice.value)
  },
  { immediate: true },
)

const missing = computed(() => {
  const d = dest.value
  if (!d.host || !d.username || !d.remote_dir) return "Enter host, username and directory."
  if (d.auth === "key" && !d.private_key) return "Paste the private key."
  if (d.auth === "password" && !d.password) return "Enter the password."
  if (d.host_key_fingerprint && !/^SHA256:[A-Za-z0-9+/]{43}=?$/.test(d.host_key_fingerprint.trim())) return "Fix the host key fingerprint."
  return ""
})
const usesSaved = computed(() => [dest.value.password, dest.value.private_key, dest.value.passphrase].includes(PGDUMP_REDACTED))

const close = () => {
  if (!sending.value) emit("update:open", false)
}
const send = async () => {
  if (missing.value) return
  sending.value = true
  try {
    const job = await sendFile(props.job.id, {
      destination: { ...dest.value, name: dest.value.name || dest.value.host },
      profile_id: usesSaved.value ? props.profileId : undefined,
    })
    toast.success(`Sending to ${dest.value.name || dest.value.host}`)
    emit("sent", job)
    emit("update:open", false)
  } catch (e) {
    toast.error(formatApiError(e, "Could not start the send"))
  } finally {
    sending.value = false
  }
}
</script>

<template>
  <div v-if="open" class="modal-overlay" @click.self="close">
    <div class="flex max-h-[90vh] w-full max-w-2xl flex-col rounded-xl border border-border bg-card shadow-2xl" role="dialog" aria-modal="true" aria-labelledby="pd-send-title">
      <div class="border-b border-border p-5">
        <h3 id="pd-send-title" class="text-base font-semibold text-foreground">Send dump file via SFTP</h3>
        <p v-if="job.artifact" class="mt-1 truncate text-sm text-muted-foreground">
          <span class="font-mono text-xs">{{ job.artifact.name }}</span>, {{ formatBytes(job.artifact.bytes) }}
        </p>
      </div>

      <div class="space-y-4 overflow-auto p-5">
        <div v-if="saved.length">
          <span class="auth-label !mb-1.5">Server</span>
          <TransferSegmented v-model="choice" :options="options" label="SFTP server" />
        </div>
        <PgDumpDestination :key="choice" v-model="dest" :removable="false" open />
        <p class="text-xs text-muted-foreground">
          This send only: the server is not saved to the dump job. The file is uploaded under a temporary name, size-checked, renamed, and a .sha256 file is written beside it. The copy on the dump volume is kept.
        </p>
      </div>

      <div class="flex items-center justify-end gap-2 border-t border-border p-4">
        <span v-if="missing" class="mr-auto text-xs text-muted-foreground">{{ missing }}</span>
        <AppButton :disabled="sending" @click="close">Cancel</AppButton>
        <AppButton variant="primary" :disabled="sending || !!missing" @click="send">
          <Send class="h-3.5 w-3.5" :stroke-width="1.75" />{{ sending ? "Starting…" : "Send file" }}
        </AppButton>
      </div>
    </div>
  </div>
</template>
