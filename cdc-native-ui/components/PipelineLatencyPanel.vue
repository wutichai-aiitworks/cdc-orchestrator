<script setup lang="ts">
/**
 * Latency tab of a pipeline (ADR 0016): current end-to-end / capture / sink, p95 and max over
 * the range, a sparkline, and the opt-in heartbeat probe for exact numbers.
 */
import { Activity, Info, RefreshCw, Timer } from "@lucide/vue"
import type { CdcProfile } from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"
import {
  METHOD_HELP,
  METHOD_LABEL,
  formatMs,
  latencyLabel,
  latencyTone,
  sparkline,
  type LatencyRange,
  type ProfileLatency,
} from "~/utils/latency"

const props = defineProps<{ profile: CdcProfile }>()
const emit = defineEmits<{ restart: [] }>()

const { fetchProfileLatency, setLatencyProbe } = useLatency()
const toast = useToast()

const RANGES: LatencyRange[] = ["1h", "6h", "24h", "7d", "30d"]
const range = ref<LatencyRange>("1h")
const data = ref<ProfileLatency | null>(null)
const loading = ref(false)
const error = ref("")
const probeBusy = ref(false)
const probeInterval = ref(10)
const restartNeeded = ref(false)

const load = async () => {
  loading.value = true
  try {
    data.value = await fetchProfileLatency(props.profile.id, range.value)
    probeInterval.value = data.value.probe.intervalSecs
    error.value = ""
  } catch (e: any) {
    error.value = formatApiError(e, "Latency could not be loaded")
  } finally {
    loading.value = false
  }
}

watch(range, load)
let timer: ReturnType<typeof setInterval> | undefined
onMounted(() => {
  void load()
  timer = setInterval(() => {
    if (!document.hidden) void load()
  }, 15_000)
})
onBeforeUnmount(() => clearInterval(timer))

const cur = computed(() => data.value?.current ?? null)
const series = computed(() => (data.value?.points ?? []).map((p) => ({ t: p.t, v: p.e2e_ms })))
const W = 600
const H = 120
const spark = computed(() => sparkline(series.value, W, H))
const p95Series = computed(() =>
  data.value?.resolution === "5m" ? sparkline((data.value?.points ?? []).map((p) => ({ t: p.t, v: p.e2e_p95_ms ?? null })), W, H, spark.value.max) : null,
)
const stats = computed(() => {
  const vals = series.value.map((p) => p.v).filter((v): v is number => v !== null).sort((a, b) => a - b)
  if (!vals.length) return null
  const pick = (q: number) => vals[Math.min(vals.length - 1, Math.ceil(q * vals.length) - 1)]
  return { p50: pick(0.5), p95: pick(0.95), max: vals[vals.length - 1] }
})
const split = computed(() => {
  const c = cur.value?.capture_ms ?? 0
  const s = cur.value?.sink_ms ?? 0
  const total = c + s
  return total > 0 ? { capture: (c / total) * 100, sink: (s / total) * 100 } : null
})

const TONE_TEXT: Record<string, string> = {
  good: "text-emerald-600 dark:text-emerald-400",
  warn: "text-amber-600 dark:text-amber-400",
  bad: "text-red-600 dark:text-red-400",
  muted: "text-muted-foreground",
}

const toggleProbe = async (enabled: boolean) => {
  probeBusy.value = true
  try {
    const r = await setLatencyProbe(props.profile.id, { enabled, intervalSecs: probeInterval.value })
    restartNeeded.value = r.restartRequired
    toast.success(enabled ? "Heartbeat probe on." : "Heartbeat probe off.")
    for (const n of r.notes) toast.info(n)
    await load()
  } catch (e: any) {
    toast.error(formatApiError(e, "Probe could not be changed"))
  } finally {
    probeBusy.value = false
  }
}
</script>

