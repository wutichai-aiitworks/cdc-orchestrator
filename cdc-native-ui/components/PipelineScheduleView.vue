<script setup lang="ts">
/**
 * Schedule view: every pipeline in the order its worker runs next, with the poll
 * interval, a live countdown and the clock time of the next run, plus the next
 * scheduled full refresh. Data comes from GET /cdc/schedule every 5 s; between
 * refreshes the countdown ticks locally and a poll tick that has passed is rolled
 * forward by its interval (the connector keeps polling on that cadence).
 */
import { CalendarClock, RefreshCw, Timer } from "@lucide/vue"
import type { ScheduleEntry, ScheduleResponse } from "~/composables/useProfilesM3"
import { formatApiError } from "~/utils/apiError"

const props = defineProps<{
  /** Pipelines the page filters (search, status, tag) leave visible; null shows all. */
  visibleIds: Set<string> | null
  selectedId?: string
  /** Rows-per-page choices; paging runs over the next-run order, not the page's sort. */
  pageSizes?: number[]
}>()

const page = defineModel<number>("page", { default: 1 })
const pageSize = defineModel<number>("pageSize", { default: 24 })

const emit = defineEmits<{ select: [id: string] }>()

const { fetchSchedule } = useProfilesM3()

const data = ref<ScheduleResponse | null>(null)
const error = ref("")
const loading = ref(false)
const now = ref(Date.now())
let pollTimer: ReturnType<typeof setInterval> | undefined
let clockTimer: ReturnType<typeof setInterval> | undefined

const load = async () => {
  loading.value = true
  try {
    data.value = await fetchSchedule()
    error.value = ""
  } catch (e: unknown) {
    error.value = formatApiError(e, "Could not load the schedule")
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  void load()
  pollTimer = setInterval(() => void load(), 5000)
  clockTimer = setInterval(() => (now.value = Date.now()), 1000)
})
onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer)
  if (clockTimer) clearInterval(clockTimer)
})

const ms = (iso?: string | null) => (iso ? Date.parse(iso) : Number.NaN)

/** Next poll tick, rolled forward past `now` by whole intervals. */
const projectedTick = (e: ScheduleEntry) => {
  let next = ms(e.next_tick_at)
  const step = e.interval_ms ?? 0
  if (!Number.isFinite(next)) return Number.NaN
  if (step > 0 && next < now.value) next += Math.ceil((now.value - next) / step) * step
  return next
}

const nextRun = (e: ScheduleEntry) => {
  const candidates = [projectedTick(e), ms(e.next_full_refresh_at)].filter(Number.isFinite)
  return candidates.length ? Math.min(...candidates) : Number.NaN
}

const nextIsRefresh = (e: ScheduleEntry) => {
  const refresh = ms(e.next_full_refresh_at)
  return Number.isFinite(refresh) && refresh === nextRun(e)
}

const kindRank: Record<string, number> = { poll: 3, stream: 1, once: 2, idle: 4 }

const rows = computed(() => {
  const entries = (data.value?.entries ?? []).filter((e) => !props.visibleIds || props.visibleIds.has(e.profile_id))
  return [...entries].sort((a, b) => {
    const na = nextRun(a)
    const nb = nextRun(b)
    const ra = Number.isFinite(na) ? 0 : kindRank[a.kind] ?? 5
    const rb = Number.isFinite(nb) ? 0 : kindRank[b.kind] ?? 5
    if (ra !== rb) return ra - rb
    if (ra === 0 && na !== nb) return na - nb
    return a.name.localeCompare(b.name, undefined, { sensitivity: "base", numeric: true })
  })
})

const pagedRows = computed(() => {
  const start = (page.value - 1) * pageSize.value
  return rows.value.slice(start, start + pageSize.value)
})
const rowOffset = computed(() => (page.value - 1) * pageSize.value)

const counts = computed(() => {
  const c = { poll: 0, stream: 0, once: 0, idle: 0 }
  for (const e of rows.value) c[e.kind] += 1
  return c
})

const countdown = (target: number) => {
  if (!Number.isFinite(target)) return ""
  const s = Math.round((target - now.value) / 1000)
  if (s <= 0) return "due now"
  if (s < 60) return `in ${s}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `in ${m}m ${String(s % 60).padStart(2, "0")}s`
  const h = Math.floor(m / 60)
  if (h < 48) return `in ${h}h ${String(m % 60).padStart(2, "0")}m`
  return `in ${Math.floor(h / 24)}d`
}

const clock = (target: number) => {
  if (!Number.isFinite(target)) return ""
  const d = new Date(target)
  const sameDay = new Date(now.value).toDateString() === d.toDateString()
  const time = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false })
  return sameDay ? time : `${d.toLocaleDateString(undefined, { day: "numeric", month: "short" })} ${time}`
}

const ago = (iso?: string | null) => {
  const t = ms(iso)
  if (!Number.isFinite(t)) return ""
  const s = Math.max(0, Math.round((now.value - t) / 1000))
  if (s < 60) return `${s}s ago`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m ago`
  const h = Math.floor(m / 60)
  return h < 48 ? `${h}h ago` : `${Math.floor(h / 24)}d ago`
}

const interval = (e: ScheduleEntry) => {
  const v = e.interval_ms
  if (!v) return ""
  if (v < 60_000) return `every ${Math.round(v / 1000)}s`
  if (v < 3_600_000) return `every ${+(v / 60_000).toFixed(1)}m`
  return `every ${+(v / 3_600_000).toFixed(1)}h`
}

