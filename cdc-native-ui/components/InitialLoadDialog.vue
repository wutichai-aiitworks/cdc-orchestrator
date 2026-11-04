<script setup lang="ts">
/**
 * Initial load (streaming): copy every source row of the selected tables into the target
 * in the background, one forward-only SELECT per table. Used for rows a rolling window
 * never re-reads (e.g. legacy rows with a NULL UPDATE_DATE).
 */
import { AlertTriangle, Database, Loader2 } from "@lucide/vue"
import type { CdcProfile, InitialLoadStatus } from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"

const props = defineProps<{
  open: boolean
  profile: CdcProfile | null
}>()

const emit = defineEmits<{
  "update:open": [value: boolean]
  /** A load reached a final state (succeeded / failed / cancelled). */
  finished: []
}>()

const { startInitialLoad, getInitialLoad, cancelInitialLoad } = useProfilesM3()

const POLL_MS = 3000

const selectedTables = ref<string[]>([])
const clearTarget = ref(false)
const status = ref<InitialLoadStatus | null>(null)
const error = ref("")
const starting = ref(false)
const cancelling = ref(false)
let timer: ReturnType<typeof setInterval> | null = null

const tables = computed(() => props.profile?.source?.tables ?? [])
const running = computed(() => status.value?.state === "running")
const finished = computed(() => Boolean(status.value) && !running.value)
const hasRollingWindow = computed(() =>
  (props.profile?.source?.table_poll ?? []).some((r) => r.strategy === "rolling_window"),
)

const fmt = (n: number | null | undefined) => (n == null ? "—" : n.toLocaleString())

const stopPolling = () => {
  if (timer) clearInterval(timer)
  timer = null
}

const poll = async () => {
  if (!props.profile) return
  try {
    status.value = await getInitialLoad(props.profile.id)
    if (status.value.state !== "running") {
      stopPolling()
      emit("finished")
    }
  } catch (e: any) {
    // 404 = no load has run on this orchestrator instance yet.
    if (e?.statusCode === 404 || e?.status === 404) status.value = null
    else error.value = formatApiError(e, "Could not read initial load progress")
    stopPolling()
  }
}

const startPolling = () => {
  stopPolling()
  timer = setInterval(() => void poll(), POLL_MS)
}

// On open: reset the form and attach to a load that may already be running.
watch(
  () => props.open,
  async (open) => {
    if (!open) {
      stopPolling()
      return
    }
    selectedTables.value = [...tables.value]
    clearTarget.value = false
    error.value = ""
    status.value = null
    await poll()
    if (running.value) startPolling()
  },
  { immediate: true },
)

onUnmounted(stopPolling)

const start = async () => {
  if (!props.profile || !selectedTables.value.length) return
  starting.value = true
  error.value = ""
  try {
    status.value = await startInitialLoad(props.profile.id, {
      clear_target: clearTarget.value,
      tables: selectedTables.value.length === tables.value.length ? [] : selectedTables.value,
    })
    startPolling()
  } catch (e: any) {
    error.value = formatApiError(e, "Initial load could not start")
  } finally {
    starting.value = false
  }
}

const cancel = async () => {
  if (!props.profile) return
  cancelling.value = true
  try {
    status.value = await cancelInitialLoad(props.profile.id)
  } catch (e: any) {
    error.value = formatApiError(e, "Cancel failed")
  } finally {
    cancelling.value = false
  }
}

const newLoad = () => {
  status.value = null
  error.value = ""
}

const close = () => {
  if (!starting.value) emit("update:open", false)
}

const tableTone = (state: string) =>
  state === "done"
    ? "text-emerald-600 dark:text-emerald-400"
    : state === "failed"
      ? "text-red-500"
      : state === "running"
        ? "text-primary"
        : "text-muted-foreground"
</script>

