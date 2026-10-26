<script setup lang="ts">
import { Eye, KeyRound, RefreshCw, Table2, X } from "@lucide/vue"
import type { ProfilePayload, SourceTableInspect } from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"

const props = defineProps<{
  open: boolean
  source?: ProfilePayload["source"]
  target?: NonNullable<ProfilePayload["target"]>
  schema: string
  name: string
}>()

const emit = defineEmits<{
  "update:open": [value: boolean]
}>()

const { inspectSourceTable, inspectTargetTable } = useProfilesM3()
const loading = ref(false)
const error = ref("")
const tab = ref<"structure" | "data">("structure")
const inspect = ref<SourceTableInspect | null>(null)

const close = () => emit("update:open", false)

const lockBody = (lock: boolean) => {
  if (typeof document === "undefined") return
  document.documentElement.classList.toggle("overflow-hidden", lock)
  document.body.classList.toggle("overflow-hidden", lock)
}

watch(
  () => props.open,
  (open) => lockBody(open),
  { immediate: true }
)
onUnmounted(() => lockBody(false))

const onOverlayWheel = (event: WheelEvent) => {
  const target = event.target as HTMLElement | null
  if (!target?.closest("[data-modal-scroll]")) event.preventDefault()
}

const formatCell = (value: unknown) => {
  if (value == null) return "—"
  if (typeof value === "string") return value
  if (typeof value === "number" || typeof value === "boolean") return String(value)
  try {
    return JSON.stringify(value)
  } catch {
    return String(value)
  }
}

const pkLabel = computed(() => {
  const keys = inspect.value?.primary_key || []
  if (!keys.length) return "No primary key"
  return keys.length === 1 ? `Primary key: ${keys[0]}` : `Compound key: ${keys.join(", ")}`
})

const dataColumns = computed(() => {
  if (inspect.value?.columns?.length) return inspect.value.columns.map((column) => column.name)
  const first = inspect.value?.rows?.[0]
  return first ? Object.keys(first) : []
})

const load = async () => {
  if (!props.open || !props.schema || !props.name) return
  loading.value = true
  error.value = ""
  try {
    inspect.value = props.target
      ? await inspectTargetTable({
          target: props.target,
          schema: props.schema,
          name: props.name,
          limit: 50,
        })
      : await inspectSourceTable({
          source: props.source!,
          schema: props.schema,
          name: props.name,
          limit: 50,
        })
  } catch (e: any) {
    inspect.value = null
    error.value = formatApiError(e, "Failed to load table structure and data")
  } finally {
    loading.value = false
  }
}

watch(
  () => [props.open, props.schema, props.name, Boolean(props.target)] as const,
  ([open]) => {
    if (open) {
      tab.value = "structure"
      void load()
    }
  },
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
      <section class="flex h-[min(88vh,calc(100dvh-2rem))] w-full max-w-5xl flex-col overflow-hidden rounded-xl border border-border bg-card p-5 shadow-2xl animate-slide-up">
        <div class="mb-4 flex shrink-0 items-start justify-between gap-3">
          <div class="min-w-0">
            <h2 class="section-title font-mono">{{ schema }}.{{ name }}</h2>
            <p class="mt-1 text-sm text-muted-foreground">
              {{ pkLabel }}
              <span v-if="inspect?.row_count != null">
                · {{ Number(inspect.row_count).toLocaleString() }}
                {{ inspect.row_count === 1 ? "record" : "records" }}
              </span>
            </p>
          </div>
          <button type="button" class="icon-button" aria-label="Close" @click="close">
            <X class="h-4 w-4" :stroke-width="1.75" />
          </button>
        </div>

        <div class="mb-4 flex shrink-0 flex-wrap items-center justify-between gap-3">
          <div class="flex gap-2">
            <AppButton :variant="tab === 'structure' ? 'primary' : 'secondary'" @click="tab = 'structure'">
              <Table2 class="h-4 w-4" :stroke-width="1.75" />
              Structure
            </AppButton>
            <AppButton :variant="tab === 'data' ? 'primary' : 'secondary'" @click="tab = 'data'">
              <Eye class="h-4 w-4" :stroke-width="1.75" />
              Data
            </AppButton>
          </div>
          <AppButton :disabled="loading" @click="load">
            <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
            {{ loading ? "Loading..." : "Refresh" }}
          </AppButton>
        </div>

        <p v-if="error" class="mb-3 shrink-0 text-sm text-red-700 dark:text-red-400">{{ error }}</p>
        <p
          v-else-if="inspect && !inspect.primary_key.length"
          class="mb-3 shrink-0 rounded-xl border border-yellow-500/30 bg-yellow-500/10 px-3 py-2 text-sm text-yellow-800 dark:text-yellow-300"
        >
          This table has no primary key. Replica upsert/delete needs a key or compound key.
        </p>

        <div data-modal-scroll class="min-h-0 flex-1 overflow-auto overscroll-contain">
          <AppStateBlock v-if="loading && !inspect" kind="loading" message="Loading table…" />
          <template v-else-if="tab === 'structure'">
            <AppTable>
              <thead>
                <tr>
                  <th>Column</th>
                  <th>Type</th>
                  <th>Key</th>
                  <th>Nullable</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="column in inspect?.columns || []" :key="column.name">
                  <td class="font-mono">
                    <span class="inline-flex items-center gap-1.5">
                      <KeyRound
                        v-if="column.primary_key"
                        class="h-3.5 w-3.5 text-brand-500"
                        :stroke-width="1.75"
                      />
                      {{ column.name }}
                    </span>
                  </td>
                  <td class="font-mono text-muted-foreground">{{ column.data_type }}</td>
                  <td>
                    <span v-if="column.primary_key" class="badge-success">
                      PK{{ column.pk_position && inspect && inspect.primary_key.length > 1 ? ` ${column.pk_position}` : "" }}
                    </span>
                    <span v-else-if="column.unique" class="badge-neutral">Unique</span>
                    <span v-else class="text-muted-foreground">—</span>
                  </td>
                  <td class="text-muted-foreground">{{ column.nullable ? "yes" : "no" }}</td>
                </tr>
                <tr v-if="!(inspect?.columns || []).length">
                  <td colspan="4">
                    <AppStateBlock kind="empty" message="No columns to show." />
                  </td>
                </tr>
              </tbody>
            </AppTable>
          </template>
          <template v-else>
            <AppTable>
              <thead>
                <tr>
                  <th v-for="column in dataColumns" :key="column">
                    <span class="inline-flex items-center gap-1">
                      <KeyRound
                        v-if="inspect?.primary_key.includes(column)"
                        class="h-3.5 w-3.5 text-brand-500"
                        :stroke-width="1.75"
                      />
                      {{ column }}
                    </span>
                  </th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="(row, index) in inspect?.rows || []" :key="index">
                  <td v-for="column in dataColumns" :key="column" class="max-w-[16rem] truncate font-mono text-xs">
                    {{ formatCell(row[column]) }}
                  </td>
                </tr>
                <tr v-if="!(inspect?.rows || []).length">
                  <td :colspan="Math.max(dataColumns.length, 1)">
                    <AppStateBlock kind="empty" message="No rows in this table." />
                  </td>
                </tr>
              </tbody>
            </AppTable>
            <p v-if="inspect?.rows?.length" class="mt-2 text-xs text-muted-foreground">
              Showing {{ inspect.rows.length }} of {{ inspect.row_count != null ? Number(inspect.row_count).toLocaleString() : "—" }} records.
            </p>
          </template>
        </div>
      </section>
    </div>
  </Teleport>
</template>
