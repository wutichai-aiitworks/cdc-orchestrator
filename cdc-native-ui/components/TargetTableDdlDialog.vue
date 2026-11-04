<script setup lang="ts">
import { CheckCircle2, Copy, Download, KeyRound, Play, RefreshCw, TriangleAlert, X } from "@lucide/vue"
import type { ProfilePayload } from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"
import { generateTargetDdl, type TargetDdlResult } from "~/utils/targetDdl"

export type DdlTableRef = {
  source_schema: string
  source_table: string
  target_schema: string
  target_table: string
}

const props = defineProps<{
  open: boolean
  source: ProfilePayload["source"]
  target: NonNullable<ProfilePayload["target"]>
  tables: DdlTableRef[]
  profileId?: string
}>()

const emit = defineEmits<{
  "update:open": [value: boolean]
  created: []
}>()

type RunState = "idle" | "running" | "ok" | "failed"
type TableScript = {
  ref: DdlTableRef
  loading: boolean
  error: string
  result: TargetDdlResult | null
  pkColumns: string[]
  indexCount: number
  run: RunState
  runError: string
}

const { inspectSourceTable, createTargetTables } = useProfilesM3()
const items = ref<TableScript[]>([])
const loading = ref(false)
const running = ref(false)
const confirmOpen = ref(false)
const copied = ref(false)

const sourceType = computed(() => props.source.source_type)
const targetType = computed(() => props.target.target_type)

const close = () => {
  if (!running.value) emit("update:open", false)
}

const lockBody = (lock: boolean) => {
  if (typeof document === "undefined") return
  document.documentElement.classList.toggle("overflow-hidden", lock)
  document.body.classList.toggle("overflow-hidden", lock)
}
watch(() => props.open, (open) => lockBody(open), { immediate: true })
onUnmounted(() => lockBody(false))

const onOverlayWheel = (event: WheelEvent) => {
  const el = event.target as HTMLElement | null
  if (!el?.closest("[data-modal-scroll]")) event.preventDefault()
}

const tableLabel = (ref: DdlTableRef) =>
  `${ref.target_schema || ref.source_schema}.${ref.target_table || ref.source_table}`

const buildOne = async (item: TableScript) => {
  item.loading = true
  item.error = ""
  item.result = null
  item.run = "idle"
  item.runError = ""
  try {
    const inspect = await inspectSourceTable({
      source: props.source,
      schema: item.ref.source_schema,
      name: item.ref.source_table,
      limit: 0,
      profile_id: props.profileId,
    })
    const result = generateTargetDdl({
      sourceDialect: sourceType.value,
      targetDialect: targetType.value,
      targetSchema: item.ref.target_schema || item.ref.source_schema,
      targetTable: item.ref.target_table || item.ref.source_table,
      columns: inspect.columns,
      primaryKey: inspect.primary_key,
      indexes: inspect.indexes,
    })
    item.result = result
    item.pkColumns = inspect.primary_key
    item.indexCount = result.statements.filter((s) => /\bINDEX\b|\bKEY\b/.test(s)).length
  } catch (e) {
    item.error = formatApiError(e, "Failed to read the source table")
  } finally {
    item.loading = false
  }
}

const load = async () => {
  if (!props.open) return
  loading.value = true
  items.value = props.tables.map((ref) => ({
    ref,
    loading: true,
    error: "",
    result: null,
    pkColumns: [],
    indexCount: 0,
    run: "idle" as RunState,
    runError: "",
  }))
  // A few at a time: each inspect opens a source connection.
  const queue = [...items.value]
  const worker = async () => {
    for (let item = queue.shift(); item; item = queue.shift()) await buildOne(item)
  }
  await Promise.all([worker(), worker(), worker()])
  loading.value = false
}

watch(
  () => [props.open, props.tables] as const,
  ([open]) => { if (open) void load() },
  { immediate: true },
)

