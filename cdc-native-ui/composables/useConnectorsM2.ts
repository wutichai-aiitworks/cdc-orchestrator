import { formatApiError } from "~/utils/apiError"

type ConnectorMap = Record<
  string,
  {
    info?: {
      name?: string
      config?: Record<string, string>
      tasks?: Array<{ connector?: string; task?: number }>
      type?: string
    }
    status?: {
      name?: string
      connector?: { state?: string; worker_id?: string }
      tasks?: Array<{
        id?: number
        state?: string
        worker_id?: string
        trace?: string
      }>
      type?: string
    }
  }
>

export type ConnectorActionResponse = {
  action: string
  status: string
  message: string
  audit_id: string
  timestamp: string
}

export type ConnectorTemplate = {
  id: string
  name: string
  description: string
  kind?: "source" | "sink"
  database_type?: string
  config: Record<string, string>
}

export type ConnectorValidateResponse = {
  valid: boolean
  normalized_name?: string | null
  errors: Array<{ field: string; message: string }>
  warnings: string[]
  upstream?: unknown
  timestamp: string
}

export type ConnectorTimelineEntry = {
  ts: string
  connector: string
  from?: string | null
  to: string
  rule?: string | null
  task_errors: Array<{ taskId: number; trace: string }>
  resolved?: boolean | null
}

export const useConnectorsM2 = () => {
  const fetchConnectors = async () => {
    let lastErr: unknown = null
    for (let i = 0; i < 3; i += 1) {
      try {
        return await $fetch<{ connectors: ConnectorMap; timestamp: string }>(
          "/api/cdc/connectors"
        )
      } catch (e) {
        lastErr = e
        if (i < 2) await new Promise((resolve) => setTimeout(resolve, 200 * (i + 1)))
      }
    }
    throw lastErr
  }

  const fetchTemplates = () =>
    $fetch<{ templates: ConnectorTemplate[]; timestamp: string }>(
      "/api/cdc/connectors/templates"
    )

  const validateConnector = (payload: {
    name?: string
    config: Record<string, string>
  }) =>
    $fetch<ConnectorValidateResponse>("/api/cdc/connectors/validate", {
      method: "POST",
      body: payload,
    })

  const createConnector = (payload: {
    name: string
    config: Record<string, string>
  }) =>
    $fetch<ConnectorActionResponse>("/api/cdc/connectors", {
      method: "POST",
      body: payload,
    })

  const updateConnectorConfig = (name: string, config: Record<string, string>) =>
    $fetch<ConnectorActionResponse>(`/api/cdc/connectors/${encodeURIComponent(name)}/config`, {
      method: "PUT",
      body: { config },
    })

  const runAction = (
    name: string,
    action: "pause" | "resume" | "restart" | "stop" | "reset-offsets",
  ) =>
    $fetch<ConnectorActionResponse>(`/api/cdc/connectors/${encodeURIComponent(name)}/${action}`, {
      method: "POST",
    })

  const restartTask = (name: string, task: number) =>
    $fetch<ConnectorActionResponse>(
      `/api/cdc/connectors/${encodeURIComponent(name)}/tasks/${task}/restart`,
      {
        method: "POST",
      }
    )

  const fetchConnectorStatus = (name: string) =>
    $fetch<{ name: string; status: any; timestamp: string }>(
      `/api/cdc/connectors/${encodeURIComponent(name)}/status`
    )

  const fetchConnectorTimeline = (name: string, limit = 50) =>
    $fetch<{ connector: string; total: number; items: ConnectorTimelineEntry[]; timestamp: string }>(
      `/api/cdc/connectors/${encodeURIComponent(name)}/timeline?limit=${limit}`
    )

  const fetchConnectorConfig = (name: string) =>
    $fetch<{ name: string; config: Record<string, string>; timestamp: string }>(
      `/api/cdc/connectors/${encodeURIComponent(name)}/config`
    )

  const probeConnector = (payload: { name?: string; config?: Record<string, string> }) =>
    $fetch<{
      valid: boolean
      reachable: boolean
      kind: string
      database_type: string
      database_label: string
      class_name: string
      connection: { host: string; port: string; database: string; username: string; jdbc_url?: string }
      configured_schemas: string[]
      configured_tables: string[]
      message: string
      timestamp: string
    }>("/api/cdc/connectors/probe", { method: "POST", body: payload })

  const catalogConnector = (payload: { name?: string; config?: Record<string, string>; schema?: string }) =>
    $fetch<{
      kind: string
      database_type: string
      database_label: string
      class_name: string
      connection: { host: string; port: string; database: string; username: string; jdbc_url?: string }
      reachable: boolean
      live: boolean
      message: string
      configured_schemas: string[]
      configured_tables: string[]
      schemas: Array<{ name: string; table_count: number; captured: boolean }>
      tables: Array<{ schema: string; name: string; captured: boolean; row_count?: number | null }>
      timestamp: string
    }>("/api/cdc/connectors/catalog", { method: "POST", body: payload })

  const removeConnector = (name: string) =>
    $fetch<ConnectorActionResponse>(`/api/cdc/connectors/${encodeURIComponent(name)}`, {
      method: "DELETE",
    })

  const normalizeApiError = (e: any, fallback: string) => {
    return formatApiError(e, fallback)
  }

  return {
    fetchConnectors,
    fetchTemplates,
    validateConnector,
    createConnector,
    updateConnectorConfig,
    runAction,
    restartTask,
    fetchConnectorStatus,
    fetchConnectorTimeline,
    fetchConnectorConfig,
    probeConnector,
    catalogConnector,
    removeConnector,
    normalizeApiError,
  }
}
