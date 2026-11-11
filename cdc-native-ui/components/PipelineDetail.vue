<script setup lang="ts">
import type { Component } from "vue"
import {
  Tag,
  Activity,
  AlertCircle,
  AlertTriangle,
  Copy,
  FlaskConical,
  MoreHorizontal,
  Pause,
  Pencil,
  Play,
  Power,
  RefreshCcwDot,
  DatabaseZap,
  RotateCw,
  ScrollText,
  Settings2,
  Square,
  Table2,
  Split,
  Trash2,
  Unplug,
} from "@lucide/vue"
import type { ActionMenuItem } from "~/components/AppActionMenu.vue"
import type { CdcProfile, ProfileDlqStatus } from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"
import { eventLabel } from "~/utils/pipelineEvents"

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
  | "initial-load"
  | "split"
  | "duplicate"

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
  /** A lifecycle action is in flight: block a second click until it settles. */
  busy?: boolean
  /** Tags used on other pipelines, offered while editing this one's tags. */
  tagSuggestions?: string[]
  /** Tags are being saved. */
  tagsSaving?: boolean
}>()

const emit = defineEmits<{
  action: [action: PipelineDetailAction]
  autostart: [enabled: boolean]
  /** Save these tags now (the parent persists them and reloads). */
  tags: [tags: string[]]
}>()

// Inline tag editor in the header: edit, then Save or Cancel. It closes once a save
// finishes and the reloaded profile carries the draft's tags; on error it stays open.
const editingTags = ref(false)
const tagDraft = ref<string[]>([])
const startTagEdit = () => {
  tagDraft.value = [...(props.profile.tags ?? [])]
  editingTags.value = true
}
const saveTags = () => emit("tags", [...tagDraft.value])
const tagKey = (tags: string[] | undefined) => (tags ?? []).map((t) => t.toLowerCase()).join("\u0000")
watch(
  () => [props.tagsSaving, tagKey(props.profile.tags)] as const,
  ([saving]) => {
    if (editingTags.value && !saving && tagKey(props.profile.tags) === tagKey(tagDraft.value)) editingTags.value = false
  },
)

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

/** Streaming initial load copies into PostgreSQL replicas only. */
/** Source / target connection state reported by the Tables panel (null = not checked). */
const endpointHealth = ref<import("~/components/ProfileTableSyncPanel.vue").PipelineHealth | null>(null)
watch(
  () => props.profile.id,
  () => {
    endpointHealth.value = null
  },
)

const endpointProblems = computed(() => {
  const h = endpointHealth.value
  if (!h) return []
  const out: { key: string; label: string; offline: boolean }[] = []
  for (const side of ["source", "target"] as const) {
    const name = side === "source" ? "Source" : "Target"
    if (h[side] === "offline") out.push({ key: side, label: `${name} offline`, offline: true })
    else if (h[side] === "attention") out.push({ key: side, label: `${name} needs attention`, offline: false })
  }
  if (h.engine && !out.length) out.push({ key: "engine", label: "Sync engine issue", offline: false })
  return out
})

/** Explicit table list with ≥ 2 tables, not trigger capture (triggers belong to one pipeline). */
const supportsSplit = computed(
  () =>
    props.profile.pattern !== "source_to_olake" &&
    props.profile.source?.capture_mode !== "trigger" &&
    props.profile.source?.table_mode === "specific_tables" &&
    (props.profile.source?.tables ?? []).filter((t) => t.trim()).length >= 2,
)

const supportsInitialLoad = computed(
  () =>
    isReplica.value &&
    ["postgres", "postgresql"].includes(String(props.profile.target?.target_type || "").toLowerCase()),
)

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
  if (supportsInitialLoad.value) {
    items.push({ key: "initial-load", label: "Initial load (streaming)…", icon: DatabaseZap, disabled: !props.profile.enabled })
  }
  if (supportsSplit.value) {
    items.push({ key: "split", label: "Split tables into new pipeline…", icon: Split, separatorBefore: true })
  }
  items.push({ key: "duplicate", label: "Duplicate as new pipeline…", icon: Copy, separatorBefore: !supportsSplit.value })
  if (primary.value.key !== "toggle-enabled") {
    items.push({ key: "toggle-enabled", label: props.profile.enabled ? "Disable" : "Enable", icon: Power, separatorBefore: true })
  }
  items.push({ key: "delete", label: "Delete…", icon: Trash2, destructive: true, separatorBefore: primary.value.key === "toggle-enabled" })
  return items
})

// ── Activity ────────────────────────────────────────────────────────────────
// The timeline itself lives in PipelineActivityTimeline. Here only the tab's red dot:
// a failure in the last 24 h (pipeline history or audit trail), not "ever failed".

const { fetchAuditEvents } = useOperationsM3()
const RECENT_MS = 24 * 3_600_000
const recentAuditFailure = ref(false)

