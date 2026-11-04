// node --test --import ./tests/unit/register-ts.mjs tests/unit/*.test.ts
import assert from "node:assert/strict"
import { test } from "node:test"

import * as mongo from "../../cdc-native-ui/utils/transferTools/mongocopy.ts"
import * as pgcopy from "../../cdc-native-ui/utils/transferTools/pgcopy.ts"
import * as pgloader from "../../cdc-native-ui/utils/transferTools/pgloader.ts"
import * as redis from "../../cdc-native-ui/utils/transferTools/redisshake.ts"
import { basicProblems, changedBy, diagnose, railLine, stepsAt } from "../../cdc-native-ui/utils/transferView.ts"

const conn = (type: string, host: string, database = "shop", port = "") => ({ type, host, port, database, username: "u", password: "********" })
const states = (s: { state: string }[]) => s.map((x) => x.state)

test("stepsAt: running marks the reached step, failures mark where it stopped", () => {
  const defs = [
    { key: "a", label: "A", detail: "" },
    { key: "b", label: "B", detail: "" },
    { key: "c", label: "C", detail: "" },
  ]
  assert.deepEqual(states(stepsAt(defs, undefined, 1)), ["pending", "pending", "pending"])
  assert.deepEqual(states(stepsAt(defs, { id: "j", status: "running" }, 1)), ["done", "now", "pending"])
  assert.deepEqual(states(stepsAt(defs, { id: "j", status: "failed" }, 2)), ["done", "done", "failed"])
  assert.deepEqual(states(stepsAt(defs, { id: "j", status: "succeeded" }, 0)), ["done", "done", "done"])
})

test("diagnose names the cause in plain words", () => {
  const auth = diagnose('psql: error: FATAL:  password authentication failed for user "u"')
  assert.match(auth.fix!, /rejected the login/)
  assert.match(diagnose("ERROR 2005 (HY000): Unknown MySQL server host; getaddrinfo ENOTFOUND db").fix!, /does not resolve/)
  assert.match(diagnose("dial tcp 10.0.0.1:6379: connect: connection refused\npanic: failed").fix!, /did not accept the connection/)
  // An sslmode setting in the log is not a TLS problem.
  assert.equal(diagnose("connecting with sslmode=disable\nall good").fix, undefined)
  const p = basicProblems({ id: "j", status: "failed", exit_code: 12, error: "process exited" }, "x\nERROR relation \"a\" already exists\n")
  assert.equal(p[0].title, "The run failed (exit code 12)")
  assert.match(p[0].detail!, /already exists/)
  assert.match(p[0].fix!, /already exist on the target/)
  assert.deepEqual(basicProblems({ id: "j", status: "succeeded" }, ""), [])
  // Tool prefixes are dropped; lines carrying credentials in a URI are never quoted.
  const pg = diagnose("pgcopydb clone --source postgres://u:secret@h/db failed?\n2026-11-03 00:25:56.312 87 ERROR  string_utils.c:500        pg_dump: error: aborting because of server version mismatch")
  assert.equal(pg.line, "pg_dump: error: aborting because of server version mismatch")
  assert.match(pg.fix!, /not compatible/)
  const owner = diagnose('2026-11-03 00:30:42.736 185 ERROR  string_utils.c:500        pg_restore: error: could not execute query: ERROR:  role "src" does not exist')
  assert.match(owner.fix!, /Do not set owners/)
  const toc = diagnose("2026-11-03 00:58:15.492 59 ERROR  pgcmd.c:1710              Failed to parse Archive TOC comment or acl: bookings COLUMN airplanes_data.model demo")
  assert.match(toc.fix!, /Leave out comments/)
})

test("pgcopy: steps follow pgcopydb's STEP lines; summary and effects say what happens", () => {
  const p = { name: "seed", source: conn("postgres", "prod"), target: conn("postgres", "replica", "orders"), include_tables: [], options: { table_jobs: 4, index_jobs: 2, drop_if_exists: true } }
  const running = { id: "j", status: "running" }
  assert.deepEqual(states(pgcopy.steps(p, running, "")), ["now", "pending", "pending"])
  assert.deepEqual(states(pgcopy.steps(p, running, "STEP 1: fetch\nSTEP 4: starting 4 table-data COPY processes")), ["done", "now", "pending"])
  assert.deepEqual(states(pgcopy.steps(p, { id: "j", status: "failed" }, "STEP 5: starting 2 CREATE INDEX processes")), ["done", "done", "failed"])
  assert.equal(pgcopy.steps({ ...p, options: { ...p.options, follow: true } }, undefined, "").length, 4)
  // Failing in step 2: pg_dump's command line says "post-data", which is not the index step.
  const step2 = "STEP 1: fetch\nSTEP 2: dump the source database schema (pre/post data)\n/usr/bin/pg_dump -Fc --section=pre-data --section=post-data\nERROR pg_dump: error: aborting because of server version mismatch"
  assert.deepEqual(states(pgcopy.steps(p, { id: "j", status: "failed" }, step2)), ["failed", "pending", "pending"])
  assert.equal(pgcopy.summary(p)[1].value, "Drops objects that already exist in orders on replica, then recreates them")
  assert.match(pgcopy.effects(p)[0], /^Drop every copied object that already exists in orders on replica/)
  assert.deepEqual(changedBy(pgcopy.changeLabels, p, { ...p, options: { ...p.options, table_jobs: 8 } }), ["speed"])
  assert.equal(pgcopy.verb({ ...p, options: { follow: true } }), "Copy and follow")
})

