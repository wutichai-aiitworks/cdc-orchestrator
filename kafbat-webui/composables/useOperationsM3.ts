export type AlertHistoryItem = {
  type?: string
  ts?: string
  connector?: string
  from?: string | null
  to?: string
  rule?: string | null
  resolved?: boolean | null
  taskErrors?: Array<{ taskId?: number; trace?: string }>
  task_errors?: Array<{ taskId?: number; trace?: string }>
}

export type AlertRule = {
  id: string
  name: string
  conditionType: string
  conditionValue?: Record<string, any> | null
  connectorFilter: string
  channels: string[]
  webhookUrl?: string | null
  webhookSecret?: string | null
  enabled: boolean
  createdAt?: string | null
}

export const useOperationsM3 = () => {
  const fetchLogServices = () => $fetch<Record<string, { service?: string; container?: string }>>("/api/cdc/logs")
  const fetchLogs = (service: string, tail = 200) =>
    $fetch<string[]>(`/api/cdc/logs/${encodeURIComponent(service)}`, { query: { tail } })
  const fetchLoggers = () => $fetch<Record<string, any>>("/api/cdc/logs/loggers")
  const updateLogger = (logger: string, level: string) =>
    $fetch(`/api/cdc/logs/loggers/${encodeURIComponent(logger)}`, {
      method: "PUT",
      body: { level },
    })
  const fetchAlertHistory = (limit = 200) =>
    $fetch<AlertHistoryItem[]>("/api/cdc/notifications/history", { query: { limit } })
  const fetchAlertRules = () => $fetch<AlertRule[]>("/api/cdc/notifications/rules")
  const createAlertRule = (body: AlertRule) =>
    $fetch<AlertRule>("/api/cdc/notifications/rules", { method: "POST", body })
  const updateAlertRule = (body: AlertRule) =>
    $fetch<AlertRule>(`/api/cdc/notifications/rules/${encodeURIComponent(body.id)}`, {
      method: "PUT",
      body,
    })
  const deleteAlertRule = (id: string) =>
    $fetch(`/api/cdc/notifications/rules/${encodeURIComponent(id)}`, { method: "DELETE" })
  const fetchMetrics = () => $fetch<string>("/api/cdc/metrics", { responseType: "text" })

  return {
    fetchLogServices,
    fetchLogs,
    fetchLoggers,
    updateLogger,
    fetchAlertHistory,
    fetchAlertRules,
    createAlertRule,
    updateAlertRule,
    deleteAlertRule,
    fetchMetrics,
  }
}
