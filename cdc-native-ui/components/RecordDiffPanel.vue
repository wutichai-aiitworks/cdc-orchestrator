<script setup lang="ts">
/**
 * Verify Record for one pipeline: pick a table, compare source vs target by key, and see
 * which rows exist only in the source, only in the target, or in both with different
 * values. Jobs run in the background on the orchestrator (ADR 0013); this panel queues
 * them, polls progress while any is active, and pages / downloads the results.
 */
import {
  ArrowLeftRight,
  Ban,
  Download,
  GitCompareArrows,
  Info,
  Play,
  RefreshCw,
  RotateCcw,
  Trash2,
} from "@lucide/vue"
import type {
  CdcProfile,
  RecordDiffJob,
  RecordDiffKind,
  RecordDiffRow,
  SourceTableColumn,
} from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"

const props = defineProps<{ profile: CdcProfile }>()

const {
  startRecordDiff,
  listRecordDiffs,
  cancelRecordDiff,
  deleteRecordDiff,
  recordDiffRows,
  recordDiffDownloadUrl,
  inspectSourceTable,
} = useProfilesM3()
const toast = useToast()

// ── Form ─────────────────────────────────────────────────────────────────────

/** The pipeline's tables (explicit list, else mapped tables); free text otherwise. */
const tableOptions = computed(() => {
  const listed = props.profile.source.tables ?? []
  const mapped = (props.profile.flow?.mappings ?? []).map((m) => `${m.source_schema}.${m.source_table}`)
  return [...new Set([...listed, ...mapped].map((t) => t.trim()).filter(Boolean))].sort((a, b) =>
    a.localeCompare(b, undefined, { sensitivity: "base", numeric: true }),
  )
})

type KeyMode = "auto" | "custom"
type CompareMode = "keys" | "selected" | "all"
const table = ref(tableOptions.value[0] ?? "")
const keyMode = ref<KeyMode>("auto")
const keyColumns = ref<string[]>([])
const compareMode = ref<CompareMode>("keys")
const compareColumns = ref<string[]>([])
const trimSpaces = ref(true)
const caseInsensitive = ref(false)
const recheck = ref(true)
const recheckLimit = ref(10000)
const starting = ref(false)

const columns = ref<SourceTableColumn[]>([])
const primaryKey = ref<string[]>([])
const columnsLoading = ref(false)
const columnsError = ref("")

const splitTable = (value: string) => {
  const i = value.lastIndexOf(".")
  return i > 0 ? { schema: value.slice(0, i), name: value.slice(i + 1) } : null
}

let columnsRequest = 0
const loadColumns = async () => {
  const parts = splitTable(table.value.trim())
  columns.value = []
  primaryKey.value = []
  columnsError.value = ""
  if (!parts) return
  const request = ++columnsRequest
  columnsLoading.value = true
  try {
    const result = await inspectSourceTable({
      source: props.profile.source,
      schema: parts.schema,
      name: parts.name,
      limit: 0,
      profile_id: props.profile.id,
    })
    if (request !== columnsRequest) return
    columns.value = [...result.columns].sort((a, b) => (a.position ?? 0) - (b.position ?? 0))
    primaryKey.value = result.primary_key ?? []
  } catch (e: unknown) {
    if (request === columnsRequest) columnsError.value = formatApiError(e, "Could not read the table's columns")
  } finally {
    if (request === columnsRequest) columnsLoading.value = false
  }
}
watch(table, () => {
  keyColumns.value = []
  compareColumns.value = []
  void loadColumns()
})

const autoKeyLabel = computed(() => {
  const mapping = (props.profile.flow?.mappings ?? []).find(
    (m) => `${m.source_schema}.${m.source_table}`.toLowerCase() === table.value.trim().toLowerCase(),
  )
  if (mapping?.pk_columns?.length) return `Mapping key: ${mapping.pk_columns.join(", ")}`
  if (primaryKey.value.length) return `Primary key: ${primaryKey.value.join(", ")}`
  return columnsLoading.value ? "Reading the primary key…" : "Target primary key (source has none)"
})

const toggle = (list: string[], name: string) =>
  list.includes(name) ? list.filter((c) => c !== name) : [...list, name]

