import type { LatencyCurrent, LatencyRange, ProfileLatency } from "~/utils/latency"

/** Pipeline latency API (ADR 0016). */
export const useLatency = () => {
  const fetchLatency = () =>
    $fetch<{ items: LatencyCurrent[]; timestamp: string }>("/api/cdc/latency")
  const fetchProfileLatency = (id: string, range: LatencyRange = "1h") =>
    $fetch<ProfileLatency>(`/api/cdc/profiles/${encodeURIComponent(id)}/latency`, { query: { range } })
  const setLatencyProbe = (id: string, body: { enabled: boolean; intervalSecs?: number }) =>
    $fetch<{ enabled: boolean; intervalSecs: number; restartRequired: boolean; notes: string[] }>(
      `/api/cdc/profiles/${encodeURIComponent(id)}/latency-probe`,
      { method: "PUT", body },
    )
  return { fetchLatency, fetchProfileLatency, setLatencyProbe }
}
