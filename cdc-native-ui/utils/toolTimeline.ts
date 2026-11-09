// Copy & Sync timeline (ADR 0009): pure view rules for a profile's audit trail. The orchestrator
// records profile changes, runs (start, refusal, stop, finish) and connection tests; this turns
// them into tones, labels, day groups, detail rows and CSV. Unit-tested in tests/unit/toolTimeline.test.ts.
import { formatDuration, type Tone } from "./transferView"

export type ToolEvent = {
  id: string
  timestamp: string
  tool: string
  profile_id?: string | null
  profile_name?: string | null
  job_id?: string | null
  kind: string
  status?: string | null
  actor: string
  summary: string
  detail: Record<string, any>
}

export type TimelineFilter = "all" | "runs" | "changes" | "tests" | "failures"

export const FILTERS: { key: TimelineFilter; label: string }[] = [
  { key: "all", label: "All" },
  { key: "runs", label: "Runs" },
  { key: "changes", label: "Changes" },
  { key: "tests", label: "Connection tests" },
  { key: "failures", label: "Problems" },
]

const KIND_LABELS: Record<string, string> = {
  "run.started": "Run started",
  "run.finished": "Run finished",
  "run.start_failed": "Run refused",
  "run.stop_requested": "Stop requested",
  "profile.created": "Created",
  "profile.updated": "Changed",
  "profile.deleted": "Deleted",
  "connection.tested": "Connection test",
}

export const kindLabel = (kind: string) => KIND_LABELS[kind] || kind

/** ok = worked, err = failed, warn = stopped/deleted (needs a look), run = run started, idle = info. */
export function eventTone(e: ToolEvent): Tone {
  if (e.kind === "run.finished") return e.status === "succeeded" ? "ok" : e.status === "stopped" ? "warn" : "err"
  if (e.kind === "run.start_failed") return "err"
  if (e.kind === "connection.tested") return e.status === "ok" ? "ok" : "err"
  if (e.kind === "profile.deleted" || e.kind === "run.stop_requested") return "warn"
  if (e.kind === "run.started") return "run"
  return "idle"
}

export function matchesFilter(e: ToolEvent, f: TimelineFilter): boolean {
  switch (f) {
    case "runs":
      return e.kind.startsWith("run.")
    case "changes":
      return e.kind.startsWith("profile.")
    case "tests":
      return e.kind.startsWith("connection.")
    case "failures":
      return eventTone(e) === "err" || eventTone(e) === "warn"
    default:
      return true
  }
}

export function matchesSearch(e: ToolEvent, q: string): boolean {
  const s = q.trim().toLowerCase()
  if (!s) return true
  return [e.summary, e.actor, e.job_id, kindLabel(e.kind), e.status, JSON.stringify(e.detail || {})]
    .filter(Boolean)
    .some((v) => String(v).toLowerCase().includes(s))
}

/** YYYY-MM-DD in the viewer's time zone. */
const localDay = (iso: string) => {
  const d = new Date(iso)
  const p = (n: number) => String(n).padStart(2, "0")
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`
}

/** Events (newest first) grouped by local day, labelled "Today", "Yesterday" or the date. */
export function groupByDay(events: ToolEvent[], now = new Date()): { day: string; label: string; events: ToolEvent[] }[] {
  const today = localDay(now.toISOString())
  const yesterday = localDay(new Date(now.getTime() - 86_400_000).toISOString())
  const groups: { day: string; label: string; events: ToolEvent[] }[] = []
  for (const e of events) {
    const day = localDay(e.timestamp)
    let g = groups[groups.length - 1]
    if (!g || g.day !== day) {
      const label =
        day === today
          ? "Today"
          : day === yesterday
            ? "Yesterday"
            : new Date(e.timestamp).toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long", year: "numeric" })
      g = { day, label, events: [] }
      groups.push(g)
    }
    g.events.push(e)
  }
  return groups
}

export const clockTime = (iso: string) =>
  new Date(iso).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit", hourCycle: "h23" })

/** Same 24-hour clock as the time column. */
export const fullTime = (iso: string) =>
  new Date(iso).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "medium", hourCycle: "h23" })

/** Before/after value of a changed setting. */
export function fmtValue(v: unknown): string {
  if (v === null || v === undefined || v === "") return "—"
  if (Array.isArray(v)) return v.length ? v.map((x) => (typeof x === "object" ? JSON.stringify(x) : String(x))).join(", ") : "—"
  if (typeof v === "object") return JSON.stringify(v)
  return String(v)
}

export type ChangeRow = { field: string; before: string; after: string; secret: boolean }

export const changeRows = (e: ToolEvent): ChangeRow[] =>
  ((e.detail?.changes as any[]) || []).map((c) => ({
    field: String(c.field),
    before: c.secret ? "hidden" : fmtValue(c.before),
    after: c.secret ? "changed" : fmtValue(c.after),
    secret: !!c.secret,
  }))

/** Key facts of a finished run, in reading order. */
export function runFacts(e: ToolEvent): { label: string; value: string }[] {
  const d = e.detail || {}
  const rows: { label: string; value: string }[] = []
  if (e.status) rows.push({ label: "Status", value: e.status })
  if (d.started_at) rows.push({ label: "Started", value: fullTime(d.started_at) })
  if (d.finished_at) rows.push({ label: "Finished", value: fullTime(d.finished_at) })
  if (typeof d.duration_secs === "number") rows.push({ label: "Took", value: formatDuration(d.duration_secs * 1000) })
  if (d.exit_code !== undefined && d.exit_code !== null) rows.push({ label: "Exit code", value: String(d.exit_code) })
  if (d.phase) rows.push({ label: "Phase", value: String(d.phase) })
  if (d.transport) rows.push({ label: "Transport", value: String(d.transport) })
  if (d.dry_run) rows.push({ label: "Mode", value: "Preview (dry run)" })
  if (d.stats && typeof d.stats === "object") {
    for (const [k, v] of Object.entries(d.stats)) {
      if (v === null || typeof v === "object") continue
      rows.push({ label: k.replace(/_/g, " "), value: typeof v === "number" ? v.toLocaleString() : String(v) })
    }
  }
  return rows
}

/** Whether the row can expand (something beyond the summary to show). */
export function hasDetail(e: ToolEvent): boolean {
  const d = e.detail || {}
  if (e.kind === "profile.updated") return changeRows(e).length > 0
  if (e.kind === "profile.created") return !!d.profile
  return Object.keys(d).some((k) => k !== "preview")
}

const csvCell = (v: unknown) => {
  const s = v === null || v === undefined ? "" : typeof v === "object" ? JSON.stringify(v) : String(v)
  return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s
}

/** Spreadsheet-ready export of the events shown (UTC timestamps, detail as JSON). */
export function toCsv(events: ToolEvent[]): string {
  const head = ["timestamp_utc", "tool", "profile", "event", "status", "actor", "job_id", "summary", "detail"]
  const lines = events.map((e) =>
    [e.timestamp, e.tool, e.profile_name || e.profile_id, kindLabel(e.kind), e.status, e.actor, e.job_id, e.summary, e.detail].map(csvCell).join(","),
  )
  return [head.join(","), ...lines].join("\r\n") + "\r\n"
}
