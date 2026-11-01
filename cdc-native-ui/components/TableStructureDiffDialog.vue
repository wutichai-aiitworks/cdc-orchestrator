<script setup lang="ts">
import { ArrowRight, CheckCircle2, KeyRound, RefreshCw, X } from "@lucide/vue"
import type { ProfilePayload, SourceTableColumn } from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"

const props = defineProps<{
  open: boolean
  source: ProfilePayload["source"]
  target: NonNullable<ProfilePayload["target"]>
  sourceSchema: string
  sourceTable: string
  targetSchema: string
  targetTable: string
  profileId?: string
}>()

const emit = defineEmits<{
  "update:open": [value: boolean]
}>()

type DiffStatus = "match" | "type_diff" | "source_only" | "target_only"
type DiffRow = {
  name: string
  sourceCol?: SourceTableColumn
  targetCol?: SourceTableColumn
  status: DiffStatus
}

const { inspectSourceTable, inspectTargetTable } = useProfilesM3()
const loading = ref(false)
const sourceError = ref("")
const targetError = ref("")
const sourceCols = ref<SourceTableColumn[]>([])
const targetCols = ref<SourceTableColumn[]>([])

const close = () => emit("update:open", false)

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

const diffRows = computed<DiffRow[]>(() => {
  const srcMap = new Map(sourceCols.value.map((c) => [c.name, c]))
  const tgtMap = new Map(targetCols.value.map((c) => [c.name, c]))
  const allNames = [...new Set([...srcMap.keys(), ...tgtMap.keys()])]
  const rows: DiffRow[] = allNames.map((name) => {
    const srcCol = srcMap.get(name)
    const tgtCol = tgtMap.get(name)
    let status: DiffStatus
    if (srcCol && tgtCol) {
      status = srcCol.data_type === tgtCol.data_type ? "match" : "type_diff"
    } else if (srcCol) {
      status = "source_only"
    } else {
      status = "target_only"
    }
    return { name, sourceCol: srcCol, targetCol: tgtCol, status }
  })
  rows.sort((a, b) => {
    const aPos = a.sourceCol?.position ?? a.targetCol?.position ?? 9999
    const bPos = b.sourceCol?.position ?? b.targetCol?.position ?? 9999
    return aPos - bPos
  })
  return rows
})

const summary = computed(() => ({
  match: diffRows.value.filter((r) => r.status === "match").length,
  type_diff: diffRows.value.filter((r) => r.status === "type_diff").length,
  source_only: diffRows.value.filter((r) => r.status === "source_only").length,
  target_only: diffRows.value.filter((r) => r.status === "target_only").length,
}))

const hasDifferences = computed(
  () => summary.value.type_diff > 0 || summary.value.source_only > 0 || summary.value.target_only > 0
)

const load = async () => {
  if (!props.open) return
  loading.value = true
  sourceError.value = ""
  targetError.value = ""
  sourceCols.value = []
  targetCols.value = []

  const [srcResult, tgtResult] = await Promise.allSettled([
    inspectSourceTable({
      source: props.source,
      schema: props.sourceSchema,
      name: props.sourceTable,
      limit: 0,
      profile_id: props.profileId,
    }),
    inspectTargetTable({
      target: props.target,
      schema: props.targetSchema || props.sourceSchema,
      name: props.targetTable || props.sourceTable,
      limit: 0,
      profile_id: props.profileId,
    }),
  ])

  if (srcResult.status === "fulfilled") {
    sourceCols.value = srcResult.value.columns
  } else {
    sourceError.value = formatApiError(srcResult.reason, "Failed to load source structure")
  }
  if (tgtResult.status === "fulfilled") {
    targetCols.value = tgtResult.value.columns
  } else {
    targetError.value = formatApiError(tgtResult.reason, "Failed to load target structure")
  }

  loading.value = false
}

