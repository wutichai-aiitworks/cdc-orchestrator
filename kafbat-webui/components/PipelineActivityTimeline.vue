<script setup lang="ts">
/**
 * Pipeline activity as a timeline, for finding when and why something went wrong.
 *
 * Merges the pipeline's own history (lifecycle events, capped at 200 per pipeline) with
 * the audit trail (connector state changes from the watcher, failed operations, and
 * lifecycle events older than the history cap), filtered to a time range on the server.
 * A histogram over the range (clock-aligned bars, 1 min … 1 day) shows where events — and failures — cluster; click a bar
 * to zoom into it.
 */
import {
  AlertTriangle,
  CircleCheck,
  CircleX,
  Info,
  Plug,
  RefreshCw,
  Search,
  Workflow,
} from "@lucide/vue"
import type { AuditEventItem } from "~/composables/useOperationsM3"
import type { CdcProfile } from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"
import { auditActionLabel, eventLabel } from "~/utils/pipelineEvents"

const props = defineProps<{ profile: CdcProfile }>()

const { fetchAuditEvents } = useOperationsM3()

const AUDIT_LIMIT = 1000
const MAX_BARS = 120

type Preset = "1h" | "6h" | "24h" | "7d" | "30d" | "custom"
const PRESETS: { key: Exclude<Preset, "custom">; label: string; ms: number }[] = [
  { key: "1h", label: "1 hour", ms: 3_600_000 },
  { key: "6h", label: "6 hours", ms: 6 * 3_600_000 },
  { key: "24h", label: "24 hours", ms: 24 * 3_600_000 },
  { key: "7d", label: "7 days", ms: 7 * 86_400_000 },
  { key: "30d", label: "30 days", ms: 30 * 86_400_000 },
]

type Level = "error" | "warn" | "ok" | "info"
type Category = "pipeline" | "connector" | "operation"
type Kind = "all" | "problems" | "pipeline" | "connector"

type Row = {
  key: string
  at: number
  iso: string
  title: string
  detail: string
  level: Level
  category: Category
  actor?: string
}

// ── Range ───────────────────────────────────────────────────────────────────

/** `<input type="datetime-local">` value in the browser's time zone. */
const toLocalInput = (ms: number) => {
  const d = new Date(ms)
  return new Date(d.getTime() - d.getTimezoneOffset() * 60_000).toISOString().slice(0, 16)
}

const preset = ref<Preset>("24h")
const fromInput = ref("")
const toInput = ref("")
const kind = ref<Kind>("all")
const search = ref("")

const applyPreset = (key: Exclude<Preset, "custom">) => {
  const ms = PRESETS.find((p) => p.key === key)!.ms
  const now = Date.now()
  preset.value = key
  fromInput.value = toLocalInput(now - ms)
  toInput.value = toLocalInput(now)
  void load()
}

const range = computed(() => {
  const from = fromInput.value ? new Date(fromInput.value).getTime() : NaN
  // datetime-local has minute precision: include the whole "to" minute.
  const to = toInput.value ? new Date(toInput.value).getTime() + 59_999 : NaN
  return { from, to, valid: Number.isFinite(from) && Number.isFinite(to) && from < to }
})

const tzLabel = (() => {
  const off = -new Date().getTimezoneOffset()
  const sign = off >= 0 ? "+" : "−"
  const h = Math.floor(Math.abs(off) / 60)
  const m = Math.abs(off) % 60
  return `UTC${sign}${h}${m ? `:${String(m).padStart(2, "0")}` : ""}`
})()

// ── Data ────────────────────────────────────────────────────────────────────

const audit = ref<AuditEventItem[]>([])
const loading = ref(false)
const loadError = ref("")
const truncated = ref(false)

const load = async () => {
  if (!range.value.valid) return
  loading.value = true
  loadError.value = ""
  try {
    const res = await fetchAuditEvents({
      profile_id: props.profile.id,
      since: new Date(range.value.from).toISOString(),
      until: new Date(range.value.to).toISOString(),
      limit: AUDIT_LIMIT,
    })
    audit.value = res.items ?? []
    truncated.value = (res.items?.length ?? 0) >= AUDIT_LIMIT
  } catch (e: unknown) {
    audit.value = []
    loadError.value = formatApiError(e, "Audit trail unavailable — showing pipeline history only")
  } finally {
    loading.value = false
  }
}

