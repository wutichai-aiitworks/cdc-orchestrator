<script setup lang="ts">
import {
  Braces,
  Copy,
  Database,
  Download,
  Eye,
  FolderTree,
  List,
  Pause,
  Play,
  Plug,
  Plus,
  RefreshCcwDot,
  RefreshCw,
  RotateCw,
  Square,
  Tag,
  Trash2,
  X,
} from "@lucide/vue"
import ConnectorCreateDialog from "~/components/connectors/ConnectorCreateDialog.vue"
import DatabaseTypeMark from "~/components/connectors/DatabaseTypeMark.vue"
import type { ConnectorTemplate } from "~/composables/useConnectorsM2"
import { insightFromConfig, stringifyConfig, type ConnectorKind } from "~/utils/connectorMeta"

const route = useRoute()
const router = useRouter()
const { getPref, setPref } = useViewPrefs()
const toast = useToast()
const {
  fetchConnectors,
  fetchTemplates,
  validateConnector,
  updateConnectorConfig,
  runAction,
  restartTask,
  fetchConnectorStatus,
  fetchConnectorTimeline,
  fetchConnectorConfig,
  probeConnector,
  catalogConnector,
  removeConnector,
  normalizeApiError,
} = useConnectorsM2()

const loading = ref(false)
const templatesLoading = ref(false)
const templatesError = ref("")
const actionLoading = ref(false)
const error = ref("")
const lastUpdated = ref("")
const lastAuditId = ref("")
const initialized = ref(false)
const showCreate = ref(false)

const connectors = ref<Record<string, any>>({})
const templates = ref<ConnectorTemplate[]>([])

const selectedName = ref("")
const updateConfigText = ref("{}")
const updateValidateResult = ref<any>(null)
const updateValidateHash = ref("")
const deleteConfirmText = ref("")
const showDeleteConfirm = ref(false)
const traceView = ref<{ taskId: number; trace: string } | null>(null)
const timelineLoading = ref(false)
const timelineError = ref("")
const timelineEntries = ref<
  Array<{
    ts: string
    connector: string
    from?: string | null
    to: string
    rule?: string | null
    task_errors: Array<{ taskId: number; trace: string }>
    resolved?: boolean | null
  }>
>([])
const listSearch = ref("")
const stateFilter = ref("all")
const kindFilter = ref<"all" | ConnectorKind>("all")
const failedOnly = ref(false)
const sortMode = ref<"name_asc" | "name_desc" | "state">("name_asc")
const detailTab = ref<"overview" | "json" | "catalog" | "tasks" | "timeline" | "topics">("overview")
const connectorTopics = ref<string[]>([])
const topicsLoading = ref(false)
const topicsError = ref("")
const pluginsList = ref<any[]>([])
const pluginsLoading = ref(false)
const pluginsError = ref("")
const revealSecrets = ref(false)
const probeLoading = ref(false)
const probeResult = ref<any>(null)
const catalogLoading = ref(false)
const catalogResult = ref<any>(null)
const selectedSchema = ref("")
const catalogSearch = ref("")
const catalogPage = ref(1)
const catalogPageSize = ref("20")

const connectorNames = computed(() => Object.keys(connectors.value).sort())
const selected = computed(() => (selectedName.value ? connectors.value[selectedName.value] : null))
const selectedState = computed(() => selected.value?.status?.connector?.state || "UNKNOWN")
const selectedConfig = computed<Record<string, string>>(
  () => selected.value?.info?.config || {}
)
const selectedInsight = computed(() =>
  insightFromConfig(selectedConfig.value, selected.value?.info?.type || selected.value?.status?.type)
)
const connectionSubtitle = computed(() => {
  const c = selectedInsight.value.connection
  if (!c.host) return "Connection details are in the JSON tab"
  return `${c.host}:${c.port || "—"}${c.database ? `/${c.database}` : ""}`
})

const parseConfigSafe = computed<Record<string, string> | null>(() => {
  try {
    return parseJsonObject(updateConfigText.value)
  } catch {
    return null
  }
})

const configDiff = computed<
  Array<{ key: string; current: string; next: string; changed: boolean }>
>(() => {
  const before = selectedConfig.value || {}
  const after = parseConfigSafe.value || {}
  const keys = Array.from(new Set([...Object.keys(before), ...Object.keys(after)])).sort()
  return keys.map((key) => {
    const current = before[key] ?? ""
    const next = after[key] ?? ""
    return { key, current, next, changed: current !== next }
  })
})
const changedCount = computed(() => configDiff.value.filter((d) => d.changed).length)
const canApplyUpdate = computed(() => {
  if (!selectedName.value) return false
  if (!parseConfigSafe.value) return false
  if (!updateValidateResult.value) return false
  if (updateValidateHash.value !== updateConfigText.value) return false
  return updateValidateResult.value.valid === true
})

const insightFor = (name: string) => {
  const connector = connectors.value[name]
  return insightFromConfig(
    connector?.info?.config || {},
    connector?.info?.type || connector?.status?.type
  )
}

const filteredConnectorNames = computed(() => {
  const search = listSearch.value.trim().toLowerCase()
  const arr = connectorNames.value.filter((name) => {
    const c = connectors.value[name]
    const state = String(c?.status?.connector?.state || "UNKNOWN").toUpperCase()
    const insight = insightFor(name)
    if (search && !name.toLowerCase().includes(search) && !insight.databaseLabel.toLowerCase().includes(search)) {
      return false
    }
    if (kindFilter.value !== "all" && insight.kind !== kindFilter.value) return false
    if (stateFilter.value !== "all" && state !== stateFilter.value.toUpperCase()) return false
    if (failedOnly.value) {
      const failedTask = (c?.status?.tasks || []).some((t: any) =>
        String(t?.state || "").toUpperCase().includes("FAILED")
      )
      if (!failedTask && state !== "FAILED") return false
    }
    return true
  })
  if (sortMode.value === "name_desc") return arr.sort((a, b) => b.localeCompare(a))
  if (sortMode.value === "state") {
    return arr.sort((a, b) => {
      const sa = String(connectors.value[a]?.status?.connector?.state || "UNKNOWN")
      const sb = String(connectors.value[b]?.status?.connector?.state || "UNKNOWN")
      if (sa === sb) return a.localeCompare(b)
      return sa.localeCompare(sb)
    })
  }
  return arr.sort((a, b) => a.localeCompare(b))
})