watch(
  () => [props.open, props.sourceSchema, props.sourceTable, props.targetSchema, props.targetTable] as const,
  ([open]) => { if (open) void load() },
  { immediate: true }
)
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
            <h2 class="section-title flex flex-wrap items-center gap-2">
              <span class="font-mono">{{ sourceSchema }}.{{ sourceTable }}</span>
              <ArrowRight class="h-4 w-4 shrink-0 text-brand-500" :stroke-width="1.75" />
              <span class="font-mono">{{ targetSchema || sourceSchema }}.{{ targetTable || sourceTable }}</span>
            </h2>
            <p class="mt-1 text-sm text-muted-foreground">Column structure comparison · source vs target</p>
          </div>
          <div class="flex shrink-0 items-center gap-2">
            <AppButton :disabled="loading" @click="load">
              <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
              {{ loading ? "Loading..." : "Refresh" }}
            </AppButton>
            <button type="button" class="icon-button" aria-label="Close" @click="close">
              <X class="h-4 w-4" :stroke-width="1.75" />
            </button>
          </div>
        </div>

        <!-- Errors -->
        <div v-if="sourceError || targetError" class="mb-3 shrink-0 space-y-1.5">
          <p v-if="sourceError" class="text-sm text-red-700 dark:text-red-400">Source: {{ sourceError }}</p>
          <p v-if="targetError" class="text-sm text-red-700 dark:text-red-400">Target: {{ targetError }}</p>
        </div>

        <!-- Summary chips -->
        <div v-if="!loading && diffRows.length" class="mb-4 flex shrink-0 flex-wrap items-center gap-2">
          <span
            class="inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1 text-xs font-medium"
            :class="hasDifferences
              ? 'border-yellow-500/40 bg-yellow-500/10 text-yellow-700 dark:text-yellow-300'
              : 'border-emerald-500/40 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400'"
          >
            <CheckCircle2 class="h-3.5 w-3.5" :stroke-width="1.75" />
            {{ hasDifferences ? "Differences found" : "Structures match" }}
          </span>
          <span class="rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-2.5 py-1 text-xs font-medium text-emerald-700 dark:text-emerald-400">
            {{ summary.match }} matching
          </span>
          <span
            v-if="summary.type_diff"
            class="rounded-lg border border-yellow-500/30 bg-yellow-500/10 px-2.5 py-1 text-xs font-medium text-yellow-700 dark:text-yellow-300"
          >
            {{ summary.type_diff }} type {{ summary.type_diff === 1 ? "mismatch" : "mismatches" }}
          </span>
          <span
            v-if="summary.source_only"
            class="rounded-lg border border-red-500/30 bg-red-500/10 px-2.5 py-1 text-xs font-medium text-red-700 dark:text-red-400"
          >
            {{ summary.source_only }} missing in target
          </span>
          <span
            v-if="summary.target_only"
            class="rounded-lg border border-blue-500/30 bg-blue-500/10 px-2.5 py-1 text-xs font-medium text-blue-700 dark:text-blue-400"
          >
            {{ summary.target_only }} extra in target
          </span>
        </div>

        <!-- Table -->
        <div data-modal-scroll class="min-h-0 flex-1 overflow-auto overscroll-contain">
          <AppStateBlock v-if="loading && !diffRows.length" kind="loading" message="Comparing structures…" />
          <AppStateBlock v-else-if="!diffRows.length && !sourceError && !targetError" kind="empty" message="No columns to compare." />
          <AppTable v-else>
            <thead>
              <tr>
                <th class="w-[28%]">Source column</th>
                <th class="w-[22%]">Source type</th>
                <th class="w-[12%] text-center">Status</th>
                <th class="w-[28%]">Target column</th>
                <th class="w-[10%]">Target type</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="row in diffRows"
                :key="row.name"
                :class="{
                  'bg-yellow-500/5 dark:bg-yellow-500/5': row.status === 'type_diff',
                  'bg-red-500/5 dark:bg-red-500/5': row.status === 'source_only',
                  'bg-blue-500/5 dark:bg-blue-500/5': row.status === 'target_only',
                }"
              >
                <!-- Source column -->
                <td class="font-mono">
                  <template v-if="row.sourceCol">
                    <span class="inline-flex items-center gap-1.5">
                      <KeyRound
                        v-if="row.sourceCol.primary_key"
                        class="h-3.5 w-3.5 shrink-0 text-brand-500"
                        :stroke-width="1.75"
                      />
                      <span :class="row.status === 'source_only' ? 'text-red-600 dark:text-red-400' : ''">
                        {{ row.sourceCol.name }}
                      </span>
                    </span>
                    <span v-if="!row.sourceCol.nullable" class="ml-1 text-[10px] text-muted-foreground">NOT NULL</span>
                  </template>
                  <span v-else class="text-muted-foreground">—</span>
                </td>
                <!-- Source type -->
                <td class="font-mono text-sm text-muted-foreground">
                  <span
                    v-if="row.sourceCol"
                    :class="row.status === 'type_diff' ? 'text-yellow-600 dark:text-yellow-400 font-medium' : ''"
                  >
                    {{ row.sourceCol.data_type }}
                  </span>
                  <span v-else class="text-muted-foreground">—</span>
                </td>
                <!-- Status -->
                <td class="text-center">
                  <span v-if="row.status === 'match'" class="badge-success">Match</span>
                  <span v-else-if="row.status === 'type_diff'" class="badge-warning">Type diff</span>
                  <span v-else-if="row.status === 'source_only'" class="badge-error">Missing</span>
                  <span v-else class="rounded-lg border border-blue-500/30 bg-blue-500/10 px-2 py-0.5 text-[11px] font-medium text-blue-700 dark:text-blue-400">Extra</span>
                </td>
                <!-- Target column -->
                <td class="font-mono">
                  <template v-if="row.targetCol">
                    <span class="inline-flex items-center gap-1.5">
                      <KeyRound
                        v-if="row.targetCol.primary_key"
                        class="h-3.5 w-3.5 shrink-0 text-brand-500"
                        :stroke-width="1.75"
                      />
                      <span :class="row.status === 'target_only' ? 'text-blue-600 dark:text-blue-400' : ''">
                        {{ row.targetCol.name }}
                      </span>
                    </span>
                    <span v-if="!row.targetCol.nullable" class="ml-1 text-[10px] text-muted-foreground">NOT NULL</span>
                  </template>
                  <span v-else class="text-muted-foreground">—</span>
                </td>
                <!-- Target type -->
                <td class="font-mono text-sm text-muted-foreground">
                  <span
                    v-if="row.targetCol"
                    :class="row.status === 'type_diff' ? 'text-yellow-600 dark:text-yellow-400 font-medium' : ''"
                  >
                    {{ row.targetCol.data_type }}
                  </span>
                  <span v-else class="text-muted-foreground">—</span>
                </td>
              </tr>
            </tbody>
          </AppTable>
        </div>
      </section>
    </div>
  </Teleport>
</template>
