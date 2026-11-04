// Pure view logic for the Postgres dump workspace (no Nuxt/Vue imports, unit-tested in
// tests/unit/pgDumpView.test.ts). Words here are the UI's vocabulary: keep the run verb
// ("Run dump" / "Run dump and restore") identical on buttons, toasts and run rows.
import { formatBytes } from "./bytes"
import {
  changedBy,
  formatDuration,
  isActive,
  jobDurationMs,
  joinList as list,
  railLine,
  type RouteStep,
  type Tone,
} from "./transferView"

export type ConnLike = { host: string; port: string; database: string; username: string; password?: string }

export type UploadLike = {
  name: string
  host?: string
  remote_dir?: string
  remote_path?: string
  status: string
  bytes_sent?: number
  error?: string
  hint?: string
  host_key_fingerprint?: string
  host_key_verified?: boolean
}

export type JobLike = {
  id: string
  status: string
  phase?: string
  mode?: string
  created_at?: string
  started_at?: string
  finished_at?: string
  tables_dumped?: number
  tables_restored?: number
  errors_ignored?: number
  artifact?: { name: string; format: string; bytes: number; sha256?: string; local_deleted?: boolean } | null
  uploads?: UploadLike[]
  download_path?: string
  error?: string
  first_error?: string
  hint?: string
}

export type DestLike = { name: string; host: string; port?: string; username?: string; remote_dir: string; host_key_fingerprint?: string }

export type ProfileLike = {
  name: string
  mode: string
  source: ConnLike
  target?: ConnLike | null
  include_tables: string[]
  options: Record<string, any>
  destinations?: DestLike[]
}

export const isRestoreMode = (p: { mode?: string }) => p.mode === "dump_restore"
export const runVerb = (p: { mode?: string }) => (isRestoreMode(p) ? "Run dump and restore" : "Run dump")
export const startedToast = (p: { mode?: string }) => (isRestoreMode(p) ? "Dump and restore started" : "Dump started")

const PHASE_WORD: Record<string, string> = { dump: "dumping", restore: "restoring", upload: "uploading" }

export const failedUploads = (job: JobLike) => (job.uploads || []).filter((u) => u.status === "failed")

/** One label for a run, used in the rail, the run table and the header. */
export function runResult(job: JobLike): { tone: Tone; text: string } {
  switch (job.status) {
    case "starting":
      return { tone: "run", text: "Starting" }
    case "running":
      return { tone: "run", text: `Running, ${PHASE_WORD[job.phase || ""] || "working"}` }
    case "succeeded":
      return { tone: "ok", text: "Succeeded" }
    case "completed_with_errors": {
      const failed = failedUploads(job).length
      if (failed) return { tone: "warn", text: failed === 1 ? "Upload failed" : `${failed} uploads failed` }
      if (job.errors_ignored) return { tone: "warn", text: `Restored with ${job.errors_ignored} errors` }
      return { tone: "warn", text: "Finished with errors" }
    }
    case "failed":
      return { tone: "err", text: "Failed" }
    case "stopped":
      return { tone: "idle", text: "Stopped" }
    default:
      return { tone: "idle", text: job.status || "Unknown" }
  }
}

/** Rail line: "Succeeded 2 h ago", "Running, dumping", "Never run". */
export const railStatus = (job: JobLike | undefined, now = Date.now()) => railLine(job, runResult, now)

/**
 * Steps of a transfer, from the profile (what will happen) and optionally the job being
 * shown (what happened). Dump → (Restore) → (Upload).
 */
