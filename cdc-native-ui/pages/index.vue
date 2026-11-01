<script setup lang="ts">
import { Activity, ArrowRight, Cable, Check, List, RefreshCw, Server, Workflow } from "@lucide/vue"
import type { CdcProfile } from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"

const { fetchEngineStatus } = useControlPlane()
const { fetchConnectors } = useConnectorsM2()
const { fetchTopics } = useKafkaM1()
const { fetchProfiles } = useProfilesM3()

const engine = ref<EngineStatus | null>(null)
const connectors = ref<Record<string, any>>({})
const topics = ref(0)
const profiles = ref<CdcProfile[]>([])
const loading = ref(false)
const error = ref("")
const lastUpdated = ref("")

const connectorEntries = computed(() => Object.entries(connectors.value))
const runningConnectors = computed(
  () => connectorEntries.value.filter(([, value]) => String(value?.status?.connector?.state || "").toUpperCase() === "RUNNING").length
)
const failedConnectors = computed(
  () => connectorEntries.value.filter(([, value]) => String(value?.status?.connector?.state || "").toUpperCase() === "FAILED").length
)
const runningServices = computed(
  () => engine.value?.services?.filter((service) => service.state.toLowerCase().includes("running")).length || 0
)

const refresh = async () => {
  loading.value = true
  error.value = ""
  const results = await Promise.allSettled([
    fetchEngineStatus(),
    fetchConnectors(),
    fetchTopics({ page: 1, page_size: 1 }),
    fetchProfiles(),
  ])
  if (results[0].status === "fulfilled") engine.value = results[0].value
  if (results[1].status === "fulfilled") connectors.value = results[1].value.connectors || {}
  if (results[2].status === "fulfilled") topics.value = results[2].value.total
  if (results[3].status === "fulfilled") profiles.value = results[3].value
  const failed = results.find((result) => result.status === "rejected")
  if (failed?.status === "rejected") error.value = formatApiError(failed.reason, "Some overview data could not be loaded")
  lastUpdated.value = new Date().toLocaleTimeString()
  loading.value = false
}

onMounted(refresh)
</script>

