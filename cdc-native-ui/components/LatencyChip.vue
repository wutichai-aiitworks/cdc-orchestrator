<script setup lang="ts">
/** End-to-end latency for one pipeline: coloured value, method and status in the tooltip. */
import { Timer } from "@lucide/vue"
import { METHOD_LABEL, formatMs, latencyLabel, latencyTone, type LatencyCurrent } from "~/utils/latency"

const props = defineProps<{ latency?: LatencyCurrent | null }>()

const TONE: Record<string, string> = {
  good: "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400",
  warn: "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-400",
  bad: "border-red-500/30 bg-red-500/10 text-red-700 dark:text-red-400",
  muted: "border-border bg-muted text-muted-foreground",
}

const title = computed(() => {
  const l = props.latency
  if (!l) return "Latency not measured yet"
  const parts = [`End-to-end: ${formatMs(l.e2e_ms)} (${METHOD_LABEL[l.method]})`]
  if (l.e2e_p95_1h_ms !== null) parts.push(`p95 last hour: ${formatMs(l.e2e_p95_1h_ms)}`)
  if (l.capture_ms !== null) parts.push(`Capture: ${formatMs(l.capture_ms)}`)
  if (l.sink_ms !== null) parts.push(`Sink: ${formatMs(l.sink_ms)}`)
  if (l.lag_records !== null) parts.push(`Backlog: ${l.lag_records.toLocaleString()} records`)
  parts.push(`Status: ${l.status}`)
  if (l.note) parts.push(l.note)
  return parts.join("\n")
})
</script>

<template>
  <span
    class="inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs font-medium tabular-nums"
    :class="TONE[latencyTone(latency)]"
    :title="title"
  >
    <Timer class="h-3 w-3" :stroke-width="2" />
    {{ latencyLabel(latency) }}
    <span v-if="latency?.method === 'heartbeat' || latency?.method === 'trigger_inbox'" class="opacity-70" aria-label="exact">●</span>
  </span>
</template>
