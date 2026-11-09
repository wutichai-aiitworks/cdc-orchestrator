// Redis sync (RedisShake): pure view rules. A sync is continuous: while running it is "Syncing",
// the normal state, until someone stops it. Unit-tested in tests/unit/transferTools.test.ts.
import { basicProblems, basicRunResult, joinList, lastMatching, stepsAt, stripAnsi, type BaseJob, type Problem, type RouteStep, type SummaryRow } from "../transferView"
import type { Conn } from "./pgcopy"

export type RedisShakePayload = { name: string; source: Conn; target: Conn; options: Record<string, any> }

// Start modes, in CDC terms: a one-time sync (full copy, then the run ends) or CDC (follow source
// changes until stopped), optionally beginning with an initial snapshot (the full copy).
export type StartModeKey = "once" | "cdc" | "cdc-live"
export const MODE_LABELS: Record<StartModeKey, string> = {
  once: "Run one-time sync",
  cdc: "Start CDC",
  "cdc-live": "Start CDC from now",
}
export const MODE_HELP: Record<StartModeKey, string> = {
  once: "Copy every key once, then stop.",
  cdc: "Copy every key, then keep applying source changes until you stop it.",
  "cdc-live": "Skip the full copy: apply only changes made on the source from now on.",
}
/** The profile's default start mode (main button). */
export function defaultMode(o: Record<string, any>): StartModeKey {
  if (o.sync_aof === false) return "once"
  return o.sync_rdb === false ? "cdc-live" : "cdc"
}
/** Where "empty the target" deletes: the databases the full copy writes, or all of them. */
export function emptyScopeText(p: RedisShakePayload): string {
  const o = p.options || {}
  if (o.target_cluster) return `every key on the cluster ${addr(p.target)}`
  const dbs: number[] = o.source_dbs || []
  const t = targetDb(o)
  const scope = !dbs.length ? null : t !== undefined ? [t] : dbs
  return scope ? `every key in ${joinList(scope.map((d) => `db ${d}`))} on ${addr(p.target)}` : `every key in every database on ${addr(p.target)}`
}
/** Whether a run in this mode empties the target first (only before a full copy). */
export const emptiesFirst = (o: Record<string, any>, mode: StartModeKey, override?: boolean) =>
  mode !== "cdc-live" && (override ?? !!o.empty_target)

/** Mode of a run: from the job record (newer sidecars), else the profile default. */
export function jobMode(job: BaseJob | undefined, o: Record<string, any>): StartModeKey {
  const j = job as any
  if (j?.mode === "once") return "once"
  if (j?.mode === "cdc") return j.snapshot === false ? "cdc-live" : "cdc"
  return defaultMode(o)
}
export const verb = (p?: RedisShakePayload) => MODE_LABELS[defaultMode(p?.options || {})]
export const stopVerb = "Stop sync"
export const startedToast = (p?: RedisShakePayload) =>
  defaultMode(p?.options || {}) === "once" ? "One-time sync started" : "CDC started"
const addr = (c: Conn) => `${c.host || "?"}:${c.port || "6379"}`

export const DUPLICATE_CHOICES = [
  { value: "panic", label: "Stop the sync", help: "The safest choice: nothing on the target is overwritten." },
  { value: "rewrite", label: "Overwrite the target key", help: "The source wins. The target's value is lost." },
  { value: "skip", label: "Keep the target key", help: "The target wins. The source's value is not copied." },
] as const

/** Database numbers from the token input: "2", "db2" → 2; anything else is dropped. Sorted, unique. */
export const parseDbs = (tokens: string[]): number[] =>
  [...new Set(tokens.map((t) => t.trim().replace(/^db/i, "")).filter((t) => /^\d{1,4}$/.test(t)).map(Number))].sort((a, b) => a - b)

export const dbTokens = (o: Record<string, any>): string[] => (o.source_dbs || []).map(String)

