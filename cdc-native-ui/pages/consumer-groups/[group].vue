<script setup lang="ts">
import { ChevronDown, ChevronRight, Eye, MoreVertical, RefreshCw, RotateCcw, Search, Square, Trash2 } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"

const route = useRoute()
const toast = useToast()
const { fetchGroup, deleteGroup, resetGroup, deleteTopicOffsets } = useKafkaAdmin()
const { runAction: runConnectorAction, fetchConnectorStatus } = useConnectorsM2()

const group = computed(() => decodeURIComponent(String(route.params.group || "")))
const isInternal = computed(() =>
  group.value.startsWith("__") ||
  group.value === "_schemas" ||
  group.value.endsWith("-cluster"),
)

// Kafka Connect sink groups are named `connect-<connector>`. For those, deleting
// the group requires stopping the owning connector first (otherwise the broker
// rejects the delete because the group still has active members).
const sinkConnectorName = computed(() =>
  group.value.startsWith("connect-") ? group.value.slice("connect-".length) : null,
)
const loading = ref(false)
const error = ref("")
const detail = ref<Awaited<ReturnType<typeof fetchGroup>> | null>(null)
let timer: ReturnType<typeof setInterval> | null = null

// ── Load ───────────────────────────────────────────────────────────────────
const load = async () => {
  loading.value = true
  error.value = ""
  try {
    detail.value = await fetchGroup(group.value)
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load consumer group")
  } finally {
    loading.value = false
  }
}

// ── Header 3-dots menu ────────────────────────────────────────────────────
const headerMenuOpen = ref(false)
const headerMenuBtnRef = ref<HTMLElement | null>(null)
const headerMenuStyle = ref({ top: "0px", right: "0px" })

const openHeaderMenu = () => {
  if (headerMenuBtnRef.value) {
    const rect = headerMenuBtnRef.value.getBoundingClientRect()
    headerMenuStyle.value = {
      top: `${rect.bottom + 4}px`,
      right: `${window.innerWidth - rect.right}px`,
    }
  }
  headerMenuOpen.value = true
}

// ── Per-topic 3-dots menu ──────────────────────────────────────────────────
const topicMenuFor = ref<string | null>(null)
const topicMenuStyle = ref({ top: "0px", left: "0px" })
const topicMenuBtnRefs = ref<Map<string, HTMLElement>>(new Map())

const setTopicMenuRef = (el: any, t: string) => {
  if (el) topicMenuBtnRefs.value.set(t, el as HTMLElement)
}

const openTopicMenu = (t: string, e: MouseEvent) => {
  e.stopPropagation()
  topicMenuFor.value = t
  const btn = topicMenuBtnRefs.value.get(t)
  if (btn) {
    const rect = btn.getBoundingClientRect()
    topicMenuStyle.value = {
      top: `${rect.bottom + 4}px`,
      left: `${Math.max(8, rect.right - 200)}px`,
    }
  }
}

const closeTopicMenu = () => { topicMenuFor.value = null }

// ── Peek last message ─────────────────────────────────────────────────────
const peekTopic = ref<string | null>(null)

// ── Delete offsets for a topic ────────────────────────────────────────────
const pendingDeleteOffsets = ref<string | null>(null)
const deletingOffsets = ref(false)
const confirmDeleteOffsets = async () => {
  const topic = pendingDeleteOffsets.value
  if (!topic) return
  pendingDeleteOffsets.value = null
  deletingOffsets.value = true
  try {
    await deleteTopicOffsets(group.value, topic)
    toast.success(`Offsets deleted for "${topic}"`)
    await load()
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to delete offsets"))
  } finally {
    deletingOffsets.value = false
  }
}

// ── Delete ─────────────────────────────────────────────────────────────────
const pendingDelete = ref(false)
const remove = async () => {
  pendingDelete.value = false
  try {
    await deleteGroup(group.value)
    toast.success("Consumer group deleted")
    await navigateTo("/consumer-groups")
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to delete group"))
  }
}

// ── Stop connector & delete group (Kafka Connect sink groups) ───────────────
const pendingStopDelete = ref(false)
const stopDeleting = ref(false)
const stopDeleteStep = ref("")