test("pgloader: CSV is a file load that only adds rows; databases list target behaviour", () => {
  const csv = { name: "x", source: conn("csv", "", "/data/orders.csv"), target: conn("postgres", "pg", "lake"), include_tables: [], options: { target_table: "public.orders", csv_delimiter: ";", truncate: true } }
  assert.deepEqual(pgloader.steps(csv, undefined).map((s) => s.label), ["Read file", "Load rows"])
  assert.match(pgloader.effects(csv)[0], /Add the file's rows to public.orders in lake on pg\. Existing rows are kept/)
  assert.equal(pgloader.summary(csv)[2].value, "Adds the rows to the table; existing rows stay")

  const my = { ...csv, source: conn("mysql", "legacy", "shop"), options: { create_tables: true, create_indexes: false, drop_if_exists: true } }
  assert.deepEqual(pgloader.steps(my, undefined).map((s) => s.key), ["read", "create", "load"])
  assert.equal(pgloader.summary(my)[0].value, "Every table from MySQL shop on legacy")
  assert.match(pgloader.effects(my)[0], /^Drop and recreate the loaded tables/)
  assert.deepEqual(states(pgloader.steps(my, { id: "j", status: "running" }, "LOG Migrating from\nCreate tables")), ["done", "now", "pending"])
})

test("mongo: renames only when databases differ and nothing is filtered", () => {
  const p = { name: "m", source: conn("mongodb", "a", "app"), target: conn("mongodb", "b", "app_copy"), include_collections: [], options: { drop: true } }
  assert.equal(mongo.renames(p), true)
  assert.equal(mongo.renames({ ...p, include_collections: ["users"] }), false)
  assert.deepEqual(mongo.summary(p)[1], { label: "Renames", value: "app.* to app_copy.*" })
  assert.deepEqual(states(mongo.steps(p, { id: "j", status: "running" }, "writing app.users to archive\npreparing collections to restore from")), ["done", "now"])
  assert.match(mongo.effects(p)[0], /^Drop each copied collection in app_copy on b/)
})

test("redis: a running sync is the normal state; existing-key choice is spelled out", () => {
  assert.deepEqual(redis.runResult({ id: "j", status: "running" }), { tone: "ok", text: "Syncing" })
  assert.equal(railLine({ id: "j", status: "running" }, redis.runResult).text, "Syncing")
  const p = { name: "r", source: conn("redis", "cache-a", "", "6379"), target: conn("redis", "cache-b", "", ""), options: { rdb_restore_command_behavior: "rewrite" } }
  assert.deepEqual(redis.steps(p, undefined).map((s) => s.label), ["Full copy", "Live changes"])
  assert.equal(redis.summary(p)[2].value, "When a key already exists on the target: overwrite the target key")
  assert.deepEqual(redis.effects(p), [
    "Write keys from cache-a:6379 into cache-b:6379, and keep writing changes until you stop the sync.",
    "Overwrite keys that already exist on cache-b:6379.",
  ])
  assert.deepEqual(redis.steps({ ...p, options: { sync_aof: false } }, undefined).map((s) => s.key), ["full"])
  // Real RedisShake output (colour codes included): live once it reports "syncing aof".
  const log = "\x1b[90m2026-11-03 00:26:20\x1b[0m \x1b[32mINF\x1b[0m source db is not doing bgsave! continue.\n2026-11-03 00:27:50 INF read_count=[3], read_ops=[0.00], write_count=[3], write_ops=[0.00], syncing aof, diff=[0]"
  assert.deepEqual(states(redis.steps(p, { id: "j", status: "running" }, log)), ["done", "now"])
  assert.deepEqual(redis.liveStats(log), { written: 3, lag: 0 })
  assert.deepEqual(redis.liveStats("nothing yet"), {})
  const busy = redis.problems({ id: "j", status: "failed", exit_code: 1 }, "ERR [writer] redisStandaloneWriter received BUSYKEY reply. cmd=[RESTORE user:3 0 …]")
  assert.match(busy[0].fix!, /Existing keys, choose “Overwrite the target key” or “Keep the target key”/)
})