export function routeSteps(profile: ProfileLike, job?: JobLike, now = Date.now()): RouteStep[] {
  const restore = isRestoreMode(profile)
  const dests = profile.destinations || []
  const fmt = profile.options.format || "custom"
  const level = profile.options.compress
  const steps: RouteStep[] = [
    {
      key: "dump",
      label: "Dump",
      detail: restore ? "custom archive" : `${fmt === "plain" ? "plain SQL" : fmt}${level != null && level !== "" ? `, level ${level}` : ""}`,
      state: "pending",
    },
  ]
  if (restore) steps.push({ key: "restore", label: "Restore", detail: profile.options.restore_jobs > 1 ? `${profile.options.restore_jobs} parallel jobs` : "into target", state: "pending" })
  if (dests.length) steps.push({ key: "upload", label: "Upload", detail: dests.length === 1 ? "1 server" : `${dests.length} servers`, state: "pending" })
  if (!job) return steps

  const order = steps.map((s) => s.key)
  if (isActive(job.status)) {
    const current = job.status === "starting" ? "dump" : job.phase || "dump"
    const idx = Math.max(0, order.indexOf(current))
    steps.forEach((s, i) => (s.state = i < idx ? "done" : i === idx ? "now" : "pending"))
    const elapsed = formatDuration(jobDurationMs(job, now))
    if (steps[idx]) steps[idx].detail = current === "dump" && job.tables_dumped ? `${job.tables_dumped} tables, ${elapsed}` : elapsed
    return steps
  }

  const uploadsFailed = failedUploads(job).length
  for (const s of steps) {
    if (job.status === "succeeded") s.state = "done"
    else if (s.key === "upload" && job.uploads?.length) s.state = uploadsFailed ? "failed" : "done"
    else if (s.key === "restore" && job.status === "completed_with_errors") s.state = job.errors_ignored ? "failed" : "done"
    else if (job.status === "completed_with_errors") s.state = "done"
  }
  if (job.status === "failed" || job.status === "stopped") {
    // The first step without evidence of completion is where it stopped.
    const dumped = !!job.artifact
    const restored = job.tables_restored != null && job.tables_restored > 0
    const stoppedAt = !dumped ? "dump" : restore && !restored ? "restore" : "upload"
    const idx = Math.max(0, order.indexOf(stoppedAt))
    steps.forEach((s, i) => (s.state = i < idx ? "done" : i === idx ? "failed" : "pending"))
  }
  const dump = steps[0]
  if (job.artifact) dump.detail = `${job.tables_dumped ?? 0} tables, ${formatBytes(job.artifact.bytes)}`
  const up = steps.find((s) => s.key === "upload")
  if (up && job.uploads?.length) {
    const ok = job.uploads.filter((u) => u.status === "uploaded").length
    up.detail = uploadsFailed ? `${uploadsFailed} of ${job.uploads.length} failed` : `${ok} of ${job.uploads.length} stored`
  }
  const rs = steps.find((s) => s.key === "restore")
  if (rs && job.tables_restored != null && !isActive(job.status)) {
    rs.detail = job.errors_ignored ? `${job.errors_ignored} errors ignored` : `${job.tables_restored} tables`
  }
  return steps
}

export const connLabel = (c: ConnLike) => `${c.username || "?"}@${c.host || "?"}:${c.port || "5432"}/${c.database || "?"}`

/** "What this transfer does", in sentences. */
export function summarize(p: ProfileLike): { label: string; value: string }[] {
  const o = p.options || {}
  const out: { label: string; value: string }[] = []
  const what: string[] = []
  if (o.schemas?.length) what.push(`schema${o.schemas.length > 1 ? "s" : ""} ${list(o.schemas)}`)
  if (p.include_tables?.length) what.push(`tables matching ${list(p.include_tables)}`)
  let scope = what.length ? `Only ${list(what)}` : "The whole database"
  if (o.schema_only) scope += ", definitions only (no rows)"
  else if (o.data_only) scope += ", rows only (no definitions)"
  out.push({ label: "Copies", value: scope })
  const skips: string[] = []
  if (o.exclude_schemas?.length) skips.push(`schemas ${list(o.exclude_schemas)}`)
  if (o.exclude_tables?.length) skips.push(`tables ${list(o.exclude_tables)}`)
  if (o.exclude_table_data?.length) skips.push(`rows of ${list(o.exclude_table_data)}`)
  if (skips.length) out.push({ label: "Skips", value: list(skips) })

  if (isRestoreMode(p) && p.target) {
    const t = `${p.target.database} on ${p.target.host}`
    const how = o.restore_clean ? `Drops the archived objects in ${t}, then recreates them` : `Creates objects and loads rows into ${t}`
    const par = o.restore_jobs > 1 ? `, ${o.restore_jobs} tables at a time` : ""
    const stop = o.restore_exit_on_error ? "; stops at the first error" : "; skips objects that fail and reports them"
    out.push({ label: "On the target", value: `${how}${par}${stop}` })
  }
  const fmt = isRestoreMode(p) ? "custom" : o.format || "custom"
  const fmtText = fmt === "plain" ? "Plain SQL" : fmt === "tar" ? "Tar archive" : "Custom archive"
  const dests = p.destinations || []
  let file = `${fmtText}${o.compress != null && o.compress !== "" && fmt !== "tar" ? `, compression ${o.compress}` : ""}`
  if (dests.length) {
    file += `; uploaded to ${list(dests.map((d) => d.name || d.host))}`
    file += o.keep_local ? ", and kept on the dump volume" : ", then removed from the dump volume"
  } else file += ", kept on the dump volume for download"
  out.push({ label: "File", value: file })
  return out
}

