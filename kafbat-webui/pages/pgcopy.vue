<script setup lang="ts">
import { Copy, Play, RefreshCw, Save, Square, Trash2 } from "@lucide/vue"
import {
  defaultPgCopyOptions,
  type PgCopyConn,
  type PgCopyJob,
  type PgCopyProfile,
  usePgCopy,
} from "~/composables/usePgCopy"
import { applyContractOptionDefaults } from "~/composables/useMigrationTools"
import { formatApiError } from "~/utils/apiError"

definePageMeta({ layout: "default" })

const toast = useToast()
const {
  fetchContract,
  fetchCapability,
  fetchProfiles,
  createProfile,
  updateProfile,
  deleteProfile,
  runProfile,
  fetchJobs,
  fetchJobLogs,
  stopJob,
  validateConnection,
} = usePgCopy()

const BOOL_OPTIONS: Array<{ key: keyof ReturnType<typeof defaultPgCopyOptions>; label: string }> = [
  { key: "drop_if_exists", label: "Drop if exists on target" },
  { key: "roles", label: "Copy roles" },
  { key: "no_role_passwords", label: "Skip role passwords" },
  { key: "no_owner", label: "Skip owner restore" },
  { key: "no_acl", label: "Skip ACL restore" },
  { key: "no_comments", label: "Skip comments" },
  { key: "no_tablespaces", label: "Skip tablespaces" },
  { key: "skip_large_objects", label: "Skip large objects" },
  { key: "skip_extensions", label: "Skip extensions" },
  { key: "skip_ext_comments", label: "Skip extension comments" },
  { key: "skip_collations", label: "Skip collations" },
  { key: "skip_vacuum", label: "Skip VACUUM" },
  { key: "skip_analyze", label: "Skip ANALYZE" },
  { key: "skip_db_properties", label: "Skip DB properties" },
  { key: "skip_split_by_ctid", label: "Skip CTID split" },
  { key: "fail_fast", label: "Fail fast" },
  { key: "restart", label: "Restart (reuse temp files)" },
  { key: "resume", label: "Resume after failure" },
  { key: "not_consistent", label: "Allow new snapshot (not consistent)" },
  { key: "follow", label: "Follow (logical decoding after copy)" },
  { key: "create_slot", label: "Create replication slot" },
  { key: "wal2json_numeric_as_string", label: "wal2json numeric as string" },
  { key: "use_copy_binary", label: "COPY binary format" },
  { key: "all_databases", label: "All databases on instance" },
  { key: "estimate_table_sizes", label: "Estimate table sizes" },
]

const emptyConn = (): PgCopyConn => ({
  type: "postgres",
  host: "",
  port: "5432",
  database: "",
  username: "",
  password: "",
  schema: "",
})

const profiles = ref<PgCopyProfile[]>([])
const jobs = ref<PgCopyJob[]>([])
const selectedId = ref("")
const loading = ref(false)
const saving = ref(false)
const running = ref(false)
const sidecarVersion = ref("")
const includeTablesText = ref("")
const logText = ref("")
const logJobId = ref("")
const error = ref("")

const form = reactive({
  name: "",
  source: emptyConn(),
  target: emptyConn(),
  options: defaultPgCopyOptions(),
  plugin: "",
  slot_name: "",
  origin: "",
})

const selectedProfile = computed(() => profiles.value.find((p) => p.id === selectedId.value) || null)

const loadAll = async () => {
  loading.value = true
  error.value = ""
  try {
    const [cap, profs, jobList] = await Promise.all([
      fetchCapability().catch(() => ({})),
      fetchProfiles(),
      fetchJobs(),
    ])
    sidecarVersion.value = String((cap as any)?.pgcopydb_version || "")
    profiles.value = profs
    jobs.value = jobList
    if (!selectedId.value && profs.length) selectProfile(profs[0].id)
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load pgcopy")
  } finally {
    loading.value = false
  }
}

const selectProfile = (id: string) => {
  selectedId.value = id
  const p = profiles.value.find((item) => item.id === id)
  if (!p) return
  form.name = p.name
  form.source = { ...emptyConn(), ...p.source }
  form.target = { ...emptyConn(), ...p.target }
  form.options = { ...defaultPgCopyOptions(), ...p.options }
  form.plugin = p.options.plugin || ""
  form.slot_name = p.options.slot_name || ""
  form.origin = p.options.origin || ""
  includeTablesText.value = (p.include_tables || []).join("\n")
}

const payloadFromForm = () => {
  const include_tables = includeTablesText.value
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
  const options = {
    ...form.options,
    plugin: form.plugin.trim() || undefined,
    slot_name: form.slot_name.trim() || undefined,
    origin: form.origin.trim() || undefined,
  }
  return {
    name: form.name.trim(),
    source: form.source,
    target: form.target,
    include_tables,
    options,
  }
}

