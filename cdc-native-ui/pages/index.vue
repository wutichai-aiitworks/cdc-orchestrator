<script setup lang="ts">
import {
  AlertCircle,
  AlertTriangle,
  ArrowRight,
  Cable,
  CheckCircle2,
  ChevronRight,
  CircleDashed,
  List,
  Pause,
  RefreshCw,
  Server,
  ShieldCheck,
  Workflow,
  Timer,
} from "@lucide/vue"
import type { Component } from "vue"
import { formatMs, type LatencyCurrent } from "~/utils/latency"
import type { CdcProfile } from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"

const { fetchEngineStatus } = useControlPlane()
const { fetchConnectors } = useConnectorsM2()
const { fetchTopics } = useKafkaM1()
const { fetchProfiles } = useProfilesM3()

const AUTO_REFRESH_MS = 30_000

const engine = ref<EngineStatus | null>(null)
const connectors = ref<Record<string, any>>({})
const topics = ref(0)
const profiles = ref<CdcProfile[]>([])
const loading = ref(false)
const loaded = ref(false)
/** The topic count comes from the broker and lands after the rest; its card waits alone. */
const topicsLoaded = ref(false)
const { fetchLatency } = useLatency()
const latencies = ref<LatencyCurrent[]>([])
/** Highest current end-to-end latency among running pipelines. */
const worstLatency = computed(() =>
  latencies.value
    .filter((l) => l.e2e_ms !== null && l.status !== "stopped")
    .sort((a, b) => (b.e2e_ms ?? 0) - (a.e2e_ms ?? 0))[0] ?? null,
)
const error = ref("")
const lastUpdatedAt = ref<number | null>(null)
const now = ref(Date.now())
const autoRefresh = ref(true)

// ── connector helpers ──────────────────────────────────────────────
type ConnState = "RUNNING" | "PAUSED" | "FAILED" | "STOPPED" | "MISSING" | "UNKNOWN"

const connectorState = (name?: string | null): ConnState => {
  if (!name) return "MISSING"
  const entry = connectors.value[name]
  if (!entry) return "MISSING"
  const state = String(entry?.status?.connector?.state || "").toUpperCase()
  if (["RUNNING", "PAUSED", "FAILED", "STOPPED"].includes(state)) return state as ConnState
  return "UNKNOWN"
}

const taskStats = (name?: string | null) => {
  const tasks: Array<{ state?: string }> = (name && connectors.value[name]?.status?.tasks) || []
  const failed = tasks.filter((t) => String(t.state).toUpperCase() === "FAILED").length
  const running = tasks.filter((t) => String(t.state).toUpperCase() === "RUNNING").length
  return { total: tasks.length, running, failed }
}

const connectorEntries = computed(() => Object.keys(connectors.value).sort())
const connectorCounts = computed(() => {
  const counts = { running: 0, paused: 0, failed: 0, other: 0 }
  for (const name of connectorEntries.value) {
    const state = connectorState(name)
    if (state === "FAILED" || taskStats(name).failed) counts.failed++
    else if (state === "RUNNING") counts.running++
    else if (state === "PAUSED") counts.paused++
    else counts.other++
  }
  return counts
})

// ── services ───────────────────────────────────────────────────────
const services = computed(() => engine.value?.services || [])
// One-shot init/job containers that exited cleanly ("Exited (0) …") are complete, not down.
const isServiceUp = (s: EngineService) => {
  const state = s.state.toLowerCase()
  if (/exited\s*\(0\)/.test(`${state} ${String(s.health || "").toLowerCase()}`)) return true
  return state.includes("running") && !String(s.health || "").toLowerCase().includes("unhealthy")
}
const servicesDown = computed(() => services.value.filter((s) => !isServiceUp(s)))
const servicesUp = computed(() => services.value.length - servicesDown.value.length)

// ── pipelines (profile ⇄ source/sink connectors) ──────────────────
type Health = "failed" | "paused" | "running" | "idle"

