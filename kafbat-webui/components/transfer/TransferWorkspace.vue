<script setup lang="ts">
// The page shell shared by every Copy & Sync tool: saved transfers on the left; the selected one as
// a route (source → steps → destinations) with Overview / Configuration / Runs. One primary action
// (Run, or Stop while running). Run is disabled while there are unsaved changes, and a run that
// writes to another database is confirmed with the exact effects. Spec: docs/design/migration-tools-ux.md
//
// Slots: #config (the tool's configuration sections), #run-extra (extra run details, e.g. a dump
// file), #last-run-actions (e.g. Download on Overview).
import { ChevronDown, Copy, Eye, Play, Plus, Square, Trash2 } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import {
  changedBy,
  cleanLogLine,
  formatDuration,
  isActive,
  jobDurationMs,
  loginKeyOf,
  railLine,
  startedLabel,
  stripAnsi,
  type Tone,
} from "~/utils/transferView"
import type { ConnCheck, TransferTool } from "~/utils/transferTools/types"

const props = defineProps<{ tool: TransferTool }>()
const tool = props.tool
const toast = useToast()
const route = useRoute()
const router = useRouter()

// ── data ────────────────────────────────────────────────────────────────────
const profiles = ref<any[]>([])
const jobs = ref<any[]>([])
const capability = ref<Record<string, any> | null>(null)
const serviceError = ref("")
const loading = ref(true)
const loadError = ref("")
const now = ref(Date.now())
const logs = reactive<Record<string, string>>({})

type Tab = "overview" | "config" | "runs"
const NEW = "new"
const selectedId = ref("")
const tab = ref<Tab>("overview")
const openRunId = ref("")

const draft = ref<any>(tool.newDraft())
const saved = ref<any | null>(null)
const checks = reactive<Record<string, ConnCheck | null>>({ source: null, target: null })
const clone = <T,>(v: T): T => JSON.parse(JSON.stringify(v))

const loadDraft = (p: any | null) => {
  draft.value = p ? tool.toDraft(p) : tool.newDraft()
  saved.value = p ? clone(tool.toPayload(draft.value)) : null
  checks.source = null
  checks.target = null
}

// ── derived ─────────────────────────────────────────────────────────────────
const isNew = computed(() => selectedId.value === NEW)
const current = computed(() => profiles.value.find((p) => p.id === selectedId.value) || null)
const payload = computed(() => tool.toPayload(draft.value))
const changes = computed(() => (selectedId.value ? changedBy(tool.changeLabels, isNew.value ? null : saved.value, payload.value) : []))
const dirty = computed(() => changes.value.length > 0)

const byNewest = (a: any, b: any) => Date.parse(b.created_at || b.started_at || "") - Date.parse(a.created_at || a.started_at || "")
const jobsFor = (id: string) => tool.jobsOf(jobs.value, id).slice().sort(byNewest)
const profileJobs = computed(() => (current.value ? jobsFor(current.value.id) : []))
const activeJob = computed(() => profileJobs.value.find((j) => isActive(j.status)))
const lastJob = computed(() => profileJobs.value[0])
const lastSuccess = computed(() => profileJobs.value.find((j) => j.status === "succeeded"))
const anyActive = computed(() => jobs.value.some((j) => isActive(j.status)))
const routeJob = computed(() => (dirty.value ? undefined : activeJob.value || lastJob.value))

const steps = computed(() => tool.steps(payload.value, routeJob.value, routeJob.value ? logs[routeJob.value.id] || "" : "", now.value))
const routeSource = computed(() => tool.routeSource(payload.value, checks))
const routeDestinations = computed(() => tool.routeDestinations(payload.value, checks, capability.value))
const result = (j: any) => tool.runResult(j)
const railItems = computed(() => {
  const items = profiles.value.map((p) => {
    const [from] = tool.railEngines(p)
    return { id: p.id, name: p.name, route: tool.railRoute(p), from, status: railLine(jobsFor(p.id)[0], result, now.value) }
  })
  if (isNew.value) {
    const [from] = tool.railEngines({ ...payload.value, id: NEW })
    items.unshift({ id: NEW, name: draft.value.name || "New transfer", route: "Not saved yet", from, status: { tone: "idle" as Tone, text: "Draft" } })
  }
  return items
})

