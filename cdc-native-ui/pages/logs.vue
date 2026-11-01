<script setup lang="ts">
import { Download, Radio, RefreshCw, ScrollText, Search, SlidersHorizontal } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"

const { fetchLogServices, fetchLogs, fetchLoggers, updateLogger } = useOperationsM3()
const toast = useToast()
const tab = ref<"logs" | "levels">("logs")
const services = ref<Record<string, { service?: string; container?: string }>>({})
const selectedService = ref("")
const lines = ref<string[]>([])
const loggers = ref<Record<string, any>>({})
const tail = ref(200)
const search = ref("")
const loading = ref(false)
const followLive = ref(true)
const streamConnected = ref(false)
const streamState = ref("Connecting…")
const error = ref("")
const initialized = ref(false)
const logViewport = ref<HTMLElement | null>(null)
let eventSource: EventSource | null = null

const serviceNames = computed(() => Object.keys(services.value).sort())
const filteredLines = computed(() => {
  const query = search.value.trim().toLowerCase()
  return query ? lines.value.filter((line) => line.toLowerCase().includes(query)) : lines.value
})
const loggerEntries = computed(() => {
  const source = (loggers.value as any)?.loggers || loggers.value || {}
  return Object.entries(source)
    .map(([name, value]: [string, any]) => ({
      name,
      level: String(value?.level || value || "INFO").toUpperCase(),
    }))
    .sort((a, b) => a.name.localeCompare(b.name))
})

const lineClass = (line: string) => {
  if (/\b(error|fatal|exception|failed)\b/i.test(line)) return "text-red-300"
  if (/\b(warn|warning)\b/i.test(line)) return "text-amber-300"
  if (/\b(debug|trace)\b/i.test(line)) return "text-blue-300"
  return "text-surface-200"
}

const loadSnapshot = async () => {
  if (!selectedService.value) return
  loading.value = true
  error.value = ""
  try {
    const response = await fetchLogs(selectedService.value, tail.value)
    lines.value = Array.isArray(response) ? response : []
  } catch (e: any) {
    lines.value = []
    error.value = formatApiError(e, "Failed to load service logs")
  } finally {
    loading.value = false
  }
}

const closeStream = () => {
  eventSource?.close()
  eventSource = null
  streamConnected.value = false
}

const scrollToLatest = () => {
  requestAnimationFrame(() => {
    if (logViewport.value) logViewport.value.scrollTop = logViewport.value.scrollHeight
  })
}

const startStream = () => {
  closeStream()
  if (!selectedService.value || !followLive.value) return

  loading.value = true
  error.value = ""
  streamState.value = "Connecting…"
  lines.value = []
  const source = new EventSource(
    `/api/cdc/logs/${encodeURIComponent(selectedService.value)}/stream?tail=${tail.value}`
  )
  eventSource = source

  source.onopen = () => {
    loading.value = false
    streamConnected.value = true
    streamState.value = "Live"
  }
  source.onmessage = (event) => {
    if (!event.data) return
    lines.value = [...lines.value, event.data].slice(-tail.value)
    scrollToLatest()
  }
  source.onerror = () => {
    loading.value = false
    streamConnected.value = false
    streamState.value = source.readyState === EventSource.CLOSED ? "Disconnected" : "Reconnecting…"
  }
}

const refreshFeed = async () => {
  if (followLive.value) startStream()
  else await loadSnapshot()
}

const load = async () => {
  loading.value = true
  error.value = ""
  try {
    const [serviceResponse, loggerResponse] = await Promise.all([fetchLogServices(), fetchLoggers()])
    services.value = serviceResponse || {}
    loggers.value = loggerResponse || {}
    if (!selectedService.value && serviceNames.value.length) selectedService.value = serviceNames.value[0]
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load log services")
  } finally {
    loading.value = false
  }
}

const setLevel = async (logger: string, level: string) => {
  try {
    await updateLogger(logger, level)
    toast.success(`${logger} changed to ${level}.`)
    loggers.value = await fetchLoggers()
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to update logger level")
  }
}

const download = () => {
  const blob = new Blob([`${filteredLines.value.join("\n")}\n`], { type: "text/plain" })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement("a")
  anchor.href = url
  anchor.download = `${selectedService.value || "cdc"}-${new Date().toISOString()}.log`
  anchor.click()
  URL.revokeObjectURL(url)
}

watch([selectedService, tail], () => {
  if (initialized.value) refreshFeed()
})
watch(followLive, () => {
  if (!initialized.value) return
  if (followLive.value) startStream()
  else {
    closeStream()
    streamState.value = "Paused"
    loadSnapshot()
  }
})
onMounted(async () => {
  await load()
  initialized.value = true
  await refreshFeed()
})
onUnmounted(() => {
  closeStream()
})
</script>

