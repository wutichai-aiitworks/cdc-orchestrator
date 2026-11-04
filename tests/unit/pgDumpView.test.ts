// node --test --import ./tests/unit/register-ts.mjs tests/unit/*.test.ts
import assert from "node:assert/strict"
import { test } from "node:test"

import {
  changedFields,
  destructiveEffects,
  railStatus,
  routeSteps,
  runProblems,
  runResult,
  runVerb,
  summarize,
  type JobLike,
  type ProfileLike,
} from "../../cdc-native-ui/utils/pgDumpView.ts"
import { formatDuration, relativeTime, startedLabel } from "../../cdc-native-ui/utils/transferView.ts"

const NOW = Date.parse("2026-11-03T10:00:00Z")
const src = { host: "shop-db", port: "5432", database: "shop", username: "backup_ro", password: "********" }
const tgt = { host: "replica-01", port: "5432", database: "orders", username: "writer", password: "********" }

const dump = (over: Partial<ProfileLike> = {}): ProfileLike => ({
  name: "Shop nightly",
  mode: "dump",
  source: { ...src },
  include_tables: [],
  options: { format: "custom", compress: 6 },
  destinations: [],
  ...over,
})
const restore = (options: Record<string, unknown> = {}): ProfileLike =>
  dump({ mode: "dump_restore", target: { ...tgt }, options: { format: "custom", ...options } })

test("durations and relative times read like a person wrote them", () => {
  assert.equal(formatDuration(11_000), "11 s")
  assert.equal(formatDuration(252_000), "4 m 12 s")
  assert.equal(formatDuration(3_780_000), "1 h 03 m")
  assert.equal(formatDuration(null), "—")
  assert.equal(relativeTime("2026-11-03T09:59:40Z", NOW), "just now")
  assert.equal(relativeTime("2026-11-03T08:00:00Z", NOW), "2 h ago")
  assert.equal(relativeTime("2026-11-02T08:00:00Z", NOW), "yesterday")
  assert.equal(relativeTime("2026-10-28T08:00:00Z", NOW), "28 Oct")
  // Labels are local time; build inputs in local time so the test passes in any time zone.
  const at = (dayOffset: number, h: number) => {
    const n = new Date(NOW)
    return new Date(n.getFullYear(), n.getMonth(), n.getDate() + dayOffset, h, 5).toISOString()
  }
  assert.equal(startedLabel(at(0, 9), NOW), "Today 09:05")
  assert.equal(startedLabel(at(-1, 2), NOW), "Yesterday 02:05")
  assert.match(startedLabel(at(3, 9), NOW), /^\d{1,2} [A-Z][a-z]{2} 09:05$/, "future dates (file expiry) are never 'Today'")
})

test("run results name the actual problem", () => {
  assert.deepEqual(runResult({ id: "1", status: "running", phase: "restore" }), { tone: "run", text: "Running, restoring" })
  const upFail: JobLike = { id: "2", status: "completed_with_errors", uploads: [{ name: "backup-sg", status: "failed" }] }
  assert.equal(runResult(upFail).text, "Upload failed")
  assert.equal(runResult({ id: "3", status: "completed_with_errors", errors_ignored: 3 }).text, "Restored with 3 errors")
  assert.deepEqual(railStatus(undefined), { tone: "idle", text: "Never run" })
  assert.equal(
    railStatus({ id: "4", status: "succeeded", finished_at: "2026-11-03T08:00:00Z" }, NOW).text,
    "Succeeded 2 h ago",
  )
  assert.equal(runVerb(restore()), "Run dump and restore")
})

test("route steps follow the profile, then the job", () => {
  const p = dump({ destinations: [{ name: "db-host", host: "10.0.4.12", remote_dir: "/var/backups/pg" }] })
  assert.deepEqual(routeSteps(p).map((s) => [s.key, s.state, s.detail]), [
    ["dump", "pending", "custom, level 6"],
    ["upload", "pending", "1 server"],
  ])
  const running = routeSteps(p, { id: "j", status: "running", phase: "upload", started_at: "2026-11-03T09:59:00Z" }, NOW)
  assert.deepEqual(running.map((s) => s.state), ["done", "now"])
  assert.equal(running[1].detail, "1 m 00 s")

  const failedUpload = routeSteps(p, {
    id: "j",
    status: "completed_with_errors",
    tables_dumped: 9,
    artifact: { name: "a.dump", format: "custom", bytes: 139_130_033 },
    uploads: [{ name: "db-host", status: "failed" }],
  })
  assert.deepEqual(failedUpload.map((s) => [s.state, s.detail]), [
    ["done", "9 tables, 133 MB"],
    ["failed", "1 of 1 failed"],
  ])

  const dumpFailed = routeSteps(restore(), { id: "j", status: "failed" })
  assert.deepEqual(dumpFailed.map((s) => s.state), ["failed", "pending"])
})

test("summary describes scope, target behaviour and the file", () => {
  const s = summarize(
    dump({
      options: { format: "custom", compress: 6, schemas: ["sales", "ops"], exclude_table_data: ["ops.audit_log"] },
      destinations: [{ name: "backup-sg", host: "b", remote_dir: "/srv" }],
    }),
  )
  assert.deepEqual(s, [
    { label: "Copies", value: "Only schemas sales and ops" },
    { label: "Skips", value: "rows of ops.audit_log" },
    { label: "File", value: "Custom archive, compression 6; uploaded to backup-sg, then removed from the dump volume" },
  ])
  const r = summarize(restore({ restore_clean: true, restore_jobs: 4 }))
  assert.equal(r[1].value, "Drops the archived objects in orders on replica-01, then recreates them, 4 tables at a time; skips objects that fail and reports them")
})

test("only restores into a target need a confirmation", () => {
  assert.deepEqual(destructiveEffects(dump()), [])
  assert.match(destructiveEffects(restore({ restore_clean: true }))[0], /^Drop every object .* orders on replica-01/)
  assert.match(destructiveEffects(restore())[0], /^Create objects and load rows into orders on replica-01/)
})

test("changed fields ignore empty-vs-missing noise", () => {
  const saved = dump()
  assert.deepEqual(changedFields(saved, dump({ options: { format: "custom", compress: 6, no_owner: false, schemas: [] } })), [])
  assert.deepEqual(changedFields(saved, dump({ options: { format: "plain", compress: 6, schemas: ["sales"] } })), ["schemas", "format"])
  assert.deepEqual(changedFields(null, dump()), ["new transfer"])
})

test("problems carry a fix", () => {
  const p = runProblems({
    id: "j",
    status: "completed_with_errors",
    uploads: [{ name: "backup-sg", status: "failed", error: "PermissionError", hint: "Give the SFTP user write access." }],
  })
  assert.deepEqual(p, [{ title: "Upload to backup-sg failed", detail: "PermissionError", fix: "Give the SFTP user write access.", upload: "backup-sg" }])
  assert.equal(runProblems({ id: "k", status: "failed", error: "pg_dump exited 1", hint: "Authentication failed" })[0].fix, "Authentication failed")
})