const runBlockedReason = computed(() => {
  if (isNew.value) return "Create the transfer first"
  if (dirty.value) return "Save your changes first"
  if (activeJob.value) return tool.continuous ? "The sync is running" : "A run is in progress"
  return ""
})
const effects = computed(() => tool.effects(payload.value))
const summary = computed(() => tool.summary(payload.value))
const sections = computed(() => tool.sections(payload.value))
const verb = computed(() => tool.verb(payload.value))
const stopVerb = tool.stopVerb || "Stop run"
const retentionHours = computed(() => Number(capability.value?.retention_hours) || 0)

const savedLogin = (side: "source" | "target", defaultPort = "") =>
  isNew.value || !current.value?.[side] ? undefined : loginKeyOf(current.value[side], defaultPort)

// ── URL state: ?t=<id>&tab=runs&run=<id> ───────────────────────────────────
watch([selectedId, tab, openRunId], () => {
  const q: Record<string, string> = {}
  if (selectedId.value && !isNew.value) q.t = selectedId.value
  if (tab.value !== "overview") q.tab = tab.value
  if (openRunId.value && tab.value === "runs") q.run = openRunId.value
  router.replace({ query: q })
})

// ── loading & polling ───────────────────────────────────────────────────────
const refreshJobs = async () => {
  try {
    jobs.value = await tool.api.fetchJobs()
    serviceError.value = ""
  } catch (e: any) {
    serviceError.value = formatApiError(e, `The ${tool.title} service did not answer`)
  }
}
const refreshLog = async (id?: string) => {
  if (!id) return
  try {
    logs[id] = (await tool.api.fetchJobLogs(id)).log || ""
  } catch (e: any) {
    logs[id] = formatApiError(e, "Could not load the log")
  }
}

const loadAll = async () => {
  loading.value = true
  loadError.value = ""
  try {
    const [profs] = await Promise.all([
      tool.api.fetchProfiles(),
      refreshJobs(),
      tool.api.fetchCapability?.()
        .then((c) => (capability.value = c))
        .catch((e) => (serviceError.value = formatApiError(e, `The ${tool.title} service did not answer`))),
    ])
    profiles.value = profs
    const want = String(route.query.t || "")
    const first = profs.find((p: any) => p.id === want) || profs[0]
    if (first) {
      selectedId.value = first.id
      loadDraft(first)
      tab.value = (["overview", "config", "runs"].includes(String(route.query.tab)) ? route.query.tab : "overview") as Tab
      if (route.query.run) openRunId.value = String(route.query.run)
    }
  } catch (e: any) {
    loadError.value = formatApiError(e, `Could not load ${tool.title} transfers`)
  } finally {
    loading.value = false
  }
}

// Logs feed the route's step markers and the run detail: keep the shown run's log current.
watch(
  () => [routeJob.value?.id, openRunId.value].join("|"),
  () => {
    for (const id of [routeJob.value?.id, openRunId.value]) if (id && logs[id] === undefined) refreshLog(id)
  },
)
let timer: ReturnType<typeof setTimeout> | null = null
let clock: ReturnType<typeof setInterval> | null = null
const poll = async () => {
  const before = new Map(jobs.value.map((j) => [j.id, j.status]))
  await refreshJobs()
  for (const j of jobs.value) {
    const was = before.get(j.id)
    const watched = j.id === routeJob.value?.id || j.id === openRunId.value
    // Live runs refresh their log; a run that just finished reads its final log once.
    if (watched && (isActive(j.status) || (was && isActive(was)))) await refreshLog(j.id)
  }
  timer = setTimeout(poll, anyActive.value ? 3000 : 20000)
}
onMounted(async () => {
  await loadAll()
  timer = setTimeout(poll, anyActive.value ? 3000 : 20000)
  clock = setInterval(() => (now.value = Date.now()), 15000)
})
onUnmounted(() => {
  if (timer) clearTimeout(timer)
  if (clock) clearInterval(clock)
})

