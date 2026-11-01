<script setup lang="ts">
import { Download, Pencil, Plus, RefreshCw, Search, Trash2, X } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import { downloadCsv } from "~/utils/csv"
import type { SchemaSubjectItem } from "~/composables/useKafkaAdmin"

const { fetchSchemas, deleteSchema } = useKafkaAdmin()
const toast = useToast()

const search = ref("")
const typeFilter = ref("")
const loading = ref(false)
const error = ref("")
const configured = ref(true)
const items = ref<SchemaSubjectItem[]>([])
const pendingDelete = ref<SchemaSubjectItem | null>(null)

const SCHEMA_TYPES = ["AVRO", "JSON", "PROTOBUF"]

const filtered = computed(() => {
  let list = items.value
  const q = search.value.trim().toLowerCase()
  if (q) list = list.filter((item) => item.subject.toLowerCase().includes(q))
  if (typeFilter.value) list = list.filter((item) => (item.schema_type || "").toUpperCase() === typeFilter.value)
  return list
})

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
    const result = await fetchSchemas()
    items.value = result.items || []
    configured.value = result.configured
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load schemas")
  } finally {
    loading.value = false
  }
}

const confirmDelete = async () => {
  if (!pendingDelete.value) return
  const row = pendingDelete.value
  pendingDelete.value = null
  try {
    await deleteSchema(row.subject)
    toast.success(`Deleted subject "${row.subject}"`)
    await load()
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to delete schema"))
  }
}

onMounted(load)
</script>

<template>
  <div class="page-container max-w-7xl">
    <AppPageHeader
      title="Schema Registry"
      description="CDC pipelines register Avro schemas here by default. You can also manage JSON Schema and Protobuf subjects."
    >
      <AppButton :disabled="loading" @click="load">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
      <AppButton :disabled="!filtered.length" @click="downloadCsv('schemas', filtered as unknown as Array<Record<string, unknown>>)">
        <Download class="h-4 w-4" :stroke-width="1.75" />
        Export CSV
      </AppButton>
      <AppButton as="link" to="/schemas/create" variant="primary">
        <Plus class="h-4 w-4" :stroke-width="1.75" />
        Create Schema
      </AppButton>
    </AppPageHeader>

    <!-- Metrics -->
    <section class="grid grid-cols-2 gap-3 sm:grid-cols-4">
      <AppMetricCard label="Total subjects" :value="String(items.length)" tone="info">
        <RefreshCw class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="AVRO"
        :value="String(items.filter(i => (i.schema_type || '').toUpperCase() === 'AVRO').length)"
        tone="brand"
      />
      <AppMetricCard
        label="JSON"
        :value="String(items.filter(i => (i.schema_type || '').toUpperCase() === 'JSON').length)"
        tone="success"
      />
      <AppMetricCard
        label="PROTOBUF"
        :value="String(items.filter(i => (i.schema_type || '').toUpperCase() === 'PROTOBUF').length)"
      />
    </section>

    <!-- Filters -->
    <AppCard>
      <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <label class="sm:col-span-2">
          <span class="auth-label">Search subjects</span>
          <div class="relative">
            <Search class="pointer-events-none absolute left-3 top-3 h-4 w-4 text-surface-400" :stroke-width="1.75" />
            <AppInput v-model="search" input-class="pl-9" placeholder="Subject name" />
          </div>
        </label>
        <label>
          <span class="auth-label">Schema type</span>
          <AppSelect v-model="typeFilter">
            <option value="">All types</option>
            <option v-for="t in SCHEMA_TYPES" :key="t" :value="t">{{ t }}</option>
          </AppSelect>
        </label>
      </div>
    </AppCard>

    <AppStateBlock v-if="error" kind="error" :message="error" />
    <AppStateBlock
      v-else-if="!configured"
      kind="empty"
      message="Schema Registry is not available. Enable Redpanda schema registry or set ORCH_SCHEMA_REGISTRY_URL."
    />

    <AppCard>
      <div class="mb-3 text-xs text-surface-400">{{ filtered.length }} subject{{ filtered.length !== 1 ? 's' : '' }}</div>
      <AppTable>
        <thead>
          <tr>
            <th>Subject</th>
            <th>ID</th>
            <th>Version</th>
            <th>Type</th>
            <th>Compatibility</th>
            <th class="w-24" />
          </tr>
        </thead>
        <tbody>
          <tr v-for="item in filtered" :key="item.subject" class="group">
            <td>
              <NuxtLink :to="`/schemas/${encodeURIComponent(item.subject)}`" class="font-semibold hover:text-brand-500">
                {{ item.subject }}
              </NuxtLink>
            </td>
            <td class="font-mono text-sm">{{ item.id ?? "—" }}</td>
            <td class="font-mono text-sm">{{ item.version ?? "—" }}</td>
            <td>
              <span
                v-if="item.schema_type"
                :class="['inline-flex items-center rounded-md px-2 py-0.5 text-xs font-medium', schemaTypeBadge(item.schema_type)]"
              >
                {{ item.schema_type.toUpperCase() }}
              </span>
              <span v-else class="text-surface-400">—</span>
            </td>
            <td>
              <span v-if="item.compatibility" class="badge-neutral">{{ item.compatibility }}</span>
              <span v-else class="text-surface-400">—</span>
            </td>
            <td>
              <div class="flex items-center justify-end gap-1">
                <NuxtLink
                  :to="`/schemas/${encodeURIComponent(item.subject)}`"
                  class="invisible icon-button group-hover:visible"
                  title="Edit / view schema"
                >
                  <Pencil class="h-4 w-4" :stroke-width="1.75" />
                </NuxtLink>
                <button
                  type="button"
                  title="Delete subject"
                  class="invisible icon-button text-red-500 group-hover:visible"
                  @click="pendingDelete = item"
                >
                  <Trash2 class="h-4 w-4" :stroke-width="1.75" />
                </button>
              </div>
            </td>
          </tr>
          <tr v-if="!loading && !filtered.length">
            <td colspan="6"><AppStateBlock kind="empty" message="No schema subjects found." /></td>
          </tr>
          <tr v-if="loading">
            <td colspan="6" class="py-6 text-center text-sm text-surface-400">Loading…</td>
          </tr>
        </tbody>
      </AppTable>
    </AppCard>

    <AppConfirmDialog
      :open="Boolean(pendingDelete)"
      title="Delete this subject?"
      danger
      confirm-label="Delete"
      @update:open="(v) => { if (!v) pendingDelete = null }"
      @confirm="confirmDelete"
    >
      <div class="text-sm">
        Delete subject <span class="font-mono font-semibold">{{ pendingDelete?.subject }}</span>?
        This permanently removes all versions.
      </div>
    </AppConfirmDialog>
  </div>
</template>