<template>
  <div v-if="open" class="modal-overlay" @click.self="close">
    <div class="w-full max-w-lg rounded-xl border border-border bg-card p-5 shadow-2xl">
      <h3 class="flex items-center gap-2 text-base font-semibold text-foreground">
        <Database class="h-4 w-4" :stroke-width="1.75" />
        Initial load (streaming)
      </h3>

      <!-- Choose what to load -->
      <div v-if="!status" class="mt-3 space-y-3 text-sm text-muted-foreground">
        <p>
          Copies <span class="font-semibold text-foreground">every row</span> of the selected source
          tables into the target, in the background. Each table is read with one
          <code class="text-xs">SELECT *</code> — no paging, no re-reads. Rows already in the target
          are kept, so it is safe beside the running pipeline and safe to re-run.
        </p>
        <p v-if="hasRollingWindow" class="text-xs">
          Rolling-window tables only re-read recent rows; run this once so older rows (for example
          rows with an empty <code>UPDATE_DATE</code>) reach the target.
        </p>

        <fieldset class="space-y-1.5">
          <legend class="auth-label">Tables</legend>
          <label v-for="t in tables" :key="t" class="flex items-center gap-2">
            <input
              v-model="selectedTables"
              type="checkbox"
              :value="t"
              class="h-4 w-4 rounded border-border accent-brand-500"
            />
            <span class="font-mono text-xs text-foreground">{{ t }}</span>
          </label>
        </fieldset>

        <label class="flex items-start gap-2">
          <input v-model="clearTarget" type="checkbox" class="mt-0.5 h-4 w-4 rounded border-border accent-brand-500" />
          <span>
            Empty each target table first
            <span class="block text-xs">
              Truncates each table just before its copy starts. Leave unchecked to only fill in
              missing rows.
            </span>
          </span>
        </label>
        <div v-if="clearTarget" class="alert-warning flex items-start gap-2 rounded-lg px-3 py-2 text-xs">
          <AlertTriangle class="mt-0.5 h-4 w-4 shrink-0" :stroke-width="1.75" />
          <span>The target stays incomplete until the load finishes. Large tables can take a long time.</span>
        </div>
        <p class="text-xs">
          This reads the whole source table. On a production database, prefer off-peak hours.
        </p>
      </div>

      <!-- Progress / result -->
      <div v-else class="mt-3 space-y-3 text-sm">
        <p class="flex items-center gap-2 text-muted-foreground">
          <Loader2 v-if="running" class="h-4 w-4 animate-spin" :stroke-width="1.75" />
          <span v-if="running">Copying… {{ fmt(status.rows_copied) }} row(s) so far</span>
          <span v-else-if="status.state === 'succeeded'" class="text-emerald-600 dark:text-emerald-400">
            Finished — {{ fmt(status.rows_copied) }} row(s) copied.
          </span>
          <span v-else-if="status.state === 'cancelled'">Cancelled after {{ fmt(status.rows_copied) }} row(s).</span>
          <span v-else class="text-red-500">Failed after {{ fmt(status.rows_copied) }} row(s).</span>
        </p>
        <table class="w-full text-xs">
          <thead>
            <tr class="text-left text-muted-foreground">
              <th class="py-1 font-medium">Table</th>
              <th class="py-1 font-medium">State</th>
              <th class="py-1 text-right font-medium">Rows</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="t in status.tables" :key="t.source" class="border-t border-border align-top">
              <td class="py-1.5 pr-2">
                <span class="font-mono text-foreground">{{ t.source }}</span>
                <span class="block font-mono text-muted-foreground">→ {{ t.target }}</span>
                <span v-if="t.error" class="block break-words text-red-500">{{ t.error }}</span>
              </td>
              <td class="py-1.5 pr-2" :class="tableTone(t.state)">{{ t.state }}</td>
              <td class="py-1.5 text-right font-mono tabular-nums">{{ fmt(t.rows_copied) }}</td>
            </tr>
          </tbody>
        </table>
        <p v-if="finished && status.state !== 'succeeded'" class="text-xs text-muted-foreground">
          Rows already copied are kept. Start the load again to finish it.
        </p>
      </div>

      <p v-if="error" class="mt-3 break-words text-xs text-red-500">{{ error }}</p>

      <div class="mt-5 flex justify-end gap-2">
        <AppButton :disabled="starting" @click="close">{{ running ? "Close (keeps running)" : "Close" }}</AppButton>
        <AppButton v-if="running" variant="destructive" :disabled="cancelling" @click="cancel">
          {{ cancelling ? "Cancelling…" : "Cancel load" }}
        </AppButton>
        <AppButton v-else-if="finished" @click="newLoad">New load…</AppButton>
        <AppButton
          v-else
          variant="primary"
          :disabled="starting || !selectedTables.length"
          @click="start"
        >
          {{ starting ? "Starting…" : "Start initial load" }}
        </AppButton>
      </div>
    </div>
  </div>
</template>
