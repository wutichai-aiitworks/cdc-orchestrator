<script setup lang="ts">
// A profile's audit trail (ADR 0009): every change, run start/refusal/stop/finish and connection
// test, newest first, grouped by day. Kept by the orchestrator beyond the sidecar's run retention,
// so a problem can be traced back to when it started and what changed just before.
import { ChevronRight, Download, RefreshCw, Search } from "@lucide/vue"
import { formatApiError } from "~/utils/apiError"
import {
  changeRows,
  clockTime,
  eventTone,
  FILTERS,
  fullTime,
  groupByDay,
  hasDetail,
  kindLabel,
  matchesFilter,
  matchesSearch,
  runFacts,
  toCsv,
  type TimelineFilter,
  type ToolEvent,
} from "~/utils/toolTimeline"
import { relativeTime, type Tone } from "~/utils/transferView"

const props = defineProps<{ tool: string; profileId: string; profileName?: string; retainedRunIds: string[] }>()
const emit = defineEmits<{ "open-run": [id: string] }>()

const PAGE = 300
const POLL_MS = 15_000
const events = ref<ToolEvent[]>([])
const loading = ref(false)
const loadingMore = ref(false)
const exhausted = ref(false)
const error = ref("")
const persisted = ref(true)
const retentionDays = ref(365)
const fetchedAt = ref<number | null>(null)
const now = ref(Date.now())
const filter = ref<TimelineFilter>("all")
const search = ref("")
const fromDate = ref("")
const toDate = ref("")
const open = reactive<Record<string, boolean>>({})

// Date inputs are local days; the API takes UTC instants.
const since = computed(() => (fromDate.value ? new Date(`${fromDate.value}T00:00:00`).toISOString() : undefined))
const until = computed(() => (toDate.value ? new Date(`${toDate.value}T23:59:59.999`).toISOString() : undefined))

const fetchPage = (before?: string) =>
  $fetch<{ events: ToolEvent[]; persisted: boolean; retention_days: number }>("/api/cdc/tool-events", {
    query: { tool: props.tool, profile_id: props.profileId, limit: PAGE, since: since.value, until: before || until.value },
  })

/** First page (newest). Keeps already loaded older pages when only new events arrived. */
const refresh = async () => {
  if (!props.profileId) return
  loading.value = !events.value.length
  try {
    const r = await fetchPage()
    persisted.value = r.persisted
    retentionDays.value = r.retention_days
    const ids = new Set(r.events.map((e) => e.id))
    const older = events.value.filter((e) => !ids.has(e.id) && r.events.length && e.timestamp < r.events[r.events.length - 1].timestamp)
    events.value = [...r.events, ...older]
    // With older pages already loaded, whether more exist was decided by loadOlder.
    if (!older.length) exhausted.value = r.events.length < PAGE
    fetchedAt.value = Date.now()
    error.value = ""
  } catch (e: any) {
    error.value = formatApiError(e, "Could not load the timeline")
  } finally {
    loading.value = false
  }
}

const loadOlder = async () => {
  const last = events.value[events.value.length - 1]
  if (!last) return
  loadingMore.value = true
  try {
    // `until` is inclusive: step back 1 ms so the last event is not fetched again.
    const before = new Date(new Date(last.timestamp).getTime() - 1).toISOString()
    const r = await fetchPage(before)
    const ids = new Set(events.value.map((e) => e.id))
    events.value = [...events.value, ...r.events.filter((e) => !ids.has(e.id))]
    exhausted.value = r.events.length < PAGE
  } catch (e: any) {
    error.value = formatApiError(e, "Could not load older events")
  } finally {
    loadingMore.value = false
  }
}

const restart = () => {
  events.value = []
  exhausted.value = false
  refresh()
}
watch(() => [props.tool, props.profileId, fromDate.value, toDate.value], restart)

let timer: ReturnType<typeof setInterval> | null = null
onMounted(() => {
  refresh()
  timer = setInterval(() => {
    now.value = Date.now()
    if (document.visibilityState === "visible") refresh()
  }, POLL_MS)
})
onBeforeUnmount(() => timer && clearInterval(timer))

const counts = computed(() => Object.fromEntries(FILTERS.map((f) => [f.key, events.value.filter((e) => matchesFilter(e, f.key)).length])))
const shown = computed(() => events.value.filter((e) => matchesFilter(e, filter.value) && matchesSearch(e, search.value)))
const groups = computed(() => groupByDay(shown.value, new Date(now.value)))
const retained = computed(() => new Set(props.retainedRunIds))

const DOT: Record<Tone, string> = {
  ok: "bg-emerald-500",
  err: "bg-red-500",
  warn: "bg-amber-500",
  run: "bg-primary",
  idle: "bg-zinc-400 dark:bg-zinc-500",
}
const STATUS_TEXT: Record<Tone, string> = {
  ok: "text-emerald-600 dark:text-emerald-400",
  err: "text-red-600 dark:text-red-400",
  warn: "text-amber-600 dark:text-amber-400",
  run: "text-primary",
  idle: "text-muted-foreground",
}

