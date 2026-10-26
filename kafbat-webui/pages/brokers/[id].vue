<script setup lang="ts">
import { Cable, HardDrive, Info, Layers, Pencil, RefreshCw, Server } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import { formatBytes } from "~/utils/bytes"
import type { BrokerConfigEntry } from "~/composables/useKafkaAdmin"

const route = useRoute()
const toast = useToast()
const { fetchBroker, fetchBrokerLogdirs, fetchBrokerConfigs, fetchBrokerMetrics, updateBrokerConfig } = useKafkaAdmin()

const id = computed(() => String(route.params.id || ""))
const tab = computed(() => {
  const value = String(route.query.tab || "logdirs")
  return ["logdirs", "configs", "metrics"].includes(value)
    ? (value as "logdirs" | "configs" | "metrics")
    : "logdirs"
})

const loading = ref(false)
const metricsLoading = ref(false)
const saving = ref(false)
const error = ref("")
const searchQuery = ref("")
const expandedDir = ref("")
const editingName = ref("")
const editingValue = ref("")
const detail = ref<Awaited<ReturnType<typeof fetchBroker>> | null>(null)
const logdirs = ref<Array<{
  name: string
  error?: string | null
  topics_count: number
  partitions_count: number
  size: number
  topics: Array<{ name: string; partitions: Array<{ partition: number; size: number }> }>
}>>([])
const configs = ref<BrokerConfigEntry[]>([])
const metrics = ref<Awaited<ReturnType<typeof fetchBrokerMetrics>> | null>(null)

const normalizeConfigKey = (value: string) => value.toLowerCase().replace(/[._-]+/g, ".")

const configDisplayName = (row: BrokerConfigEntry) => row.display_name || row.name
const configDisplayValue = (row: BrokerConfigEntry) => row.display_value || row.value || "—"
const configKeyTitle = (row: BrokerConfigEntry) => {
  const display = configDisplayName(row)
  return display === row.name ? display : `${display} (${row.name})`
}

const filteredConfigs = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  if (!q) return configs.value
  const nq = normalizeConfigKey(q)
  return configs.value.filter((row) => {
    const name = row.name.toLowerCase()
    const display = configDisplayName(row).toLowerCase()
    const value = (row.value || "").toLowerCase()
    const displayValue = (row.display_value || "").toLowerCase()
    return (
      name.includes(q) ||
      display.includes(q) ||
      value.includes(q) ||
      displayValue.includes(q) ||
      normalizeConfigKey(row.name).includes(nq) ||
      normalizeConfigKey(display).includes(nq)
    )
  })
})

const formatConfigSource = (source?: string | null) => {
  const raw = String(source || "").trim()
  const key = raw.toUpperCase().replace(/\s+/g, "_")
  if (["STATIC_BROKER_CONFIG", "STATIC BROKER CONFIG"].includes(raw.toUpperCase()) || raw === "Static broker config") {
    return "Static broker config"
  }
  if (["DEFAULT_CONFIG", "DEFAULT CONFIG"].includes(key) || raw === "Default config") {
    return "Default config"
  }
  if (["DYNAMIC_CLUSTER_CONFIG", "DYNAMIC_BROKER_CONFIG", "DYNAMIC_DEFAULT_BROKER_CONFIG"].includes(key)) {
    return "Static broker config"
  }
  return raw || "Default config"
}

const metricsEditorText = computed(() => {
  if (!metrics.value) return "Metrics data not available"
  return JSON.stringify(
    {
      segmentSize: metrics.value.segment_size,
      segmentCount: metrics.value.segment_count,
      metrics: metrics.value.metrics,
      node: metrics.value.node,
      health: metrics.value.health,
    },
    null,
    2
  )
})

const load = async () => {
  loading.value = true
  error.value = ""
  try {
    const [broker, dirs, cfg] = await Promise.all([
      fetchBroker(id.value),
      fetchBrokerLogdirs(id.value),
      fetchBrokerConfigs(id.value),
    ])
    detail.value = broker
    logdirs.value = dirs.items || broker.logdirs || []
    configs.value = cfg.items || broker.configs || []
    if (tab.value === "metrics") await loadMetrics()
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load broker")
  } finally {
    loading.value = false
  }
}

