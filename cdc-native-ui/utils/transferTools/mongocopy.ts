// Mongo copy (mongodump → mongorestore): pure view rules. Unit-tested in tests/unit/transferTools.test.ts.
import { basicProblems, basicRunResult, joinList, lastMatching, stepsAt, type BaseJob, type Problem, type RouteStep, type SummaryRow } from "../transferView"
import type { Conn } from "./pgcopy"

export type MongoCopyPayload = { name: string; source: Conn; target: Conn; include_collections: string[]; options: Record<string, any> }

export const verb = () => "Copy MongoDB"
export const startedToast = () => "Copy started"
const where = (c: Conn) => `${c.database || "every database"} on ${c.host || "?"}`

/** mongorestore maps source.* to target.* only when both databases are set, differ, and no collections are picked. */
export const renames = (p: MongoCopyPayload) =>
  !!(p.source.database && p.target.database && p.source.database !== p.target.database && !p.include_collections?.length)

export function steps(p: MongoCopyPayload, job: BaseJob | undefined, log = ""): RouteStep[] {
  const reached = lastMatching(log, [/writing .* to archive|done dumping|dumping up to/i, /preparing collections to restore|restoring|finished restoring|document\(s\) restored/i])
  return stepsAt(
    [
      { key: "dump", label: "Dump", detail: p.options.gzip === false ? "archive" : "gzip archive" },
      { key: "restore", label: "Restore", detail: p.options.drop ? "drop and replace" : "add documents" },
    ],
    job,
    reached < 0 ? 0 : reached,
  )
}

export const runResult = (job: BaseJob) => basicRunResult(job)

export function summary(p: MongoCopyPayload): SummaryRow[] {
  const cols = p.include_collections || []
  const rows: SummaryRow[] = [
    {
      label: "Copies",
      value: cols.length ? `Only collections ${joinList(cols)}` : p.source.database ? `Every collection in ${p.source.database}` : "Every database on the source server",
    },
  ]
  if (renames(p)) rows.push({ label: "Renames", value: `${p.source.database}.* to ${p.target.database}.*` })
  rows.push({
    label: "On the target",
    value: p.options.drop
      ? `Drops each copied collection in ${where(p.target)}, then restores it`
      : `Adds documents to ${where(p.target)}; documents whose _id already exists are skipped and reported`,
  })
  if (p.options.auth_source) rows.push({ label: "Sign-in", value: `Users are checked in the ${p.options.auth_source} database on both servers` })
  return rows
}

export function effects(p: MongoCopyPayload): string[] {
  return [
    p.options.drop
      ? `Drop each copied collection in ${where(p.target)}, then restore it from the source.`
      : `Insert documents into ${where(p.target)}.`,
  ]
}

export const problems = (job: BaseJob, log: string): Problem[] => basicProblems(job, log, "The copy failed")

export const changeLabels: [string, (p: MongoCopyPayload) => unknown][] = [
  ["name", (p) => p.name],
  ["source connection", (p) => [p.source.host, p.source.port, p.source.database, p.source.username, p.source.password]],
  ["target connection", (p) => [p.target.host, p.target.port, p.target.database, p.target.username, p.target.password]],
  ["collections", (p) => p.include_collections || []],
  ["target behaviour", (p) => !!p.options.drop],
  ["archive", (p) => p.options.gzip !== false],
  ["authentication database", (p) => p.options.auth_source || null],
]
