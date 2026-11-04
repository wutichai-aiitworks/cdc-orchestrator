// Shared, pure view logic for the Copy & Sync tools (Postgres dump, Postgres copy, Load to
// Postgres, Mongo copy, Redis sync). No Nuxt/Vue imports: unit-tested in tests/unit/.
// Tool-specific rules live in utils/pgDumpView.ts and utils/transferTools/*.ts.

export type Tone = "ok" | "warn" | "err" | "run" | "idle"
export type StepState = "pending" | "now" | "done" | "failed"
export type RouteStep = { key: string; label: string; detail: string; state: StepState }
export type SummaryRow = { label: string; value: string }
export type Problem = { title: string; detail?: string; fix?: string; upload?: string }

/** The fields every sidecar job has. */
export type BaseJob = {
  id: string
  status: string
  created_at?: string
  started_at?: string
  finished_at?: string
  exit_code?: number | null
  error?: string
}

export const ACTIVE_STATUSES = ["starting", "running"]
export const isActive = (status?: string) => ACTIVE_STATUSES.includes(String(status || ""))

const pad2 = (n: number) => String(n).padStart(2, "0")
const ts = (iso?: string) => (iso ? Date.parse(iso) : NaN)
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]

/** 11 s · 4 m 12 s · 1 h 03 m */
export function formatDuration(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return "—"
  const s = Math.round(ms / 1000)
  if (s < 60) return `${s} s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m} m ${pad2(s % 60)} s`
  return `${Math.floor(m / 60)} h ${pad2(m % 60)} m`
}

export function jobDurationMs(job: BaseJob, now = Date.now()): number | null {
  const start = ts(job.started_at || job.created_at)
  if (!Number.isFinite(start)) return null
  const end = isActive(job.status) ? now : ts(job.finished_at)
  return Number.isFinite(end) ? Math.max(0, end - start) : null
}

/** just now · 5 min ago · 2 h ago · yesterday · 28 Oct */
export function relativeTime(iso: string | undefined, now = Date.now()): string {
  const t = ts(iso)
  if (!Number.isFinite(t)) return ""
  const min = Math.floor(Math.max(0, now - t) / 60000)
  if (min < 1) return "just now"
  if (min < 60) return `${min} min ago`
  const h = Math.floor(min / 60)
  if (h < 24) return `${h} h ago`
  if (h < 48) return "yesterday"
  const d = new Date(t)
  return `${d.getDate()} ${MONTHS[d.getMonth()]}`
}

/** Today 09:42 · Yesterday 02:00 · 25 Oct 02:00 (future dates are never "Today"). */
export function startedLabel(iso: string | undefined, now = Date.now()): string {
  const t = ts(iso)
  if (!Number.isFinite(t)) return "—"
  const d = new Date(t)
  const n = new Date(now)
  const hm = `${pad2(d.getHours())}:${pad2(d.getMinutes())}`
  const dayStart = new Date(n.getFullYear(), n.getMonth(), n.getDate()).getTime()
  if (t >= dayStart + 86400000) return `${d.getDate()} ${MONTHS[d.getMonth()]} ${hm}`
  if (t >= dayStart) return `Today ${hm}`
  if (t >= dayStart - 86400000) return `Yesterday ${hm}`
  return `${d.getDate()} ${MONTHS[d.getMonth()]} ${hm}`
}

/** "a", "a and b", "a, b and c" */
export const joinList = (items: string[]) =>
  items.length <= 1 ? items.join("") : `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`

/**
 * Result of a job from a sidecar with the basic status set. `continuous` tools (Redis sync)
 * are "Live" while running: that is the normal state, not something in progress.
 */
