// File sync (rsync over SSH): pure view rules. No Nuxt/Vue imports; unit-tested in
// tests/unit/rsyncView.test.ts. Vocabulary: "direct" = the source pushes straight to the target;
// "relay" = through the sync service's staging copy (ADR 0007).
import { formatBytes } from "../bytes"
import { changedBy, formatDuration, isActive, jobDurationMs, joinList as list, railLine, type Problem, type RouteStep, type Tone } from "../transferView"

export type EndLike = {
  name?: string
  host: string
  port?: string
  username: string
  auth?: string
  password?: string
  private_key?: string
  passphrase?: string
  path: string
  host_key_fingerprint?: string
  rsync_path?: string
  reach_host?: string
  reach_port?: string
}

export type ProfileLike = { name: string; source: EndLike; target: EndLike; options: Record<string, any> }

export type JobLike = {
  id: string
  status: string
  phase?: string
  created_at?: string
  started_at?: string
  finished_at?: string
  transport?: string
  transport_requested?: string
  transport_reason?: string
  dry_run?: boolean
  progress?: { bytes: number; percent: number; rate: string; eta: string; files_transferred?: number; files_to_check?: number; files_total?: number } | null
  stats?: Record<string, number>
  changes?: { counts: Record<string, number>; items: { op: string; kind: string; path: string }[]; truncated: boolean }
  exit_code?: number | null
  error?: string
  first_error?: string
  hint?: string
}

export const transportOf = (p: { options?: Record<string, any> }) => (p.options?.transport || "auto") as "auto" | "direct" | "relay"
export const isPreviewProfile = (p: { options?: Record<string, any> }) => !!p.options?.dry_run
export const runVerb = (p: { options?: Record<string, any> }) => (isPreviewProfile(p) ? "Run preview" : "Run sync")
export const startedToast = (p: { options?: Record<string, any> }) => (isPreviewProfile(p) ? "Preview started" : "Sync started")

const PHASE_WORD: Record<string, string> = {
  connect: "connecting",
  route: "checking the direct route",
  sync: "syncing",
  pull: "pulling to the relay",
  push: "pushing to the target",
}

/** "3 new, 1 changed, 2 deleted" from a preview's itemized changes. */
export function changeCounts(job: JobLike): string {
  const c = job.changes?.counts || {}
  const parts = [
    c.create ? `${c.create} new` : "",
    c.update ? `${c.update} changed` : "",
    c.delete ? `${c.delete} deleted` : "",
  ].filter(Boolean)
  return parts.length ? list(parts) : "no changes"
}

export function runResult(job: JobLike): { tone: Tone; text: string } {
  switch (job.status) {
    case "starting":
      return { tone: "run", text: "Starting" }
    case "running":
      return { tone: "run", text: `Running, ${PHASE_WORD[job.phase || ""] || "working"}` }
    case "succeeded":
      return job.dry_run ? { tone: "ok", text: `Preview: ${changeCounts(job)}` } : { tone: "ok", text: "Succeeded" }
    case "completed_with_errors":
      if (job.exit_code === 25) return { tone: "warn", text: "Stopped at the delete limit" }
      if (job.exit_code === 24) return { tone: "warn", text: "Some files vanished" }
      return { tone: "warn", text: "Some files not copied" }
    case "failed":
      return { tone: "err", text: "Failed" }
    case "stopped":
      return { tone: "idle", text: "Stopped" }
    default:
      return { tone: "idle", text: job.status || "Unknown" }
  }
}

export const railStatus = (job: JobLike | undefined, now = Date.now()) => railLine(job, runResult, now)

const filesDetail = (job: JobLike) => {
  if (job.dry_run) return changeCounts(job)
  const s = job.stats || {}
  const n = s.number_of_regular_files_transferred ?? 0
  const bytes = s.total_transferred_file_size
  const del = s.number_of_deleted_files ? `, ${s.number_of_deleted_files} deleted` : ""
  return `${n === 1 ? "1 file" : `${n} files`}${bytes ? `, ${formatBytes(bytes)}` : ""}${del}`
}