// ── navigation guarded by unsaved changes ──────────────────────────────────
const pendingNav = ref<null | (() => void)>(null)
const guarded = (fn: () => void) => (dirty.value ? (pendingNav.value = fn) : fn())
const confirmDiscard = () => {
  const fn = pendingNav.value
  pendingNav.value = null
  fn?.()
}
const select = (id: string) => {
  if (id === selectedId.value) return
  guarded(() => {
    const p = profiles.value.find((x) => x.id === id) || null
    selectedId.value = id
    loadDraft(p)
    openRunId.value = ""
  })
}
const startNew = (from?: any) =>
  guarded(() => {
    selectedId.value = NEW
    loadDraft(null)
    if (from) {
      loadDraft(from)
      saved.value = null
      draft.value.name = `${from.name} (copy)`
      // Secrets are not copied: the API never returns them.
      for (const side of ["source", "target"]) if (draft.value[side]) draft.value[side].password = ""
      for (const d of draft.value.destinations || []) Object.assign(d, { password: "", private_key: "", passphrase: "" })
    }
    tab.value = "config"
    openRunId.value = ""
  })
const discard = () => {
  if (isNew.value) {
    const first = profiles.value[0] || null
    selectedId.value = first?.id || ""
    loadDraft(first)
    tab.value = "overview"
  } else loadDraft(current.value)
}

const beforeUnload = (e: BeforeUnloadEvent) => {
  if (dirty.value) e.preventDefault()
}
onMounted(() => window.addEventListener("beforeunload", beforeUnload))
onUnmounted(() => window.removeEventListener("beforeunload", beforeUnload))
onBeforeRouteLeave(() => (dirty.value ? window.confirm("Leave without saving your changes?") : true))

// ── actions ─────────────────────────────────────────────────────────────────
const saving = ref(false)
const save = async () => {
  if (!draft.value.name?.trim()) {
    toast.error("Give the transfer a name")
    tab.value = "config"
    nextTick(() => document.getElementById("tw-name")?.focus())
    return
  }
  saving.value = true
  const keep = { ...checks }
  try {
    const body = payload.value
    if (isNew.value) {
      const created = await tool.api.createProfile(body)
      profiles.value.push(created)
      selectedId.value = created.id
      loadDraft(created)
      toast.success("Transfer created")
    } else {
      const updated = await tool.api.updateProfile(selectedId.value, body)
      const i = profiles.value.findIndex((p) => p.id === updated.id)
      if (i >= 0) profiles.value[i] = updated
      loadDraft(updated)
      toast.success("Changes saved")
    }
    Object.assign(checks, keep)
  } catch (e: any) {
    toast.error(formatApiError(e, "Could not save"))
  } finally {
    saving.value = false
  }
}

const confirmRun = ref(false)
const starting = ref(false)
const requestRun = () => {
  if (runBlockedReason.value) return
  if (effects.value.length) confirmRun.value = true
  else run()
}
const run = () => startJob((id) => tool.api.runProfile(id), tool.startedToast(payload.value))
const runPreview = () => tool.preview && startJob(tool.preview.run, tool.preview.startedToast)
const startJob = async (start: (profileId: string) => Promise<any>, startedText: string) => {
  if (!current.value) return
  starting.value = true
  try {
    const job = await start(current.value.id)
    confirmRun.value = false
    toast.success(startedText)
    delete logs[job.id]
    await refreshJobs()
    tab.value = "runs"
    openRunId.value = job.id
    refreshLog(job.id)
    if (timer) clearTimeout(timer)
    timer = setTimeout(poll, 1500)
  } catch (e: any) {
    toast.error(formatApiError(e, "Could not start"))
  } finally {
    starting.value = false
  }
}