/** The target database when it differs from the single source database, else undefined. */
export function targetDb(o: Record<string, any>): number | undefined {
  const dbs: number[] = o.source_dbs || []
  const t = o.target_db
  return dbs.length === 1 && typeof t === "number" && Number.isInteger(t) && t !== dbs[0] ? t : undefined
}

/** Why the database choice cannot run (the server rejects the same cases), or null. */
export function dbProblem(o: Record<string, any>): string | null {
  const dbs: number[] = o.source_dbs || []
  if (o.source_cluster && dbs.some((d) => d !== 0)) return "A Redis Cluster source has only database 0."
  if (o.target_cluster && (targetDb(o) ?? 0) !== 0) return "A Redis Cluster target has only database 0."
  return null
}

/** "every database", "db 0 and db 5", "db 2 into db 0 on the target". */
export function dbLabel(o: Record<string, any>): string {
  const dbs: number[] = o.source_dbs || []
  if (!dbs.length) return "every database"
  const t = targetDb(o)
  return t === undefined ? joinList(dbs.map((d) => `db ${d}`)) : `db ${dbs[0]} into db ${t} on the target`
}

export function steps(p: RedisShakePayload, job: BaseJob | undefined, log = ""): RouteStep[] {
  const mode = jobMode(job, p.options || {})
  const defs: { key: string; label: string; detail: string }[] = []
  const emptied = (job as any)?.emptied
  if (emptied || (!job && emptiesFirst(p.options || {}, mode))) {
    const n = emptied?.keys_removed
    defs.push({ key: "empty", label: "Empty target", detail: typeof n === "number" ? `${n.toLocaleString()} key${n === 1 ? "" : "s"} removed` : "before the copy" })
  }
  if (mode !== "cdc-live") defs.push({ key: "full", label: "Full copy", detail: mode === "once" ? "every key, once" : "initial snapshot" })
  if (mode !== "once") defs.push({ key: "live", label: "Live changes", detail: "until stopped" })
  if (!defs.length) defs.push({ key: "full", label: "Full copy", detail: "nothing selected" })
  // The sidecar logs the emptying; RedisShake reports the RDB transfer, then AOF (incremental) syncing.
  const PATTERNS: Record<string, RegExp> = {
    empty: /Emptied the target|RedisShake empties it/i,
    full: /syncing rdb|receiving rdb|send_rdb|full sync|start syncing/i,
    live: /syncing aof/i,
  }
  const reached = defs.length > 1 ? lastMatching(stripAnsi(log), defs.map((d) => PATTERNS[d.key])) : 0
  const out = stepsAt(defs, job, reached < 0 ? 0 : reached)
  // Stopping is how CDC ends, not a failure: show the phases it reached as done.
  if (job?.status === "stopped" && mode !== "once") return out.map((st) => (st.state === "failed" ? { ...st, state: "done" } : st))
  return out
}

/** Latest counters from RedisShake's status lines: keys written and replication lag (bytes). */
export function liveStats(log: string): { written?: number; lag?: number } {
  const lines = stripAnsi(log).split("\n").reverse()
  const line = lines.find((l) => /write_count=\[\d+\]/.test(l))
  if (!line) return {}
  const written = Number(line.match(/write_count=\[(\d+)\]/)?.[1])
  const diff = line.match(/diff=\[(-?\d+)\]/)?.[1]
  return { written, lag: diff === undefined ? undefined : Number(diff) }
}

/**
 * A sync never ends on its own: after the full copy it follows live changes until stopped.
 * While running, say which phase it is in (from RedisShake's status lines), so a finished full
 * copy does not read as "still syncing".
 */
export function runResult(job: BaseJob, log = "") {
  // A one-time sync ends by itself, like any finite run.
  if ((job as any).mode === "once") {
    const r = basicRunResult(job)
    return r.text === "Running" ? { ...r, text: "Full copy in progress" } : r
  }
  const base = basicRunResult(job, { continuous: true, runningText: "Syncing" })
  if (base.text !== "Syncing") return base
  const text = stripAnsi(log)
  if (!/syncing aof/i.test(text)) return { ...base, text: text.trim() ? "Full copy in progress" : "Syncing" }
  const { lag } = liveStats(text)
  return { ...base, text: lag === 0 ? "Up to date · following live changes" : "Catching up with the source" }
}

