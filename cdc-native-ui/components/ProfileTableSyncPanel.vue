<script setup lang="ts">
import {
  AlertTriangle,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Database,
  Info,
  Gauge,
  RefreshCw,
  Search,
  Table2,
} from "@lucide/vue"
import type {
  ErrorDiagnosis,
  ProfileTableSyncStatusResponse,
  TableSyncStatusRow,
} from "~/composables/useOperationsM3"
import { describeApiError, formatApiError, type FriendlyError } from "~/utils/apiError"

const props = withDefaults(
  defineProps<{
    profileId: string
    /** When false, caller triggers refresh via exposed `refresh`. */
    autoLoad?: boolean
    compact?: boolean
    /**
     * Changes whenever the pipeline changes (pass the profile's `updated_at`). A new
     * value refetches counts, so a finished test initial sync / resync shows the rows
     * it just wrote instead of the numbers from before the action.
     */
    refreshToken?: string | null
    /** Vendor logo / name / connection of each end, for the connection cards. */
    endpoints?: {
      source?: { image?: string; label?: string; caption?: string }
      target?: { image?: string; label?: string; caption?: string }
    } | null
  }>(),
  { autoLoad: true, compact: false },
)

export type EndpointState = "ok" | "offline" | "attention" | "unknown"
export type PipelineHealth = { source: EndpointState; target: EndpointState; engine: boolean }

const emit = defineEmits<{
  /** Plain source / target connection state, for the pipeline header. */
  health: [value: PipelineHealth | null]
}>()

const { fetchProfileTableSyncStatus } = useOperationsM3()
const LIVE_MS = 3000
const toast = useToast()

const loading = ref(false)
const data = ref<ProfileTableSyncStatusResponse | null>(null)
const search = ref("")
const phaseFilter = ref("")
const expanded = ref<Record<string, boolean>>({})

/**
 * Live mode: re-poll every LIVE_MS counting only the target (+ Kafka offsets). Source
 * counts come from the last full refresh — `COUNT(*)` on a large production source every
 * few seconds is not acceptable — so pending / progress / phase are recomputed here.
 */
const live = ref(true)
/** Last Live update failure, explained (null = updates are working). */
const liveError = ref<FriendlyError | null>(null)
const sourceCache = new Map<string, number>()
const lastSample = new Map<string, { at: number; target: number }>()
/** Smoothed target growth per table, rows/s. */
const rates = ref<Record<string, number>>({})
/** Tables whose target count grew on the latest tick (brief highlight). */
const bumped = ref<Record<string, boolean>>({})
/**
 * Source problems seen by the last full refresh. Live ticks never count the source, so
 * without this a "source offline" would vanish 3s later and come back on Refresh.
 */
const sourceCountProblems = ref<ErrorDiagnosis[]>([])
let liveTimer: ReturnType<typeof setTimeout> | null = null
let fetching = false

const fmtCount = (n: number | null | undefined) => {
  if (n == null) return "—"
  return n.toLocaleString()
}

/**
 * Pipeline is not moving data (stopped, draft, validated, paused). Source-vs-target gaps
 * are then expected — e.g. after a test initial sync — not "catching up", and capture
 * topics that were never written to simply do not exist yet. Failed keeps the live view
 * so its errors stay red.
 */
const pipelineIdle = computed(() =>
  ["stopped", "draft", "validated", "paused"].includes(
    String(data.value?.profile_status || "").toLowerCase(),
  ),
)
const pipelinePaused = computed(() => String(data.value?.profile_status || "").toLowerCase() === "paused")

/** Phases that only make sense while the pipeline runs. */
const isIdlePhase = (phase: TableSyncStatusRow["phase"]) =>
  pipelineIdle.value && (phase === "behind" || phase === "in_progress")

/** Debezium capture (Logical / SyncOnce): no per-table JDBC watermark exists. */
const logBased = computed(() =>
  ["logical", "synconce"].includes(String(data.value?.capture_mode || "").toLowerCase()),
)

/** Orchestrator's `kafka_error` for a capture topic that does not exist (yet). */
const MISSING_TOPIC = "topic not found or no partitions"
const topicMissing = (row: TableSyncStatusRow) => row.kafka_error === MISSING_TOPIC

/**
 * A missing topic is usually a normal state, not a failure: say why, in neutral text.
 * Only other Kafka errors (broker unreachable, rpk failed) stay red.
 */
const missingTopicNote = (row: TableSyncStatusRow) => {
  if (pipelineIdle.value) {
    return "Created when the pipeline first runs (Start or Resync). A test initial sync copies rows directly and does not use Kafka."
  }
  const mode = String(data.value?.capture_mode || "").toLowerCase()
  if (row.phase === "complete") {
    return "Topic no longer in Kafka (retention or cleanup). Not needed — the target already has these rows."
  }
  if (mode === "incremental") {
    return "Waiting for the first poll that finds rows in this table — the topic is created then."
  }
  if (mode === "synconce") {
    return "Waiting for the snapshot to reach this table — the topic is created with its first row."
  }
  return "Waiting for the first record from this table — the topic is created by the initial snapshot or the first change."
}

