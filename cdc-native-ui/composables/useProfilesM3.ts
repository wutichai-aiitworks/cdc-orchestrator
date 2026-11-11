export type DatabaseType = "postgres" | "mysql" | "mariadb" | "sqlserver" | "oracle" | "db2" | "informix" | "sqlite"
export type PipelinePattern = "source_to_stream" | "source_to_target" | "source_to_lake" | "source_to_olake"
export type TableFormat = "iceberg" | "parquet" | "delta"
export type LakeZone = "bronze" | "silver" | "gold"
export type ChangeMode = "changelog" | "upsert" | "scd2"
export type LakeCatalogType = "jdbc" | "rest" | "hadoop" | "glue" | "nessie"
export type Sensitivity = "public" | "internal" | "confidential" | "restricted"
export type SinkErrorPolicy = "fail_fast" | "dlq"

export type SourceTableColumn = {
  name: string
  data_type: string
  nullable: boolean
  position: number
  primary_key: boolean
  pk_position?: number | null
  unique: boolean
  /** Character length or numeric precision (jdbc-admin COLUMN_SIZE). */
  size?: number | null
  /** Numeric scale (jdbc-admin DECIMAL_DIGITS). */
  scale?: number | null
  default_value?: string | null
  auto_increment?: boolean
}

export type SourceTableIndex = {
  name: string
  unique: boolean
  columns: string[]
  descending?: string[]
  filter?: string | null
  expression?: boolean
}

export type SourceTableInspect = {
  schema: string
  name: string
  row_count?: number | null
  primary_key: string[]
  columns: SourceTableColumn[]
  /** Absent when the inspector does not report indexes (older cdc-jdbc-admin). */
  indexes?: SourceTableIndex[]
  rows: Array<Record<string, unknown>>
  limit: number
}

export type ColumnMapping = {
  source: string
  /** Multiple source columns (for merge/concat/add etc.) — supersedes `source` when present */
  sources?: string[]
  target: string
  data_type: string
  /** Transform function ID from utils/transformFunctions.ts */
  transform?: string | null
  /** Arguments for the transform function */
  transform_args?: Record<string, unknown>
}

/** Rich internal UI model for one column connection (may be multi-source) */
export type ColumnLink = {
  id: string
  /** One or more source column names */
  sources: string[]
  /** Target column name */
  target: string
  /** Transform function ID, or null for a direct 1:1 copy */
  fn: string | null
  /** Args for the transform function */
  fn_args: Record<string, unknown>
}

export type TableMapping = {
  source_schema: string
  source_table: string
  target_schema: string
  target_table: string
  identical: boolean
  columns: ColumnMapping[]
  /** Rich UI-level column links (serialised to columns on save) */
  column_links?: ColumnLink[]
  /**
   * Logical PK columns used by the trigger-apply function for upsert/delete.
   * Leave empty to let the orchestrator discover the real PK from the target catalog.
   * Required when the target table has no PRIMARY KEY constraint.
   * Comma-separated in the UI, serialised as an array to the backend.
   */
  pk_columns?: string[]
}

export type LakeDestination = {
  name?: string
  table_format: TableFormat
  zone: LakeZone
  change_mode: ChangeMode
  catalog_type: LakeCatalogType
  namespace?: string
  warehouse?: string
  connection: Record<string, string>
}

export type GovernanceConfig = {
  domain?: string
  dataset?: string
  sensitivity?: Sensitivity
  industry_pack?: string | null
  pii_fields?: string[]
}