export function summary(p: RedisShakePayload): SummaryRow[] {
  const o = p.options || {}
  const choice = DUPLICATE_CHOICES.find((c) => c.value === (o.rdb_restore_command_behavior || "panic")) || DUPLICATE_CHOICES[0]
  const kind = (cluster: boolean, tls: boolean) => [cluster ? "cluster" : "single server", tls && "TLS"].filter(Boolean).join(", ")
  return [
    { label: "Syncs", value: `${addr(p.source)} (${kind(!!o.source_cluster, !!o.source_tls)}) to ${addr(p.target)} (${kind(!!o.target_cluster, !!o.target_tls)})` },
    { label: "Databases", value: dbLabel(o).replace(/^./, (c) => c.toUpperCase()) },
    { label: "Default start", value: `${MODE_LABELS[defaultMode(o)]}: ${MODE_HELP[defaultMode(o)].charAt(0).toLowerCase()}${MODE_HELP[defaultMode(o)].slice(1)}` },
    {
      label: "Existing keys",
      value: o.empty_target
        ? `Deleted before each full copy (${emptyScopeText(p)})`
        : `When a key already exists on the target: ${choice.label.toLowerCase()}`,
    },
  ]
}

export function effects(p: RedisShakePayload, mode: StartModeKey = defaultMode(p.options || {}), emptyFirst?: boolean): string[] {
  const o = p.options || {}
  const dbs = (o.source_dbs || []).length ? ` (${dbLabel(o)})` : ""
  const how =
    mode === "once"
      ? `Copy every key from ${addr(p.source)}${dbs} into ${addr(p.target)} once.`
      : mode === "cdc-live"
        ? `Write changes made on ${addr(p.source)}${dbs} from now on into ${addr(p.target)}, until you stop the sync.`
        : `Write keys from ${addr(p.source)}${dbs} into ${addr(p.target)}, and keep writing changes until you stop the sync.`
  const out = emptiesFirst(o, mode, emptyFirst) ? [`Delete ${emptyScopeText(p)} first (cannot be undone).`, how] : [how]
  // Existing-key handling applies to the full copy only.
  if (mode !== "cdc-live" && !emptiesFirst(o, mode, emptyFirst) && (o.rdb_restore_command_behavior || "panic") === "rewrite") out.push(`Overwrite keys that already exist on ${addr(p.target)}.`)
  return out
}

export function problems(job: BaseJob, log: string): Problem[] {
  const out = basicProblems(job, log, "The sync stopped with an error")
  // BUSYKEY: a key exists on both sides and "When a key already exists" is "Stop the sync".
  if (out.length && /BUSYKEY/i.test(stripAnsi(log))) {
    out[0].fix = "A key already exists on the target, and Existing keys is set to “Stop the sync”. Under Configuration › Existing keys, choose “Overwrite the target key” or “Keep the target key”, or empty the target first."
  }
  return out
}

export const changeLabels: [string, (p: RedisShakePayload) => unknown][] = [
  ["name", (p) => p.name],
  ["source connection", (p) => [p.source.host, p.source.port, p.source.username, p.source.password, !!p.options.source_cluster, !!p.options.source_tls]],
  ["target connection", (p) => [p.target.host, p.target.port, p.target.username, p.target.password, !!p.options.target_cluster, !!p.options.target_tls]],
  ["start mode", (p) => [p.options.sync_rdb !== false, p.options.sync_aof !== false, !!p.options.empty_target]],
  ["databases", (p) => [p.options.source_dbs || [], targetDb(p.options) ?? null]],
  ["existing keys", (p) => p.options.rdb_restore_command_behavior || "panic"],
  ["advanced", (p) => [!!p.options.prefer_replica, !!p.options.try_diskless, !!p.options.off_reply]],
]