const canStart = computed(
  () =>
    Boolean(splitTable(table.value.trim())) &&
    (keyMode.value === "auto" || keyColumns.value.length > 0) &&
    (compareMode.value !== "selected" || compareColumns.value.length > 0) &&
    !starting.value,
)

const start = async (override?: RecordDiffJob) => {
  starting.value = true
  try {
    const body = override
      ? {
          table: override.table,
          key_columns: override.requested_key_columns,
          compare_columns: override.requested_compare_columns,
          compare_all_columns: override.compare_all_columns,
          options: override.options,
        }
      : {
          table: table.value.trim(),
          key_columns: keyMode.value === "custom" ? keyColumns.value : [],
          compare_columns: compareMode.value === "selected" ? compareColumns.value : [],
          compare_all_columns: compareMode.value === "all",
          options: {
            trim_trailing_spaces: trimSpaces.value,
            case_insensitive: caseInsensitive.value,
            recheck: recheck.value,
            recheck_limit: Math.max(0, Math.floor(Number(recheckLimit.value) || 0)),
          },
        }
    const job = await startRecordDiff(props.profile.id, body)
    toast.success(`Verify Record of ${job.table} queued.`)
    selectedJobId.value = job.id
    await loadJobs()
  } catch (e: unknown) {
    toast.error(formatApiError(e, "Could not start Verify Record"))
  } finally {
    starting.value = false
  }
}

// ── Jobs ─────────────────────────────────────────────────────────────────────

const jobs = ref<RecordDiffJob[]>([])
const jobsLoading = ref(false)
const jobsError = ref("")
const selectedJobId = ref("")
const selectedJob = computed(() => jobs.value.find((j) => j.id === selectedJobId.value) ?? null)
const isActive = (job: RecordDiffJob) => job.state === "queued" || job.state === "running"

const loadJobs = async () => {
  jobsLoading.value = true
  try {
    jobs.value = await listRecordDiffs(props.profile.id)
    jobsError.value = ""
    if (!selectedJob.value && jobs.value.length) selectedJobId.value = jobs.value[0].id
  } catch (e: unknown) {
    jobsError.value = formatApiError(e, "Could not load Verify Record runs")
  } finally {
    jobsLoading.value = false
  }
}

// Poll only while something is queued or running.
let pollTimer: ReturnType<typeof setInterval> | null = null
const anyActive = computed(() => jobs.value.some(isActive))
watch(
  anyActive,
  (active) => {
    if (pollTimer) clearInterval(pollTimer)
    pollTimer = active ? setInterval(() => void loadJobs(), 2000) : null
  },
  { immediate: true },
)

onMounted(() => {
  void loadJobs()
  void loadColumns()
})
onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer)
})

const cancelJob = async (job: RecordDiffJob) => {
  try {
    await cancelRecordDiff(props.profile.id, job.id)
    toast.info("Cancelling…")
    await loadJobs()
  } catch (e: unknown) {
    toast.error(formatApiError(e, "Could not cancel the Verify Record run"))
  }
}

const pendingDelete = ref<RecordDiffJob | null>(null)
const removeJob = async () => {
  const job = pendingDelete.value
  if (!job) return
  try {
    await deleteRecordDiff(props.profile.id, job.id)
    if (selectedJobId.value === job.id) selectedJobId.value = ""
    pendingDelete.value = null
    await loadJobs()
  } catch (e: unknown) {
    toast.error(formatApiError(e, "Could not delete the Verify Record run"))
  }
}

const stateBadge = (job: RecordDiffJob) => {
  switch (job.state) {
    case "succeeded":
      return { status: "success", label: "Succeeded" }
    case "running":
      return { status: "pending", label: phaseLabel[job.phase] }
    case "queued":
      return { status: "pending", label: "Queued" }
    case "cancelled":
      return { status: "stopped", label: "Cancelled" }
    default:
      return { status: "failed", label: "Failed" }
  }
}

const phaseLabel: Record<RecordDiffJob["phase"], string> = {
  queued: "Queued",
  planning: "Planning",
  scanning: "Scanning",
  comparing: "Comparing",
  rechecking: "Rechecking",
  done: "Done",
}

const fmt = (n?: number | null) => (n ?? 0).toLocaleString()

