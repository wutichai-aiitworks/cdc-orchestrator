<script setup lang="ts">
import { ChevronDown, Download, Plus, RefreshCw, Search, Trash2, X } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import { downloadCsv } from "~/utils/csv"
import type { AclItem } from "~/composables/useKafkaAdmin"

const { fetchAcls, deleteAcl } = useKafkaAdmin()
const toast = useToast()

const search = ref("")
const host = ref("")
const filterResourceType = ref("")
const filterOperation = ref("")
const filterPermission = ref("")
const filterPatternType = ref("")
const loading = ref(false)
const error = ref("")
const configured = ref(true)
const items = ref<AclItem[]>([])
const pending = ref<AclItem | null>(null)

const RESOURCE_TYPES = [
  { value: "topic", label: "Topic" },
  { value: "group", label: "Group" },
  { value: "cluster", label: "Cluster" },
  { value: "transactional_id", label: "Transactional ID" },
  { value: "delegation_token", label: "Delegation Token" },
]

const OPERATIONS = [
  { value: "all", label: "ALL" },
  { value: "read", label: "READ" },
  { value: "write", label: "WRITE" },
  { value: "create", label: "CREATE" },
  { value: "delete", label: "DELETE" },
  { value: "alter", label: "ALTER" },
  { value: "describe", label: "DESCRIBE" },
  { value: "clusteraction", label: "CLUSTER_ACTION" },
  { value: "describe_configs", label: "DESCRIBE_CONFIGS" },
  { value: "alter_configs", label: "ALTER_CONFIGS" },
  { value: "idempotent_write", label: "IDEMPOTENT_WRITE" },
]

const PATTERN_TYPES = [
  { value: "literal", label: "LITERAL" },
  { value: "prefixed", label: "PREFIXED" },
  { value: "match", label: "MATCH" },
]

const activeFilters = computed(() =>
  [filterResourceType.value, filterOperation.value, filterPermission.value, filterPatternType.value]
    .filter(Boolean).length,
)

const load = async () => {
  loading.value = true
  error.value = ""
  try {
    const result = await fetchAcls({
      search: search.value.trim() || undefined,
      host: host.value.trim() || undefined,
      resource_type: filterResourceType.value || undefined,
      operation: filterOperation.value || undefined,
      permission: filterPermission.value || undefined,
      pattern_type: filterPatternType.value || undefined,
    })
    items.value = result.items || []
    configured.value = result.configured
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load ACLs")
  } finally {
    loading.value = false
  }
}

const clearFilters = () => {
  search.value = ""
  host.value = ""
  filterResourceType.value = ""
  filterOperation.value = ""
  filterPermission.value = ""
  filterPatternType.value = ""
  load()
}

const exportCsv = () => {
  downloadCsv("acls", items.value as unknown as Array<Record<string, unknown>>)
}

const confirmDelete = async () => {
  if (!pending.value) return
  const row = pending.value
  pending.value = null
  try {
    await deleteAcl({
      principal: row.principal,
      host: row.host,
      resource_type: row.resource_type.toLowerCase(),
      resource_name: row.resource_name,
      pattern_type: row.pattern_type.toLowerCase(),
      operation: row.operation.toLowerCase(),
      permission: row.permission.toLowerCase(),
    })
    toast.success("ACL deleted")
    await load()
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to delete ACL"))
  }
}

onMounted(load)
</script>

