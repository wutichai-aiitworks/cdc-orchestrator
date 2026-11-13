// Live log viewer line parser (cdc-native-ui/utils/logLine.ts), against real lines from the CDC stack.
import test from "node:test"
import assert from "node:assert/strict"

import { logfmtFields, parseLogLine } from "../../cdc-native-ui/utils/logLine.ts"

test("vector: span noise moves to fields; component_id is the source; message starts after the target", () => {
  const p = parseLogLine(
    '2026-11-12T14:44:54.182135Z  INFO source{component_kind="source" component_id=copy_job_logs component_type=file}:file_server: file_source::file_server: More than one file has the same fingerprint. path="/var/lib/x.log"',
  )
  assert.equal(p.ts, Date.parse("2026-11-12T14:44:54.182Z"))
  assert.equal(p.level, "info")
  assert.equal(p.source, "copy_job_logs")
  assert.equal(p.message, 'More than one file has the same fingerprint. path="/var/lib/x.log"')
  assert.equal(p.fields.component_type, "file")
  assert.equal(p.fields.target, "file_source::file_server")
  assert.equal(p.fields.path, "/var/lib/x.log")
})

test("vector: a line without a span uses the last target segment as source", () => {
  const p = parseLogLine("2026-11-12T14:44:54.100000Z  WARN vector::topology::builder: Sink is not healthy.")
  assert.equal(p.level, "warn")
  assert.equal(p.source, "builder")
  assert.equal(p.message, "Sink is not healthy.")
})

test("connect: Debezium MDC (empty or set), trailing logger, and connector context", () => {
  const rest = parseLogLine(
    '2026-11-12 14:44:51,333 INFO   ||  172.18.0.7 - - [12/Nov/2026:14:44:51 +0000] "GET /connectors HTTP/1.1" 200 181 "-" "-" 0   [org.apache.kafka.connect.runtime.rest.RestServer]',
  )
  assert.equal(rest.ts, Date.parse("2026-11-12T14:44:51.333Z"))
  assert.equal(rest.source, "RestServer")
  assert.equal(rest.fields.logger, "org.apache.kafka.connect.runtime.rest.RestServer")
  assert.match(rest.message, /^172\.18\.0\.7 .* 0$/)

  const dbz = parseLogLine("2026-11-12 14:45:00,001 ERROR  postgres|collabx_sync|streaming  Producer failure   [io.debezium.pipeline.ErrorHandler]")
  assert.equal(dbz.level, "error")
  assert.equal(dbz.source, "collabx_sync")
  assert.equal(dbz.fields.context, "streaming")
  assert.equal(dbz.message, "Producer failure")

  const sink = parseLogLine("2026-11-12 14:45:00,002 WARN   ||  [orders-sink|task-0] Retrying batch   [org.apache.kafka.connect.runtime.WorkerSinkTask]")
  assert.equal(sink.source, "orders-sink")
  assert.equal(sink.fields.task, "task-0")
  assert.equal(sink.message, "Retrying batch")
})

test("kafka (redpanda): 'Failed to create topic' written at WARN stays a warning", () => {
  const p = parseLogLine(
    "WARN  2026-11-12 14:41:32,270 [shard 0:main] kafka - create_topics.cc:159 - Failed to create topic(s) {{kafka/x.public.groups}} error_code observed: cluster::errc::topic_already_exists",
  )
  assert.equal(p.level, "warn")
  assert.equal(p.source, "kafka")
  assert.equal(p.fields.file, "create_topics.cc:159")
  assert.match(p.message, /^Failed to create topic/)
})

test("redis: symbol levels, role as source, UTC timestamp", () => {
  const p = parseLogLine("1:M 11 Nov 2026 22:28:47.666 * Ready to accept connections tcp")
  assert.equal(p.ts, Date.UTC(2026, 10, 11, 22, 28, 47, 666))
  assert.equal(p.level, "info")
  assert.equal(p.source, "master")
  assert.equal(p.message, "Ready to accept connections tcp")
  assert.equal(parseLogLine("1:M 11 Nov 2026 22:28:47.666 # WARNING overcommit_memory").level, "warn")
  assert.equal(parseLogLine("1:C 11 Nov 2026 22:28:47.666 . DB saved").level, "debug")
  const sig = parseLogLine("1:signal-handler (1794408605) Received SIGTERM scheduling shutdown...")
  assert.equal(sig.ts, 1794408605000)
  assert.equal(sig.source, "signal-handler")
  assert.equal(sig.level, "warn")
})

test("stack traces attach to the event above them with its level and source", () => {
  const head = parseLogLine("2026-11-12 14:45:00,001 ERROR  ||  WorkerSinkTask threw   [org.apache.kafka.connect.runtime.WorkerTask]")
  const ex = parseLogLine("org.apache.kafka.connect.errors.ConnectException: Exiting WorkerSinkTask", head)
  const frame = parseLogLine("\tat org.apache.kafka.connect.runtime.WorkerSinkTask.deliver(WorkerSinkTask.java:611)", ex)
  const more = parseLogLine("\t... 10 more", frame)
  for (const p of [ex, frame, more]) {
    assert.equal(p.continuation, true)
    assert.equal(p.level, "error")
    assert.equal(p.source, "WorkerTask")
    assert.equal(p.ts, null)
  }
})

test("generic: words in the message do not set the level; explicit tokens and level= do", () => {
  assert.equal(parseLogLine("2026-11-12T10:00:00Z retry failed, will try again").level, "info")
  assert.equal(parseLogLine("2026-11-12T10:00:00Z [ERROR] boom").level, "error")
  assert.equal(parseLogLine('ts=2026-11-12T10:00:00Z level=warn msg="slow"').level, "warn")
  assert.equal(parseLogLine("plain line with no structure").continuation, false)
  assert.equal(parseLogLine("[cdc-error] docker logs: no such container").level, "error")
})

test("generic: JSON lines", () => {
  const p = parseLogLine('{"time":"2026-11-12T10:00:00Z","level":"error","logger":"api","msg":"db down","code":7}')
  assert.equal(p.level, "error")
  assert.equal(p.source, "api")
  assert.equal(p.message, "db down")
  assert.equal(p.fields.code, "7")
})

test("logfmtFields reads quoted and bare values", () => {
  assert.deepEqual(logfmtFields('a=1 b="x y" c=z'), { a: "1", b: "x y", c: "z" })
})
