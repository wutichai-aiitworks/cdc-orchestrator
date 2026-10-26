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
}

export type SourceTableInspect = {
  schema: string
  name: string
  row_count?: number | null
  primary_key: string[]
  columns: SourceTableColumn[]
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
  }
  target?: {
    name?: string
    target_type: DatabaseType
    connection: Record<string, string>
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
}

export type CdcProfile = ProfilePayload & {
  id: string
  enabled: boolean
  status: string
  created_at?: string
  updated_at?: string
  /** Source connector name assigned by the orchestrator (e.g. `cdc-test01`). */
  connector_name?: string | null
  /** Sink connector name assigned by the orchestrator (e.g. `cdc-test01-sink`). */
  sink_connector_name?: string | null
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
  const validateSource = (body: ProfilePayload["source"]) =>
    $fetch<{ valid: boolean; message: string }>("/api/cdc/profiles/validate/source", {
      method: "POST",
      body,
    })
  const validateTarget = (body: NonNullable<ProfilePayload["target"]>) =>
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
  const previewSourceTriggers = (body: {
    source: ProfilePayload["source"]
    tables?: string[]
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
  const runAction = (id: string, action: "start" | "pause" | "resume" | "restart" | "stop") =>
    $fetch<{ profile: CdcProfile }>(
      `/api/cdc/profiles/${encodeURIComponent(id)}/actions/${action}`,
      { method: "POST" }
    )

  /// Replay the pipeline from the start of the source, optionally emptying the target first.
  const resyncProfile = (id: string, clearTarget: boolean) =>
    $fetch<{ profile: CdcProfile }>(
      `/api/cdc/profiles/${encodeURIComponent(id)}/actions/resync`,
      { method: "POST", query: clearTarget ? { clear_target: "true" } : undefined }
    )

  return {
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
    previewSourceTriggers,
    installSourceTriggers,
    sourceTriggerStatus,
    setEnabled,
    runAction,
    resyncProfile,
  }
}
