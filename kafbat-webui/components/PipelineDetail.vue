<script setup lang="ts">
import type { Component } from "vue"
import {
  Activity,
  AlertCircle,
  CircleCheck,
  CircleX,
  FlaskConical,
  MoreHorizontal,
  Pause,
  Pencil,
  Play,
  Power,
  RefreshCcwDot,
  RotateCw,
  ScrollText,
  Settings2,
  Square,
  Table2,
  Trash2,
} from "@lucide/vue"
import type { ActionMenuItem } from "~/components/AppActionMenu.vue"
import type { AuditEventItem } from "~/composables/useOperationsM3"
import type { CdcProfile, ProfileDlqStatus } from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"

/**
 * Selected-pipeline view: status with its reason, the one next action, and tabs
 * (Tables, Activity, Logs, Configuration) so problems can be found without leaving
 * the page. Lifecycle side effects stay in the parent page; this component emits.
 */

export type PipelineDetailAction =
  | "start"
  | "pause"
  | "resume"
  | "restart"
  | "stop"
  | "edit"
  | "toggle-enabled"
  | "delete"
  | "test-sync"
  | "resync"

type FlowProps = {
  sourceLabel: string
  sourceCaption: string
  sourceImage: string
  transformCaption: string
  targetLabel: string
  targetCaption: string
  targetImage: string
}

const props = defineProps<{
  profile: CdcProfile
  flow: FlowProps
  connectors: string[]
  dlqStatus: ProfileDlqStatus | null
  dlqLoading: boolean
  canStart: boolean
  startBlockedReason: string
  supportsTestSync: boolean
}>()

const emit = defineEmits<{
  action: [action: PipelineDetailAction]
  autostart: [enabled: boolean]
}>()

type TabKey = "tables" | "activity" | "logs" | "configuration"

const isReplica = computed(() => props.profile.pattern === "source_to_target")
type TabDef = { key: TabKey; label: string; icon: Component }
const tabs = computed<TabDef[]>(() => [
  ...(isReplica.value ? [{ key: "tables" as const, label: "Tables", icon: Table2 }] : []),
  { key: "activity", label: "Activity", icon: Activity },
  { key: "logs", label: "Logs", icon: ScrollText },
  { key: "configuration", label: "Configuration", icon: Settings2 },
])
const tab = ref<TabKey>(isReplica.value ? "tables" : "activity")

// ── Status and its reason ────────────────────────────────────────────────────

const status = computed(() => String(props.profile.status || "unknown").toLowerCase())
const history = computed(() => [...(props.profile.history ?? [])].reverse())

const EVENT_LABELS: Record<string, string> = {
  profile_created: "Created",
  profile_updated: "Configuration updated",
  profile_started: "Started",
  profile_autostarted: "Started after orchestrator restart",
  profile_stopped: "Stopped",
  profile_paused: "Paused",
  profile_resumed: "Resumed",
  profile_restarted: "Restarted",
  profile_resynced: "Resynced",
  profile_enable_changed: "Enabled state changed",
  profile_test_initial_sync: "Test initial sync passed",
  profile_test_initial_sync_failed: "Test initial sync failed",
}

const eventLabel = (event: string) =>
  EVENT_LABELS[event] ?? event.replace(/^profile[._]/, "").replace(/[._]/g, " ").replace(/^\w/, (c) => c.toUpperCase())

const relativeTime = (iso?: string | null) => {
  if (!iso) return ""
  const seconds = Math.round((Date.now() - new Date(iso).getTime()) / 1000)
  if (!Number.isFinite(seconds)) return ""
  if (seconds < 45) return "just now"
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `${minutes} min ago`
  const hours = Math.round(minutes / 60)
  if (hours < 24) return `${hours} h ago`
  return `${Math.round(hours / 24)} d ago`
}

const statusReason = computed(() => {
  if (!props.profile.enabled) return "Disabled — actions are blocked until you enable it"
  if (status.value === "draft") return "Draft — finish setup before it can run"
  const last = history.value[0]
  return last ? `${eventLabel(last.event)} · ${relativeTime(last.timestamp)}` : ""
})

const lastTest = computed(() =>
  history.value.find((h) => h.event === "profile_test_initial_sync" || h.event === "profile_test_initial_sync_failed"),
)

const autoNamed = computed(() => /^draft\s*·/i.test(props.profile.name))

// ── Actions ──────────────────────────────────────────────────────────────────

type Primary = { key: PipelineDetailAction; label: string; icon: Component; disabled?: boolean; title?: string }

