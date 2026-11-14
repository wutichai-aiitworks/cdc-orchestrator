// Database version display and live CDC readiness helpers (cdc-native-ui/utils/cdcReadiness.ts).
import test from "node:test"
import assert from "node:assert/strict"

import { formatServer, readinessHeadline, recentlyChanged, sortChecks, versionLine, versionTitle, type ReadinessReport } from "../../cdc-native-ui/utils/cdcReadiness.ts"

test("formatServer: one readable line per engine", () => {
  assert.equal(formatServer({ product: "PostgreSQL", version: "17.6 (Debian 17.6-1.pgdg12+1)" }), "PostgreSQL 17.6")
  assert.equal(formatServer({ product: "MySQL", version: "8.0.36" }), "MySQL 8.0.36")
  assert.equal(formatServer({ product: "MySQL", version: "5.7.44-log" }), "MySQL 5.7.44")
  assert.equal(formatServer({ product: "MySQL", version: "10.11.6-MariaDB-1:10.11.6+maria~ubu2204" }), "MariaDB 10.11.6")
  assert.equal(formatServer({ product: "Microsoft SQL Server", version: "16.00.4135", major: 16 }), "SQL Server 2022 (16.0.4135)")
  assert.equal(
    formatServer({ product: "Oracle", version: "Oracle Database 19c Enterprise Edition Release 19.0.0.0.0 - Production\nVersion 19.3.0.0.0" }),
    "Oracle Database 19c Enterprise Edition",
  )
  assert.equal(formatServer(null), "")
  assert.equal(formatServer({ product: "H2", version: "" }), "H2")
})

const report = (summary: ReadinessReport["summary"], applicable = true): ReadinessReport => ({
  engine: "postgres", capture_mode: "logical", applicable, server: null, platform: "self_managed",
  checks: [], parameters: [], fix_script: "", summary, ready: summary.fail === 0, checked_at: "",
})

test("readinessHeadline: failures beat unknowns beat warnings", () => {
  assert.equal(readinessHeadline(report({ pass: 3, warn: 1, fail: 2, unknown: 1 })).tone, "error")
  assert.match(readinessHeadline(report({ pass: 3, warn: 0, fail: 0, unknown: 2 })).text, /could not be read/)
  assert.equal(readinessHeadline(report({ pass: 3, warn: 1, fail: 0, unknown: 0 })).tone, "warning")
  assert.equal(readinessHeadline(report({ pass: 5, warn: 0, fail: 0, unknown: 0 })).text, "Ready for CDC.")
  assert.equal(readinessHeadline(report({ pass: 0, warn: 0, fail: 0, unknown: 0 }, false)).tone, "info")
})

test("sortChecks puts failures first", () => {
  const ids = sortChecks([
    { id: "a", title: "", status: "pass", detail: "" },
    { id: "b", title: "", status: "unknown", detail: "" },
    { id: "c", title: "", status: "fail", detail: "" },
  ]).map((c) => c.id)
  assert.deepEqual(ids, ["c", "b", "a"])
})

test("versionTitle and recentlyChanged describe the stored version", () => {
  const now = Date.parse("2026-11-14T12:00:00Z")
  const s = { product: "PostgreSQL", version: "17.7", checked_at: "2026-11-14T11:50:00Z", changed_at: "2026-11-13T12:00:00Z", previous: "PostgreSQL 17.6", error: null }
  assert.equal(versionTitle(s, now), "Checked 10 min ago\nChanged from PostgreSQL 17.6 1 d ago")
  assert.ok(recentlyChanged(s, now))
  assert.ok(!recentlyChanged({ ...s, changed_at: "2026-10-01T00:00:00Z" }, now))
  assert.match(versionTitle({ ...s, error: "Connection refused" }, now), /last known version/)
  assert.equal(versionTitle(null), "")
})

test("versionLine: source → target, or source only for non-database targets", () => {
  const v = { source: { product: "PostgreSQL", version: "16.15" }, target: { product: "PostgreSQL", version: "17.7 (Debian)" } }
  assert.equal(versionLine(v, true), "PostgreSQL 16.15 → PostgreSQL 17.7")
  assert.equal(versionLine(v, false), "PostgreSQL 16.15")
  assert.equal(versionLine({ target: v.target }, true), "? → PostgreSQL 17.7")
  assert.equal(versionLine(null, true), "")
})
