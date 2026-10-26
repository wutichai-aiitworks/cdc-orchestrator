<script setup lang="ts">
import { Radio, RefreshCw, Search, ScrollText, Filter } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"

const props = withDefaults(
  defineProps<{
    /** Connector names / prefixes to filter the connect logs by (e.g. ["cdc-test01"]). */
    connectors: string[]
    /** Log service to stream. Defaults to the Kafka Connect container. */
    service?: string
    /** How many historical lines to prime the view with. */
    tail?: number
    title?: string
  }>(),
  { service: "connect", tail: 500, title: "Processing Log" },
)

const { fetchLogs } = useOperationsM3()

const lines = ref<string[]>([])
const loading = ref(false)
const error = ref("")
const live = ref(true)
const streamState = ref("Connecting…")
const streamConnected = ref(false)
const search = ref("")
/** When true, only lines mentioning one of this pipeline's connectors are shown. */
const onlyThisPipeline = ref(true)
const viewport = ref<HTMLElement | null>(null)
const autoScroll = ref(true)
let eventSource: EventSource | null = null

const connectorTokens = computed(() =>
  props.connectors.map((c) => c.trim()).filter(Boolean),
)

const matchesPipeline = (line: string) =>
  connectorTokens.value.length === 0 ||
  connectorTokens.value.some((token) => line.includes(token))

const filteredLines = computed(() => {
  const q = search.value.trim().toLowerCase()
  return lines.value.filter((line) => {
    if (onlyThisPipeline.value && !matchesPipeline(line)) return false
    if (q && !line.toLowerCase().includes(q)) return false
    return true
  })
})

const lineClass = (line: string) => {
  if (/\b(error|fatal|exception|failed)\b/i.test(line)) return "text-red-300"
  if (/\b(warn|warning)\b/i.test(line)) return "text-amber-300"
  if (/\b(debug|trace)\b/i.test(line)) return "text-blue-300"
  return "text-surface-200"
}

const scrollToLatest = () => {
  requestAnimationFrame(() => {
    if (autoScroll.value && viewport.value) {
      viewport.value.scrollTop = viewport.value.scrollHeight
    }
  })
}

const onScroll = () => {
  if (!viewport.value) return
  const { scrollTop, scrollHeight, clientHeight } = viewport.value
  autoScroll.value = scrollHeight - scrollTop - clientHeight < 48
}

const closeStream = () => {
  eventSource?.close()
  eventSource = null
  streamConnected.value = false
}

const loadSnapshot = async () => {
  loading.value = true
  error.value = ""
  try {
    const res = await fetchLogs(props.service, props.tail)
    lines.value = Array.isArray(res) ? res : []
    scrollToLatest()
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load logs")
  } finally {
    loading.value = false
  }
}

const startStream = () => {
  closeStream()
  if (!live.value) return
  loading.value = true
  error.value = ""
  streamState.value = "Connecting…"
  lines.value = []
  const source = new EventSource(
    `/api/cdc/logs/${encodeURIComponent(props.service)}/stream?tail=${props.tail}`,
  )
  eventSource = source
  source.onopen = () => {
    loading.value = false
    streamConnected.value = true
    streamState.value = "Live"
  }
  source.onmessage = (event) => {
    if (!event.data) return
    // Keep a bounded buffer so long-running streams don't grow unbounded.
    lines.value = [...lines.value, event.data].slice(-2000)
    scrollToLatest()
  }
  source.onerror = () => {
    loading.value = false
    streamConnected.value = false
    streamState.value = source.readyState === EventSource.CLOSED ? "Disconnected" : "Reconnecting…"
  }
}

const toggleLive = () => {
  live.value = !live.value
  if (live.value) startStream()
  else {
    closeStream()
    streamState.value = "Paused"
    loadSnapshot()
  }
}

const refreshNow = () => {
  if (live.value) startStream()
  else loadSnapshot()
}

onMounted(startStream)
onUnmounted(closeStream)
</script>