const primary = computed<Primary>(() => {
  if (status.value === "draft") return { key: "edit", label: "Continue setup", icon: Pencil }
  if (!props.profile.enabled) {
    return { key: "toggle-enabled", label: "Enable", icon: Power, disabled: !props.canStart, title: props.startBlockedReason }
  }
  if (status.value === "running") return { key: "pause", label: "Pause", icon: Pause }
  if (status.value === "paused") return { key: "resume", label: "Resume", icon: Play }
  return { key: "start", label: "Start", icon: Play, disabled: !props.canStart, title: props.startBlockedReason }
})

const moreItems = computed<ActionMenuItem[]>(() => {
  const blocked = !props.canStart
  const items: ActionMenuItem[] = [
    { key: "start", label: "Start", icon: Play, disabled: blocked },
    { key: "pause", label: "Pause", icon: Pause },
    { key: "resume", label: "Resume", icon: Play },
    { key: "restart", label: "Restart", icon: RotateCw, disabled: blocked },
    { key: "stop", label: "Stop", icon: Square },
  ].filter((item) => item.key !== primary.value.key)
  if (props.supportsTestSync) {
    items.push({ key: "test-sync", label: "Test initial sync…", icon: FlaskConical, disabled: blocked, separatorBefore: true })
  }
  items.push({ key: "resync", label: "Resync…", icon: RefreshCcwDot, disabled: blocked, separatorBefore: !props.supportsTestSync })
  if (primary.value.key !== "toggle-enabled") {
    items.push({ key: "toggle-enabled", label: props.profile.enabled ? "Disable" : "Enable", icon: Power, separatorBefore: true })
  }
  items.push({ key: "delete", label: "Delete…", icon: Trash2, destructive: true, separatorBefore: primary.value.key === "toggle-enabled" })
  return items
})

// ── Activity: profile history + audit trail ──────────────────────────────────

const { fetchAuditEvents } = useOperationsM3()
const audit = ref<AuditEventItem[]>([])
const auditLoading = ref(false)
const auditError = ref("")

const loadAudit = async () => {
  auditLoading.value = true
  auditError.value = ""
  try {
    audit.value = (await fetchAuditEvents({ profile_id: props.profile.id, limit: 50 })).items ?? []
  } catch (e: any) {
    auditError.value = formatApiError(e, "Audit trail unavailable")
  } finally {
    auditLoading.value = false
  }
}

watch(
  () => [tab.value, props.profile.updated_at] as const,
  ([current]) => {
    if (current === "activity") void loadAudit()
  },
  { immediate: true },
)

type ActivityRow = { key: string; at: string; title: string; detail: string; failed: boolean; source: "pipeline" | "audit" }

const activity = computed<ActivityRow[]>(() => {
  const rows: ActivityRow[] = history.value.map((h, i) => ({
    key: `h-${i}-${h.timestamp}`,
    at: h.timestamp,
    title: eventLabel(h.event),
    detail: h.detail,
    failed: h.level === "error" || h.event.endsWith("_failed"),
    source: "pipeline",
  }))
  for (const e of audit.value) {
    if (e.outcome === "success") continue // successes already appear in the pipeline history
    rows.push({
      key: `a-${e.audit_id}`,
      at: e.timestamp,
      title: `${e.action} failed`,
      detail: auditErrorText(e.detail),
      failed: true,
      source: "audit",
    })
  }
  return rows.sort((a, b) => b.at.localeCompare(a.at))
})

const auditErrorText = (detail: string) => {
  try {
    const parsed = JSON.parse(detail)
    return String(parsed?.error ?? detail)
  } catch {
    return detail
  }
}