/** Effects on someone else's data, listed in the run confirmation. Empty = no confirm needed. */
export function destructiveEffects(p: ProfileLike): string[] {
  if (!isRestoreMode(p) || !p.target) return []
  const o = p.options || {}
  const where = `${p.target.database} on ${p.target.host}`
  const out: string[] = []
  if (o.restore_clean) out.push(`Drop every object in the archive that already exists in ${where}, then recreate it.`)
  else out.push(`Create objects and load rows into ${where}. Objects that already exist cause errors.`)
  if (o.data_only) out.push("Add rows to existing tables without touching their definitions.")
  if (o.restore_single_transaction) out.push("Restore in one transaction: nothing is kept if any step fails.")
  return out
}

/** Labels for the save bar's change list (also used by the workspace). */
export const pgDumpChangeLabels: [string, (p: ProfileLike) => unknown][] = [
  ["name", (p) => p.name],
  ["mode", (p) => p.mode],
  ["source connection", (p) => [p.source.host, p.source.port, p.source.database, p.source.username, p.source.password, p.options.source_sslmode]],
  ["target connection", (p) => (isRestoreMode(p) ? [p.target?.host, p.target?.port, p.target?.database, p.target?.username, p.target?.password, p.options.target_sslmode] : null)],
  ["schemas", (p) => p.options.schemas || []],
  ["tables", (p) => p.include_tables || []],
  ["exclusions", (p) => [p.options.exclude_schemas || [], p.options.exclude_tables || [], p.options.exclude_table_data || []]],
  ["contents", (p) => [!!p.options.schema_only, !!p.options.data_only]],
  ["format", (p) => p.options.format],
  ["compression", (p) => p.options.compress ?? null],
  ["restore behaviour", (p) => ["restore_clean", "restore_if_exists", "restore_jobs", "restore_exit_on_error", "restore_single_transaction", "restore_no_owner", "restore_no_privileges"].map((k) => p.options[k] ?? null)],
  ["SFTP servers", (p) => p.destinations || []],
  ["keep local copy", (p) => !!p.options.keep_local],
  ["advanced", (p) => ["no_owner", "no_privileges", "no_comments", "no_publications", "no_subscriptions", "serializable_deferrable", "lock_wait_timeout_ms"].map((k) => p.options[k] ?? null)],
]

/** Human labels of what differs between the saved profile and the editor. */
export const changedFields = (saved: ProfileLike | null, draft: ProfileLike) => changedBy(pgDumpChangeLabels, saved, draft)

/** Problems on a run, each with what to do next. */
export function runProblems(job: JobLike): { title: string; detail?: string; fix?: string; upload?: string }[] {
  const out: { title: string; detail?: string; fix?: string; upload?: string }[] = []
  if (job.status === "failed") {
    out.push({ title: job.error || "The run failed", detail: job.first_error && job.first_error !== job.error ? job.first_error : undefined, fix: job.hint })
  } else if (job.status === "completed_with_errors" && job.errors_ignored) {
    out.push({ title: `pg_restore skipped ${job.errors_ignored} object${job.errors_ignored === 1 ? "" : "s"} that failed`, detail: job.first_error, fix: job.hint })
  }
  for (const u of failedUploads(job)) {
    out.push({ title: `Upload to ${u.name} failed`, detail: u.error, fix: u.hint || "The file is still on the dump volume; download it or run again.", upload: u.name })
  }
  return out
}
