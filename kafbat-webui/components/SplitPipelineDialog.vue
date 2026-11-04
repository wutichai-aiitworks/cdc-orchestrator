<script setup lang="ts">
/**
 * Split tables into a new pipeline: pick tables, name the new pipeline, see what stays
 * and what moves, then create it as a Draft with the same source and target. Nothing is
 * started; the result step lists the next steps (start new one, restart this one).
 */
import { ArrowRight, CheckCircle2, Info, Search, Split } from "@lucide/vue"
import type { CdcProfile, SplitProfileResult } from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"

const props = defineProps<{
  open: boolean
  profile: CdcProfile | null
}>()

const emit = defineEmits<{
  "update:open": [value: boolean]
  /** Split saved: reload the list. */
  done: [result: SplitProfileResult]
  /** Open the new pipeline. */
  "open-created": [id: string]
  /** Restart the original so it stops capturing the moved tables. */
  "restart-original": []
}>()

const { splitProfile } = useProfilesM3()

const name = ref("")
const description = ref("")
const search = ref("")
const picked = ref<string[]>([])
const saving = ref(false)
const error = ref("")
const result = ref<SplitProfileResult | null>(null)

const tables = computed(() => (props.profile?.source?.tables ?? []).filter((t) => t.trim()))
const mode = computed(() => String(props.profile?.source?.capture_mode ?? "logical"))
const running = computed(() => ["running", "paused"].includes(String(props.profile?.status ?? "")))

const visible = computed(() => {
  const q = search.value.trim().toLowerCase()
  return q ? tables.value.filter((t) => t.toLowerCase().includes(q)) : tables.value
})
const pickedSet = computed(() => new Set(picked.value))
const staying = computed(() => tables.value.filter((t) => !pickedSet.value.has(t)))

const allVisiblePicked = computed(
  () => visible.value.length > 0 && visible.value.every((t) => pickedSet.value.has(t)),
)
const toggleVisible = () => {
  const set = new Set(picked.value)
  if (allVisiblePicked.value) visible.value.forEach((t) => set.delete(t))
  else visible.value.forEach((t) => set.add(t))
  picked.value = tables.value.filter((t) => set.has(t)) // keep pipeline order
}

const problem = computed(() => {
  if (!name.value.trim()) return "Name the new pipeline."
  if (!picked.value.length) return "Select the tables to move."
  if (!staying.value.length) return "Leave at least one table in this pipeline."
  return ""
})

/** What starting the new pipeline will do, by capture mode. */
const startNote = computed(() => {
  switch (mode.value) {
    case "logical":
      return "When you start it, it takes its own snapshot of these tables (rows already in the target are overwritten by key — no duplicates), then streams changes through its own replication slot. The source needs one more free slot (max_replication_slots)."
    case "sync_once":
      return "When you start it, it copies these tables once into the same target tables."
    case "incremental":
      return "When you start it, it polls these tables from the beginning (it does not inherit this pipeline's watermarks); rows are upserted by key."
    default:
      return "When you start it, it captures these tables into the same target tables."
  }
})

watch(
  () => props.open,
  (open) => {
    if (!open) return
    name.value = props.profile ? `${props.profile.name} · part 2` : ""
    description.value = ""
    search.value = ""
    picked.value = []
    error.value = ""
    result.value = null
  },
  { immediate: true },
)

const close = () => {
  if (!saving.value) emit("update:open", false)
}

