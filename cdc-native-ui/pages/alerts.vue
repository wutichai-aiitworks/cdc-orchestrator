<script setup lang="ts">
import { Bell, Pencil, Plus, RefreshCw, Send, ShieldAlert, Trash2 } from "@lucide/vue"
import type { AlertHistoryItem, AlertRule } from "~/composables/useOperationsM3"
import { formatApiError } from "~/utils/apiError"
import {
  CONDITIONS,
  FILTER_PLACEHOLDER,
  channelMeta,
  conditionMeta,
  type NotificationChannel,
  type NotificationDelivery,
} from "~/utils/notificationChannels"

const {
  fetchAlertHistory,
  fetchAlertRules,
  createAlertRule,
  updateAlertRule,
  deleteAlertRule,
  fetchChannels,
  deleteChannel,
  testChannel,
  fetchDeliveries,
} = useOperationsM3()
const { data: capability, refresh: refreshCapability } = useCdcCapability()
const toast = useToast()
const route = useRoute()
const router = useRouter()

type Tab = "history" | "rules" | "channels" | "deliveries"
const TABS: { key: Tab; label: string }[] = [
  { key: "history", label: "Event History" },
  { key: "rules", label: "Rules" },
  { key: "channels", label: "Channels" },
  { key: "deliveries", label: "Deliveries" },
]
const tab = computed<Tab>({
  get: () => (TABS.some((t) => t.key === route.query.tab) ? (route.query.tab as Tab) : "history"),
  set: (t) => {
    router.replace({ query: { ...route.query, tab: t } })
  },
})

const history = ref<AlertHistoryItem[]>([])
const rules = ref<AlertRule[]>([])
const channels = ref<NotificationChannel[]>([])
const secretsAvailable = ref(true)
const deliveries = ref<NotificationDelivery[]>([])
const loading = ref(false)
const saving = ref(false)
const error = ref("")
const available = computed(() => capability.value?.features?.notifications === true)

const channelById = computed(() => new Map(channels.value.map((c) => [c.id, c])))
/** Latest delivery per channel, for the channel cards. */
const lastDelivery = computed(() => {
  const m = new Map<string, NotificationDelivery>()
  for (const d of deliveries.value) if (d.channel_id && !m.has(d.channel_id)) m.set(d.channel_id, d)
  return m
})

const deliveryFilter = reactive({ channel_id: "", outcome: "" })

const loadDeliveries = async () => {
  if (!available.value) return
  try {
    const res = await fetchDeliveries({ limit: 300, channel_id: deliveryFilter.channel_id || undefined, outcome: deliveryFilter.outcome || undefined })
    deliveries.value = res.items || []
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to load deliveries")
  }
}

const load = async () => {
  loading.value = true
  error.value = ""
  await refreshCapability()
  const results = await Promise.allSettled([
    fetchAlertHistory(200),
    available.value ? fetchAlertRules() : Promise.resolve([]),
    available.value ? fetchChannels() : Promise.resolve(null),
    available.value ? fetchDeliveries({ limit: 300 }) : Promise.resolve(null),
  ])
  if (results[0].status === "fulfilled") history.value = results[0].value || []
  if (results[1].status === "fulfilled") rules.value = results[1].value || []
  if (results[2].status === "fulfilled" && results[2].value) {
    channels.value = results[2].value.items || []
    secretsAvailable.value = results[2].value.secretsAvailable
  }
  if (results[3].status === "fulfilled" && results[3].value) deliveries.value = results[3].value.items || []
  const failed = results.find((r) => r.status === "rejected")
  if (failed?.status === "rejected") error.value = formatApiError(failed.reason, "Failed to load alerts")
  loading.value = false
}

watch(deliveryFilter, loadDeliveries)
let poll: ReturnType<typeof setInterval> | undefined
onMounted(() => {
  load()
  poll = setInterval(() => {
    if (tab.value === "deliveries" && !document.hidden) loadDeliveries()
  }, 10_000)
})
onBeforeUnmount(() => clearInterval(poll))

// ── Rules ───────────────────────────────────────────────────────────────────

const showRuleForm = ref(false)
const deleteRuleTarget = ref<AlertRule | null>(null)
const blankRule = (): AlertRule => ({
  id: "",
  name: "",
  conditionType: "connector_failed",
  conditionValue: null,
  connectorFilter: "*",
  channels: [],
  channelIds: [],
  cooldownMinutes: 5,
  enabled: true,
})
const form = reactive<AlertRule>(blankRule())
const minutes = ref(10)
const thresholdSecs = ref(60)
const forMinutes = ref(5)
const formCondition = computed(() => conditionMeta(form.conditionType))
const ruleExists = computed(() => rules.value.some((r) => r.id === form.id))

