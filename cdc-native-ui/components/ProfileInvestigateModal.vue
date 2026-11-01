<script setup lang="ts">
import { X } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"

const props = defineProps<{
  open: boolean
  profileId: string
  profileName?: string
}>()

const emit = defineEmits<{
  "update:open": [value: boolean]
}>()

const { fetchProfileInvestigate } = useOperationsM3()
const toast = useToast()

const tab = ref<"tables" | "diagnostics">("tables")
const investigateLoading = ref(false)
const investigatePayload = ref<Record<string, unknown> | null>(null)
const panelRef = ref<{ refresh: () => Promise<void> } | null>(null)

const title = computed(() => {
  if (props.profileName) return props.profileName
  const p = investigatePayload.value?.profile
  if (p && typeof p === "object" && p !== null && "name" in p) {
    const n = (p as { name?: string }).name
    if (n) return n
  }
  return "Pipeline"
})

const close = () => emit("update:open", false)

const loadDiagnostics = async () => {
  if (!props.profileId) return
  investigateLoading.value = true
  investigatePayload.value = null
  try {
    investigatePayload.value = await fetchProfileInvestigate(props.profileId)
  } catch (e: unknown) {
    toast.error(formatApiError(e, "Diagnostics failed"))
  } finally {
    investigateLoading.value = false
  }
}

watch(
  () => [props.open, props.profileId] as const,
  ([isOpen, id]) => {
    if (!isOpen || !id) return
    tab.value = "tables"
    void loadDiagnostics()
    nextTick(() => panelRef.value?.refresh())
  },
)
</script>

<template>
  <Teleport to="body">
    <Transition name="modal">
      <div v-if="open" class="modal-overlay" @click.self="close">
        <div
          class="modal-panel flex max-h-[92vh] w-full max-w-6xl flex-col gap-0 overflow-hidden p-0 animate-slide-up"
          role="dialog"
          aria-labelledby="investigate-title"
        >
          <header
            class="flex flex-wrap items-center justify-between gap-3 border-b border-border px-6 py-4"
          >
            <div class="min-w-0">
              <p class="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                Pipeline investigate
              </p>
              <h2 id="investigate-title" class="truncate text-lg font-semibold text-foreground">
                {{ title }}
              </h2>
            </div>
            <div class="flex items-center gap-2">
              <div
                class="inline-flex rounded-xl border border-border bg-muted/40 p-0.5 text-sm"
                role="tablist"
              >
                <button
                  type="button"
                  role="tab"
                  class="rounded-lg px-3 py-1.5 font-medium transition-colors"
                  :class="
                    tab === 'tables'
                      ? 'bg-card text-foreground shadow-sm'
                      : 'text-muted-foreground hover:text-foreground'
                  "
                  @click="tab = 'tables'"
                >
                  Table sync
                </button>
                <button
                  type="button"
                  role="tab"
                  class="rounded-lg px-3 py-1.5 font-medium transition-colors"
                  :class="
                    tab === 'diagnostics'
                      ? 'bg-card text-foreground shadow-sm'
                      : 'text-muted-foreground hover:text-foreground'
                  "
                  @click="tab = 'diagnostics'"
                >
                  Diagnostics
                </button>
              </div>
              <button type="button" class="icon-button" aria-label="Close" @click="close">
                <X class="h-5 w-5" :stroke-width="1.75" />
              </button>
            </div>
          </header>

          <div class="min-h-0 flex-1 overflow-y-auto px-6 py-5">
            <ProfileTableSyncPanel
              v-show="tab === 'tables'"
              ref="panelRef"
              :profile-id="profileId"
              :auto-load="false"
            />
            <div v-if="tab === 'diagnostics'" class="flex min-h-[40vh] flex-col gap-3">
              <p class="text-sm text-muted-foreground">
                Connector status, offsets snapshot, and recent pipeline logs from the orchestrator.
              </p>
              <div v-if="investigateLoading" class="text-sm text-muted-foreground">Loading…</div>
              <ClientOnly v-else>
                <AppJsonViewer
                  :content="
                    investigatePayload ? JSON.stringify(investigatePayload, null, 2) : '{}'
                  "
                  height="min(60vh, 520px)"
                />
              </ClientOnly>
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
