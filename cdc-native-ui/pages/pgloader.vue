<script setup lang="ts">
import { Database, Play, RefreshCw, Save, Square, Trash2 } from "@lucide/vue"
import {
  defaultPgLoaderOptions,
  PGLOADER_SOURCE_TYPES,
  type PgLoaderConn,
  type PgLoaderJob,
  type PgLoaderProfile,
  usePgLoader,
} from "~/composables/usePgLoader"
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
} = usePgLoader()

const emptySource = (): PgLoaderConn => ({
  type: "mysql",
  host: "",
  port: "3306",
  database: "",
  username: "",
  password: "",
})

const emptyTarget = (): PgLoaderConn => ({
  type: "postgres",
  host: "",
  port: "5432",
  database: "",
  username: "",
  password: "",
})

const profiles = ref<PgLoaderProfile[]>([])
const jobs = ref<PgLoaderJob[]>([])
const selectedId = ref("")
const loading = ref(false)
const saving = ref(false)
const running = ref(false)
const sidecarVersion = ref("")
const includeTablesText = ref("")
const logText = ref("")
const logJobId = ref("")
const error = ref("")
const contractDefaults = ref<Record<string, unknown> | null>(null)

const form = reactive({
  name: "",
  source: emptySource(),
  target: emptyTarget(),
  options: defaultPgLoaderOptions(),
})

const isFileSource = computed(() => ["sqlite", "csv"].includes(form.source.type))

const onSourceType = () => {
  const meta = PGLOADER_SOURCE_TYPES.find((item) => item.value === form.source.type)
  form.source.port = meta?.port || ""
}

const loadAll = async () => {
  loading.value = true
  error.value = ""
  try {
    const [cap, profs, jobList] = await Promise.all([
      fetchCapability().catch(() => ({})),
      fetchProfiles(),
      fetchJobs(),
    ])
    sidecarVersion.value = String((cap as any)?.pgloader_version || "")
    profiles.value = profs
    jobs.value = jobList
    if (!selectedId.value && profs.length) selectProfile(profs[0].id)
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load pgloader")
  } finally {
    loading.value = false
  }
}

const selectProfile = (id: string) => {
  selectedId.value = id
  const p = profiles.value.find((item) => item.id === id)
  if (!p) return
  form.name = p.name
  form.source = { ...emptySource(), ...p.source }
  form.target = { ...emptyTarget(), ...p.target }
  form.options = { ...defaultPgLoaderOptions(), ...p.options }
  includeTablesText.value = (p.include_tables || []).join("\n")
}

const payloadFromForm = () => ({
  name: form.name.trim(),
  source: form.source,
  target: form.target,
  include_tables: includeTablesText.value.split("\n").map((line) => line.trim()).filter(Boolean),
  options: form.options,
})

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

const newProfile = () => {
  selectedId.value = ""
  form.name = ""
  form.source = emptySource()
  form.target = emptyTarget()
  form.options = applyContractOptionDefaults(defaultPgLoaderOptions(), contractDefaults.value ?? undefined)
  includeTablesText.value = ""
}

const removeProfile = async () => {
  if (!selectedId.value || !confirm("Delete this pgloader profile?")) return
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
    toast.success(`Load job started (${job.id})`)
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
    logText.value = (await fetchJobLogs(jobId)).log || ""
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
  try {
    const res = await validateConnection(which === "source" ? form.source : form.target)
    toast[res.valid ? "success" : "error"](res.message)
  } catch (e: any) {
    toast.error(formatApiError(e, "Request failed"))
  }
}