/**
 * Plain-language health of each end of the pipeline, from diagnosed errors: connector
 * failures (every live tick), target counts (every tick) and source counts (last full
 * refresh). "offline" = can't connect; "attention" = connected but something is wrong.
 */
const allDiagnoses = computed<ErrorDiagnosis[]>(() => {
  const out: ErrorDiagnosis[] = (data.value?.connector_issues ?? []).map((i) => ({
    ...i.diagnosis,
    // A failed connector without a side-specific cause still belongs to its end.
    side: i.diagnosis.side ?? (i.role === "sink" ? "target" : undefined),
  })) as ErrorDiagnosis[]
  out.push(...sourceCountProblems.value)
  for (const r of data.value?.tables ?? []) if (r.target_count_diagnosis) out.push(r.target_count_diagnosis)
  return out
})

const endpointState = (side: "source" | "target"): EndpointState => {
  const mine = allDiagnoses.value.filter((d) => d.side === side)
  if (mine.some((d) => d.area === "network")) return "offline"
  if (mine.length) return "attention"
  const counted = (data.value?.tables ?? []).some((r) =>
    side === "source" ? r.source_row_count != null : r.target_row_count != null,
  )
  return counted ? "ok" : "unknown"
}

const health = computed<PipelineHealth | null>(() =>
  data.value
    ? {
        source: endpointState("source"),
        target: endpointState("target"),
        engine: allDiagnoses.value.some((d) => !d.side),
      }
    : null,
)

watch(health, (h) => emit("health", h), { immediate: true })

/** First diagnosis for one end — its title / fix explain the state in words. */
const endpointDiagnosis = (side: "source" | "target") =>
  allDiagnoses.value.find((d) => d.side === side && d.area === "network") ??
  allDiagnoses.value.find((d) => d.side === side)

const ENDPOINT_TEXT: Record<EndpointState, string> = {
  ok: "Connected",
  offline: "Can't connect",
  attention: "Needs attention",
  unknown: "Not checked yet",
}

/** Why a row is in error, in plain words. */
const errorCauseLabel = (row: TableSyncStatusRow) => {
  const src = row.source_count_diagnosis
  const tgt = row.target_count_diagnosis
  if (tgt?.area === "network" || health.value?.target === "offline") return "Target offline"
  if (src?.area === "network" || health.value?.source === "offline") return "Source offline"
  if (tgt || health.value?.target === "attention") return "Target issue"
  if (src || health.value?.source === "attention") return "Source issue"
  if (health.value?.engine) return "Sync engine issue"
  return "Error"
}

const rowPhaseLabel = (row: TableSyncStatusRow) =>
  isIdlePhase(row.phase)
    ? pipelinePaused.value
      ? "Paused"
      : "Not running"
    : row.phase === "error"
      ? errorCauseLabel(row)
      : phaseLabel(row.phase)

const rowBadgeStatus = (row: TableSyncStatusRow) =>
  isIdlePhase(row.phase) ? (pipelinePaused.value ? "paused" : "stopped") : badgeStatus(row.phase)

const phaseLabel = (phase: TableSyncStatusRow["phase"]) => {
  switch (phase) {
    case "complete":
      return "Complete"
    case "in_progress":
      return "Syncing"
    case "behind":
      return "Catching up"
    case "error":
      return "Error"
    default:
      return "Unknown"
  }
}

const badgeStatus = (phase: TableSyncStatusRow["phase"]) => {
  switch (phase) {
    case "complete":
      return "success"
    case "in_progress":
      return "pending"
    case "behind":
      return "warning"
    case "error":
      return "failed"
    default:
      return "unknown"
  }
}

const progressPct = (row: TableSyncStatusRow) => {
  const s = row.source_row_count
  const t = row.target_row_count
  if (s == null || t == null || s <= 0) return null
  return Math.min(100, Math.round((t / s) * 100))
}

const progressTone = (pct: number | null, phase: TableSyncStatusRow["phase"]) => {
  if (phase === "error") return "bg-red-500"
  if (pct == null) return "bg-muted-foreground/30"
  if (pct >= 99) return "bg-emerald-500"
  if (pct >= 50) return "bg-brand-500"
  return "bg-amber-500"
}

const rowKey = (row: TableSyncStatusRow) => row.source_qualified

const fmtRate = (r: number) =>
  r >= 1000 ? `${(r / 1000).toFixed(r >= 10000 ? 0 : 1)}k/s` : `${Math.round(r)}/s`

const fmtEta = (seconds: number) => {
  if (!Number.isFinite(seconds) || seconds <= 0) return ""
  if (seconds < 60) return `${Math.ceil(seconds)}s`
  const m = Math.ceil(seconds / 60)
  if (m < 60) return `${m}m`
  return `${Math.floor(m / 60)}h ${m % 60}m`
}

const rowRate = (row: TableSyncStatusRow) => rates.value[rowKey(row)] ?? 0

const rowEta = (row: TableSyncStatusRow) => {
  const rate = rowRate(row)
  const pending = row.pending_rows ?? 0
  return rate > 0.5 && pending > 0 ? fmtEta(pending / rate) : ""
}