const ready = computed(() => items.value.filter((i) => i.result?.statements.length))
const failedLoads = computed(() => items.value.filter((i) => i.error))
const noteCount = computed(() => items.value.reduce((n, i) => n + (i.result?.notes.length ?? 0), 0))
const fullScript = computed(() =>
  items.value
    .map((i) => i.result?.script ?? (i.error ? `-- ${tableLabel(i.ref)}: ${i.error}\n` : ""))
    .filter(Boolean)
    .join("\n"),
)
const runSummary = computed(() => ({
  ok: items.value.filter((i) => i.run === "ok").length,
  failed: items.value.filter((i) => i.run === "failed").length,
}))

const copyScript = async () => {
  await navigator.clipboard.writeText(fullScript.value)
  copied.value = true
  setTimeout(() => (copied.value = false), 1500)
}

const downloadScript = () => {
  const name = props.tables.length === 1 ? tableLabel(props.tables[0]) : `${props.tables.length}-tables`
  const blob = new Blob([fullScript.value], { type: "application/sql" })
  const url = URL.createObjectURL(blob)
  const a = document.createElement("a")
  a.href = url
  a.download = `create-${name.replace(/[^\w.-]+/g, "_")}.${targetType.value}.sql`
  a.click()
  URL.revokeObjectURL(url)
}