<template>
  <div class="page-container max-w-7xl">
    <AppPageHeader title="ACL" description="View and manage Kafka access control lists.">
      <AppButton :disabled="loading" @click="load">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
      <AppButton :disabled="!items.length" @click="exportCsv">
        <Download class="h-4 w-4" :stroke-width="1.75" />
        Export CSV
      </AppButton>
      <AppButton as="link" to="/acl/create" variant="primary">
        <Plus class="h-4 w-4" :stroke-width="1.75" />
        Create ACL
      </AppButton>
    </AppPageHeader>

    <!-- Filter bar -->
    <AppCard>
      <div class="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
        <!-- Search -->
        <label class="xl:col-span-2">
          <span class="auth-label">Search</span>
          <div class="relative">
            <Search class="pointer-events-none absolute left-3 top-3 h-4 w-4 text-surface-400" :stroke-width="1.75" />
            <AppInput v-model="search" input-class="pl-9" placeholder="Principal, resource, or host" @enter="load" />
          </div>
        </label>

        <!-- Resource type filter -->
        <label>
          <span class="auth-label">Resource type</span>
          <AppSelect v-model="filterResourceType">
            <option value="">All types</option>
            <option v-for="rt in RESOURCE_TYPES" :key="rt.value" :value="rt.value">{{ rt.label }}</option>
          </AppSelect>
        </label>

        <!-- Operation filter -->
        <label>
          <span class="auth-label">Operation</span>
          <AppSelect v-model="filterOperation">
            <option value="">All operations</option>
            <option v-for="op in OPERATIONS" :key="op.value" :value="op.value">{{ op.label }}</option>
          </AppSelect>
        </label>

        <!-- Permission filter -->
        <label>
          <span class="auth-label">Permission</span>
          <AppSelect v-model="filterPermission">
            <option value="">All</option>
            <option value="allow">ALLOW</option>
            <option value="deny">DENY</option>
          </AppSelect>
        </label>

        <!-- Pattern type filter -->
        <label>
          <span class="auth-label">Pattern type</span>
          <AppSelect v-model="filterPatternType">
            <option value="">All patterns</option>
            <option v-for="pt in PATTERN_TYPES" :key="pt.value" :value="pt.value">{{ pt.label }}</option>
          </AppSelect>
        </label>

        <!-- Host filter -->
        <label>
          <span class="auth-label">Host</span>
          <AppInput v-model="host" placeholder="* or specific host" @enter="load" />
        </label>

        <!-- Actions -->
        <div class="flex items-end gap-2">
          <AppButton variant="primary" class="flex-1" :disabled="loading" @click="load">Apply</AppButton>
          <AppButton v-if="activeFilters > 0 || search || host" variant="ghost" @click="clearFilters">
            <X class="h-4 w-4" :stroke-width="1.75" />
          </AppButton>
        </div>
      </div>

      <!-- Active filter chips -->
      <div v-if="activeFilters > 0" class="mt-3 flex flex-wrap gap-2">
        <span v-if="filterResourceType" class="inline-flex items-center gap-1 rounded-lg bg-brand-500/10 px-2 py-0.5 text-xs text-brand-400">
          Type: {{ filterResourceType }}
          <button type="button" class="hover:text-brand-300" @click="filterResourceType = ''; load()">
            <X class="h-3 w-3" />
          </button>
        </span>
        <span v-if="filterOperation" class="inline-flex items-center gap-1 rounded-lg bg-brand-500/10 px-2 py-0.5 text-xs text-brand-400">
          Op: {{ filterOperation.toUpperCase() }}
          <button type="button" class="hover:text-brand-300" @click="filterOperation = ''; load()">
            <X class="h-3 w-3" />
          </button>
        </span>
        <span v-if="filterPermission" class="inline-flex items-center gap-1 rounded-lg bg-brand-500/10 px-2 py-0.5 text-xs text-brand-400">
          Permission: {{ filterPermission.toUpperCase() }}
          <button type="button" class="hover:text-brand-300" @click="filterPermission = ''; load()">
            <X class="h-3 w-3" />
          </button>
        </span>
        <span v-if="filterPatternType" class="inline-flex items-center gap-1 rounded-lg bg-brand-500/10 px-2 py-0.5 text-xs text-brand-400">
          Pattern: {{ filterPatternType.toUpperCase() }}
          <button type="button" class="hover:text-brand-300" @click="filterPatternType = ''; load()">
            <X class="h-3 w-3" />
          </button>
        </span>
      </div>
    </AppCard>

    <AppStateBlock v-if="error" kind="error" :message="error" />
    <AppStateBlock v-else-if="!configured" kind="empty" message="Kafka ACLs are not enabled on this cluster." />

    <AppCard>
      <div class="mb-3 flex items-center justify-between">
        <span class="text-xs text-surface-400">{{ items.length }} ACL{{ items.length !== 1 ? 's' : '' }}</span>
      </div>
      <AppTable>
        <thead>
          <tr>
            <th>Principal</th>
            <th>Host</th>
            <th>Resource type</th>
            <th>Resource</th>
            <th>Pattern</th>
            <th>Operation</th>
            <th>Permission</th>
            <th class="w-12" />
          </tr>
        </thead>
        <tbody>
          <tr v-for="(row, idx) in items" :key="`${row.principal}-${idx}`">
            <td class="font-mono text-sm">{{ row.principal }}</td>
            <td>{{ row.host }}</td>
            <td>
              <span class="badge-neutral">{{ row.resource_type }}</span>
            </td>
            <td class="font-mono text-sm">{{ row.resource_name }}</td>
            <td class="text-surface-300">{{ row.pattern_type }}</td>
            <td class="text-surface-300">{{ row.operation }}</td>
            <td>
              <span :class="row.permission.toUpperCase() === 'ALLOW' ? 'badge-success' : 'badge-error'">
                {{ row.permission }}
              </span>
            </td>
            <td>
              <button type="button" class="icon-button text-red-500" title="Delete ACL" @click="pending = row">
                <Trash2 class="h-4 w-4" :stroke-width="1.75" />
              </button>
            </td>
          </tr>
          <tr v-if="!loading && !items.length">
            <td colspan="8"><AppStateBlock kind="empty" message="No ACLs found." /></td>
          </tr>
          <tr v-if="loading">
            <td colspan="8" class="py-6 text-center text-sm text-surface-400">Loading…</td>
          </tr>
        </tbody>
      </AppTable>
    </AppCard>

    <AppConfirmDialog
      :open="Boolean(pending)"
      title="Delete this ACL?"
      danger
      confirm-label="Delete"
      @update:open="(value) => { if (!value) pending = null }"
      @confirm="confirmDelete"
    >
      <div class="space-y-1 text-sm">
        <div><span class="text-surface-400">Principal:</span> <span class="font-mono">{{ pending?.principal }}</span></div>
        <div><span class="text-surface-400">Operation:</span> {{ pending?.operation }}</div>
        <div><span class="text-surface-400">Resource:</span> <span class="font-mono">{{ pending?.resource_name }}</span></div>
        <div><span class="text-surface-400">Permission:</span> {{ pending?.permission }}</div>
      </div>
    </AppConfirmDialog>
  </div>
</template>
