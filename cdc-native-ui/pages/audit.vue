<script setup lang="ts">
import { ChevronDown, ChevronRight, History, Radio, RefreshCw, Search } from "@lucide/vue"
import type { PipelineLogItem } from "~/composables/useOperationsM3"
import { formatApiError } from "~/utils/apiError"

definePageMeta({ layout: "default" })

const { fetchPipelineLogs } = useOperationsM3()
const { fetchProfiles } = useProfilesM3()
const POLL_MS = 3000
const MAX_ROWS = 500

const live = ref(true)
const loading = ref(false)
const error = ref("")
const lastFetchedAt = ref<Date | null>(null)
const events = ref<PipelineLogItem[]>([])
const expanded = ref<Record<string, boolean>>({})
const investigateOpen = ref(false)
const investigateProfileId = ref("")
const investigateProfileName = ref("")

const filterOutcome = ref("")
const filterAction = ref("")
const filterProfileId = ref("")
const filterLevel = ref("")
const filterPhase = ref("")
const searchText = ref("")

const profiles = ref<{ id: string; name: string }[]>([])

const actionPresets = [
  { value: "", label: "All actions" },
  { value: "profile.", label: "Profile control (profile.*)" },
  { value: "sync.", label: "Sync (sync.*)" },
  { value: "connector.", label: "Connectors (connector.*)" },
  { value: "profile.start", label: "profile.start" },
  { value: "profile.pause", label: "profile.pause" },
  { value: "profile.resume", label: "profile.resume" },
  { value: "profile.resync", label: "profile.resync" },
  { value: "sync.interval_tick", label: "sync.interval_tick" },
  { value: "sync.watermark_advanced", label: "sync.watermark_advanced" },
  { value: "connector.failed", label: "connector.failed" },
]

let pollTimer: ReturnType<typeof setInterval> | null = null

const filteredEvents = computed(() => {
  const q = searchText.value.trim().toLowerCase()
  let list = events.value
  if (filterAction.value.endsWith(".")) {
    const prefix = filterAction.value
    list = list.filter((e) => e.action.startsWith(prefix))
  } else if (filterAction.value) {
    list = list.filter((e) => e.action === filterAction.value)
  }
  if (!q) return list
  return list.filter((e) => {
    const hay = [
      e.action,
      e.outcome,
      e.level,
      e.phase,
      e.message,
      e.profile_name,
      e.profile_id,
      e.detail,
    ]
      .filter(Boolean)
      .join(" ")
      .toLowerCase()
    return hay.includes(q)
  })
})

const failureCount = computed(
  () => events.value.filter((e) => e.outcome === "failure").length,
)

const mergeEvents = (incoming: PipelineLogItem[]) => {
  const map = new Map<string, PipelineLogItem>()
  for (const row of [...incoming, ...events.value]) {
    map.set(row.log_id, row)
  }
  events.value = [...map.values()]
    .sort((a, b) => b.timestamp.localeCompare(a.timestamp))
    .slice(0, MAX_ROWS)
}

const buildQuery = () => {
  const query: Parameters<typeof fetchPipelineLogs>[0] = { limit: 150 }
  if (filterOutcome.value) query.outcome = filterOutcome.value
  if (filterProfileId.value) query.profile_id = filterProfileId.value
  if (filterLevel.value) query.level = filterLevel.value
  if (filterPhase.value) query.phase = filterPhase.value
  if (filterAction.value && !filterAction.value.endsWith(".")) {
    query.action = filterAction.value
  }
  return query
}

const pull = async (manual = false) => {
  if (loading.value && !manual) return
  loading.value = true
  if (manual) error.value = ""
  try {
    const res = await fetchPipelineLogs(buildQuery())
    const items = res.items || []
    if (filterAction.value.endsWith(".")) {
      const prefix = filterAction.value
      mergeEvents(items.filter((e) => e.action.startsWith(prefix)))
    } else {
      mergeEvents(items)
    }
    lastFetchedAt.value = new Date()
  } catch (e: unknown) {
    error.value = formatApiError(e, "Failed to load pipeline logs")
  } finally {
    loading.value = false
  }
}

const toggleExpanded = (id: string) => {
  expanded.value[id] = !expanded.value[id]
}

const formatDetail = (raw: string) => {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2)
  } catch {
    return raw
  }
}

const formatTime = (iso: string) => {
  try {
    return new Date(iso).toLocaleString()
  } catch {
    return iso
  }
}

const openInvestigate = (profileId: string, profileName?: string | null) => {
  investigateProfileId.value = profileId
  investigateProfileName.value = profileName || ""
  investigateOpen.value = true
}

const startPoll = () => {
  if (pollTimer) clearInterval(pollTimer)
  pollTimer = setInterval(() => {
    if (live.value) void pull()
  }, POLL_MS)
}

watch([filterOutcome, filterAction, filterProfileId, filterLevel, filterPhase], () => {
  void pull(true)
})

onMounted(async () => {
  try {
    const list = await fetchProfiles()
    profiles.value = (list || []).map((p) => ({ id: p.id, name: p.name }))
  } catch {
    profiles.value = []
  }
  await pull(true)
  startPoll()
})

onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer)
})
</script>