const saveProfile = async () => {
  if (!form.name.trim()) {
    toast.error("Profile name is required")
    return
  }
  saving.value = true
  try {
    const body = payloadFromForm()
    if (selectedId.value) {
      const updated = await updateProfile(selectedId.value, body)
      const idx = profiles.value.findIndex((p) => p.id === updated.id)
      if (idx >= 0) profiles.value[idx] = updated
      toast.success("Profile saved")
    } else {
      const created = await createProfile(body as any)
      profiles.value.push(created)
      selectProfile(created.id)
      toast.success("Profile created")
    }
  } catch (e: any) {
    toast.error(formatApiError(e, "Request failed"))
  } finally {
    saving.value = false
  }
}

const contractDefaults = ref<Record<string, unknown> | null>(null)

const newProfile = () => {
  selectedId.value = ""
  form.name = ""
  form.source = emptyConn()
  form.target = emptyConn()
  form.options = applyContractOptionDefaults(
    defaultPgCopyOptions(),
    contractDefaults.value ?? undefined,
  )
  form.plugin = ""
  form.slot_name = ""
  form.origin = ""
  includeTablesText.value = ""
}

const removeProfile = async () => {
  if (!selectedId.value) return
  if (!confirm("Delete this pgcopy profile?")) return
  try {
    await deleteProfile(selectedId.value)
    profiles.value = profiles.value.filter((p) => p.id !== selectedId.value)
    newProfile()
    toast.success("Profile deleted")
  } catch (e: any) {
    toast.error(formatApiError(e, "Request failed"))
  }
}

const runSelected = async () => {
  if (!selectedId.value) {
    toast.error("Save a profile before running")
    return
  }
  running.value = true
  try {
    await saveProfile()
    const job = await runProfile(selectedId.value)
    toast.success(`Clone job started (${job.id})`)
    await loadAll()
    if (job.id) await showLogs(job.id)
  } catch (e: any) {
    toast.error(formatApiError(e, "Request failed"))
  } finally {
    running.value = false
  }
}

const showLogs = async (jobId: string) => {
  logJobId.value = jobId
  try {
    const res = await fetchJobLogs(jobId)
    logText.value = res.log || ""
  } catch (e: any) {
    logText.value = formatApiError(e, "Failed to load logs")
  }
}

const stopRunningJob = async (jobId: string) => {
  try {
    await stopJob(jobId)
    toast.success("Stop requested")
    await loadAll()
  } catch (e: any) {
    toast.error(formatApiError(e, "Request failed"))
  }
}

const testConn = async (which: "source" | "target") => {
  const connection = which === "source" ? form.source : form.target
  try {
    const res = await validateConnection(connection)
    toast[res.valid ? "success" : "error"](res.message)
  } catch (e: any) {
    toast.error(formatApiError(e, "Request failed"))
  }
}

let pollTimer: ReturnType<typeof setInterval> | null = null

onMounted(async () => {
  fetchContract()
    .then((c) => {
      contractDefaults.value = c
    })
    .catch(() => {})
  loadAll()
  pollTimer = setInterval(() => {
    fetchJobs().then((j) => (jobs.value = j)).catch(() => {})
    if (logJobId.value) showLogs(logJobId.value)
  }, 8000)
})

onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer)
})
</script>

