<script setup lang="ts">
import { ChevronDown, RefreshCw, Trash2 } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"

const route = useRoute()
const router = useRouter()
const toast = useToast()
const {
  fetchSchema,
  updateSchema,
  deleteSchema,
  checkSchemaCompatibility,
  setSchemaCompatibility,
  deleteSchemaVersion,
} = useKafkaAdmin()

const subject = computed(() => decodeURIComponent(String(route.params.subject || "")))
const loading = ref(false)
const error = ref("")
const detail = ref<Awaited<ReturnType<typeof fetchSchema>> | null>(null)

// Edit / publish new version
const editSchema = ref("")
const schemaType = ref("avro")
const publishing = ref(false)

// Version browser
const selectedVersion = ref<string>("")
const versionLoading = ref(false)
const versionSchema = ref("")

// Compat check
const compatCheckSchema = ref("")
const compatCheckType = ref("avro")
const compatChecking = ref(false)
const compatResult = ref<{ compatible: boolean; message: string } | null>(null)

// Compat set
const setCompatLevel = ref("")
const settingCompat = ref(false)

// Delete specific version
const pendingDeleteVersion = ref<string | null>(null)
const deletingVersion = ref(false)

// Delete subject
const pendingDelete = ref(false)

const tab = ref<"editor" | "versions" | "compat" | "settings">("editor")

const COMPAT_LEVELS = ["BACKWARD", "BACKWARD_TRANSITIVE", "FORWARD", "FORWARD_TRANSITIVE", "FULL", "FULL_TRANSITIVE", "NONE"]

const schemaTypeBadge = (type: string | undefined | null) => {
  const t = (type || "").toUpperCase()
  if (t === "AVRO") return "bg-blue-500/10 text-blue-400"
  if (t === "JSON") return "bg-emerald-500/10 text-emerald-400"
  if (t === "PROTOBUF") return "bg-purple-500/10 text-purple-400"
  return "badge-neutral"
}

const load = async () => {
  loading.value = true
  error.value = ""
  try {
    detail.value = await fetchSchema(subject.value)
    editSchema.value = detail.value.schema || ""
    schemaType.value = (detail.value.schema_type || "avro").toLowerCase()
    setCompatLevel.value = detail.value.compatibility || ""
    if (detail.value.versions?.length) {
      selectedVersion.value = String(detail.value.version || detail.value.versions[detail.value.versions.length - 1])
    }
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load schema")
  } finally {
    loading.value = false
  }
}

const publish = async () => {
  publishing.value = true
  try {
    await updateSchema(subject.value, { schema: editSchema.value, schema_type: schemaType.value })
    toast.success("Schema version published")
    await load()
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to update schema"))
  } finally {
    publishing.value = false
  }
}

const loadVersion = async () => {
  if (!selectedVersion.value) return
  versionLoading.value = true
  try {
    const v = await fetchSchema(subject.value, selectedVersion.value)
    versionSchema.value = v.schema || ""
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to load version"))
  } finally {
    versionLoading.value = false
  }
}

watch(selectedVersion, () => {
  if (tab.value === "versions") loadVersion()
})

const checkCompat = async () => {
  if (!compatCheckSchema.value.trim()) return
  compatChecking.value = true
  compatResult.value = null
  try {
    compatResult.value = await checkSchemaCompatibility(subject.value, {
      schema: compatCheckSchema.value,
      schema_type: compatCheckType.value,
    })
  } catch (e: any) {
    compatResult.value = { compatible: false, message: formatApiError(e, "Compatibility check failed") }
  } finally {
    compatChecking.value = false
  }
}

const setCompat = async () => {
  if (!setCompatLevel.value) return
  settingCompat.value = true
  try {
    await setSchemaCompatibility(subject.value, setCompatLevel.value)
    toast.success(`Compatibility set to ${setCompatLevel.value}`)
    await load()
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to set compatibility"))
  } finally {
    settingCompat.value = false
  }
}

const confirmDeleteVersion = async () => {
  if (!pendingDeleteVersion.value) return
  const v = pendingDeleteVersion.value
  pendingDeleteVersion.value = null
  deletingVersion.value = true
  try {
    await deleteSchemaVersion(subject.value, v)
    toast.success(`Deleted version ${v}`)
    await load()
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to delete version"))
  } finally {
    deletingVersion.value = false
  }
}

const remove = async () => {
  pendingDelete.value = false
  try {
    await deleteSchema(subject.value)
    toast.success("Subject deleted")
    await router.push("/schemas")
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to delete schema"))
  }
}

onMounted(load)
</script>

