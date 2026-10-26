<script setup lang="ts">
import { Eye, RefreshCw, X } from "@lucide/vue"
import type { TopicMessages } from "~/composables/useKafkaM1"

const props = defineProps<{
  topic: string | null
}>()

const emit = defineEmits<{
  (e: "close"): void
}>()

const loading = ref(false)
const error = ref("")
const msg = ref<TopicMessages["items"][0] | null>(null)

// rpk outputs epoch-ms as a plain number string — new Date("1752644400000") is Invalid Date,
// so we must cast to Number first. Fall back to raw ISO strings too.
const formatTs = (ts: string | null | undefined): string => {
  if (!ts) return "—"
  const num = Number(ts)
  const d = !Number.isNaN(num) && num > 0 ? new Date(num) : new Date(ts)
  if (Number.isNaN(d.getTime())) return ts
  return d.toLocaleString(undefined, {
    year: "numeric", month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit", second: "2-digit",
  })
}

const tryJson = (s: string) => {
  try { return { ok: true, pretty: JSON.stringify(JSON.parse(s), null, 2) } }
  catch { return { ok: false, pretty: s } }
}

const valueInfo = computed(() => {
  if (!msg.value?.payload) return null
  const { ok, pretty } = tryJson(msg.value.payload)
  return { ok, pretty, bytes: new TextEncoder().encode(msg.value.payload).length }
})

const keyInfo = computed(() => {
  if (!msg.value?.key) return null
  const { ok, pretty } = tryJson(msg.value.key)
  return { ok, pretty, bytes: new TextEncoder().encode(msg.value.key).length }
})

const lockBody = (lock: boolean) => {
  if (typeof document === "undefined") return
  document.documentElement.classList.toggle("overflow-hidden", lock)
  document.body.classList.toggle("overflow-hidden", lock)
}

watch(() => props.topic, (t) => lockBody(!!t), { immediate: true })
onUnmounted(() => lockBody(false))

const onOverlayWheel = (event: WheelEvent) => {
  const target = event.target as HTMLElement | null
  if (!target?.closest("[data-modal-scroll]")) event.preventDefault()
}

const load = async () => {
  if (!props.topic) return
  loading.value = true
  error.value = ""
  msg.value = null
  try {
    const result = await $fetch<TopicMessages>(
      `/api/cdc/topics/${encodeURIComponent(props.topic)}/messages`,
      { query: { mode: "latest", limit: 1 } }
    )
    msg.value = result.items?.[0] ?? null
  } catch (e: any) {
    error.value = e?.data?.message || e?.message || "Failed to fetch last message"
  } finally {
    loading.value = false
  }
}

watch(() => props.topic, (t) => { if (t) load() }, { immediate: true })
</script>

<template>
  <Teleport to="body">
    <div
      v-if="topic"
      class="modal-overlay overflow-hidden overscroll-none"
      role="dialog"
      aria-modal="true"
      @click.self="emit('close')"
      @wheel="onOverlayWheel"
    >
      <section class="flex max-h-[min(90vh,calc(100dvh-2rem))] w-full max-w-2xl flex-col overflow-hidden rounded-xl border border-border bg-card shadow-2xl animate-slide-up">

        <!-- Header -->
        <div class="flex flex-shrink-0 items-center gap-3 border-b border-border/60 px-5 py-4">
          <Eye class="h-4 w-4 flex-shrink-0 text-primary" :stroke-width="1.75" />
          <div class="min-w-0 flex-1">
            <div class="text-sm font-semibold text-foreground">Last Message</div>
            <div class="mt-0.5 truncate font-mono text-xs text-muted-foreground" :title="topic">{{ topic }}</div>
          </div>
          <button
            type="button"
            class="icon-button"
            title="Refresh"
            :disabled="loading"
            @click="load"
          >
            <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
          </button>
          <button type="button" class="icon-button" title="Close" @click="emit('close')">
            <X class="h-4 w-4" :stroke-width="1.75" />
          </button>
        </div>

        <!-- Body -->
        <div data-modal-scroll class="min-h-0 flex-1 overflow-auto overscroll-contain px-5 py-4 space-y-4">

          <!-- Loading -->
          <div v-if="loading" class="flex items-center justify-center gap-2 py-10 text-sm text-muted-foreground">
            <RefreshCw class="h-4 w-4 animate-spin text-primary" :stroke-width="1.75" />
            Fetching last message…
          </div>

          <!-- Error -->
          <div v-else-if="error" class="rounded-xl border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-400">
            {{ error }}
          </div>

          <!-- Empty -->
          <div v-else-if="!msg" class="flex flex-col items-center justify-center gap-1 py-10 text-sm text-muted-foreground">
            <Eye class="mb-1 h-8 w-8 opacity-30" :stroke-width="1.5" />
            No messages in this topic yet.
          </div>

          <template v-else>
            <!-- Meta chips -->
            <div class="grid grid-cols-3 gap-2">
              <div class="rounded-xl border border-border/60 bg-muted/40 px-3 py-2.5">
                <div class="text-xs text-muted-foreground">Partition</div>
                <div class="mt-1 font-mono text-sm font-semibold text-foreground">{{ msg.partition ?? '—' }}</div>
              </div>
              <div class="rounded-xl border border-border/60 bg-muted/40 px-3 py-2.5">
                <div class="text-xs text-muted-foreground">Offset</div>
                <div class="mt-1 font-mono text-sm font-semibold text-foreground">{{ msg.offset ?? '—' }}</div>
              </div>
              <div class="rounded-xl border border-border/60 bg-muted/40 px-3 py-2.5">
                <div class="text-xs text-muted-foreground">Timestamp</div>
                <div class="mt-1 text-sm font-medium text-foreground/80 leading-tight">{{ formatTs(msg.timestamp) }}</div>
              </div>
            </div>

            <!-- Key -->
            <div v-if="keyInfo">
              <div class="mb-1.5 flex items-center justify-between">
                <span class="text-xs font-medium text-muted-foreground">Key</span>
                <span class="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                  {{ keyInfo.ok ? 'JSON' : 'RAW' }} · {{ keyInfo.bytes }} bytes
                </span>
              </div>
              <pre class="code-block max-h-[120px] overflow-auto whitespace-pre-wrap text-sm">{{ keyInfo.pretty }}</pre>
            </div>

            <!-- Value -->
            <div v-if="valueInfo">
              <div class="mb-1.5 flex items-center justify-between">
                <span class="text-xs font-medium text-muted-foreground">Value</span>
                <span class="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                  {{ valueInfo.ok ? 'JSON' : 'RAW' }} · {{ valueInfo.bytes }} bytes
                </span>
              </div>
              <pre class="code-block max-h-[320px] overflow-auto whitespace-pre-wrap text-sm">{{ valueInfo.pretty }}</pre>
            </div>
          </template>
        </div>

        <!-- Footer -->
        <div class="flex flex-shrink-0 items-center justify-between border-t border-border/60 px-5 py-3">
          <NuxtLink
            v-if="topic"
            :to="`/topics/${encodeURIComponent(topic)}?tab=messages`"
            class="text-xs text-primary hover:underline"
            @click="emit('close')"
          >
            View all messages →
          </NuxtLink>
          <span v-else />
          <button
            type="button"
            class="inline-flex h-9 items-center justify-center rounded-xl border border-border bg-secondary px-4 text-sm font-medium text-secondary-foreground transition-all hover:brightness-95"
            @click="emit('close')"
          >Close</button>
        </div>
      </section>
    </div>
  </Teleport>
</template>