const groupedNames = computed(() => ({
  source: filteredConnectorNames.value.filter((name) => insightFor(name).kind === "source"),
  sink: filteredConnectorNames.value.filter((name) => insightFor(name).kind === "sink"),
}))

const summary = computed(() => {
  const names = connectorNames.value
  return {
    total: names.length,
    source: names.filter((name) => insightFor(name).kind === "source").length,
    sink: names.filter((name) => insightFor(name).kind === "sink").length,
    running: names.filter((name) => String(connectors.value[name]?.status?.connector?.state || "").toUpperCase() === "RUNNING").length,
    failed: names.filter((name) => {
      const state = String(connectors.value[name]?.status?.connector?.state || "").toUpperCase()
      const failedTask = (connectors.value[name]?.status?.tasks || []).some((t: any) =>
        String(t?.state || "").toUpperCase().includes("FAILED")
      )
      return state === "FAILED" || failedTask
    }).length,
  }
})

const filteredCatalogTables = computed(() => {
  const tables = (catalogResult.value?.tables || []) as Array<{
    schema: string
    name: string
    captured: boolean
    row_count?: number | null
  }>
  const schema = selectedSchema.value.trim().toLowerCase()
  const search = catalogSearch.value.trim().toLowerCase()
  return tables.filter((table) => {
    if (schema && table.schema.toLowerCase() !== schema) return false
    if (search && !`${table.schema}.${table.name}`.toLowerCase().includes(search)) return false
    return true
  })
})
const catalogPageSizeNumber = computed(() => Number(catalogPageSize.value) || 20)
const catalogTotalPages = computed(() =>
  Math.max(1, Math.ceil(filteredCatalogTables.value.length / catalogPageSizeNumber.value) || 1)
)
const pagedCatalogTables = computed(() => {
  const start = (catalogPage.value - 1) * catalogPageSizeNumber.value
  return filteredCatalogTables.value.slice(start, start + catalogPageSizeNumber.value)
})

const syncQuery = async () => {
  const q: Record<string, string> = {}
  if (selectedName.value) q.name = selectedName.value
  if (listSearch.value.trim()) q.search = listSearch.value.trim()
  if (stateFilter.value !== "all") q.state = stateFilter.value
  if (kindFilter.value !== "all") q.kind = kindFilter.value
  if (failedOnly.value) q.failed_only = "true"
  if (sortMode.value !== "name_asc") q.sort = sortMode.value
  if (detailTab.value !== "overview") q.tab = detailTab.value
  await router.replace({ query: q })
}

const parseJsonObject = (text: string) => {
  const v = JSON.parse(text)
  if (!v || typeof v !== "object" || Array.isArray(v)) {
    throw new Error("JSON must be an object")
  }
  return Object.fromEntries(
    Object.entries(v as Record<string, unknown>).map(([k, val]) => [k, String(val)])
  )
}

const applyConfigToEditor = (config: Record<string, string>) => {
  updateConfigText.value = stringifyConfig(config, true)
}

const loadSelectedConfig = async (name = selectedName.value) => {
  if (!name) return
  const existing = connectors.value[name]?.info?.config
  if (existing && Object.keys(existing).length > 0) {
    applyConfigToEditor(existing)
    return
  }
  try {
    const res = await fetchConnectorConfig(name)
    const config = res.config || {}
    connectors.value[name] = {
      ...(connectors.value[name] || {}),
      info: { ...(connectors.value[name]?.info || {}), name, config },
    }
    applyConfigToEditor(config)
  } catch (e: any) {
    error.value = normalizeApiError(e, "Failed to load connector config")
  }
}

const loadConnectors = async () => {
  loading.value = true
  error.value = ""
  try {
    const res = await fetchConnectors()
    connectors.value = res.connectors || {}
    if (!selectedName.value && connectorNames.value.length > 0) {
      selectedName.value = connectorNames.value[0]
    }
    await loadSelectedConfig()
    lastUpdated.value = new Date().toLocaleTimeString()
  } catch (e: any) {
    error.value = normalizeApiError(e, "Failed to load connectors")
  } finally {
    loading.value = false
  }
}

const loadTimeline = async (name = selectedName.value) => {
  if (!name) {
    timelineEntries.value = []
    timelineError.value = ""
    return
  }
  timelineLoading.value = true
  timelineError.value = ""
  try {
    const res = await fetchConnectorTimeline(name, 50)
    timelineEntries.value = res.items || []
  } catch (e: any) {
    timelineEntries.value = []
    timelineError.value = normalizeApiError(e, "Failed to load connector timeline")
  } finally {
    timelineLoading.value = false
  }
}

const loadTemplates = async () => {
  templatesLoading.value = true
  templatesError.value = ""
  try {
    const res = await fetchTemplates()
    templates.value = res.templates || []
  } catch (e: any) {
    templates.value = []
    templatesError.value = normalizeApiError(
      e,
      "Connector templates are unavailable. Manual JSON configuration remains available."
    )
  } finally {
    templatesLoading.value = false
  }
}

const runValidateUpdate = async () => {
  if (!selectedName.value) return
  error.value = ""
  try {
    const config = parseJsonObject(updateConfigText.value)
    updateValidateResult.value = await validateConnector({
      name: selectedName.value,
      config,
    })
    updateValidateHash.value = updateConfigText.value
    if (updateValidateResult.value.valid) toast.success("Update validation passed.")
    else toast.warning("Update validation found issues.")
  } catch (e: any) {
    updateValidateResult.value = null
    error.value = normalizeApiError(e, "Update validation failed")
  }
}