/** Poll the connector until it reports STOPPED, so its consumer leaves the group. */
const waitForConnectorStopped = async (connector: string, maxMs = 30_000) => {
  const deadline = Date.now() + maxMs
  while (Date.now() < deadline) {
    try {
      const res = await fetchConnectorStatus(connector)
      if (res?.status?.connector?.state === "STOPPED") return true
    } catch { /* keep polling */ }
    await new Promise((r) => setTimeout(r, 1_500))
  }
  return false
}

const stopAndDeleteGroup = async () => {
  const connector = sinkConnectorName.value
  if (!connector) return
  pendingStopDelete.value = false
  stopDeleting.value = true
  try {
    stopDeleteStep.value = `Stopping connector ${connector}…`
    await runConnectorAction(connector, "stop")

    stopDeleteStep.value = "Waiting for consumer to leave the group…"
    const stopped = await waitForConnectorStopped(connector)
    if (!stopped) {
      throw new Error(
        `Connector ${connector} did not reach STOPPED in time. Its consumer may still be active — retry in a moment.`,
      )
    }
    // Extra grace for the broker-side heartbeat timeout to expire.
    await new Promise((r) => setTimeout(r, 2_000))

    stopDeleteStep.value = "Deleting consumer group…"
    await deleteGroup(group.value)
    toast.success(`Stopped ${connector} and deleted the group`)
    await navigateTo("/consumer-groups")
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to stop connector and delete group"))
  } finally {
    stopDeleting.value = false
    stopDeleteStep.value = ""
  }
}

// ── Inline Reset Offsets ──────────────────────────────────────────────────
const showResetPanel = ref(false)
const resetType = ref("earliest")
const resetTopic = ref("")
const resetTimestampMs = ref("")
const resetting = ref(false)

const resetTopics = computed(() => [...new Set((detail.value?.entries ?? []).map((e) => e.topic))])

const submitReset = async () => {
  resetting.value = true
  try {
    await resetGroup(group.value, {
      reset_type: resetType.value,
      topic: resetTopic.value || undefined,
      timestamp_ms: resetType.value === "timestamp" ? Number(resetTimestampMs.value) : undefined,
    })
    toast.success("Offsets reset successfully")
    showResetPanel.value = false
    headerMenuOpen.value = false
    await load()
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to reset offsets"))
  } finally {
    resetting.value = false
  }
}

// ── Grouping by topic ──────────────────────────────────────────────────────
type Entry = NonNullable<typeof detail.value>["entries"][number]

const searchTopic = ref("")

const groupedByTopic = computed(() => {
  const entries = detail.value?.entries ?? []
  const map = new Map<string, Entry[]>()
  for (const e of entries) {
    if (!map.has(e.topic)) map.set(e.topic, [])
    map.get(e.topic)!.push(e)
  }
  return Array.from(map.entries())
    .map(([topic, rows]) => ({
      topic,
      rows: rows.sort((a, b) => a.partition - b.partition),
      consumerLag: rows.reduce((s, r) => s + r.lag, 0),
    }))
    .filter((g) =>
      searchTopic.value.trim() === "" ||
      g.topic.toLowerCase().includes(searchTopic.value.trim().toLowerCase()),
    )
})

const expandedTopics = ref(new Set<string>())
const toggleTopic = (t: string) => {
  if (expandedTopics.value.has(t)) expandedTopics.value.delete(t)
  else expandedTopics.value.add(t)
}

watch(
  () => detail.value,
  (d) => {
    if (d) expandedTopics.value = new Set((d.entries ?? []).map((e) => e.topic))
  },
  { immediate: true },
)

// ── Derived metrics ────────────────────────────────────────────────────────
const assignedTopics = computed(() => new Set((detail.value?.entries ?? []).map((e) => e.topic)).size)
const assignedPartitions = computed(() => (detail.value?.entries ?? []).length)

// ── Colour helpers ─────────────────────────────────────────────────────────
const lagBadge = (lag: number) => {
  if (lag === 0) return "text-emerald-600 dark:text-emerald-400"
  if (lag < 1000) return "text-yellow-600 dark:text-yellow-400"
  return "text-red-500 dark:text-red-400"
}

const stateBg = (s: string | null | undefined) => {
  const v = (s || "").toLowerCase()
  if (v === "stable") return "bg-emerald-500/15 text-emerald-700 dark:text-emerald-400 border-emerald-500/30"
  if (v === "dead") return "bg-red-500/15 text-red-700 dark:text-red-400 border-red-500/30"
  if (v === "rebalancing" || v === "preparing") return "bg-yellow-500/15 text-yellow-700 dark:text-yellow-400 border-yellow-500/30"
  return "bg-surface-700/30 text-muted-foreground border-border"
}