const kindLabel: Record<string, string> = {
  poll: "Poll",
  stream: "Streaming",
  once: "One-time",
  idle: "Not running",
}

const captureLabel: Record<string, string> = {
  incremental: "Incremental",
  trigger: "Trigger",
  logical: "Logical",
  sync_once: "Sync once",
}

/** Why a row has no next time, in a few words. */
const noRunReason = (e: ScheduleEntry) => {
  if (e.kind === "stream") return "Continuous, runs as changes arrive"
  if (e.kind === "once") return "Ran once on Start"
  if (e.kind === "poll") return `Timing after first audit sweep (≤ ${data.value?.poll_audit_check_secs ?? 5}s)`
  return e.enabled ? `Nothing scheduled while ${e.status}` : "Disabled"
}

const firstUpId = computed(() => {
  const first = rows.value[0]
  return first && Number.isFinite(nextRun(first)) ? first.profile_id : ""
})
</script>

<template>
  <div>
    <div class="mb-3 flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
      <p>
        {{ counts.poll }} polling · {{ counts.stream }} streaming · {{ counts.once }} one-time · {{ counts.idle }} not running
      </p>
      <p class="flex items-center gap-1.5">
        <RefreshCw class="h-3 w-3" :class="loading ? 'animate-spin' : ''" :stroke-width="1.75" />
        Live, refreshes every 5 s. Poll times follow the connector interval, accurate to
        {{ data?.poll_audit_check_secs ?? 5 }} s.
      </p>
    </div>

    <p v-if="error" class="mb-3 text-xs text-red-500">{{ error }}</p>

    <AppTable>
      <thead>
        <tr>
          <th class="w-10">#</th>
          <th>Pipeline</th>
          <th>Worker</th>
          <th>Next run</th>
          <th>Last poll</th>
          <th>Full refresh</th>
          <th>Status</th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="(e, i) in pagedRows"
          :key="e.profile_id"
          class="cursor-pointer"
          :class="{
            'bg-brand-500/5': selectedId === e.profile_id,
            'opacity-60': e.kind === 'idle',
          }"
          @click="emit('select', e.profile_id)"
        >
          <td class="font-mono text-xs text-muted-foreground">{{ rowOffset + i + 1 }}</td>
          <td>
            <p class="font-medium">{{ e.name }}</p>
            <div v-if="e.tags.length" class="mt-1 flex flex-wrap gap-1">
              <span v-for="tag in e.tags" :key="tag" class="tag-chip">{{ tag }}</span>
            </div>
          </td>
          <td class="whitespace-nowrap">
            <p>{{ kindLabel[e.kind] }} <span class="text-muted-foreground">· {{ captureLabel[e.capture_mode] ?? e.capture_mode }}</span></p>
            <p v-if="e.interval_ms" class="mt-0.5 flex items-center gap-1 text-xs text-muted-foreground">
              <Timer class="h-3 w-3" :stroke-width="1.75" />
              {{ interval(e) }}
              <span v-if="e.interval_source === 'default'" title="Realtime delivery uses the 1 s default poll">(default)</span>
            </p>
          </td>
          <td class="whitespace-nowrap">
            <template v-if="Number.isFinite(nextRun(e))">
              <p class="font-medium tabular-nums" :class="firstUpId === e.profile_id ? 'text-brand-600 dark:text-brand-400' : ''">
                {{ countdown(nextRun(e)) }}
                <span v-if="firstUpId === e.profile_id" class="badge-info ml-1">Next</span>
              </p>
              <p class="mt-0.5 font-mono text-xs text-muted-foreground">
                {{ clock(nextRun(e)) }} · {{ nextIsRefresh(e) ? "full refresh" : "poll" }}
              </p>
            </template>
            <p v-else class="text-xs text-muted-foreground">{{ noRunReason(e) }}</p>
          </td>
          <td class="whitespace-nowrap text-xs text-muted-foreground">
            <template v-if="e.last_tick_at">
              <p class="tabular-nums">{{ ago(e.last_tick_at) }}</p>
              <p class="font-mono">{{ clock(ms(e.last_tick_at)) }}</p>
            </template>
            <span v-else>—</span>
          </td>
          <td class="whitespace-nowrap text-xs">
            <template v-if="e.full_refresh_minutes">
              <p class="flex items-center gap-1">
                <CalendarClock class="h-3 w-3 text-muted-foreground" :stroke-width="1.75" />
                every {{ e.full_refresh_minutes }} min
              </p>
              <p v-if="e.next_full_refresh_at" class="mt-0.5 font-mono text-muted-foreground">
                next {{ clock(ms(e.next_full_refresh_at)) }}
              </p>
              <p v-if="e.last_full_refresh_at" class="mt-0.5 text-muted-foreground">last {{ ago(e.last_full_refresh_at) }}</p>
            </template>
            <span v-else class="text-muted-foreground">—</span>
          </td>
          <td><AppStatusBadge :status="e.status" /></td>
        </tr>
        <tr v-if="!rows.length">
          <td colspan="7">
            <AppStateBlock
              :kind="loading && !data ? 'loading' : 'empty'"
              :message="loading && !data ? 'Loading schedule…' : 'No pipelines match these filters.'"
            />
          </td>
        </tr>
      </tbody>
    </AppTable>

    <AppListPager
      v-model:page="page"
      v-model:page-size="pageSize"
      :total="rows.length"
      :sizes="pageSizes"
      noun="pipelines"
    />
  </div>
</template>