<template>
  <div class="page-container max-w-6xl">
    <AppPageHeader :title="subject" description="Schema versions, compatibility, and publisher.">
      <AppButton :disabled="loading" @click="load">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
      </AppButton>
      <AppButton as="link" to="/schemas">Subjects</AppButton>
      <AppButton variant="destructive" @click="pendingDelete = true">
        <Trash2 class="h-4 w-4" :stroke-width="1.75" />
        Delete Subject
      </AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <!-- Metrics -->
    <section class="grid grid-cols-2 gap-3 sm:grid-cols-4">
      <AppMetricCard label="ID" :value="detail?.id == null ? '—' : String(detail.id)" />
      <AppMetricCard label="Version" :value="detail?.version == null ? '—' : String(detail.version)" />
      <AppMetricCard label="Type">
        <template #value>
          <span
            v-if="detail?.schema_type"
            :class="['inline-flex items-center rounded-md px-2 py-0.5 text-sm font-semibold', schemaTypeBadge(detail.schema_type)]"
          >
            {{ detail.schema_type.toUpperCase() }}
          </span>
          <span v-else>—</span>
        </template>
      </AppMetricCard>
      <AppMetricCard label="Compatibility" :value="detail?.compatibility || 'DEFAULT'" />
    </section>

    <!-- Tabs -->
    <div class="flex flex-wrap gap-1 rounded-xl border border-surface-700 bg-surface-900/30 p-1">
      <button
        v-for="t in [
          { id: 'editor', label: 'Latest Schema' },
          { id: 'versions', label: `Versions (${detail?.versions?.length ?? 0})` },
          { id: 'compat', label: 'Check Compatibility' },
          { id: 'settings', label: 'Settings' },
        ]"
        :key="t.id"
        type="button"
        :class="[
          'rounded-lg px-4 py-2 text-sm font-medium transition-colors',
          tab === t.id ? 'bg-surface-800 text-surface-50 shadow-sm' : 'text-surface-400 hover:text-surface-200',
        ]"
        @click="tab = t.id as typeof tab; if (t.id === 'versions' && selectedVersion) loadVersion()"
      >
        {{ t.label }}
      </button>
    </div>

    <!-- Latest Schema / Editor tab -->
    <AppCard v-if="tab === 'editor'" class="space-y-4">
      <h2 class="section-title">Latest Schema</h2>
      <p class="text-sm text-surface-400">Publish a new version by editing the schema below and clicking "Publish".</p>
      <textarea
        v-model="editSchema"
        rows="20"
        class="auth-input font-mono text-sm"
        spellcheck="false"
        placeholder='{ "type": "record", ... }'
      />
      <div class="flex flex-wrap items-center gap-3">
        <label class="flex items-center gap-2">
          <span class="auth-label mb-0">Type</span>
          <AppSelect v-model="schemaType" select-class="w-36">
            <option value="avro">AVRO</option>
            <option value="json">JSON</option>
            <option value="protobuf">PROTOBUF</option>
          </AppSelect>
        </label>
        <AppButton variant="primary" :disabled="publishing" @click="publish">
          {{ publishing ? "Publishing…" : "Publish new version" }}
        </AppButton>
      </div>
    </AppCard>

    <!-- Versions tab -->
    <AppCard v-else-if="tab === 'versions'" class="space-y-4">
      <div class="flex items-center justify-between">
        <h2 class="section-title">Version History</h2>
        <span class="text-xs text-surface-400">{{ detail?.versions?.length ?? 0 }} version(s)</span>
      </div>
      <div v-if="detail?.versions?.length" class="flex flex-wrap gap-2">
        <button
          v-for="v in [...(detail?.versions ?? [])].reverse()"
          :key="v"
          type="button"
          :class="[
            'group/v flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-sm font-medium transition-all',
            String(v) === selectedVersion
              ? 'border-brand-500 bg-brand-500/10 text-brand-400'
              : 'border-surface-700 bg-surface-800/40 text-surface-300 hover:border-surface-500',
          ]"
          @click="selectedVersion = String(v); loadVersion()"
        >
          v{{ v }}
          <span
            v-if="v === detail?.version"
            class="rounded bg-brand-500/20 px-1 text-[10px] font-semibold text-brand-400"
          >latest</span>
          <button
            v-if="v !== detail?.version"
            type="button"
            class="invisible ml-0.5 rounded p-0.5 text-red-500 hover:bg-red-500/10 group-hover/v:visible"
            title="Delete this version"
            @click.stop="pendingDeleteVersion = String(v)"
          >
            <Trash2 class="h-3 w-3" :stroke-width="1.75" />
          </button>
        </button>
      </div>
      <AppStateBlock v-if="versionLoading" kind="loading" message="Loading version…" />
      <div v-else-if="versionSchema">
        <div class="mb-1 text-xs text-surface-400">Version {{ selectedVersion }}</div>
        <pre class="code-block max-h-[480px] overflow-auto whitespace-pre-wrap text-sm">{{ versionSchema }}</pre>
      </div>
      <AppStateBlock v-else kind="empty" message="Select a version to view its schema." />
    </AppCard>

    <!-- Check Compatibility tab -->
    <AppCard v-else-if="tab === 'compat'" class="space-y-4">
      <h2 class="section-title">Check Compatibility</h2>
      <p class="text-sm text-surface-400">
        Paste a candidate schema to check if it is compatible with the current subject's compatibility level
        (<strong>{{ detail?.compatibility || 'DEFAULT' }}</strong>).
      </p>
      <textarea
        v-model="compatCheckSchema"
        rows="14"
        class="auth-input font-mono text-sm"
        spellcheck="false"
        placeholder='{ "type": "record", ... }'
      />
      <div class="flex flex-wrap items-center gap-3">
        <label class="flex items-center gap-2">
          <span class="auth-label mb-0">Schema type</span>
          <AppSelect v-model="compatCheckType" select-class="w-36">
            <option value="avro">AVRO</option>
            <option value="json">JSON</option>
            <option value="protobuf">PROTOBUF</option>
          </AppSelect>
        </label>
        <AppButton variant="primary" :disabled="compatChecking || !compatCheckSchema.trim()" @click="checkCompat">
          {{ compatChecking ? "Checking…" : "Check Compatibility" }}
        </AppButton>
      </div>
      <div
        v-if="compatResult"
        :class="[
          'rounded-xl border px-4 py-3 text-sm',
          compatResult.compatible
            ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-400'
            : 'border-red-500/30 bg-red-500/10 text-red-400',
        ]"
      >
        <div class="font-semibold">{{ compatResult.compatible ? '✓ Compatible' : '✗ Incompatible' }}</div>
        <div v-if="compatResult.message" class="mt-1 font-mono text-xs opacity-80">{{ compatResult.message }}</div>
      </div>
    </AppCard>

    <!-- Settings tab -->
    <AppCard v-else-if="tab === 'settings'" class="space-y-5">
      <h2 class="section-title">Subject Settings</h2>
      <div>
        <div class="auth-label">Compatibility level</div>
        <p class="mb-3 text-xs text-surface-400">
          Controls how schema evolution is validated when new versions are published.
        </p>
        <div class="flex flex-wrap gap-2">
          <button
            v-for="level in COMPAT_LEVELS"
            :key="level"
            type="button"
            :class="[
              'rounded-lg border px-3 py-1.5 text-xs font-medium transition-all',
              setCompatLevel === level
                ? 'border-brand-500 bg-brand-500/10 text-brand-400'
                : 'border-surface-700 bg-surface-800/40 text-surface-300 hover:border-surface-500',
            ]"
            @click="setCompatLevel = level"
          >
            {{ level }}
          </button>
        </div>
        <AppButton
          variant="primary"
          class="mt-4"
          :disabled="settingCompat || !setCompatLevel"
          @click="setCompat"
        >
          {{ settingCompat ? "Saving…" : "Save Compatibility" }}
        </AppButton>
      </div>

      <hr class="border-surface-700/50" />

      <div>
        <div class="auth-label text-red-400">Danger zone</div>
        <p class="mb-3 text-sm text-surface-400">Delete the entire subject and all its schema versions permanently.</p>
        <AppButton variant="destructive" @click="pendingDelete = true">
          <Trash2 class="h-4 w-4" :stroke-width="1.75" />
          Delete Subject
        </AppButton>
      </div>
    </AppCard>

    <!-- Delete version confirm -->
    <AppConfirmDialog
      :open="Boolean(pendingDeleteVersion)"
      title="Delete this version?"
      danger
      confirm-label="Delete version"
      @update:open="(v) => { if (!v) pendingDeleteVersion = null }"
      @confirm="confirmDeleteVersion"
    >
      Delete version <strong>{{ pendingDeleteVersion }}</strong> of <span class="font-mono">{{ subject }}</span>?
    </AppConfirmDialog>

    <!-- Delete subject confirm -->
    <AppConfirmDialog
      :open="pendingDelete"
      title="Delete this subject?"
      danger
      confirm-label="Delete"
      @update:open="(v) => { if (!v) pendingDelete = false }"
      @confirm="remove"
    >
      This permanently removes all versions of <span class="font-mono font-semibold">{{ subject }}</span>.
    </AppConfirmDialog>
  </div>
</template>
