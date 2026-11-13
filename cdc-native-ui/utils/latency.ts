/**
 * Pipeline latency display helpers (ADR 0016). Pure: formatting, tone, sparkline geometry.
 * API: GET /cdc/latency, GET /cdc/profiles/{id}/latency?range=, PUT …/latency-probe.
 */

export type LatencyMethod = "heartbeat" | "trigger_inbox" | "estimate"
export type LatencyStatus = "ok" | "behind" | "idle" | "stopped" | "unknown"

export type LatencySample = {
  ts: string
  e2e_ms: number | null
  capture_ms: number | null
  sink_ms: number | null
  lag_records: number | null
  throughput_rps: number | null
  last_event_at: string | null
  method: LatencyMethod
  status: LatencyStatus
  /** Smallest delay the method can tell from zero; e2e null + status ok = below it. */
  resolution_ms?: number | null
  note?: string
}

export type LatencyCurrent = LatencySample & {
  profile_id: string
  name: string
  e2e_p95_1h_ms: number | null
  probe_enabled: boolean
  probe_error?: string
}

export type LatencyPoint = {
  t: string
  e2e_ms: number | null
  e2e_p95_ms?: number | null
  capture_ms: number | null
  sink_ms: number | null
  lag_records: number | null
  throughput_rps: number | null
}

export type LatencyRange = "1h" | "6h" | "24h" | "7d" | "30d"

export type ProfileLatency = {
  profileId: string
  name: string
  range: LatencyRange
  resolution: "sample" | "5m"
  current: LatencyCurrent | null
  probe: { enabled: boolean; intervalSecs: number; unsupportedReason: string | null }
  points: LatencyPoint[]
}

export const formatMs = (ms: number | null | undefined): string => {
  if (ms === null || ms === undefined || Number.isNaN(ms)) return "—"
  if (ms < 1000) return `${Math.round(ms)} ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)} s`
  if (ms < 3_600_000) {
    const m = Math.floor(ms / 60_000)
    const s = Math.round((ms % 60_000) / 1000)
    return s ? `${m}m ${s}s` : `${m} min`
  }
  return `${Math.floor(ms / 3_600_000)}h ${Math.round((ms % 3_600_000) / 60_000)}m`
}

export type Tone = "good" | "warn" | "bad" | "muted"

/** Green under 5 s, amber under 60 s, red above or when the sink is behind. */
export const latencyTone = (c: Pick<LatencySample, "e2e_ms" | "status" | "resolution_ms"> | null | undefined): Tone => {
  if (!c || c.status === "stopped" || c.status === "unknown" || c.status === "idle") return "muted"
  if (c.status === "behind") return "bad"
  const ms = c.e2e_ms
  if (ms === null) return c.status === "ok" && c.resolution_ms ? "good" : "muted"
  return ms < 5_000 ? "good" : ms < 60_000 ? "warn" : "bad"
}

/** Short chip text: the value, or why there is none. */
export const latencyLabel = (c: LatencySample | null | undefined): string => {
  if (!c) return "—"
  if (c.status === "stopped") return "stopped"
  if (c.e2e_ms !== null) return formatMs(c.e2e_ms)
  if (c.status === "ok" && c.resolution_ms) return `< ${formatMs(c.resolution_ms)}`
  if (c.status === "idle") return "idle"
  return "—"
}

export const METHOD_LABEL: Record<LatencyMethod, string> = {
  heartbeat: "Exact (heartbeat)",
  trigger_inbox: "Exact (trigger inbox)",
  estimate: "Estimate",
}

export const METHOD_HELP: Record<LatencyMethod, string> = {
  heartbeat: "A heartbeat row written on the source every few seconds and timed until it lands on the target.",
  trigger_inbox: "Time between the source change and its apply on the target, from the trigger inbox.",
  estimate:
    "From Kafka record times, the source connector's last commit time and the sink's committed offsets. The sink commits every offset-flush interval, so small delays read as 0 and large ones as an upper bound. Turn on the heartbeat probe for exact numbers.",
}

/** Sort key: missing values last in either direction is handled by the caller. */
export const latencySortValue = (c: LatencySample | null | undefined): number | null =>
  c && c.status !== "stopped" ? c.e2e_ms : null

/**
 * SVG polyline points for values over time in a w×h box. Gaps (null) split the line into
 * segments. The y scale starts at 0 and tops out at the max (at least `minMax`).
 */
export const sparkline = (
  points: { t: string; v: number | null }[],
  w: number,
  h: number,
  minMax = 1000,
): { segments: string[]; max: number } => {
  const vals = points.map((p) => p.v).filter((v): v is number => v !== null)
  const max = Math.max(minMax, ...vals)
  if (points.length < 2) return { segments: [], max }
  const t0 = Date.parse(points[0].t)
  const t1 = Date.parse(points[points.length - 1].t)
  const span = Math.max(1, t1 - t0)
  const segments: string[] = []
  let cur: string[] = []
  for (const p of points) {
    if (p.v === null) {
      if (cur.length > 1) segments.push(cur.join(" "))
      cur = []
      continue
    }
    const x = ((Date.parse(p.t) - t0) / span) * w
    const y = h - (p.v / max) * h
    cur.push(`${x.toFixed(1)},${y.toFixed(1)}`)
  }
  if (cur.length > 1) segments.push(cur.join(" "))
  return { segments, max }
}