<template>
  <div class="page-container">
    <AppPageHeader
      title="Logs"
      description="Troubleshoot the CDC stack with searchable service logs and runtime logger controls."
    >
      <label class="flex items-center gap-2 text-sm text-muted-foreground">
        <input v-model="followLive" type="checkbox" class="h-4 w-4 accent-brand-500" />
        Live SSE feed
      </label>
      <AppButton :disabled="loading" @click="refreshFeed">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        {{ followLive ? "Reconnect" : "Refresh Snapshot" }}
      </AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <div class="inline-flex rounded-xl border border-border bg-muted p-1">
      <button
        type="button"
        :class="['rounded-lg px-4 py-2 text-sm font-medium', tab === 'logs' ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground']"
        @click="tab = 'logs'"
      >
        Service Logs
      </button>
      <button
        type="button"
        :class="['rounded-lg px-4 py-2 text-sm font-medium', tab === 'levels' ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground']"
        @click="tab = 'levels'"
      >
        Logger Levels
      </button>
    </div>

    <section v-if="tab === 'logs'" class="grid grid-cols-1 gap-4 xl:grid-cols-[260px,minmax(0,1fr)]">
      <aside class="stat-card h-fit">
        <h2 class="section-title mb-3">Services</h2>
        <AppEmptyState
          v-if="!serviceNames.length"
          title="No Log Services"
          description="The orchestrator did not return any managed log sources."
        >
          <template #icon><ScrollText class="h-5 w-5" /></template>
        </AppEmptyState>
        <div v-else class="space-y-2">
          <button
            v-for="name in serviceNames"
            :key="name"
            type="button"
            class="list-option"
            :class="{ 'list-option-active': selectedService === name }"
            @click="selectedService = name"
          >
            <p class="text-sm font-medium">{{ name }}</p>
            <p class="mt-1 truncate font-mono text-sm text-muted-foreground">{{ services[name]?.container || "—" }}</p>
          </button>
        </div>
      </aside>

      <section class="stat-card min-w-0">
        <div class="mb-3 flex flex-wrap items-center justify-between gap-3">
          <div>
            <div class="flex items-center gap-2">
              <h2 class="section-title">{{ selectedService || "Service Logs" }}</h2>
              <span
                :class="[
                  'inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-sm font-medium',
                  streamConnected
                    ? 'border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400'
                    : 'border-border bg-muted text-muted-foreground',
                ]"
              >
                <Radio class="h-3 w-3" :class="{ 'animate-pulse': streamConnected }" :stroke-width="2" />
                {{ followLive ? streamState : "Snapshot" }}
              </span>
            </div>
            <AppInfoHint :text="`${filteredLines.length} visible lines · retaining latest ${tail}`" />
          </div>
          <div class="flex flex-wrap items-center gap-2">
            <div class="relative">
              <Search class="pointer-events-none absolute left-3 top-3 h-4 w-4 text-muted-foreground" :stroke-width="1.75" />
              <AppInput v-model="search" input-class="w-56 pl-9" placeholder="Filter logs..." />
            </div>
            <AppSelect v-model="tail" select-class="w-28" aria-label="Log line count">
              <option :value="100">100 Lines</option>
              <option :value="200">200 Lines</option>
              <option :value="500">500 Lines</option>
              <option :value="1000">1,000 Lines</option>
            </AppSelect>
            <AppButton size="icon" title="Download Visible Logs" :disabled="!filteredLines.length" @click="download">
              <Download class="h-4 w-4" :stroke-width="1.75" />
            </AppButton>
          </div>
        </div>
        <div
          ref="logViewport"
          class="h-[60vh] overflow-auto rounded-xl border border-surface-700 bg-surface-950 p-4 font-mono text-sm leading-6"
          aria-live="polite"
        >
          <p v-if="loading" class="text-surface-400">Loading logs…</p>
          <p v-else-if="!filteredLines.length" class="text-surface-400">No matching log lines.</p>
          <div v-for="(line, index) in filteredLines" v-else :key="index" :class="lineClass(line)">
            <span class="mr-4 select-none text-surface-600">{{ String(index + 1).padStart(4, "0") }}</span>{{ line }}
          </div>
        </div>
      </section>
    </section>

    <section v-else class="stat-card">
      <div class="mb-4">
        <AppSectionTitle title="Kafka Connect Logger Levels" hint="Temporarily increase verbosity for targeted troubleshooting. Avoid DEBUG in normal production operation." />
      </div>
      <AppEmptyState
        v-if="!loggerEntries.length"
        title="No Runtime Loggers"
        description="Kafka Connect did not return logger configuration."
      >
        <template #icon><SlidersHorizontal class="h-5 w-5" /></template>
      </AppEmptyState>
      <AppTable v-else>
        <thead><tr><th>Logger</th><th>Current Level</th><th>Change Level</th></tr></thead>
        <tbody>
          <tr v-for="logger in loggerEntries" :key="logger.name">
            <td class="break-all font-mono">{{ logger.name }}</td>
            <td><AppStatusBadge :status="logger.level" /></td>
            <td>
              <AppSelect
                :model-value="logger.level"
                select-class="w-32"
                :aria-label="`Log level for ${logger.name}`"
                @update:model-value="setLevel(logger.name, String($event))"
              >
                <option value="ERROR">ERROR</option>
                <option value="WARN">WARN</option>
                <option value="INFO">INFO</option>
                <option value="DEBUG">DEBUG</option>
                <option value="TRACE">TRACE</option>
              </AppSelect>
            </td>
          </tr>
        </tbody>
      </AppTable>
    </section>
  </div>
</template>
