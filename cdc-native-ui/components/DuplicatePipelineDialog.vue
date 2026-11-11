<script setup lang="ts">
/**
 * Duplicate a pipeline: name the copy and create it as a Draft with everything the
 * original has (connections, capture mode, schedule, tables, mappings, poll rules).
 * The original is untouched and nothing starts; the result step opens the copy so
 * tables, target and keys can be edited before its first Start.
 */
import { CheckCircle2, Copy, Info } from "@lucide/vue"
import type { CdcProfile, DuplicateProfileResult } from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"

const props = defineProps<{
  open: boolean
  profile: CdcProfile | null
}>()

const emit = defineEmits<{
  "update:open": [value: boolean]
  /** Copy saved: reload the list. */
  done: [result: DuplicateProfileResult]
  /** Open the new pipeline. */
  "open-created": [id: string]
}>()

const { duplicateProfile } = useProfilesM3()

const name = ref("")
const description = ref("")
const saving = ref(false)
const error = ref("")
const result = ref<DuplicateProfileResult | null>(null)

const tables = computed(() => (props.profile?.source?.tables ?? []).filter((t) => t.trim()))
const mode = computed(() => String(props.profile?.source?.capture_mode ?? "logical"))

const problem = computed(() => (name.value.trim() ? "" : "Name the new pipeline."))

/** What the copy shares with the original, by capture mode. */
const shareNote = computed(() => {
  switch (mode.value) {
    case "logical":
      return "The copy gets its own replication slot and Kafka topics when started. Starting both on the same tables doubles the load on the source and the target rows are upserted twice."
    case "trigger":
      return "Trigger capture: the copy would reuse the same source triggers and changelog. Change its tables or target before starting it, or it competes with this pipeline for the same rows."
    case "incremental":
      return "The copy keeps the same watermark columns, upsert keys and per-table poll strategies. It starts from the beginning (no watermark is inherited)."
    case "sync_once":
      return "The copy runs its own one-time snapshot when started."
    default:
      return ""
  }
})

watch(
  () => props.open,
  (open) => {
    if (!open) return
    name.value = props.profile ? `${props.profile.name} · copy` : ""
    description.value = ""
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
    result.value = await duplicateProfile(props.profile.id, {
      name: name.value.trim(),
      description: description.value.trim() || undefined,
    })
    emit("done", result.value)
  } catch (e: unknown) {
    error.value = formatApiError(e, "Duplicate failed")
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <div v-if="open" class="modal-overlay" @click.self="close">
    <div class="flex max-h-[90vh] w-full max-w-xl flex-col rounded-xl border border-border bg-card shadow-2xl">
      <div class="border-b border-border px-5 py-4">
        <h3 class="flex items-center gap-2 text-base font-semibold text-foreground">
          <Copy class="h-4 w-4" :stroke-width="1.75" />
          Duplicate as a new pipeline
        </h3>
        <p class="mt-1 text-xs text-muted-foreground">
          A full copy of <span class="font-medium text-foreground">{{ profile?.name }}</span> — same source, target,
          capture mode, schedule, tables and mappings. Saved as a draft; nothing starts until you start it.
        </p>
      </div>

      <!-- Step 1: name it -->
      <div v-if="!result" class="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-4 text-sm">
        <label class="block">
          <span class="auth-label">New pipeline name</span>
          <input v-model="name" class="auth-input w-full" placeholder="e.g. CRM_STG Master · CarType" />
        </label>
        <label class="block">
          <span class="auth-label">Description (optional)</span>
          <input v-model="description" class="auth-input w-full" :placeholder="`Duplicated from '${profile?.name ?? ''}'.`" />
        </label>

        <div class="rounded-xl border border-border p-3">
          <p class="text-xs font-semibold text-foreground">
            Tables copied <span class="font-normal text-muted-foreground">({{ tables.length }})</span>
          </p>
          <ul class="mt-1 max-h-32 space-y-0.5 overflow-y-auto">
            <li v-for="t in tables" :key="t" class="truncate font-mono text-[11px] text-muted-foreground">{{ t }}</li>
            <li v-if="!tables.length" class="text-[11px] text-muted-foreground">All tables of the selected schemas.</li>
          </ul>
        </div>

        <div class="flex items-start gap-2 rounded-lg border border-border bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
          <Info class="mt-0.5 h-3.5 w-3.5 shrink-0" :stroke-width="1.75" />
          <div class="space-y-1">
            <p>
              The copy is a template: open it, change the tables, target table names, upsert keys and poll strategy,
              then Start. The original is not changed.
            </p>
            <p v-if="shareNote">{{ shareNote }}</p>
          </div>
        </div>
      </div>

      <!-- Step 2: done -->
      <div v-else class="flex-1 space-y-3 overflow-y-auto px-5 py-4 text-sm">
        <p class="flex items-center gap-2 font-medium text-emerald-600 dark:text-emerald-400">
          <CheckCircle2 class="h-4 w-4" :stroke-width="1.75" />
          Created “{{ result.created.name }}” — saved as a draft.
        </p>
        <ol class="list-decimal space-y-1 pl-5 text-muted-foreground">
          <li>Open <span class="font-medium text-foreground">{{ result.created.name }}</span> and press <span class="font-medium text-foreground">Edit</span>.</li>
          <li>Adjust tables, target names, upsert keys and poll strategy, then <span class="font-medium text-foreground">Start</span>.</li>
        </ol>
      </div>

      <p v-if="error" class="break-words px-5 pb-2 text-xs text-red-500">{{ error }}</p>

      <div class="flex flex-wrap items-center justify-between gap-2 border-t border-border px-5 py-3">
        <p v-if="!result" class="text-xs text-muted-foreground">{{ problem }}</p>
        <span v-else />
        <div class="flex gap-2">
          <template v-if="!result">
            <AppButton :disabled="saving" @click="close">Cancel</AppButton>
            <AppButton variant="primary" :disabled="saving || Boolean(problem)" @click="submit">
              {{ saving ? "Creating…" : "Create copy" }}
            </AppButton>
          </template>
          <template v-else>
            <AppButton @click="close">Close</AppButton>
            <AppButton variant="primary" @click="emit('open-created', result.created.id)">
              Open {{ result.created.name }}
            </AppButton>
          </template>
        </div>
      </div>
    </div>
  </div>
</template>