export type ProfilePayload = {
  name: string
  description?: string
  /** Grouping labels; omit on update to keep the saved ones, [] clears them. */
  tags?: string[]
  pattern: PipelinePattern
  stream_platform?: "kafka" | "redpanda" | null
  transform_strategy: "none" | "predefined" | "custom"
  source: {
    source_type: DatabaseType
    connection: Record<string, string>
    schema_mode: "all_schemas" | "specific_schemas"
    table_mode?: "all_tables" | "specific_tables"
    capture_mode?: "logical" | "trigger" | "sync_once" | "incremental"
    schemas: string[]
    tables: string[]
    changelog_retention_hours?: number | null
    timestamp_column?: string | null
    incrementing_column?: string | null
    upsert_key_columns?: string[]
    table_poll?: TablePollRule[]
  }
  target?: {
    name?: string
    target_type: DatabaseType
    connection: Record<string, string>
    /** Catalog schema filter (backend TargetConfig.schemas, optional). */
    schemas?: string[]
  } | null
  lake?: LakeDestination | null
  governance?: GovernanceConfig
  sink_error_policy?: SinkErrorPolicy
  flow: {
    transforms: Array<Record<string, any>>
    schedule: {
      delivery_mode: "realtime" | "micro_batch"
      interval_seconds?: number | null
      full_refresh_minutes?: number | null
    }
    mappings?: TableMapping[]
  }
  /** When true (default), orchestrator restarts this pipeline after boot if it was running. */
  autostart?: boolean
}

/** Incremental capture: how one table is polled (see docs/openapi.yaml TablePollRule). */
export type TablePollStrategy = "watermark" | "rolling_window" | "full_copy"

export type TablePollRule = {
  table: string
  strategy: TablePollStrategy
  window_column?: string | null
  window_days?: number | null
}

export type InitialLoadTable = {
  source: string
  target: string
  state: "pending" | "running" | "done" | "failed" | "skipped"
  rows_copied: number
  error?: string
}

export type SplitProfileResult = {
  original: CdcProfile
  created: CdcProfile
  moved_tables: string[]
  /** Original is running/paused: restart it after starting the new pipeline. */
  original_restart_required: boolean
}

export type ScheduleKind = "poll" | "stream" | "once" | "idle"

/** One row of GET /cdc/schedule (see docs/openapi.yaml ScheduleEntry). */
export type ScheduleEntry = {
  profile_id: string
  name: string
  tags: string[]
  status: string
  enabled: boolean
  pattern: PipelinePattern
  capture_mode: "logical" | "trigger" | "sync_once" | "incremental"
  delivery_mode: "realtime" | "micro_batch"
  kind: ScheduleKind
  interval_ms?: number | null
  interval_source?: "micro_batch" | "default" | null
  last_tick_at?: string | null
  next_tick_at?: string | null
  full_refresh_minutes?: number | null
  last_full_refresh_at?: string | null
  next_full_refresh_at?: string | null
  next_run_at?: string | null
  source_offsets?: string | null
}

export type ScheduleResponse = {
  timestamp: string
  poll_audit_check_secs: number
  refresh_sweep_secs: number
  entries: ScheduleEntry[]
}

export type DuplicateProfileResult = {
  created: CdcProfile
}

export type InitialLoadStatus = {
  profile_id: string
  state: "running" | "succeeded" | "failed" | "cancelled"
  started_at: string
  finished_at?: string
  clear_target: boolean
  actor: string
  error?: string
  rows_copied: number
  tables: InitialLoadTable[]
}

export type CdcProfile = ProfilePayload & {
  id: string
  enabled: boolean
  autostart?: boolean
  status: string
  created_at?: string
  updated_at?: string
  /** Source connector name assigned by the orchestrator (e.g. `cdc-test01`). */
  connector_name?: string | null
  /** Sink connector name assigned by the orchestrator (e.g. `cdc-test01-sink`). */
  sink_connector_name?: string | null
  last_error?: string | null
  /** Capped lifecycle log, oldest first. */
  history?: ProfileHistoryItem[]
}

export type ProfileHistoryItem = {
  timestamp: string
  level: string
  event: string
  detail: string
}

export type ProfileDlqStatus = {
  profile_id: string
  configured_policy: SinkErrorPolicy
  effective_policy: SinkErrorPolicy
  enabled: boolean
  topic?: string | null
  exists: boolean
  messages_count: number
  high_watermark: number
  size_bytes?: number | null
  timestamp: string
}