export function basicRunResult(job: BaseJob, opts: { continuous?: boolean; runningText?: string } = {}): { tone: Tone; text: string } {
  switch (job.status) {
    case "starting":
      return { tone: "run", text: "Starting" }
    case "running":
      return { tone: opts.continuous ? "ok" : "run", text: opts.runningText || (opts.continuous ? "Live" : "Running") }
    case "succeeded":
      return { tone: "ok", text: opts.continuous ? "Finished" : "Succeeded" }
    case "completed_with_errors":
      return { tone: "warn", text: "Finished with errors" }
    case "failed":
      return { tone: "err", text: "Failed" }
    case "stopped":
      return { tone: "idle", text: "Stopped" }
    default:
      return { tone: "idle", text: job.status || "Unknown" }
  }
}

/** Rail line: "Succeeded 2 h ago", "Running, copying data", "Never run". */
export function railLine(
  job: BaseJob | undefined,
  result: (j: any) => { tone: Tone; text: string },
  now = Date.now(),
): { tone: Tone; text: string } {
  if (!job) return { tone: "idle", text: "Never run" }
  const r = result(job)
  if (isActive(job.status)) return r
  const when = relativeTime(job.finished_at || job.started_at || job.created_at, now)
  return { tone: r.tone, text: when ? `${r.text} ${when}` : r.text }
}

const norm = (v: unknown): string => JSON.stringify(v, (_k, x) => (x === "" || x === false || x === undefined ? null : x))

/** Human labels of what differs between the saved transfer and the editor (null saved = new). */
export function changedBy<P>(labels: [string, (p: P) => unknown][], saved: P | null, draft: P): string[] {
  if (!saved) return ["new transfer"]
  return labels.filter(([, get]) => norm(get(saved)) !== norm(get(draft))).map(([label]) => label)
}

/**
 * Steps of a run from a fixed list and the index of the step the run reached (from the log).
 * Finished runs mark everything done; failed/stopped runs mark the reached step failed.
 */
export function stepsAt(defs: Omit<RouteStep, "state">[], job: BaseJob | undefined, reached: number): RouteStep[] {
  const steps = defs.map((d) => ({ ...d, state: "pending" as StepState }))
  if (!job || !steps.length) return steps
  const at = Math.min(Math.max(reached, 0), steps.length - 1)
  if (job.status === "succeeded") steps.forEach((s) => (s.state = "done"))
  else if (isActive(job.status)) steps.forEach((s, i) => (s.state = i < at ? "done" : i === at ? "now" : "pending"))
  else steps.forEach((s, i) => (s.state = i < at ? "done" : i === at ? "failed" : "pending"))
  return steps
}

/** Index of the last pattern that matches the log (-1 if none). Patterns are in step order. */
export function lastMatching(log: string, patterns: RegExp[]): number {
  let found = -1
  patterns.forEach((re, i) => {
    if (re.test(log)) found = i
  })
  return found
}