// ── Lifecycle ──────────────────────────────────────────────────────────────
onMounted(async () => {
  await load()
  timer = setInterval(load, 10_000)
})
onUnmounted(() => { if (timer) clearInterval(timer) })
</script>

<template>
  <div class="page-container max-w-6xl space-y-4">
    <!-- Breadcrumb + title row -->
    <div class="flex items-center justify-between">
      <div class="min-w-0">
        <div class="flex items-center gap-1.5 text-xs text-muted-foreground">
          <NuxtLink to="/consumer-groups" class="hover:text-foreground">Consumers</NuxtLink>
          <span>/</span>
          <span class="font-medium text-foreground">{{ group }}</span>
          <span
            v-if="isInternal"
            class="inline-flex items-center rounded border border-blue-500/30 bg-blue-500/10 px-1.5 py-0.5 text-xs font-medium text-blue-600 dark:text-blue-400"
            title="Kafka Connect or Kafka infrastructure group — not a user consumer group"
          >Internal</span>
        </div>
      </div>
      <div class="flex items-center gap-2">
        <AppButton :disabled="loading" size="icon" title="Refresh" @click="load">
          <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        </AppButton>
        <!-- Header 3-dots -->
        <button
          ref="headerMenuBtnRef"
          type="button"
          class="inline-flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground hover:bg-accent hover:text-foreground"
          @click="openHeaderMenu"
        >
          <MoreVertical class="h-4 w-4" :stroke-width="1.75" />
        </button>
      </div>
    </div>

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <!-- Metric strip — matches Kafbat's horizontal row -->
    <div class="flex flex-wrap gap-0 divide-x divide-border rounded-xl border border-border bg-card">
      <!-- State -->
      <div class="flex min-w-[100px] flex-col gap-1 px-4 py-3">
        <span class="text-xs text-muted-foreground">State</span>
        <span
          class="inline-block self-start rounded-md border px-2 py-0.5 text-xs font-medium"
          :class="stateBg(detail?.state)"
        >{{ detail?.state || '—' }}</span>
      </div>
      <!-- Members -->
      <div class="flex min-w-[80px] flex-col gap-1 px-4 py-3">
        <span class="text-xs text-muted-foreground">Members</span>
        <span class="text-sm font-semibold text-foreground">{{ detail?.members ?? '—' }}</span>
      </div>
      <!-- Assigned Topics -->
      <div class="flex min-w-[110px] flex-col gap-1 px-4 py-3">
        <span class="text-xs text-muted-foreground">Assigned Topics</span>
        <span class="text-sm font-semibold text-foreground">{{ detail ? assignedTopics : '—' }}</span>
      </div>
      <!-- Assigned Partitions -->
      <div class="flex min-w-[130px] flex-col gap-1 px-4 py-3">
        <span class="text-xs text-muted-foreground">Assigned Partitions</span>
        <span class="text-sm font-semibold text-foreground">{{ detail ? assignedPartitions : '—' }}</span>
      </div>
      <!-- Coordinator ID -->
      <div class="flex min-w-[110px] flex-col gap-1 px-4 py-3">
        <span class="text-xs text-muted-foreground">Coordinator ID</span>
        <span class="font-mono text-sm font-semibold text-foreground">{{ detail?.coordinator ?? '—' }}</span>
      </div>
      <!-- Total Lag -->
      <div class="flex min-w-[90px] flex-col gap-1 px-4 py-3">
        <span class="text-xs text-muted-foreground">Total lag</span>
        <span class="font-mono text-sm font-semibold" :class="lagBadge(detail?.total_lag ?? 0)">
          {{ detail?.total_lag ?? '—' }}
        </span>
      </div>
    </div>

    <!-- Inline Reset panel -->
    <AppCard v-if="showResetPanel" class="border-brand-500/30 bg-brand-500/5 space-y-4">
      <div class="flex items-center justify-between">
        <h2 class="text-sm font-semibold">Reset Offsets</h2>
        <button class="text-xs text-muted-foreground hover:text-foreground" @click="showResetPanel = false">✕</button>
      </div>
      <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <label>
          <span class="auth-label">Reset type</span>
          <AppSelect v-model="resetType">
            <option value="earliest">Earliest</option>
            <option value="latest">Latest</option>
            <option value="timestamp">Timestamp</option>
          </AppSelect>
        </label>
        <label>
          <span class="auth-label">Topic (optional)</span>
          <AppSelect v-model="resetTopic">
            <option value="">All assigned topics</option>
            <option v-for="t in resetTopics" :key="t" :value="t">{{ t }}</option>
          </AppSelect>
        </label>
        <label v-if="resetType === 'timestamp'">
          <span class="auth-label">Timestamp (epoch ms)</span>
          <AppInput v-model="resetTimestampMs" type="number" placeholder="1710000000000" />
        </label>
      </div>
      <div class="flex justify-end gap-2">
        <AppButton @click="showResetPanel = false">Cancel</AppButton>
        <AppButton variant="primary" :disabled="resetting" @click="submitReset">
          <RefreshCw v-if="resetting" class="h-4 w-4 animate-spin" :stroke-width="1.75" />
          <RotateCcw v-else class="h-4 w-4" :stroke-width="1.75" />
          {{ resetting ? "Resetting…" : "Apply Reset" }}
        </AppButton>
      </div>
    </AppCard>

    <!-- Search by topic -->
    <div class="relative">
      <Search class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" :stroke-width="1.75" />
      <AppInput v-model="searchTopic" input-class="pl-9" placeholder="Search by Topic Name" />
    </div>

    <!-- Topics expandable table — Kafbat-style -->
    <AppCard class="overflow-hidden !p-0">
      <!-- Loading state -->
      <div v-if="loading && !detail" class="flex items-center justify-center gap-2 py-10 text-sm text-muted-foreground">
        <RefreshCw class="h-4 w-4 animate-spin opacity-50" :stroke-width="1.75" />
        Loading…
      </div>

      <template v-else>
        <!-- Topic list header -->
        <div class="grid grid-cols-[1fr_140px_112px] border-b border-border px-4 py-2.5 text-xs font-medium text-muted-foreground">
          <div>Topic</div>
          <div class="text-right">Consumer Lag</div>
          <div />
        </div>

        <template v-for="grp in groupedByTopic" :key="grp.topic">
          <!-- Topic row -->
          <div
            class="group grid cursor-pointer grid-cols-[1fr_140px_112px] items-center border-b border-border/50 px-4 py-2.5 transition-colors hover:bg-accent/30"
            :class="expandedTopics.has(grp.topic) ? 'bg-accent/20' : ''"
            @click="toggleTopic(grp.topic)"
          >
            <!-- Topic name + expand toggle -->
            <div class="flex min-w-0 items-center gap-2">
              <button
                type="button"
                class="flex h-5 w-5 flex-shrink-0 items-center justify-center rounded border border-border text-xs text-muted-foreground transition-colors"
                :class="expandedTopics.has(grp.topic) ? 'border-brand-500 bg-brand-500/10 text-brand-400' : ''"
                @click.stop="toggleTopic(grp.topic)"
              >
                <component :is="expandedTopics.has(grp.topic) ? ChevronDown : ChevronRight" class="h-3 w-3" :stroke-width="2" />
              </button>
              <NuxtLink
                :to="`/topics/${encodeURIComponent(grp.topic)}`"
                class="truncate font-mono text-sm hover:text-brand-500"
                @click.stop
              >{{ grp.topic }}</NuxtLink>
            </div>
            <!-- Consumer Lag -->
            <div class="text-right font-mono text-sm font-semibold" :class="lagBadge(grp.consumerLag)">
              {{ grp.consumerLag }}
            </div>
            <!-- Action buttons: peek + delete offsets + 3-dots (always visible) -->
            <div class="flex items-center justify-end gap-0.5">
              <button
                type="button"
                title="Peek last message"
                class="inline-flex h-7 w-7 items-center justify-center rounded-lg text-surface-400 hover:bg-accent hover:text-brand-400"
                @click.stop="peekTopic = grp.topic"
              >
                <Eye class="h-3.5 w-3.5" :stroke-width="1.75" />
              </button>
              <button
                type="button"
                title="Delete Offsets"
                class="inline-flex h-7 w-7 items-center justify-center rounded-lg text-red-500 hover:bg-red-500/10"
                @click.stop="pendingDeleteOffsets = grp.topic"
              >
                <Trash2 class="h-3.5 w-3.5" :stroke-width="1.75" />
              </button>
              <button
                :ref="(el) => setTopicMenuRef(el, grp.topic)"
                type="button"
                title="More actions"
                class="inline-flex h-7 w-7 items-center justify-center rounded-lg text-muted-foreground hover:bg-accent hover:text-foreground"
                :class="{ 'bg-accent text-foreground': topicMenuFor === grp.topic }"
                @click.stop="openTopicMenu(grp.topic, $event)"
              >
                <MoreVertical class="h-3.5 w-3.5" :stroke-width="1.75" />
              </button>
            </div>
          </div>

          <!-- Expanded partition sub-table -->
          <div v-if="expandedTopics.has(grp.topic)" class="border-b border-border bg-surface-900/20">
            <!-- Sub-header -->
            <div class="grid grid-cols-[80px_1fr_140px_100px_130px_130px] border-b border-border/50 px-8 py-2 text-xs font-medium text-muted-foreground">
              <div>Partition</div>
              <div>Consumer ID</div>
              <div>Host</div>
              <div class="text-right">Consumer Lag</div>
              <div class="text-right">Current Offset</div>
              <div class="text-right">End Offset</div>
            </div>
            <!-- Partition rows -->
            <div
              v-for="row in grp.rows"
              :key="`${row.topic}-${row.partition}`"
              class="grid grid-cols-[80px_1fr_140px_100px_130px_130px] items-center border-b border-border/30 px-8 py-2 text-sm"
            >
              <div class="font-mono font-medium text-foreground">{{ row.partition }}</div>
              <div class="min-w-0 overflow-hidden pr-4">
                <span class="block truncate font-mono text-muted-foreground" :title="row.consumer_id || undefined">
                  {{ row.consumer_id || '—' }}
                </span>
              </div>
              <div class="font-mono text-muted-foreground">{{ row.host || '—' }}</div>
              <div class="text-right font-mono font-semibold" :class="lagBadge(row.lag)">{{ row.lag }}</div>
              <div class="text-right font-mono text-foreground">{{ row.current_offset }}</div>
              <div class="text-right font-mono text-foreground">{{ row.log_end_offset }}</div>
            </div>
          </div>
        </template>

        <!-- Empty -->
        <div v-if="detail && groupedByTopic.length === 0" class="py-10">
          <AppStateBlock kind="empty" :message="searchTopic ? 'No topics match your search.' : 'No assigned partitions for this group.'" />
        </div>
      </template>
    </AppCard>

    <!-- Header 3-dots menu (Teleport) -->
    <Teleport to="body">
      <div
        v-if="headerMenuOpen"
        class="fixed z-[9999] min-w-[200px] rounded-xl border border-border bg-popover p-1 shadow-2xl"
        :style="{ top: headerMenuStyle.top, right: headerMenuStyle.right }"
        @click.stop
      >
        <div class="fixed inset-0 -z-10" @click="headerMenuOpen = false" />
        <div class="px-3 py-1.5 text-xs font-medium text-muted-foreground truncate max-w-[240px]">{{ group }}</div>
        <div class="my-0.5 border-t border-border/50" />
        <button
          type="button"
          class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm font-medium text-foreground hover:bg-accent"
          @click="showResetPanel = true; headerMenuOpen = false"
        >
          <RotateCcw class="h-3.5 w-3.5" :stroke-width="1.75" />
          Reset Offsets
        </button>
        <div class="my-0.5 border-t border-border/50" />
        <!-- Kafka Connect sink group: stop the connector first, then delete -->
        <button
          v-if="sinkConnectorName"
          type="button"
          class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm font-medium text-red-500 hover:bg-red-500/10"
          @click="pendingStopDelete = true; headerMenuOpen = false"
        >
          <Square class="h-3.5 w-3.5" :stroke-width="1.75" />
          Stop Connector &amp; Delete Group
        </button>
        <button
          type="button"
          class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm font-medium text-red-500 hover:bg-red-500/10"
          @click="pendingDelete = true; headerMenuOpen = false"
        >
          <Trash2 class="h-3.5 w-3.5" :stroke-width="1.75" />
          Delete Group
        </button>
      </div>
    </Teleport>

    <!-- Per-topic 3-dots menu (Teleport) -->
    <Teleport to="body">
      <div
        v-if="topicMenuFor"
        class="fixed z-[9999] min-w-[180px] rounded-xl border border-border bg-popover p-1 shadow-2xl"
        :style="topicMenuStyle"
        @click.stop
      >
        <div class="fixed inset-0 -z-10" @click="closeTopicMenu" />
        <div class="px-3 py-1.5 text-xs font-medium text-muted-foreground truncate max-w-[200px]">{{ topicMenuFor }}</div>
        <div class="my-0.5 border-t border-border/50" />
        <NuxtLink
          :to="`/topics/${encodeURIComponent(topicMenuFor)}`"
          class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm font-medium text-foreground hover:bg-accent"
          @click="closeTopicMenu"
        >
          Open Topic
        </NuxtLink>
        <button
          type="button"
          class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm font-medium text-foreground hover:bg-accent"
          @click="peekTopic = topicMenuFor; closeTopicMenu()"
        >
          Peek Last Message
        </button>
        <button
          type="button"
          class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm font-medium text-foreground hover:bg-accent"
          @click="resetTopic = topicMenuFor ?? ''; showResetPanel = true; closeTopicMenu()"
        >
          <RotateCcw class="h-3.5 w-3.5" :stroke-width="1.75" />
          Reset Offsets for Topic
        </button>
        <div class="my-0.5 border-t border-border/50" />
        <button
          type="button"
          class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm font-medium text-red-500 hover:bg-red-500/10"
          @click="pendingDeleteOffsets = topicMenuFor; closeTopicMenu()"
        >
          <Trash2 class="h-3.5 w-3.5 flex-shrink-0" :stroke-width="1.75" />
          Delete Offsets
        </button>
      </div>
    </Teleport>

    <!-- Delete confirm -->
    <AppConfirmDialog
      :open="pendingDelete"
      title="Delete consumer group?"
      danger
      confirm-label="Delete"
      @update:open="(v) => { if (!v) pendingDelete = false }"
      @confirm="remove"
    >
      Delete group <strong class="font-mono">{{ group }}</strong>? This cannot be undone.
      <p v-if="sinkConnectorName" class="mt-2 text-xs text-amber-600 dark:text-amber-400">
        This looks like a Kafka Connect sink group. If its connector
        (<span class="font-mono">{{ sinkConnectorName }}</span>) is still running, the delete
        will fail. Use "Stop Connector &amp; Delete Group" instead.
      </p>
    </AppConfirmDialog>

    <!-- Stop connector & delete confirm (Kafka Connect sink groups) -->
    <AppConfirmDialog
      :open="pendingStopDelete"
      title="Stop connector & delete group?"
      danger
      confirm-label="Stop & Delete"
      :loading="stopDeleting"
      @update:open="(v) => { if (!v && !stopDeleting) pendingStopDelete = false }"
      @confirm="stopAndDeleteGroup"
    >
      This will stop connector <strong class="font-mono">{{ sinkConnectorName }}</strong>,
      wait for its consumer to leave, then delete group
      <strong class="font-mono">{{ group }}</strong>.
      <p class="mt-2 text-xs text-muted-foreground">
        The connector stays stopped afterwards — resume or resync it when you're ready.
      </p>
      <p v-if="stopDeleting && stopDeleteStep" class="mt-3 flex items-center gap-2 text-xs text-brand-500">
        <RefreshCw class="h-3.5 w-3.5 animate-spin" :stroke-width="1.75" />
        {{ stopDeleteStep }}
      </p>
    </AppConfirmDialog>

    <!-- Delete offsets confirm -->
    <AppConfirmDialog
      :open="!!pendingDeleteOffsets"
      title="Delete topic offsets?"
      danger
      confirm-label="Delete Offsets"
      :loading="deletingOffsets"
      @update:open="(v) => { if (!v) pendingDeleteOffsets = null }"
      @confirm="confirmDeleteOffsets"
    >
      Delete committed offsets for topic <strong class="font-mono">{{ pendingDeleteOffsets }}</strong>
      from group <strong class="font-mono">{{ group }}</strong>?
      <p class="mt-2 text-xs text-muted-foreground">The broker requires the group to be inactive or the topic to be unassigned. This cannot be undone.</p>
    </AppConfirmDialog>

    <!-- Peek last message -->
    <AppPeekMessage :topic="peekTopic" @close="peekTopic = null" />
  </div>
</template>