const exportCsv = () => {
  const blob = new Blob([toCsv(shown.value)], { type: "text/csv;charset=utf-8" })
  const a = document.createElement("a")
  const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")
  a.href = URL.createObjectURL(blob)
  a.download = `${props.tool}-${(props.profileName || props.profileId).replace(/[^\w.-]+/g, "_")}-timeline-${stamp}.csv`
  a.click()
  URL.revokeObjectURL(a.href)
}
const clearRange = () => {
  fromDate.value = ""
  toDate.value = ""
}
</script>

<template>
  <div class="space-y-4">
    <!-- Filters -->
    <div class="flex flex-wrap items-center gap-2">
      <div class="flex flex-wrap gap-1 rounded-lg border border-border p-0.5" role="radiogroup" aria-label="Show">
        <button
          v-for="f in FILTERS"
          :key="f.key"
          type="button"
          role="radio"
          :aria-checked="filter === f.key"
          class="rounded-md px-2.5 py-1 text-xs font-medium transition-colors"
          :class="filter === f.key ? 'bg-muted text-foreground' : 'text-muted-foreground hover:text-foreground'"
          @click="filter = f.key"
        >
          {{ f.label }}<span class="ml-1 tabular-nums text-muted-foreground">{{ counts[f.key] }}</span>
        </button>
      </div>
      <label class="relative min-w-[12rem] flex-1">
        <Search class="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" :stroke-width="1.75" />
        <input v-model="search" type="search" class="auth-input w-full !py-1.5 !pl-8 text-sm" placeholder="Search summary, user, job, error" aria-label="Search the timeline" />
      </label>
      <div class="flex items-center gap-1.5 text-xs text-muted-foreground">
        <input v-model="fromDate" type="date" class="auth-input !w-auto !py-1 text-xs" aria-label="From date" :max="toDate || undefined" />
        <span>to</span>
        <input v-model="toDate" type="date" class="auth-input !w-auto !py-1 text-xs" aria-label="To date" :min="fromDate || undefined" />
        <button v-if="fromDate || toDate" type="button" class="text-xs text-primary hover:underline" @click="clearRange">Clear</button>
      </div>
      <div class="ml-auto flex items-center gap-1">
        <AppButton size="sm" variant="ghost" :disabled="!shown.length" title="Download the events shown as CSV" @click="exportCsv">
          <Download class="h-3.5 w-3.5" :stroke-width="1.75" />CSV
        </AppButton>
        <AppButton size="sm" variant="ghost" title="Refresh" @click="refresh">
          <RefreshCw class="h-3.5 w-3.5" :class="loading ? 'animate-spin' : ''" :stroke-width="1.75" />
        </AppButton>
      </div>
    </div>

    <p class="text-xs text-muted-foreground">
      <template v-if="persisted">Kept for {{ retentionDays }} days, including runs the tool no longer lists.</template>
      <span v-else class="text-amber-600 dark:text-amber-400">Not saved: the orchestrator has no database (rqlite), so this history is lost when it restarts.</span>
      <template v-if="fetchedAt"> Updated {{ relativeTime(new Date(fetchedAt).toISOString(), now) || "just now" }}.</template>
    </p>

    <AppStateBlock v-if="error" kind="error" :message="error" />
    <AppStateBlock v-else-if="loading" kind="loading" message="Loading the timeline…" />
    <p v-else-if="!events.length" class="rounded-xl border border-dashed border-border p-4 text-sm text-muted-foreground">
      Nothing recorded yet{{ fromDate || toDate ? " in this date range" : "" }}. Changes, runs and connection tests appear here as they happen.
    </p>
    <p v-else-if="!shown.length" class="rounded-xl border border-dashed border-border p-4 text-sm text-muted-foreground">No events match the filter.</p>

    <!-- Day groups -->
    <section v-for="g in groups" :key="g.day" :aria-label="g.label">
      <h3 class="sticky top-0 z-[1] mb-1 bg-background py-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">{{ g.label }}</h3>
      <ol class="relative ml-[4.75rem] border-l border-border">
        <li v-for="e in g.events" :key="e.id" class="relative pb-1 pl-5">
          <time class="absolute -left-[4.75rem] top-2 w-16 text-right text-xs tabular-nums text-muted-foreground" :datetime="e.timestamp" :title="fullTime(e.timestamp)">
            {{ clockTime(e.timestamp) }}
          </time>
          <span class="absolute -left-[5px] top-[0.7rem] h-2.5 w-2.5 rounded-full ring-4 ring-background" :class="DOT[eventTone(e)]" aria-hidden="true" />

          <div class="rounded-lg px-2 py-1.5" :class="hasDetail(e) ? 'hover:bg-muted' : ''">
            <button
              v-if="hasDetail(e)"
              type="button"
              class="flex w-full items-start gap-1.5 text-left"
              :aria-expanded="!!open[e.id]"
              @click="open[e.id] = !open[e.id]"
            >
              <ChevronRight class="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform" :class="open[e.id] ? 'rotate-90' : ''" :stroke-width="2" />
              <span class="min-w-0 text-sm font-medium text-foreground" :class="eventTone(e) === 'err' ? 'text-red-600 dark:text-red-400' : ''">{{ e.summary }}</span>
            </button>
            <p v-else class="pl-5 text-sm font-medium text-foreground">{{ e.summary }}</p>

            <div class="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 pl-5 text-[11px] text-muted-foreground">
              <span class="font-medium" :class="STATUS_TEXT[eventTone(e)]">{{ kindLabel(e.kind) }}<template v-if="e.status && e.kind !== 'run.started'"> · {{ e.status }}</template></span>
              <span>by {{ e.actor }}</span>
              <span v-if="e.job_id" class="font-mono" :title="e.job_id">run {{ e.job_id.slice(0, 8) }}</span>
              <button v-if="e.job_id && retained.has(e.job_id)" type="button" class="text-primary hover:underline" @click="emit('open-run', e.job_id)">Open run and log</button>
            </div>

            <!-- Detail -->
            <div v-if="open[e.id]" class="mt-2 ml-5 space-y-2 rounded-lg border border-border bg-card p-3 text-xs">
              <table v-if="e.kind === 'profile.updated'" class="w-full">
                <thead>
                  <tr class="text-left text-muted-foreground">
                    <th class="pb-1 pr-3 font-medium">Setting</th>
                    <th class="pb-1 pr-3 font-medium">Before</th>
                    <th class="pb-1 font-medium">After</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="c in changeRows(e)" :key="c.field" class="border-t border-border align-top">
                    <td class="py-1 pr-3 font-mono text-foreground">{{ c.field }}</td>
                    <td class="py-1 pr-3 font-mono" :class="c.secret ? 'italic text-muted-foreground' : 'text-red-600/90 line-through decoration-red-600/40 dark:text-red-400/90'">{{ c.before }}</td>
                    <td class="py-1 font-mono" :class="c.secret ? 'italic text-muted-foreground' : 'text-emerald-700 dark:text-emerald-400'">{{ c.after }}</td>
                  </tr>
                </tbody>
              </table>

              <template v-else-if="e.kind === 'run.finished'">
                <dl class="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1">
                  <template v-for="r in runFacts(e)" :key="r.label">
                    <dt class="capitalize text-muted-foreground">{{ r.label }}</dt>
                    <dd class="tabular-nums text-foreground">{{ r.value }}</dd>
                  </template>
                </dl>
                <pre v-if="e.detail.error || e.detail.reason" class="whitespace-pre-wrap break-words rounded-md bg-red-500/5 p-2 font-mono text-[11px] text-red-700 dark:text-red-300">{{ e.detail.error || e.detail.reason }}</pre>
                <div v-if="e.detail.log_tail">
                  <p class="mb-1 font-medium text-muted-foreground">Last log lines, saved when the run ended</p>
                  <pre class="max-h-64 overflow-auto whitespace-pre rounded-md border border-border bg-muted p-2 font-mono text-[11px] leading-relaxed text-foreground">{{ e.detail.log_tail }}</pre>
                </div>
                <p v-if="e.job_id && !retained.has(e.job_id)" class="text-muted-foreground">The tool no longer keeps this run's full log; this record is what remains.</p>
              </template>

              <template v-else-if="e.kind === 'connection.tested'">
                <p v-if="e.detail.target" class="font-mono text-foreground">{{ e.detail.target }}</p>
                <pre v-if="e.detail.error" class="whitespace-pre-wrap break-words rounded-md bg-red-500/5 p-2 font-mono text-[11px] text-red-700 dark:text-red-300">{{ e.detail.error }}</pre>
                <p v-if="e.detail.hint" class="text-foreground"><span class="font-medium">Hint:</span> {{ e.detail.hint }}</p>
                <p v-if="e.detail.warning" class="text-amber-600 dark:text-amber-400">{{ e.detail.warning }}</p>
                <p v-if="e.detail.facts?.length" class="text-muted-foreground">{{ e.detail.facts.join(" · ") }}</p>
              </template>

              <template v-else-if="e.kind === 'run.start_failed'">
                <pre class="whitespace-pre-wrap break-words rounded-md bg-red-500/5 p-2 font-mono text-[11px] text-red-700 dark:text-red-300">{{ e.detail.error }}</pre>
              </template>

              <pre v-else class="max-h-72 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] text-muted-foreground">{{ JSON.stringify(e.detail.profile ?? e.detail, null, 2) }}</pre>
            </div>
          </div>
        </li>
      </ol>
    </section>

    <div v-if="events.length && !exhausted" class="flex justify-center">
      <AppButton size="sm" variant="ghost" :disabled="loadingMore" @click="loadOlder">{{ loadingMore ? "Loading…" : "Load older events" }}</AppButton>
    </div>
  </div>
</template>
