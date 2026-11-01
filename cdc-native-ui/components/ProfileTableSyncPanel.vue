<script setup lang="ts">
import {
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  Database,
  RefreshCw,
  Search,
  Table2,
} from "@lucide/vue"
import type { ProfileTableSyncStatusResponse, TableSyncStatusRow } from "~/composables/useOperationsM3"
import { formatApiError } from "~/utils/apiError"

const props = withDefaults(
  defineProps<{
    profileId: string
    /** When false, caller triggers refresh via exposed `refresh`. */
    autoLoad?: boolean
    compact?: boolean
  }>(),
  { autoLoad: true, compact: false },
)

const { fetchProfileTableSyncStatus } = useOperationsM3()
const toast = useToast()

const loading = ref(false)
const data = ref<ProfileTableSyncStatusResponse | null>(null)
const skipCounts = ref(false)
const search = ref("")
const phaseFilter = ref("")
const expanded = ref<Record<string, boolean>>({})

const fmtCount = (n: number | null | undefined) => {
  if (n == null) return "—"
  return n.toLocaleString()
}

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

const summary = computed(() => {
  const tables = data.value?.tables || []
  return {
    total: tables.length,
    complete: tables.filter((t) => t.phase === "complete").length,
    active: tables.filter((t) => t.phase === "in_progress" || t.phase === "behind").length,
    errors: tables.filter((t) => t.phase === "error").length,
    pendingSum: tables.reduce((acc, t) => acc + (t.pending_rows ?? 0), 0),
  }
})

const lastUpdated = computed(() => {
  if (!data.value?.timestamp) return ""
  try {
    return new Date(data.value.timestamp).toLocaleString()
  } catch {
    return data.value.timestamp
  }
})

const refresh = async () => {
  if (!props.profileId) return
  loading.value = true
  try {
    data.value = await fetchProfileTableSyncStatus(props.profileId, {
      skip_counts: skipCounts.value,
    })
  } catch (e: unknown) {
    data.value = null
    toast.error(formatApiError(e, "Table sync status failed"))
  } finally {
    loading.value = false
  }
}

const toggleRow = (key: string) => {
  expanded.value[key] = !expanded.value[key]
}

watch(
  () => props.profileId,
  (id) => {
    expanded.value = {}
    data.value = null
    if (id && props.autoLoad) void refresh()
  },
  { immediate: true },
)

watch(skipCounts, () => {
  if (props.profileId) void refresh()
})

defineExpose({ refresh, data, loading })
</script>

<template>
  <div class="flex flex-col gap-4" :class="compact ? '' : 'animate-fade-in'">
    <div
      v-if="data?.counts_skipped"
      class="alert-info flex items-start gap-2 rounded-xl px-4 py-3 text-sm"
    >
      <AlertTriangle class="mt-0.5 h-4 w-4 shrink-0" :stroke-width="1.75" />
      <span>
        Fast mode: row counts omitted. Enable full refresh to compare source vs target
        <code class="text-xs">COUNT(*)</code> (can be slow on large Oracle tables).
      </span>
    </div>

    <div class="grid grid-cols-2 gap-3 md:grid-cols-5">
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
        helper="Syncing or catching up"
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
        helper="Σ(source − target)"
        tone="info"
      />
    </div>

    <div class="stat-card space-y-4">
      <div class="flex flex-col gap-3 lg:flex-row lg:items-end lg:justify-between">
        <div>
          <AppSectionTitle
            title="Replica progress by table"
            hint="Compare each source table to its target replica. Processing = Kafka backlog not yet applied; Pending = approximate rows still missing on target. Incremental watermarks may exclude source rows from the total."
          />
          <p v-if="lastUpdated" class="mt-1 text-xs text-muted-foreground">
            Last updated {{ lastUpdated }}
            <span v-if="data?.capture_mode" class="ml-2 font-mono">{{ data.capture_mode }}</span>
          </p>
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <label
            class="flex cursor-pointer items-center gap-2 rounded-xl border border-border bg-muted/30 px-3 py-2 text-sm"
          >
            <input v-model="skipCounts" type="checkbox" class="h-4 w-4 rounded accent-brand-500" />
            Fast refresh
          </label>
          <AppButton variant="secondary" :disabled="loading" @click="refresh">
            <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
            Refresh
          </AppButton>
        </div>
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
          <template v-for="row in filteredRows" :key="rowKey(row)">
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
                <AppStatusBadge :status="badgeStatus(row.phase)" :label="phaseLabel(row.phase)" />
              </td>
              <td class="text-right font-mono text-xs tabular-nums">
                {{ fmtCount(row.source_row_count) }}
              </td>
              <td class="text-right font-mono text-xs tabular-nums">
                {{ fmtCount(row.processing_rows ?? row.kafka_messages) }}
              </td>
              <td class="text-right font-mono text-xs tabular-nums">
                {{ fmtCount(row.target_row_count) }}
              </td>
              <td class="text-right font-mono text-xs tabular-nums">
                <span
                  :class="
                    (row.pending_rows ?? 0) > 0 ? 'font-semibold text-amber-600 dark:text-amber-400' : ''
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
                    <p v-if="row.kafka_error" class="text-xs text-red-500">{{ row.kafka_error }}</p>
                    <p v-if="row.source_count_error" class="text-xs text-red-500">
                      Source: {{ row.source_count_error }}
                    </p>
                    <p v-if="row.target_count_error" class="text-xs text-red-500">
                      Target: {{ row.target_count_error }}
                    </p>
                  </div>
                  <div>
                    <p class="mb-1 text-xs font-medium text-muted-foreground">JDBC watermark</p>
                    <pre class="code-block max-h-40 overflow-auto p-3 text-[11px]">{{
                      row.source_offset
                        ? JSON.stringify(row.source_offset, null, 2)
                        : "No offset yet"
                    }}</pre>
                  </div>
                </div>
              </td>
            </tr>
          </template>
        </tbody>
      </AppTable>
    </div>
  </div>
</template>