const loadMetrics = async () => {
  metricsLoading.value = true
  try {
    metrics.value = await fetchBrokerMetrics(id.value)
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load broker metrics")
  } finally {
    metricsLoading.value = false
  }
}

const startEdit = (row: BrokerConfigEntry) => {
  editingName.value = row.name
  editingValue.value = row.value
}

const cancelEdit = () => {
  editingName.value = ""
  editingValue.value = ""
}

const saveConfig = async () => {
  if (!editingName.value) return
  saving.value = true
  try {
    await updateBrokerConfig(id.value, { name: editingName.value, value: editingValue.value })
    toast.success(`Updated ${editingName.value}`)
    cancelEdit()
    const cfg = await fetchBrokerConfigs(id.value)
    configs.value = cfg.items || []
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to update config"))
  } finally {
    saving.value = false
  }
}

watch(tab, async (value) => {
  if (value === "metrics" && !metrics.value) await loadMetrics()
})

onMounted(load)
</script>

<template>
  <div class="page-container max-w-6xl space-y-4">
    <AppPageHeader :title="`Broker ${id}`" description="Log directories, cluster configs, and live broker metrics.">
      <AppButton as="link" to="/brokers">Back to brokers</AppButton>
      <AppButton :disabled="loading" @click="load">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <section class="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
      <AppMetricCard label="Segment Size" :value="formatBytes(detail?.segment_size ?? detail?.broker.segment_size)" tone="brand">
        <HardDrive class="h-4 w-4" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="Segment Count"
        :value="String(detail?.segment_count ?? detail?.broker.segment_count ?? 0)"
        tone="info"
      >
        <Layers class="h-4 w-4" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard label="Port" :value="detail?.broker.port == null ? '—' : String(detail.broker.port)" tone="neutral">
        <Cable class="h-4 w-4" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard label="Host" :value="detail?.broker.host || '—'" tone="success">
        <Server class="h-4 w-4" :stroke-width="1.75" />
      </AppMetricCard>
    </section>

    <BrokerTabs :broker-id="id" :active="tab" />

    <template v-if="tab === 'logdirs'">
      <section class="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <AppMetricCard label="Directories" :value="loading ? '…' : String(logdirs.length)" tone="brand">
          <HardDrive class="h-4 w-4" :stroke-width="1.75" />
        </AppMetricCard>
        <AppMetricCard
          label="Total Size"
          :value="loading ? '…' : formatBytes(logdirs.reduce((s, d) => s + (d.size || 0), 0))"
          tone="info"
        >
          <HardDrive class="h-4 w-4" :stroke-width="1.75" />
        </AppMetricCard>
        <AppMetricCard
          label="Total Partitions"
          :value="loading ? '…' : String(logdirs.reduce((s, d) => s + (d.partitions_count || 0), 0))"
          tone="neutral"
        >
          <Layers class="h-4 w-4" :stroke-width="1.75" />
        </AppMetricCard>
      </section>

    <AppCard>
      <AppTable>
        <thead>
          <tr>
            <th>Name</th>
            <th>Error</th>
            <th>Topics</th>
            <th>Partitions</th>
            <th>Size</th>
          </tr>
        </thead>
        <tbody>
          <template v-for="dir in logdirs" :key="dir.name">
            <tr class="cursor-pointer hover:bg-accent/60" @click="expandedDir = expandedDir === dir.name ? '' : dir.name">
              <td class="font-mono text-sm">{{ dir.name }}</td>
              <td>
                <span v-if="dir.error" class="badge-error">{{ dir.error }}</span>
                <span v-else class="text-muted-foreground">—</span>
              </td>
              <td>{{ dir.topics_count }}</td>
              <td>{{ dir.partitions_count }}</td>
              <td class="font-mono text-sm">{{ formatBytes(dir.size) }}</td>
            </tr>
            <tr v-if="expandedDir === dir.name">
              <td colspan="5" class="bg-surface-800/40">
                <AppTable>
                  <thead>
                    <tr>
                      <th>Topic</th>
                      <th>Partitions</th>
                      <th>Size</th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr v-for="topic in dir.topics" :key="topic.name">
                      <td class="font-mono text-sm">{{ topic.name }}</td>
                      <td>{{ topic.partitions.length }}</td>
                      <td class="font-mono text-sm">{{ formatBytes(topic.partitions.reduce((sum, part) => sum + (part.size || 0), 0)) }}</td>
                    </tr>
                  </tbody>
                </AppTable>
              </td>
            </tr>
          </template>
          <tr v-if="!loading && !logdirs.length">
            <td colspan="5"><AppStateBlock kind="empty" message="Log dir data not available" /></td>
          </tr>
        </tbody>
        </AppTable>
    </AppCard>
    </template>

    <AppCard v-else-if="tab === 'configs'" class="space-y-3">
      <div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <AppResultsHeader
          label="Total keys"
          :total="configs.length"
          :loading="loading"
        />
        <p v-if="searchQuery.trim()" class="text-sm text-muted-foreground">
          Showing {{ filteredConfigs.length }} of {{ configs.length }}
        </p>
      </div>
      <AppInput
        v-model="searchQuery"
        placeholder="Search by Key or Value"
        aria-label="Search by Key or Value"
      />
      <AppInfoHint aria-label="Config key naming">
        <template #default>
          Keys use Kafka dotted names (<span class="font-mono">auto.create.topics.enable</span>).
          Redpanda names use underscores (<span class="font-mono">auto_create_topics_enabled</span>).
        </template>
      </AppInfoHint>
      <AppTable table-class="table-fixed">
        <thead>
          <tr>
            <th class="w-[22%]">Key</th>
            <th>Value</th>
            <th class="w-[18%]">
              <span class="inline-flex items-center gap-1">
                Source
                <Info
                  class="h-3.5 w-3.5 text-muted-foreground"
                  :stroke-width="1.75"
                  title="Static broker config is set on this cluster or node. Default config is the Redpanda default."
                />
              </span>
            </th>
            <th class="w-[7.5rem]">Action</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="row in filteredConfigs" :key="row.name">
            <td class="max-w-0 truncate font-mono text-sm" :title="configKeyTitle(row)">{{ configDisplayName(row) }}</td>
            <td class="max-w-0">
              <div v-if="editingName === row.name" class="flex flex-wrap items-center gap-2">
                <AppInput v-model="editingValue" input-class="min-w-[12rem]" @enter="saveConfig" />
                <AppButton variant="primary" size="sm" :disabled="saving" @click="saveConfig">Save</AppButton>
                <AppButton size="sm" @click="cancelEdit">Cancel</AppButton>
              </div>
              <span v-else class="block truncate font-mono text-sm" :title="row.value || configDisplayValue(row)">{{ configDisplayValue(row) }}</span>
            </td>
            <td class="whitespace-nowrap text-muted-foreground">{{ formatConfigSource(row.source) }}</td>
            <td>
              <AppButton
                v-if="editingName !== row.name"
                size="sm"
                variant="primary"
                :title="`Edit ${configDisplayName(row)}`"
                @click="startEdit(row)"
              >
                <Pencil class="h-3.5 w-3.5" :stroke-width="1.75" />
                Edit
              </AppButton>
            </td>
          </tr>
          <tr v-if="!loading && !filteredConfigs.length">
            <td colspan="4"><AppStateBlock kind="empty" message="No configs returned for this broker." /></td>
          </tr>
        </tbody>
      </AppTable>
    </AppCard>

    <AppCard v-else>
      <AppStateBlock v-if="metricsLoading && !metrics" kind="loading" message="Loading metrics..." />
      <pre v-else class="code-block max-h-[70vh] overflow-auto whitespace-pre-wrap">{{ metricsEditorText }}</pre>
    </AppCard>
  </div>
</template>