/** Same rules as the orchestrator's classify_phase, applied after filling in source. */
const phaseFor = (
  row: TableSyncStatusRow,
  s: number | null | undefined,
  t: number | null | undefined,
  p: number | null | undefined,
): TableSyncStatusRow["phase"] => {
  if (row.source_count_error || row.target_count_error) return "error"
  if (s == null || t == null || p == null) return row.phase
  if (s > 0 && p === 0) return "complete"
  if (s > 0 && t === 0 && p === s) return "in_progress"
  if (p > 0) return "behind"
  return "unknown"
}

/** Merge a response: fill cached source counts, update rates and highlights. */
/** When the source was last counted (Live never counts it). */
const sourceCountedAt = ref<number | null>(null)

const absorb = (res: ProfileTableSyncStatusResponse, sourceCounted = false, partial = false) => {
  if (sourceCounted) {
    sourceCountedAt.value = Date.now()
    sourceCountProblems.value = res.tables
      .map((r) => r.source_count_diagnosis)
      .filter((x): x is ErrorDiagnosis => Boolean(x))
  }
  const now = Date.now()
  const nextRates: Record<string, number> = { ...rates.value }
  const nextBumped: Record<string, boolean> = {}
  for (const row of res.tables) {
    const key = rowKey(row)
    if (row.source_row_count != null) sourceCache.set(key, row.source_row_count)
    else if (sourceCache.has(key) && !res.counts_skipped) {
      // A growing source (live CDC) can pass the cached count: never show negative pending.
      const s = Math.max(sourceCache.get(key)!, row.target_row_count ?? 0)
      row.source_row_count = s
      if (row.target_row_count != null) {
        row.pending_rows = Math.max(0, s - row.target_row_count)
        let phase = phaseFor(row, s, row.target_row_count, row.pending_rows)
        if (res.connector_issue && phase !== "complete") phase = "error"
        row.phase = phase
      }
    }
    const t = row.target_row_count
    if (t == null) continue
    const prev = lastSample.get(key)
    if (prev && now > prev.at) {
      const inst = Math.max(0, (t - prev.target) / ((now - prev.at) / 1000))
      const old = nextRates[key]
      nextRates[key] = old == null ? inst : old * 0.5 + inst * 0.5
      if (t > prev.target) nextBumped[key] = true
    }
    lastSample.set(key, { at: now, target: t })
  }
  rates.value = nextRates
  bumped.value = nextBumped
  // A page-only poll updates just those rows; the others keep their last values.
  if (partial && data.value) {
    const fresh = new Map(res.tables.map((r) => [rowKey(r), r]))
    data.value = { ...res, tables: data.value.tables.map((r) => fresh.get(rowKey(r)) ?? r) }
  } else {
    data.value = res
  }
}

const resetLiveState = () => {
  sourceCountProblems.value = []
  sourceCountedAt.value = null
  sourceCache.clear()
  lastSample.clear()
  rates.value = {}
  bumped.value = {}
}

const pollRuleLabel = (rule: NonNullable<TableSyncStatusRow["poll_rule"]>) => {
  if (rule.strategy === "full_copy") return "Full copy — whole table every poll"
  const days = rule.window_days ?? 1
  const span = days === 0 ? "today" : `today + previous ${days} day${days === 1 ? "" : "s"}`
  return `Rolling window on ${rule.window_column || "?"} — ${span}`
}

const filteredRows = computed(() => {
  let rows = [...(data.value?.tables || [])]
  if (phaseFilter.value) {
    rows = rows.filter((r) => r.phase === phaseFilter.value)
  }
  const q = search.value.trim().toLowerCase()
  if (q) {
    rows = rows.filter((r) =>
      [r.source_qualified, r.target_qualified, r.kafka_topic, r.phase]
        .filter(Boolean)
        .join(" ")
        .toLowerCase()
        .includes(q),
    )
  }
  rows.sort((a, b) => {
    const pa = a.pending_rows ?? -1
    const pb = b.pending_rows ?? -1
    if (pb !== pa) return pb - pa
    return a.source_qualified.localeCompare(b.source_qualified)
  })
  return rows
})

// ── Pagination ─────────────────────────────────────────────────────────────
const PAGE_SIZES = [10, 25, 50, 100] as const
const pageSize = ref<number>(25)
const page = ref(1)

const totalPages = computed(() => Math.max(1, Math.ceil(filteredRows.value.length / pageSize.value)))
const pagedRows = computed(() => {
  const start = (page.value - 1) * pageSize.value
  return filteredRows.value.slice(start, start + pageSize.value)
})
const pageFrom = computed(() => (filteredRows.value.length ? (page.value - 1) * pageSize.value + 1 : 0))
const pageTo = computed(() => Math.min(filteredRows.value.length, page.value * pageSize.value))

watch([search, phaseFilter, pageSize], () => {
  page.value = 1
})
watch(totalPages, (n) => {
  if (page.value > n) page.value = n
})

/** Page buttons: first, last, and the current page's neighbours; gaps become "…". */
const pageButtons = computed<(number | "gap")[]>(() => {
  const n = totalPages.value
  const p = page.value
  const keep = new Set([1, n, p - 1, p, p + 1].filter((x) => x >= 1 && x <= n))
  const out: (number | "gap")[] = []
  let prev = 0
  for (const x of [...keep].sort((a, b) => a - b)) {
    if (x - prev > 1) out.push("gap")
    out.push(x)
    prev = x
  }
  return out
})

