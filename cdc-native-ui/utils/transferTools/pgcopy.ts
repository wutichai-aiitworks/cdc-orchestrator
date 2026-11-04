// Postgres copy (pgcopydb clone): pure view rules. Unit-tested in tests/unit/transferTools.test.ts.
import { formatBytes } from "../bytes"
import { basicProblems, basicRunResult, joinList, lastMatching, stepsAt, type BaseJob, type Problem, type RouteStep, type SummaryRow } from "../transferView"

export type Conn = { type: string; host: string; port: string; database: string; username: string; password: string; schema?: string }
export type PgCopyPayload = { name: string; source: Conn; target: Conn; include_tables: string[]; options: Record<string, any> }

export const verb = (p: PgCopyPayload) => (p.options.follow ? "Copy and follow" : "Copy database")
export const startedToast = (p: PgCopyPayload) => (p.options.follow ? "Copy started; changes will follow" : "Copy started")
const where = (c: Conn) => `${c.database || "?"} on ${c.host || "?"}`

// pgcopydb logs numbered steps: 1-3 schema, 4 data, 5-9 indexes, vacuum, sequences, post-data.
// Phrases are only a fallback: pg_dump's own command line mentions "post-data" during step 2.
const FOLLOW = /follow mode|logical decoding|streaming changes|apply (changes|transactions)/i
const PHRASES = [/fetch(ed)? .*tables/i, /table-data|COPY processes/i, /CREATE INDEX processes|VACUUM processes|reset sequences/i]

export function reachedStep(log: string, follow: boolean): number {
  const nums = [...log.matchAll(/\bSTEP (\d+)\b/g)].map((m) => Number(m[1]))
  let reached = nums.length ? (Math.max(...nums) <= 3 ? 0 : Math.max(...nums) === 4 ? 1 : 2) : lastMatching(log, PHRASES)
  if (follow && FOLLOW.test(log)) reached = 3
  return reached
}

export function steps(p: PgCopyPayload, job: BaseJob | undefined, log = ""): RouteStep[] {
  const o = p.options || {}
  const defs = [
    { key: "schema", label: "Schema", detail: o.drop_if_exists ? "drop and create" : "create" },
    { key: "data", label: "Data", detail: `${o.table_jobs || 1} tables at a time` },
    { key: "indexes", label: "Indexes", detail: `${o.index_jobs || 1} at a time` },
  ]
  if (o.follow) defs.push({ key: "follow", label: "Follow changes", detail: o.plugin || "test_decoding" })
  const reached = reachedStep(log, !!o.follow)
  return stepsAt(defs, job, reached < 0 ? 0 : reached)
}

export const runResult = (job: BaseJob) => basicRunResult(job)

export function summary(p: PgCopyPayload): SummaryRow[] {
  const o = p.options || {}
  const rows: SummaryRow[] = []
  let scope = o.all_databases ? "Every database on the source server" : p.include_tables?.length ? `Only tables ${joinList(p.include_tables)}` : `The whole database ${p.source.database || "?"}`
  const extra: string[] = []
  if (o.roles) extra.push("roles")
  if (!o.skip_large_objects) extra.push("large objects")
  if (!o.skip_extensions) extra.push("extensions")
  if (extra.length) scope += `, with ${joinList(extra)}`
  rows.push({ label: "Copies", value: scope })
  rows.push({
    label: "On the target",
    value: `${o.drop_if_exists ? `Drops objects that already exist in ${where(p.target)}, then recreates them` : `Creates objects in ${where(p.target)}; stops if they already exist`}${o.no_owner ? ", owned by the target user" : ""}`,
  })
  rows.push({
    label: "Speed",
    value: `${o.table_jobs || 1} tables and ${o.index_jobs || 1} indexes at a time${o.split_tables_larger_than ? `; tables over ${formatBytes(o.split_tables_larger_than)} are split` : ""}`,
  })
  const skipped = [o.skip_vacuum && "VACUUM", o.skip_analyze && "ANALYZE"].filter(Boolean) as string[]
  if (skipped.length) rows.push({ label: "Skips", value: `${joinList(skipped)} after loading` })
  if (o.follow) rows.push({ label: "Afterwards", value: `Keeps applying changes from the source (${o.plugin || "test_decoding"}) until you stop it` })
  return rows
}

export function effects(p: PgCopyPayload): string[] {
  const o = p.options || {}
  const out = [
    o.drop_if_exists
      ? `Drop every copied object that already exists in ${where(p.target)}, then recreate it.`
      : `Create schemas, tables and indexes and load rows into ${where(p.target)}.`,
  ]
  if (o.roles) out.push(`Create roles on the target server ${p.target.host || "?"}.`)
  if (o.follow && o.create_slot) out.push(`Create a replication slot on ${p.source.host || "the source"}; it keeps WAL until the slot is dropped.`)
  if (o.all_databases) out.push("Copy every database on the source server, not just one.")
  return out
}

export const problems = (job: BaseJob, log: string): Problem[] => basicProblems(job, log, "The copy failed")

export const changeLabels: [string, (p: PgCopyPayload) => unknown][] = [
  ["name", (p) => p.name],
  ["source connection", (p) => [p.source.host, p.source.port, p.source.database, p.source.username, p.source.password]],
  ["target connection", (p) => [p.target.host, p.target.port, p.target.database, p.target.username, p.target.password]],
  ["tables", (p) => p.include_tables || []],
  ["what to copy", (p) => ["roles", "no_role_passwords", "skip_large_objects", "skip_extensions", "all_databases"].map((k) => p.options[k] ?? null)],
  ["target behaviour", (p) => ["drop_if_exists", "no_owner", "no_acl", "no_comments", "no_tablespaces", "skip_db_properties"].map((k) => p.options[k] ?? null)],
  ["speed", (p) => ["table_jobs", "index_jobs", "restore_jobs", "large_objects_jobs", "split_tables_larger_than", "split_max_parts", "use_copy_binary", "skip_split_by_ctid", "estimate_table_sizes"].map((k) => p.options[k] ?? null)],
  ["after loading", (p) => [!!p.options.skip_vacuum, !!p.options.skip_analyze]],
  ["follow changes", (p) => ["follow", "create_slot", "plugin", "slot_name", "origin", "endpos", "wal2json_numeric_as_string"].map((k) => p.options[k] ?? null)],
  ["recovery", (p) => ["fail_fast", "restart", "resume", "not_consistent", "snapshot"].map((k) => p.options[k] ?? null)],
  ["advanced", (p) => ["skip_collations", "skip_ext_comments", "requirements", "filters_file"].map((k) => p.options[k] ?? null)],
]