const pipelineRows = computed(() =>
  profiles.value
    .map((p) => {
      const sourceState = connectorState(p.connector_name)
      const sinkState = p.sink_connector_name ? connectorState(p.sink_connector_name) : null
      const src = taskStats(p.connector_name)
      const snk = taskStats(p.sink_connector_name)
      const states = [sourceState, sinkState].filter(Boolean) as ConnState[]
      const taskFailures = src.failed + snk.failed
      let health: Health = "idle"
      if (states.includes("FAILED") || taskFailures) health = "failed"
      else if (states.includes("PAUSED")) health = "paused"
      else if (states.length && states.every((s) => s === "RUNNING")) health = "running"
      return {
        profile: p,
        sourceState,
        sinkState,
        tasksRunning: src.running + snk.running,
        tasksTotal: src.total + snk.total,
        taskFailures,
        health,
      }
    })
    .sort((a, b) => healthRank[a.health] - healthRank[b.health] || a.profile.name.localeCompare(b.profile.name))
)
const healthRank: Record<Health, number> = { failed: 0, paused: 1, idle: 2, running: 3 }

const managedConnectorNames = computed(() => {
  const set = new Set<string>()
  for (const p of profiles.value) {
    if (p.connector_name) set.add(p.connector_name)
    if (p.sink_connector_name) set.add(p.sink_connector_name)
  }
  return set
})
const unmanagedConnectors = computed(() => connectorEntries.value.filter((n) => !managedConnectorNames.value.has(n)))

const pipelineCounts = computed(() => {
  const c = { running: 0, paused: 0, failed: 0, idle: 0 }
  for (const row of pipelineRows.value) c[row.health]++
  return c
})

// ── attention feed ─────────────────────────────────────────────────
type Severity = "critical" | "warning" | "info"
type AttentionItem = { key: string; severity: Severity; title: string; detail: string; to: string; mono?: boolean }

const attention = computed<AttentionItem[]>(() => {
  const items: AttentionItem[] = []
  if (loaded.value && !engine.value) {
    items.push({ key: "engine", severity: "critical", title: "Control plane unreachable", detail: "Engine status could not be loaded.", to: "/infrastructure" })
  }
  for (const s of servicesDown.value) {
    items.push({ key: `svc:${s.name}`, severity: "critical", title: s.name, detail: `Service ${s.health || s.state}`, to: "/infrastructure", mono: true })
  }
  for (const row of pipelineRows.value) {
    const p = row.profile
    if (row.health === "failed") {
      items.push({
        key: `p:${p.id}`,
        severity: "critical",
        title: p.name,
        detail: row.taskFailures ? `${row.taskFailures} task${row.taskFailures > 1 ? "s" : ""} failed` : p.last_error || "Connector failed",
        to: "/pipelines",
      })
    } else if (row.health === "paused") {
      items.push({ key: `p:${p.id}`, severity: "warning", title: p.name, detail: "Pipeline paused — changes are not being captured", to: "/pipelines" })
    }
  }
  for (const name of unmanagedConnectors.value) {
    const state = connectorState(name)
    if (state === "FAILED" || taskStats(name).failed) {
      items.push({ key: `c:${name}`, severity: "critical", title: name, detail: "Unmanaged connector failed", to: "/connectors", mono: true })
    }
  }
  const rank: Record<Severity, number> = { critical: 0, warning: 1, info: 2 }
  return items.sort((a, b) => rank[a.severity] - rank[b.severity])
})

const overall = computed(() => {
  if (!loaded.value) return { tone: "neutral", title: "Checking system status…", icon: CircleDashed }
  const critical = attention.value.filter((i) => i.severity === "critical").length
  const warning = attention.value.filter((i) => i.severity === "warning").length
  if (critical) return { tone: "critical", title: `${critical} issue${critical > 1 ? "s" : ""} need attention`, icon: AlertCircle }
  if (warning) return { tone: "warning", title: `Operational — ${warning} pipeline${warning > 1 ? "s" : ""} paused`, icon: AlertTriangle }
  return { tone: "healthy", title: "All systems operational", icon: CheckCircle2 }
})