const applyCustom = () => {
  preset.value = "custom"
  void load()
}

onMounted(() => applyPreset("24h"))

// New pipeline events: keep a rolling preset anchored to "now".
watch(
  () => props.profile.updated_at,
  () => {
    if (preset.value !== "custom") applyPreset(preset.value)
  },
)

const parseJson = (raw: string): any => {
  try {
    return JSON.parse(raw)
  } catch {
    return null
  }
}

const historyLevel = (event: string, level: string): Level => {
  if (level === "error" || event.endsWith("_failed")) return "error"
  if (
    level === "warn" ||
    ["profile_paused", "profile_stopped", "profile_initial_load_cancelled", "source_offsets_reset"].includes(event)
  )
    return "warn"
  if (/(started|resumed|restarted|autostarted|initial_load|test_initial_sync|resynced)$/.test(event)) return "ok"
  return "info"
}

const rows = computed<Row[]>(() => {
  const { from, to, valid } = range.value
  if (!valid) return []
  const out: Row[] = []
  const historySeconds = new Set<number>()

  for (const [i, h] of (props.profile.history ?? []).entries()) {
    const at = new Date(h.timestamp).getTime()
    if (!(at >= from && at <= to)) continue
    historySeconds.add(Math.floor(at / 1000))
    out.push({
      key: `h-${i}-${h.timestamp}`,
      at,
      iso: h.timestamp,
      title: eventLabel(h.event),
      detail: h.detail,
      level: historyLevel(h.event, h.level),
      category: "pipeline",
    })
  }

  const nearHistory = (at: number) => {
    const s = Math.floor(at / 1000)
    for (let d = -5; d <= 5; d++) if (historySeconds.has(s + d)) return true
    return false
  }

  for (const e of audit.value) {
    const at = new Date(e.timestamp).getTime()
    const failed = e.outcome !== "success"
    const actor = e.actor && e.actor !== "system" ? e.actor.replace(/^system:/, "") : undefined
    const detail = parseJson(e.detail)

    if (e.action === "connector.auto_restart") {
      const secs = Number(detail?.next_retry_in_seconds ?? 0)
      const next = secs >= 60 ? `${Math.round(secs / 60)} min` : `${secs}s`
      out.push({
        key: `a-${e.audit_id}`,
        at,
        iso: e.timestamp,
        title: `Auto-restart of ${detail?.side ?? "source"} connector (attempt ${detail?.attempt ?? "?"})`,
        detail: [
          detail?.reason ? `Reason: ${detail.reason}` : "",
          failed ? `Restart request failed: ${detail?.error ?? ""}` : `Next try in ${next} if it fails again.`,
          detail?.connector ?? "",
        ]
          .filter(Boolean)
          .join("\n"),
        level: failed ? "error" : "warn",
        category: "connector",
        actor,
      })
      continue
    }
    if (e.action.startsWith("connector.")) {
      const toState = String(detail?.to_state ?? e.action.split(".")[1] ?? "").toUpperCase()
      const errors: string[] = Array.isArray(detail?.task_errors) ? detail.task_errors : []
      out.push({
        key: `a-${e.audit_id}`,
        at,
        iso: e.timestamp,
        title: `Connector ${detail?.from_state ? `${detail.from_state} → ` : ""}${toState || e.action}`,
        detail: [detail?.connector, ...errors].filter(Boolean).join("\n"),
        level: toState === "FAILED" || failed ? "error" : toState === "PAUSED" || toState === "STOPPED" ? "warn" : "ok",
        category: "connector",
        actor,
      })
      continue
    }
    // Successful lifecycle actions are already in the pipeline history — unless the
    // history has rotated them out (older than the 200-event cap).
    if (!failed && e.action.startsWith("profile.") && nearHistory(at)) continue
    out.push({
      key: `a-${e.audit_id}`,
      at,
      iso: e.timestamp,
      title: `${auditActionLabel(e.action)}${failed ? " failed" : ""}`,
      detail: failed ? String(detail?.error ?? e.detail) : "",
      level: failed ? "error" : "info",
      category: e.action.startsWith("profile.") ? "pipeline" : "operation",
      actor,
    })
  }
  return out.sort((a, b) => b.at - a.at)
})