/** More tables than one page: Live polls only the visible page. */
const pagedPolling = computed(() => (data.value?.tables.length ?? 0) > pageSize.value)

// ── Sync summary ───────────────────────────────────────────────────────────
const syncSummary = computed(() => {
  const tables = data.value?.tables ?? []
  const total = tables.length
  const inSync = tables.filter((t) => t.phase === "complete").length
  const sum = (pick: (t: TableSyncStatusRow) => number | null | undefined) => {
    let known = 0
    let value = 0
    for (const t of tables) {
      const v = pick(t)
      if (v != null) {
        known++
        value += v
      }
    }
    return known ? value : null
  }
  return {
    total,
    inSync,
    pct: total ? Math.round((inSync / total) * 100) : 0,
    targetRows: sum((t) => t.target_row_count),
    sourceRows: sum((t) => t.source_row_count),
  }
})

const summary = computed(() => {
  const tables = data.value?.tables || []
  return {
    total: tables.length,
    complete: tables.filter((t) => t.phase === "complete").length,
    active: pipelineIdle.value
      ? 0
      : tables.filter((t) => t.phase === "in_progress" || t.phase === "behind").length,
    errors: tables.filter((t) => t.phase === "error").length,
    pendingSum: tables.reduce((acc, t) => acc + (t.pending_rows ?? 0), 0),
    rate: tables.reduce((acc, t) => acc + rowRate(t), 0),
  }
})

const totalEta = computed(() =>
  summary.value.rate > 0.5 && summary.value.pendingSum > 0
    ? fmtEta(summary.value.pendingSum / summary.value.rate)
    : "",
)

const lastUpdated = computed(() => {
  if (!data.value?.timestamp) return ""
  try {
    return new Date(data.value.timestamp).toLocaleString()
  } catch {
    return data.value.timestamp
  }
})

/** Source COUNT(*) is running in the background (table list already shown). */
const countingSource = ref(false)

/** Full refresh: source + target counts (the explicit "count the source now"). */
const refresh = async () => {
  if (!props.profileId) return
  loading.value = true
  fetching = true
  try {
    // First open: show the table list at once from target + Kafka (fast), then add the
    // source counts. A slow or unreachable source no longer keeps the list empty.
    if (!data.value) {
      try {
        absorb(await fetchProfileTableSyncStatus(props.profileId, { skip_source_counts: true }))
      } catch {
        /* the full request below reports the error */
      }
      loading.value = false
      countingSource.value = true
    }
    absorb(
      await fetchProfileTableSyncStatus(props.profileId, {
      }),
      true,
    )
    lastSourceCheck = Date.now()
    liveError.value = null
  } catch (e: unknown) {
    // Keep the rows already shown (e.g. from the fast first pass); just report.
    toast.error(formatApiError(e, "Table sync status failed"))
  } finally {
    loading.value = false
    countingSource.value = false
    fetching = false
    if (live.value && !liveTimer) scheduleLive()
  }
}

/** While the source is marked as failing, re-check it this often (ms). */
const SOURCE_RECHECK_MS = 30_000
let lastSourceCheck = 0

/** One live tick: target-only counts, no spinner, errors shown inline (no toast storm). */
const liveTick = async () => {
  if (!props.profileId || fetching) return
  if (typeof document !== "undefined" && document.visibilityState === "hidden") return
  // A remembered "source can't connect" must not outlive the outage: Live normally
  // skips the source, so re-check it (quick TCP probe first) until it is healthy again.
  if (
    sourceCountProblems.value.length &&
    Date.now() - lastSourceCheck >= SOURCE_RECHECK_MS
  ) {
    lastSourceCheck = Date.now()
    fetching = true
    try {
      absorb(await fetchProfileTableSyncStatus(props.profileId, {}), true)
      liveError.value = null
    } catch (e: unknown) {
      liveError.value = describeApiError(e, "Live update failed")
    } finally {
      fetching = false
    }
    return
  }
  fetching = true
  try {
    // Live spares the source; it always counts the target, otherwise there is nothing
    // to show progress (or a status) from. With many tables, only the visible page.
    const pageOnly = pagedPolling.value && pagedRows.value.length > 0
    absorb(
      await fetchProfileTableSyncStatus(props.profileId, {
        skip_source_counts: true,
        ...(pageOnly ? { tables: pagedRows.value.map(rowKey).join(",") } : {}),
      }),
      false,
      pageOnly,
    )
    liveError.value = null
  } catch (e: unknown) {
    liveError.value = describeApiError(e, "Live update failed")
  } finally {
    fetching = false
  }
}

const stopLive = () => {
  if (liveTimer) clearTimeout(liveTimer)
  liveTimer = null
}

/** setTimeout chain (not setInterval): the next poll starts only after this one returns. */
const scheduleLive = () => {
  stopLive()
  if (!live.value || !props.profileId) return
  liveTimer = setTimeout(async () => {
    await liveTick()
    scheduleLive()
  }, LIVE_MS)
}

watch(live, (on) => {
  if (on) scheduleLive()
  else stopLive()
})

onBeforeUnmount(stopLive)

const toggleRow = (key: string) => {
  expanded.value[key] = !expanded.value[key]
}

