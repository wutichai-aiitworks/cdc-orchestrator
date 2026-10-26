<script setup lang="ts">
import { Copy, X, Zap } from "@lucide/vue"
import type { ProfilePayload } from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"

const props = defineProps<{
  open: boolean
  source?: ProfilePayload["source"]
  tables: string[]
  installing?: boolean
  installError?: string
}>()

const emit = defineEmits<{
  "update:open": [value: boolean]
  confirm: []
}>()

const { previewSourceTriggers } = useProfilesM3()
const toast = useToast()
const loading = ref(false)
const error = ref("")
const copied = ref(false)
const preview = ref<{
  tables: string[]
  change_log: string
  sql: string
  will_execute: boolean
  vendor: string
  message: string
} | null>(null)

const close = () => {
  if (props.installing) return
  emit("update:open", false)
}

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

const load = async () => {
  if (!props.open || !props.source || !props.tables.length) return
  loading.value = true
  error.value = ""
  copied.value = false
  preview.value = null
  try {
    preview.value = await previewSourceTriggers({
      source: props.source,
      tables: props.tables,
    })
  } catch (e: any) {
    preview.value = null
    error.value = formatApiError(e, "Failed to preview trigger SQL")
  } finally {
    loading.value = false
  }
}

watch(
  () => [props.open, props.tables.join(","), Boolean(props.source)] as const,
  ([open]) => {
    if (open) void load()
  },
  { immediate: true }
)

const copySql = async () => {
  const sql = preview.value?.sql
  if (!sql) return
  try {
    await navigator.clipboard.writeText(sql)
    copied.value = true
    toast.success("Trigger SQL copied")
  } catch {
    toast.error("Could not copy SQL")
  }
}

const confirmLabel = computed(() => {
  if (props.installing) return "Creating..."
  if (preview.value && !preview.value.will_execute) return "Use this SQL"
  return props.tables.length === 1 ? "Create trigger" : "Create triggers"
})
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      class="modal-overlay overflow-hidden overscroll-none"
      role="dialog"
      aria-modal="true"
      aria-labelledby="trigger-preview-title"
      @click.self="close"
      @wheel="onOverlayWheel"
    >
      <section class="flex h-[min(88vh,calc(100dvh-2rem))] w-full max-w-3xl flex-col overflow-hidden rounded-xl border border-border bg-card p-5 shadow-2xl animate-slide-up">
        <div class="mb-4 flex shrink-0 items-start justify-between gap-3">
          <div class="min-w-0">
            <h2 id="trigger-preview-title" class="section-title">Confirm trigger create</h2>
            <p class="mt-1 text-sm text-muted-foreground">
              Review the CREATE statements before they run on the source.
            </p>
          </div>
          <button type="button" class="icon-button" aria-label="Close" :disabled="installing" @click="close">
            <X class="h-4 w-4" :stroke-width="1.75" />
          </button>
        </div>

        <p v-if="error || installError" class="mb-3 shrink-0 text-sm text-red-700 dark:text-red-400">
          {{ installError || error }}
        </p>
        <p
          v-else-if="preview"
          class="mb-3 shrink-0 rounded-xl border px-3 py-2 text-sm"
          :class="
            preview.will_execute
              ? 'border-yellow-500/30 bg-yellow-500/10 text-yellow-800 dark:text-yellow-300'
              : 'border-blue-500/25 bg-blue-500/10 text-blue-700 dark:text-blue-400'
          "
        >
          {{ preview.message }}
        </p>

        <div v-if="preview" class="mb-3 flex shrink-0 flex-wrap items-center justify-between gap-2">
          <div class="min-w-0 text-sm">
            <p class="font-medium">{{ preview.vendor }} · {{ preview.tables.length }} table{{ preview.tables.length === 1 ? "" : "s" }}</p>
            <p class="mt-0.5 font-mono text-xs text-muted-foreground">Changelog: {{ preview.change_log }}</p>
            <p class="mt-1 font-mono text-xs text-muted-foreground">{{ preview.tables.join(", ") }}</p>
          </div>
          <AppButton size="sm" :disabled="!preview.sql" @click="copySql">
            <Copy class="h-4 w-4" :stroke-width="1.75" />
            {{ copied ? "Copied" : "Copy SQL" }}
          </AppButton>
        </div>

        <div data-modal-scroll class="min-h-0 flex-1 overflow-auto overscroll-contain">
          <AppStateBlock v-if="loading && !preview" kind="loading" message="Loading CREATE statements…" />
          <pre v-else-if="preview?.sql" class="code-block min-h-full whitespace-pre-wrap text-xs">{{ preview.sql }}</pre>
          <AppStateBlock v-else-if="!error" kind="empty" message="No trigger SQL to show." />
        </div>

        <div class="mt-4 flex shrink-0 justify-end gap-2">
          <AppButton :disabled="installing" @click="close">Cancel</AppButton>
          <AppButton
            variant="primary"
            :disabled="loading || installing || !preview?.sql"
            @click="emit('confirm')"
          >
            <Zap class="h-4 w-4" :stroke-width="1.75" />
            {{ confirmLabel }}
          </AppButton>
        </div>
      </section>
    </div>
  </Teleport>
</template>