<template>
  <div class="space-y-6 animate-fade-in">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h1 class="text-xl font-semibold text-foreground flex items-center gap-2">
          <Copy class="h-5 w-5 text-primary" />
          Postgres copy (pgcopydb)
        </h1>
        <div class="mt-1 flex items-center gap-1.5">
          <span class="text-sm font-medium text-foreground">pgcopydb clone</span>
          <AppInfoHint
            :text="`Configure parallel Postgres copy from source to target.${sidecarVersion ? ` Sidecar ${sidecarVersion}.` : ''}`"
          />
        </div>
      </div>
      <div class="flex shrink-0 items-center gap-2">
        <button type="button" class="btn-secondary !w-auto whitespace-nowrap inline-flex items-center gap-2" @click="loadAll">
          <RefreshCw class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
          Refresh
        </button>
        <button type="button" class="btn-secondary !w-auto whitespace-nowrap" @click="newProfile">New profile</button>
      </div>
    </div>

    <p v-if="error" class="alert-error text-sm">{{ error }}</p>

    <div class="grid grid-cols-1 xl:grid-cols-12 gap-6">
      <aside class="xl:col-span-3 card p-4 space-y-2">
        <h2 class="text-sm font-medium text-muted-foreground">Saved profiles</h2>
        <button
          v-for="p in profiles"
          :key="p.id"
          type="button"
          class="w-full text-left rounded-xl px-3 py-2 text-sm border transition-colors"
          :class="selectedId === p.id ? 'border-primary bg-primary/10' : 'border-border hover:bg-muted/40'"
          @click="selectProfile(p.id)"
        >
          <div class="font-medium truncate">{{ p.name }}</div>
          <div class="text-xs text-muted-foreground truncate">{{ p.source.host }} → {{ p.target.host }}</div>
        </button>
        <p v-if="!profiles.length" class="text-sm text-muted-foreground italic">No profiles yet.</p>
      </aside>

      <section class="xl:col-span-9 space-y-4">
        <div class="card p-5 space-y-4">
          <input
            v-model="form.name"
            type="text"
            placeholder="Profile name"
            class="auth-input w-full"
          />

          <div class="grid md:grid-cols-2 gap-4">
            <div v-for="side in (['source', 'target'] as const)" :key="side" class="space-y-2 rounded-xl border border-border p-4">
              <div class="flex items-center justify-between">
                <h3 class="text-sm font-semibold capitalize">{{ side }}</h3>
                <button type="button" class="text-xs text-primary underline" @click="testConn(side)">Validate</button>
              </div>
              <input v-model="form[side].host" class="auth-input w-full" placeholder="Host" />
              <div class="grid grid-cols-2 gap-2">
                <input v-model="form[side].port" class="auth-input" placeholder="Port" />
                <input v-model="form[side].database" class="auth-input" placeholder="Database" />
              </div>
              <input v-model="form[side].username" class="auth-input w-full" placeholder="Username" />
              <input v-model="form[side].password" type="password" class="auth-input w-full" placeholder="Password" />
            </div>
          </div>

          <div>
            <label class="auth-label">Include tables (optional, one per line: schema.table)</label>
            <textarea v-model="includeTablesText" rows="3" class="auth-input w-full font-mono text-xs" />
          </div>

          <div class="grid sm:grid-cols-2 lg:grid-cols-4 gap-3">
            <label class="text-xs text-muted-foreground">Table jobs
              <input v-model.number="form.options.table_jobs" type="number" min="1" class="auth-input w-full mt-1" />
            </label>
            <label class="text-xs text-muted-foreground">Index jobs
              <input v-model.number="form.options.index_jobs" type="number" min="1" class="auth-input w-full mt-1" />
            </label>
            <label class="text-xs text-muted-foreground">Restore jobs
              <input v-model.number="form.options.restore_jobs" type="number" min="1" class="auth-input w-full mt-1" />
            </label>
            <label class="text-xs text-muted-foreground">Large object jobs
              <input v-model.number="form.options.large_objects_jobs" type="number" min="1" class="auth-input w-full mt-1" />
            </label>
          </div>

          <div class="grid sm:grid-cols-2 lg:grid-cols-3 gap-2">
            <label v-for="opt in BOOL_OPTIONS" :key="String(opt.key)" class="flex items-center gap-2 text-sm">
              <input v-model="(form.options as any)[opt.key]" type="checkbox" class="rounded border-border" />
              {{ opt.label }}
            </label>
          </div>

          <div class="grid sm:grid-cols-3 gap-3">
            <input v-model="form.plugin" class="auth-input" placeholder="Plugin (pgoutput, wal2json…)" />
            <input v-model="form.slot_name" class="auth-input" placeholder="Slot name" />
            <input v-model="form.origin" class="auth-input" placeholder="Origin name" />
          </div>

          <div class="flex flex-wrap gap-2 pt-2">
            <button type="button" class="btn-primary inline-flex items-center gap-2" :disabled="saving" @click="saveProfile">
              <Save class="h-4 w-4" />
              Save
            </button>
            <button type="button" class="btn-primary inline-flex items-center gap-2" :disabled="running || !selectedId" @click="runSelected">
              <Play class="h-4 w-4" />
              Run clone
            </button>
            <button v-if="selectedId" type="button" class="btn-secondary inline-flex items-center gap-2" @click="removeProfile">
              <Trash2 class="h-4 w-4" />
              Delete
            </button>
          </div>
        </div>

        <div class="card p-5 space-y-3">
          <h2 class="text-sm font-semibold">Jobs</h2>
          <div class="overflow-x-auto">
            <table class="data-table w-full text-sm">
              <thead>
                <tr class="text-left text-muted-foreground">
                  <th class="p-2">ID</th>
                  <th class="p-2">Status</th>
                  <th class="p-2">Actions</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="job in jobs" :key="job.id" class="border-t border-border">
                  <td class="p-2 font-mono text-xs">{{ job.id.slice(0, 8) }}…</td>
                  <td class="p-2">
                    <span
                      class="badge-neutral"
                      :class="{
                        'badge-success': job.status === 'succeeded',
                        'badge-error': job.status === 'failed',
                        'badge-warning': job.status === 'running',
                      }"
                    >{{ job.status }}</span>
                  </td>
                  <td class="p-2 flex gap-2">
                    <button type="button" class="text-xs text-primary underline" @click="showLogs(job.id)">Logs</button>
                    <button
                      v-if="job.status === 'running'"
                      type="button"
                      class="text-xs text-red-400 inline-flex items-center gap-1"
                      @click="stopRunningJob(job.id)"
                    >
                      <Square class="h-3 w-3" /> Stop
                    </button>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
          <pre v-if="logText" class="code-block text-xs max-h-80 overflow-auto whitespace-pre-wrap">{{ logText }}</pre>
        </div>
      </section>
    </div>
  </div>
</template>