const submit = async () => {
  if (!props.profile || problem.value) return
  saving.value = true
  error.value = ""
  try {
    result.value = await splitProfile(props.profile.id, {
      name: name.value.trim(),
      description: description.value.trim() || undefined,
      tables: picked.value,
    })
    emit("done", result.value)
  } catch (e: unknown) {
    error.value = formatApiError(e, "Split failed")
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <div v-if="open" class="modal-overlay" @click.self="close">
    <div class="flex max-h-[90vh] w-full max-w-3xl flex-col rounded-xl border border-border bg-card shadow-2xl">
      <div class="border-b border-border px-5 py-4">
        <h3 class="flex items-center gap-2 text-base font-semibold text-foreground">
          <Split class="h-4 w-4" :stroke-width="1.75" />
          Split tables into a new pipeline
        </h3>
        <p class="mt-1 text-xs text-muted-foreground">
          Same source and target as <span class="font-medium text-foreground">{{ profile?.name }}</span>.
          The new pipeline is saved as a draft — nothing starts until you start it.
        </p>
      </div>

      <!-- Step 1: choose -->
      <div v-if="!result" class="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-4 text-sm">
        <div class="grid grid-cols-1 gap-3 md:grid-cols-2">
          <label class="block">
            <span class="auth-label">New pipeline name</span>
            <input v-model="name" class="auth-input w-full" placeholder="e.g. CollabX · archive tables" />
          </label>
          <label class="block">
            <span class="auth-label">Description (optional)</span>
            <input v-model="description" class="auth-input w-full" :placeholder="`Split from '${profile?.name ?? ''}'.`" />
          </label>
        </div>

        <div class="grid grid-cols-1 gap-3 md:grid-cols-[1fr_auto_1fr]">
          <!-- Pick tables to move -->
          <div class="flex min-h-0 flex-col rounded-xl border border-border">
            <div class="flex items-center justify-between gap-2 border-b border-border px-3 py-2">
              <p class="text-xs font-semibold text-foreground">
                Tables in this pipeline <span class="font-normal text-muted-foreground">({{ tables.length }})</span>
              </p>
              <button type="button" class="text-xs font-medium text-primary hover:underline" @click="toggleVisible">
                {{ allVisiblePicked ? "Clear" : search ? "Select matches" : "Select all" }}
              </button>
            </div>
            <div class="relative border-b border-border px-3 py-2">
              <Search class="pointer-events-none absolute left-5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" :stroke-width="1.75" />
              <input v-model="search" class="auth-input h-8 w-full pl-7 text-xs" placeholder="Filter tables…" />
            </div>
            <ul class="max-h-64 overflow-y-auto p-1">
              <li v-for="t in visible" :key="t">
                <label class="flex cursor-pointer items-center gap-2 rounded-lg px-2 py-1.5 hover:bg-muted/40">
                  <input v-model="picked" type="checkbox" :value="t" class="h-4 w-4 rounded border-border accent-brand-500" />
                  <span class="truncate font-mono text-xs" :title="t">{{ t }}</span>
                </label>
              </li>
              <li v-if="!visible.length" class="px-2 py-3 text-xs text-muted-foreground">No table matches.</li>
            </ul>
          </div>

          <div class="hidden items-center justify-center md:flex">
            <ArrowRight class="h-5 w-5 text-muted-foreground" :stroke-width="1.75" />
          </div>

          <!-- Preview -->
          <div class="space-y-3">
            <div class="rounded-xl border border-primary/40 bg-primary/5 p-3">
              <p class="text-xs font-semibold text-foreground">
                Moves to <span class="text-primary">{{ name.trim() || "the new pipeline" }}</span>
                <span class="font-normal text-muted-foreground">({{ picked.length }})</span>
              </p>
              <ul class="mt-1 max-h-28 space-y-0.5 overflow-y-auto">
                <li v-for="t in picked" :key="t" class="truncate font-mono text-[11px] text-foreground">{{ t }}</li>
                <li v-if="!picked.length" class="text-[11px] text-muted-foreground">Select tables on the left.</li>
              </ul>
            </div>
            <div class="rounded-xl border border-border p-3">
              <p class="text-xs font-semibold text-foreground">
                Stays in {{ profile?.name }} <span class="font-normal text-muted-foreground">({{ staying.length }})</span>
              </p>
              <ul class="mt-1 max-h-28 space-y-0.5 overflow-y-auto">
                <li v-for="t in staying" :key="t" class="truncate font-mono text-[11px] text-muted-foreground">{{ t }}</li>
              </ul>
            </div>
          </div>
        </div>

        <div class="flex items-start gap-2 rounded-lg border border-border bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
          <Info class="mt-0.5 h-3.5 w-3.5 shrink-0" :stroke-width="1.75" />
          <div class="space-y-1">
            <p>Mappings and per-table poll settings move with the tables. The new pipeline gets its own connector and Kafka topics.</p>
            <p>{{ startNote }}</p>
            <p v-if="running">
              <span class="font-medium text-foreground">This pipeline is running:</span> it keeps capturing the moved
              tables until you restart it. Start the new pipeline first, then restart this one — nothing is missed in between.
            </p>
          </div>
        </div>
      </div>

      <!-- Step 2: done -->
      <div v-else class="flex-1 space-y-3 overflow-y-auto px-5 py-4 text-sm">
        <p class="flex items-center gap-2 font-medium text-emerald-600 dark:text-emerald-400">
          <CheckCircle2 class="h-4 w-4" :stroke-width="1.75" />
          Created “{{ result.created.name }}” with {{ result.moved_tables.length }} table(s) — saved as a draft.
        </p>
        <ol class="list-decimal space-y-1 pl-5 text-muted-foreground">
          <li>Open <span class="font-medium text-foreground">{{ result.created.name }}</span>, review it and press <span class="font-medium text-foreground">Start</span>.</li>
          <li v-if="result.original_restart_required">
            Then restart <span class="font-medium text-foreground">{{ result.original.name }}</span> so it stops capturing the moved tables.
          </li>
        </ol>
      </div>

      <p v-if="error" class="break-words px-5 pb-2 text-xs text-red-500">{{ error }}</p>

      <div class="flex flex-wrap items-center justify-between gap-2 border-t border-border px-5 py-3">
        <p v-if="!result" class="text-xs text-muted-foreground">{{ problem || `${picked.length} moving · ${staying.length} staying` }}</p>
        <span v-else />
        <div class="flex gap-2">
          <template v-if="!result">
            <AppButton :disabled="saving" @click="close">Cancel</AppButton>
            <AppButton variant="primary" :disabled="saving || Boolean(problem)" @click="submit">
              {{ saving ? "Creating…" : "Create new pipeline" }}
            </AppButton>
          </template>
          <template v-else>
            <AppButton @click="close">Close</AppButton>
            <AppButton v-if="result.original_restart_required" @click="emit('restart-original')">
              Restart {{ result.original.name }}
            </AppButton>
            <AppButton variant="primary" @click="emit('open-created', result.created.id)">
              Open {{ result.created.name }}
            </AppButton>
          </template>
        </div>
      </div>
    </div>
  </div>
</template>