const bannerClass: Record<string, string> = {
  neutral: "border-border bg-muted/40 text-muted-foreground",
  critical: "border-red-500/30 bg-red-500/5 text-red-700 dark:text-red-400",
  warning: "border-amber-500/30 bg-amber-500/5 text-amber-700 dark:text-amber-400",
  healthy: "border-emerald-500/30 bg-emerald-500/5 text-emerald-700 dark:text-emerald-400",
}

// ── KPI tiles ──────────────────────────────────────────────────────
type Segment = { value: number; class: string; label: string }
type Kpi = { label: string; value: number | string; total?: number; pending?: boolean; icon: Component; to: string; segments: Segment[]; footnote: string }

const kpis = computed<Kpi[]>(() => [
  {
    label: "Runtime services",
    value: servicesUp.value,
    total: services.value.length,
    icon: Server,
    to: "/infrastructure",
    segments: [
      { value: servicesUp.value, class: "bg-emerald-500", label: "up" },
      { value: servicesDown.value.length, class: "bg-red-500", label: "down" },
    ],
    footnote: servicesDown.value.length
      ? `${servicesDown.value.length} down · ${servicesDown.value.map((s) => s.name).slice(0, 2).join(", ")}`
      : "All services healthy",
  },
  {
    label: "CDC pipelines",
    value: profiles.value.length,
    icon: Workflow,
    to: "/pipelines",
    segments: [
      { value: pipelineCounts.value.running, class: "bg-emerald-500", label: "running" },
      { value: pipelineCounts.value.paused, class: "bg-amber-400", label: "paused" },
      { value: pipelineCounts.value.failed, class: "bg-red-500", label: "failed" },
      { value: pipelineCounts.value.idle, class: "bg-muted-foreground/30", label: "idle" },
    ],
    footnote: `${pipelineCounts.value.running} running · ${pipelineCounts.value.paused} paused · ${pipelineCounts.value.failed} failed`,
  },
  {
    label: "Connectors",
    value: connectorEntries.value.length,
    icon: Cable,
    to: "/connectors",
    segments: [
      { value: connectorCounts.value.running, class: "bg-emerald-500", label: "running" },
      { value: connectorCounts.value.paused, class: "bg-amber-400", label: "paused" },
      { value: connectorCounts.value.failed, class: "bg-red-500", label: "failed" },
      { value: connectorCounts.value.other, class: "bg-muted-foreground/30", label: "other" },
    ],
    footnote: unmanagedConnectors.value.length
      ? `${unmanagedConnectors.value.length} not linked to a pipeline`
      : `${connectorCounts.value.running} running · ${connectorCounts.value.paused} paused`,
  },
  {
    label: "Kafka topics",
    value: topics.value,
    icon: List,
    to: "/topics",
    pending: !topicsLoaded.value,
    segments: [],
    footnote: "Event streams on the cluster",
  },
  {
    label: "Worst latency",
    value: worstLatency.value ? formatMs(worstLatency.value.e2e_ms) : "—",
    icon: Timer,
    to: worstLatency.value ? `/pipelines?pipeline=${worstLatency.value.profile_id}` : "/pipelines",
    segments: [],
    footnote: worstLatency.value
      ? `${worstLatency.value.name} · ${latencies.value.filter((l) => l.e2e_ms !== null).length} measured`
      : "No running pipeline measured yet",
  },
])

const segmentWidth = (seg: Segment, segs: Segment[]) => {
  const sum = segs.reduce((acc, s) => acc + s.value, 0)
  return sum ? `${(seg.value / sum) * 100}%` : "0%"
}