<template>
  <div class="page-container !space-y-6">
    <AppPageHeader
      title="Overview"
      description="Operate database change capture, Kafka Connect, and stream health from one workspace."
    >
      <AppButton :disabled="loading" @click="refresh">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
      <AppButton as="link" to="/pipelines" variant="primary">
        <Workflow class="h-4 w-4" :stroke-width="1.75" />
        Create Pipeline
      </AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <section class="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
      <AppMetricCard
        label="Runtime Services"
        :value="`${runningServices} / ${engine?.services?.length || 0}`"
        helper="Managed services running"
        :tone="runningServices === (engine?.services?.length || -1) ? 'success' : 'warning'"
      >
        <Server class="h-4 w-4" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="CDC Pipelines"
        :value="profiles.length"
        :helper="`${profiles.filter((profile) => profile.enabled).length} enabled`"
        tone="brand"
      >
        <Workflow class="h-4 w-4" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="Connectors"
        :value="connectorEntries.length"
        :helper="`${runningConnectors} running · ${failedConnectors} failed`"
        :tone="failedConnectors ? 'danger' : 'success'"
      >
        <Cable class="h-4 w-4" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard label="Kafka Topics" :value="topics" helper="Available event streams" tone="info">
        <List class="h-4 w-4" :stroke-width="1.75" />
      </AppMetricCard>
    </section>

    <section v-if="!profiles.length" class="rounded-2xl border border-brand-500/25 bg-brand-500/5 p-6">
      <div class="grid grid-cols-1 items-center gap-6 lg:grid-cols-[minmax(0,1fr),auto]">
        <div>
          <span class="badge-neutral mb-3">Getting Started</span>
          <div class="flex items-center gap-2">
            <h2 class="text-xl font-semibold text-foreground">Create your first production CDC pipeline</h2>
            <AppInfoHint
              text="The guided setup tests database access, chooses capture scope, and creates the Debezium connector configuration for you."
            />
          </div>
          <div class="mt-5 grid grid-cols-1 gap-3 sm:grid-cols-3">
            <div class="flex items-start gap-3">
              <span class="flex h-7 w-7 items-center justify-center rounded-full bg-primary text-sm font-semibold text-primary-foreground">1</span>
              <div class="flex items-center gap-1">
                <p class="text-sm font-medium">Connect Source</p>
                <AppInfoHint text="Enter database details" />
              </div>
            </div>
            <div class="flex items-start gap-3">
              <span class="flex h-7 w-7 items-center justify-center rounded-full bg-primary text-sm font-semibold text-primary-foreground">2</span>
              <div class="flex items-center gap-1">
                <p class="text-sm font-medium">Choose Scope</p>
                <AppInfoHint text="Select schemas and tables" />
              </div>
            </div>
            <div class="flex items-start gap-3">
              <span class="flex h-7 w-7 items-center justify-center rounded-full bg-primary text-sm font-semibold text-primary-foreground">3</span>
              <div class="flex items-center gap-1">
                <p class="text-sm font-medium">Start Capture</p>
                <AppInfoHint text="Enable and monitor" />
              </div>
            </div>
          </div>
        </div>
        <AppButton as="link" to="/pipelines" variant="primary">
          Start Guided Setup <ArrowRight class="h-4 w-4" :stroke-width="1.75" />
        </AppButton>
      </div>
    </section>

    <section class="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,2fr),minmax(300px,1fr)]">
      <div class="stat-card">
        <div class="mb-4 flex items-center justify-between gap-3">
          <div>
            <AppSectionTitle title="Connector Activity" hint="Current state of configured Kafka Connect workloads." />
          </div>
          <NuxtLink to="/connectors" class="btn-ghost">Manage</NuxtLink>
        </div>
        <AppEmptyState
          v-if="!connectorEntries.length"
          title="No Connectors Configured"
          description="Create a CDC pipeline or add a connector from a validated template."
        >
          <template #icon><Cable class="h-4 w-4" :stroke-width="1.75" /></template>
        </AppEmptyState>
        <AppTable v-else>
          <thead><tr><th>Connector</th><th>State</th><th>Tasks</th></tr></thead>
          <tbody>
            <tr v-for="[name, value] in connectorEntries.slice(0, 8)" :key="name">
              <td class="font-mono">{{ name }}</td>
              <td><AppStatusBadge :status="value?.status?.connector?.state" /></td>
              <td>{{ value?.status?.tasks?.length || 0 }}</td>
            </tr>
          </tbody>
        </AppTable>
      </div>

      <div class="stat-card">
        <div class="mb-4">
          <AppSectionTitle title="System Readiness" hint="Essential checks before starting capture." />
        </div>
        <div class="space-y-3">
          <div class="flex items-center gap-3 rounded-xl border border-border bg-muted/40 p-3">
            <span :class="['flex h-8 w-8 items-center justify-center rounded-full', engine ? 'bg-emerald-500/10 text-emerald-600' : 'bg-muted text-muted-foreground']">
              <Check class="h-4 w-4" :stroke-width="2" />
            </span>
            <div><p class="text-sm font-medium">Control Plane</p><p class="text-sm text-muted-foreground">{{ engine ? "Connected" : "Unavailable" }}</p></div>
          </div>
          <div class="flex items-center gap-3 rounded-xl border border-border bg-muted/40 p-3">
            <span :class="['flex h-8 w-8 items-center justify-center rounded-full', runningServices ? 'bg-emerald-500/10 text-emerald-600' : 'bg-muted text-muted-foreground']">
              <Activity class="h-4 w-4" :stroke-width="1.75" />
            </span>
            <div><p class="text-sm font-medium">CDC Runtime</p><p class="text-sm text-muted-foreground">{{ runningServices ? `${runningServices} services running` : "Not running" }}</p></div>
          </div>
          <div class="flex items-center gap-3 rounded-xl border border-border bg-muted/40 p-3">
            <span :class="['flex h-8 w-8 items-center justify-center rounded-full', profiles.length ? 'bg-emerald-500/10 text-emerald-600' : 'bg-muted text-muted-foreground']">
              <Workflow class="h-4 w-4" :stroke-width="1.75" />
            </span>
            <div><p class="text-sm font-medium">Pipeline Configuration</p><p class="text-sm text-muted-foreground">{{ profiles.length ? `${profiles.length} configured` : "Setup required" }}</p></div>
          </div>
        </div>
      </div>
    </section>

    <p class="text-right text-sm text-muted-foreground">Last updated {{ lastUpdated || "—" }}</p>
  </div>
</template>