const filtered = computed(() => {
  const q = search.value.trim().toLowerCase()
  return rows.value.filter((r) => {
    if (kind.value === "problems" && !(r.level === "error" || r.level === "warn")) return false
    if (kind.value === "pipeline" && r.category !== "pipeline") return false
    if (kind.value === "connector" && r.category !== "connector") return false
    if (q && !`${r.title} ${r.detail} ${r.actor ?? ""}`.toLowerCase().includes(q)) return false
    return true
  })
})

const counts = computed(() => ({
  total: filtered.value.length,
  errors: filtered.value.filter((r) => r.level === "error").length,
  warnings: filtered.value.filter((r) => r.level === "warn").length,
}))

// ── Density bar ─────────────────────────────────────────────────────────────

const MIN = 60_000
const HOUR = 60 * MIN
const DAY = 24 * HOUR
/** Calendar-friendly steps for bars and axis ticks. */
const STEPS = [MIN, 2 * MIN, 5 * MIN, 10 * MIN, 15 * MIN, 30 * MIN, HOUR, 2 * HOUR, 3 * HOUR, 6 * HOUR, 12 * HOUR, DAY, 2 * DAY, 7 * DAY]
const pickStep = (span: number, maxCount: number) => STEPS.find((s) => span / s <= maxCount) ?? STEPS[STEPS.length - 1]

/** Local-time offset, so hour/day steps align to the clock, not to UTC. */
const localOffset = (ms: number) => -new Date(ms).getTimezoneOffset() * MIN
const floorLocal = (ms: number, step: number) => {
  const off = localOffset(ms)
  return Math.floor((ms + off) / step) * step - off
}

/** Bars on clock-aligned intervals (1 min … 1 day), at most ~MAX_BARS of them. */
const histogram = computed(() => {
  const { from, to, valid } = range.value
  if (!valid) return null
  const step = pickStep(to - from, MAX_BARS)
  const start = floorLocal(from, step)
  const count = Math.max(1, Math.ceil((to - start) / step))
  const end = start + count * step
  const bars = Array.from({ length: count }, (_, i) => ({
    start: start + i * step,
    end: start + (i + 1) * step,
    total: 0,
    errors: 0,
    warnings: 0,
  }))
  for (const r of filtered.value) {
    const i = Math.min(count - 1, Math.max(0, Math.floor((r.at - start) / step)))
    bars[i].total++
    if (r.level === "error") bars[i].errors++
    else if (r.level === "warn") bars[i].warnings++
  }
  const max = Math.max(0, ...bars.map((b) => b.total))

  // X ticks: ~8 clock-aligned labels; day (or hour, for short ranges) boundaries in bold.
  const tick = pickStep(end - start, 8)
  const ticks: { at: number; left: number; label: string; major: boolean }[] = []
  for (let t = floorLocal(start, tick) + (floorLocal(start, tick) < start ? tick : 0); t <= end; t += tick) {
    const d = new Date(t)
    const midnight = d.getHours() === 0 && d.getMinutes() === 0
    const label =
      tick >= DAY || (midnight && tick >= HOUR)
        ? d.toLocaleDateString(undefined, { month: "short", day: "numeric" })
        : d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hour12: false })
    const major = tick < DAY && (midnight || (tick < HOUR && d.getMinutes() === 0))
    ticks.push({ at: t, left: ((t - start) / (end - start)) * 100, label, major })
  }
  return { bars, max, step, ticks }
})

const yTicks = computed(() => {
  const max = histogram.value?.max ?? 0
  if (!max) return [0]
  return max % 2 === 0 || max > 10 ? [max, Math.round(max / 2), 0] : [max, 0]
})

const hovered = ref<number | null>(null)
const hoveredBar = computed(() => (hovered.value === null ? null : histogram.value?.bars[hovered.value] ?? null))

const fmtBarRange = (b: { start: number; end: number }, step: number) => {
  const opts: Intl.DateTimeFormatOptions =
    step >= DAY
      ? { month: "short", day: "numeric" }
      : { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hour12: false }
  const endOpts: Intl.DateTimeFormatOptions =
    step >= DAY ? opts : { hour: "2-digit", minute: "2-digit", hour12: false }
  return `${new Date(b.start).toLocaleString(undefined, opts)} – ${new Date(b.end).toLocaleString(undefined, endOpts)}`
}

