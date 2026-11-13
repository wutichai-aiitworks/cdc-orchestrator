// Pipeline latency display helpers (cdc-native-ui/utils/latency.ts), ADR 0016.
import test from "node:test"
import assert from "node:assert/strict"

import { formatMs, latencyLabel, latencyTone, sparkline } from "../../cdc-native-ui/utils/latency.ts"

const s = (e2e_ms: number | null, status: any = "ok") => ({
  ts: "2026-11-12T10:00:00Z", e2e_ms, capture_ms: null, sink_ms: null, lag_records: null,
  throughput_rps: null, last_event_at: null, method: "estimate" as const, status,
})

test("formatMs picks a readable unit", () => {
  assert.equal(formatMs(null), "—")
  assert.equal(formatMs(850), "850 ms")
  assert.equal(formatMs(1250), "1.3 s")
  assert.equal(formatMs(42_000), "42 s")
  assert.equal(formatMs(130_000), "2m 10s")
  assert.equal(formatMs(120_000), "2 min")
  assert.equal(formatMs(3_900_000), "1h 5m")
})

test("tone: green < 5 s, amber < 60 s, red above or when behind; no value is muted", () => {
  assert.equal(latencyTone(s(800)), "good")
  assert.equal(latencyTone(s(12_000)), "warn")
  assert.equal(latencyTone(s(90_000)), "bad")
  assert.equal(latencyTone(s(800, "behind")), "bad")
  assert.equal(latencyTone(s(null, "idle")), "muted")
  assert.equal(latencyTone(null), "muted")
})

test("label says why there is no number", () => {
  assert.equal(latencyLabel(s(null, "idle")), "idle")
  assert.equal(latencyLabel(s(500, "stopped")), "stopped")
  assert.equal(latencyLabel(s(500)), "500 ms")
  assert.equal(latencyLabel(undefined), "—")
  assert.equal(latencyLabel({ ...s(null, "ok"), resolution_ms: 60_000 }), "< 1 min", "caught up below the estimate's resolution")
  assert.equal(latencyTone({ ...s(null, "ok"), resolution_ms: 60_000 }), "good")
})

test("sparkline scales to the max and splits at gaps", () => {
  const pts = [
    { t: "2026-11-12T10:00:00Z", v: 0 },
    { t: "2026-11-12T10:00:10Z", v: 2000 },
    { t: "2026-11-12T10:00:20Z", v: null },
    { t: "2026-11-12T10:00:30Z", v: 1000 },
    { t: "2026-11-12T10:00:40Z", v: 1000 },
  ]
  const { segments, max } = sparkline(pts, 100, 20)
  assert.equal(max, 2000)
  assert.deepEqual(segments, ["0.0,20.0 25.0,0.0", "75.0,10.0 100.0,10.0"])
  assert.deepEqual(sparkline(pts.slice(0, 1), 100, 20).segments, [])
})