export const useProfilesM3 = () => {
  const fetchSchedule = () => $fetch<ScheduleResponse>("/api/cdc/schedule")
  const fetchProfiles = async () => {
    const response = await $fetch<{ profiles: CdcProfile[] }>("/api/cdc/profiles")
    return response.profiles || []
  }
  const createProfile = (body: ProfilePayload) =>
    $fetch<{ profile: CdcProfile }>("/api/cdc/profiles", { method: "POST", body })
  const updateProfile = (id: string, body: ProfilePayload) =>
    $fetch<{ profile: CdcProfile }>(`/api/cdc/profiles/${encodeURIComponent(id)}`, {
      method: "PUT",
      body,
    })
  const deleteProfile = (id: string) =>
    $fetch(`/api/cdc/profiles/${encodeURIComponent(id)}`, { method: "DELETE" })
  const fetchDlqStatus = (id: string) =>
    $fetch<ProfileDlqStatus>(`/api/cdc/profiles/${encodeURIComponent(id)}/dlq`)
  // profile_id (optional): the server merges stored credentials for a saved pipeline.
  const validateSource = (body: ProfilePayload["source"] & { profile_id?: string | null }) =>
    $fetch<{ valid: boolean; message: string }>("/api/cdc/profiles/validate/source", {
      method: "POST",
      body,
    })
  const validateTarget = (body: NonNullable<ProfilePayload["target"]> & { profile_id?: string | null }) =>
    $fetch<{ valid: boolean; message: string }>("/api/cdc/profiles/validate/target", {
      method: "POST",
      body,
    })
  const validateLake = (body: NonNullable<ProfilePayload["lake"]>) =>
    $fetch<{ valid: boolean; message: string }>("/api/cdc/profiles/validate/lake", {
      method: "POST",
      body,
    })
  const browseSourceCatalog = (body: ProfilePayload["source"] & { profile_id?: string | null }) =>
    $fetch<{
      reachable: boolean
      message: string
      schemas: Array<{ name: string; table_count: number; captured: boolean }>
      tables: Array<{ schema: string; name: string; captured: boolean; row_count?: number | null; primary_key?: string[] }>
    }>("/api/cdc/profiles/catalog/source", { method: "POST", body })
  const browseTargetCatalog = (body: NonNullable<ProfilePayload["target"]> & { profile_id?: string | null }) =>
    $fetch<{
      reachable: boolean
      message: string
      schemas: Array<{ name: string; table_count: number; captured: boolean }>
      tables: Array<{ schema: string; name: string; captured: boolean; row_count?: number | null; primary_key?: string[] }>
    }>("/api/cdc/profiles/catalog/target", { method: "POST", body })
  const inspectSourceTable = (body: {
    source: ProfilePayload["source"]
    schema: string
    name: string
    limit?: number
    profile_id?: string | null
  }) =>
    $fetch<SourceTableInspect>("/api/cdc/profiles/catalog/source/table", {
      method: "POST",
      body,
    })
  const inspectTargetTable = (body: {
    target: NonNullable<ProfilePayload["target"]>
    schema: string
    name: string
    limit?: number
    profile_id?: string | null
  }) =>
    $fetch<SourceTableInspect>("/api/cdc/profiles/catalog/target/table", {
      method: "POST",
      body,
    })
  const createTargetTables = (body: {
    target: NonNullable<ProfilePayload["target"]>
    statements: string[]
    profile_id?: string | null
  }) =>
    $fetch<{
      status: boolean
      executed: number
      results: Array<{ index: number; ok: boolean; error?: string }>
    }>("/api/cdc/profiles/catalog/target/ddl", { method: "POST", body })
  const previewSourceTriggers = (body: {
    source: ProfilePayload["source"]
    tables?: string[]
    /** Backend TriggerInstallRequest: merges stored credentials for a saved pipeline. */
    profile_id?: string | null
  }) =>
    $fetch<{
      tables: string[]
      change_log: string
      sql: string
      will_execute: boolean
      vendor: string
      message: string
    }>("/api/cdc/profiles/catalog/source/triggers/preview", { method: "POST", body })
  const installSourceTriggers = (body: {
    source: ProfilePayload["source"]
    tables?: string[]
    /** Backend TriggerInstallRequest: merges stored credentials for a saved pipeline. */
    profile_id?: string | null
  }) =>
    $fetch<{
      installed: string[]
      already_present: string[]
      change_log: string
      sql?: string
      executed?: boolean
      message: string
    }>("/api/cdc/profiles/catalog/source/triggers", { method: "POST", body })
  const sourceTriggerStatus = (body: {
    source: ProfilePayload["source"]
    tables?: string[]
    /** Backend TriggerInstallRequest: merges stored credentials for a saved pipeline. */
    profile_id?: string | null
  }) =>
    $fetch<{
      change_log_present: boolean
      tables: Array<{ schema: string; name: string; installed: boolean }>
    }>("/api/cdc/profiles/catalog/source/triggers/status", { method: "POST", body })
  const setEnabled = (id: string, enabled: boolean) =>
    $fetch<{ profile: CdcProfile }>(`/api/cdc/profiles/${encodeURIComponent(id)}/enable`, {
      method: "POST",
      body: { enabled },
    })
  const runAction = (
    id: string,
    action: "start" | "pause" | "resume" | "restart" | "stop",
    opts?: { clearTarget?: boolean; rowLimit?: number | null },
  ) => {
    const query: Record<string, string> = {}
    if (opts?.clearTarget) query.clear_target = "true"
    if (opts?.rowLimit != null && opts.rowLimit > 0) {
      query.row_limit = String(Math.floor(opts.rowLimit))
    }
    return $fetch<{ profile: CdcProfile }>(
      `/api/cdc/profiles/${encodeURIComponent(id)}/actions/${action}`,
      { method: "POST", query: Object.keys(query).length ? query : undefined },
    )
  }

  /// Replay the pipeline from the start of the source, optionally emptying the target first.
  const resyncProfile = (id: string, clearTarget: boolean, rowLimit?: number | null) => {
    const query: Record<string, string> = {}
    if (clearTarget) query.clear_target = "true"
    if (rowLimit != null && rowLimit > 0) query.row_limit = String(Math.floor(rowLimit))
    return $fetch<{ profile: CdcProfile }>(
      `/api/cdc/profiles/${encodeURIComponent(id)}/actions/resync`,
      { method: "POST", query: Object.keys(query).length ? query : undefined },
    )
  }

  const initialLoadPath = (id: string) => `/api/cdc/profiles/${encodeURIComponent(id)}/initial-load`
  /// Start a background streaming copy of every source row (one SELECT per table).
  const startInitialLoad = (id: string, body: { clear_target: boolean; tables: string[] }) =>
    $fetch<InitialLoadStatus>(initialLoadPath(id), { method: "POST", body })
  const getInitialLoad = (id: string) => $fetch<InitialLoadStatus>(initialLoadPath(id))
  const cancelInitialLoad = (id: string) =>
    $fetch<InitialLoadStatus>(initialLoadPath(id), { method: "DELETE" })

  /** Move tables into a new Draft pipeline with the same source and target. */
  const splitProfile = (id: string, body: { name: string; description?: string; tables: string[] }) =>
    $fetch<SplitProfileResult>(`/api/cdc/profiles/${encodeURIComponent(id)}/split`, { method: "POST", body })

  /** Full copy of a pipeline as a new Draft under a new name (original untouched). */
  const duplicateProfile = (id: string, body: { name: string; description?: string }) =>
    $fetch<DuplicateProfileResult>(`/api/cdc/profiles/${encodeURIComponent(id)}/duplicate`, { method: "POST", body })

  return {
    fetchSchedule,
    splitProfile,
    duplicateProfile,
    startInitialLoad,
    getInitialLoad,
    cancelInitialLoad,
    fetchProfiles,
    createProfile,
    updateProfile,
    deleteProfile,
    fetchDlqStatus,
    validateSource,
    validateTarget,
    validateLake,
    browseSourceCatalog,
    browseTargetCatalog,
    inspectSourceTable,
    inspectTargetTable,
    createTargetTables,
    previewSourceTriggers,
    installSourceTriggers,
    sourceTriggerStatus,
    setEnabled,
    runAction,
    resyncProfile,
  }
}