const fmtDateTime = (ms: number) =>
  new Date(ms).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })

const zoomTo = (b: { start: number; end: number; total: number }) => {
  if (!b.total) return
  fromInput.value = toLocalInput(b.start)
  toInput.value = toLocalInput(b.end)
  applyCustom()
}

// ── Timeline grouping ───────────────────────────────────────────────────────

const dayKey = (ms: number) => {
  const d = new Date(ms)
  return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`
}

const groups = computed(() => {
  const out: { key: string; label: string; rows: Row[] }[] = []
  for (const r of filtered.value) {
    const key = dayKey(r.at)
    let g = out[out.length - 1]
    if (!g || g.key !== key) {
      g = {
        key,
        label: new Date(r.at).toLocaleDateString(undefined, {
          weekday: "long",
          year: "numeric",
          month: "short",
          day: "numeric",
        }),
        rows: [],
      }
      out.push(g)
    }
    g.rows.push(r)
  }
  return out
})

const fmtTime = (ms: number) =>
  new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" })

const expanded = ref<Record<string, boolean>>({})
const isLong = (s: string) => s.length > 220 || s.split("\n").length > 3

const DOT: Record<Level, string> = {
  error: "bg-red-500 ring-red-500/20",
  warn: "bg-amber-500 ring-amber-500/20",
  ok: "bg-emerald-500 ring-emerald-500/20",
  info: "bg-muted-foreground/50 ring-muted-foreground/10",
}
const CATEGORY_LABEL: Record<Category, string> = {
  pipeline: "Pipeline",
  connector: "Connector",
  operation: "Operation",
}
</script>

<template>
  <div class="space-y-4">
    <!-- Range + filters -->
    <div class="space-y-3 rounded-xl border border-border p-3">
      <div class="flex flex-wrap items-center gap-1.5">
        <span class="mr-1 text-xs font-medium text-muted-foreground">Last</span>
        <button
          v-for="p in PRESETS"
          :key="p.key"
          type="button"
          class="rounded-lg border px-2.5 py-1 text-xs font-medium transition-colors"
          :class="
            preset === p.key
              ? 'border-primary bg-primary/10 text-primary'
              : 'border-border text-muted-foreground hover:text-foreground'
          "
          @click="applyPreset(p.key)"
        >
          {{ p.label }}
        </button>
        <span
          v-if="preset === 'custom'"
          class="rounded-lg border border-primary bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          Custom range
        </span>
      </div>

      <div class="grid grid-cols-1 gap-3 md:grid-cols-[1fr_1fr_auto]">
        <label class="block">
          <span class="auth-label">From</span>
          <input v-model="fromInput" type="datetime-local" class="auth-input w-full" @change="applyCustom" />
        </label>
        <label class="block">
          <span class="auth-label">To</span>
          <input v-model="toInput" type="datetime-local" class="auth-input w-full" @change="applyCustom" />
        </label>
        <div class="flex items-end">
          <AppButton variant="secondary" :disabled="loading || !range.valid" @click="load">
            <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
            Reload
          </AppButton>
        </div>
      </div>
      <p v-if="!range.valid" class="text-xs text-red-500">“From” must be before “To”.</p>

      <div class="grid grid-cols-1 gap-3 md:grid-cols-[14rem_1fr]">
        <label class="block">
          <span class="auth-label">Show</span>
          <select v-model="kind" class="auth-input w-full">
            <option value="all">All events</option>
            <option value="problems">Problems only (failures, warnings)</option>
            <option value="pipeline">Pipeline actions</option>
            <option value="connector">Connector state changes</option>
          </select>
        </label>
        <label class="block">
          <span class="auth-label">Search</span>
          <div class="relative">
            <Search
              class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground"
              :stroke-width="1.75"
            />
            <input v-model="search" class="auth-input w-full pl-9" placeholder="Error text, event, connector…" />
          </div>
        </label>
      </div>
    </div>

    <!-- Summary + density over the range -->
    <div class="space-y-2">
      <div class="flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
        <p>
          <span class="font-medium text-foreground">{{ counts.total.toLocaleString() }}</span> events
          <template v-if="counts.errors">
            · <span class="font-medium text-red-600 dark:text-red-400">{{ counts.errors }} failed</span>
          </template>
          <template v-if="counts.warnings">
            · <span class="font-medium text-amber-600 dark:text-amber-400">{{ counts.warnings }} warnings</span>
          </template>
          <template v-if="range.valid">
            · {{ fmtDateTime(range.from) }} – {{ fmtDateTime(range.to) }} ({{ tzLabel }})
          </template>
        </p>
      </div>
      <!-- Histogram: events per clock-aligned interval; click a bar to zoom in -->
      <div v-if="histogram" class="rounded-lg border border-border px-2 pb-1 pt-3">
        <div class="flex">
          <!-- Y axis -->
          <div class="relative h-28 w-12 shrink-0 text-[10px] tabular-nums text-muted-foreground">
            <span
              v-for="(y, i) in yTicks"
              :key="y"
              class="absolute right-2 -translate-y-1/2"
              :style="{ top: `${yTicks.length === 1 ? 100 : (i / (yTicks.length - 1)) * 100}%` }"
            >
              {{ y.toLocaleString() }}
            </span>
          </div>
          <div class="min-w-0 flex-1">
            <!-- Plot -->
            <div class="relative h-28 border-b border-border" @mouseleave="hovered = null">
              <div
                v-for="(y, i) in yTicks"
                :key="`g${y}`"
                class="pointer-events-none absolute inset-x-0 border-t border-border/60"
                :style="{ top: `${yTicks.length === 1 ? 100 : (i / (yTicks.length - 1)) * 100}%` }"
              />
              <div class="absolute inset-0 flex items-end gap-px">
                <button
                  v-for="(b, i) in histogram.bars"
                  :key="b.start"
                  type="button"
                  class="relative flex h-full min-w-0 flex-1 flex-col justify-end"
                  :class="b.total ? 'cursor-zoom-in' : 'cursor-default'"
                  :aria-label="`${fmtBarRange(b, histogram.step)}: ${b.total} events`"
                  @mouseenter="hovered = i"
                  @click="zoomTo(b)"
                >
                  <span
                    v-if="b.total"
                    class="flex w-full flex-col overflow-hidden transition-opacity"
                    :class="hovered !== null && hovered !== i ? 'opacity-60' : ''"
                    :style="{ height: `${Math.max(2, (b.total / histogram.max) * 100)}%` }"
                  >
                    <span class="bg-red-500" :style="{ flex: b.errors }" />
                    <span class="bg-amber-500" :style="{ flex: b.warnings }" />
                    <span class="bg-indigo-400 dark:bg-indigo-400/80" :style="{ flex: b.total - b.errors - b.warnings }" />
                  </span>
                </button>
              </div>
              <!-- Hover tooltip -->
              <div
                v-if="hoveredBar && hovered !== null"
                class="pointer-events-none absolute -top-2 z-10 -translate-y-full whitespace-nowrap rounded-md border border-border bg-popover px-2 py-1 text-[11px] text-popover-foreground shadow-md"
                :class="hovered / histogram.bars.length > 0.7 ? '-translate-x-full' : ''"
                :style="{ left: `${((hovered + 0.5) / histogram.bars.length) * 100}%` }"
              >
                <p class="font-medium">{{ fmtBarRange(hoveredBar, histogram.step) }}</p>
                <p class="text-muted-foreground">
                  {{ hoveredBar.total.toLocaleString() }} events
                  <span v-if="hoveredBar.errors" class="text-red-600 dark:text-red-400">· {{ hoveredBar.errors }} failed</span>
                  <span v-if="hoveredBar.warnings" class="text-amber-600 dark:text-amber-400">· {{ hoveredBar.warnings }} warnings</span>
                </p>
                <p v-if="hoveredBar.total" class="text-muted-foreground">Click to zoom in</p>
              </div>
            </div>
            <!-- X axis -->
            <div class="relative h-6 text-[11px] text-muted-foreground">
              <span
                v-for="t in histogram.ticks"
                :key="t.at"
                class="absolute top-0 flex -translate-x-1/2 flex-col items-center whitespace-nowrap"
                :class="t.major ? 'font-semibold text-foreground' : ''"
                :style="{ left: `${t.left}%` }"
              >
                <span class="h-1 w-px bg-border" />
                {{ t.label }}
              </span>
            </div>
          </div>
        </div>
      </div>
    </div>

    <AppStateBlock v-if="loadError" kind="error" :message="loadError" />
    <p v-if="truncated" class="flex items-center gap-1.5 text-xs text-amber-600 dark:text-amber-400">
      <AlertTriangle class="h-3.5 w-3.5" :stroke-width="1.75" />
      Showing the newest {{ AUDIT_LIMIT.toLocaleString() }} audit events in this range — narrow the range to see older ones.
    </p>

    <div v-if="loading && !rows.length" class="space-y-2">
      <div v-for="i in 4" :key="i" class="h-14 animate-pulse rounded-xl bg-muted/50" />
    </div>
    <AppEmptyState
      v-else-if="!filtered.length"
      title="No activity in this range"
      :description="kind !== 'all' || search ? 'Try All events or clear the search.' : 'Pick a wider time range.'"
    />

    <!-- Timeline -->
    <div v-else class="space-y-5">
      <section v-for="g in groups" :key="g.key">
        <h4
          class="sticky top-0 z-10 mb-2 bg-card/95 py-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground backdrop-blur"
        >
          {{ g.label }} · {{ g.rows.length }}
        </h4>
        <ol class="relative ml-[5.5rem] border-l border-border">
          <li v-for="r in g.rows" :key="r.key" class="relative pb-3 pl-5 last:pb-0">
            <time
              class="absolute -left-[5.5rem] top-1 w-[4.75rem] text-right font-mono text-[11px] tabular-nums text-muted-foreground"
              :datetime="r.iso"
              :title="r.iso"
            >
              {{ fmtTime(r.at) }}
            </time>
            <span class="absolute -left-[5px] top-2 h-2.5 w-2.5 rounded-full ring-4" :class="DOT[r.level]" />
            <div
              class="rounded-xl border px-3 py-2"
              :class="{
                'border-red-500/30 bg-red-500/5': r.level === 'error',
                'border-amber-500/30 bg-amber-500/5': r.level === 'warn',
                'border-border': r.level === 'ok' || r.level === 'info',
              }"
            >
              <div class="flex flex-wrap items-center gap-2">
                <CircleX v-if="r.level === 'error'" class="h-4 w-4 text-red-500" :stroke-width="1.75" />
                <AlertTriangle v-else-if="r.level === 'warn'" class="h-4 w-4 text-amber-500" :stroke-width="1.75" />
                <CircleCheck v-else-if="r.level === 'ok'" class="h-4 w-4 text-emerald-500" :stroke-width="1.75" />
                <Info v-else class="h-4 w-4 text-muted-foreground" :stroke-width="1.75" />
                <p class="text-sm font-medium text-foreground">{{ r.title }}</p>
                <span
                  class="inline-flex items-center gap-1 rounded-full border border-border px-1.5 py-px text-[10px] text-muted-foreground"
                >
                  <Plug v-if="r.category === 'connector'" class="h-3 w-3" :stroke-width="1.75" />
                  <Workflow v-else class="h-3 w-3" :stroke-width="1.75" />
                  {{ CATEGORY_LABEL[r.category] }}
                </span>
                <span v-if="r.actor" class="text-[10px] text-muted-foreground">by {{ r.actor }}</span>
              </div>
              <template v-if="r.detail">
                <p
                  class="mt-1 whitespace-pre-wrap break-words text-xs text-muted-foreground"
                  :class="{ 'line-clamp-3': isLong(r.detail) && !expanded[r.key] }"
                >
                  {{ r.detail }}
                </p>
                <button
                  v-if="isLong(r.detail)"
                  type="button"
                  class="mt-1 text-[11px] font-medium text-primary hover:underline"
                  @click="expanded[r.key] = !expanded[r.key]"
                >
                  {{ expanded[r.key] ? "Show less" : "Show more" }}
                </button>
              </template>
            </div>
          </li>
        </ol>
      </section>
    </div>
  </div>
</template>
