<script setup lang="ts">
import { Bell, Cable, List, RefreshCw, Server, Users } from "@lucide/vue"
import type { AlertHistoryItem } from "~/composables/useOperationsM3"
import { formatApiError } from "~/utils/apiError"

const { fetchEngineStatus } = useControlPlane()
const { fetchTopics, fetchGroups } = useKafkaM1()
const { fetchConnectors } = useConnectorsM2()
const { fetchAlertHistory, fetchMetrics } = useOperationsM3()

const engine = ref<EngineStatus | null>(null)
const connectors = ref<Record<string, any>>({})
const topicCount = ref(0)
const groupCount = ref(0)
const alerts = ref<AlertHistoryItem[]>([])
const rawMetrics = ref("")
const loading = ref(false)
const error = ref("")
const autoRefresh = ref(true)
const lastUpdated = ref("")
let timer: ReturnType<typeof setInterval> | null = null

const connectorList = computed(() => Object.entries(connectors.value))
const failedConnectors = computed(
  () => connectorList.value.filter(([, value]) => String(value?.status?.connector?.state || "").toUpperCase() === "FAILED").length
)
const runningConnectors = computed(
  () => connectorList.value.filter(([, value]) => String(value?.status?.connector?.state || "").toUpperCase() === "RUNNING").length
)
const runningServices = computed(
  () => engine.value?.services?.filter((service) => service.state.toLowerCase().includes("running")).length || 0
)
const metricLines = computed(() =>
  rawMetrics.value
    .split("\n")
    .filter((line) => line && !line.startsWith("#"))
    .slice(0, 12)
)

const refresh = async () => {
  loading.value = true
  error.value = ""
  const results = await Promise.allSettled([
    fetchEngineStatus(),
    fetchConnectors(),
    fetchTopics({ page: 1, page_size: 1 }),
    fetchGroups({ page: 1, page_size: 1 }),
    fetchAlertHistory(20),
    fetchMetrics(),
  ])
  if (results[0].status === "fulfilled") engine.value = results[0].value
  if (results[1].status === "fulfilled") connectors.value = results[1].value.connectors || {}
  if (results[2].status === "fulfilled") topicCount.value = results[2].value.total
  if (results[3].status === "fulfilled") groupCount.value = results[3].value.total
  if (results[4].status === "fulfilled") alerts.value = results[4].value || []
  if (results[5].status === "fulfilled") rawMetrics.value = results[5].value || ""
  const failed = results.find((result) => result.status === "rejected")
  if (failed?.status === "rejected") error.value = formatApiError(failed.reason, "Some monitoring panels could not be loaded")
  lastUpdated.value = new Date().toLocaleTimeString()
  loading.value = false
}

const configurePolling = () => {
  if (timer) clearInterval(timer)
  if (autoRefresh.value) timer = setInterval(refresh, 15000)
}

watch(autoRefresh, configurePolling)
onMounted(async () => {
  await refresh()
  configurePolling()
})
onUnmounted(() => {
  if (timer) clearInterval(timer)
})
</script>

<template>
  <div class="page-container">
    <AppPageHeader
      title="Monitoring"
      description="One operational view of runtime health, connector state, Kafka inventory, and recent incidents."
    >
      <label class="flex items-center gap-2 text-sm text-muted-foreground">
        <input v-model="autoRefresh" type="checkbox" class="h-4 w-4 accent-brand-500" />
        Auto-refresh every 15s
      </label>
      <AppButton :disabled="loading" @click="refresh">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <section class="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
      <AppMetricCard
        label="Runtime Services"
        :value="`${runningServices} / ${engine?.services?.length || 0}`"
        helper="Services currently running"
        :tone="runningServices === (engine?.services?.length || -1) ? 'success' : 'warning'"
      >
        <Server class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="Connectors"
        :value="connectorList.length"
        :helper="`${runningConnectors} running · ${failedConnectors} failed`"
        :tone="failedConnectors ? 'danger' : 'brand'"
      >
        <Cable class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard label="Kafka Topics" :value="topicCount" helper="Available event streams" tone="info">
        <List class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard label="Consumer Groups" :value="groupCount" helper="Tracked reader groups" tone="warning">
        <Users class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
    </section>

    <section class="grid grid-cols-1 gap-4 xl:grid-cols-2">
      <div class="stat-card">
        <div class="mb-4 flex items-center justify-between gap-3">
          <div>
            <AppSectionTitle title="Connector Health" hint="Current Kafka Connect state by connector." />
          </div>
          <NuxtLink to="/connectors" class="btn-ghost">Manage Connectors</NuxtLink>
        </div>
        <AppEmptyState
          v-if="!connectorList.length"
          title="No Connectors Found"
          description="Create a connector or pipeline to start moving change events."
        >
          <template #icon><Cable class="h-5 w-5" :stroke-width="1.75" /></template>
        </AppEmptyState>
        <AppTable v-else>
          <thead><tr><th>Connector</th><th>State</th><th>Tasks</th></tr></thead>
          <tbody>
            <tr v-for="[name, value] in connectorList.slice(0, 10)" :key="name">
              <td class="font-mono">{{ name }}</td>
              <td><AppStatusBadge :status="value?.status?.connector?.state" /></td>
              <td>{{ value?.status?.tasks?.length || 0 }}</td>
            </tr>
          </tbody>
        </AppTable>
      </div>

      <div class="stat-card">
        <div class="mb-4 flex items-center justify-between gap-3">
          <div>
            <AppSectionTitle title="Recent Incidents" hint="Latest connector transitions and watcher alerts." />
          </div>
          <NuxtLink to="/alerts" class="btn-ghost">View Alerts</NuxtLink>
        </div>
        <AppEmptyState
          v-if="!alerts.length"
          title="No Recent Incidents"
          description="No connector alert events are present in the in-memory history."
        >
          <template #icon><Bell class="h-5 w-5" :stroke-width="1.75" /></template>
        </AppEmptyState>
        <div v-else class="space-y-2">
          <div
            v-for="(alert, index) in alerts.slice(0, 8)"
            :key="`${alert.ts}-${alert.connector}-${index}`"
            class="flex items-start justify-between gap-3 rounded-xl border border-border bg-muted/40 px-3 py-2"
          >
            <div class="min-w-0">
              <p class="truncate text-sm font-medium">{{ alert.connector || "Unknown Connector" }}</p>
              <p class="mt-1 text-sm text-muted-foreground">{{ alert.ts ? new Date(alert.ts).toLocaleString() : "—" }}</p>
            </div>
            <AppStatusBadge :status="alert.to" />
          </div>
        </div>
      </div>
    </section>

    <details class="stat-card">
      <summary class="flex cursor-pointer list-none items-center gap-1.5">
        <span class="section-title">Prometheus Metrics</span>
        <AppInfoHint text="Raw operational metrics for advanced diagnostics and dashboard integration." />
      </summary>
      <div class="code-block max-h-72">
        <p v-if="!metricLines.length" class="text-muted-foreground">No metrics returned.</p>
        <div v-for="line in metricLines" v-else :key="line">{{ line }}</div>
      </div>
    </details>

    <p class="text-right text-sm text-muted-foreground">Last updated {{ lastUpdated || "—" }}</p>
  </div>
</template>
