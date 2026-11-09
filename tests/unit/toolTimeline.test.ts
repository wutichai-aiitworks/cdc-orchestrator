// node --test --import ./tests/unit/register-ts.mjs tests/unit/*.test.ts
import assert from "node:assert/strict"
import { test } from "node:test"

import * as tl from "../../cdc-native-ui/utils/toolTimeline.ts"

const ev = (kind: string, status: string | null, extra: Partial<tl.ToolEvent> = {}): tl.ToolEvent => ({
  id: `${kind}-${status}`,
  timestamp: "2026-11-06T09:00:00.000Z",
  tool: "pgcopy",
  kind,
  status,
  actor: "woody",
  summary: "s",
  detail: {},
  ...extra,
})

test("timeline: tones say what needs a look", () => {
  assert.equal(tl.eventTone(ev("run.finished", "succeeded")), "ok")
  assert.equal(tl.eventTone(ev("run.finished", "failed")), "err")
  assert.equal(tl.eventTone(ev("run.finished", "stopped")), "warn")
  assert.equal(tl.eventTone(ev("run.start_failed", "failed")), "err")
  assert.equal(tl.eventTone(ev("connection.tested", "ok")), "ok")
  assert.equal(tl.eventTone(ev("connection.tested", "failed")), "err")
  assert.equal(tl.eventTone(ev("profile.deleted", null)), "warn")
  assert.equal(tl.eventTone(ev("run.started", "started")), "run")
  assert.equal(tl.eventTone(ev("profile.updated", null)), "idle")
})

test("timeline: filters and search", () => {
  const all = [ev("run.finished", "failed", { detail: { error: "relation x does not exist" } }), ev("profile.updated", null), ev("connection.tested", "ok")]
  assert.deepEqual(all.filter((e) => tl.matchesFilter(e, "runs")).map((e) => e.kind), ["run.finished"])
  assert.deepEqual(all.filter((e) => tl.matchesFilter(e, "changes")).map((e) => e.kind), ["profile.updated"])
  assert.deepEqual(all.filter((e) => tl.matchesFilter(e, "failures")).map((e) => e.kind), ["run.finished"])
  assert.equal(tl.matchesSearch(all[0], "does not exist"), true) // errors inside detail are searchable
  assert.equal(tl.matchesSearch(all[1], "WOODY"), true)
  assert.equal(tl.matchesSearch(all[1], "nobody"), false)
})

test("timeline: day groups keep newest-first order and name today/yesterday", () => {
  const now = new Date("2026-11-06T12:00:00")
  const at = (local: string) => ev("run.started", "started", { id: local, timestamp: new Date(local).toISOString() })
  const groups = tl.groupByDay([at("2026-11-06T10:00:00"), at("2026-11-06T08:00:00"), at("2026-11-05T23:00:00"), at("2026-11-01T09:00:00")], now)
  assert.deepEqual(groups.map((g) => [g.label === "Today" || g.label === "Yesterday" ? g.label : "date", g.events.length]), [
    ["Today", 2],
    ["Yesterday", 1],
    ["date", 1],
  ])
})

test("timeline: changes and run facts read naturally; secrets stay hidden", () => {
  const upd = ev("profile.updated", null, {
    detail: { changes: [{ field: "source.port", before: "6379", after: "6579" }, { field: "source.password", secret: true }, { field: "options.source_dbs", before: [], after: [2] }] },
  })
  assert.deepEqual(tl.changeRows(upd), [
    { field: "source.port", before: "6379", after: "6579", secret: false },
    { field: "source.password", before: "hidden", after: "changed", secret: true },
    { field: "options.source_dbs", before: "—", after: "2", secret: false },
  ])
  const fin = ev("run.finished", "failed", { detail: { exit_code: 12, duration_secs: 192, stats: { files: 1200, nested: { a: 1 } } } })
  assert.deepEqual(tl.runFacts(fin).map((r) => [r.label, r.value]), [
    ["Status", "failed"],
    ["Took", "3 m 12 s"],
    ["Exit code", "12"],
    ["files", (1200).toLocaleString()],
  ])
  assert.equal(tl.hasDetail(ev("profile.updated", null, { detail: { changes: [] } })), false)
  assert.equal(tl.hasDetail(fin), true)
})

test("timeline: CSV quotes commas, quotes and newlines", () => {
  const csv = tl.toCsv([ev("run.finished", "failed", { summary: 'Run failed: "x", y\nz', profile_name: "a,b", job_id: "j1", detail: { exit_code: 1 } })])
  const [head, row] = csv.trimEnd().split("\r\n")
  assert.equal(head, "timestamp_utc,tool,profile,event,status,actor,job_id,summary,detail")
  assert.equal(row, '2026-11-06T09:00:00.000Z,pgcopy,"a,b",Run finished,failed,woody,j1,"Run failed: ""x"", y\nz","{""exit_code"":1}"')
})