/**
 * Steps: Connect → Sync (direct) or Connect → Pull → Push (relay). Before a run, an auto
 * transfer shows one Sync step because the route is decided per run.
 */
export function routeSteps(p: ProfileLike, job?: JobLike, now = Date.now()): RouteStep[] {
  const transport = job?.transport || transportOf(p)
  const relay = transport === "relay"
  const steps: RouteStep[] = [{ key: "connect", label: "Connect", detail: transportOf(p) === "auto" ? "log in, check route" : "log in to the hosts", state: "pending" }]
  if (relay) {
    steps.push({ key: "pull", label: "Pull", detail: "source to relay", state: "pending" })
    steps.push({ key: "push", label: "Push", detail: "relay to target", state: "pending" })
  } else {
    steps.push({
      key: "sync",
      label: "Sync",
      detail: transport === "direct" ? "source to target" : "direct, or via relay if the source cannot reach the target",
      state: "pending",
    })
  }
  if (!job) return steps
  const order = steps.map((s) => s.key)
  const at = (phase?: string) => Math.max(0, order.indexOf(phase === "route" ? "connect" : phase || "connect"))
  const last = steps[steps.length - 1]
  if (isActive(job.status)) {
    const idx = at(job.status === "starting" ? "connect" : job.phase)
    steps.forEach((s, i) => (s.state = i < idx ? "done" : i === idx ? "now" : "pending"))
    const pr = job.progress
    steps[idx].detail = pr && steps[idx].key !== "connect" ? `${pr.percent}%, ${pr.rate}` : formatDuration(jobDurationMs(job, now))
    return steps
  }
  if (job.status === "succeeded" || job.status === "completed_with_errors") {
    steps.forEach((s) => (s.state = "done"))
    if (job.status === "completed_with_errors") last.state = "failed"
    last.detail = filesDetail(job)
  } else {
    const idx = at(job.phase)
    steps.forEach((s, i) => (s.state = i < idx ? "done" : i === idx ? "failed" : "pending"))
  }
  if (job.transport) steps[0].detail = job.transport === "direct" ? "direct route" : "relay route"
  return steps
}

export const endLabel = (e: EndLike) => `${e.username || "?"}@${e.host || "?"}${e.port && e.port !== "22" ? `:${e.port}` : ""}`

/** "The contents of /srv/www" vs "The folder /srv/www itself": rsync's trailing-slash rule in words. */
export function whatIsCopied(path: string): string {
  const p = (path || "").trim()
  if (!p) return "Nothing yet"
  if (p.endsWith("/")) return `The contents of ${p}`
  return `${p} itself (as a folder or file inside the target directory)`
}

export function summarize(p: ProfileLike): { label: string; value: string }[] {
  const o = p.options || {}
  const out: { label: string; value: string }[] = []
  out.push({ label: "Copies", value: `${whatIsCopied(p.source.path)} on ${endLabel(p.source)}` })
  out.push({ label: "Into", value: `${p.target.path || "?"} on ${endLabel(p.target)} (created if missing)` })
  const t = transportOf(p)
  const route =
    t === "direct"
      ? "Direct: the source sends to the target. The target key is lent to the source for the run and never stored there."
      : t === "relay"
        ? `Relay: pulled to the sync service, then pushed to the target${o.discard_staging ? "; the relay copy is removed after each run" : "; the relay copy is kept so the next run only moves changes"}.`
        : "Direct when the source can reach the target; otherwise through the sync service (relay). Each run records which one it used."
  out.push({ label: "Route", value: route })
  out.push({
    label: "Deletes",
    value: o.delete
      ? `Files in the target that are not on the source are deleted${o.max_delete != null && o.max_delete !== "" ? `; a run stops after ${o.max_delete} deletions` : ""}`
      : "Nothing on the target is deleted; extra files stay",
  })
  if (o.excludes?.length) out.push({ label: "Skips", value: list(o.excludes) })
  const extras = [
    o.checksum && "compare by checksum",
    o.compress && "compress in transit",
    o.no_owner && "do not keep owners",
    o.hard_links && "keep hard links",
    o.acls && "keep ACLs",
    o.xattrs && "keep extended attributes",
    o.numeric_ids && "numeric user IDs",
    o.one_file_system && "stay on one file system",
    o.bwlimit_kbps && `limit to ${o.bwlimit_kbps} KB/s`,
  ].filter(Boolean) as string[]
  if (extras.length) out.push({ label: "Also", value: list(extras) })
  if (o.dry_run) out.push({ label: "Every run", value: "Preview only: lists what would change, writes nothing" })
  return out
}

