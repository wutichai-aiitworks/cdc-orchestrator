// AI vector sync editor rules (cdc-native-ui/utils/vectorSync.ts) — ADR 0018.
import test from "node:test"
import assert from "node:assert/strict"

import {
  badgeStatus,
  emptyVsDraft,
  emptyTable,
  fmtCount,
  fromProfile,
  rebuildReasons,
  sensitiveColumns,
  slug,
  stepProblems,
  suggestTemplate,
  templateError,
  templateFields,
  vsToRequest,
  unknownTemplateFields,
} from "../../cdc-native-ui/utils/vectorSync.ts"

test("sensitive columns are excluded by default", () => {
  assert.deepEqual(sensitiveColumns(["id", "email", "password_hash", "api_key", "reset_token", "note"]), ["password_hash", "api_key", "reset_token"])
  assert.deepEqual(emptyTable(["name", "pwd"]).exclude_fields, ["pwd"])
})

test("template rules match Python string.Template (and the orchestrator)", () => {
  assert.equal(templateError("$name — ${city} costs $$5"), null)
  for (const bad of ["cost $", "${}", "${1a}", "$1", "${a"]) assert.ok(templateError(bad), bad)
  assert.deepEqual(templateFields("$a ${b} $$c $a"), ["a", "b"])
  assert.deepEqual(unknownTemplateFields("$a $zzz", ["a", "b"]), ["zzz"])
  assert.deepEqual(unknownTemplateFields("$zzz", []), [], "no columns known yet → no false alarm")
})

test("suggested template skips ids, timestamps and excluded columns", () => {
  assert.equal(suggestTemplate(["id", "customer_id", "name", "city", "secret", "created_at"], ["secret"]), "name: ${name}\ncity: ${city}")
})

test("slug makes a valid collection name", () => {
  assert.equal(slug("Orders — Search (EU)"), "orders_search_eu")
  assert.equal(slug("!!!"), "vectors")
  assert.equal(slug("x".repeat(80)).length, 63)
})

test("draft → request → draft round trip keeps per-table content modes", () => {
  const d = emptyVsDraft()
  d.name = " Orders search "
  d.topics = ["demo.public.orders", "demo.public.customers"]
  d.tables.orders = { ...emptyTable(), mode: "template", content_template: "Order $id", exclude_fields: ["secret"] }
  d.tables.customers = { ...emptyTable(), mode: "pick", include_fields: ["name"], metadata_fields: ["name", "secret"], exclude_fields: ["secret"] }
  d.target = { host: "h", port: "5432", database: "db", username: "u", password: "p", sslmode: "", collection: "orders" }
  const req = vsToRequest(d)
  assert.equal(req.name, "Orders search")
  assert.deepEqual(req.projection.tables.orders, { exclude_fields: ["secret"], content_template: "Order $id" })
  assert.deepEqual(req.projection.tables.customers, { exclude_fields: ["secret"], include_fields: ["name"], metadata_fields: ["name"] })
  assert.equal(req.embedding.api_key, "", "API key only sent for OpenAI-compatible servers")
  const back = fromProfile({ ...req, id: "x" })
  assert.equal(back.tables.orders.mode, "template")
  assert.equal(back.tables.customers.mode, "pick")
})

test("rebuild is flagged when the model or collection changes", () => {
  const a = emptyVsDraft()
  a.target.collection = "c1"
  const b = structuredClone(a)
  assert.deepEqual(rebuildReasons(a, b), [])
  b.embedding.model = "nomic-embed-text"
  b.target.collection = "c2"
  assert.equal(rebuildReasons(a, b).length, 2)
  assert.deepEqual(rebuildReasons(null, b), [])
})

test("step problems guide the wizard", () => {
  const d = emptyVsDraft()
  assert.ok(stepProblems(d, "source").length)
  d.topics = ["demo.public.orders"]
  assert.equal(stepProblems(d, "source").length, 0)
  assert.ok(stepProblems(d, "store").some((p) => p.includes("Host")))
  d.tables.orders = { ...emptyTable(), mode: "template", content_template: "$nope" }
  assert.ok(stepProblems(d, "documents", { orders: ["id"] }).some((p) => p.includes("$nope")))
})

test("display helpers", () => {
  assert.equal(badgeStatus("retrying"), "degraded")
  assert.equal(badgeStatus(undefined), "unknown")
  assert.equal(fmtCount(1234), "1,234")
  assert.equal(fmtCount(45_600), "45.6k")
  assert.equal(fmtCount(null), "—")
})

test("Docling document mode and chunking round-trip and validate", () => {
  const d = emptyVsDraft()
  d.name = "Manuals"
  d.topics = ["demo.public.manuals", "demo.public.notes"]
  d.tables.manuals = { ...emptyTable(["id", "file", "fname"]), mode: "document", document_column: "file", filename_column: "fname", include_row_text: true, chunking: { enabled: false, max_chars: 1200, overlap: 100 } }
  d.tables.notes = { ...emptyTable(["id", "body"]), chunking: { enabled: true, max_chars: 800, overlap: 80 } }
  const req = vsToRequest(d)
  assert.deepEqual(req.projection.tables.manuals, { document: { column: "file", filename_column: "fname", include_row_text: true }, chunking: { enabled: true, max_chars: 1200, overlap: 100 } })
  assert.deepEqual(req.projection.tables.notes, { chunking: { enabled: true, max_chars: 800, overlap: 80 } })
  const back = fromProfile({ ...req, id: "x" })
  assert.equal(back.tables.manuals.mode, "document")
  assert.equal(back.tables.manuals.document_column, "file")
  assert.equal(back.tables.notes.chunking.enabled, true)
  // Docling missing in the sidecar → the documents step says how to turn it on
  assert.ok(stepProblems(d, "documents", {}, false).some((p) => p.includes("VECTORSYNC_DOCLING")))
  assert.equal(stepProblems(d, "documents", {}, true).length, 0)
  d.tables.notes.chunking.max_chars = 50
  assert.ok(stepProblems(d, "documents", {}, true).some((p) => p.includes("200–20000")))
})
