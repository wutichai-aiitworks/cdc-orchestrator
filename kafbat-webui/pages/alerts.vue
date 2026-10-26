<script setup lang="ts">
import { Bell, Plus, RefreshCw, ShieldAlert, Trash2 } from "@lucide/vue"
import type { AlertHistoryItem, AlertRule } from "~/composables/useOperationsM3"
import { formatApiError } from "~/utils/apiError"

const {
  fetchAlertHistory,
  fetchAlertRules,
  createAlertRule,
  updateAlertRule,
  deleteAlertRule,
} = useOperationsM3()
const { data: capability, refresh: refreshCapability } = useCdcCapability()
const toast = useToast()
const tab = ref<"history" | "rules">("history")
const history = ref<AlertHistoryItem[]>([])
const rules = ref<AlertRule[]>([])
const loading = ref(false)
const saving = ref(false)
const error = ref("")
const showRuleForm = ref(false)
const deleteRule = ref<AlertRule | null>(null)
const notificationRulesAvailable = computed(
  () => capability.value?.features?.notifications === true
)

const form = reactive<AlertRule>({
  id: "",
  name: "",
  conditionType: "connector_failed",
  conditionValue: null,
  connectorFilter: "*",
  channels: ["webhook"],
  webhookUrl: "",
  webhookSecret: "",
  enabled: true,
})

const load = async () => {
  loading.value = true
  error.value = ""
  await refreshCapability()
  const results = await Promise.allSettled([
    fetchAlertHistory(200),
    notificationRulesAvailable.value ? fetchAlertRules() : Promise.resolve([]),
  ])
  if (results[0].status === "fulfilled") history.value = results[0].value || []
  if (results[1].status === "fulfilled") rules.value = results[1].value || []
  const failed = results.find((result) => result.status === "rejected")
  if (failed?.status === "rejected") error.value = formatApiError(failed.reason, "Failed to load alerts")
  loading.value = false
}

const newRule = () => {
  if (!notificationRulesAvailable.value) return
  Object.assign(form, {
    id: crypto.randomUUID(),
    name: "",
    conditionType: "connector_failed",
    conditionValue: null,
    connectorFilter: "*",
    channels: ["webhook"],
    webhookUrl: "",
    webhookSecret: "",
    enabled: true,
  })
  showRuleForm.value = true
}

const editRule = (rule: AlertRule) => {
  Object.assign(form, { ...rule, webhookSecret: "" })
  showRuleForm.value = true
}

const saveRule = async () => {
  if (!notificationRulesAvailable.value) return
  if (!form.name.trim()) return
  saving.value = true
  try {
    const exists = rules.value.some((rule) => rule.id === form.id)
    if (exists) await updateAlertRule({ ...form })
    else await createAlertRule({ ...form })
    toast.success(exists ? "Alert rule updated." : "Alert rule created.")
    showRuleForm.value = false
    await load()
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to save alert rule")
  } finally {
    saving.value = false
  }
}

const confirmDelete = async () => {
  if (!deleteRule.value) return
  try {
    await deleteAlertRule(deleteRule.value.id)
    toast.success("Alert rule deleted.")
    deleteRule.value = null
    await load()
  } catch (e: any) {
    error.value = formatApiError(e, "Failed to delete alert rule")
  }
}

onMounted(load)
</script>