const stopRun = async (job: any) => {
  try {
    await tool.api.stopJob(job.id)
    toast.success(tool.continuous ? "Sync stopped" : "Run stopped")
    await refreshJobs()
    refreshLog(job.id)
  } catch (e: any) {
    toast.error(formatApiError(e, "Could not stop"))
  }
}

const runToDelete = ref<any | null>(null)
const deleting = ref(false)
const deleteRun = async () => {
  const job = runToDelete.value
  if (!job || !tool.api.deleteJob) return
  deleting.value = true
  try {
    await tool.api.deleteJob(job.id)
    jobs.value = jobs.value.filter((j) => j.id !== job.id)
    if (openRunId.value === job.id) openRunId.value = ""
    toast.success("Run deleted")
    runToDelete.value = null
  } catch (e: any) {
    toast.error(formatApiError(e, "Could not delete the run"))
  } finally {
    deleting.value = false
  }
}

const confirmDeleteProfile = ref(false)
const deleteProfile = async () => {
  if (!current.value) return
  deleting.value = true
  try {
    await tool.api.deleteProfile(current.value.id)
    profiles.value = profiles.value.filter((p) => p.id !== current.value!.id)
    confirmDeleteProfile.value = false
    const first = profiles.value[0] || null
    selectedId.value = first?.id || ""
    loadDraft(first)
    tab.value = "overview"
    toast.success("Transfer deleted")
  } catch (e: any) {
    toast.error(formatApiError(e, "Could not delete the transfer"))
  } finally {
    deleting.value = false
  }
}

const menuItems = computed(() => [
  { key: "duplicate", label: "Duplicate", icon: Copy, disabled: isNew.value },
  { key: "delete", label: "Delete transfer", icon: Trash2, destructive: true, separatorBefore: true, disabled: isNew.value },
])
const onMenu = (key: string) => {
  if (key === "duplicate" && current.value) startNew(current.value)
  if (key === "delete") confirmDeleteProfile.value = true
}
const toggleRun = (id: string) => {
  openRunId.value = openRunId.value === id ? "" : id
  if (openRunId.value) refreshLog(id)
}
const openLatestRun = (id: string) => {
  tab.value = "runs"
  openRunId.value = id
  refreshLog(id)
}

const jump = (id: string) => document.getElementById(id)?.scrollIntoView({ behavior: "smooth", block: "start" })
const toneClass: Record<Tone, string> = {
  ok: "text-emerald-600 dark:text-emerald-400",
  warn: "text-amber-600 dark:text-amber-400",
  err: "text-red-600 dark:text-red-400",
  run: "text-primary",
  idle: "text-muted-foreground",
}
const dotClass: Record<Tone, string> = {
  ok: "bg-emerald-500",
  warn: "bg-amber-500",
  err: "bg-red-500",
  run: "bg-primary animate-pulse motion-reduce:animate-none",
  idle: "bg-muted-foreground/40",
}
const latestLogLine = (id: string) => cleanLogLine(stripAnsi(logs[id] || "").trim().split("\n").filter(Boolean).pop() || "")
const currentStep = (job: any) => tool.steps(payload.value, job, logs[job.id] || "", now.value).find((s) => s.state === "now")?.label || "—"
const tabs = computed(() => [
  { key: "overview" as Tab, label: "Overview" },
  { key: "config" as Tab, label: "Configuration" },
  { key: "runs" as Tab, label: tool.latestOnly ? "Last run" : "Runs", count: tool.latestOnly ? 0 : profileJobs.value.length },
])
const onTabKey = (e: KeyboardEvent) => {
  const keys = tabs.value.map((t) => t.key)
  const i = keys.indexOf(tab.value)
  const next = e.key === "ArrowRight" ? keys[(i + 1) % keys.length] : e.key === "ArrowLeft" ? keys[(i - 1 + keys.length) % keys.length] : null
  if (next && !(isNew.value && next !== "config")) {
    tab.value = next
    nextTick(() => document.getElementById(`tw-tab-${next}`)?.focus())
  }
}
const slotProps = computed(() => ({
  draft: draft.value,
  payload: payload.value,
  isNew: isNew.value,
  current: current.value,
  checks,
  capability: capability.value,
  savedLogin,
  selectedId: selectedId.value,
}))
</script>

