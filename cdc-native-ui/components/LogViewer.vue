<script setup lang="ts">
/**
 * Live tail of one service's container logs (SSE from /api/cdc/logs/{service}/stream).
 *
 * Built for reading while the feed runs: scrolling up (or opening a line) pauses the
 * view and holds new lines back until you resume, so nothing moves under the cursor.
 * Lines are parsed into time / level / source / message (utils/logLine.ts); stack-trace
 * lines stay under their event. Incoming lines are batched once per animation frame.
 *
 * Keys (outside inputs): / search · Space pause/resume · G latest · n / N next/previous
 * error · w wrap · f fullscreen · Esc clear search or close open lines.
 */
import {
  ArrowDown,
  ChevronDown,
  ChevronUp,
  Copy,
  Download,
  Filter,
  Maximize2,
  Minimize2,
  Pause,
  Play,
  RefreshCw,
  Regex,
  Search,
  WrapText,
  X,
} from "@lucide/vue"
import { useEventListener, useFullscreen, useIntervalFn, useLocalStorage } from "@vueuse/core"
import { parseLogLine, type LogLevel, type ParsedLogLine } from "~/utils/logLine"

const props = defineProps<{
  service: string
  /** Container name, shown in the status bar. */
  container?: string
}>()

const toast = useToast()

type LevelFilter = "all" | "warn" | "error"
type Row = ParsedLogLine & { id: number; raw: string }
type Conn = "connecting" | "live" | "reconnecting" | "closed"

const HISTORY_OPTIONS = [500, 1000, 2000, 5000]

const prefs = useLocalStorage(
  "cdc.logs.viewer",
  { history: 1000, level: "all" as LevelFilter, wrap: false, utc: false },
  { mergeDefaults: true },
)

// ── Buffer ──────────────────────────────────────────────────────────────────

const rows = shallowRef<Row[]>([])
/** Lines that arrived while paused; shown on resume. */
const held = shallowRef<Row[]>([])
let queue: string[] = []
let frame = 0
let seq = 0
let last: ParsedLogLine | undefined
/** After a reconnect the server resends its backlog; drop lines already on screen. */
let dedupe: Set<string> | null = null

const paused = ref(false)
const pauseReason = ref<"user" | "scroll" | "inspect">("user")

const flush = () => {
  frame = 0
  const batch = queue
  queue = []
  const parsed: Row[] = []
  for (const raw of batch) {
    if (dedupe) {
      if (dedupe.has(raw)) continue
      dedupe = null
    }
    const p = parseLogLine(raw, last)
    last = p
    parsed.push({ ...p, raw, id: ++seq })
  }
  arrivals += parsed.length
  if (!parsed.length) return
  const cap = prefs.value.history
  if (paused.value) {
    held.value = held.value.concat(parsed).slice(-cap)
    return
  }
  rows.value = rows.value.concat(parsed).slice(-cap)
  nextTick(scrollToBottom)
}

const enqueue = (line: string) => {
  queue.push(line)
  if (!frame) frame = requestAnimationFrame(flush)
}

const resetBuffer = () => {
  rows.value = []
  held.value = []
  queue = []
  seq = 0
  last = undefined
  dedupe = null
  expanded.clear()
}

// ── Connection ──────────────────────────────────────────────────────────────

const conn = ref<Conn>("connecting")
let source: EventSource | null = null

const connect = ({ keepBuffer = false } = {}) => {
  source?.close()
  if (keepBuffer) dedupe = new Set([...rows.value, ...held.value].map((r) => r.raw))
  else resetBuffer()
  conn.value = "connecting"
  const es = new EventSource(
    `/api/cdc/logs/${encodeURIComponent(props.service)}/stream?tail=${prefs.value.history}`,
  )
  source = es
  es.onopen = () => {
    conn.value = "live"
  }
  es.onmessage = (event) => {
    if (event.data) enqueue(event.data)
  }
  es.onerror = () => {
    if (es.readyState === EventSource.CLOSED) {
      conn.value = "closed"
      return
    }
    // The browser reconnects by itself; the server will resend its backlog.
    conn.value = "reconnecting"
    dedupe = new Set([...rows.value, ...held.value].map((r) => r.raw))
  }
}