let pollTimer: ReturnType<typeof setInterval> | null = null
onMounted(() => {
  fetchContract().then((c) => { contractDefaults.value = c }).catch(() => {})
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
          <Database class="h-5 w-5 text-primary" />
          Load to Postgres (pgloader)
        </h1>
        <div class="mt-1 flex items-center gap-1.5">
          <span class="text-sm font-medium text-foreground">MySQL / SQL Server / SQLite / CSV → Postgres</span>
          <AppInfoHint
            :text="`Configure a load into PostgreSQL without running pgloader yourself.${sidecarVersion ? ` Sidecar ${sidecarVersion}.` : ''}`"
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
          <div class="text-xs text-muted-foreground truncate">{{ p.source.type }} → Postgres</div>
        </button>
        <p v-if="!profiles.length" class="text-sm text-muted-foreground italic">No profiles yet.</p>
      </aside>

      <section class="xl:col-span-9 space-y-4">
        <div class="card p-5 space-y-4">
          <input v-model="form.name" type="text" placeholder="Profile name" class="auth-input w-full" />

          <div class="grid md:grid-cols-2 gap-4">
            <div class="space-y-2 rounded-xl border border-border p-4">
              <div class="flex items-center justify-between">
                <h3 class="text-sm font-semibold">Source</h3>
                <button type="button" class="text-xs text-primary underline" @click="testConn('source')">Validate</button>
              </div>
              <select v-model="form.source.type" class="auth-input w-full" @change="onSourceType">
                <option v-for="item in PGLOADER_SOURCE_TYPES" :key="item.value" :value="item.value">{{ item.label }}</option>
              </select>
              <input
                v-if="isFileSource"
                v-model="form.source.database"
                class="auth-input w-full"
                :placeholder="form.source.type === 'csv' ? 'CSV path on sidecar (/data/file.csv)' : 'SQLite path on sidecar'"
              />
              <template v-else>
                <input v-model="form.source.host" class="auth-input w-full" placeholder="Host" />
                <div class="grid grid-cols-2 gap-2">
                  <input v-model="form.source.port" class="auth-input" placeholder="Port" />
                  <input v-model="form.source.database" class="auth-input" placeholder="Database" />
                </div>
                <input v-model="form.source.username" class="auth-input w-full" placeholder="Username" />
                <input v-model="form.source.password" type="password" class="auth-input w-full" placeholder="Password" />
              </template>
            </div>
            <div class="space-y-2 rounded-xl border border-border p-4">
              <div class="flex items-center justify-between">
                <h3 class="text-sm font-semibold">Target Postgres</h3>
                <button type="button" class="text-xs text-primary underline" @click="testConn('target')">Validate</button>
              </div>
              <input v-model="form.target.host" class="auth-input w-full" placeholder="Host" />
              <div class="grid grid-cols-2 gap-2">
                <input v-model="form.target.port" class="auth-input" placeholder="Port" />
                <input v-model="form.target.database" class="auth-input" placeholder="Database" />
              </div>
              <input v-model="form.target.username" class="auth-input w-full" placeholder="Username" />
              <input v-model="form.target.password" type="password" class="auth-input w-full" placeholder="Password" />
            </div>
          </div>

          <div v-if="form.source.type !== 'csv'">
            <label class="auth-label">Include tables (optional, one per line)</label>
            <textarea v-model="includeTablesText" rows="3" class="auth-input w-full font-mono text-xs" />
          </div>
          <div v-else class="grid sm:grid-cols-3 gap-3">
            <label class="text-xs text-muted-foreground">Target table
              <input v-model="form.options.target_table" class="auth-input w-full mt-1" placeholder="public.imported" />
            </label>
            <label class="text-xs text-muted-foreground">Delimiter
              <input v-model="form.options.csv_delimiter" class="auth-input w-full mt-1" />
            </label>
            <label class="text-xs text-muted-foreground">Skip header rows
              <input v-model.number="form.options.csv_skip_header" type="number" min="0" class="auth-input w-full mt-1" />
            </label>
          </div>

          <div class="grid sm:grid-cols-2 lg:grid-cols-3 gap-2">
            <label class="flex items-center gap-2 text-sm"><input v-model="form.options.drop_if_exists" type="checkbox" class="rounded border-border" /> Drop existing</label>
            <label class="flex items-center gap-2 text-sm"><input v-model="form.options.create_tables" type="checkbox" class="rounded border-border" /> Create tables</label>
            <label class="flex items-center gap-2 text-sm"><input v-model="form.options.create_indexes" type="checkbox" class="rounded border-border" /> Create indexes</label>
            <label class="flex items-center gap-2 text-sm"><input v-model="form.options.reset_sequences" type="checkbox" class="rounded border-border" /> Reset sequences</label>
            <label class="flex items-center gap-2 text-sm"><input v-model="form.options.truncate" type="checkbox" class="rounded border-border" /> Truncate</label>
          </div>

          <div class="flex flex-wrap gap-2 pt-2">
            <button type="button" class="btn-primary inline-flex items-center gap-2" :disabled="saving" @click="saveProfile">
              <Save class="h-4 w-4" /> Save
            </button>
            <button type="button" class="btn-primary inline-flex items-center gap-2" :disabled="running || !selectedId" @click="runSelected">
              <Play class="h-4 w-4" /> Run load
            </button>
            <button v-if="selectedId" type="button" class="btn-secondary inline-flex items-center gap-2" @click="removeProfile">
              <Trash2 class="h-4 w-4" /> Delete
            </button>
          </div>
        </div>

        <div class="card p-5 space-y-3">
          <h2 class="text-sm font-semibold">Jobs</h2>
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
                  <span class="badge-neutral" :class="{ 'badge-success': job.status === 'succeeded', 'badge-error': job.status === 'failed', 'badge-warning': job.status === 'running' }">{{ job.status }}</span>
                </td>
                <td class="p-2 flex gap-2">
                  <button type="button" class="text-xs text-primary underline" @click="showLogs(job.id)">Logs</button>
                  <button v-if="job.status === 'running'" type="button" class="text-xs text-red-400 inline-flex items-center gap-1" @click="stopRunningJob(job.id)">
                    <Square class="h-3 w-3" /> Stop
                  </button>
                </td>
              </tr>
            </tbody>
          </table>
          <pre v-if="logText" class="code-block text-xs max-h-80 overflow-auto whitespace-pre-wrap">{{ logText }}</pre>
        </div>
      </section>
    </div>
  </div>
</template>