/** 0–100 for a progress bar, or null when indeterminate. */
const progressPct = (job: RecordDiffJob): number | null => {
  const p = job.progress
  if (job.phase === "scanning") {
    const est = (p.source_rows_estimate ?? 0) + (p.target_rows_estimate ?? 0)
    if (!est) return null
    return Math.min(99, Math.round(((p.source_rows_read + p.target_rows_read) / est) * 100))
  }
  if (job.phase === "comparing" && p.buckets_total) return Math.round((p.buckets_done / p.buckets_total) * 100)
  if (job.phase === "rechecking" && p.recheck_total) return Math.round((p.recheck_done / p.recheck_total) * 100)
  return null
}

const progressText = (job: RecordDiffJob) => {
  const p = job.progress
  switch (job.phase) {
    case "queued":
      return "Waiting for a free slot"
    case "planning":
      return "Reading table structure and choosing the key"
    case "scanning": {
      const src = p.source_rows_estimate ? `${fmt(p.source_rows_read)} / ~${fmt(p.source_rows_estimate)}` : fmt(p.source_rows_read)
      const tgt = p.target_rows_estimate ? `${fmt(p.target_rows_read)} / ~${fmt(p.target_rows_estimate)}` : fmt(p.target_rows_read)
      return `Read source ${src} · target ${tgt} rows`
    }
    case "comparing":
      return `Compared ${p.buckets_done} of ${p.buckets_total} key partitions`
    case "rechecking":
      return `Rechecked ${fmt(p.recheck_done)} of ${fmt(p.recheck_total)} differences`
    default:
      return ""
  }
}

const when = (iso?: string) => (iso ? new Date(iso).toLocaleString() : "")
const duration = (job: RecordDiffJob) => {
  if (!job.started_at) return ""
  const ms = new Date(job.finished_at ?? Date.now()).getTime() - new Date(job.started_at).getTime()
  const s = Math.max(0, Math.round(ms / 1000))
  return s < 60 ? `${s}s` : s < 3600 ? `${Math.floor(s / 60)}m ${s % 60}s` : `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`
}

// ── Results ──────────────────────────────────────────────────────────────────

const KINDS: Array<{ key: RecordDiffKind; label: string; hint: string }> = [
  { key: "source_only", label: "Only in source", hint: "Rows the target is missing." },
  { key: "target_only", label: "Only in target", hint: "Rows the source no longer has (or never had)." },
  { key: "mismatched", label: "Different values", hint: "Same key on both sides, compared columns differ." },
]
const kind = ref<RecordDiffKind>("source_only")
const page = ref(1)
const pageSize = ref(25)
const rows = ref<RecordDiffRow[]>([])
const rowsTotal = ref(0)
const rowsLoading = ref(false)
const rowsError = ref("")

const kindCount = (job: RecordDiffJob, k: RecordDiffKind) => job.counts[k]

const loadRows = async () => {
  const job = selectedJob.value
  rows.value = []
  rowsError.value = ""
  if (!job || job.state !== "succeeded") return
  rowsLoading.value = true
  try {
    const res = await recordDiffRows(props.profile.id, job.id, kind.value, (page.value - 1) * pageSize.value, pageSize.value)
    rows.value = res.rows
    rowsTotal.value = res.total
  } catch (e: unknown) {
    rowsError.value = formatApiError(e, "Could not load result rows")
  } finally {
    rowsLoading.value = false
  }
}
watch([() => selectedJob.value?.id, () => selectedJob.value?.state], () => {
  page.value = 1
  const job = selectedJob.value
  // Open the first non-empty result tab.
  if (job?.state === "succeeded") kind.value = KINDS.find((k) => kindCount(job, k.key) > 0)?.key ?? "source_only"
  void loadRows()
})
watch(kind, () => {
  page.value = 1
  void loadRows()
})
watch([page, pageSize], () => void loadRows())

const keyColumnsFor = (job: RecordDiffJob, k: RecordDiffKind) =>
  (k === "target_only" ? job.plan?.target_key_columns : job.plan?.source_key_columns) ?? []
const valueColumnsFor = (job: RecordDiffJob, k: RecordDiffKind) =>
  k === "source_only" ? job.plan?.source_compare_columns ?? [] : k === "target_only" ? job.plan?.target_compare_columns ?? [] : []

const keySourceLabel: Record<string, string> = {
  request: "chosen columns",
  mapping: "mapping key",
  source_pk: "source primary key",
  target_pk: "target primary key",
}

