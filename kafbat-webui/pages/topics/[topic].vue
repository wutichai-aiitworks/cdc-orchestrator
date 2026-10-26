<script setup lang="ts">
import { Cable, ChevronRight, Copy, Layers, MessageSquare, MoreVertical, RefreshCw, Send, Settings, Trash2, X } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import { formatBytes } from "~/utils/bytes"

const route = useRoute()
const router = useRouter()
const toast = useToast()
const { fetchTopicMessages, fetchTopicMetadata, fetchGroups, fetchGroupLag, deleteTopic, recreateTopic, clearTopicMessages, copyTopic } = useKafkaM1()
const { fetchAcls, produceMessage, updateTopicConfigs } = useKafkaAdmin()
const { fetchConnectors } = useConnectorsM2()
const { getPref, setPref } = useViewPrefs()

const topic = computed(() => decodeURIComponent(String(route.params.topic || "")))
const tab = computed(() => {
  const value = String(route.query.tab || "overview")
  return ["overview", "messages", "consumers", "settings", "statistics", "connectors", "acls"].includes(value)
    ? (value as "overview" | "messages" | "consumers" | "settings" | "statistics" | "connectors" | "acls")
    : "overview"
})

// ── per-partition 3-dots menu ───────────────────────────────────────────────
const partMenuOpen = ref(false)
const partMenuPartition = ref<number | null>(null)
const partMenuStyle = ref({ top: "0px", left: "0px" })
const partMenuBtnRefs = ref<Map<number, HTMLElement>>(new Map())

const setPartMenuRef = (el: any, partition: number) => {
  if (el) partMenuBtnRefs.value.set(partition, el)
}

const openPartMenu = (partition: number) => {
  partMenuPartition.value = partition
  const btn = partMenuBtnRefs.value.get(partition)
  if (btn) {
    const rect = btn.getBoundingClientRect()
    partMenuStyle.value = {
      top: `${rect.bottom + 4}px`,
      left: `${Math.max(8, rect.right - 180)}px`,
    }
  }
  partMenuOpen.value = true
}

const closePartMenu = () => {
  partMenuOpen.value = false
  partMenuPartition.value = null
}

const clearPartitionMessages = () => {
  closePartMenu()
  openConfirm("clear")
}

// ── state ──────────────────────────────────────────────────────────────────
const loading = ref(false)
const metadataLoading = ref(false)
const consumersLoading = ref(false)
const actionLoading = ref(false)
const error = ref("")
const messagesError = ref("")
const consumersError = ref("")
const lastUpdated = ref("")

// Topic action menu (3-dots next to Produce Message)
const actionsMenuOpen = ref(false)
const actionsMenuBtnRef = ref<HTMLElement | null>(null)
const actionsMenuStyle = ref({ top: "0px", left: "0px" })
const confirm = ref<{ kind: "clear" | "recreate" | "remove"; title: string; body: string } | null>(null)

// Copy topic modal
const showCopyModal = ref(false)
const copyName = ref("")
const copyLoading = ref(false)

const toggleActionsMenu = () => {
  if (!actionsMenuOpen.value) {
    const rect = actionsMenuBtnRef.value?.getBoundingClientRect()
    if (rect) {
      actionsMenuStyle.value = {
        top: `${rect.bottom + 4}px`,
        left: `${Math.max(8, rect.right - 240)}px`,
      }
    }
  }
  actionsMenuOpen.value = !actionsMenuOpen.value
}

const closeActionsMenu = () => {
  actionsMenuOpen.value = false
}

const openConfirm = (kind: "clear" | "recreate" | "remove") => {
  closeActionsMenu()
  if (kind === "clear") {
    confirm.value = {
      kind,
      title: `Clear messages in ${topic.value}?`,
      body: "This will delete all messages from the topic. This action cannot be undone.",
    }
  } else if (kind === "recreate") {
    confirm.value = {
      kind,
      title: `Recreate topic ${topic.value}?`,
      body: "The topic will be deleted and immediately recreated with the same configuration. All messages will be lost.",
    }
  } else {
    confirm.value = {
      kind,
      title: `Remove topic ${topic.value}?`,
      body: "This will permanently delete the topic and all its messages.",
    }
  }
}

const runConfirmed = async () => {
  if (!confirm.value) return
  const { kind } = confirm.value
  confirm.value = null
  actionLoading.value = true
  try {
    if (kind === "clear") {
      await clearTopicMessages(topic.value)
      toast.success("Messages cleared")
    } else if (kind === "recreate") {
      await recreateTopic(topic.value)
      toast.success("Topic recreated")
      await loadMetadata()
    } else {
      await deleteTopic(topic.value)
      toast.success(`Topic ${topic.value} removed`)
      await router.replace("/topics")
    }
  } catch (e: any) {
    toast.error(formatApiError(e, "Action failed"))
  } finally {
    actionLoading.value = false
  }
}

const runCopy = async () => {
  const newName = copyName.value.trim()
  if (!newName || newName === topic.value) return
  copyLoading.value = true
  try {
    await copyTopic(topic.value, newName)
    toast.success("Topic copied successfully")
    showCopyModal.value = false
    copyName.value = ""
    await router.push(`/topics/${encodeURIComponent(newName)}`)
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to copy topic"))
  } finally {
    copyLoading.value = false
  }
}

const cleanupIsDelete = computed(() =>
  (metadata.value?.configs?.["cleanup.policy"] || "delete").toLowerCase() === "delete"
)

// ── per-message expanded state ─────────────────────────────────────────────
const expandedIdx = ref<number | null>(null)
const expandedInnerTab = ref<"key" | "value" | "headers">("value")

const toggleExpand = (idx: number) => {
  if (expandedIdx.value === idx) {
    expandedIdx.value = null
  } else {
    expandedIdx.value = idx
    expandedInnerTab.value = "value"
  }
}

// ── per-message 3-dots menu ────────────────────────────────────────────────
const msgMenuIdx = ref<number | null>(null)
const msgMenuStyle = ref({ top: "0px", left: "0px" })
const msgMenuBtnRefs = ref<Map<number, HTMLElement>>(new Map())

const setMsgMenuRef = (el: any, idx: number) => {
  if (el) msgMenuBtnRefs.value.set(idx, el as HTMLElement)
}