const runUpdate = async () => {
  if (!selectedName.value) return
  actionLoading.value = true
  error.value = ""
  try {
    if (updateValidateHash.value !== updateConfigText.value || !updateValidateResult.value?.valid) {
      await runValidateUpdate()
      if (!updateValidateResult.value?.valid) {
        throw new Error("Update blocked: validation failed. Fix issues and validate again.")
      }
    }
    const config = parseJsonObject(updateConfigText.value)
    const res = await updateConnectorConfig(selectedName.value, config)
    lastAuditId.value = res.audit_id || ""
    toast.success(`Connector ${selectedName.value} updated${lastAuditId.value ? ` (audit: ${lastAuditId.value})` : ""}`)
    await loadConnectors()
    await loadTimeline()
  } catch (e: any) {
    error.value = normalizeApiError(e, "Update failed")
  } finally {
    actionLoading.value = false
  }
}

const pendingResetOffsets = ref(false)

const confirmResetOffsets = async () => {
  pendingResetOffsets.value = false
  await runConnectorAction("reset-offsets")
}

const runConnectorAction = async (
  action: "pause" | "resume" | "restart" | "stop" | "reset-offsets",
) => {
  if (!selectedName.value) return
  actionLoading.value = true
  error.value = ""
  try {
    const res = await runAction(selectedName.value, action)
    lastAuditId.value = res.audit_id || ""
    toast.success(`Connector ${selectedName.value} ${action} requested${lastAuditId.value ? ` (audit: ${lastAuditId.value})` : ""}`)
    await loadConnectors()
    await loadTimeline()
  } catch (e: any) {
    error.value = normalizeApiError(e, "Action failed")
  } finally {
    actionLoading.value = false
  }
}

const runRestartTask = async (taskId: number) => {
  if (!selectedName.value) return
  actionLoading.value = true
  error.value = ""
  try {
    const res = await restartTask(selectedName.value, taskId)
    lastAuditId.value = res.audit_id || ""
    toast.success(`Task ${taskId} restart requested${lastAuditId.value ? ` (audit: ${lastAuditId.value})` : ""}`)
    await loadConnectors()
    await loadTimeline()
    if (selectedName.value) {
      const status = await fetchConnectorStatus(selectedName.value)
      connectors.value[selectedName.value] = {
        ...(connectors.value[selectedName.value] || {}),
        status: status.status,
      }
    }
  } catch (e: any) {
    error.value = normalizeApiError(e, "Task restart failed")
  } finally {
    actionLoading.value = false
  }
}

const requestDelete = () => {
  if (!selectedName.value) return
  deleteConfirmText.value = ""
  showDeleteConfirm.value = true
}

const runDelete = async () => {
  if (!selectedName.value) return
  if (deleteConfirmText.value.trim() !== selectedName.value) {
    error.value = "Delete confirmation text does not match connector name."
    return
  }
  actionLoading.value = true
  error.value = ""
  try {
    const deleted = selectedName.value
    const res = await removeConnector(deleted)
    lastAuditId.value = res.audit_id || ""
    toast.success(`Connector ${deleted} deleted${lastAuditId.value ? ` (audit: ${lastAuditId.value})` : ""}`)
    selectedName.value = ""
    showDeleteConfirm.value = false
    await loadConnectors()
    await loadTimeline()
    await syncQuery()
  } catch (e: any) {
    error.value = normalizeApiError(e, "Delete failed")
  } finally {
    actionLoading.value = false
  }
}

const openTaskTrace = (task: any) => {
  if (!task?.trace) return
  traceView.value = { taskId: task.id ?? -1, trace: String(task.trace) }
}

const copyAuditId = async () => {
  if (!lastAuditId.value) return
  await navigator.clipboard.writeText(lastAuditId.value)
  toast.info("Audit ID copied.")
}

const copyJson = async () => {
  await navigator.clipboard.writeText(updateConfigText.value)
  toast.info("JSON config copied.")
}

const runProbe = async () => {
  if (!selectedName.value) return
  probeLoading.value = true
  probeResult.value = null
  try {
    probeResult.value = await probeConnector({ name: selectedName.value })
    if (probeResult.value.valid) toast.success("Connection test passed.")
    else toast.warning(probeResult.value.message || "Connection test failed.")
  } catch (e: any) {
    probeResult.value = { valid: false, reachable: false, message: normalizeApiError(e, "Connection test failed") }
  } finally {
    probeLoading.value = false
  }
}

const runCatalog = async () => {
  if (!selectedName.value) return
  catalogLoading.value = true
  catalogResult.value = null
  selectedSchema.value = ""
  catalogPage.value = 1
  try {
    catalogResult.value = await catalogConnector({ name: selectedName.value })
    if (catalogResult.value.reachable) toast.success("Catalog loaded.")
    else toast.warning(catalogResult.value.message || "Catalog browse failed.")
  } catch (e: any) {
    catalogResult.value = null
    error.value = normalizeApiError(e, "Failed to browse schemas and tables")
  } finally {
    catalogLoading.value = false
  }
}

const selectSchema = (name: string) => {
  selectedSchema.value = selectedSchema.value === name ? "" : name
  catalogPage.value = 1
}

const loadConnectorTopics = async (name: string) => {
  topicsLoading.value = true
  topicsError.value = ""
  try {
    const data = await $fetch<Record<string, { topics: string[] }>>(`/api/cdc/connectors/${encodeURIComponent(name)}/topics`)
    connectorTopics.value = data[name]?.topics || []
  } catch (e: any) {
    topicsError.value = normalizeApiError(e, "Failed to load topics")
    connectorTopics.value = []
  } finally {
    topicsLoading.value = false
  }
}

const loadPlugins = async () => {
  pluginsLoading.value = true
  pluginsError.value = ""
  try {
    pluginsList.value = await $fetch<any[]>("/api/cdc/connectors/plugins")
  } catch (e: any) {
    pluginsError.value = normalizeApiError(e, "Failed to load plugins")
    pluginsList.value = []
  } finally {
    pluginsLoading.value = false
  }
}

const setDetailTab = (id: string) => {
  if (["overview", "json", "catalog", "tasks", "timeline", "topics"].includes(id)) {
    detailTab.value = id as typeof detailTab.value
    if (id === "topics" && selectedName.value) {
      loadConnectorTopics(selectedName.value)
    }
  }
}

const onCreated = async (name: string) => {
  showCreate.value = false
  selectedName.value = name
  await loadConnectors()
  await loadTimeline()
  await syncQuery()
}