const show = (v: string | null | undefined) => (v === null || v === undefined ? "NULL" : v)
</script>

<template>
  <div class="space-y-5">
    <!-- New diff -->
    <section class="rounded-xl border border-border p-4">
      <div class="mb-3 flex items-center gap-2">
        <GitCompareArrows class="h-4 w-4 text-muted-foreground" :stroke-width="1.75" />
        <h3 class="text-sm font-semibold text-foreground">Verify records</h3>
        <AppInfoHint
          text="Reads both tables once (key columns, plus any compared columns), compares them by key on the orchestrator, then re-reads each difference by key so rows still in flight through the pipeline are not reported. Read-only on both databases."
        />
      </div>

      <div class="grid gap-4 lg:grid-cols-2">
        <label class="block">
          <AppFormLabel label="Table" hint="Source table of this pipeline (SCHEMA.TABLE)." />
          <AppCombobox
            v-model="table"
            :options="tableOptions"
            placeholder="SCHEMA.TABLE"
            aria-label="Table to compare"
            empty-text="Type SCHEMA.TABLE"
          />
        </label>

        <div>
          <AppFormLabel label="Compare by" hint="Rows are matched on these key column(s)." />
          <div class="flex flex-wrap gap-4 text-sm">
            <label class="flex items-center gap-2">
              <input v-model="keyMode" type="radio" value="auto" class="accent-brand-500" />
              Automatic
            </label>
            <label class="flex items-center gap-2">
              <input v-model="keyMode" type="radio" value="custom" class="accent-brand-500" />
              Choose key columns
            </label>
          </div>
          <p v-if="keyMode === 'auto'" class="mt-1.5 text-xs text-muted-foreground">{{ autoKeyLabel }}</p>
        </div>
      </div>

      <p v-if="columnsError" class="mt-3 text-xs text-red-500">{{ columnsError }}</p>

      <div v-if="keyMode === 'custom'" class="mt-4">
        <AppFormLabel label="Key columns" :hint="`${keyColumns.length} selected. Order follows your clicks.`" />
        <p v-if="columnsLoading" class="text-xs text-muted-foreground">Loading columns…</p>
        <div v-else class="flex flex-wrap gap-1.5">
          <button
            v-for="col in columns"
            :key="col.name"
            type="button"
            class="tag-chip"
            :class="keyColumns.includes(col.name) ? 'bg-brand-500/15 text-primary ring-1 ring-brand-500/40' : ''"
            :aria-pressed="keyColumns.includes(col.name)"
            @click="keyColumns = toggle(keyColumns, col.name)"
          >
            {{ col.name }}<span v-if="col.primary_key" class="ml-1 opacity-60">PK</span>
          </button>
        </div>
      </div>

      <div class="mt-4">
        <AppFormLabel label="Values" hint="Missing rows are always found. Optionally also compare column values of rows on both sides." />
        <div class="flex flex-wrap gap-4 text-sm">
          <label class="flex items-center gap-2">
            <input v-model="compareMode" type="radio" value="keys" class="accent-brand-500" />
            Keys only (missing rows)
          </label>
          <label class="flex items-center gap-2">
            <input v-model="compareMode" type="radio" value="selected" class="accent-brand-500" />
            Selected columns
          </label>
          <label class="flex items-center gap-2">
            <input v-model="compareMode" type="radio" value="all" class="accent-brand-500" />
            All mapped columns
          </label>
        </div>
        <div v-if="compareMode === 'selected'" class="mt-2 flex flex-wrap gap-1.5">
          <p v-if="columnsLoading" class="text-xs text-muted-foreground">Loading columns…</p>
          <button
            v-for="col in columns"
            v-else
            :key="col.name"
            type="button"
            class="tag-chip"
            :class="compareColumns.includes(col.name) ? 'bg-brand-500/15 text-primary ring-1 ring-brand-500/40' : ''"
            :aria-pressed="compareColumns.includes(col.name)"
            @click="compareColumns = toggle(compareColumns, col.name)"
          >
            {{ col.name }}
          </button>
        </div>
      </div>

      <div class="mt-4 flex flex-wrap items-center gap-x-5 gap-y-2 text-sm">
        <label class="flex items-center gap-2">
          <input v-model="trimSpaces" type="checkbox" class="h-4 w-4 accent-brand-500" />
          Ignore trailing spaces
          <AppInfoHint text="CHAR columns pad with spaces (Oracle, SQL Server); a varchar copy does not." />
        </label>
        <label class="flex items-center gap-2">
          <input v-model="caseInsensitive" type="checkbox" class="h-4 w-4 accent-brand-500" />
          Ignore letter case
        </label>
        <label class="flex items-center gap-2">
          <input v-model="recheck" type="checkbox" class="h-4 w-4 accent-brand-500" />
          Recheck differences by key
          <AppInfoHint text="Re-reads each difference on both sides after the scan. Rows that caught up while the scan ran are dropped and counted as resolved in flight. Differences over the limit are reported as unverified." />
        </label>
        <label v-if="recheck" class="flex items-center gap-2">
          <span class="text-muted-foreground">up to</span>
          <AppInput v-model.number="recheckLimit" type="number" min="0" class="w-28" aria-label="Recheck limit per kind" />
          <span class="text-muted-foreground">per kind</span>
        </label>
      </div>

      <div class="mt-4 flex justify-end">
        <AppButton variant="primary" :disabled="!canStart" @click="start()">
          <Play class="h-4 w-4" :stroke-width="1.75" />
          {{ starting ? "Starting…" : "Start verify" }}
        </AppButton>
      </div>
    </section>

    <!-- Jobs -->
    <section>
      <div class="mb-2 flex items-center justify-between">
        <h3 class="text-sm font-semibold text-foreground">Verify runs</h3>
        <button type="button" class="icon-button" title="Refresh" :disabled="jobsLoading" @click="loadJobs">
          <RefreshCw class="h-4 w-4" :class="jobsLoading ? 'animate-spin' : ''" :stroke-width="1.75" />
        </button>
      </div>
      <p v-if="jobsError" class="mb-2 text-xs text-red-500">{{ jobsError }}</p>
      <AppTable>
        <thead>
          <tr>
            <th>Table</th>
            <th>Started</th>
            <th>State</th>
            <th class="text-right">Only in source</th>
            <th class="text-right">Only in target</th>
            <th class="text-right">Different</th>
            <th class="w-24" />
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="job in jobs"
            :key="job.id"
            class="cursor-pointer"
            :class="{ 'bg-brand-500/5': selectedJobId === job.id }"
            @click="selectedJobId = job.id"
          >
            <td>
              <p class="font-mono text-xs">{{ job.table }}</p>
              <p v-if="job.plan" class="mt-0.5 text-xs text-muted-foreground">
                by {{ job.plan.source_key_columns.join(", ") }}
              </p>
            </td>
            <td class="whitespace-nowrap text-xs text-muted-foreground">
              <p>{{ when(job.created_at) }}</p>
              <p v-if="duration(job)">{{ duration(job) }}</p>
            </td>
            <td class="min-w-[12rem]">
              <AppStatusBadge v-bind="stateBadge(job)" />
              <template v-if="isActive(job)">
                <div v-if="progressPct(job) !== null" class="mt-1.5 h-1 w-full overflow-hidden rounded bg-muted">
                  <div class="h-full bg-brand-500 transition-all" :style="{ width: `${progressPct(job)}%` }" />
                </div>
                <p class="mt-1 text-xs text-muted-foreground">{{ progressText(job) }}</p>
              </template>
              <p v-else-if="job.error" class="mt-1 line-clamp-2 text-xs text-red-500" :title="job.error">{{ job.error }}</p>
            </td>
            <template v-if="job.state === 'succeeded'">
              <td class="text-right tabular-nums" :class="job.counts.source_only ? 'font-semibold text-amber-600 dark:text-amber-400' : ''">{{ fmt(job.counts.source_only) }}</td>
              <td class="text-right tabular-nums" :class="job.counts.target_only ? 'font-semibold text-amber-600 dark:text-amber-400' : ''">{{ fmt(job.counts.target_only) }}</td>
              <td class="text-right tabular-nums">
                {{ job.plan?.source_compare_columns.length ? fmt(job.counts.mismatched) : "—" }}
              </td>
            </template>
            <template v-else>
              <td class="text-right text-muted-foreground">—</td>
              <td class="text-right text-muted-foreground">—</td>
              <td class="text-right text-muted-foreground">—</td>
            </template>
            <td>
              <div class="flex items-center justify-end gap-1" @click.stop>
                <button v-if="isActive(job)" type="button" class="icon-button" title="Cancel" @click="cancelJob(job)">
                  <Ban class="h-4 w-4" :stroke-width="1.75" />
                </button>
                <template v-else>
                  <button type="button" class="icon-button" title="Run again with the same settings" :disabled="starting" @click="start(job)">
                    <RotateCcw class="h-4 w-4" :stroke-width="1.75" />
                  </button>
                  <button type="button" class="icon-button text-red-500" title="Delete" @click="pendingDelete = job">
                    <Trash2 class="h-4 w-4" :stroke-width="1.75" />
                  </button>
                </template>
              </div>
            </td>
          </tr>
          <tr v-if="!jobs.length">
            <td colspan="7">
              <AppStateBlock
                :kind="jobsLoading ? 'loading' : 'empty'"
                :message="jobsLoading ? 'Loading…' : 'No verify runs yet. Choose a table above and start one.'"
              />
            </td>
          </tr>
        </tbody>
      </AppTable>
    </section>

    <!-- Result of the selected run -->
    <section v-if="selectedJob && selectedJob.state === 'succeeded' && selectedJob.plan" class="space-y-4">
      <div class="flex flex-wrap items-center gap-2 text-sm">
        <span class="font-mono text-xs">{{ selectedJob.plan.source_schema }}.{{ selectedJob.plan.source_table }}</span>
        <ArrowLeftRight class="h-3.5 w-3.5 text-muted-foreground" :stroke-width="1.75" />
        <span class="font-mono text-xs">{{ selectedJob.plan.target_schema }}.{{ selectedJob.plan.target_table }}</span>
        <span class="text-xs text-muted-foreground">
          · key {{ selectedJob.plan.source_key_columns.join(", ") }} ({{ keySourceLabel[selectedJob.plan.key_source] }})
          <template v-if="selectedJob.plan.source_compare_columns.length">
            · {{ selectedJob.plan.source_compare_columns.length }} compared column(s)
          </template>
        </span>
      </div>

      <div class="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
        <AppMetricCard label="Source rows" :value="fmt(selectedJob.counts.source_rows)" />
        <AppMetricCard label="Target rows" :value="fmt(selectedJob.counts.target_rows)" />
        <AppMetricCard label="Matched" :value="fmt(selectedJob.counts.matched)" tone="success" />
        <AppMetricCard
          label="Only in source"
          :value="fmt(selectedJob.counts.source_only)"
          :tone="selectedJob.counts.source_only ? 'warning' : 'neutral'"
        />
        <AppMetricCard
          label="Only in target"
          :value="fmt(selectedJob.counts.target_only)"
          :tone="selectedJob.counts.target_only ? 'warning' : 'neutral'"
        />
        <AppMetricCard
          label="Different values"
          :value="selectedJob.plan.source_compare_columns.length ? fmt(selectedJob.counts.mismatched) : '—'"
          :helper="selectedJob.plan.source_compare_columns.length ? '' : 'Keys only'"
          :tone="selectedJob.counts.mismatched ? 'warning' : 'neutral'"
        />
      </div>

      <div class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
        <span v-if="selectedJob.counts.resolved_in_flight">
          {{ fmt(selectedJob.counts.resolved_in_flight) }} resolved in flight (caught up during the scan)
        </span>
        <span v-if="selectedJob.counts.unverified" class="text-amber-600 dark:text-amber-400">
          {{ fmt(selectedJob.counts.unverified) }} not rechecked
        </span>
        <span v-if="selectedJob.counts.source_duplicate_keys || selectedJob.counts.target_duplicate_keys" class="text-amber-600 dark:text-amber-400">
          Duplicate keys — source {{ fmt(selectedJob.counts.source_duplicate_keys) }}, target {{ fmt(selectedJob.counts.target_duplicate_keys) }} (key is not unique; extra rows ignored)
        </span>
        <span v-if="selectedJob.counts.source_null_keys || selectedJob.counts.target_null_keys">
          NULL keys skipped — source {{ fmt(selectedJob.counts.source_null_keys) }}, target {{ fmt(selectedJob.counts.target_null_keys) }}
        </span>
      </div>
      <div v-for="w in selectedJob.plan.warnings ?? []" :key="w" class="alert-warning text-xs">
        <Info class="h-4 w-4 shrink-0" :stroke-width="1.75" />
        {{ w }}
      </div>

      <div class="rounded-xl border border-border">
        <div class="flex flex-wrap items-center justify-between gap-2 border-b border-border px-2">
          <div class="flex flex-wrap gap-x-1" role="tablist">
            <button
              v-for="k in KINDS"
              :key="k.key"
              type="button"
              role="tab"
              :aria-selected="kind === k.key"
              :title="k.hint"
              class="-mb-px inline-flex h-10 items-center gap-2 border-b-2 px-3 text-sm font-medium transition-colors"
              :class="kind === k.key ? 'border-primary text-primary' : 'border-transparent text-muted-foreground hover:text-foreground'"
              @click="kind = k.key"
            >
              {{ k.label }}
              <span class="badge-neutral">{{ fmt(kindCount(selectedJob, k.key)) }}</span>
            </button>
          </div>
          <div class="flex items-center gap-1 py-1.5">
            <a
              class="icon-button"
              :href="recordDiffDownloadUrl(profile.id, selectedJob.id, kind, 'csv')"
              title="Download CSV"
              download
            >
              <Download class="h-4 w-4" :stroke-width="1.75" />
            </a>
            <a
              class="text-xs text-muted-foreground hover:text-foreground hover:underline"
              :href="recordDiffDownloadUrl(profile.id, selectedJob.id, kind, 'ndjson')"
              download
            >NDJSON</a>
          </div>
        </div>

        <div class="p-3">
          <p v-if="rowsError" class="mb-2 text-xs text-red-500">{{ rowsError }}</p>
          <AppTable v-if="rows.length || rowsLoading">
            <thead>
              <tr>
                <th v-for="c in keyColumnsFor(selectedJob, kind)" :key="`k-${c}`">{{ c }}</th>
                <th v-for="c in valueColumnsFor(selectedJob, kind)" :key="`v-${c}`" class="text-muted-foreground">{{ c }}</th>
                <th v-if="kind === 'mismatched'">Differences (source → target)</th>
                <th class="w-28">Check</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="(row, i) in rows" :key="i">
                <td v-for="c in keyColumnsFor(selectedJob, kind)" :key="`k-${c}`" class="font-mono text-xs">{{ show(row.key[c]) }}</td>
                <td v-for="c in valueColumnsFor(selectedJob, kind)" :key="`v-${c}`" class="max-w-[16rem] truncate font-mono text-xs text-muted-foreground" :title="show(row.values?.[c])">
                  {{ show(row.values?.[c]) }}
                </td>
                <td v-if="kind === 'mismatched'" class="text-xs">
                  <p v-for="d in row.differences ?? []" :key="d.column" class="font-mono">
                    <span class="text-muted-foreground">{{ d.column }}:</span>
                    <span class="text-red-600 dark:text-red-400">{{ show(d.source) }}</span>
                    →
                    <span class="text-emerald-700 dark:text-emerald-400">{{ show(d.target) }}</span>
                  </p>
                </td>
                <td>
                  <span :class="row.status === 'confirmed' ? 'badge-info' : 'badge-neutral'">
                    {{ row.status === "confirmed" ? "Rechecked" : "Not rechecked" }}
                  </span>
                </td>
              </tr>
            </tbody>
          </AppTable>
          <AppStateBlock
            v-else
            kind="empty"
            :message="
              kind === 'mismatched' && !selectedJob.plan.source_compare_columns.length
                ? 'Values were not compared in this run. Choose columns under Values and run again.'
                : 'No differences of this kind.'
            "
          />
          <AppListPager v-model:page="page" v-model:page-size="pageSize" :total="rowsTotal" :sizes="[25, 50, 100, 250]" noun="rows" />
        </div>
      </div>
    </section>

    <AppConfirmDialog
      :open="Boolean(pendingDelete)"
      title="Delete this Verify Record run?"
      confirm-label="Delete"
      danger
      @confirm="removeJob"
      @update:open="(open) => { if (!open) pendingDelete = null }"
    >
      Removes the run and its result files from the orchestrator.
    </AppConfirmDialog>
  </div>
</template>