<template>
  <div class="page-container">
    <AppPageHeader
      title="Sync audit"
      description="Live processing log per pipeline — read from rqlite (cdc_pipeline_logs), refreshed every 3s."
    >
      <AppButton
        :variant="live ? 'primary' : 'secondary'"
        :title="live ? 'Live refresh every 3s' : 'Paused'"
        @click="live = !live"
      >
        <Radio class="h-4 w-4" :class="{ 'text-emerald-400': live }" :stroke-width="1.75" />
        {{ live ? "Live" : "Paused" }}
      </AppButton>
      <AppButton :disabled="loading" @click="pull(true)">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loading }" :stroke-width="1.75" />
        Refresh
      </AppButton>
    </AppPageHeader>

    <AppStateBlock v-if="error" kind="error" :message="error" />

    <section class="grid grid-cols-1 gap-4 md:grid-cols-3">
      <AppMetricCard label="Log lines" :value="filteredEvents.length" helper="Merged, newest first" tone="brand">
        <History class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="Failures (buffer)"
        :value="failureCount"
        helper="In current view buffer"
        :tone="failureCount > 0 ? 'danger' : 'success'"
      >
        <History class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
      <AppMetricCard
        label="Last fetch"
        :value="lastFetchedAt ? lastFetchedAt.toLocaleTimeString() : '—'"
        helper="Poll interval 3 seconds"
        tone="neutral"
      >
        <Radio class="h-5 w-5" :stroke-width="1.75" />
      </AppMetricCard>
    </section>

    <section class="stat-card space-y-4">
      <div class="grid grid-cols-1 gap-3 md:grid-cols-3 lg:grid-cols-6">
        <label>
          <span class="auth-label">Outcome</span>
          <select v-model="filterOutcome" class="auth-input w-full">
            <option value="">All</option>
            <option value="success">success</option>
            <option value="failure">failure</option>
          </select>
        </label>
        <label>
          <span class="auth-label">Level</span>
          <select v-model="filterLevel" class="auth-input w-full">
            <option value="">All</option>
            <option value="info">info</option>
            <option value="warn">warn</option>
            <option value="error">error</option>
          </select>
        </label>
        <label>
          <span class="auth-label">Phase</span>
          <select v-model="filterPhase" class="auth-input w-full">
            <option value="">All</option>
            <option value="control">control</option>
            <option value="sync">sync</option>
            <option value="connector">connector</option>
          </select>
        </label>
        <label>
          <span class="auth-label">Action</span>
          <select v-model="filterAction" class="auth-input w-full">
            <option v-for="opt in actionPresets" :key="opt.value || 'all'" :value="opt.value">
              {{ opt.label }}
            </option>
          </select>
        </label>
        <label>
          <span class="auth-label">Pipeline</span>
          <select v-model="filterProfileId" class="auth-input w-full">
            <option value="">All pipelines</option>
            <option v-for="p in profiles" :key="p.id" :value="p.id">{{ p.name }}</option>
          </select>
        </label>
        <label>
          <span class="auth-label">Search</span>
          <div class="relative">
            <Search class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <input v-model="searchText" class="auth-input w-full pl-9" placeholder="message, action, detail…" />
          </div>
        </label>
      </div>

      <AppEmptyState
        v-if="!filteredEvents.length && !loading"
        title="No pipeline logs"
        description="Start a pipeline or wait for the next sync interval. Logs are written to rqlite (cdc_pipeline_logs) when ORCH_RQLITE_URL is set."
      >
        <template #icon><History class="h-5 w-5" :stroke-width="1.75" /></template>
      </AppEmptyState>

      <AppTable v-else>
        <thead>
          <tr>
            <th class="w-8" />
            <th>Time</th>
            <th>Level</th>
            <th>Phase</th>
            <th>Action</th>
            <th>Outcome</th>
            <th>Pipeline</th>
            <th>Message</th>
          </tr>
        </thead>
        <tbody>
          <template v-for="row in filteredEvents" :key="row.log_id">
            <tr class="cursor-pointer hover:bg-muted/40" @click="toggleExpanded(row.log_id)">
              <td class="text-muted-foreground">
                <ChevronDown v-if="expanded[row.log_id]" class="h-4 w-4" />
                <ChevronRight v-else class="h-4 w-4" />
              </td>
              <td class="whitespace-nowrap text-xs">{{ formatTime(row.timestamp) }}</td>
              <td class="font-mono text-xs capitalize">{{ row.level }}</td>
              <td class="font-mono text-xs">{{ row.phase }}</td>
              <td class="font-mono text-xs">{{ row.action }}</td>
              <td>
                <span
                  class="badge-neutral rounded-lg px-2 py-0.5 text-xs font-medium"
                  :class="row.outcome === 'failure' ? 'badge-error' : row.outcome === 'success' ? 'badge-success' : ''"
                >
                  {{ row.outcome }}
                </span>
              </td>
              <td>
                <button
                  v-if="row.profile_id"
                  type="button"
                  class="text-left text-sm text-brand-500 hover:underline"
                  @click.stop="openInvestigate(row.profile_id!, row.profile_name)"
                >
                  {{ row.profile_name || row.profile_id }}
                  <span class="ml-1 text-xs text-muted-foreground">· sync</span>
                </button>
                <span v-else class="text-muted-foreground">—</span>
              </td>
              <td class="max-w-md truncate text-sm" :title="row.message">
                {{ row.message }}
              </td>
            </tr>
            <tr v-if="expanded[row.log_id]">
              <td colspan="8" class="bg-muted/20 p-0">
                <pre class="code-block max-h-64 overflow-auto p-4 text-xs">{{ formatDetail(row.detail) }}</pre>
              </td>
            </tr>
          </template>
        </tbody>
      </AppTable>
    </section>

    <ProfileInvestigateModal
      v-model:open="investigateOpen"
      :profile-id="investigateProfileId"
      :profile-name="investigateProfileName"
    />
  </div>
</template>