const recentFailure = computed(
  () =>
    recentAuditFailure.value ||
    history.value.some(
      (h) =>
        (h.level === "error" || h.event.endsWith("_failed")) &&
        Date.now() - new Date(h.timestamp).getTime() < RECENT_MS,
    ),
)

watch(
  () => [props.profile.id, props.profile.updated_at] as const,
  async () => {
    try {
      const res = await fetchAuditEvents({
        profile_id: props.profile.id,
        outcome: "failure",
        since: new Date(Date.now() - RECENT_MS).toISOString(),
        limit: 1,
      })
      recentAuditFailure.value = (res.items?.length ?? 0) > 0
    } catch {
      recentAuditFailure.value = false
    }
  },
  { immediate: true },
)

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
            <!-- Which end is broken, in plain words (from the Tables panel's checks). -->
            <template v-if="profile.status === 'running' && endpointHealth">
              <span
                v-for="side in endpointProblems"
                :key="side.key"
                class="inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs font-medium"
                :class="side.offline
                  ? 'border-red-500/40 bg-red-500/10 text-red-700 dark:text-red-300'
                  : 'border-amber-500/40 bg-amber-500/10 text-amber-700 dark:text-amber-300'"
              >
                <Unplug v-if="side.offline" class="h-3 w-3" :stroke-width="2" />
                <AlertTriangle v-else class="h-3 w-3" :stroke-width="2" />
                {{ side.label }}
              </span>
            </template>
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
          <div v-if="!editingTags" class="flex flex-wrap items-center gap-1 pt-1">
            <span v-for="tag in profile.tags ?? []" :key="tag" class="tag-chip">{{ tag }}</span>
            <button type="button" class="btn-ghost inline-flex items-center gap-1 text-xs" @click="startTagEdit">
              <Tag class="h-3 w-3" :stroke-width="1.75" />
              {{ profile.tags?.length ? "Edit tags" : "Add tags" }}
            </button>
          </div>
          <div v-else class="max-w-xl space-y-2 pt-1">
            <PipelineTagInput v-model="tagDraft" :suggestions="tagSuggestions" />
            <div class="flex gap-2">
              <AppButton variant="primary" :disabled="tagsSaving" @click="saveTags">
                {{ tagsSaving ? "Saving…" : "Save tags" }}
              </AppButton>
              <AppButton :disabled="tagsSaving" @click="editingTags = false">Cancel</AppButton>
            </div>
          </div>
        </div>
        <div class="relative z-20 flex flex-wrap items-center gap-2">
          <AppButton
            variant="primary"
            :disabled="primary.disabled || busy"
            :title="primary.disabled ? primary.title : busy ? 'Working…' : undefined"
            @click="emit('action', primary.key)"
          >
            <component :is="primary.icon" class="h-4 w-4" :stroke-width="1.75" :class="{ 'animate-pulse': busy }" />
            {{ busy && primary.key !== 'edit' ? 'Working…' : primary.label }}
          </AppButton>
          <AppButton v-if="primary.key !== 'edit'" :disabled="busy" @click="emit('action', 'edit')">
            <Pencil class="h-4 w-4" :stroke-width="1.75" />
            Edit
          </AppButton>
          <AppActionMenu
            label="More"
            :icon="MoreHorizontal"
            :items="moreItems"
            :disabled="busy"
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
      <!-- No scroll container: overflow-x-auto also makes overflow-y auto, and the tabs'
           -mb-px underline overlap then showed a vertical scrollbar. Wrap instead. -->
      <div class="flex flex-wrap gap-x-1 border-b border-border px-2" role="tablist">
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
            v-if="t.key === 'activity' && recentFailure"
            class="h-1.5 w-1.5 rounded-full bg-destructive"
            title="Failures in the last 24 hours"
            aria-label="has failures in the last 24 hours"
          />
        </button>
      </div>

      <div class="p-4">
        <ProfileTableSyncPanel
          v-if="tab === 'tables'"
          :key="`${profile.id}-sync`"
          :profile-id="profile.id"
          :refresh-token="profile.updated_at"
          compact
          :endpoints="{
            source: { image: flow.sourceImage, label: flow.sourceLabel, caption: flow.sourceCaption },
            target: { image: flow.targetImage, label: flow.targetLabel, caption: flow.targetCaption },
          }"
          @health="endpointHealth = $event"
        />

        <PipelineActivityTimeline
          v-else-if="tab === 'activity'"
          :key="`${profile.id}-activity`"
          :profile="profile"
        />

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
            <div v-if="profile.tags?.length" class="grid gap-1 px-4 py-3 sm:grid-cols-[14rem_1fr] sm:gap-4">
              <dt class="text-sm text-muted-foreground">Tags</dt>
              <dd class="flex flex-wrap gap-1">
                <span v-for="tag in profile.tags" :key="tag" class="tag-chip">{{ tag }}</span>
              </dd>
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
