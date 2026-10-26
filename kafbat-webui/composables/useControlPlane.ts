export type EngineService = {
  name: string
  state: string
  health?: string | null
}

export type EngineStatus = {
  engine_target: string
  services: EngineService[]
  timestamp: string
}

export type ActionResult = {
  action: string
  status: string
  message: string
  audit_id: string
  timestamp: string
}

export const useControlPlane = () => {
  const fetchEngineStatus = () => $fetch<EngineStatus>("/api/cdc/engine/status")
  const fetchCluster = () => $fetch<Record<string, any>>("/api/cdc/cluster")
  const runEngineAction = (action: "up" | "down" | "restart") =>
    $fetch<ActionResult>(`/api/cdc/engine/${action}`, { method: "POST" })

  return { fetchEngineStatus, fetchCluster, runEngineAction }
}