const newRule = () => {
  if (!available.value) return
  Object.assign(form, blankRule(), { id: crypto.randomUUID() })
  minutes.value = 10
  showRuleForm.value = true
}

const editRule = (rule: AlertRule) => {
  Object.assign(form, blankRule(), JSON.parse(JSON.stringify(rule)), {
    conditionType: conditionMeta(rule.conditionType).value,
    channelIds: rule.channelIds ?? [],
    cooldownMinutes: rule.cooldownMinutes ?? 0,
  })
  minutes.value = Number(rule.conditionValue?.minutes ?? 10)
  thresholdSecs.value = Number(rule.conditionValue?.seconds ?? 60)
  forMinutes.value = Number(rule.conditionValue?.forMinutes ?? 5)
  showRuleForm.value = true
}

const toggleRuleChannel = (id: string) => {
  form.channelIds = form.channelIds.includes(id) ? form.channelIds.filter((c) => c !== id) : [...form.channelIds, id]
}

const isLegacy = (rule: AlertRule) => !(rule.channelIds?.length) && !!rule.webhookUrl

const saveRule = async () => {
  if (!form.name.trim()) return
  if (!form.channelIds.length && !isLegacy(form)) {
    error.value = "Choose at least one channel for the rule."
    return
  }
  saving.value = true
  error.value = ""
  try {
    const body: AlertRule = {
      ...form,
      conditionValue: formCondition.value.needsMinutes
        ? { minutes: Math.max(1, Number(minutes.value) || 10) }
        : formCondition.value.needsThreshold
          ? { seconds: Math.max(1, Number(thresholdSecs.value) || 60), forMinutes: Math.max(0, Number(forMinutes.value) || 0) }
          : null,
      cooldownMinutes: Math.max(0, Number(form.cooldownMinutes) || 0),
    }
    if (ruleExists.value) await updateAlertRule(body)
    else await createAlertRule(body)
    toast.success(ruleExists.value ? "Rule updated." : "Rule created.")
    showRuleForm.value = false
    await load()
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to save rule")
  } finally {
    saving.value = false
  }
}

const confirmDeleteRule = async () => {
  if (!deleteRuleTarget.value) return
  try {
    await deleteAlertRule(deleteRuleTarget.value.id)
    toast.success("Rule deleted.")
    deleteRuleTarget.value = null
    await load()
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to delete rule")
  }
}

// ── Channels ────────────────────────────────────────────────────────────────

const channelDialog = ref<{ channel: NotificationChannel | null } | null>(null)
const deleteChannelTarget = ref<NotificationChannel | null>(null)
const testingId = ref("")

const onChannelSaved = async () => {
  channelDialog.value = null
  await load()
}

const runTest = async (c: NotificationChannel) => {
  testingId.value = c.id
  try {
    const r = await testChannel(c.id)
    if (r.outcome === "delivered") toast.success(`Test sent to ${c.name}.`)
    else toast.error(`Test to ${c.name} failed: ${r.error || "unknown error"}`)
    await loadDeliveries()
  } catch (e: any) {
    toast.error(formatApiError(e, "Test could not be sent"))
  } finally {
    testingId.value = ""
  }
}

const confirmDeleteChannel = async () => {
  if (!deleteChannelTarget.value) return
  try {
    await deleteChannel(deleteChannelTarget.value.id)
    toast.success("Channel deleted.")
    deleteChannelTarget.value = null
    await load()
  } catch (e: any) {
    deleteChannelTarget.value = null
    error.value = formatApiError(e, "Failed to delete channel")
  }
}

// ── Deliveries ──────────────────────────────────────────────────────────────

const OUTCOME_CLASS: Record<string, string> = {
  delivered: "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400",
  failed: "border-red-500/30 bg-red-500/10 text-red-700 dark:text-red-400",
  dropped: "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-400",
}
const fmtTime = (iso?: string) => (iso ? new Date(iso).toLocaleString() : "—")
const failedRecently = computed(() => deliveries.value.filter((d) => d.outcome !== "delivered" && !d.test).length)
</script>

