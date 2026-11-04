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

export type AuditEventItem = {
  audit_id: string
  timestamp: string
  actor: string
  action: string
  outcome: string
  detail: string
  profile_id?: string | null
  profile_name?: string | null
}

export type AuditEventsResponse = {
  items: AuditEventItem[]
  count: number
  store?: string
  table?: string
}

export type PipelineLogItem = {
  log_id: string
  timestamp: string
  profile_id?: string | null
  profile_name?: string | null
  level: string
  phase: string
  action: string
  outcome: string
  message: string
  detail: string
}

export type PipelineLogsResponse = {
  items: PipelineLogItem[]
  count: number
  store?: string
  table?: string
  profile_id?: string
}

export type TableSyncPhase = "complete" | "in_progress" | "behind" | "unknown" | "error"

export type TableSyncStatusRow = {
  source_schema: string
  source_table: string
  target_schema: string
  target_table: string
  source_qualified: string
  target_qualified: string
  kafka_topic?: string | null
  phase: TableSyncPhase
  source_row_count?: number | null
  target_row_count?: number | null
  pending_rows?: number | null
  kafka_messages?: number | null
  processing_rows?: number | null
  source_offset?: unknown
  /** Incremental rolling-window / full-copy rule; absent = JDBC watermark. */
  poll_rule?: import("~/composables/useProfilesM3").TablePollRule | null
  source_count_error?: string | null
  target_count_error?: string | null
  kafka_error?: string | null
  source_count_diagnosis?: ErrorDiagnosis | null
  target_count_diagnosis?: ErrorDiagnosis | null
  kafka_diagnosis?: ErrorDiagnosis | null
}

/** Orchestrator's answer to "where is the problem and how do I fix it". */
export type ErrorDiagnosis = {
  area:
    | "source_database"
    | "target_database"
    | "network"
    | "kafka_connect"
    | "kafka"
    | "schema_registry"
    | "pipeline_config"
    | "orchestrator"
  area_label: string
  /** Which end of the pipeline the failure belongs to, when it belongs to one. */
  side?: "source" | "target" | null
  title: string
  fix: string
}

export type ConnectorIssue = {
  connector: string
  role: "source" | "sink"
  message: string
  diagnosis: ErrorDiagnosis
}

export type ProfileTableSyncStatusResponse = {
  timestamp: string
  profile_id: string
  profile_name: string
  profile_status: string
  capture_mode: string
  tables: TableSyncStatusRow[]
  counts_skipped: boolean
  /** Profile says Running but a connector/task FAILED or is PAUSED in Kafka Connect. */
  connector_issue?: string | null
  connector_issues?: ConnectorIssue[]
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
  const fetchAuditEvents = (query: {
    limit?: number
    profile_id?: string
    action?: string
    outcome?: string
    since?: string
    until?: string
  }) => $fetch<AuditEventsResponse>("/api/cdc/audit/events", { query })

  const fetchPipelineLogs = (query: {
    limit?: number
    profile_id?: string
    action?: string
    outcome?: string
    since?: string
    until?: string
    level?: string
    phase?: string
  }) => $fetch<PipelineLogsResponse>("/api/cdc/pipeline-logs", { query })

  const fetchProfilePipelineLogs = (
    profileId: string,
    query?: {
      limit?: number
      action?: string
      outcome?: string
      since?: string
      until?: string
      level?: string
      phase?: string
    },
  ) =>
    $fetch<PipelineLogsResponse>(
      `/api/cdc/profiles/${encodeURIComponent(profileId)}/logs`,
      { query },
    )

  const fetchProfileInvestigate = (profileId: string) =>
    $fetch<Record<string, unknown>>(`/api/cdc/profiles/${encodeURIComponent(profileId)}/investigate`)

  const fetchProfileTableSyncStatus = (
    profileId: string,
    query?: { skip_counts?: boolean; skip_source_counts?: boolean; tables?: string },
  ) =>
    $fetch<ProfileTableSyncStatusResponse>(
      `/api/cdc/profiles/${encodeURIComponent(profileId)}/tables/sync-status`,
      { query },
    )

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
    fetchAuditEvents,
    fetchPipelineLogs,
    fetchProfilePipelineLogs,
    fetchProfileInvestigate,
    fetchProfileTableSyncStatus,
  }
}