<template>
  <div class="page-container">
    <AppPageHeader
      title="Alerts"
      description="Turn connector failures and state changes into actionable notifications for operators."
    >
      <AppButton :disabled="loading" @click="load">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
      <AppButton
        variant="primary"
        :disabled="!notificationRulesAvailable"
        :title="notificationRulesAvailable ? 'Create Alert Rule' : 'Configure DataService persistence to enable rules'"
        @click="newRule"
      >
        <Plus class="h-4 w-4" :stroke-width="1.75" />
        New Alert Rule
      </AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="error" kind="error" :message="error" />
    <div
      v-if="capability && !notificationRulesAvailable"
      class="rounded-xl border border-blue-500/25 bg-blue-500/10 px-4 py-3 text-sm text-blue-700 dark:text-blue-400"
    >
      Live alert history remains available. Persistent notification rules require the orchestrator DataService keystore configuration.
    </div>

    <section class="grid grid-cols-1 gap-4 md:grid-cols-3">
      <AppMetricCard label="Alert Rules" :value="rules.length" helper="Configured detection policies" tone="brand">
        <ShieldAlert class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="Enabled Rules"
        :value="rules.filter((rule) => rule.enabled).length"
        helper="Policies actively evaluated"
        tone="success"
      >
        <Bell class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="Recent Events"
        :value="history.length"
        helper="In-memory watcher history"
        :tone="history.some((item) => String(item.to).toUpperCase() === 'FAILED') ? 'danger' : 'neutral'"
      >
        <Bell class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
    </section>

    <div class="inline-flex rounded-xl border border-border bg-muted p-1">
      <button
        type="button"
        :class="['rounded-lg px-4 py-2 text-sm font-medium', tab === 'history' ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground']"
        @click="tab = 'history'"
      >
        Event History
      </button>
      <button
        type="button"
        :class="['rounded-lg px-4 py-2 text-sm font-medium', tab === 'rules' ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground']"
        @click="tab = 'rules'"
      >
        Notification Rules
      </button>
    </div>

    <section v-if="tab === 'history'" class="stat-card">
      <div class="mb-4">
        <AppSectionTitle title="Connector Event History" hint="Recent state transitions retained by the running orchestrator instance." />
      </div>
      <AppEmptyState
        v-if="!history.length"
        title="No Alert Events"
        description="Events appear here when watched connectors change state."
      >
        <template #icon><Bell class="h-5 w-5" :stroke-width="1.75" /></template>
      </AppEmptyState>
      <AppTable v-else>
        <thead><tr><th>Time</th><th>Connector</th><th>Transition</th><th>Rule</th><th>Resolution</th></tr></thead>
        <tbody>
          <tr v-for="(event, index) in history" :key="`${event.ts}-${event.connector}-${index}`">
            <td>{{ event.ts ? new Date(event.ts).toLocaleString() : "—" }}</td>
            <td class="font-mono">{{ event.connector || "—" }}</td>
            <td>
              <span class="text-muted-foreground">{{ event.from || "Unknown" }} → </span>
              <AppStatusBadge :status="event.to" />
            </td>
            <td>{{ event.rule || "Watcher" }}</td>
            <td><AppStatusBadge :status="event.resolved ? 'Resolved' : 'Open'" /></td>
          </tr>
        </tbody>
      </AppTable>
    </section>

    <section v-else class="stat-card">
      <div class="mb-4">
        <AppSectionTitle title="Notification Rules" hint="Webhook delivery is currently supported. Keep credentials in the protected secret field." />
      </div>
      <AppEmptyState
        v-if="!notificationRulesAvailable"
        title="Notification Rules Are Unavailable"
        description="Configure ORCH_DATASERVICE_URL and ORCH_DATASERVICE_API_KEY, then restart the orchestrator."
      >
        <template #icon><ShieldAlert class="h-5 w-5" :stroke-width="1.75" /></template>
      </AppEmptyState>
      <AppEmptyState
        v-else-if="!rules.length"
        title="No Alert Rules"
        description="Create a rule to notify your operations system when a connector fails or pauses."
      >
        <template #icon><ShieldAlert class="h-5 w-5" :stroke-width="1.75" /></template>
        <AppButton variant="primary" @click="newRule">Create Alert Rule</AppButton>
      </AppEmptyState>
      <AppTable v-else>
        <thead><tr><th>Name</th><th>Condition</th><th>Connector Filter</th><th>Delivery</th><th>Status</th><th>Actions</th></tr></thead>
        <tbody>
          <tr v-for="rule in rules" :key="rule.id">
            <td class="font-medium">{{ rule.name }}</td>
            <td>{{ rule.conditionType === "connector_failed" ? "Connector Failed" : "Connector Paused" }}</td>
            <td class="font-mono">{{ rule.connectorFilter }}</td>
            <td>{{ rule.channels.join(", ") || "None" }}</td>
            <td><AppStatusBadge :status="rule.enabled ? 'Enabled' : 'Disabled'" /></td>
            <td>
              <div class="flex items-center gap-2">
                <AppButton size="icon" title="Edit Alert Rule" @click="editRule(rule)">
                  <span aria-hidden="true">✎</span>
                </AppButton>
                <AppButton size="icon" variant="destructive" title="Delete Alert Rule" @click="deleteRule = rule">
                  <Trash2 class="h-4 w-4" :stroke-width="1.75" />
                </AppButton>
              </div>
            </td>
          </tr>
        </tbody>
      </AppTable>
    </section>
  </div>

  <Teleport to="body">
    <div v-if="showRuleForm" class="modal-overlay" @click.self="showRuleForm = false">
      <section class="auth-card w-full max-w-xl bg-card p-5" role="dialog" aria-modal="true">
        <div class="mb-4 flex items-center gap-1.5">
          <h2 class="section-title mb-0">{{ rules.some((rule) => rule.id === form.id) ? "Edit Alert Rule" : "New Alert Rule" }}</h2>
          <AppInfoHint text="Define when to notify operators and where to send the webhook." />
        </div>
        <div class="space-y-4">
          <label>
            <span class="auth-label">Rule Name</span>
            <AppInput v-model="form.name" placeholder="Production Connector Failures" />
          </label>
          <div class="grid grid-cols-1 gap-4 md:grid-cols-2">
            <label>
              <span class="auth-label">Condition</span>
              <AppSelect v-model="form.conditionType" aria-label="Alert condition">
                <option value="connector_failed">Connector Failed</option>
                <option value="connector_paused">Connector Paused</option>
              </AppSelect>
            </label>
            <label>
              <span class="auth-label">Connector Filter</span>
              <AppInput v-model="form.connectorFilter" placeholder="* or orders-*" />
            </label>
          </div>
          <label>
            <span class="auth-label">Webhook URL</span>
            <AppInput v-model="form.webhookUrl" type="url" placeholder="https://alerts.example.com/cdc" />
          </label>
          <label>
            <span class="auth-label">Webhook Secret</span>
            <AppInput v-model="form.webhookSecret" type="password" placeholder="Leave blank to keep existing secret" />
          </label>
          <label class="flex items-center gap-2 text-sm text-foreground">
            <input v-model="form.enabled" type="checkbox" class="h-4 w-4 accent-brand-500" />
            Enable this rule immediately
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

    <div v-if="deleteRule" class="modal-overlay" @click.self="deleteRule = null">
      <section class="auth-card w-full max-w-md bg-card p-5" role="alertdialog" aria-modal="true">
        <h2 class="section-title text-destructive">Delete Alert Rule</h2>
        <p class="mt-2 text-sm text-muted-foreground">
          Delete <span class="font-medium text-foreground">{{ deleteRule.name }}</span>? Notifications from this rule stop immediately.
        </p>
        <div class="mt-5 flex justify-end gap-2">
          <AppButton @click="deleteRule = null">Cancel</AppButton>
          <AppButton variant="destructive" @click="confirmDelete">Delete Rule</AppButton>
        </div>
      </section>
    </div>
  </Teleport>
</template>