watch(
  () => props.profileId,
  (id) => {
    expanded.value = {}
    data.value = null
    resetLiveState()
    stopLive()
    if (id && props.autoLoad) void refresh()
  },
  { immediate: true },
)

watch(
  () => props.refreshToken,
  (token, previous) => {
    if (props.profileId && props.autoLoad && token && token !== previous && !loading.value) {
      void refresh()
    }
  },
)

defineExpose({ refresh, data, loading })
</script>

<template>
  <div class="flex flex-col gap-4" :class="compact ? '' : 'animate-fade-in'">
    <!-- Plain connection status of each end, while the pipeline should be moving. -->
    <div v-if="health && !pipelineIdle" class="grid grid-cols-1 gap-3 sm:grid-cols-2">
      <div
        v-for="side in (['source', 'target'] as const)"
        :key="side"
        class="flex items-start gap-3 rounded-xl border px-4 py-3"
        :class="{
          'border-red-500/40 bg-red-500/5': health[side] === 'offline',
          'border-amber-500/40 bg-amber-500/5': health[side] === 'attention',
          'border-emerald-500/30 bg-emerald-500/5': health[side] === 'ok',
          'border-border bg-muted/20': health[side] === 'unknown',
        }"
      >
        <!-- Vendor logo with the status dot on its corner. -->
        <span class="relative mt-0.5 shrink-0">
          <img
            v-if="endpoints?.[side]?.image"
            :src="endpoints[side]!.image"
            :alt="endpoints[side]!.label || side"
            class="h-9 w-9 rounded-lg border border-border bg-background object-contain p-1"
          />
          <span v-else class="block h-9 w-9 rounded-lg border border-border bg-muted/40" />
          <span
            class="absolute -bottom-0.5 -right-0.5 h-3 w-3 rounded-full ring-2 ring-background"
            :class="{
              'bg-red-500': health[side] === 'offline',
              'bg-amber-500': health[side] === 'attention',
              'bg-emerald-500': health[side] === 'ok',
              'bg-muted-foreground/40': health[side] === 'unknown',
            }"
          />
        </span>
        <div class="min-w-0 text-sm">
          <p class="font-medium text-foreground">
            {{ side === "source" ? "Source database" : "Target database" }} ·
            <span
              :class="{
                'text-red-600 dark:text-red-400': health[side] === 'offline',
                'text-amber-600 dark:text-amber-400': health[side] === 'attention',
                'text-emerald-600 dark:text-emerald-400': health[side] === 'ok',
                'text-muted-foreground': health[side] === 'unknown',
              }"
            >{{ ENDPOINT_TEXT[health[side]] }}</span>
          </p>
          <p
            v-if="endpoints?.[side]?.label || endpoints?.[side]?.caption"
            class="truncate font-mono text-[11px] text-muted-foreground"
            :title="endpoints?.[side]?.caption"
          >
            {{ [endpoints?.[side]?.label, endpoints?.[side]?.caption].filter(Boolean).join(" · ") }}
          </p>
          <p v-if="endpointDiagnosis(side)" class="mt-0.5 text-xs text-muted-foreground">
            {{ endpointDiagnosis(side)!.title }}. {{ endpointDiagnosis(side)!.fix }}
          </p>
        </div>
      </div>
    </div>
    <div
      v-if="data?.connector_issue"
      class="alert-error flex items-start gap-2 rounded-xl px-4 py-3 text-sm"
      role="alert"
    >
      <AlertTriangle class="mt-0.5 h-4 w-4 shrink-0" :stroke-width="1.75" />
      <div class="min-w-0 flex-1 space-y-3 break-words">
        <p class="font-semibold">Pipeline is not moving.</p>
        <template v-if="data.connector_issues?.length">
          <ErrorDiagnosis
            v-for="(issue, i) in data.connector_issues"
            :key="`${issue.connector}-${i}`"
            :diagnosis="issue.diagnosis"
            :context="`${issue.role === 'sink' ? 'Sink' : 'Source'} connector · ${issue.connector}`"
            :detail="issue.message"
          />
        </template>
        <p v-else>{{ data.connector_issue }}</p>
      </div>
    </div>

    <div class="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
      <AppMetricCard label="Tables" :value="summary.total" helper="Mapped capture set" tone="brand">
        <Table2 class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="Complete"
        :value="summary.complete"
        helper="Pending rows ≈ 0"
        tone="success"
      >
        <Database class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="In flight"
        :value="summary.active"
        :helper="pipelineIdle ? 'Pipeline not running' : 'Syncing or catching up'"
        tone="warning"
      />
      <AppMetricCard
        label="Errors"
        :value="summary.errors"
        helper="Count or connector issues"
        :tone="summary.errors > 0 ? 'danger' : 'neutral'"
      />
      <AppMetricCard
        label="Pending rows"
        :value="data?.counts_skipped ? '—' : fmtCount(summary.pendingSum)"
        :helper="totalEta ? `Σ(source − target) · ETA ${totalEta}` : 'Σ(source − target)'"
        tone="info"
      />
      <AppMetricCard
        label="Throughput"
        :value="live ? fmtRate(summary.rate) : '—'"
        :helper="live ? 'Rows/s applied to target' : 'Turn on Live to measure'"
        tone="brand"
      >
        <Gauge class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
    </div>

    <div class="stat-card space-y-4">
      <div class="flex flex-col gap-3 lg:flex-row lg:items-end lg:justify-between">
        <div>
          <AppSectionTitle
            title="Replica progress by table"
            hint="Compare each source table to its target replica. Processing = change events in Kafka the sink has not applied yet (inserts, updates and deletes; it may show a few for up to a minute because the sink saves its position periodically). Pending = rows the target is still missing (source − target). Incremental watermarks may exclude source rows from the total."
          />
          <p v-if="lastUpdated" class="mt-1 text-xs text-muted-foreground">
            Last updated {{ lastUpdated }}
            <span v-if="data?.capture_mode" class="ml-2 font-mono">{{ data.capture_mode }}</span>
            <span v-if="live" class="ml-2">· live every {{ LIVE_MS / 1000 }}s</span>
            <span v-if="sourceCountedAt" class="ml-2">
              · source counted {{ new Date(sourceCountedAt).toLocaleTimeString() }}
            </span>
          </p>
          <p v-if="countingSource" class="mt-1 flex items-center gap-1.5 text-xs text-muted-foreground">
            <RefreshCw class="h-3 w-3 animate-spin" :stroke-width="2" />
            Counting source rows…
          </p>
          <!-- Plain words first; the raw request error only on demand. -->
          <div
            v-if="live && liveError"
            class="mt-1.5 max-w-2xl rounded-lg border border-amber-500/40 bg-amber-500/5 px-3 py-2 text-xs"
            role="status"
          >
            <p class="flex items-center gap-1.5 font-medium text-amber-700 dark:text-amber-300">
              <AlertTriangle class="h-3.5 w-3.5 shrink-0" :stroke-width="2" />
              {{ liveError.title }}
              <span class="font-normal text-muted-foreground">
                · retrying automatically every {{ LIVE_MS / 1000 }}s — numbers below are from {{ lastUpdated || "the last update" }}
              </span>
            </p>
            <p v-if="liveError.hint" class="mt-0.5 text-muted-foreground">{{ liveError.hint }}</p>
            <details class="group mt-1 text-muted-foreground">
              <summary class="flex cursor-pointer list-none items-center gap-1 hover:text-foreground">
                <ChevronRight class="h-3 w-3 transition-transform group-open:rotate-90" :stroke-width="2" />
                Technical detail
              </summary>
              <pre class="mt-1 whitespace-pre-wrap break-all rounded bg-muted/40 p-2 font-mono text-[11px]">{{ liveError.detail }}</pre>
            </details>
          </div>
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <!-- One switch: Live keeps target + Kafka current every few seconds and never
               loads the source; Refresh is the explicit "count the source now". -->
          <button
            type="button"
            role="switch"
            :aria-checked="live"
            class="flex items-center gap-2 rounded-xl border px-3 py-2 text-sm font-medium transition-colors"
            :class="
              live
                ? 'border-emerald-500/50 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300'
                : 'border-border bg-muted/30 text-muted-foreground hover:text-foreground'
            "
            :title="
              live
                ? 'Live: target and Kafka update every few seconds. Click to pause.'
                : 'Paused: click to update every few seconds.'
            "
            @click="live = !live"
          >
            <span class="relative flex h-2 w-2">
              <span
                v-if="live"
                class="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-75"
              />
              <span
                class="relative inline-flex h-2 w-2 rounded-full"
                :class="live ? 'bg-emerald-500' : 'bg-muted-foreground/40'"
              />
            </span>
            {{ live ? "Live" : "Paused" }}
          </button>
          <AppButton
            variant="secondary"
            :disabled="loading"
            title="Count source and target rows now (source COUNT(*) can be slow on large tables)"
            @click="refresh"
          >
            <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
            Refresh
          </AppButton>
        </div>
      </div>

      <!-- How much of the configured capture set is in sync -->
      <div v-if="syncSummary.total" class="rounded-xl border border-border bg-muted/20 px-4 py-3">
        <div class="flex flex-wrap items-baseline justify-between gap-2 text-sm">
          <p>
            <span class="text-base font-semibold text-foreground">{{ syncSummary.inSync.toLocaleString() }}</span>
            <span class="text-muted-foreground"> of </span>
            <span class="font-semibold text-foreground">{{ syncSummary.total.toLocaleString() }}</span>
            <span class="text-muted-foreground"> configured tables in sync</span>
            <span class="ml-1 font-medium" :class="syncSummary.pct === 100 ? 'text-emerald-600 dark:text-emerald-400' : 'text-foreground'">
              ({{ syncSummary.pct }}%)
            </span>
          </p>
          <p class="font-mono text-xs text-muted-foreground">
            Target {{ fmtCount(syncSummary.targetRows) }} rows
            <template v-if="syncSummary.sourceRows != null"> · Source {{ fmtCount(syncSummary.sourceRows) }} rows</template>
          </p>
        </div>
        <div class="mt-2 h-2 overflow-hidden rounded-full bg-muted">
          <div
            class="h-full rounded-full transition-all duration-500"
            :class="syncSummary.pct === 100 ? 'bg-emerald-500' : 'bg-brand-500'"
            :style="{ width: `${syncSummary.pct}%` }"
          />
        </div>
        <p v-if="pagedPolling && live" class="mt-1.5 text-[11px] text-muted-foreground">
          Live updates the tables on this page; other pages keep their numbers from the last Refresh.
        </p>
      </div>

      <div class="grid grid-cols-1 gap-3 md:grid-cols-2">
        <label class="block">
          <span class="auth-label">Filter status</span>
          <select v-model="phaseFilter" class="auth-input w-full">
            <option value="">All statuses</option>
            <option value="complete">Complete</option>
            <option value="in_progress">Syncing</option>
            <option value="behind">Catching up</option>
            <option value="error">Error</option>
            <option value="unknown">Unknown</option>
          </select>
        </label>
        <label class="block">
          <span class="auth-label">Search tables</span>
          <div class="relative">
            <Search
              class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground"
              :stroke-width="1.75"
            />
            <input
              v-model="search"
              class="auth-input w-full pl-9"
              placeholder="Schema, table, topic…"
            />
          </div>
        </label>
      </div>

      <div v-if="loading && !data" class="space-y-2">
        <div v-for="i in 4" :key="i" class="h-12 animate-pulse rounded-xl bg-muted/50" />
      </div>

      <AppEmptyState
        v-else-if="!filteredRows.length"
        title="No matching tables"
        description="Adjust filters or confirm table mappings on the pipeline."
      />

      <AppTable v-else table-class="min-w-[720px]">
        <thead>
          <tr>
            <th class="w-8" />
            <th>Table</th>
            <th class="min-w-[8rem]">Progress</th>
            <th>Status</th>
            <th class="text-right tabular-nums">Source</th>
            <th class="text-right tabular-nums">Processing</th>
            <th class="text-right tabular-nums">Target</th>
            <th class="text-right tabular-nums">Pending</th>
          </tr>
        </thead>
        <tbody>
          <template v-for="row in pagedRows" :key="rowKey(row)">
            <tr
              class="cursor-pointer transition-colors hover:bg-muted/40"
              @click="toggleRow(rowKey(row))"
            >
              <td class="text-muted-foreground">
                <ChevronDown v-if="expanded[rowKey(row)]" class="h-4 w-4" />
                <ChevronRight v-else class="h-4 w-4" />
              </td>
              <td>
                <div class="font-mono text-xs font-medium text-foreground">
                  {{ row.source_qualified }}
                </div>
                <div
                  v-if="row.target_qualified !== row.source_qualified"
                  class="mt-0.5 font-mono text-[11px] text-muted-foreground"
                >
                  → {{ row.target_qualified }}
                </div>
              </td>
              <td>
                <div class="flex items-center gap-2">
                  <div class="h-2 min-w-[5rem] flex-1 overflow-hidden rounded-full bg-muted">
                    <div
                      class="h-full rounded-full transition-all duration-300"
                      :class="progressTone(progressPct(row), row.phase)"
                      :style="{ width: `${progressPct(row) ?? (row.phase === 'complete' ? 100 : 8)}%` }"
                    />
                  </div>
                  <span class="w-10 text-right text-xs tabular-nums text-muted-foreground">
                    {{ progressPct(row) != null ? `${progressPct(row)}%` : "—" }}
                  </span>
                </div>
              </td>
              <td>
                <AppStatusBadge :status="rowBadgeStatus(row)" :label="rowPhaseLabel(row)" />
              </td>
              <td class="text-right font-mono text-xs tabular-nums">
                {{ fmtCount(row.source_row_count) }}
              </td>
              <td class="text-right font-mono text-xs tabular-nums">
                {{ fmtCount(row.processing_rows ?? row.kafka_messages) }}
              </td>
              <td class="text-right font-mono text-xs tabular-nums">
                <span
                  class="rounded px-1 transition-colors duration-700"
                  :class="bumped[rowKey(row)] ? 'bg-emerald-500/15 text-emerald-700 dark:text-emerald-300' : ''"
                >
                  {{ fmtCount(row.target_row_count) }}
                </span>
                <div
                  v-if="live && rowRate(row) > 0.5"
                  class="mt-0.5 text-[11px] text-emerald-600 dark:text-emerald-400"
                >
                  +{{ fmtRate(rowRate(row)) }}<template v-if="rowEta(row)"> · {{ rowEta(row) }} left</template>
                </div>
              </td>
              <td class="text-right font-mono text-xs tabular-nums">
                <span
                  :class="
                    (row.pending_rows ?? 0) > 0 && !pipelineIdle
                      ? 'font-semibold text-amber-600 dark:text-amber-400'
                      : ''
                  "
                >
                  {{ fmtCount(row.pending_rows) }}
                </span>
              </td>
            </tr>
            <tr v-if="expanded[rowKey(row)]">
              <td colspan="8" class="bg-muted/20 p-0">
                <div class="grid gap-3 p-4 md:grid-cols-2">
                  <div class="space-y-1 text-sm">
                    <p class="text-xs font-medium text-muted-foreground">Capture topic</p>
                    <p class="font-mono text-xs">{{ row.kafka_topic || "—" }}</p>
                    <p v-if="topicMissing(row)" class="flex items-start gap-1.5 text-xs text-muted-foreground">
                      <Info class="mt-0.5 h-3.5 w-3.5 shrink-0" :stroke-width="1.75" />
                      <span>{{ missingTopicNote(row) }}</span>
                    </p>
                    <template v-else-if="row.kafka_error">
                      <ErrorDiagnosis
                        v-if="row.kafka_diagnosis"
                        class="pt-1"
                        compact
                        :diagnosis="row.kafka_diagnosis"
                        context="Capture topic"
                        :detail="row.kafka_error"
                      />
                      <p v-else class="text-xs text-red-500">{{ row.kafka_error }}</p>
                    </template>
                    <template v-if="row.source_count_error">
                      <ErrorDiagnosis
                        v-if="row.source_count_diagnosis"
                        class="pt-2"
                        compact
                        :diagnosis="row.source_count_diagnosis"
                        context="Counting source rows"
                        :detail="row.source_count_error"
                      />
                      <p v-else class="text-xs text-red-500">Source: {{ row.source_count_error }}</p>
                    </template>
                    <template v-if="row.target_count_error">
                      <ErrorDiagnosis
                        v-if="row.target_count_diagnosis"
                        class="pt-2"
                        compact
                        :diagnosis="row.target_count_diagnosis"
                        context="Counting target rows"
                        :detail="row.target_count_error"
                      />
                      <p v-else class="text-xs text-red-500">Target: {{ row.target_count_error }}</p>
                    </template>
                  </div>
                  <div v-if="row.poll_rule">
                    <p class="mb-1 text-xs font-medium text-muted-foreground">Poll strategy</p>
                    <p class="text-sm">{{ pollRuleLabel(row.poll_rule) }}</p>
                    <p class="mt-1 text-xs text-muted-foreground">
                      Re-read on every poll; no JDBC offset is kept.
                      <template v-if="row.poll_rule.strategy === 'rolling_window'">
                        Older rows reach the target only through Initial load.
                      </template>
                    </p>
                  </div>
                  <div v-else-if="logBased">
                    <p class="mb-1 text-xs font-medium text-muted-foreground">Capture position</p>
                    <p class="text-sm">
                      {{ String(data?.capture_mode).toLowerCase() === "synconce" ? "One-time snapshot" : "Streams from the database log (WAL)" }}
                    </p>
                    <p class="mt-1 text-xs text-muted-foreground">
                      Debezium keeps one log position for the whole pipeline, not per table.
                    </p>
                  </div>
                  <div v-else>
                    <p class="mb-1 text-xs font-medium text-muted-foreground">JDBC watermark</p>
                    <pre class="code-block max-h-40 overflow-auto p-3 text-[11px]">{{
                      row.source_offset
                        ? JSON.stringify(row.source_offset, null, 2)
                        : pipelineIdle
                          ? "No offset yet — set when the pipeline runs"
                          : "No offset yet"
                    }}</pre>
                  </div>
                </div>
              </td>
            </tr>
          </template>
        </tbody>
      </AppTable>

      <!-- Pagination -->
      <div
        v-if="filteredRows.length > PAGE_SIZES[0]"
        class="flex flex-wrap items-center justify-between gap-3 text-sm"
      >
        <p class="text-muted-foreground">
          Showing <span class="font-medium text-foreground">{{ pageFrom.toLocaleString() }}–{{ pageTo.toLocaleString() }}</span>
          of <span class="font-medium text-foreground">{{ filteredRows.length.toLocaleString() }}</span> tables
        </p>
        <div class="flex flex-wrap items-center gap-3">
          <label class="flex items-center gap-2 text-muted-foreground">
            Rows per page
            <select v-model.number="pageSize" class="auth-input h-8 w-20 py-0">
              <option v-for="n in PAGE_SIZES" :key="n" :value="n">{{ n }}</option>
            </select>
          </label>
          <nav class="flex items-center gap-1" aria-label="Table pages">
            <button
              type="button"
              class="inline-flex h-8 w-8 items-center justify-center rounded-lg border border-border disabled:opacity-40"
              :disabled="page <= 1"
              aria-label="Previous page"
              @click="page--"
            >
              <ChevronLeft class="h-4 w-4" :stroke-width="1.75" />
            </button>
            <template v-for="(b, i) in pageButtons" :key="`${b}-${i}`">
              <span v-if="b === 'gap'" class="px-1 text-muted-foreground">…</span>
              <button
                v-else
                type="button"
                class="h-8 min-w-8 rounded-lg border px-2 text-xs font-medium tabular-nums"
                :class="
                  b === page
                    ? 'border-primary bg-primary/10 text-primary'
                    : 'border-border text-muted-foreground hover:text-foreground'
                "
                :aria-current="b === page ? 'page' : undefined"
                @click="page = b"
              >
                {{ b }}
              </button>
            </template>
            <button
              type="button"
              class="inline-flex h-8 w-8 items-center justify-center rounded-lg border border-border disabled:opacity-40"
              :disabled="page >= totalPages"
              aria-label="Next page"
              @click="page++"
            >
              <ChevronRight class="h-4 w-4" :stroke-width="1.75" />
            </button>
          </nav>
        </div>
      </div>
    </div>
  </div>
</template>