const runOnTarget = async () => {
  confirmOpen.value = false
  running.value = true
  // One request per table: a failure stops that table only; every outcome is audited.
  for (const item of ready.value) {
    item.run = "running"
    item.runError = ""
    try {
      const res = await createTargetTables({
        target: props.target,
        statements: item.result!.statements,
        profile_id: props.profileId,
      })
      item.run = res.status ? "ok" : "failed"
      if (!res.status) {
        const failed = res.results.find((r) => !r.ok)
        item.runError = failed?.error ?? "Statement failed"
      }
    } catch (e) {
      item.run = "failed"
      item.runError = formatApiError(e, "Create failed")
    }
  }
  running.value = false
  if (runSummary.value.ok) emit("created")
}
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      class="modal-overlay overflow-hidden overscroll-none"
      role="dialog"
      aria-modal="true"
      @click.self="close"
      @wheel="onOverlayWheel"
    >
      <section class="flex h-[min(90vh,calc(100dvh-2rem))] w-full max-w-5xl flex-col overflow-hidden rounded-xl border border-border bg-card p-5 shadow-2xl animate-slide-up">
        <!-- Header -->
        <div class="mb-4 flex shrink-0 items-start justify-between gap-3">
          <div class="min-w-0">
            <h2 class="section-title">
              Create target {{ tables.length === 1 ? "table" : `tables (${tables.length})` }}
            </h2>
            <p class="mt-1 text-sm text-muted-foreground">
              {{ sourceType }} → {{ targetType }} · same primary key, unique constraints and indexes as the source,
              written in {{ targetType }} syntax. Safe to re-run: existing objects are skipped.
            </p>
          </div>
          <div class="flex shrink-0 items-center gap-2">
            <AppButton :disabled="loading || running" @click="load">
              <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
              {{ loading ? "Reading source..." : "Refresh" }}
            </AppButton>
            <button type="button" class="icon-button" aria-label="Close" :disabled="running" @click="close">
              <X class="h-4 w-4" :stroke-width="1.75" />
            </button>
          </div>
        </div>

        <!-- Per-table summary -->
        <div class="mb-3 flex max-h-40 shrink-0 flex-wrap gap-2 overflow-auto" data-modal-scroll>
          <span
            v-for="item in items"
            :key="tableLabel(item.ref)"
            class="inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1 font-mono text-xs"
            :class="item.error || item.run === 'failed'
              ? 'border-red-500/30 bg-red-500/10 text-red-700 dark:text-red-400'
              : item.run === 'ok'
                ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400'
                : 'border-border bg-muted/40 text-foreground'"
            :title="item.error || item.runError || (item.result?.notes.join('\n') ?? '')"
          >
            <RefreshCw v-if="item.loading || item.run === 'running'" class="h-3 w-3 animate-spin" :stroke-width="1.75" />
            <CheckCircle2 v-else-if="item.run === 'ok'" class="h-3 w-3" :stroke-width="1.75" />
            <TriangleAlert v-else-if="item.error || item.run === 'failed'" class="h-3 w-3" :stroke-width="1.75" />
            <KeyRound v-else-if="item.pkColumns.length" class="h-3 w-3 text-brand-500" :stroke-width="1.75" />
            {{ tableLabel(item.ref) }}
            <span v-if="item.result" class="text-muted-foreground">
              · PK {{ item.pkColumns.length ? item.pkColumns.length : "none" }} · {{ item.indexCount }} idx
            </span>
          </span>
        </div>

        <div v-if="failedLoads.length" class="mb-3 shrink-0 space-y-1">
          <p v-for="item in failedLoads" :key="`err-${tableLabel(item.ref)}`" class="text-sm text-red-700 dark:text-red-400">
            {{ item.ref.source_schema }}.{{ item.ref.source_table }}: {{ item.error }}
          </p>
        </div>
        <div v-if="runSummary.failed" class="mb-3 shrink-0 space-y-1">
          <p
            v-for="item in items.filter((i) => i.run === 'failed')"
            :key="`run-${tableLabel(item.ref)}`"
            class="text-sm text-red-700 dark:text-red-400"
          >
            {{ tableLabel(item.ref) }}: {{ item.runError }}
          </p>
        </div>
        <p v-if="noteCount && !loading" class="alert-warning mb-3 shrink-0">
          <TriangleAlert class="h-4 w-4 shrink-0" :stroke-width="1.75" />
          {{ noteCount }} {{ noteCount === 1 ? "item needs" : "items need" }} review — see the NOTE lines in the script.
        </p>
        <p v-if="runSummary.ok && !running" class="alert-success mb-3 shrink-0">
          <CheckCircle2 class="h-4 w-4 shrink-0" :stroke-width="1.75" />
          {{ runSummary.ok }} {{ runSummary.ok === 1 ? "table" : "tables" }} ready on the target.
        </p>

        <!-- Script -->
        <div data-modal-scroll class="min-h-0 flex-1 overflow-auto overscroll-contain">
          <AppStateBlock v-if="loading && !ready.length" kind="loading" message="Reading source structure…" />
          <pre v-else class="code-block whitespace-pre text-xs leading-relaxed">{{ fullScript }}</pre>
        </div>

        <!-- Actions -->
        <div class="mt-4 flex shrink-0 flex-wrap items-center justify-end gap-2">
          <AppButton :disabled="!fullScript || loading" @click="copyScript">
            <Copy class="h-4 w-4" :stroke-width="1.75" />
            {{ copied ? "Copied" : "Copy" }}
          </AppButton>
          <AppButton :disabled="!fullScript || loading" @click="downloadScript">
            <Download class="h-4 w-4" :stroke-width="1.75" />
            Download .sql
          </AppButton>
          <AppButton variant="primary" :disabled="!ready.length || loading || running" @click="confirmOpen = true">
            <Play class="h-4 w-4" :stroke-width="1.75" />
            {{ running ? "Creating..." : "Run on target" }}
          </AppButton>
        </div>
      </section>

      <AppConfirmDialog
        v-model:open="confirmOpen"
        title="Create tables on the target?"
        confirm-label="Run on target"
        :loading="running"
        @confirm="runOnTarget"
      >
        Runs {{ ready.reduce((n, i) => n + (i.result?.statements.length ?? 0), 0) }} statements on
        <span class="font-mono text-foreground">{{ target.connection.host }}/{{ target.connection.database }}</span>
        for {{ ready.length }} {{ ready.length === 1 ? "table" : "tables" }}. Existing tables and indexes are left as they are.
        The run is recorded in the audit trail.
      </AppConfirmDialog>
    </div>
  </Teleport>
</template>