<template>
  <div class="space-y-4">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div class="flex rounded-lg border border-border p-0.5" role="radiogroup" aria-label="Range">
        <button
          v-for="r in RANGES"
          :key="r"
          type="button"
          role="radio"
          :aria-checked="range === r"
          class="rounded-md px-3 py-1 text-xs font-medium"
          :class="range === r ? 'bg-muted text-foreground' : 'text-muted-foreground hover:text-foreground'"
          @click="range = r"
        >
          {{ r }}
        </button>
      </div>
      <div class="flex items-center gap-2 text-xs text-muted-foreground">
        <span v-if="cur" class="rounded-full border border-border px-2 py-0.5" :title="METHOD_HELP[cur.method]">
          {{ METHOD_LABEL[cur.method] }}
        </span>
        <AppButton size="icon" title="Reload" :disabled="loading" @click="load">
          <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        </AppButton>
      </div>
    </div>

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <section class="grid grid-cols-2 gap-3 lg:grid-cols-5">
      <div class="stat-card !p-3">
        <p class="text-xs text-muted-foreground">End-to-end now</p>
        <p class="mt-1 font-display text-2xl font-semibold tabular-nums" :class="TONE_TEXT[latencyTone(cur)]">
          {{ latencyLabel(cur) }}
        </p>
        <p class="mt-0.5 text-xs capitalize text-muted-foreground">{{ cur?.status ?? "not measured yet" }}</p>
      </div>
      <div class="stat-card !p-3">
        <p class="text-xs text-muted-foreground">p50 · {{ range }}</p>
        <p class="mt-1 font-display text-2xl font-semibold tabular-nums">{{ formatMs(stats?.p50) }}</p>
      </div>
      <div class="stat-card !p-3">
        <p class="text-xs text-muted-foreground">p95 · {{ range }}</p>
        <p class="mt-1 font-display text-2xl font-semibold tabular-nums">{{ formatMs(stats?.p95) }}</p>
      </div>
      <div class="stat-card !p-3">
        <p class="text-xs text-muted-foreground">Max · {{ range }}</p>
        <p class="mt-1 font-display text-2xl font-semibold tabular-nums">{{ formatMs(stats?.max) }}</p>
      </div>
      <div class="stat-card !p-3">
        <p class="text-xs text-muted-foreground">Throughput</p>
        <p class="mt-1 font-display text-2xl font-semibold tabular-nums">
          {{ cur?.throughput_rps !== null && cur?.throughput_rps !== undefined ? `${cur.throughput_rps}/s` : "—" }}
        </p>
        <p class="mt-0.5 text-xs text-muted-foreground">
          Backlog {{ cur?.lag_records !== null && cur?.lag_records !== undefined ? cur.lag_records.toLocaleString() : "—" }}
        </p>
      </div>
    </section>

    <section class="stat-card">
      <div class="mb-2 flex items-center justify-between text-xs text-muted-foreground">
        <span class="flex items-center gap-1.5"><Activity class="h-3.5 w-3.5" />End-to-end latency{{ data?.resolution === "5m" ? " (5-min p50, dashed p95)" : "" }}</span>
        <span class="tabular-nums">max {{ formatMs(spark.max) }}</span>
      </div>
      <svg v-if="spark.segments.length" :viewBox="`0 0 ${W} ${H}`" preserveAspectRatio="none" class="h-32 w-full" role="img" aria-label="Latency over time">
        <line :x1="0" :x2="W" :y1="H" :y2="H" class="stroke-border" stroke-width="1" />
        <line :x1="0" :x2="W" :y1="H / 2" :y2="H / 2" class="stroke-border" stroke-width="1" stroke-dasharray="3 4" />
        <polyline
          v-for="(seg, i) in p95Series?.segments ?? []"
          :key="`p95-${i}`"
          :points="seg"
          fill="none"
          class="stroke-amber-500/70"
          stroke-width="1.5"
          stroke-dasharray="4 3"
          vector-effect="non-scaling-stroke"
        />
        <polyline
          v-for="(seg, i) in spark.segments"
          :key="i"
          :points="seg"
          fill="none"
          class="stroke-primary"
          stroke-width="2"
          vector-effect="non-scaling-stroke"
        />
      </svg>
      <p v-else class="py-10 text-center text-sm text-muted-foreground">
        {{ loading ? "Loading…" : "No measurements in this range yet. Samples arrive every 15 s while the pipeline runs." }}
      </p>
    </section>

    <section v-if="cur" class="stat-card space-y-2">
      <p class="text-xs font-medium text-muted-foreground">Where the time goes (now)</p>
      <div v-if="split" class="flex h-3 overflow-hidden rounded-full bg-muted">
        <div class="bg-sky-500" :style="{ width: `${split.capture}%` }" :title="`Capture ${formatMs(cur.capture_ms)}`" />
        <div class="bg-violet-500" :style="{ width: `${split.sink}%` }" :title="`Sink ${formatMs(cur.sink_ms)}`" />
      </div>
      <div class="flex flex-wrap gap-4 text-xs">
        <span><span class="mr-1 inline-block h-2 w-2 rounded-full bg-sky-500" />Capture {{ formatMs(cur.capture_ms) }}</span>
        <span><span class="mr-1 inline-block h-2 w-2 rounded-full bg-violet-500" />Sink {{ formatMs(cur.sink_ms) }}</span>
        <span v-if="cur.last_event_at" class="text-muted-foreground">Last change captured {{ new Date(cur.last_event_at).toLocaleString() }}</span>
      </div>
      <p class="flex items-start gap-1.5 text-xs text-muted-foreground"><Info class="mt-0.5 h-3.5 w-3.5 shrink-0" />{{ METHOD_HELP[cur.method] }}</p>
      <p v-if="cur.note" class="text-xs text-amber-700 dark:text-amber-400">{{ cur.note }}</p>
    </section>

    <section class="stat-card space-y-3">
      <div class="flex flex-wrap items-start justify-between gap-3">
        <div>
          <p class="flex items-center gap-1.5 font-medium text-foreground"><Timer class="h-4 w-4" :stroke-width="1.75" />Exact latency (heartbeat probe)</p>
          <p class="mt-1 max-w-2xl text-sm text-muted-foreground">
            Writes a timestamp row into <code class="font-mono text-xs">hyper_cdc.hyper_cdc_heartbeat</code> on the source every
            few seconds and times how long it takes to appear on the target. Needs CREATE and INSERT on the source; also keeps an
            idle Postgres replication slot moving.
          </p>
        </div>
        <div v-if="data && !data.probe.unsupportedReason" class="flex items-center gap-2">
          <label class="flex items-center gap-1.5 text-xs text-muted-foreground">
            every
            <input v-model.number="probeInterval" type="number" min="5" max="300" class="auth-input !h-8 !min-h-8 w-16 text-xs" :disabled="data.probe.enabled" />
            s
          </label>
          <AppButton :variant="data.probe.enabled ? 'secondary' : 'primary'" size="sm" :disabled="probeBusy" @click="toggleProbe(!data.probe.enabled)">
            {{ probeBusy ? "Working…" : data.probe.enabled ? "Turn off" : "Turn on" }}
          </AppButton>
        </div>
      </div>
      <p v-if="data?.probe.unsupportedReason" class="text-xs text-muted-foreground">Not available: {{ data.probe.unsupportedReason }}.</p>
      <p v-if="cur?.probe_error" class="text-xs text-red-600 dark:text-red-400">{{ cur.probe_error }}</p>
      <div v-if="restartNeeded" class="alert-warning !items-center justify-between">
        <span>Restart the pipeline so the connectors capture and replicate the heartbeat table.</span>
        <AppButton size="sm" @click="restartNeeded = false; emit('restart')">Restart now</AppButton>
      </div>
    </section>
  </div>
</template>