watch(() => props.service, () => connect())
watch(() => prefs.value.history, () => connect())
onMounted(() => connect())
onBeforeUnmount(() => {
  source?.close()
  if (frame) cancelAnimationFrame(frame)
})

// ── Rate ────────────────────────────────────────────────────────────────────

let arrivals = 0
const window5s: number[] = []
const rate = ref(0)
useIntervalFn(() => {
  window5s.push(arrivals)
  arrivals = 0
  if (window5s.length > 5) window5s.shift()
  rate.value = window5s.reduce((a, b) => a + b, 0) / window5s.length
}, 1000)

const rateLabel = computed(() =>
  rate.value >= 10 ? `${Math.round(rate.value)}/s` : rate.value >= 0.1 ? `${rate.value.toFixed(1)}/s` : "idle",
)

// ── Pause / follow ──────────────────────────────────────────────────────────

const viewport = ref<HTMLElement | null>(null)
let programmaticScroll = false

const scrollToBottom = () => {
  const el = viewport.value
  if (!el || paused.value) return
  programmaticScroll = true
  el.scrollTop = el.scrollHeight
}

const pause = (reason: "user" | "scroll" | "inspect") => {
  if (paused.value) return
  paused.value = true
  pauseReason.value = reason
}

const resume = () => {
  if (held.value.length) rows.value = rows.value.concat(held.value).slice(-prefs.value.history)
  held.value = []
  paused.value = false
  nextTick(scrollToBottom)
}

const jumpToLatest = () => {
  expanded.clear()
  resume()
}

const onScroll = () => {
  const el = viewport.value
  if (!el) return
  if (programmaticScroll) {
    programmaticScroll = false
    return
  }
  const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 32
  if (!atBottom) pause("scroll")
  else if (paused.value && pauseReason.value === "scroll") resume()
}

// ── Filters ─────────────────────────────────────────────────────────────────

const search = ref("")
const useRegex = ref(false)
const sourceFilter = ref("")
const searchInput = ref<HTMLInputElement | null>(null)

const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
const pattern = computed<RegExp | null | "invalid">(() => {
  const q = search.value.trim()
  if (!q) return null
  try {
    return new RegExp(useRegex.value ? q : escapeRe(q), "i")
  } catch {
    return "invalid"
  }
})

const levelOk = (level: LogLevel, f: LevelFilter) =>
  f === "all" || (f === "error" ? level === "error" : level === "error" || level === "warn")

const visible = computed(() => {
  const re = pattern.value === "invalid" ? null : pattern.value
  const lf = prefs.value.level
  const src = sourceFilter.value
  return rows.value.filter(
    (r) => levelOk(r.level, lf) && (!src || r.source === src) && (!re || re.test(r.raw)),
  )
})

/** Event counts (stack-trace lines are not separate events). */
const counts = computed(() => {
  let all = 0
  let warn = 0
  let error = 0
  for (const r of rows.value) {
    if (r.continuation) continue
    all++
    if (r.level === "error") error++
    else if (r.level === "warn") warn++
  }
  return { all, warn: warn + error, error }
})

const filtersActive = computed(() => !!search.value.trim() || !!sourceFilter.value || prefs.value.level !== "all")
const clearFilters = () => {
  search.value = ""
  sourceFilter.value = ""
  prefs.value.level = "all"
}

const LEVEL_OPTIONS: { key: LevelFilter; label: string }[] = [
  { key: "all", label: "All" },
  { key: "warn", label: "Warn+" },
  { key: "error", label: "Errors" },
]

// ── Rendering helpers ───────────────────────────────────────────────────────