watch(selectedName, async (name) => {
  await syncQuery()
  traceView.value = null
  updateValidateResult.value = null
  updateValidateHash.value = ""
  probeResult.value = null
  catalogResult.value = null
  selectedSchema.value = ""
  catalogSearch.value = ""
  catalogPage.value = 1
  await loadTimeline(name)
  await loadSelectedConfig(name)
})

watch([listSearch, stateFilter, failedOnly, sortMode, kindFilter, detailTab], async () => {
  if (!initialized.value) return
  setPref("connectors.sort", sortMode.value)
  await syncQuery()
})

watch(updateConfigText, () => {
  if (updateValidateHash.value !== updateConfigText.value) {
    updateValidateResult.value = null
  }
})

watch([catalogSearch, catalogPageSize, selectedSchema], () => {
  catalogPage.value = 1
})

const displayedJson = computed(() => {
  if (revealSecrets.value) return updateConfigText.value
  try {
    return stringifyConfig(parseJsonObject(updateConfigText.value), false)
  } catch {
    return updateConfigText.value
  }
})

const handleDialogKeydown = (event: KeyboardEvent) => {
  if (event.key !== "Escape") return
  traceView.value = null
  showDeleteConfirm.value = false
  showCreate.value = false
}

onMounted(async () => {
  window.addEventListener("keydown", handleDialogKeydown)
  selectedName.value = String(route.query.name || "")
  listSearch.value = String(route.query.search || "")
  stateFilter.value = String(route.query.state || "all")
  kindFilter.value = (String(route.query.kind || "all") as "all" | ConnectorKind) || "all"
  failedOnly.value = String(route.query.failed_only || "false") === "true"
  sortMode.value = (String(route.query.sort || getPref("connectors.sort", "name_asc")) ||
    "name_asc") as "name_asc" | "name_desc" | "state"
  detailTab.value = (String(route.query.tab || "overview") as typeof detailTab.value) || "overview"
  setPref("connectors.sort", sortMode.value)
  await Promise.all([loadConnectors(), loadTemplates()])
  await loadTimeline()
  initialized.value = true
})

onUnmounted(() => {
  window.removeEventListener("keydown", handleDialogKeydown)
})
</script>