/** Effects confirmed before a run. Overwriting changed files is the point of a sync; deleting is not. */
export function destructiveEffects(p: ProfileLike): string[] {
  const o = p.options || {}
  if (o.dry_run || !o.delete) return []
  const where = `${p.target.path} on ${p.target.host}`
  const out = [`Delete every file in ${where} that is not on the source (${p.source.path} on ${p.source.host}).`]
  if (o.max_delete != null && o.max_delete !== "") out.push(`The run stops after ${o.max_delete} deletions.`)
  else out.push("There is no deletion limit. Use Preview changes first to see the list.")
  if (o.excludes?.length) out.push(`Excluded patterns are left alone on the target: ${list(o.excludes)}.`)
  return out
}

const endKey = (e?: EndLike) =>
  e ? [e.host, e.port, e.username, e.auth, e.password, e.private_key, e.passphrase, e.host_key_fingerprint, e.rsync_path] : null

export const rsyncChangeLabels: [string, (p: ProfileLike) => unknown][] = [
  ["name", (p) => p.name],
  ["source host", (p) => endKey(p.source)],
  ["source path", (p) => p.source.path],
  ["target host", (p) => endKey(p.target)],
  ["target path", (p) => p.target.path],
  ["direct route address", (p) => [p.target.reach_host, p.target.reach_port]],
  ["route", (p) => transportOf(p)],
  ["deletes", (p) => [!!p.options.delete, p.options.max_delete ?? null]],
  ["exclusions", (p) => p.options.excludes || []],
  ["preview only", (p) => !!p.options.dry_run],
  [
    "advanced",
    (p) =>
      ["checksum", "compress", "no_owner", "hard_links", "acls", "xattrs", "numeric_ids", "one_file_system", "bwlimit_kbps", "io_timeout_seconds", "discard_staging"].map(
        (k) => p.options[k] ?? null,
      ),
  ],
]

export const changedFields = (saved: ProfileLike | null, draft: ProfileLike) => changedBy(rsyncChangeLabels, saved, draft)

export function runProblems(job: JobLike): Problem[] {
  if (job.status !== "failed" && job.status !== "completed_with_errors") return []
  return [
    {
      title: job.error || (job.status === "failed" ? "The sync failed" : "The sync finished with errors"),
      detail: job.first_error && job.first_error !== job.error ? job.first_error : undefined,
      fix: job.hint,
    },
  ]
}

/** Short "what it did" for the run table and last-run line. */
export function output(job: JobLike): string {
  if (isActive(job.status) || job.status === "failed" || job.status === "stopped") return ""
  const via = job.transport ? `, ${job.transport}` : ""
  return `${filesDetail(job)}${via}`
}

export function liveFacts(job: JobLike): { label: string; value: string }[] {
  const out: { label: string; value: string }[] = []
  if (job.transport) out.push({ label: "Route", value: job.transport === "direct" ? "Direct" : "Relay" })
  const pr = job.progress
  if (pr) {
    out.push({ label: "Progress", value: `${pr.percent}%` })
    out.push({ label: "Speed", value: pr.rate })
    if (pr.files_total) out.push({ label: "Files checked", value: `${pr.files_total - (pr.files_to_check ?? 0)} of ${pr.files_total}` })
  }
  return out
}