const pad = (n: number, w = 2) => String(n).padStart(w, "0")
const formatTime = (ms: number) => {
  const d = new Date(ms)
  return prefs.value.utc
    ? `${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}:${pad(d.getUTCSeconds())}.${pad(d.getUTCMilliseconds(), 3)}`
    : `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`
}
const dayKey = (ms: number) => {
  const d = new Date(ms)
  return prefs.value.utc ? d.toISOString().slice(0, 10) : `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}
const formatDay = (ms: number) =>
  new Date(ms).toLocaleDateString(undefined, {
    weekday: "short",
    day: "numeric",
    month: "short",
    year: "numeric",
    timeZone: prefs.value.utc ? "UTC" : undefined,
  })

/** Visible rows with a day divider wherever the date changes. */
const view = computed(() => {
  let lastDay = ""
  return visible.value.map((row) => {
    let day = ""
    if (row.ts !== null) {
      const k = dayKey(row.ts)
      if (k !== lastDay) {
        day = lastDay ? formatDay(row.ts) : ""
        lastDay = k
      }
    }
    return { row, day }
  })
})

/** Message split into plain and matched parts for highlighting. */
const segments = (text: string): { t: string; hit: boolean }[] => {
  const re = pattern.value
  if (!re || re === "invalid") return [{ t: text, hit: false }]
  const g = new RegExp(re.source, "gi")
  const out: { t: string; hit: boolean }[] = []
  let at = 0
  for (const m of text.matchAll(g)) {
    if (!m[0]) break
    if (m.index! > at) out.push({ t: text.slice(at, m.index), hit: false })
    out.push({ t: m[0], hit: true })
    at = m.index! + m[0].length
  }
  if (at < text.length) out.push({ t: text.slice(at), hit: false })
  return out
}

const ROW_CLASS: Record<LogLevel, string> = {
  error: "border-l-red-500 bg-red-500/[0.06]",
  warn: "border-l-amber-500 bg-amber-500/[0.05]",
  info: "border-l-transparent",
  debug: "border-l-transparent text-muted-foreground",
}
const LEVEL_TEXT: Record<LogLevel, string> = {
  error: "text-red-600 dark:text-red-400",
  warn: "text-amber-600 dark:text-amber-400",
  info: "text-sky-600 dark:text-sky-400",
  debug: "text-muted-foreground",
}
const SOURCE_COLORS = [
  "text-violet-600 dark:text-violet-400",
  "text-teal-600 dark:text-teal-400",
  "text-fuchsia-600 dark:text-fuchsia-400",
  "text-indigo-600 dark:text-indigo-400",
  "text-emerald-600 dark:text-emerald-400",
  "text-orange-600 dark:text-orange-400",
]
const sourceColor = (s: string) => {
  let h = 0
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0
  return SOURCE_COLORS[Math.abs(h) % SOURCE_COLORS.length]
}

// ── Inspecting a line ───────────────────────────────────────────────────────

const expanded = reactive(new Set<number>())
const flashId = ref<number | null>(null)

const toggleRow = (row: Row) => {
  // Selecting text inside a line should not toggle it.
  if (window.getSelection()?.toString()) return
  if (expanded.has(row.id)) expanded.delete(row.id)
  else {
    expanded.add(row.id)
    pause("inspect")
  }
}

const prettyJson = (text: string) => {
  const t = text.trim()
  if (!t.startsWith("{") && !t.startsWith("[")) return ""
  try {
    return JSON.stringify(JSON.parse(t), null, 2)
  } catch {
    return ""
  }
}

const copyLine = async (row: Row) => {
  try {
    await navigator.clipboard.writeText(row.raw)
    toast.success("Line copied.")
  } catch {
    toast.error("Clipboard is not available.")
  }
}

const scrollToRow = (id: number) => {
  nextTick(() => {
    const el = viewport.value?.querySelector<HTMLElement>(`[data-id="${id}"]`)
    if (!el) return
    programmaticScroll = true
    el.scrollIntoView({ block: "center" })
    flashId.value = id
    setTimeout(() => {
      if (flashId.value === id) flashId.value = null
    }, 1400)
  })
}

const showInContext = (row: Row) => {
  clearFilters()
  pause("inspect")
  scrollToRow(row.id)
}

// ── Error navigation ────────────────────────────────────────────────────────

const errorIds = computed(() => visible.value.filter((r) => r.level === "error" && !r.continuation).map((r) => r.id))
const errorCursor = ref(-1)
watch(errorIds, (ids) => {
  if (errorCursor.value >= ids.length) errorCursor.value = ids.length - 1
})

const gotoError = (dir: 1 | -1) => {
  const ids = errorIds.value
  if (!ids.length) return
  pause("inspect")
  const next = errorCursor.value < 0 ? (dir === 1 ? 0 : ids.length - 1) : (errorCursor.value + dir + ids.length) % ids.length
  errorCursor.value = next
  scrollToRow(ids[next])
}

// ── Toolbar actions ─────────────────────────────────────────────────────────

const root = ref<HTMLElement | null>(null)
const { isFullscreen, toggle: toggleFullscreen } = useFullscreen(root)

const download = () => {
  const text = visible.value.map((r) => r.raw).join("\n")
  const blob = new Blob([`${text}\n`], { type: "text/plain" })
  const url = URL.createObjectURL(blob)
  const a = document.createElement("a")
  a.href = url
  a.download = `${props.service}-${new Date().toISOString().replace(/[:.]/g, "-")}.log`
  a.click()
  URL.revokeObjectURL(url)
}

const togglePause = () => (paused.value ? resume() : pause("user"))

const statusText = computed(() => {
  if (conn.value === "connecting") return "Connecting…"
  if (conn.value === "reconnecting") return "Reconnecting…"
  if (conn.value === "closed") return "Disconnected"
  if (paused.value) return pauseReason.value === "user" ? "Paused" : "Paused while reading"
  return `Live · ${rateLabel.value}`
})

// ── Keyboard ────────────────────────────────────────────────────────────────

useEventListener(window, "keydown", (e: KeyboardEvent) => {
  if (e.metaKey || e.ctrlKey || e.altKey) return
  const t = e.target as HTMLElement | null
  const typing = !!t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))
  if (typing) {
    if (e.key === "Escape" && t === searchInput.value) {
      search.value = ""
      searchInput.value?.blur()
    }
    return
  }
  switch (e.key) {
    case "/":
      e.preventDefault()
      searchInput.value?.focus()
      break
    case " ":
      e.preventDefault()
      togglePause()
      break
    case "G":
    case "End":
      e.preventDefault()
      jumpToLatest()
      break
    case "n":
      gotoError(1)
      break
    case "N":
      gotoError(-1)
      break
    case "w":
      prefs.value.wrap = !prefs.value.wrap
      break
    case "f":
      toggleFullscreen()
      break
    case "Escape":
      if (expanded.size) expanded.clear()
      else if (filtersActive.value) clearFilters()
      break
  }
})
</script>

<template>
  <section
    ref="root"
    class="flex min-w-0 flex-col overflow-hidden border border-border bg-card text-card-foreground"
    :class="isFullscreen ? 'h-screen' : 'h-[calc(100dvh-13rem)] min-h-[32rem] rounded-xl'"
  >
    <!-- Source + feed controls -->
    <header class="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2">
      <slot name="services" />

      <div class="ml-auto flex flex-wrap items-center gap-1.5">
        <span
          class="inline-flex h-8 items-center gap-2 rounded-full border px-3 text-xs font-medium tabular-nums"
          :class="
            conn === 'closed'
              ? 'border-red-500/30 bg-red-500/10 text-red-700 dark:text-red-400'
              : conn !== 'live' || paused
                ? 'border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-400'
                : 'border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400'
          "
          role="status"
        >
          <span class="relative flex h-2 w-2">
            <span v-if="conn === 'live' && !paused" class="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-500 opacity-60" />
            <span
              class="relative inline-flex h-2 w-2 rounded-full"
              :class="conn === 'closed' ? 'bg-red-500' : conn !== 'live' || paused ? 'bg-amber-500' : 'bg-emerald-500'"
            />
          </span>
          {{ statusText }}
        </span>

        <AppButton v-if="conn === 'closed'" size="sm" @click="connect({ keepBuffer: true })">
          <RefreshCw class="h-3.5 w-3.5" :stroke-width="1.75" />
          Reconnect
        </AppButton>
        <AppButton v-else size="sm" :title="paused ? 'Resume (Space)' : 'Pause (Space)'" @click="togglePause">
          <component :is="paused ? Play : Pause" class="h-3.5 w-3.5" :stroke-width="1.75" />
          {{ paused ? "Resume" : "Pause" }}
        </AppButton>

        <div class="mx-1 h-5 w-px bg-border" aria-hidden="true" />

        <button
          type="button"
          class="log-tool"
          :class="{ 'log-tool-on': prefs.wrap }"
          :aria-pressed="prefs.wrap"
          title="Wrap long lines (w)"
          @click="prefs.wrap = !prefs.wrap"
        >
          <WrapText class="h-4 w-4" :stroke-width="1.75" />
        </button>
        <button
          type="button"
          class="log-tool w-auto px-2 text-[11px] font-semibold tabular-nums"
          :title="prefs.utc ? 'Showing UTC — switch to local time' : 'Showing local time — switch to UTC'"
          @click="prefs.utc = !prefs.utc"
        >
          {{ prefs.utc ? "UTC" : "Local" }}
        </button>
        <button type="button" class="log-tool" title="Download visible lines" :disabled="!visible.length" @click="download">
          <Download class="h-4 w-4" :stroke-width="1.75" />
        </button>
        <button type="button" class="log-tool" :title="isFullscreen ? 'Exit full screen (f)' : 'Full screen (f)'" @click="toggleFullscreen()">
          <component :is="isFullscreen ? Minimize2 : Maximize2" class="h-4 w-4" :stroke-width="1.75" />
        </button>
      </div>
    </header>

    <!-- Filters -->
    <div class="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2">
      <div
        class="flex h-8 min-w-[16rem] flex-1 items-center gap-2 rounded-lg border bg-background px-2.5 focus-within:ring-2 focus-within:ring-ring/40 sm:max-w-md"
        :class="pattern === 'invalid' ? 'border-red-500' : 'border-input'"
      >
        <Search class="h-3.5 w-3.5 shrink-0 text-muted-foreground" :stroke-width="1.75" />
        <input
          ref="searchInput"
          v-model="search"
          type="text"
          spellcheck="false"
          class="h-full min-w-0 flex-1 bg-transparent font-mono text-xs outline-none placeholder:font-body placeholder:text-muted-foreground"
          :placeholder="useRegex ? 'Regular expression…' : 'Search lines…'"
          aria-label="Search log lines"
        />
        <span v-if="search.trim() && pattern !== 'invalid'" class="shrink-0 text-[11px] tabular-nums text-muted-foreground">
          {{ visible.length.toLocaleString() }}
        </span>
        <span v-if="pattern === 'invalid'" class="shrink-0 text-[11px] text-red-600 dark:text-red-400">invalid</span>
        <button v-if="search" type="button" class="text-muted-foreground hover:text-foreground" title="Clear (Esc)" @click="search = ''">
          <X class="h-3.5 w-3.5" />
        </button>
        <kbd v-else class="log-kbd">/</kbd>
        <button
          type="button"
          class="-mr-1 rounded p-1"
          :class="useRegex ? 'bg-primary/15 text-primary' : 'text-muted-foreground hover:text-foreground'"
          :aria-pressed="useRegex"
          title="Use regular expression"
          @click="useRegex = !useRegex"
        >
          <Regex class="h-3.5 w-3.5" :stroke-width="1.75" />
        </button>
      </div>

      <div class="flex h-8 items-center rounded-lg border border-border p-0.5" role="radiogroup" aria-label="Minimum level">
        <button
          v-for="opt in LEVEL_OPTIONS"
          :key="opt.key"
          type="button"
          role="radio"
          :aria-checked="prefs.level === opt.key"
          class="flex h-full items-center gap-1.5 rounded-md px-2.5 text-xs font-medium transition-colors"
          :class="prefs.level === opt.key ? 'bg-muted text-foreground' : 'text-muted-foreground hover:text-foreground'"
          @click="prefs.level = opt.key"
        >
          {{ opt.label }}
          <span
            class="tabular-nums"
            :class="
              opt.key === 'error' && counts.error
                ? 'text-red-600 dark:text-red-400'
                : opt.key === 'warn' && counts.warn
                  ? 'text-amber-600 dark:text-amber-400'
                  : 'text-muted-foreground'
            "
          >{{ counts[opt.key].toLocaleString() }}</span>
        </button>
      </div>

      <div v-if="errorIds.length" class="flex h-8 items-center rounded-lg border border-red-500/30 bg-red-500/5 text-xs text-red-700 dark:text-red-400">
        <button type="button" class="flex h-full items-center px-1.5 hover:bg-red-500/10" title="Previous error (N)" @click="gotoError(-1)">
          <ChevronUp class="h-3.5 w-3.5" />
        </button>
        <span class="px-1 tabular-nums">
          {{ errorCursor >= 0 ? `${errorCursor + 1} / ${errorIds.length}` : `${errorIds.length} error${errorIds.length === 1 ? "" : "s"}` }}
        </span>
        <button type="button" class="flex h-full items-center px-1.5 hover:bg-red-500/10" title="Next error (n)" @click="gotoError(1)">
          <ChevronDown class="h-3.5 w-3.5" />
        </button>
      </div>

      <button
        v-if="sourceFilter"
        type="button"
        class="inline-flex h-8 items-center gap-1.5 rounded-lg border border-border bg-muted px-2.5 font-mono text-xs"
        title="Remove source filter"
        @click="sourceFilter = ''"
      >
        <Filter class="h-3 w-3 text-muted-foreground" />
        <span :class="sourceColor(sourceFilter)">{{ sourceFilter }}</span>
        <X class="h-3 w-3 text-muted-foreground" />
      </button>

      <label class="ml-auto flex items-center gap-2 text-xs text-muted-foreground">
        Keep
        <AppSelect v-model.number="prefs.history" select-class="!h-8 !min-h-8 w-24 text-xs" aria-label="Lines to keep">
          <option v-for="n in HISTORY_OPTIONS" :key="n" :value="n">{{ n.toLocaleString() }}</option>
        </AppSelect>
        lines
      </label>
    </div>

    <!-- Lines -->
    <div class="relative min-h-0 flex-1">
      <div
        ref="viewport"
        class="h-full overflow-y-auto overflow-x-hidden bg-background font-mono text-xs leading-5"
        aria-live="off"
        @scroll.passive="onScroll"
      >
        <div
          class="sticky top-0 z-10 flex border-b border-l-2 border-border border-l-transparent bg-muted/90 py-1 font-body text-[10px] font-semibold uppercase tracking-wider text-muted-foreground backdrop-blur"
        >
          <span class="w-[6.75rem] shrink-0 pl-3">Time {{ prefs.utc ? "(UTC)" : "" }}</span>
          <span class="w-14 shrink-0">Level</span>
          <span class="hidden w-36 shrink-0 sm:block">Source</span>
          <span class="min-w-0 flex-1">Message</span>
        </div>

        <div v-if="!rows.length" class="flex h-[60%] flex-col items-center justify-center gap-2 font-body text-sm text-muted-foreground">
          <RefreshCw v-if="conn !== 'closed'" class="h-4 w-4 animate-spin" :stroke-width="1.75" />
          <span>{{ conn === "closed" ? "Not connected." : conn === "live" ? `Waiting for ${service} to write…` : `Connecting to ${service}…` }}</span>
        </div>
        <div v-else-if="!visible.length" class="flex h-[60%] flex-col items-center justify-center gap-3 font-body text-sm text-muted-foreground">
          <span>No lines match the current filters.</span>
          <AppButton size="sm" @click="clearFilters">Clear filters</AppButton>
        </div>

        <template v-for="{ row, day } in view" v-else :key="row.id">
          <div v-if="day" class="flex items-center gap-3 px-3 py-1.5 font-body text-[11px] font-medium text-muted-foreground">
            <span class="h-px flex-1 bg-border" />{{ day }}<span class="h-px flex-1 bg-border" />
          </div>
          <div
            :data-id="row.id"
            class="log-row flex cursor-pointer border-l-2 hover:bg-muted/70"
            :class="[ROW_CLASS[row.level], flashId === row.id && 'log-flash', expanded.has(row.id) && '!bg-muted']"
            @click="toggleRow(row)"
          >
            <span
              class="w-[6.75rem] shrink-0 pl-3 tabular-nums text-muted-foreground"
              :title="row.ts !== null ? new Date(row.ts).toISOString() : undefined"
            >{{ row.ts !== null ? formatTime(row.ts) : "" }}</span>
            <span class="w-14 shrink-0 font-semibold" :class="LEVEL_TEXT[row.level]">{{ row.continuation ? "" : row.levelLabel || "·" }}</span>
            <span class="hidden w-36 shrink-0 pr-3 sm:block">
              <button
                v-if="row.source && !row.continuation"
                type="button"
                class="max-w-full truncate align-top hover:underline"
                :class="sourceColor(row.source)"
                :title="`Show only ${row.source}`"
                @click.stop="sourceFilter = row.source"
              >{{ row.source }}</button>
            </span>
            <span
              class="min-w-0 flex-1 pr-3"
              :class="[
                prefs.wrap || expanded.has(row.id) ? 'whitespace-pre-wrap break-words' : 'truncate whitespace-pre',
                row.continuation && 'pl-4 text-muted-foreground',
              ]"
            ><template v-for="(s, i) in segments(row.message)" :key="i"><mark v-if="s.hit" class="rounded-sm bg-yellow-300/70 text-foreground dark:bg-yellow-500/40">{{ s.t }}</mark><template v-else>{{ s.t }}</template></template></span>
          </div>

          <div
            v-if="expanded.has(row.id)"
            class="cursor-auto border-b border-l-2 border-border bg-muted/40 py-3 pl-[7.25rem] pr-4 font-mono text-xs"
            :class="ROW_CLASS[row.level].split(' ')[0]"
          >
            <pre class="whitespace-pre-wrap break-words text-foreground">{{ prettyJson(row.message) || row.raw }}</pre>
            <dl
              v-if="Object.keys(row.fields).length"
              class="mt-3 grid grid-cols-[minmax(6rem,max-content)_1fr] gap-x-4 gap-y-0.5"
            >
              <template v-for="(value, key) in row.fields" :key="key">
                <dt class="text-muted-foreground">{{ key }}</dt>
                <dd class="break-all">{{ value }}</dd>
              </template>
            </dl>
            <div class="mt-3 flex flex-wrap gap-2 font-body">
              <AppButton size="sm" @click="copyLine(row)"><Copy class="h-3.5 w-3.5" :stroke-width="1.75" />Copy line</AppButton>
              <AppButton v-if="row.source && sourceFilter !== row.source" size="sm" @click="sourceFilter = row.source">
                <Filter class="h-3.5 w-3.5" :stroke-width="1.75" />Only {{ row.source }}
              </AppButton>
              <AppButton v-if="filtersActive" size="sm" @click="showInContext(row)">Show in context</AppButton>
            </div>
          </div>
        </template>
      </div>

      <!-- Paused: what's waiting, and the way back -->
      <button
        v-if="paused && conn !== 'closed'"
        type="button"
        class="absolute bottom-4 left-1/2 z-20 inline-flex -translate-x-1/2 items-center gap-2 rounded-full bg-foreground px-4 py-2 text-xs font-medium text-background shadow-lg transition hover:opacity-90"
        @click="jumpToLatest"
      >
        <ArrowDown class="h-3.5 w-3.5" />
        {{ held.length ? `${held.length >= prefs.history ? `${prefs.history.toLocaleString()}+` : held.length.toLocaleString()} new line${held.length === 1 ? "" : "s"}` : "Back to live" }}
        <kbd class="log-kbd border-background/30 text-background/70">G</kbd>
      </button>
    </div>

    <!-- Status bar -->
    <footer class="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 border-t border-border bg-muted/40 px-3 py-1.5 text-[11px] text-muted-foreground">
      <span class="tabular-nums">
        <span v-if="container" class="font-mono">{{ container }}</span>
        <span v-if="container"> · </span>
        {{ rows.length.toLocaleString() }} / {{ prefs.history.toLocaleString() }} lines
        <template v-if="filtersActive"> · {{ visible.length.toLocaleString() }} shown</template>
      </span>
      <span class="hidden items-center gap-3 md:flex">
        <span><kbd class="log-kbd">/</kbd> search</span>
        <span><kbd class="log-kbd">Space</kbd> pause</span>
        <span><kbd class="log-kbd">G</kbd> latest</span>
        <span><kbd class="log-kbd">n</kbd> <kbd class="log-kbd">N</kbd> errors</span>
        <span><kbd class="log-kbd">f</kbd> full screen</span>
      </span>
    </footer>
  </section>
</template>

<style scoped>
/* Rows off screen skip layout and paint, so thousands of lines stay cheap. */
.log-row {
  content-visibility: auto;
  contain-intrinsic-size: auto 20px;
}
.log-flash {
  animation: log-flash 1.4s ease-out;
}
@keyframes log-flash {
  0%,
  30% {
    background-color: color-mix(in oklab, var(--primary) 28%, transparent);
  }
}
.log-tool {
  @apply inline-flex h-8 w-8 items-center justify-center rounded-lg border border-transparent text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:pointer-events-none disabled:opacity-40;
}
.log-tool-on {
  @apply border-border bg-muted text-foreground;
}
.log-kbd {
  @apply inline-flex h-4 min-w-4 items-center justify-center rounded border border-border px-1 font-mono text-[10px] leading-none text-muted-foreground;
}
</style>