const openMsgMenu = (idx: number, e: MouseEvent) => {
  e.stopPropagation()
  msgMenuIdx.value = idx
  const btn = msgMenuBtnRefs.value.get(idx)
  if (btn) {
    const rect = btn.getBoundingClientRect()
    msgMenuStyle.value = {
      top: `${rect.bottom + 4}px`,
      left: `${Math.max(8, rect.right - 200)}px`,
    }
  }
}

const closeMsgMenu = () => { msgMenuIdx.value = null }

const copyMsgToClipboard = (idx: number) => {
  closeMsgMenu()
  const item = messages.value?.items?.[idx]
  if (!item) return
  navigator.clipboard?.writeText(item.payload).then(() => toast.success("Copied to clipboard"))
}

const saveMsgAsFile = (idx: number) => {
  closeMsgMenu()
  const item = messages.value?.items?.[idx]
  if (!item) return
  const blob = new Blob([item.payload], { type: "application/json" })
  const url = URL.createObjectURL(blob)
  const a = document.createElement("a")
  a.href = url
  a.download = `message-offset-${item.offset ?? idx}.json`
  a.click()
  URL.revokeObjectURL(url)
}

const reproduceMsg = (idx: number) => {
  closeMsgMenu()
  const item = messages.value?.items?.[idx]
  if (!item) return
  produceKey.value = item.key || ""
  produceValue.value = item.payload
  showProduce.value = true
}

// ── message helpers ────────────────────────────────────────────────────────
const isJson = (s: string | null | undefined): boolean => {
  if (!s) return false
  try { JSON.parse(s); return true } catch { return false }
}

const previewStr = (s: string | null | undefined, len = 80): string => {
  if (!s) return "null"
  const compact = s.replace(/\s+/g, " ").trim()
  return compact.length <= len ? compact : compact.slice(0, len) + "…"
}

const fmtTimestamp = (ts: string | null | undefined): string => {
  if (!ts) return "—"
  const n = Number(ts)
  if (!isNaN(n) && n > 1_000_000_000_000) return new Date(n).toLocaleString()
  return ts
}