<template>
  <div class="animate-fade-in">
    <header class="mb-5 flex flex-wrap items-end justify-between gap-3">
      <!-- The layout's top bar already names the page. -->
      <p class="text-sm text-muted-foreground">{{ tool.intro }}</p>
      <p v-if="capability && tool.capabilityLine" class="text-xs tabular-nums text-muted-foreground">{{ tool.capabilityLine(capability) }}</p>
    </header>

    <p v-if="serviceError" class="alert-warning mb-4 text-sm">
      {{ serviceError }}. Runs need the {{ tool.title }} service: check that its container is running and that the orchestrator's
      <code class="font-mono text-xs">ORCH_{{ tool.key.toUpperCase() }}_URL</code> points at it.
    </p>

    <AppStateBlock v-if="loading" kind="loading" message="Loading transfers…" />
    <AppStateBlock v-else-if="loadError" kind="error" :message="loadError" />

    <div v-else-if="!profiles.length && !isNew" class="mx-auto max-w-lg rounded-xl border border-dashed border-border px-6 py-12 text-center">
      <component :is="tool.icon" class="mx-auto h-8 w-8 text-muted-foreground" :stroke-width="1.5" />
      <h2 class="mt-3 text-base font-semibold text-foreground">No {{ tool.title }} transfers yet</h2>
      <p class="mt-1 text-sm text-muted-foreground">{{ tool.emptyText }}</p>
      <AppButton variant="primary" class="mt-5" @click="startNew()"><Plus class="h-4 w-4" :stroke-width="2" />New transfer</AppButton>
    </div>

    <div v-else class="flex flex-col gap-6 lg:flex-row lg:items-start">
      <TransferRail
        :items="railItems"
        :selected="selectedId"
        purpose="Saved transfers and their last result."
        new-label="New"
        class="lg:sticky lg:top-0 lg:max-h-[calc(100vh-8rem)]"
        @select="select"
        @new="startNew()"
      />

      <main class="min-w-0 flex-1 space-y-5">
        <div class="flex flex-wrap items-start justify-between gap-3">
          <div class="min-w-0">
            <h2 class="truncate text-base font-semibold text-foreground">{{ draft.name || "New transfer" }}</h2>
            <div class="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
              <span>{{ tool.modeLabel(payload) }}</span>
              <span v-if="lastJob" class="inline-flex items-center gap-1.5">
                <span class="h-1.5 w-1.5 rounded-full" :class="dotClass[result(lastJob).tone]" aria-hidden="true" />
                {{ isActive(lastJob.status) ? result(lastJob).text : `Last run: ${railLine(lastJob, result, now).text.toLowerCase()}` }}
              </span>
              <span v-else-if="!isNew">Never run</span>
            </div>
          </div>
          <div class="flex shrink-0 items-center gap-2">
            <AppButton v-if="activeJob" :variant="tool.continuous ? 'primary' : 'secondary'" @click="stopRun(activeJob)">
              <Square class="h-4 w-4" :stroke-width="1.75" />{{ stopVerb }}
            </AppButton>
            <AppButton v-if="!activeJob && tool.preview" variant="secondary" :disabled="!!runBlockedReason || starting" :title="runBlockedReason" @click="runPreview">
              <Eye class="h-4 w-4" :stroke-width="1.75" />{{ tool.preview.label }}
            </AppButton>
            <AppButton v-if="!activeJob" variant="primary" :disabled="!!runBlockedReason || starting" :title="runBlockedReason" @click="requestRun">
              <Play class="h-4 w-4" :stroke-width="1.75" />{{ starting ? "Starting…" : verb }}
            </AppButton>
            <AppActionMenu label="More" :items="menuItems" @select="onMenu" />
          </div>
        </div>

        <TransferRoute :source="routeSource" :steps="steps" :destinations="routeDestinations" :live="!!activeJob && !dirty" />

        <div class="flex gap-1 border-b border-border" role="tablist" aria-label="Transfer views" @keydown="onTabKey">
          <button
            v-for="t in tabs"
            :id="`tw-tab-${t.key}`"
            :key="t.key"
            type="button"
            role="tab"
            :aria-selected="tab === t.key"
            :tabindex="tab === t.key ? 0 : -1"
            :disabled="isNew && t.key !== 'config'"
            class="-mb-px border-b-2 px-3 py-2 text-sm transition-colors focus:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-40"
            :class="tab === t.key ? 'border-primary font-medium text-foreground' : 'border-transparent text-muted-foreground hover:text-foreground'"
            @click="tab = t.key"
          >
            {{ t.label }}<span v-if="t.count" class="ml-1.5 rounded-full bg-muted px-1.5 py-0.5 text-[11px] tabular-nums text-muted-foreground">{{ t.count }}</span>
            <span v-if="t.key === 'config' && dirty" class="ml-1.5 inline-block h-1.5 w-1.5 rounded-full bg-amber-500 align-middle" title="Unsaved changes" />
          </button>
        </div>

        <!-- ── Overview ─────────────────────────────────────────────── -->
        <section v-if="tab === 'overview'" role="tabpanel" aria-labelledby="tw-tab-overview" class="max-w-4xl space-y-6">
          <div v-if="activeJob" class="space-y-3 rounded-xl border p-4" :class="tool.continuous ? 'border-emerald-500/30 bg-emerald-500/5' : 'border-primary/30 bg-primary/5'">
            <div class="grid grid-cols-2 gap-4 sm:grid-cols-4">
              <div><div class="text-xs text-muted-foreground">Started</div><div class="text-sm font-medium tabular-nums">{{ startedLabel(activeJob.started_at || activeJob.created_at, now) }}</div></div>
              <div><div class="text-xs text-muted-foreground">{{ tool.continuous ? "Running for" : "Elapsed" }}</div><div class="text-sm font-medium tabular-nums">{{ formatDuration(jobDurationMs(activeJob, now)) }}</div></div>
              <div><div class="text-xs text-muted-foreground">Step</div><div class="text-sm font-medium">{{ currentStep(activeJob) }}</div></div>
              <div v-for="f in tool.liveFacts?.(activeJob, logs[activeJob.id] || '') || []" :key="f.label"><div class="text-xs text-muted-foreground">{{ f.label }}</div><div class="text-sm font-medium tabular-nums">{{ f.value }}</div></div>
            </div>
            <div v-if="latestLogLine(activeJob.id)" class="truncate font-mono text-xs text-muted-foreground">{{ latestLogLine(activeJob.id) }}</div>
            <AppButton size="sm" variant="ghost" @click="openLatestRun(activeJob.id)">Open run and log</AppButton>
          </div>

          <div v-else-if="lastJob" class="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-border p-4">
            <div>
              <div class="flex items-center gap-2 text-sm font-medium" :class="toneClass[result(lastJob).tone]">
                <span class="h-2 w-2 rounded-full" :class="dotClass[result(lastJob).tone]" aria-hidden="true" />{{ result(lastJob).text }}
              </div>
              <div class="mt-0.5 text-xs tabular-nums text-muted-foreground">
                {{ startedLabel(lastJob.started_at || lastJob.created_at, now) }}, took {{ formatDuration(jobDurationMs(lastJob)) }}<template v-if="tool.output?.(lastJob)">, {{ tool.output(lastJob) }}</template>
              </div>
            </div>
            <div class="flex gap-2">
              <slot name="last-run-actions" :job="lastJob" />
              <AppButton size="sm" variant="ghost" @click="openLatestRun(lastJob.id)">Details</AppButton>
            </div>
          </div>

          <div v-else class="rounded-xl border border-dashed border-border p-4 text-sm text-muted-foreground">
            Not run yet. Check the connections under Configuration, then use {{ verb }}.
          </div>

          <div>
            <h3 class="mb-3 text-sm font-semibold text-foreground">What this transfer does</h3>
            <dl class="grid gap-x-6 gap-y-3 text-sm sm:grid-cols-[9rem_minmax(0,1fr)]">
              <template v-for="row in summary" :key="row.label">
                <dt class="text-muted-foreground">{{ row.label }}</dt>
                <dd class="text-foreground">{{ row.value }}</dd>
              </template>
              <template v-if="lastSuccess && lastSuccess !== lastJob">
                <dt class="text-muted-foreground">Last success</dt>
                <dd class="tabular-nums">{{ startedLabel(lastSuccess.finished_at, now) }}, took {{ formatDuration(jobDurationMs(lastSuccess)) }}</dd>
              </template>
            </dl>
          </div>
        </section>

        <!-- ── Configuration ────────────────────────────────────────── -->
        <section v-else-if="tab === 'config'" role="tabpanel" aria-labelledby="tw-tab-config" class="flex gap-8">
          <div class="min-w-0 max-w-4xl flex-1">
            <TransferSection id="tw-general" title="Transfer" why="The name appears in the list, run history and audit log.">
              <label class="block max-w-md">
                <span class="auth-label !mb-1.5">Name</span>
                <input id="tw-name" v-model="draft.name" class="auth-input" :placeholder="tool.namePlaceholder" />
              </label>
              <slot name="general" v-bind="slotProps" />
            </TransferSection>
            <slot name="config" v-bind="slotProps" />
            <TransferSaveBar :changes="changes" :saving="saving" :is-new="isNew" @save="save" @discard="discard" />
          </div>
          <nav class="sticky top-0 hidden w-44 shrink-0 self-start xl:block" aria-label="Sections">
            <ul class="space-y-1 border-l border-border text-sm">
              <li v-for="s in [{ id: 'tw-general', label: 'Transfer' }, ...sections]" :key="s.id">
                <button type="button" class="-ml-px border-l-2 border-transparent px-3 py-1 text-left text-muted-foreground hover:border-border hover:text-foreground" @click="jump(s.id)">
                  {{ s.label }}
                </button>
              </li>
            </ul>
          </nav>
        </section>

        <!-- ── Runs ─────────────────────────────────────────────────── -->
        <section v-else role="tabpanel" aria-labelledby="tw-tab-runs" class="space-y-3">
          <p v-if="tool.latestOnly && profileJobs.length" class="text-xs text-muted-foreground">
            Only the latest run is kept. Starting a new run replaces it and its log.
          </p>
          <p v-if="!profileJobs.length" class="rounded-xl border border-dashed border-border p-4 text-sm text-muted-foreground">
            No runs yet. {{ verb }} starts one; its steps and log appear here.
            <template v-if="retentionHours"> Runs and their files are kept for {{ retentionHours }} hours.</template>
          </p>
          <div v-else class="overflow-hidden rounded-xl border border-border">
            <table class="w-full text-sm">
              <thead class="bg-muted/50 text-left text-xs text-muted-foreground">
                <tr>
                  <th class="px-4 py-2 font-medium">Result</th>
                  <th class="px-4 py-2 font-medium">Started</th>
                  <th class="hidden px-4 py-2 font-medium sm:table-cell">Duration</th>
                  <th v-if="tool.output" class="hidden px-4 py-2 font-medium md:table-cell">Output</th>
                  <th class="w-10 px-2 py-2"><span class="sr-only">Expand</span></th>
                </tr>
              </thead>
              <tbody>
                <template v-for="j in profileJobs" :key="j.id">
                  <tr
                    class="cursor-pointer border-t border-border tabular-nums hover:bg-muted/40"
                    :class="openRunId === j.id ? 'bg-muted/40' : ''"
                    tabindex="0"
                    :aria-expanded="openRunId === j.id"
                    @click="toggleRun(j.id)"
                    @keydown.enter.prevent="toggleRun(j.id)"
                  >
                    <td class="px-4 py-2.5">
                      <span class="inline-flex items-center gap-2 font-medium" :class="toneClass[result(j).tone]">
                        <span class="h-2 w-2 rounded-full" :class="dotClass[result(j).tone]" aria-hidden="true" />{{ result(j).text }}
                      </span>
                    </td>
                    <td class="px-4 py-2.5">{{ startedLabel(j.started_at || j.created_at, now) }}</td>
                    <td class="hidden px-4 py-2.5 sm:table-cell">{{ formatDuration(jobDurationMs(j, now)) }}</td>
                    <td v-if="tool.output" class="hidden px-4 py-2.5 md:table-cell">{{ tool.output(j) || "—" }}</td>
                    <td class="px-2 py-2.5 text-muted-foreground">
                      <ChevronDown class="h-4 w-4 transition-transform" :class="openRunId === j.id ? 'rotate-180' : ''" :stroke-width="1.75" />
                    </td>
                  </tr>
                  <tr v-if="openRunId === j.id" class="border-t border-border">
                    <td :colspan="tool.output ? 5 : 4" class="bg-card px-4 py-4">
                      <TransferRunDetail
                        :job="j"
                        :steps="tool.steps(payload, j, logs[j.id] || '', now)"
                        :problems="tool.problems(j, logs[j.id] || '')"
                        :log="logs[j.id] ?? ''"
                        :continuous="tool.continuous"
                        :stop-verb="stopVerb"
                        :can-delete="!!tool.api.deleteJob"
                        :now="now"
                        @stop="stopRun(j)"
                        @delete="runToDelete = j"
                      >
                        <slot name="run-extra" :job="j" v-bind="slotProps" />
                      </TransferRunDetail>
                    </td>
                  </tr>
                </template>
              </tbody>
            </table>
          </div>
        </section>
      </main>
    </div>

    <AppConfirmDialog :open="confirmRun" :title="`${verb}?`" :confirm-label="verb" danger :loading="starting" @update:open="confirmRun = $event" @confirm="run">
      <p>This changes another database:</p>
      <ul class="mt-2 list-disc space-y-1 pl-5 text-foreground">
        <li v-for="e in effects" :key="e">{{ e }}</li>
      </ul>
    </AppConfirmDialog>
    <AppConfirmDialog :open="!!pendingNav" title="Discard unsaved changes?" confirm-label="Discard changes" danger @update:open="pendingNav = null" @confirm="confirmDiscard">
      Your changes to {{ changes.join(", ") }} will be lost.
    </AppConfirmDialog>
    <AppConfirmDialog :open="!!runToDelete" title="Delete this run?" confirm-label="Delete run" danger :loading="deleting" @update:open="runToDelete = null" @confirm="deleteRun">
      The run record, its log and any file it left on the tool's volume are deleted.
    </AppConfirmDialog>
    <AppConfirmDialog
      :open="confirmDeleteProfile"
      :title="`Delete ${current?.name || 'this transfer'}?`"
      confirm-label="Delete transfer"
      danger
      :loading="deleting"
      @update:open="confirmDeleteProfile = $event"
      @confirm="deleteProfile"
    >
      The saved settings and credentials are deleted. Past runs stay until they expire or you delete them.
    </AppConfirmDialog>
  </div>
</template>
