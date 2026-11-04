// Load to Postgres (pgloader): pure view rules. Unit-tested in tests/unit/transferTools.test.ts.
import { basicProblems, basicRunResult, joinList, lastMatching, stepsAt, type BaseJob, type Problem, type RouteStep, type SummaryRow } from "../transferView"
import type { Conn } from "./pgcopy"

export type PgLoaderPayload = { name: string; source: Conn; target: Conn; include_tables: string[]; options: Record<string, any> }

export const ENGINES = [
  { value: "mysql", label: "MySQL", port: "3306", file: false },
  { value: "mariadb", label: "MariaDB", port: "3306", file: false },
  { value: "mssql", label: "SQL Server", port: "1433", file: false },
  { value: "sqlite", label: "SQLite file", port: "", file: true },
  { value: "csv", label: "CSV file", port: "", file: true },
] as const

export const engineOf = (type: string) => ENGINES.find((e) => e.value === type) || ENGINES[0]
export const isFile = (p: { source: { type: string } }) => engineOf(p.source.type).file
export const isCsv = (p: { source: { type: string } }) => p.source.type === "csv"

export const verb = () => "Load into Postgres"
export const startedToast = () => "Load started"
const where = (c: Conn) => `${c.database || "?"} on ${c.host || "?"}`
export const sourceLabel = (p: PgLoaderPayload) => (isFile(p) ? p.source.database || "?" : `${p.source.database || "?"} on ${p.source.host || "?"}`)

// pgloader prints its phases in the summary table and in LOG lines.
const PHASES = [
  /fetch meta data|Migrating from|LOAD (DATABASE|CSV)/i,
  /Create (Schemas|tables|SQL Types)/i,
  /\bCOPY\b|COPY Threads Completion|rows? (copied|loaded)/i,
  /Create Indexes|Index Build Completion|Primary Keys|Reset Sequences|Total import time/i,
]

export function steps(p: PgLoaderPayload, job: BaseJob | undefined, log = ""): RouteStep[] {
  const o = p.options || {}
  if (isCsv(p)) {
    const reached = lastMatching(log, [PHASES[0], PHASES[2]])
    return stepsAt(
      [
        { key: "read", label: "Read file", detail: o.csv_delimiter && o.csv_delimiter !== "," ? `delimiter ${o.csv_delimiter}` : "CSV" },
        { key: "load", label: "Load rows", detail: o.target_table || "target table" },
      ],
      job,
      reached < 0 ? 0 : reached,
    )
  }
  const defs: { key: string; label: string; detail: string }[] = [{ key: "read", label: "Read source", detail: engineOf(p.source.type).label }]
  const pats = [PHASES[0]]
  if (o.create_tables !== false) {
    defs.push({ key: "create", label: "Create tables", detail: o.drop_if_exists ? "drop first" : "if missing" })
    pats.push(PHASES[1])
  }
  defs.push({ key: "load", label: "Load rows", detail: o.workers ? `${o.workers} workers` : o.truncate ? "after truncate" : "" })
  pats.push(PHASES[2])
  if (o.create_indexes !== false) {
    defs.push({ key: "indexes", label: "Indexes", detail: o.reset_sequences !== false ? "and sequences" : "" })
    pats.push(PHASES[3])
  }
  const reached = lastMatching(log, pats)
  return stepsAt(defs, job, reached < 0 ? 0 : reached)
}

export const runResult = (job: BaseJob) => basicRunResult(job)

export function summary(p: PgLoaderPayload): SummaryRow[] {
  const o = p.options || {}
  if (isCsv(p)) {
    return [
      { label: "Loads", value: `The file ${p.source.database || "?"} into table ${o.target_table || "?"} in ${where(p.target)}` },
      { label: "Format", value: `Fields separated by “${o.csv_delimiter || ","}”, skipping ${o.csv_skip_header ?? 1} header line${(o.csv_skip_header ?? 1) === 1 ? "" : "s"}` },
      // The sidecar's CSV load ignores truncate/drop: rows are always added.
      { label: "On the target", value: "Adds the rows to the table; existing rows stay" },
    ]
  }
  const rows: SummaryRow[] = [
    {
      label: "Loads",
      value: `${p.include_tables?.length ? `Tables matching ${joinList(p.include_tables)}` : "Every table"} from ${engineOf(p.source.type).label} ${sourceLabel(p)}`,
    },
  ]
  const target: string[] = []
  if (o.drop_if_exists) target.push("drops tables that already exist")
  if (o.create_tables !== false) target.push("creates the tables")
  else target.push("loads into existing tables")
  if (o.truncate) target.push("empties them first")
  if (o.create_indexes !== false) target.push("builds indexes")
  if (o.reset_sequences !== false) target.push("resets sequences")
  const text = joinList(target)
  rows.push({ label: "On the target", value: `In ${where(p.target)}: ${text}` })
  if (o.workers || o.concurrency) rows.push({ label: "Speed", value: `${o.workers || "default"} workers, ${o.concurrency || "default"} concurrent loads` })
  return rows
}

export function effects(p: PgLoaderPayload): string[] {
  const o = p.options || {}
  const t = where(p.target)
  if (isCsv(p)) {
    return [`Add the file's rows to ${o.target_table || "the table"} in ${t}. Existing rows are kept, so running twice loads them twice.`]
  }
  const out: string[] = []
  if (o.drop_if_exists) out.push(`Drop and recreate the loaded tables that already exist in ${t}.`)
  else if (o.truncate) out.push(`Empty the loaded tables in ${t} (TRUNCATE) before loading.`)
  else out.push(`Create tables and load rows into ${t}.`)
  return out
}

export const problems = (job: BaseJob, log: string): Problem[] => basicProblems(job, log, "The load failed")

export const changeLabels: [string, (p: PgLoaderPayload) => unknown][] = [
  ["name", (p) => p.name],
  ["source", (p) => [p.source.type, p.source.host, p.source.port, p.source.database, p.source.username, p.source.password]],
  ["target connection", (p) => [p.target.host, p.target.port, p.target.database, p.target.username, p.target.password]],
  ["tables", (p) => p.include_tables || []],
  ["CSV format", (p) => (isCsv(p) ? [p.options.target_table, p.options.csv_delimiter, p.options.csv_skip_header] : null)],
  ["target behaviour", (p) => ["drop_if_exists", "create_tables", "create_indexes", "reset_sequences", "truncate"].map((k) => p.options[k] ?? null)],
  ["speed", (p) => [p.options.workers ?? null, p.options.concurrency ?? null]],
]