const byteSize = (s: string | null | undefined): string => {
  if (!s) return "0 B"
  const b = new TextEncoder().encode(s).length
  if (b < 1024) return `${b} B`
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`
  return `${(b / 1024 / 1024).toFixed(1)} MB`
}

onMounted(() => {
  window.addEventListener("scroll", closeActionsMenu, true)
  window.addEventListener("scroll", closePartMenu, true)
  window.addEventListener("scroll", closeMsgMenu, true)
})
onUnmounted(() => {
  window.removeEventListener("scroll", closeActionsMenu, true)
  window.removeEventListener("scroll", closePartMenu, true)
  window.removeEventListener("scroll", closeMsgMenu, true)
})

// Messages
const limit = ref(20)
const limitPreset = ref<"20" | "50" | "100" | "200" | "custom">("20")
const limitCustomStr = ref("500")
const onLimitPreset = (val: string) => {
  limitPreset.value = val as typeof limitPreset.value
  if (val !== "custom") {
    limit.value = Number(val)
    msgPage.value = 1
  }
}
const applyCustomLimit = () => {
  const n = Math.max(1, Math.min(10000, Math.round(Number(limitCustomStr.value) || 100)))
  limitCustomStr.value = String(n)
  limit.value = n
  msgPage.value = 1
}

const offset = ref("end")
const mode = ref<"latest" | "earliest">("latest")
const partitionText = ref("")
const includeMeta = ref(true)
const filterText = ref("")
const messages = ref<{
  items: Array<{ partition: number | null; offset: number | null; timestamp: string | null; payload: string; key?: string | null }>
  total: number
  limit: number
  offset: string
} | null>(null)

// Pagination for messages list (client-side over fetched items)
const msgPage = ref(1)
const msgPageSize = 20
const totalMsgPages = computed(() => Math.max(1, Math.ceil((messages.value?.items?.length ?? 0) / msgPageSize)))
const pagedMsgs = computed(() => {
  const items = messages.value?.items ?? []
  const start = (msgPage.value - 1) * msgPageSize
  return items.slice(start, start + msgPageSize)
})

// Produce
const showProduce = ref(false)
const produceKey = ref("")
const produceValue = ref("")
const producePartition = ref("")
const keepContents = ref(false)
const producing = ref(false)

// Metadata / Overview / Settings
const metadata = ref<{
  partition_count: number
  urp: number
  partitions: Array<{
    partition: number
    leader: number | null
    replicas: number[]
    in_sync_replicas: number[]
    log_start_offset?: number | null
    high_watermark: number | null
    message_count?: number | null
  }>
  configs: Record<string, string>
} | null>(null)
const configDraft = ref<Array<{ name: string; value: string }>>([])
const configSaving = ref(false)

// Consumers
const groupRows = ref<Array<{ group: string; lag: number }>>([])

// Connectors
const connectorRows = ref<Array<{ name: string; state: string }>>([])

// ACLs
const aclRows = ref<Array<Record<string, string>>>([])

// ── loaders ────────────────────────────────────────────────────────────────
const loadMetadata = async () => {
  metadataLoading.value = true
  try {
    metadata.value = await fetchTopicMetadata(topic.value)
    configDraft.value = Object.entries(metadata.value.configs || {}).map(([name, value]) => ({ name, value }))
  } catch {
    metadata.value = null
  } finally {
    metadataLoading.value = false
  }
}

const loadMessages = async () => {
  loading.value = true
  messagesError.value = ""
  try {
    const partition = partitionText.value.trim()
    messages.value = await fetchTopicMessages(topic.value, {
      limit: limit.value,
      offset: offset.value,
      mode: mode.value,
      partition: partition === "" ? undefined : Number(partition),
      include_meta: includeMeta.value,
      q: filterText.value.trim() || undefined,
    })
    lastUpdated.value = new Date().toLocaleTimeString()
    msgPage.value = 1
  } catch (e: any) {
    messagesError.value = formatApiError(e, "Failed to fetch messages")
  } finally {
    loading.value = false
  }
}

const loadConsumers = async () => {
  consumersLoading.value = true
  consumersError.value = ""
  try {
    const groups = await fetchGroups({ page: 1, page_size: 100 })
    // Deduplicate by group name (rpk may return the same group multiple times)
    const uniqueGroups = [...new Map((groups.items || []).map((g) => [g.group, g])).values()]
    const rows: Array<{ group: string; lag: number }> = []
    for (const item of uniqueGroups) {
      try {
        const lag = await fetchGroupLag(item.group)
        const match = (lag.entries || []).filter((entry) => entry.topic === topic.value)
        if (match.length) rows.push({ group: item.group, lag: match.reduce((sum, e) => sum + e.lag, 0) })
      } catch { /* ignore */ }
    }
    groupRows.value = rows
  } catch (e: any) {
    consumersError.value = formatApiError(e, "Failed to load consumer groups")
  } finally {
    consumersLoading.value = false
  }
}

const loadConnectors = async () => {
  try {
    const data = await fetchConnectors()
    connectorRows.value = Object.entries(data.connectors || {})
      .filter(([, value]: [string, any]) => {
        const topics = String(value?.config?.topics || value?.config?.["topics.regex"] || "")
        return topics.includes(topic.value)
      })
      .map(([name, value]: [string, any]) => ({
        name,
        state: String(value?.status?.connector?.state || "UNKNOWN"),
      }))
  } catch { /* ignore */ }
}

const loadAcls = async () => {
  try {
    const data = await fetchAcls({ resource: topic.value })
    aclRows.value = (data.items || []) as unknown as Array<Record<string, string>>
  } catch { /* ignore */ }
}

const loadTabData = async (tabName: string) => {
  if (tabName === "messages") await loadMessages()
  else if (tabName === "consumers") await loadConsumers()
  else if (tabName === "connectors") await loadConnectors()
  else if (tabName === "acls") await loadAcls()
}

const reload = async () => {
  await loadMetadata()
  await loadTabData(tab.value)
}

// ── produce ────────────────────────────────────────────────────────────────
const sendMessage = async () => {
  if (!produceValue.value) return
  producing.value = true
  try {
    await produceMessage(topic.value, {
      value: produceValue.value,
      key: produceKey.value || undefined,
      partition: producePartition.value === "" ? undefined : Number(producePartition.value),
    })
    toast.success("Message produced")
    if (!keepContents.value) {
      produceKey.value = ""
      produceValue.value = ""
    }
    await loadMessages()
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to produce message"))
  } finally {
    producing.value = false
  }
}

// ── settings ───────────────────────────────────────────────────────────────
const saveConfigs = async () => {
  configSaving.value = true
  try {
    const configs = Object.fromEntries(
      configDraft.value.filter((row) => row.name.trim()).map((row) => [row.name.trim(), row.value])
    )
    await updateTopicConfigs(topic.value, configs)
    toast.success("Topic configs updated")
    await loadMetadata()
  } catch (e: any) {
    toast.error(formatApiError(e, "Failed to update configs"))
  } finally {
    configSaving.value = false
  }
}

// ── lifecycle ──────────────────────────────────────────────────────────────
onMounted(async () => {
  const q = route.query
  limit.value = Number(q.limit || 20) || 20
  if ([20, 50, 100, 200].includes(limit.value)) {
    limitPreset.value = String(limit.value) as typeof limitPreset.value
  } else {
    limitPreset.value = "custom"
    limitCustomStr.value = String(limit.value)
  }
  offset.value = String(q.offset || "end")
  mode.value = String(q.mode || "latest") === "earliest" ? "earliest" : "latest"
  includeMeta.value = q.include_meta != null ? String(q.include_meta) !== "false" : Boolean(getPref("topic.includeMeta", true))
  partitionText.value = String(q.partition || "")
  filterText.value = String(q.q || "")
  await loadMetadata()
  await loadTabData(tab.value)
})

watch(includeMeta, (v) => setPref("topic.includeMeta", v))
watch(tab, (t) => loadTabData(t))

// ── helpers ────────────────────────────────────────────────────────────────

const isInSyncReplica = (partition: { leader: number | null; replicas: number[]; in_sync_replicas?: number[] }, replicaId: number) =>
  (partition.in_sync_replicas ?? [replicaId].filter((r) => r === partition.leader)).includes(replicaId)

// Computed ISR totals for the "In Sync Replicas: X of Y" display
const totalReplicas = computed(() => metadata.value?.partitions?.reduce((s, p) => s + p.replicas.length, 0) ?? 0)
const totalIsr = computed(() => metadata.value?.partitions?.reduce((s, p) => s + (p.in_sync_replicas?.length ?? p.replicas.length), 0) ?? 0)

// Statistics: pick meaningful config keys
const statsKeys = [
  "cleanup.policy",
  "retention.ms",
  "retention.bytes",
  "segment.bytes",
  "max.message.bytes",
  "min.insync.replicas",
  "compression.type",
  "message.timestamp.type",
]
const statsConfigs = computed(() =>
  statsKeys
    .map((k) => ({ name: k, value: metadata.value?.configs?.[k] ?? "—" }))
    .filter((row) => row.value !== "—" || statsKeys.includes(row.name))
)
</script>

<template>
  <div class="page-container max-w-7xl space-y-4">
    <!-- Breadcrumb -->
    <nav class="flex items-center gap-2 text-sm text-muted-foreground">
      <NuxtLink to="/topics" class="hover:text-brand-500">Topics</NuxtLink>
      <ChevronRight class="h-3.5 w-3.5" :stroke-width="1.75" />
      <span class="truncate font-semibold text-foreground">{{ topic }}</span>
    </nav>

    <!-- Page header -->
    <AppPageHeader :title="topic" description="Overview, messages, consumers, settings, statistics, connectors, and ACLs.">
      <AppButton :disabled="loading || metadataLoading || actionLoading" @click="reload">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading || metadataLoading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
      <AppButton variant="primary" :disabled="actionLoading" @click="showProduce = true">
        <Send class="h-4 w-4" :stroke-width="1.75" />
        Produce Message
      </AppButton>
      <!-- 3-dots topic actions -->
      <button
        ref="actionsMenuBtnRef"
        type="button"
        class="inline-flex h-10 w-10 items-center justify-center rounded-xl border border-border bg-card text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :disabled="actionLoading"
        title="Topic actions"
        @click="toggleActionsMenu"
      >
        <MoreVertical class="h-4 w-4" :stroke-width="1.75" />
      </button>
    </AppPageHeader>

    <!-- Topic actions dropdown (Teleport to escape overflow) -->
    <Teleport to="body">
      <div
        v-if="actionsMenuOpen"
        class="fixed z-[9999] w-60 rounded-xl border border-border bg-popover p-1.5 shadow-2xl"
        :style="actionsMenuStyle"
        @click.stop
      >
        <div class="fixed inset-0 -z-10" @click="closeActionsMenu" />

        <!-- Edit settings -->
        <NuxtLink
          :to="`/topics/${encodeURIComponent(topic)}?tab=settings`"
          class="block rounded-lg px-3 py-2.5 text-sm hover:bg-accent"
          @click="closeActionsMenu"
        >
          <div class="font-medium text-foreground">Edit settings</div>
          <div class="mt-0.5 text-xs text-muted-foreground">
            Pay attention! This operation has especially important consequences.
          </div>
        </NuxtLink>

        <!-- Copy topic -->
        <button
          type="button"
          class="flex w-full items-center gap-2.5 rounded-lg px-3 py-2.5 text-left text-sm font-medium hover:bg-accent"
          @click="closeActionsMenu(); showCopyModal = true; copyName = ''"
        >
          <Copy class="h-4 w-4 flex-shrink-0 text-muted-foreground" :stroke-width="1.75" />
          <div>
            <div class="text-foreground">Copy topic</div>
            <div class="mt-0.5 text-xs text-muted-foreground">Duplicate with a new name</div>
          </div>
        </button>

        <div class="my-1 border-t border-border/60" />

        <!-- Clear messages -->
        <button
          type="button"
          class="flex w-full flex-col items-start rounded-lg px-3 py-2.5 text-left disabled:cursor-not-allowed disabled:opacity-40"
          :class="cleanupIsDelete ? 'text-red-500 hover:bg-red-500/10' : 'text-muted-foreground'"
          :disabled="!cleanupIsDelete"
          @click="openConfirm('clear')"
        >
          <span class="text-sm font-medium">Clear messages</span>
          <span class="mt-0.5 text-xs" :class="cleanupIsDelete ? 'text-red-400/70' : 'text-muted-foreground'">
            Clearing messages is only allowed for topics with DELETE policy
          </span>
        </button>

        <!-- Recreate topic -->
        <button
          type="button"
          class="w-full rounded-lg px-3 py-2.5 text-left text-sm font-medium text-red-500 hover:bg-red-500/10"
          @click="openConfirm('recreate')"
        >
          Recreate Topic
        </button>

        <!-- Remove topic -->
        <button
          type="button"
          class="w-full rounded-lg px-3 py-2.5 text-left text-sm font-medium text-red-500 hover:bg-red-500/10"
          @click="openConfirm('remove')"
        >
          Remove Topic
        </button>
      </div>
    </Teleport>

    <!-- Confirm dialog -->
    <AppConfirmDialog
      :open="Boolean(confirm)"
      :title="confirm?.title || ''"
      :danger="true"
      confirm-label="Confirm"
      @update:open="(v) => { if (!v) confirm = null }"
      @confirm="runConfirmed"
    >
      {{ confirm?.body }}
    </AppConfirmDialog>

    <!-- Metric cards -->
    <section class="grid grid-cols-2 gap-3 sm:grid-cols-4 lg:grid-cols-7">
      <AppMetricCard label="Partitions" :value="metadata ? String(metadata.partition_count) : '—'">
        <Layers class="h-4 w-4" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="Replication Factor"
        :value="metadata?.partitions?.[0]?.replicas?.length != null ? String(metadata.partitions[0].replicas.length) : '—'"
      >
        <Cable class="h-4 w-4" :stroke-width="1.75" />
      </AppMetricCard>
      <!-- URP -->
      <AppMetricCard
        label="URP"
        :value="metadata ? String(metadata.urp ?? 0) : '—'"
        :tone="(metadata?.urp ?? 0) > 0 ? 'error' : 'success'"
      >
        <span
          class="h-2 w-2 rounded-full"
          :class="(metadata?.urp ?? 0) > 0 ? 'bg-red-500' : 'bg-emerald-500'"
        />
      </AppMetricCard>
      <!-- In Sync Replicas -->
      <AppMetricCard
        label="In Sync Replicas"
        :value="metadata ? `${totalIsr} of ${totalReplicas}` : '—'"
        :tone="totalIsr < totalReplicas ? 'error' : 'success'"
      >
        <span
          class="h-2 w-2 rounded-full"
          :class="totalIsr < totalReplicas ? 'bg-red-500' : 'bg-emerald-500'"
        />
      </AppMetricCard>
      <!-- Cleanup Policy -->
      <AppMetricCard label="Clean Up Policy" :value="metadata?.configs?.['cleanup.policy'] || '—'">
        <Trash2 class="h-4 w-4" :stroke-width="1.75" />
      </AppMetricCard>
      <!-- Message Count -->
      <AppMetricCard
        label="Message Count"
        :value="metadata?.partitions?.length ? String(metadata.partitions.reduce((s, p) => s + (p.message_count ?? 0), 0)) : '—'"
        tone="brand"
      >
        <MessageSquare class="h-4 w-4" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard label="Config keys" :value="String(Object.keys(metadata?.configs || {}).length || '—')" tone="info">
        <Settings class="h-4 w-4" :stroke-width="1.75" />
      </AppMetricCard>
    </section>

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <!-- Tabs -->
    <TopicTabs :topic="topic" :active="tab" />

    <!-- ── OVERVIEW ──────────────────────────────────────────────────────── -->
    <template v-if="tab === 'overview'">
      <AppCard>
        <div class="mb-4 flex items-center justify-between">
          <h2 class="section-title">Partitions</h2>
          <span v-if="metadataLoading" class="text-xs text-muted-foreground">Loading…</span>
        </div>
        <AppTable>
          <thead>
            <tr>
              <th>Partition ID</th>
              <th>Replicas</th>
              <th>In-Sync Replicas</th>
              <th>First Offset</th>
              <th>Next Offset</th>
              <th>Message Count</th>
              <th class="w-10" />
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in metadata?.partitions || []" :key="row.partition">
              <td class="font-mono">{{ row.partition }}</td>
              <td>
                <span
                  v-for="r in row.replicas"
                  :key="r"
                  class="mr-1 inline-flex h-5 min-w-[1.25rem] items-center justify-center rounded px-1 text-xs font-mono"
                  :class="isInSyncReplica(row, r) ? 'bg-emerald-500/20 text-emerald-400' : 'bg-red-500/15 text-red-400'"
                  :title="r === row.leader ? 'Leader' : isInSyncReplica(row, r) ? 'In-sync replica' : 'Out-of-sync replica'"
                >{{ r }}</span>
              </td>
              <td class="font-mono text-sm">
                <span :class="(row.in_sync_replicas?.length ?? row.replicas.length) < row.replicas.length ? 'text-red-400' : 'text-emerald-400'">
                  {{ row.in_sync_replicas?.length ?? row.replicas.length }} of {{ row.replicas.length }}
                </span>
              </td>
              <td class="font-mono">{{ row.log_start_offset ?? "—" }}</td>
              <td class="font-mono">{{ row.high_watermark ?? "—" }}</td>
              <td class="font-mono">{{ row.message_count ?? "—" }}</td>
              <!-- per-partition 3-dots -->
              <td class="w-10 text-right">
                <button
                  :ref="(el) => setPartMenuRef(el, row.partition)"
                  type="button"
                  class="inline-flex h-7 w-7 items-center justify-center rounded-lg text-muted-foreground hover:bg-accent hover:text-foreground"
                  title="Partition actions"
                  @click.stop="openPartMenu(row.partition)"
                >
                  <MoreVertical class="h-3.5 w-3.5" :stroke-width="1.75" />
                </button>
              </td>
            </tr>
            <tr v-if="!metadataLoading && !metadata?.partitions?.length">
              <td colspan="7">
                <AppStateBlock kind="empty" :message="'No partition data available.'" />
              </td>
            </tr>
          </tbody>
        </AppTable>
      </AppCard>

      <!-- Per-partition actions menu -->
      <Teleport to="body">
        <div
          v-if="partMenuOpen"
          class="fixed z-[9999] min-w-[180px] rounded-xl border border-border bg-popover p-1 shadow-2xl"
          :style="partMenuStyle"
          @click.stop
        >
          <div class="fixed inset-0 -z-10" @click="closePartMenu" />
          <div class="px-3 py-1.5 text-xs font-medium text-muted-foreground">
            Partition {{ partMenuPartition }}
          </div>
          <div class="my-0.5 border-t border-border/50" />
          <button
            type="button"
            class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm font-medium disabled:cursor-not-allowed disabled:opacity-40"
            :class="cleanupIsDelete ? 'text-red-500 hover:bg-red-500/10' : 'text-muted-foreground'"
            :disabled="!cleanupIsDelete"
            :title="!cleanupIsDelete ? 'Only available for topics with DELETE cleanup policy' : undefined"
            @click="clearPartitionMessages"
          >
            <Trash2 class="h-3.5 w-3.5 flex-shrink-0" :stroke-width="1.75" />
            Clear Messages
          </button>
        </div>
      </Teleport>
    </template>

    <!-- ── MESSAGES ──────────────────────────────────────────────────────── -->
    <template v-if="tab === 'messages'">
      <!-- Toolbar -->
      <div class="flex flex-wrap items-center gap-2">
        <!-- Newest/Oldest -->
        <AppSelect v-model="offset" class="w-32">
          <option value="end">Newest</option>
          <option value="start">Oldest</option>
        </AppSelect>
        <!-- Partition selector -->
        <AppSelect v-model="partitionText" class="w-44">
          <option value="">All Partitions</option>
          <option v-for="p in metadata?.partitions || []" :key="p.partition" :value="String(p.partition)">
            Partition {{ p.partition }}
          </option>
        </AppSelect>
        <!-- Limit: preset + custom -->
        <div class="flex items-center gap-1.5">
          <AppSelect :model-value="limitPreset" class="w-32" @update:model-value="onLimitPreset">
            <option value="20">20 rows</option>
            <option value="50">50 rows</option>
            <option value="100">100 rows</option>
            <option value="200">200 rows</option>
            <option value="custom">Custom…</option>
          </AppSelect>
          <AppInput
            v-if="limitPreset === 'custom'"
            v-model="limitCustomStr"
            type="number"
            min="1"
            max="10000"
            placeholder="e.g. 500"
            class="w-24"
            @keyup.enter="applyCustomLimit"
            @blur="applyCustomLimit"
          />
        </div>
        <!-- Refresh -->
        <AppButton :disabled="loading" @click="loadMessages">
          <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
          Refresh
        </AppButton>
        <!-- Search -->
        <div class="ml-auto">
          <AppInput v-model="filterText" placeholder="Search key or value…" class="w-64" @keyup.enter="loadMessages" />
        </div>
      </div>

      <!-- Stats bar -->
      <div v-if="messages" class="flex items-center gap-4 text-xs text-muted-foreground">
        <span class="flex items-center gap-1.5">
          <span class="h-1.5 w-1.5 rounded-full bg-emerald-500" />
          {{ messages.total }} message{{ messages.total !== 1 ? "s" : "" }} consumed
        </span>
        <span v-if="lastUpdated">Updated {{ lastUpdated }}</span>
      </div>

      <AppStateBlock v-if="messagesError" kind="error" :message="messagesError" />

      <!-- Messages table -->
      <AppCard class="overflow-hidden !p-0">
        <!-- Table header -->
        <div class="grid grid-cols-[2rem_minmax(9.5rem,max-content)_4.5rem_12rem_minmax(0,1fr)_minmax(0,1fr)_2.5rem] gap-0 border-b border-border bg-surface-900/60 px-3 py-2.5 text-xs font-medium text-muted-foreground">
          <div />
          <div>Offset</div>
          <div>Partition</div>
          <div>Timestamp</div>
          <div>Key Preview</div>
          <div>Value Preview</div>
          <div />
        </div>

        <!-- Loading -->
        <div v-if="loading" class="flex flex-col items-center gap-2 py-12 text-sm text-muted-foreground">
          <RefreshCw class="h-5 w-5 animate-spin opacity-50" :stroke-width="1.5" />
          Loading messages…
        </div>

        <!-- Empty states -->
        <div v-else-if="messagesError" class="py-8 text-center text-sm text-muted-foreground">
          Could not load messages — see error above
        </div>
        <div v-else-if="messages === null" class="py-8 text-center text-sm text-muted-foreground">
          Click <strong>Refresh</strong> to load messages
        </div>
        <div v-else-if="!messages?.items?.length" class="py-8 text-center text-sm text-muted-foreground">
          No messages found. Try switching to <strong>Oldest</strong> or clearing the filter.
        </div>

        <!-- Rows -->
        <template v-for="(item, pageIdx) in pagedMsgs" :key="(msgPage - 1) * msgPageSize + pageIdx">
          <!-- real index into full messages array -->
          <!-- Summary row -->
          <div
            class="group grid cursor-pointer grid-cols-[2rem_minmax(9.5rem,max-content)_4.5rem_12rem_minmax(0,1fr)_minmax(0,1fr)_2.5rem] gap-0 border-b border-border/50 px-3 py-2.5 text-sm transition-colors hover:bg-accent/40"
            :class="{ 'bg-brand-500/5 border-brand-500/20': expandedIdx === (msgPage - 1) * msgPageSize + pageIdx }"
            @click="toggleExpand((msgPage - 1) * msgPageSize + pageIdx)"
          >
            <!-- Expand indicator -->
            <div class="flex items-center">
              <span
                class="inline-flex h-5 w-5 flex-shrink-0 items-center justify-center rounded border border-border text-xs text-muted-foreground transition-colors"
                :class="expandedIdx === (msgPage - 1) * msgPageSize + pageIdx ? 'border-brand-500 bg-brand-500/10 text-brand-400' : ''"
              >{{ expandedIdx === (msgPage - 1) * msgPageSize + pageIdx ? "−" : "+" }}</span>
            </div>
            <!-- Offset -->
            <div class="flex min-w-0 items-center overflow-hidden pr-2">
              <span
                class="badge-neutral max-w-full !gap-0 !px-2.5 !py-0.5 font-mono !text-[11px] tabular-nums"
                :title="item.offset == null ? undefined : String(item.offset)"
              >{{ item.offset ?? "—" }}</span>
            </div>
            <!-- Partition -->
            <div class="flex items-center">
              <span class="badge-neutral !gap-0 !px-2 !py-0.5 font-mono !text-[11px] tabular-nums">{{ item.partition ?? "—" }}</span>
            </div>
            <!-- Timestamp -->
            <div class="flex items-center text-xs text-muted-foreground">{{ fmtTimestamp(item.timestamp) }}</div>
            <!-- Key preview -->
            <div class="flex items-center overflow-hidden pr-2">
              <span class="truncate font-mono text-xs text-surface-300">{{ previewStr(item.key) }}</span>
            </div>
            <!-- Value preview -->
            <div class="flex items-center overflow-hidden pr-2">
              <span class="truncate font-mono text-xs text-surface-300">{{ previewStr(item.payload) }}</span>
            </div>
            <!-- 3-dots -->
            <div class="flex items-center justify-end">
              <button
                :ref="(el) => setMsgMenuRef(el, (msgPage - 1) * msgPageSize + pageIdx)"
                type="button"
                class="invisible inline-flex h-7 w-7 items-center justify-center rounded-lg text-muted-foreground hover:bg-accent hover:text-foreground group-hover:visible"
                :class="{ '!visible': msgMenuIdx === (msgPage - 1) * msgPageSize + pageIdx }"
                @click.stop="openMsgMenu((msgPage - 1) * msgPageSize + pageIdx, $event)"
              >
                <MoreVertical class="h-3.5 w-3.5" :stroke-width="1.75" />
              </button>
            </div>
          </div>

          <!-- Expanded detail -->
          <div v-if="expandedIdx === (msgPage - 1) * msgPageSize + pageIdx" class="border-b border-border bg-surface-900/40">
            <div class="grid grid-cols-[1fr_280px] gap-0">
              <!-- Left: tabs + content -->
              <div class="border-r border-border">
                <!-- Tab strip -->
                <div class="flex border-b border-border">
                  <button
                    v-for="t in ['key', 'value', 'headers'] as const"
                    :key="t"
                    type="button"
                    class="px-4 py-2 text-xs font-medium capitalize transition-colors"
                    :class="expandedInnerTab === t
                      ? 'border-b-2 border-brand-500 text-brand-400'
                      : 'text-muted-foreground hover:text-foreground'"
                    @click.stop="expandedInnerTab = t"
                  >{{ t }}</button>
                </div>
                <!-- Content -->
                <div class="h-64 overflow-hidden">
                  <template v-if="expandedInnerTab === 'value'">
                    <AppJsonViewer v-if="isJson(item.payload)" :content="item.payload" height="100%" />
                    <div v-else class="h-full overflow-auto p-3">
                      <pre class="font-mono text-xs text-foreground">{{ item.payload }}</pre>
                    </div>
                  </template>
                  <template v-else-if="expandedInnerTab === 'key'">
                    <AppJsonViewer v-if="isJson(item.key)" :content="item.key" height="100%" />
                    <div v-else class="h-full overflow-auto p-3">
                      <pre class="font-mono text-xs text-foreground">{{ item.key || "(no key)" }}</pre>
                    </div>
                  </template>
                  <template v-else>
                    <div class="p-3">
                      <p class="text-xs text-muted-foreground">No headers available.</p>
                    </div>
                  </template>
                </div>
              </div>
              <!-- Right: metadata -->
              <div class="space-y-4 p-4 text-xs">
                <div>
                  <div class="mb-1 text-xs font-medium text-muted-foreground">Timestamp</div>
                  <div class="font-mono text-foreground">{{ fmtTimestamp(item.timestamp) }}</div>
                  <div class="text-muted-foreground/60">Timestamp type: CREATE_TIME</div>
                </div>
                <div>
                  <div class="mb-1 text-xs font-medium text-muted-foreground">Key Serde</div>
                  <div class="font-medium text-foreground">String</div>
                  <div class="text-muted-foreground">Size: {{ byteSize(item.key) }}</div>
                </div>
                <div>
                  <div class="mb-1 text-xs font-medium text-muted-foreground">Value Serde</div>
                  <div class="font-medium text-foreground">String</div>
                  <div class="text-muted-foreground">Size: {{ byteSize(item.payload) }}</div>
                </div>
              </div>
            </div>
          </div>
        </template>
      </AppCard>

      <!-- Pagination -->
      <div v-if="messages && messages.items.length > msgPageSize" class="flex items-center justify-between gap-4 pt-1">
        <span class="text-xs text-muted-foreground">
          Showing {{ (msgPage - 1) * msgPageSize + 1 }}–{{ Math.min(msgPage * msgPageSize, messages.items.length) }} of {{ messages.items.length }} messages
        </span>
        <AppPager
          :page="msgPage"
          :total-pages="totalMsgPages"
          :disabled="loading"
          @prev="msgPage--; expandedIdx = null"
          @next="msgPage++; expandedIdx = null"
        />
      </div>

      <!-- Per-message 3-dots menu -->
      <Teleport to="body">
        <div
          v-if="msgMenuIdx !== null"
          class="fixed z-[9999] min-w-[200px] rounded-xl border border-border bg-popover p-1 shadow-2xl"
          :style="msgMenuStyle"
          @click.stop
        >
          <div class="fixed inset-0 -z-10" @click="closeMsgMenu" />
          <button
            type="button"
            class="flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-sm hover:bg-accent"
            @click="copyMsgToClipboard(msgMenuIdx!)"
          >
            Copy to clipboard
          </button>
          <button
            type="button"
            class="flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-sm hover:bg-accent"
            @click="saveMsgAsFile(msgMenuIdx!)"
          >
            Save as a file
          </button>
          <div class="my-0.5 border-t border-border/50" />
          <button
            type="button"
            class="flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-sm hover:bg-accent"
            @click="reproduceMsg(msgMenuIdx!)"
          >
            Reproduce message
          </button>
        </div>
      </Teleport>
    </template>

    <!-- ── CONSUMERS ─────────────────────────────────────────────────────── -->
    <template v-if="tab === 'consumers'">
      <AppStateBlock v-if="consumersError" kind="error" :message="consumersError" />
      <AppCard>
        <div class="mb-4 flex items-center justify-between">
          <h2 class="section-title">Consumer Groups</h2>
          <span v-if="consumersLoading" class="flex items-center gap-1.5 text-xs text-muted-foreground">
            <RefreshCw class="h-3.5 w-3.5 animate-spin" :stroke-width="1.5" />
            Loading groups…
          </span>
        </div>
        <AppTable>
          <thead>
            <tr>
              <th>Consumer Group</th>
              <th>Total Lag</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="consumersLoading">
              <td colspan="2" class="py-8 text-center text-sm text-muted-foreground">
                <RefreshCw class="mx-auto mb-2 h-5 w-5 animate-spin opacity-50" :stroke-width="1.5" />
                Scanning consumer groups…
              </td>
            </tr>
            <tr v-else-if="!groupRows.length">
              <td colspan="2"><AppStateBlock kind="empty" message="No consumer groups on this topic." /></td>
            </tr>
            <tr v-for="row in groupRows" :key="row.group">
              <td>
                <NuxtLink :to="`/consumer-groups/${encodeURIComponent(row.group)}`" class="hover:text-brand-500">
                  {{ row.group }}
                </NuxtLink>
              </td>
              <td>
                <span :class="row.lag > 0 ? 'text-yellow-400' : 'text-emerald-400'">{{ row.lag }}</span>
              </td>
            </tr>
          </tbody>
        </AppTable>
      </AppCard>
    </template>

    <!-- ── SETTINGS ──────────────────────────────────────────────────────── -->
    <AppCard v-if="tab === 'settings'" class="space-y-4">
      <div class="flex items-center justify-between">
        <h2 class="section-title">Topic Configs</h2>
        <span v-if="metadataLoading" class="text-xs text-muted-foreground">Loading…</span>
      </div>
      <AppTable table-class="table-fixed">
        <thead>
          <tr>
            <th class="w-[45%]">Key</th>
            <th>Value</th>
            <th class="w-24">Action</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(row, idx) in configDraft" :key="idx">
            <td class="font-mono text-sm"><AppInput v-model="row.name" placeholder="Config key" /></td>
            <td class="font-mono text-sm"><AppInput v-model="row.value" placeholder="Value" /></td>
            <td>
              <AppButton size="sm" variant="ghost" @click="configDraft.splice(idx, 1)">Remove</AppButton>
            </td>
          </tr>
          <tr v-if="!configDraft.length">
            <td colspan="3"><AppStateBlock kind="empty" message="No configs loaded." /></td>
          </tr>
        </tbody>
      </AppTable>
      <div class="flex gap-2 pt-1">
        <AppButton @click="configDraft.push({ name: '', value: '' })">+ Add config</AppButton>
        <AppButton variant="primary" :disabled="configSaving" @click="saveConfigs">
          {{ configSaving ? "Saving…" : "Save" }}
        </AppButton>
      </div>
    </AppCard>

    <!-- ── STATISTICS ────────────────────────────────────────────────────── -->
    <template v-if="tab === 'statistics'">
      <section class="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <AppMetricCard label="Partitions" :value="metadata ? String(metadata.partition_count) : '—'" />
        <AppMetricCard
          label="Replication Factor"
          :value="metadata?.partitions?.[0]?.replicas?.length != null ? String(metadata.partitions[0].replicas.length) : '—'"
        />
        <AppMetricCard label="Cleanup Policy" :value="metadata?.configs?.['cleanup.policy'] || '—'" />
        <AppMetricCard
          label="Total Messages"
          :value="metadata?.partitions?.length ? String(metadata.partitions.reduce((s, p) => s + (p.message_count ?? 0), 0)) : '—'"
        />
      </section>
      <AppCard>
        <h2 class="section-title mb-4">Config summary</h2>
        <AppTable table-class="table-fixed">
          <thead>
            <tr>
              <th class="w-[45%]">Key</th>
              <th>Value</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in statsConfigs" :key="row.name">
              <td class="font-mono text-sm">{{ row.name }}</td>
              <td class="font-mono text-sm">{{ row.value }}</td>
            </tr>
          </tbody>
        </AppTable>
      </AppCard>
      <AppCard>
        <h2 class="section-title mb-4">Partition details</h2>
        <AppTable>
          <thead>
            <tr>
              <th>Partition ID</th>
              <th>Leader</th>
              <th>Replicas</th>
              <th>First Offset</th>
              <th>Next Offset</th>
              <th>Message Count</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in metadata?.partitions || []" :key="row.partition">
              <td class="font-mono">{{ row.partition }}</td>
              <td class="font-mono">{{ row.leader ?? "—" }}</td>
              <td class="font-mono">{{ row.replicas.join(", ") || "—" }}</td>
              <td class="font-mono">{{ row.log_start_offset ?? "—" }}</td>
              <td class="font-mono">{{ row.high_watermark ?? "—" }}</td>
              <td class="font-mono">{{ row.message_count ?? "—" }}</td>
            </tr>
          </tbody>
        </AppTable>
      </AppCard>
    </template>

    <!-- ── CONNECTORS ────────────────────────────────────────────────────── -->
    <AppCard v-if="tab === 'connectors'">
      <h2 class="section-title mb-4">Connectors</h2>
      <AppTable>
        <thead>
          <tr>
            <th>Connector</th>
            <th>State</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="row in connectorRows" :key="row.name">
            <td>
              <NuxtLink to="/connectors" class="hover:text-brand-500">{{ row.name }}</NuxtLink>
            </td>
            <td>
              <AppStatusBadge :status="row.state" />
            </td>
          </tr>
          <tr v-if="!connectorRows.length">
            <td colspan="2"><AppStateBlock kind="empty" message="No connectors reference this topic." /></td>
          </tr>
        </tbody>
      </AppTable>
    </AppCard>

    <!-- ── ACL ───────────────────────────────────────────────────────────── -->
    <AppCard v-if="tab === 'acls'">
      <h2 class="section-title mb-4">Access Control List</h2>
      <AppTable>
        <thead>
          <tr>
            <th>Principal</th>
            <th>Host</th>
            <th>Operation</th>
            <th>Permission</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(row, idx) in aclRows" :key="idx">
            <td class="font-mono text-sm">{{ row.principal }}</td>
            <td class="font-mono text-sm">{{ row.host }}</td>
            <td>{{ row.operation }}</td>
            <td>
              <span
                :class="row.permission?.toUpperCase() === 'ALLOW' ? 'badge-success' : 'badge-error'"
                class="inline-block rounded-md px-2 py-0.5 text-xs font-medium"
              >
                {{ row.permission }}
              </span>
            </td>
          </tr>
          <tr v-if="!aclRows.length">
            <td colspan="4"><AppStateBlock kind="empty" message="No ACL rules for this topic." /></td>
          </tr>
        </tbody>
      </AppTable>
    </AppCard>

    <!-- ── PRODUCE MESSAGE MODAL ─────────────────────────────────────────── -->
    <div v-if="showProduce" class="modal-overlay" @click.self="showProduce = false">      <div class="w-full max-w-lg rounded-xl border border-border bg-card p-5 shadow-2xl">
        <div class="mb-4 flex items-center justify-between">
          <h2 class="text-base font-semibold">Produce Message to <span class="font-mono text-brand-400">{{ topic }}</span></h2>
          <button type="button" class="text-muted-foreground hover:text-foreground" @click="showProduce = false">
            <X class="h-5 w-5" :stroke-width="1.75" />
          </button>
        </div>
        <div class="space-y-3">
          <div class="grid grid-cols-2 gap-3">
            <label>
              <span class="auth-label">Key (optional)</span>
              <AppInput v-model="produceKey" placeholder="Message key" />
            </label>
            <label>
              <span class="auth-label">Partition (optional)</span>
              <AppInput v-model="producePartition" type="number" min="0" placeholder="Auto" />
            </label>
          </div>
          <label>
            <span class="auth-label">Value <span class="text-red-500">*</span></span>
            <textarea v-model="produceValue" rows="7" class="auth-input font-mono text-sm" placeholder='{"key": "value"}' />
          </label>
          <label class="inline-flex items-center gap-2 text-sm text-muted-foreground">
            <input v-model="keepContents" type="checkbox" class="accent-brand-500" />
            Keep contents after send
          </label>
        </div>
        <div class="mt-4 flex justify-end gap-2">
          <AppButton @click="showProduce = false">Cancel</AppButton>
          <AppButton variant="primary" :disabled="producing || !produceValue" @click="sendMessage">
            <Send class="h-4 w-4" :stroke-width="1.75" />
            {{ producing ? "Sending…" : "Send" }}
          </AppButton>
        </div>
      </div>
    </div>

    <!-- ── COPY TOPIC MODAL ──────────────────────────────────────────────── -->
    <div v-if="showCopyModal" class="modal-overlay" @click.self="showCopyModal = false">
      <div class="w-full max-w-md rounded-xl border border-border bg-card p-5 shadow-2xl">
        <div class="mb-4 flex items-center justify-between">
          <h2 class="text-base font-semibold">Copy topic <span class="font-mono text-brand-400">{{ topic }}</span></h2>
          <button type="button" class="text-muted-foreground hover:text-foreground" @click="showCopyModal = false">
            <X class="h-5 w-5" :stroke-width="1.75" />
          </button>
        </div>
        <div class="space-y-3">
          <label>
            <span class="auth-label">New topic name <span class="text-red-500">*</span></span>
            <AppInput
              v-model="copyName"
              placeholder="my-topic-copy"
              @keyup.enter="runCopy"
            />
            <p v-if="copyName.trim() && copyName.trim() === topic" class="mt-1 text-xs text-red-400">
              New name must be different from the original topic.
            </p>
          </label>
        </div>
        <div class="mt-4 flex justify-end gap-2">
          <AppButton @click="showCopyModal = false">Cancel</AppButton>
          <AppButton
            variant="primary"
            :disabled="copyLoading || !copyName.trim() || copyName.trim() === topic"
            @click="runCopy"
          >
            <Copy class="h-4 w-4" :stroke-width="1.75" />
            {{ copyLoading ? "Copying…" : "Copy" }}
          </AppButton>
        </div>
      </div>
    </div>
  </div>
</template>

