// Redis sync (RedisShake): pure view rules. A sync is continuous: while running it is "Syncing",
// the normal state, until someone stops it. Unit-tested in tests/unit/transferTools.test.ts.
import { basicProblems, basicRunResult, joinList, lastMatching, stepsAt, stripAnsi, type BaseJob, type Problem, type RouteStep, type SummaryRow } from "../transferView"
import type { Conn } from "./pgcopy"

export type RedisShakePayload = { name: string; source: Conn; target: Conn; options: Record<string, any> }

export const verb = () => "Start sync"
export const stopVerb = "Stop sync"
export const startedToast = () => "Sync started"
const addr = (c: Conn) => `${c.host || "?"}:${c.port || "6379"}`

export const DUPLICATE_CHOICES = [
  { value: "panic", label: "Stop the sync", help: "The safest choice: nothing on the target is overwritten." },
  { value: "rewrite", label: "Overwrite the target key", help: "The source wins. The target's value is lost." },
  { value: "skip", label: "Keep the target key", help: "The target wins. The source's value is not copied." },
] as const

export function steps(p: RedisShakePayload, job: BaseJob | undefined, log = ""): RouteStep[] {
  const o = p.options || {}
  const defs: { key: string; label: string; detail: string }[] = []
  if (o.sync_rdb !== false) defs.push({ key: "full", label: "Full copy", detail: "every key" })
  if (o.sync_aof !== false) defs.push({ key: "live", label: "Live changes", detail: "until stopped" })
  if (!defs.length) defs.push({ key: "full", label: "Full copy", detail: "nothing selected" })
  // RedisShake reports the RDB transfer, then switches to AOF (incremental) syncing.
  const reached = defs.length > 1 ? lastMatching(stripAnsi(log), [/syncing rdb|receiving rdb|send_rdb|full sync/i, /syncing aof/i]) : 0
  return stepsAt(defs, job, reached < 0 ? 0 : reached)
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

export const runResult = (job: BaseJob) => basicRunResult(job, { continuous: true, runningText: "Syncing" })

export function summary(p: RedisShakePayload): SummaryRow[] {
  const o = p.options || {}
  const how = [o.sync_rdb !== false && "copies every key once", o.sync_aof !== false && "then keeps applying changes until you stop it"].filter(Boolean) as string[]
  const choice = DUPLICATE_CHOICES.find((c) => c.value === (o.rdb_restore_command_behavior || "panic")) || DUPLICATE_CHOICES[0]
  const kind = (cluster: boolean, tls: boolean) => [cluster ? "cluster" : "single server", tls && "TLS"].filter(Boolean).join(", ")
  return [
    { label: "Syncs", value: `${addr(p.source)} (${kind(!!o.source_cluster, !!o.source_tls)}) to ${addr(p.target)} (${kind(!!o.target_cluster, !!o.target_tls)})` },
    { label: "How", value: how.length ? `${joinList(how).replace(/^./, (c) => c.toUpperCase())}` : "Nothing: turn on the full copy or live changes" },
    { label: "Existing keys", value: `When a key already exists on the target: ${choice.label.toLowerCase()}` },
  ]
}

export function effects(p: RedisShakePayload): string[] {
  const o = p.options || {}
  const out = [`Write keys from ${addr(p.source)} into ${addr(p.target)}${o.sync_aof !== false ? ", and keep writing changes until you stop the sync" : ""}.`]
  if ((o.rdb_restore_command_behavior || "panic") === "rewrite") out.push(`Overwrite keys that already exist on ${addr(p.target)}.`)
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
  ["what to sync", (p) => [p.options.sync_rdb !== false, p.options.sync_aof !== false]],
  ["existing keys", (p) => p.options.rdb_restore_command_behavior || "panic"],
  ["advanced", (p) => [!!p.options.prefer_replica, !!p.options.try_diskless, !!p.options.off_reply]],
]