// ── readiness ──────────────────────────────────────────────────────
type Check = { label: string; detail: string; status: "ok" | "warn" | "fail" }
const readiness = computed<Check[]>(() => [
  { label: "Control plane", detail: engine.value ? `Connected · ${engine.value.engine_target}` : "Unavailable", status: engine.value ? "ok" : "fail" },
  {
    label: "CDC runtime",
    detail: services.value.length ? `${servicesUp.value} of ${services.value.length} services up` : "Not running",
    status: !services.value.length ? "fail" : servicesDown.value.length ? "warn" : "ok",
  },
  {
    label: "Kafka Connect",
    detail: connectorEntries.value.length ? `${connectorEntries.value.length} connectors registered` : "No connectors",
    status: connectorCounts.value.failed ? "fail" : connectorEntries.value.length ? "ok" : "warn",
  },
  {
    label: "Pipelines",
    detail: profiles.value.length ? `${profiles.value.filter((p) => p.enabled).length} of ${profiles.value.length} enabled` : "Setup required",
    status: profiles.value.length ? "ok" : "warn",
  },
])
const checkIcon = { ok: CheckCircle2, warn: AlertTriangle, fail: AlertCircle }
const checkClass = {
  ok: "text-emerald-600 dark:text-emerald-400",
  warn: "text-amber-600 dark:text-amber-400",
  fail: "text-red-600 dark:text-red-400",
}
const severityDot: Record<Severity, string> = { critical: "bg-red-500", warning: "bg-amber-400", info: "bg-blue-500" }
const healthStripe: Record<Health, string> = {
  failed: "bg-red-500",
  paused: "bg-amber-400",
  running: "bg-emerald-500",
  idle: "bg-muted-foreground/30",
}

const captureLabel = (p: CdcProfile) =>
  ({ logical: "Log-based", trigger: "Trigger", sync_once: "One-time sync", incremental: "Incremental" } as Record<string, string>)[
    p.source?.capture_mode || ""
  ] || "CDC"

// ── refresh lifecycle ──────────────────────────────────────────────
const refresh = async () => {
  if (loading.value) return
  loading.value = true
  error.value = ""
  const topicsRequest = fetchTopics({ page: 1, page_size: 1, summary: true }).then((r) => {
    topics.value = r.total
    topicsLoaded.value = true
  })
  fetchLatency()
    .then((r) => (latencies.value = r.items))
    .catch(() => {})
  const results = await Promise.allSettled([fetchEngineStatus(), fetchConnectors(), fetchProfiles()])
  if (results[0].status === "fulfilled") engine.value = results[0].value
  else engine.value = null
  if (results[1].status === "fulfilled") connectors.value = results[1].value.connectors || {}
  if (results[2].status === "fulfilled") profiles.value = results[2].value
  // Show the page now; don't hold it for the broker.
  loaded.value = true
  const [topicsResult] = await Promise.allSettled([topicsRequest])
  const failed = [...results, topicsResult].find((result) => result.status === "rejected")
  if (failed?.status === "rejected") error.value = formatApiError(failed.reason, "Some overview data could not be loaded")
  lastUpdatedAt.value = Date.now()
  loading.value = false
}

const updatedLabel = computed(() => {
  if (!lastUpdatedAt.value) return "Not yet updated"
  const secs = Math.max(0, Math.round((now.value - lastUpdatedAt.value) / 1000))
  if (secs < 5) return "Updated just now"
  if (secs < 60) return `Updated ${secs}s ago`
  return `Updated ${Math.floor(secs / 60)}m ago`
})

let tick: ReturnType<typeof setInterval> | undefined
let poll: ReturnType<typeof setInterval> | undefined
onMounted(() => {
  refresh()
  tick = setInterval(() => (now.value = Date.now()), 1000)
  poll = setInterval(() => {
    if (autoRefresh.value && !document.hidden) refresh()
  }, AUTO_REFRESH_MS)
})
onBeforeUnmount(() => {
  clearInterval(tick)
  clearInterval(poll)
})
</script>