<template>
  <AppCard class="!p-0 overflow-hidden">
    <!-- Header -->
    <div class="flex flex-wrap items-center justify-between gap-3 border-b border-border px-4 py-3">
      <div class="flex items-center gap-2">
        <ScrollText class="h-4 w-4 text-brand-400" :stroke-width="1.75" />
        <h2 class="text-sm font-semibold text-foreground">{{ title }}</h2>
        <span
          :class="[
            'inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-xs font-medium',
            streamConnected
              ? 'border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400'
              : 'border-border bg-muted text-muted-foreground',
          ]"
        >
          <Radio class="h-3 w-3" :class="{ 'animate-pulse': streamConnected }" :stroke-width="2" />
          {{ live ? streamState : "Paused" }}
        </span>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <!-- Filter to this pipeline -->
        <button
          type="button"
          :title="onlyThisPipeline ? 'Showing only this pipeline’s connectors' : 'Showing all connect logs'"
          class="inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium transition-colors"
          :class="onlyThisPipeline
            ? 'border-brand-500/40 bg-brand-500/10 text-brand-500'
            : 'border-border bg-muted text-muted-foreground hover:text-foreground'"
          @click="onlyThisPipeline = !onlyThisPipeline"
        >
          <Filter class="h-3.5 w-3.5" :stroke-width="1.75" />
          {{ onlyThisPipeline ? "This pipeline" : "All connectors" }}
        </button>

        <!-- Search -->
        <div class="relative">
          <Search class="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" :stroke-width="1.75" />
          <AppInput v-model="search" input-class="h-8 w-44 pl-8 text-xs" placeholder="Filter…" />
        </div>

        <!-- Live toggle -->
        <label class="flex items-center gap-1.5 text-xs text-muted-foreground">
          <input v-model="live" type="checkbox" class="h-3.5 w-3.5 accent-brand-500" @change="toggleLive" />
          Live
        </label>

        <!-- Refresh -->
        <AppButton size="icon" title="Refresh" :disabled="loading" @click="refreshNow">
          <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        </AppButton>
      </div>
    </div>

    <!-- Connector chips -->
    <div v-if="connectorTokens.length" class="flex flex-wrap gap-1.5 border-b border-border/60 px-4 py-2">
      <span
        v-for="c in connectorTokens"
        :key="c"
        class="rounded-md border border-border bg-muted px-2 py-0.5 font-mono text-xs text-muted-foreground"
      >{{ c }}</span>
    </div>

    <!-- Error -->
    <AppStateBlock v-if="error" kind="error" :message="error" class="m-3" />

    <!-- Log viewport -->
    <div
      ref="viewport"
      class="h-80 overflow-auto bg-surface-950 p-4 font-mono text-xs leading-6"
      aria-live="polite"
      @scroll="onScroll"
    >
      <p v-if="loading && !filteredLines.length" class="text-surface-400">Loading logs…</p>
      <p v-else-if="!filteredLines.length" class="text-surface-400">
        No log lines yet{{ onlyThisPipeline ? " for this pipeline" : "" }}.
      </p>
      <div
        v-for="(line, index) in filteredLines"
        v-else
        :key="index"
        class="whitespace-pre-wrap break-all"
        :class="lineClass(line)"
      >
        <span class="mr-3 select-none text-surface-600">{{ String(index + 1).padStart(4, "0") }}</span>{{ line }}
      </div>
    </div>

    <!-- Footer -->
    <div class="flex items-center justify-between border-t border-border px-4 py-2 text-xs text-muted-foreground">
      <span>{{ filteredLines.length }} visible lines</span>
      <button
        v-if="!autoScroll"
        type="button"
        class="rounded-md px-2 py-0.5 text-brand-500 hover:bg-brand-500/10"
        @click="autoScroll = true; scrollToLatest()"
      >
        ↓ Jump to latest
      </button>
    </div>
  </AppCard>
</template>