type Rule = { test: RegExp; fix: string }
const RULES: Rule[] = [
  {
    test: /password authentication failed|access denied for user|authentication failed|wrongpass|noauth|invalid username-password|login failed for user|no password supplied/i,
    fix: "The server rejected the login. Check the username and password; a saved password must be entered again after changing the host.",
  },
  {
    test: /could not translate host name|name or service not known|no such host|temporary failure in name resolution|getaddrinfo/i,
    fix: "The host name does not resolve from the tool's container. Use a name or IP that the container can reach (for a database on this machine, host.docker.internal).",
  },
  {
    test: /connection refused|i\/o timeout|timed out|timeout expired|no route to host|server selection (error|timeout)|could not connect|connection reset/i,
    fix: "The server did not accept the connection from the tool's container. Check host, port, firewall, and that both are on a reachable network.",
  },
  {
    test: /role "[^"]+" does not exist/i,
    fix: "The objects' owner role does not exist on the target. Turn on “Do not set owners”, or create that role on the target first.",
  },
  {
    // pgcopydb cannot parse COMMENT ON COLUMN entries of the schema archive.
    test: /Failed to parse Archive TOC comment/i,
    fix: "pgcopydb cannot read the column comments in this database. Turn on “Leave out comments” (Target behaviour) and run again.",
  },
  {
    test: /invalid snapshot identifier/i,
    fix: "The run tried to continue from an earlier run's snapshot, which no longer exists. Turn off “Resume the previous run” and run again.",
  },
  { test: /no pg_hba\.conf entry/i, fix: "pg_hba.conf on the server rejects this client. Allow the tool's address, or change the TLS setting." },
  { test: /ssl (error|handshake|syscall)|tls handshake|certificate verify|x509|server does not support ssl/i, fix: "The TLS settings do not match the server. Turn TLS on or off to match it, or check the certificate." },
  { test: /permission denied|not authorized|insufficient privilege|must be owner|noperm/i, fix: "The user lacks a privilege. Grant it on the server, or narrow what is copied." },
  { test: /already exists|duplicate key|busykey/i, fix: "Objects already exist on the target. Empty it first, or turn on the option that replaces existing data." },
  { test: /no space left on device|disk full|could not extend file/i, fix: "A disk is full. Free space on the target (or on the tool's work volume) and run again." },
  { test: /out of memory|oom|cannot allocate memory/i, fix: "The tool ran out of memory. Lower the parallel jobs or give the container more memory." },
  { test: /server version mismatch|unsupported .*version|incompatible/i, fix: "The tool's client is not compatible with this server version. Rebuild the sidecar image with a newer client." },
]

/** First error-looking line of a log and what to do about it, from known patterns. */
/** Removes terminal colour codes (RedisShake and others colour their logs). */
// eslint-disable-next-line no-control-regex
export const stripAnsi = (text: string) => (text || "").replace(/\x1b\[[0-9;]*[A-Za-z]/g, "")

/** Drops tool log prefixes (timestamp, pid, level, source file) so the message itself shows. */
export const cleanLogLine = (line: string) =>
  line
    .replace(/^\s*\d{4}-\d{2}-\d{2}[T ][\d:.]+(?:[+-]\d{2}:?\d{2}|Z)?\s+/, "")
    .replace(/^\d+\s+(?:ERROR|FATAL|WARN(?:ING)?|INFO|LOG)\s+[\w./-]+:\d+\s+/, "")
    .trim()

export function diagnose(text: string): { line?: string; fix?: string } {
  const lines = stripAnsi(text).split("\n")
  const raw = lines.find((l) => /\b(error|fatal|panic|failed)\b/i.test(l) && !/^\s*-{2,}/.test(l) && !/:\/\/[^\s/]*:[^\s/@]*@/.test(l))
  const line = raw ? cleanLogLine(raw).slice(0, 500) : undefined
  const haystack = line ? `${line}\n${text}` : text
  const rule = RULES.find((r) => r.test.test(line || "")) || RULES.find((r) => r.test.test(haystack || ""))
  return { line, fix: rule?.fix }
}

/** Problems of a basic job: the error, the first error line of its log, and a fix. */
export function basicProblems(job: BaseJob, log: string, title = "The run failed"): Problem[] {
  if (job.status !== "failed") return []
  const d = diagnose(`${job.error || ""}\n${log}`)
  const detail = [job.error, d.line].filter((x, i, a) => x && a.indexOf(x) === i).join("\n")
  return [
    {
      title: job.exit_code != null && job.exit_code !== 0 ? `${title} (exit code ${job.exit_code})` : title,
      detail: detail || undefined,
      fix: d.fix || "Open the log below; the first error line usually names the cause.",
    },
  ]
}

/** Identity of a login for saved-password handling (same rule as the API's saved_secret.rs). */
export const loginKeyOf = (
  c: { type?: string; host: string; port: string; username: string } | null | undefined,
  defaultPort = "",
) => (c ? `${c.type || ""}|${c.host.trim().toLowerCase()}|${c.port.trim() || defaultPort}|${c.username.trim()}` : "")