<template>
  <div class="page-container">
    <AppPageHeader
      title="Alerts"
      description="Notify operators by email, LINE, Slack, Teams, Discord or webhook when pipelines and connectors need attention."
    >
      <AppButton :disabled="loading" @click="load">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
      <AppButton v-if="tab === 'channels'" variant="primary" :disabled="!available" @click="channelDialog = { channel: null }">
        <Plus class="h-4 w-4" :stroke-width="1.75" />
        New Channel
      </AppButton>
      <AppButton v-else variant="primary" :disabled="!available" :title="available ? 'Create rule' : 'Requires ORCH_RQLITE_URL'" @click="newRule">
        <Plus class="h-4 w-4" :stroke-width="1.75" />
        New Rule
      </AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="error" kind="error" :message="error" />
    <div v-if="capability && !available" class="alert-info">
      Live alert history is available. Rules, channels and the delivery log need persistence: set
      <code class="font-mono">ORCH_RQLITE_URL</code> and restart the orchestrator.
    </div>

    <section class="grid grid-cols-1 gap-4 md:grid-cols-4">
      <AppMetricCard label="Rules" :value="rules.length" :helper="`${rules.filter((r) => r.enabled).length} enabled`" tone="brand">
        <ShieldAlert class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard label="Channels" :value="channels.length" :helper="`${channels.filter((c) => c.enabled).length} enabled`" tone="neutral">
        <Send class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="Failed deliveries"
        :value="failedRecently"
        helper="In the latest 300 deliveries"
        :tone="failedRecently ? 'danger' : 'success'"
      >
        <Bell class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="Recent events"
        :value="history.length"
        helper="Connector state changes"
        :tone="history.some((item) => String(item.to).toUpperCase() === 'FAILED') ? 'danger' : 'neutral'"
      >
        <Bell class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
    </section>

    <div class="inline-flex flex-wrap rounded-xl border border-border bg-muted p-1" role="tablist">
      <button
        v-for="t in TABS"
        :key="t.key"
        type="button"
        role="tab"
        :aria-selected="tab === t.key"
        :class="['rounded-lg px-4 py-2 text-sm font-medium', tab === t.key ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground']"
        @click="tab = t.key"
      >
        {{ t.label }}
      </button>
    </div>

    <!-- History -->
    <section v-if="tab === 'history'" class="stat-card">
      <div class="mb-4">
        <AppSectionTitle title="Connector Event History" hint="Recent state transitions retained by the running orchestrator instance." />
      </div>
      <AppEmptyState v-if="!history.length" title="No Alert Events" description="Events appear here when watched connectors change state.">
        <template #icon><Bell class="h-5 w-5" :stroke-width="1.75" /></template>
      </AppEmptyState>
      <AppTable v-else>
        <thead><tr><th>Time</th><th>Connector</th><th>Transition</th><th>Rule</th><th>Resolution</th></tr></thead>
        <tbody>
          <tr v-for="(event, index) in history" :key="`${event.ts}-${event.connector}-${index}`">
            <td>{{ fmtTime(event.ts) }}</td>
            <td class="font-mono">{{ event.connector || "—" }}</td>
            <td>
              <span class="text-muted-foreground">{{ event.from || "Unknown" }} → </span>
              <AppStatusBadge :status="event.to" />
            </td>
            <td>{{ event.rule || "Watcher" }}</td>
            <td><AppStatusBadge :status="event.resolved ? 'Resolved' : /^(running|healthy|stable)$/i.test(String(event.to || '')) ? 'Recovered' : 'Open'" /></td>
          </tr>
        </tbody>
      </AppTable>
    </section>

    <!-- Rules -->
    <section v-else-if="tab === 'rules'" class="stat-card">
      <div class="mb-4">
        <AppSectionTitle title="Rules" hint="When to notify, and which channels to send to. A cooldown stops a flapping connector from flooding a channel." />
      </div>
      <AppEmptyState v-if="available && !rules.length" title="No Rules" description="Create a rule to notify a channel when a connector fails, a pipeline operation fails, or a DLQ grows.">
        <template #icon><ShieldAlert class="h-5 w-5" :stroke-width="1.75" /></template>
        <AppButton variant="primary" @click="newRule">Create Rule</AppButton>
      </AppEmptyState>
      <AppTable v-else-if="rules.length">
        <thead><tr><th>Name</th><th>Condition</th><th>Filter</th><th>Channels</th><th>Cooldown</th><th>Status</th><th class="w-24">Actions</th></tr></thead>
        <tbody>
          <tr v-for="rule in rules" :key="rule.id">
            <td class="font-medium">{{ rule.name }}</td>
            <td>
              {{ conditionMeta(rule.conditionType).label }}
              <span v-if="conditionMeta(rule.conditionType).needsMinutes" class="text-muted-foreground">{{ rule.conditionValue?.minutes ?? 10 }} min</span>
              <span v-if="conditionMeta(rule.conditionType).needsThreshold" class="text-muted-foreground">{{ rule.conditionValue?.seconds ?? 60 }} s for {{ rule.conditionValue?.forMinutes ?? 5 }} min</span>
            </td>
            <td class="font-mono text-xs">{{ rule.connectorFilter }}</td>
            <td>
              <div class="flex flex-wrap gap-1">
                <span v-if="isLegacy(rule)" class="rounded border border-amber-500/30 bg-amber-500/10 px-1.5 py-0.5 text-xs text-amber-700 dark:text-amber-400" title="Moved to a channel when the orchestrator restarts with ORCH_SECRET_KEY set">Legacy webhook</span>
                <span v-for="id in rule.channelIds || []" :key="id" class="inline-flex items-center gap-1 rounded border border-border bg-muted py-0.5 pl-0.5 pr-1.5 text-xs">
                  <ChannelBrandIcon v-if="channelById.get(id)" :type="channelById.get(id)!.type" size="sm" class="!h-4 !w-4 !rounded" />
                  {{ channelById.get(id)?.name || "Missing channel" }}
                </span>
              </div>
            </td>
            <td class="tabular-nums">{{ rule.cooldownMinutes ? `${rule.cooldownMinutes} min` : "—" }}</td>
            <td><AppStatusBadge :status="rule.enabled ? 'Enabled' : 'Disabled'" /></td>
            <td>
              <div class="flex items-center gap-2">
                <AppButton size="icon" title="Edit rule" @click="editRule(rule)"><Pencil class="h-4 w-4" :stroke-width="1.75" /></AppButton>
                <AppButton size="icon" variant="destructive" title="Delete rule" @click="deleteRuleTarget = rule"><Trash2 class="h-4 w-4" :stroke-width="1.75" /></AppButton>
              </div>
            </td>
          </tr>
        </tbody>
      </AppTable>
    </section>

    <!-- Channels -->
    <section v-else-if="tab === 'channels'" class="space-y-4">
      <div v-if="available && !secretsAvailable" class="alert-warning !items-start">
        <span>
          <code class="font-mono">ORCH_SECRET_KEY</code> is not set, so channels that need a token or password (LINE, Slack, Teams,
          Discord, SMTP login, webhook signing) can't be saved. Generate one with
          <code class="font-mono">openssl rand -base64 32</code>, add it to <code class="font-mono">docker/.env</code>, and restart the orchestrator.
        </span>
      </div>
      <AppEmptyState v-if="available && !channels.length" title="No Channels" description="Add where alerts go: an email list, a LINE group, a Slack or Teams channel, or your own webhook.">
        <template #icon><Send class="h-5 w-5" :stroke-width="1.75" /></template>
        <AppButton variant="primary" @click="channelDialog = { channel: null }">Add Channel</AppButton>
      </AppEmptyState>
      <div v-else class="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
        <article v-for="c in channels" :key="c.id" class="stat-card flex flex-col gap-3">
          <div class="flex items-start justify-between gap-3">
            <div class="flex min-w-0 items-center gap-3">
              <ChannelBrandIcon :type="c.type" />
              <div class="min-w-0">
                <p class="truncate font-medium text-foreground">{{ c.name }}</p>
                <p class="text-xs text-muted-foreground">{{ channelMeta(c.type).label }}</p>
              </div>
            </div>
            <AppStatusBadge :status="c.enabled ? 'Enabled' : 'Disabled'" />
          </div>
          <p class="truncate font-mono text-xs text-muted-foreground" :title="c.target">{{ c.target || "—" }}</p>
          <p class="text-xs">
            <template v-if="lastDelivery.get(c.id)">
              <span class="rounded border px-1.5 py-0.5" :class="OUTCOME_CLASS[lastDelivery.get(c.id)!.outcome]">{{ lastDelivery.get(c.id)!.outcome }}</span>
              <span class="ml-2 text-muted-foreground">{{ fmtTime(lastDelivery.get(c.id)!.timestamp) }}</span>
            </template>
            <span v-else class="text-muted-foreground">No deliveries yet</span>
          </p>
          <p v-if="c.migratedFromRule" class="text-xs text-muted-foreground">Moved from an older rule's webhook.</p>
          <div class="mt-auto flex gap-2 border-t border-border pt-3">
            <AppButton size="sm" :disabled="testingId === c.id" @click="runTest(c)">
              <Send class="h-3.5 w-3.5" :stroke-width="1.75" />{{ testingId === c.id ? "Sending…" : "Test" }}
            </AppButton>
            <AppButton size="sm" @click="channelDialog = { channel: c }"><Pencil class="h-3.5 w-3.5" :stroke-width="1.75" />Edit</AppButton>
            <AppButton size="sm" variant="ghost" class="ml-auto text-destructive" @click="deleteChannelTarget = c">
              <Trash2 class="h-3.5 w-3.5" :stroke-width="1.75" />
            </AppButton>
          </div>
        </article>
      </div>
    </section>

    <!-- Deliveries -->
    <section v-else class="stat-card">
      <div class="mb-4 flex flex-wrap items-end justify-between gap-3">
        <AppSectionTitle title="Delivery Log" hint="Every send attempt's final result (after retries). Refreshes every 10 seconds." />
        <div class="flex gap-2">
          <AppSelect v-model="deliveryFilter.channel_id" select-class="w-48" aria-label="Channel">
            <option value="">All channels</option>
            <option v-for="c in channels" :key="c.id" :value="c.id">{{ c.name }}</option>
          </AppSelect>
          <AppSelect v-model="deliveryFilter.outcome" select-class="w-36" aria-label="Outcome">
            <option value="">All outcomes</option>
            <option value="delivered">Delivered</option>
            <option value="failed">Failed</option>
            <option value="dropped">Dropped</option>
          </AppSelect>
        </div>
      </div>
      <AppEmptyState v-if="!deliveries.length" title="No Deliveries" description="Deliveries appear here when a rule fires or you send a test.">
        <template #icon><Send class="h-5 w-5" :stroke-width="1.75" /></template>
      </AppEmptyState>
      <AppTable v-else>
        <thead><tr><th>Time</th><th>Channel</th><th>Rule</th><th>Event</th><th>Outcome</th><th>Attempts</th><th>Detail</th></tr></thead>
        <tbody>
          <tr v-for="d in deliveries" :key="d.delivery_id">
            <td class="whitespace-nowrap">{{ fmtTime(d.timestamp) }}</td>
            <td>
              <span class="flex items-center gap-2">
                <ChannelBrandIcon :type="d.channel_type" size="sm" />
                <span class="font-medium">{{ d.channel_name }}</span>
              </span>
            </td>
            <td>{{ d.test ? "Test" : d.rule_name || "—" }}</td>
            <td>
              {{ conditionMeta(d.condition).label }}
              <span class="block font-mono text-xs text-muted-foreground">{{ d.subject }}</span>
            </td>
            <td><span class="rounded border px-1.5 py-0.5 text-xs" :class="OUTCOME_CLASS[d.outcome]">{{ d.outcome }}</span></td>
            <td class="tabular-nums">{{ d.attempts }}</td>
            <td class="max-w-md break-words text-xs text-muted-foreground">
              <template v-if="d.error">{{ d.error }}</template>
              <template v-else>{{ d.status_code ? `HTTP ${d.status_code} · ` : "" }}{{ d.duration_ms }} ms</template>
            </td>
          </tr>
        </tbody>
      </AppTable>
    </section>
  </div>

  <NotificationChannelDialog
    v-if="channelDialog"
    :channel="channelDialog.channel"
    :secrets-available="secretsAvailable"
    @close="channelDialog = null"
    @saved="onChannelSaved"
  />

  <Teleport to="body">
    <div v-if="showRuleForm" class="modal-overlay" @click.self="showRuleForm = false">
      <section class="auth-card w-full max-w-xl bg-card p-5" role="dialog" aria-modal="true">
        <div class="mb-4 flex items-center gap-1.5">
          <h2 class="section-title mb-0">{{ ruleExists ? "Edit Rule" : "New Rule" }}</h2>
          <AppInfoHint text="Define when to notify operators and which channels receive it." />
        </div>
        <div class="space-y-4">
          <label class="block">
            <span class="auth-label">Rule name</span>
            <AppInput v-model="form.name" placeholder="Production connector failures" />
          </label>
          <div class="grid grid-cols-1 gap-4 md:grid-cols-2">
            <label class="block">
              <span class="auth-label">Condition</span>
              <AppSelect v-model="form.conditionType" aria-label="Alert condition">
                <option v-for="c in CONDITIONS" :key="c.value" :value="c.value">{{ c.label }}</option>
              </AppSelect>
            </label>
            <label class="block">
              <span class="auth-label">{{ formCondition.subject === "topic" ? "DLQ topic filter" : formCondition.subject === "pipeline" ? "Pipeline filter" : "Connector filter" }}</span>
              <AppInput v-model="form.connectorFilter" input-class="font-mono" :placeholder="FILTER_PLACEHOLDER[formCondition.subject]" />
            </label>
          </div>
          <p class="-mt-2 text-xs text-muted-foreground">{{ formCondition.hint }}</p>
          <div class="grid grid-cols-1 gap-4 md:grid-cols-2">
            <label v-if="formCondition.needsMinutes" class="block">
              <span class="auth-label">Still failed after (minutes)</span>
              <input v-model.number="minutes" type="number" min="1" class="auth-input" />
            </label>
            <template v-if="formCondition.needsThreshold">
              <label class="block">
                <span class="auth-label">Above (seconds)</span>
                <input v-model.number="thresholdSecs" type="number" min="1" class="auth-input" />
              </label>
              <label class="block">
                <span class="auth-label">For at least (minutes)</span>
                <input v-model.number="forMinutes" type="number" min="0" class="auth-input" />
              </label>
            </template>
            <label class="block">
              <span class="auth-label">Cooldown (minutes, 0 = none)</span>
              <input v-model.number="form.cooldownMinutes" type="number" min="0" class="auth-input" />
            </label>
          </div>

          <div class="block">
            <div class="mb-1.5 flex items-center justify-between">
              <span class="text-xs font-medium text-muted-foreground">Send to</span>
              <button type="button" class="text-xs text-primary hover:underline" @click="showRuleForm = false; tab = 'channels'; channelDialog = { channel: null }">
                + New channel
              </button>
            </div>
            <p v-if="!channels.length" class="rounded-lg border border-dashed border-border px-3 py-3 text-sm text-muted-foreground">
              No channels yet. Create one first, then pick it here.
            </p>
            <div v-else class="max-h-48 space-y-1 overflow-y-auto rounded-lg border border-border p-1">
              <label
                v-for="c in channels"
                :key="c.id"
                class="flex cursor-pointer items-center gap-3 rounded-md px-2 py-1.5 hover:bg-muted"
              >
                <input type="checkbox" class="h-4 w-4 accent-brand-500" :checked="form.channelIds.includes(c.id)" @change="toggleRuleChannel(c.id)" />
                <ChannelBrandIcon :type="c.type" size="sm" />
                <span class="min-w-0 flex-1 truncate text-sm">{{ c.name }}</span>
                <span class="text-xs text-muted-foreground">{{ channelMeta(c.type).label }}</span>
                <span v-if="!c.enabled" class="text-xs text-amber-600">disabled</span>
              </label>
            </div>
            <p v-if="isLegacy(form) && !form.channelIds.length" class="mt-1 text-xs text-amber-700 dark:text-amber-400">
              This rule still uses its old inline webhook ({{ form.webhookUrl }}). Pick a channel to replace it.
            </p>
          </div>

          <label class="flex items-center gap-2 text-sm text-foreground">
            <input v-model="form.enabled" type="checkbox" class="h-4 w-4 accent-brand-500" />
            Enabled
          </label>
        </div>
        <div class="mt-5 flex justify-end gap-2">
          <AppButton :disabled="saving" @click="showRuleForm = false">Cancel</AppButton>
          <AppButton variant="primary" :disabled="saving || !form.name.trim()" @click="saveRule">
            {{ saving ? "Saving..." : "Save Rule" }}
          </AppButton>
        </div>
      </section>
    </div>

    <AppConfirmDialog
      :open="!!deleteRuleTarget"
      title="Delete rule"
      confirm-label="Delete Rule"
      danger
      @update:open="(v: boolean) => { if (!v) deleteRuleTarget = null }"
      @confirm="confirmDeleteRule"
    >
      Delete <span class="font-medium text-foreground">{{ deleteRuleTarget?.name }}</span>? Notifications from this rule stop immediately.
    </AppConfirmDialog>
    <AppConfirmDialog
      :open="!!deleteChannelTarget"
      title="Delete channel"
      confirm-label="Delete Channel"
      danger
      @update:open="(v: boolean) => { if (!v) deleteChannelTarget = null }"
      @confirm="confirmDeleteChannel"
    >
      Delete <span class="font-medium text-foreground">{{ deleteChannelTarget?.name }}</span>? A channel still used by a rule can't be deleted.
    </AppConfirmDialog>
  </Teleport>
</template>