<template>
  <div class="page-container !space-y-5">
    <!-- Header -->
    <section class="flex flex-wrap items-end justify-between gap-4">
      <div class="min-w-0">
        <h1 class="page-title">Overview</h1>
        <p class="page-description">Change data capture, Kafka Connect and runtime health at a glance.</p>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <button
          type="button"
          class="inline-flex items-center gap-2 rounded-lg px-2 py-1 text-xs text-muted-foreground hover:bg-accent focus:outline-none focus:ring-1 focus:ring-ring"
          :title="autoRefresh ? 'Auto-refresh every 30s — click to pause' : 'Auto-refresh paused — click to resume'"
          @click="autoRefresh = !autoRefresh"
        >
          <span class="relative flex h-2 w-2">
            <span v-if="autoRefresh" class="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-60 motion-reduce:animate-none" />
            <span :class="['relative inline-flex h-2 w-2 rounded-full', autoRefresh ? 'bg-emerald-500' : 'bg-muted-foreground/50']" />
          </span>
          <span class="tabular-nums">{{ updatedLabel }}</span>
          <span v-if="!autoRefresh">· paused</span>
        </button>
        <AppButton :disabled="loading" @click="refresh">
          <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
          Refresh
        </AppButton>
        <AppButton as="link" to="/pipelines" variant="primary">
          <Workflow class="h-4 w-4" :stroke-width="1.75" />
          Create Pipeline
        </AppButton>
      </div>
    </section>

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <!-- Overall status banner -->
    <section :class="['flex flex-wrap items-center gap-x-6 gap-y-2 rounded-xl border px-4 py-3', bannerClass[overall.tone]]">
      <div class="flex items-center gap-2.5">
        <component :is="overall.icon" class="h-5 w-5" :stroke-width="1.75" />
        <p class="text-sm font-semibold">{{ overall.title }}</p>
      </div>
      <div v-if="loaded" class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
        <span class="inline-flex items-center gap-1.5"><span class="h-1.5 w-1.5 rounded-full bg-emerald-500" />{{ pipelineCounts.running }} streaming</span>
        <span class="inline-flex items-center gap-1.5"><span class="h-1.5 w-1.5 rounded-full bg-amber-400" />{{ pipelineCounts.paused }} paused</span>
        <span class="inline-flex items-center gap-1.5"><span class="h-1.5 w-1.5 rounded-full bg-red-500" />{{ pipelineCounts.failed + servicesDown.length }} failing</span>
      </div>
    </section>

    <!-- KPI tiles -->
    <section class="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-5">
      <NuxtLink
        v-for="kpi in kpis"
        :key="kpi.label"
        :to="kpi.to"
        class="stat-card group flex flex-col gap-3 !p-4 transition-colors hover:border-primary/40 focus:outline-none focus:ring-1 focus:ring-ring"
      >
        <div class="flex items-center justify-between">
          <p class="text-xs font-medium uppercase tracking-wide text-muted-foreground">{{ kpi.label }}</p>
          <component :is="kpi.icon" class="h-4 w-4 text-muted-foreground transition-colors group-hover:text-primary" :stroke-width="1.75" />
        </div>
        <p class="font-display text-3xl font-semibold leading-none tabular-nums text-foreground">
          <span v-if="!loaded || kpi.pending" class="inline-block h-7 w-12 animate-pulse rounded bg-muted align-middle" />
          <template v-else>
            {{ kpi.value }}<span v-if="kpi.total !== undefined" class="text-lg font-medium text-muted-foreground"> / {{ kpi.total }}</span>
          </template>
        </p>
        <div class="mt-auto space-y-2">
          <div v-if="kpi.segments.length" class="flex h-1.5 w-full overflow-hidden rounded-full bg-muted">
            <span
              v-for="seg in kpi.segments"
              :key="seg.label"
              :class="seg.class"
              :style="{ width: segmentWidth(seg, kpi.segments) }"
              :title="`${seg.value} ${seg.label}`"
            />
          </div>
          <div v-else class="h-1.5" />
          <p class="truncate text-xs text-muted-foreground" :title="kpi.footnote">{{ kpi.footnote }}</p>
        </div>
      </NuxtLink>
    </section>

    <!-- Getting started (no pipelines yet) -->
    <section v-if="loaded && !profiles.length" class="rounded-xl border border-brand-500/25 bg-brand-500/5 p-6">
      <div class="grid grid-cols-1 items-center gap-6 lg:grid-cols-[minmax(0,1fr),auto]">
        <div>
          <span class="badge-neutral mb-3">Getting started</span>
          <h2 class="text-lg font-display font-semibold text-foreground">Create your first CDC pipeline</h2>
          <p class="mt-1 text-sm text-muted-foreground">
            The guided setup tests database access, chooses capture scope and generates the connector configuration.
          </p>
          <ol class="mt-5 grid grid-cols-1 gap-3 sm:grid-cols-3">
            <li v-for="(step, i) in ['Connect source', 'Choose scope', 'Start capture']" :key="step" class="flex items-center gap-3">
              <span class="flex h-7 w-7 items-center justify-center rounded-full bg-primary text-sm font-semibold text-primary-foreground">{{ i + 1 }}</span>
              <span class="text-sm font-medium">{{ step }}</span>
            </li>
          </ol>
        </div>
        <AppButton as="link" to="/pipelines" variant="primary">
          Start guided setup <ArrowRight class="h-4 w-4" :stroke-width="1.75" />
        </AppButton>
      </div>
    </section>

    <!-- Main grid -->
    <section class="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,2fr),minmax(320px,1fr)]">
      <!-- Pipelines -->
      <div class="stat-card !p-0">
        <div class="flex items-center justify-between gap-3 border-b border-border px-4 py-3">
          <AppSectionTitle title="Pipelines" hint="Each pipeline pairs a source connector with its sink. Sorted by severity." />
          <NuxtLink to="/pipelines" class="btn-ghost inline-flex items-center gap-1">
            View all <ChevronRight class="h-4 w-4" :stroke-width="1.75" />
          </NuxtLink>
        </div>

        <div v-if="!loaded" class="space-y-2 p-4">
          <div v-for="i in 4" :key="i" class="h-12 animate-pulse rounded-lg bg-muted/60" />
        </div>
        <AppEmptyState
          v-else-if="!pipelineRows.length"
          title="No pipelines yet"
          description="Create a CDC pipeline to start streaming changes."
        >
          <template #icon><Workflow class="h-4 w-4" :stroke-width="1.75" /></template>
        </AppEmptyState>
        <table v-else class="w-full text-sm">
          <thead>
            <tr class="border-b border-border text-left text-xs font-medium text-muted-foreground">
              <th class="py-2.5 pl-4 pr-2 font-medium">Pipeline</th>
              <th class="hidden px-2 py-2.5 font-medium md:table-cell">Route</th>
              <th class="px-2 py-2.5 font-medium">Source</th>
              <th class="px-2 py-2.5 font-medium">Sink</th>
              <th class="py-2.5 pl-2 pr-4 text-right font-medium">Tasks</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="row in pipelineRows.slice(0, 10)"
              :key="row.profile.id"
              class="border-b border-border/50 transition-colors last:border-0 hover:bg-accent"
            >
              <td class="relative py-3 pl-4 pr-2">
                <span :class="['absolute inset-y-2 left-0 w-0.5 rounded-r', healthStripe[row.health]]" />
                <NuxtLink to="/pipelines" class="font-medium text-foreground hover:text-primary hover:underline underline-offset-4">
                  {{ row.profile.name }}
                </NuxtLink>
                <p class="mt-0.5 truncate font-mono text-xs text-muted-foreground">
                  {{ row.profile.connector_name || "not deployed" }} · {{ captureLabel(row.profile) }}
                </p>
              </td>
              <td class="hidden px-2 py-3 md:table-cell">
                <div class="flex items-center gap-1.5">
                  <AppDbIcon :type="row.profile.source?.source_type" size="sm" />
                  <ArrowRight class="h-3.5 w-3.5 text-muted-foreground" :stroke-width="1.75" />
                  <AppDbIcon v-if="row.profile.target" :type="row.profile.target.target_type" size="sm" />
                  <span v-else class="text-xs text-muted-foreground">{{ row.profile.lake ? "Lake" : "Topic" }}</span>
                </div>
              </td>
              <td class="px-2 py-3">
                <AppStatusBadge v-if="row.sourceState !== 'MISSING'" :status="row.sourceState" />
                <span v-else class="text-xs text-muted-foreground">—</span>
              </td>
              <td class="px-2 py-3">
                <AppStatusBadge v-if="row.sinkState && row.sinkState !== 'MISSING'" :status="row.sinkState" />
                <span v-else class="text-xs text-muted-foreground">—</span>
              </td>
              <td class="py-3 pl-2 pr-4 text-right tabular-nums">
                <span :class="row.taskFailures ? 'font-medium text-red-600 dark:text-red-400' : 'text-foreground'">
                  {{ row.tasksRunning }}/{{ row.tasksTotal }}
                </span>
              </td>
            </tr>
          </tbody>
        </table>

        <div
          v-if="unmanagedConnectors.length"
          class="flex items-center justify-between gap-3 border-t border-border px-4 py-2.5 text-xs text-muted-foreground"
        >
          <span>
            {{ unmanagedConnectors.length }} connector{{ unmanagedConnectors.length > 1 ? "s" : "" }} not managed by a pipeline:
            <span class="font-mono">{{ unmanagedConnectors.slice(0, 3).join(", ") }}<template v-if="unmanagedConnectors.length > 3">…</template></span>
          </span>
          <NuxtLink to="/connectors" class="btn-ghost text-xs">Manage connectors</NuxtLink>
        </div>
      </div>

      <!-- Right rail -->
      <div class="flex flex-col gap-4">
        <!-- Needs attention -->
        <div class="stat-card !p-0">
          <div class="flex items-center justify-between gap-3 border-b border-border px-4 py-3">
            <AppSectionTitle title="Needs attention" hint="Failed services, failed tasks and paused pipelines, most severe first." />
            <span v-if="attention.length" class="badge-neutral !px-2 !py-0.5 !text-xs tabular-nums">{{ attention.length }}</span>
          </div>
          <div v-if="loaded && !attention.length" class="flex items-center gap-3 px-4 py-5">
            <span class="flex h-8 w-8 items-center justify-center rounded-full bg-emerald-500/10 text-emerald-600 dark:text-emerald-400">
              <ShieldCheck class="h-4 w-4" :stroke-width="1.75" />
            </span>
            <div>
              <p class="text-sm font-medium">Nothing needs attention</p>
              <p class="text-xs text-muted-foreground">All pipelines and services are healthy.</p>
            </div>
          </div>
          <ul v-else class="max-h-72 divide-y divide-border/60 overflow-auto">
            <li v-for="item in attention" :key="item.key">
              <NuxtLink :to="item.to" class="group flex items-center gap-3 px-4 py-2.5 transition-colors hover:bg-accent">
                <span :class="['h-2 w-2 shrink-0 rounded-full', severityDot[item.severity]]" />
                <div class="min-w-0 flex-1">
                  <p :class="['truncate text-sm text-foreground', item.mono ? 'font-mono' : 'font-medium']">{{ item.title }}</p>
                  <p class="truncate text-xs text-muted-foreground">{{ item.detail }}</p>
                </div>
                <Pause v-if="item.severity === 'warning'" class="h-3.5 w-3.5 text-amber-500" :stroke-width="1.75" />
                <ChevronRight class="h-4 w-4 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100" :stroke-width="1.75" />
              </NuxtLink>
            </li>
          </ul>
        </div>

        <!-- Readiness -->
        <div class="stat-card !p-0">
          <div class="border-b border-border px-4 py-3">
            <AppSectionTitle title="System readiness" hint="Essential checks before starting capture." />
          </div>
          <ul class="divide-y divide-border/60">
            <li v-for="check in readiness" :key="check.label" class="flex items-center gap-3 px-4 py-2.5">
              <component :is="checkIcon[check.status]" :class="['h-4 w-4 shrink-0', checkClass[check.status]]" :stroke-width="1.75" />
              <p class="flex-1 text-sm font-medium text-foreground">{{ check.label }}</p>
              <p class="truncate text-xs text-muted-foreground">{{ check.detail }}</p>
            </li>
          </ul>
        </div>
      </div>
    </section>
  </div>
</template>