// ── Configuration in plain language ──────────────────────────────────────────

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`

const everyInterval = (seconds?: number | null) => {
  if (!seconds) return "on a schedule"
  if (seconds % 3600 === 0) return `every ${plural(seconds / 3600, "hour")}`
  if (seconds % 60 === 0) return `every ${plural(seconds / 60, "minute")}`
  return `every ${plural(seconds, "second")}`
}

const captureText = computed(() => {
  const s = props.profile.source
  switch (s.capture_mode ?? "logical") {
    case "trigger":
      return "Database triggers record each change in a changelog table, which is read on a schedule."
    case "incremental":
      if (s.timestamp_column) return `Reads rows whose ${s.timestamp_column} changed since the last poll. Hard deletes are not captured.`
      if (s.incrementing_column) return `Reads rows with a higher ${s.incrementing_column} than the last poll. Updates and deletes are not captured.`
      return "Polls the source for changed rows. Hard deletes are not captured."
    case "sync_once":
      return "Copies the selected tables once, without ongoing change capture."
    default:
      return "Streams every insert, update, and delete from the database log (WAL / binlog / redo)."
  }
})

const scheduleText = computed(() => {
  const sch = props.profile.flow.schedule
  return sch.delivery_mode === "micro_batch"
    ? `In batches, ${everyInterval(sch.interval_seconds)}.`
    : "Continuously, as changes arrive."
})

const scopeText = computed(() => {
  const s = props.profile.source
  if (s.tables.length) return `${plural(s.tables.length, "table")}: ${s.tables.join(", ")}`
  if (s.schemas.length) return `All tables in ${s.schemas.join(", ")}`
  return "All tables in all schemas"
})

const sinkErrorText = computed(() => {
  if (!isReplica.value && props.profile.pattern !== "source_to_lake") return "No managed sink for this pattern."
  if ((props.profile.sink_error_policy ?? "fail_fast") === "fail_fast") {
    return "Stop the pipeline on the first record the target rejects."
  }
  if (props.dlqLoading) return "Send rejected records to the dead-letter queue (loading count…)."
  const n = props.dlqStatus?.messages_count ?? 0
  return `Send rejected records to the dead-letter queue — ${plural(n, "record")} so far.`
})

const connectionText = (c?: Record<string, string> | null) => {
  if (!c) return "—"
  const hostPort = [c.host, c.port].filter(Boolean).join(":")
  return [hostPort, c.database].filter(Boolean).join(" / ") || "—"
}

const configRows = computed(() => [
  { label: "Source", value: `${props.flow.sourceLabel} · ${connectionText(props.profile.source.connection)}` },
  { label: "Destination", value: [props.flow.targetLabel, props.flow.targetCaption].filter(Boolean).join(" · ") },
  { label: "How changes are captured", value: captureText.value },
  { label: "When changes are delivered", value: scheduleText.value },
  { label: "What is captured", value: scopeText.value },
  { label: "If the target rejects a record", value: sinkErrorText.value },
])
</script>

<template>
  <section class="space-y-4">
    <!-- Header: identity, status with its reason, and the one next action -->
    <div class="stat-card overflow-visible">
      <div class="flex flex-wrap items-start justify-between gap-4">
        <div class="min-w-0 space-y-1">
          <div class="flex flex-wrap items-center gap-2">
            <h2 class="truncate text-lg font-semibold text-foreground">{{ profile.name }}</h2>
            <AppStatusBadge :status="profile.enabled ? profile.status : 'disabled'" />
            <button
              v-if="autoNamed"
              type="button"
              class="btn-ghost text-xs"
              title="Auto-generated name — give it a name you'll recognise"
              @click="emit('action', 'edit')"
            >
              Rename
            </button>
          </div>
          <p v-if="statusReason" class="text-sm text-muted-foreground">{{ statusReason }}</p>
          <p class="truncate font-mono text-xs text-muted-foreground" :title="`${flow.sourceCaption} → ${flow.targetCaption}`">
            {{ flow.sourceLabel }} {{ connectionText(profile.source.connection) }} → {{ flow.targetCaption || flow.targetLabel }}
          </p>
        </div>
        <div class="relative z-20 flex flex-wrap items-center gap-2">
          <AppButton
            variant="primary"
            :disabled="primary.disabled"
            :title="primary.disabled ? primary.title : undefined"
            @click="emit('action', primary.key)"
          >
            <component :is="primary.icon" class="h-4 w-4" :stroke-width="1.75" />
            {{ primary.label }}
          </AppButton>
          <AppButton v-if="primary.key !== 'edit'" @click="emit('action', 'edit')">
            <Pencil class="h-4 w-4" :stroke-width="1.75" />
            Edit
          </AppButton>
          <AppActionMenu
            label="More"
            :icon="MoreHorizontal"
            :items="moreItems"
            @select="(key) => emit('action', key as PipelineDetailAction)"
          />
        </div>
      </div>

      <div v-if="status === 'failed' && profile.last_error" class="alert-error mt-4 items-start">
        <AlertCircle class="mt-0.5 h-4 w-4 shrink-0" :stroke-width="1.75" />
        <div class="min-w-0">
          <p class="font-medium">Last error</p>
          <p class="break-words font-mono text-xs">{{ profile.last_error }}</p>
        </div>
      </div>
      <p v-if="!canStart && startBlockedReason" class="mt-3 text-xs text-muted-foreground">{{ startBlockedReason }}</p>
    </div>

    <!-- Tabs -->
    <div class="stat-card p-0">
      <div class="flex gap-1 overflow-x-auto border-b border-border px-2" role="tablist">
        <button
          v-for="t in tabs"
          :key="t.key"
          type="button"
          role="tab"
          :aria-selected="tab === t.key"
          class="-mb-px inline-flex h-11 items-center gap-2 border-b-2 px-3 text-sm font-medium transition-colors focus:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          :class="tab === t.key ? 'border-primary text-primary' : 'border-transparent text-muted-foreground hover:text-foreground'"
          @click="tab = t.key"
        >
          <component :is="t.icon" class="h-4 w-4" :stroke-width="1.75" />
          {{ t.label }}
          <span
            v-if="t.key === 'activity' && activity.some((r) => r.failed)"
            class="h-1.5 w-1.5 rounded-full bg-destructive"
            aria-label="has failures"
          />
        </button>
      </div>

      <div class="p-4">
        <ProfileTableSyncPanel v-if="tab === 'tables'" :key="`${profile.id}-sync`" :profile-id="profile.id" compact />

        <div v-else-if="tab === 'activity'" class="space-y-3">
          <AppStateBlock v-if="auditError" kind="error" :message="auditError" />
          <p v-if="!activity.length && !auditLoading" class="text-sm text-muted-foreground">No activity yet.</p>
          <ol class="space-y-2">
            <li
              v-for="row in activity"
              :key="row.key"
              class="flex gap-3 rounded-xl border border-border px-3 py-2"
              :class="{ 'row-error': row.failed }"
            >
              <CircleX v-if="row.failed" class="mt-0.5 h-4 w-4 shrink-0 text-destructive" :stroke-width="1.75" />
              <CircleCheck v-else class="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" :stroke-width="1.75" />
              <div class="min-w-0 flex-1">
                <div class="flex flex-wrap items-baseline justify-between gap-2">
                  <p class="text-sm font-medium text-foreground">{{ row.title }}</p>
                  <time class="text-xs text-muted-foreground" :datetime="row.at" :title="row.at">{{ relativeTime(row.at) }}</time>
                </div>
                <p v-if="row.detail" class="mt-0.5 break-words text-xs text-muted-foreground">{{ row.detail }}</p>
              </div>
            </li>
          </ol>
        </div>

        <PipelineProcessingLog
          v-else-if="tab === 'logs'"
          :key="`${profile.id}-log`"
          :connectors="connectors"
          :title="`Processing log — ${profile.name}`"
        />

        <div v-else class="space-y-4">
          <PipelineFlow
            variant="viewer"
            :pattern="profile.pattern"
            :status="profile.status"
            :source-label="flow.sourceLabel"
            :source-caption="flow.sourceCaption"
            :source-image="flow.sourceImage"
            :transform-caption="flow.transformCaption"
            :target-label="flow.targetLabel"
            :target-caption="flow.targetCaption"
            :target-image="flow.targetImage"
          />
          <dl class="divide-y divide-border rounded-xl border border-border">
            <div v-for="row in configRows" :key="row.label" class="grid gap-1 px-4 py-3 sm:grid-cols-[14rem_1fr] sm:gap-4">
              <dt class="text-sm text-muted-foreground">{{ row.label }}</dt>
              <dd class="break-words text-sm text-foreground">{{ row.value }}</dd>
            </div>
            <div v-if="lastTest" class="grid gap-1 px-4 py-3 sm:grid-cols-[14rem_1fr] sm:gap-4">
              <dt class="text-sm text-muted-foreground">Last test initial sync</dt>
              <dd class="text-sm" :class="lastTest.event.endsWith('_failed') ? 'text-destructive' : 'text-foreground'">
                {{ relativeTime(lastTest.timestamp) }} — {{ lastTest.detail }}
              </dd>
            </div>
            <div v-if="profile.description" class="grid gap-1 px-4 py-3 sm:grid-cols-[14rem_1fr] sm:gap-4">
              <dt class="text-sm text-muted-foreground">Description</dt>
              <dd class="text-sm text-foreground">{{ profile.description }}</dd>
            </div>
          </dl>
          <label v-if="canStart" class="flex cursor-pointer items-center gap-2 text-sm">
            <input
              type="checkbox"
              class="h-4 w-4 rounded border-border accent-brand-500"
              :checked="profile.autostart !== false"
              @change="emit('autostart', ($event.target as HTMLInputElement).checked)"
            />
            <span>Autostart after orchestrator restart</span>
            <AppInfoHint
              text="When enabled, a running pipeline is started again after redeploy or container boot. Turn off to leave connectors stopped until you click Start."
            />
          </label>
        </div>
      </div>
    </div>
  </section>
</template>