<template>
  <div class="page-container">
    <AppPageHeader
      title="Connectors"
      description="Source connectors capture database changes. Sink connectors apply those events to a target database. Inspect JSON, test connectivity, and browse schemas without leaving this page."
    >
      <AppButton :disabled="loading" @click="loadConnectors">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
      <AppButton variant="primary" @click="showCreate = true">
        <Plus class="h-4 w-4" :stroke-width="1.75" />
        New Connector
      </AppButton>
    </AppPageHeader>

    <div class="grid grid-cols-2 gap-3 md:grid-cols-4">
      <AppMetricCard label="Source" :value="summary.source" helper="Capture from databases" tone="info">
        <Download class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard label="Sink" :value="summary.sink" helper="Write to target databases" tone="success">
        <Database class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard label="Running" :value="summary.running" helper="Healthy connector tasks" tone="brand">
        <Play class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard label="Failed" :value="summary.failed" helper="Needs attention" tone="danger">
        <Plug class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
    </div>

    <AppCard>
      <AppResultsHeader
        label="Connectors"
        :total="filteredConnectorNames.length"
        :loading="loading"
        :last-updated="lastUpdated"
      />
    </AppCard>

    <AppStateBlock v-if="error" kind="error" :message="error" />
    <div
      v-if="lastAuditId"
      class="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-blue-500/25 bg-blue-500/10 px-4 py-3 text-sm text-blue-700 dark:text-blue-400"
    >
      <span>Last operation audit ID: <code class="font-mono">{{ lastAuditId }}</code></span>
      <AppButton size="icon" title="Copy Audit ID" @click="copyAuditId">
        <Copy class="h-4 w-4" :stroke-width="1.75" />
      </AppButton>
    </div>

    <div
      v-if="!loading && Object.keys(connectors).length === 0"
      class="rounded-xl border border-blue-500/25 bg-blue-500/10 px-4 py-3 text-sm text-blue-700 dark:text-blue-400"
    >
      New to Kafka Connect? Use
      <NuxtLink to="/pipelines" class="font-semibold underline">Guided CDC Pipeline</NuxtLink>
      or create a Source / Sink connector with JSON.
    </div>

    <section class="grid grid-cols-1 xl:grid-cols-3 gap-4">
      <AppCard class="xl:col-span-1">
        <h2 class="section-title mb-3">Connector List</h2>
        <div class="space-y-2 mb-3">
          <AppInput v-model="listSearch" placeholder="Search connector or database..." />
          <div class="flex items-center gap-2">
            <AppSelect v-model="kindFilter" select-class="flex-1" aria-label="Connector kind">
              <option value="all">All kinds</option>
              <option value="source">Source</option>
              <option value="sink">Sink</option>
            </AppSelect>
            <AppSelect v-model="stateFilter" select-class="flex-1" aria-label="Connector state">
              <option value="all">All states</option>
              <option value="RUNNING">RUNNING</option>
              <option value="PAUSED">PAUSED</option>
              <option value="FAILED">FAILED</option>
              <option value="UNASSIGNED">UNASSIGNED</option>
            </AppSelect>
          </div>
          <div class="flex items-center justify-between gap-2">
            <label class="inline-flex items-center gap-2 text-sm text-muted-foreground">
              <input v-model="failedOnly" type="checkbox" class="accent-brand-500" />
              Failed only
            </label>
            <AppSelect v-model="sortMode" select-class="w-32" aria-label="Sort connectors">
              <option value="name_asc">Name A-Z</option>
              <option value="name_desc">Name Z-A</option>
              <option value="state">By state</option>
            </AppSelect>
          </div>
        </div>
        <div class="max-h-[560px] space-y-4 overflow-y-auto overflow-x-hidden px-px">
          <section v-for="group in [{ key: 'source', label: 'Source', names: groupedNames.source }, { key: 'sink', label: 'Sink', names: groupedNames.sink }]" :key="group.key">
            <div v-if="kindFilter === 'all' || kindFilter === group.key" class="mb-2 flex items-center justify-between">
              <h3 class="text-sm font-medium text-muted-foreground">{{ group.label }}</h3>
              <span class="text-sm text-muted-foreground">{{ group.names.length }}</span>
            </div>
            <div v-if="kindFilter === 'all' || kindFilter === group.key" class="space-y-1">
              <button
                v-for="name in group.names"
                :key="name"
                class="list-option"
                :class="selectedName === name ? 'list-option-active' : ''"
                @click="selectedName = name"
              >
                <DatabaseTypeMark
                  :database-type="insightFor(name).databaseType"
                  :kind="insightFor(name).kind"
                  size="sm"
                  :subtitle="name"
                />
                <div class="mt-2 flex items-center justify-between gap-2">
                  <AppStatusBadge :status="connectors[name]?.status?.connector?.state || 'UNKNOWN'" />
                  <span class="truncate font-mono text-sm text-muted-foreground">
                    {{ insightFor(name).connection.host || "—" }}
                  </span>
                </div>
              </button>
              <AppStateBlock
                v-if="!loading && group.names.length === 0"
                kind="empty"
                :message="`No ${group.label.toLowerCase()} connectors match current filters.`"
              />
            </div>
          </section>
        </div>
      </AppCard>

      <div class="xl:col-span-2 space-y-4">
        <AppEmptyState
          v-if="!selectedName"
          title="Select a Connector"
          description="Choose a Source or Sink connector to inspect configuration, test connectivity, and browse schemas."
        >
          <template #icon><List class="h-5 w-5" :stroke-width="1.75" /></template>
        </AppEmptyState>

        <template v-else>
          <AppCard>
            <div class="flex flex-wrap items-start justify-between gap-3">
              <DatabaseTypeMark
                :database-type="selectedInsight.databaseType"
                :kind="selectedInsight.kind"
                size="lg"
                :subtitle="`${selectedName} · ${connectionSubtitle}`"
              />
              <AppStatusBadge :status="selectedState" />
            </div>
            <div class="mt-4 flex flex-wrap gap-2">
              <AppButton :disabled="actionLoading || probeLoading" @click="runProbe">
                <Plug class="h-4 w-4" :stroke-width="1.75" />
                {{ probeLoading ? "Testing..." : "Test Connectivity" }}
              </AppButton>
              <AppButton :disabled="actionLoading" @click="runConnectorAction('pause')">
                <Pause class="h-4 w-4" :stroke-width="1.75" />
                Pause
              </AppButton>
              <AppButton :disabled="actionLoading" @click="runConnectorAction('resume')">
                <Play class="h-4 w-4" :stroke-width="1.75" />
                Resume
              </AppButton>
              <AppButton :disabled="actionLoading" @click="runConnectorAction('stop')">
                <Square class="h-4 w-4" :stroke-width="1.75" />
                Stop
              </AppButton>
              <AppButton :disabled="actionLoading" @click="runConnectorAction('restart')">
                <RotateCw class="h-4 w-4" :stroke-width="1.75" />
                Restart
              </AppButton>
              <AppButton :disabled="actionLoading" @click="pendingResetOffsets = true">
                <RefreshCcwDot class="h-4 w-4" :stroke-width="1.75" />
                Reset Offsets
              </AppButton>
              <AppButton variant="destructive" :disabled="actionLoading" @click="requestDelete">
                <Trash2 class="h-4 w-4" :stroke-width="1.75" />
                Delete
              </AppButton>
            </div>
            <p
              v-if="probeResult"
              class="mt-3 text-sm"
              :class="probeResult.valid ? 'text-emerald-700 dark:text-emerald-400' : 'text-red-700 dark:text-red-400'"
            >
              {{ probeResult.message }}
            </p>
          </AppCard>

          <div class="flex flex-wrap gap-1 rounded-xl border border-border bg-muted/40 p-1">
            <button
              v-for="tab in [
                { id: 'overview', label: 'Overview', icon: Database },
                { id: 'json', label: 'JSON', icon: Braces },
                { id: 'catalog', label: 'Catalog', icon: FolderTree },
                { id: 'topics', label: 'Topics', icon: Tag },
                { id: 'tasks', label: 'Tasks', icon: List },
                { id: 'timeline', label: 'Timeline', icon: Eye },
              ]"
              :key="tab.id"
              type="button"
              class="inline-flex items-center gap-1.5 rounded-lg px-3 py-2 text-sm font-medium transition-colors"
              :class="detailTab === tab.id ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'"
              @click="setDetailTab(tab.id)"
            >
              <component :is="tab.icon" class="h-4 w-4" :stroke-width="1.75" />
              {{ tab.label }}
            </button>
          </div>

          <AppCard v-if="detailTab === 'overview'" class="min-h-[520px]">
            <h2 class="section-title mb-4">Connection Overview</h2>
            <div class="mb-6 grid grid-cols-1 gap-3 md:grid-cols-[1fr,auto,1fr] md:items-center">
              <div class="rounded-xl border border-border bg-muted/40 p-4">
                <p class="mb-3 text-sm text-muted-foreground">{{ selectedInsight.kind === 'sink' ? 'Target Database' : 'Source Database' }}</p>
                <DatabaseTypeMark
                  :database-type="selectedInsight.databaseType"
                  :kind="selectedInsight.kind"
                  size="md"
                  :subtitle="connectionSubtitle"
                />
              </div>
              <div class="hidden justify-center text-muted-foreground md:flex">→</div>
              <div class="rounded-xl border border-border bg-muted/40 p-4">
                <p class="mb-3 text-sm text-muted-foreground">{{ selectedInsight.kind === 'sink' ? 'Consumes From' : 'Publishes To' }}</p>
                <p class="text-sm font-medium">Kafka / Redpanda</p>
                <p class="mt-1 text-sm text-muted-foreground">
                  {{ selectedInsight.kind === 'sink' ? (selectedConfig['topics.regex'] || selectedConfig.topics || 'Configured topics') : (selectedConfig['topic.prefix'] || 'Topic prefix from JSON') }}
                </p>
              </div>
            </div>
            <div class="grid grid-cols-1 gap-3 md:grid-cols-3">
              <div class="rounded-xl border border-border p-4">
                <p class="text-sm text-muted-foreground">Class</p>
                <p class="mt-1 break-all font-mono text-sm">{{ selectedInsight.className || "—" }}</p>
              </div>
              <div class="rounded-xl border border-border p-4">
                <p class="text-sm text-muted-foreground">User</p>
                <p class="mt-1 font-mono text-sm">{{ selectedInsight.connection.username || "—" }}</p>
              </div>
              <div class="rounded-xl border border-border p-4">
                <p class="text-sm text-muted-foreground">Capture Scope</p>
                <p class="mt-1 text-sm">
                  {{ selectedInsight.configuredSchemas.join(", ") || "All schemas" }}
                </p>
              </div>
            </div>
            <div v-if="selectedInsight.configuredTables.length" class="mt-4">
              <p class="auth-label">Configured Tables</p>
              <div class="flex flex-wrap gap-2">
                <span v-for="table in selectedInsight.configuredTables" :key="table" class="badge-neutral">{{ table }}</span>
              </div>
            </div>
          </AppCard>

          <AppCard v-else-if="detailTab === 'json'" class="min-h-[520px]">
            <div class="mb-3 flex flex-wrap items-center justify-between gap-2">
              <h2 class="section-title">JSON Config</h2>
              <div class="flex flex-wrap items-center gap-2">
                <label class="inline-flex items-center gap-2 text-sm text-muted-foreground">
                  <input v-model="revealSecrets" type="checkbox" class="accent-brand-500" />
                  Reveal secrets
                </label>
                <AppButton size="icon" title="Copy JSON" @click="copyJson">
                  <Copy class="h-4 w-4" :stroke-width="1.75" />
                </AppButton>
              </div>
            </div>
            <p class="mb-3 text-sm text-muted-foreground">
              Full {{ selectedInsight.kind === 'sink' ? 'Sink' : 'Source' }} configuration for {{ selectedInsight.databaseLabel }}.
              Validate before applying updates.
            </p>
            <textarea
              v-if="revealSecrets"
              v-model="updateConfigText"
              class="auth-input form-textarea min-h-[280px]"
              spellcheck="false"
            />
            <textarea
              v-else
              :value="displayedJson"
              class="auth-input form-textarea min-h-[280px]"
              spellcheck="false"
              readonly
            />
            <div v-if="updateValidateResult" class="mt-2 text-sm">
              <p>
                Update valid:
                <span :class="updateValidateResult.valid ? 'text-emerald-700 dark:text-emerald-400' : 'text-red-700 dark:text-red-400'">
                  {{ updateValidateResult.valid }}
                </span>
              </p>
              <p v-if="updateValidateResult.warnings?.length" class="mt-1 text-amber-700 dark:text-amber-400">
                Warnings: {{ updateValidateResult.warnings.join(" | ") }}
              </p>
              <p v-if="updateValidateResult.errors?.length" class="mt-1 text-red-700 dark:text-red-400">
                Errors:
                <span v-for="(item, idx) in updateValidateResult.errors" :key="idx">{{ item.field }} - {{ item.message }}; </span>
              </p>
            </div>
            <div class="mt-3 flex flex-wrap gap-2">
              <AppButton :disabled="actionLoading || !revealSecrets" @click="runValidateUpdate">Validate Update</AppButton>
              <AppButton variant="primary" :disabled="!canApplyUpdate || actionLoading || !revealSecrets" @click="runUpdate">
                Update Config
              </AppButton>
            </div>
            <div class="mt-5">
              <div class="mb-2 flex items-center justify-between">
                <h3 class="section-title">Config Diff Preview</h3>
                <span class="text-sm text-muted-foreground">Changed keys: {{ changedCount }}</span>
              </div>
              <AppTable>
                <thead>
                  <tr>
                    <th>Key</th>
                    <th>Current</th>
                    <th>Next</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="row in configDiff.filter((item) => item.changed || changedCount < 12)" :key="row.key" :class="row.changed ? 'bg-brand-500/10' : ''">
                    <td class="font-mono break-all">{{ row.key }}</td>
                    <td class="font-mono break-all">{{ row.current || "—" }}</td>
                    <td class="font-mono break-all">{{ row.next || "—" }}</td>
                  </tr>
                  <tr v-if="configDiff.length === 0">
                    <td colspan="3">
                      <AppStateBlock kind="empty" message="No config to compare." />
                    </td>
                  </tr>
                </tbody>
              </AppTable>
            </div>
          </AppCard>

          <AppCard v-else-if="detailTab === 'catalog'" class="min-h-[520px]">
            <div class="mb-4 flex flex-wrap items-center justify-between gap-2">
              <div>
                <AppSectionTitle title="Schema Catalog" hint="Test the {{ selectedInsight.kind === 'sink' ? 'target' : 'source' }} database and browse live schemas and tables." />
              </div>
              <div class="flex gap-2">
                <AppButton :disabled="probeLoading || !selectedName" @click="runProbe">
                  <Plug class="h-4 w-4" :stroke-width="1.75" />
                  Test Connectivity
                </AppButton>
                <AppButton variant="primary" :disabled="catalogLoading || !selectedName" @click="runCatalog">
                  <FolderTree class="h-4 w-4" :stroke-width="1.75" />
                  {{ catalogLoading ? "Browsing..." : "Browse Schemas" }}
                </AppButton>
              </div>
            </div>
            <p
              v-if="probeResult || catalogResult"
              class="mb-4 text-sm"
              :class="(catalogResult?.reachable ?? probeResult?.valid) ? 'text-emerald-700 dark:text-emerald-400' : 'text-red-700 dark:text-red-400'"
            >
              {{ catalogResult?.message || probeResult?.message }}
            </p>
            <AppStateBlock v-if="catalogLoading" kind="loading" message="Loading schemas and tables..." />
            <div v-else-if="catalogResult" class="grid grid-cols-1 gap-4 md:grid-cols-[220px,minmax(0,1fr)]">
              <div>
                <p class="auth-label">Schemas</p>
                <div class="max-h-[360px] space-y-1 overflow-y-auto overflow-x-hidden px-px">
                  <button
                    type="button"
                    class="list-option"
                    :class="selectedSchema === '' ? 'list-option-active' : ''"
                    @click="selectSchema('')"
                  >
                    All Schemas
                    <span class="mt-0.5 block text-sm text-muted-foreground">{{ catalogResult.schemas?.length || 0 }}</span>
                  </button>
                  <button
                    v-for="schema in catalogResult.schemas || []"
                    :key="schema.name"
                    type="button"
                    class="list-option"
                    :class="selectedSchema === schema.name ? 'list-option-active' : ''"
                    @click="selectSchema(schema.name)"
                  >
                    <div class="flex items-center justify-between gap-2">
                      <span class="font-mono text-sm">{{ schema.name }}</span>
                      <span :class="schema.captured ? 'badge-success' : 'badge-neutral'">
                        {{ schema.captured ? "Captured" : "Other" }}
                      </span>
                    </div>
                    <p class="mt-1 text-sm text-muted-foreground">{{ schema.table_count }} tables</p>
                  </button>
                </div>
              </div>
              <div>
                <div class="mb-2 flex flex-wrap items-center justify-between gap-2">
                  <p class="auth-label mb-0">Tables</p>
                  <AppInput v-model="catalogSearch" placeholder="Filter tables..." input-class="w-56" />
                </div>
                <AppTable>
                  <thead>
                    <tr>
                      <th>Schema</th>
                      <th>Table</th>
                      <th>Records</th>
                      <th>Capture</th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr v-for="table in pagedCatalogTables" :key="`${table.schema}.${table.name}`">
                      <td class="font-mono">{{ table.schema }}</td>
                      <td class="font-mono">{{ table.name }}</td>
                      <td class="text-muted-foreground">{{ table.row_count == null ? "—" : Number(table.row_count).toLocaleString() }}</td>
                      <td>
                        <span :class="table.captured ? 'badge-success' : 'badge-neutral'">
                          {{ table.captured ? "Captured" : "Not In Connector" }}
                        </span>
                      </td>
                    </tr>
                    <tr v-if="pagedCatalogTables.length === 0">
                      <td colspan="4">
                        <AppStateBlock kind="empty" message="No tables to show. Browse schemas after a successful connection test." />
                      </td>
                    </tr>
                  </tbody>
                </AppTable>
                <div v-if="filteredCatalogTables.length" class="mt-3 flex flex-wrap items-center justify-between gap-3">
                  <label class="flex items-center gap-2 text-sm text-muted-foreground">
                    Rows per page
                    <AppSelect v-model="catalogPageSize" select-class="h-8 w-20" aria-label="Rows per page">
                      <option value="20">20</option>
                      <option value="50">50</option>
                      <option value="100">100</option>
                    </AppSelect>
                  </label>
                  <AppPager
                    :page="catalogPage"
                    :total-pages="catalogTotalPages"
                    @prev="catalogPage = Math.max(1, catalogPage - 1)"
                    @next="catalogPage = Math.min(catalogTotalPages, catalogPage + 1)"
                  />
                </div>
              </div>
            </div>
            <AppStateBlock
              v-else
              kind="empty"
              message="Run Browse Schemas to inspect the live database catalog for this connector."
            />
          </AppCard>

          <AppCard v-else-if="detailTab === 'tasks'" class="min-h-[520px]">
            <h2 class="section-title mb-3">Tasks</h2>
            <AppTable>
              <thead>
                <tr>
                  <th>Task</th>
                  <th>State</th>
                  <th>Worker</th>
                  <th>Action</th>
                  <th>Trace</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="task in selected?.status?.tasks || []" :key="`${task.id}-${task.worker_id}`">
                  <td>{{ task.id }}</td>
                  <td>{{ task.state || "UNKNOWN" }}</td>
                  <td class="font-mono">{{ task.worker_id || "—" }}</td>
                  <td>
                    <AppButton
                      size="icon"
                      title="Restart Task"
                      :disabled="actionLoading || task.id == null"
                      @click="task.id != null && runRestartTask(task.id)"
                    >
                      <RotateCw class="h-4 w-4" :stroke-width="1.75" />
                    </AppButton>
                  </td>
                  <td>
                    <AppButton size="icon" title="View Trace" :disabled="!task.trace" @click="openTaskTrace(task)">
                      <Eye class="h-4 w-4" :stroke-width="1.75" />
                    </AppButton>
                  </td>
                </tr>
                <tr v-if="!selectedName || (selected?.status?.tasks?.length || 0) === 0">
                  <td colspan="5">
                    <AppStateBlock kind="empty" message="No tasks for selected connector." />
                  </td>
                </tr>
              </tbody>
            </AppTable>
          </AppCard>

          <!-- Topics tab -->
          <AppCard v-else-if="detailTab === 'topics'" class="min-h-[520px]">
            <div class="mb-4 flex items-center justify-between">
              <h2 class="section-title">Subscribed Topics</h2>
              <AppButton :disabled="topicsLoading" @click="loadConnectorTopics(selectedName)">
                <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': topicsLoading }" :stroke-width="1.75" />
                Refresh
              </AppButton>
            </div>
            <AppStateBlock v-if="topicsLoading" kind="loading" message="Loading topics…" />
            <AppStateBlock v-else-if="topicsError" kind="error" :message="topicsError" />
            <div v-else-if="connectorTopics.length" class="space-y-1">
              <div
                v-for="topic in connectorTopics"
                :key="topic"
                class="flex items-center gap-3 rounded-xl border border-surface-700/50 bg-surface-800/30 px-4 py-2.5"
              >
                <Tag class="h-4 w-4 flex-shrink-0 text-brand-400" :stroke-width="1.75" />
                <NuxtLink :to="`/topics/${encodeURIComponent(topic)}`" class="font-mono text-sm text-brand-400 hover:underline">
                  {{ topic }}
                </NuxtLink>
              </div>
            </div>
            <AppStateBlock v-else kind="empty" message="No topics subscribed yet. Topics appear after the connector processes records." />
          </AppCard>

          <!-- Timeline tab (default / else) -->
          <AppCard v-else class="min-h-[520px]">
            <div class="mb-3 flex items-center justify-between">
              <h2 class="section-title">State Timeline</h2>
              <span class="text-sm text-muted-foreground">Last 50 transitions</span>
            </div>
            <div class="max-h-[420px] space-y-1 overflow-auto text-sm">
              <AppStateBlock v-if="timelineLoading" kind="loading" message="Loading timeline..." />
              <AppStateBlock v-else-if="timelineError" kind="error" :message="timelineError" />
              <div
                v-for="(entry, idx) in timelineEntries"
                :key="`${entry.ts}-${idx}`"
                class="rounded-lg border border-border bg-muted px-2 py-1 text-foreground"
              >
                <span class="text-muted-foreground">{{ new Date(entry.ts).toLocaleTimeString() }}</span>
                ·
                <span class="font-mono">{{ entry.from || "—" }}</span>
                →
                <span class="font-mono">{{ entry.to }}</span>
                <span v-if="entry.rule" class="text-muted-foreground">({{ entry.rule }})</span>
                <span v-if="entry.task_errors?.length" class="text-red-700 dark:text-red-400">
                  · task errors: {{ entry.task_errors.length }}
                </span>
              </div>
              <AppStateBlock
                v-if="!timelineLoading && !timelineError && (!selectedName || timelineEntries.length === 0)"
                kind="empty"
                message="No timeline events yet for this connector."
              />
            </div>
          </AppCard>
        </template>
      </div>
    </section>

    <!-- Plugins section -->
    <AppCard>
      <div class="flex items-center justify-between">
        <div>
          <h2 class="section-title">Installed Plugins</h2>
          <p class="mt-0.5 text-xs text-surface-400">All connector classes available in this Kafka Connect cluster</p>
        </div>
        <AppButton :disabled="pluginsLoading" @click="loadPlugins">
          <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': pluginsLoading }" :stroke-width="1.75" />
          {{ pluginsList.length ? 'Refresh' : 'Load Plugins' }}
        </AppButton>
      </div>
      <AppStateBlock v-if="pluginsLoading" kind="loading" message="Loading plugins…" class="mt-4" />
      <AppStateBlock v-else-if="pluginsError" kind="error" :message="pluginsError" class="mt-4" />
      <div v-else-if="pluginsList.length" class="mt-4 grid grid-cols-1 gap-2 md:grid-cols-2 xl:grid-cols-3">
        <div
          v-for="plugin in pluginsList"
          :key="plugin.class"
          class="rounded-xl border border-surface-700/50 bg-surface-800/30 px-4 py-3"
        >
          <div class="flex items-start justify-between gap-2">
            <span class="font-mono text-xs text-surface-200 leading-relaxed break-all">{{ plugin.class }}</span>
            <span
              :class="[
                'mt-0.5 flex-shrink-0 rounded-md px-1.5 py-0.5 text-[10px] font-medium',
                plugin.type === 'source' ? 'bg-blue-500/10 text-blue-400' :
                plugin.type === 'sink' ? 'bg-emerald-500/10 text-emerald-400' :
                'bg-surface-700/50 text-surface-400'
              ]"
            >
              {{ (plugin.type || 'unknown').toUpperCase() }}
            </span>
          </div>
          <div v-if="plugin.version" class="mt-1 text-xs text-surface-400">v{{ plugin.version }}</div>
        </div>
      </div>
      <AppStateBlock v-else-if="!pluginsLoading" kind="empty" message="Click 'Load Plugins' to browse available connector classes." class="mt-4" />
    </AppCard>
  </div>

  <ConnectorCreateDialog
    :open="showCreate"
    :templates="templates"
    :templates-loading="templatesLoading"
    :templates-error="templatesError"
    :action-loading="actionLoading"
    @close="showCreate = false"
    @created="onCreated"
  />

  <Teleport to="body">
    <div
      v-if="traceView"
      class="modal-overlay"
      role="dialog"
      aria-modal="true"
      aria-labelledby="task-trace-title"
      @click.self="traceView = null"
    >
      <section class="auth-card flex max-h-[85vh] w-full max-w-4xl flex-col overflow-hidden bg-card p-5 animate-slide-up">
        <div class="mb-3 flex items-center justify-between gap-3">
          <h2 id="task-trace-title" class="section-title">
            Task Trace (Task {{ traceView.taskId >= 0 ? traceView.taskId : "Unknown" }})
          </h2>
          <AppButton size="icon" title="Close Task Trace" @click="traceView = null">
            <X class="h-4 w-4" :stroke-width="1.75" />
          </AppButton>
        </div>
        <pre class="code-block min-h-0 flex-1">{{ traceView.trace }}</pre>
      </section>
    </div>

    <div
      v-if="showDeleteConfirm"
      class="modal-overlay"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="delete-connector-title"
      @click.self="showDeleteConfirm = false"
    >
      <section class="auth-card w-full max-w-md bg-card p-5 animate-slide-up">
        <h2 id="delete-connector-title" class="section-title mb-2 text-destructive">Confirm Delete</h2>
        <p class="mb-4 text-sm text-muted-foreground">
          Type connector name <span class="font-mono text-foreground">{{ selectedName }}</span> to confirm deletion.
        </p>
        <AppInput
          v-model="deleteConfirmText"
          placeholder="Type connector name"
          aria-label="Connector name confirmation"
        />
        <div class="mt-4 flex items-center justify-end gap-2">
          <AppButton :disabled="actionLoading" @click="showDeleteConfirm = false">Cancel</AppButton>
          <AppButton
            variant="destructive"
            :disabled="actionLoading || deleteConfirmText.trim() !== selectedName"
            @click="runDelete"
          >
            <Trash2 class="h-4 w-4" :stroke-width="1.75" />
            Delete Now
          </AppButton>
        </div>
      </section>
    </div>

    <AppConfirmDialog
      :open="pendingResetOffsets"
      title="Reset connector offsets?"
      confirm-label="Reset Offsets"
      :loading="actionLoading"
      @update:open="(open) => { if (!open) pendingResetOffsets = false }"
      @confirm="confirmResetOffsets"
    >
      <p>
        Clears the stored position of
        <strong class="font-mono">{{ selectedName }}</strong>. A source connector will
        re-read its table from the beginning the next time it runs.
      </p>
      <p class="mt-2 text-xs text-muted-foreground">
        The connector must be stopped first. To reset and replay in one step, use the
        Resync action on the pipeline instead.
      </p>
    </AppConfirmDialog>
  </Teleport>
</template>
