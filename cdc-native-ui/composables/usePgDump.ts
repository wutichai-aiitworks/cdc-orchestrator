export type PgDumpConn = {
  type: string
  host: string
  port: string
  database: string
  username: string
  password: string
  schema?: string
}

export type PgDumpMode = "dump" | "dump_restore"

export type PgDumpOptions = {
  format?: "custom" | "plain" | "tar"
  compress?: number | null
  schema_only?: boolean
  data_only?: boolean
  schemas?: string[]
  exclude_schemas?: string[]
  exclude_tables?: string[]
  exclude_table_data?: string[]
  no_owner?: boolean
  no_privileges?: boolean
  no_comments?: boolean
  no_publications?: boolean
  no_subscriptions?: boolean
  clean?: boolean
  if_exists?: boolean
  create?: boolean
  serializable_deferrable?: boolean
  lock_wait_timeout_ms?: number | null
  source_sslmode?: string
  target_sslmode?: string
  restore_jobs?: number | null
  restore_clean?: boolean
  restore_if_exists?: boolean
  restore_no_owner?: boolean
  restore_no_privileges?: boolean
  restore_single_transaction?: boolean
  restore_exit_on_error?: boolean
  keep_local?: boolean
}

export type PgDumpDestination = {
  name: string
  host: string
  port: string
  username: string
  auth: "key" | "password"
  password: string
  private_key: string
  passphrase: string
  remote_dir: string
  host_key_fingerprint: string
}

export type PgDumpUpload = {
  name: string
  host: string
  port: number
  username: string
  remote_dir: string
  host_key_pinned?: boolean
  status: "pending" | "uploading" | "uploaded" | "failed" | "stopped" | string
  bytes_sent?: number
  bytes?: number
  remote_path?: string
  host_key_fingerprint?: string
  host_key_verified?: boolean
  seconds?: number
  error?: string
  hint?: string
  /** Sent after the run with "Send via SFTP" (not a saved server). */
  adhoc?: boolean
  started_at?: string
  finished_at?: string
}

export const emptyDestination = (): PgDumpDestination => ({
  name: "",
  host: "",
  port: "22",
  username: "",
  auth: "key",
  password: "",
  private_key: "",
  passphrase: "",
  remote_dir: "",
  host_key_fingerprint: "",
})

export type PgDumpProfile = {
  id: string
  name: string
  mode: PgDumpMode
  source: PgDumpConn
  target?: PgDumpConn | null
  include_tables: string[]
  options: PgDumpOptions
  destinations?: PgDumpDestination[]
  last_job_id?: string | null
  created_at: string
  updated_at: string
}

export type PgDumpJob = {
  id: string
  name?: string
  profile_id?: string | null
  mode?: PgDumpMode
  status: "starting" | "running" | "succeeded" | "completed_with_errors" | "failed" | "stopped" | string
  phase?: string
  source?: string
  target?: string | null
  created_at?: string
  started_at?: string
  finished_at?: string
  tables_dumped?: number
  tables_restored?: number
  errors_ignored?: number
  artifact?: { name: string; format: string; bytes: number; sha256?: string; local_deleted?: boolean } | null
  uploads?: PgDumpUpload[]
  download_path?: string
  /** An ad-hoc SFTP send of the file is in progress (its entry is the last of `uploads`). */
  sending?: boolean
  exit_code?: number | null
  error?: string
  first_error?: string
  hint?: string
  last_message?: string
}

export const defaultPgDumpOptions = (): PgDumpOptions => ({
  format: "custom",
  compress: null,
  schema_only: false,
  data_only: false,
  schemas: [],
  exclude_schemas: [],
  exclude_tables: [],
  exclude_table_data: [],
  no_owner: false,
  no_privileges: false,
  no_comments: false,
  no_publications: false,
  no_subscriptions: false,
  clean: false,
  if_exists: false,
  create: false,
  serializable_deferrable: false,
  lock_wait_timeout_ms: null,
  restore_jobs: null,
  restore_clean: false,
  restore_if_exists: false,
  restore_no_owner: false,
  restore_no_privileges: false,
  restore_single_transaction: false,
  restore_exit_on_error: false,
  keep_local: false,
})

export type PgDumpSchemaInfo = { name: string; tables: number; bytes: number }

/** Answer of POST /cdc/pgdump/test-connection (run from the sidecar, where pg_dump runs). */
export type PgDumpTestResult = {
  ok: boolean
  target: string
  latency_ms?: number
  server_version?: string
  server_major?: number | null
  client_version?: string
  client_major?: number | null
  compatible?: boolean
  current_user?: string
  database_bytes?: number
  ssl?: boolean
  schemas?: PgDumpSchemaInfo[]
  error?: string
  hint?: string | null
}

/** Marker the API returns instead of a stored secret; sending it back means "unchanged". */
export const PGDUMP_REDACTED = "********"

/** Browser URL that streams a finished job's dump file through the UI server. */
export const pgDumpFileUrl = (jobId: string) => `/api/cdc/pgdump/jobs/${encodeURIComponent(jobId)}/file`

export const usePgDump = () => {
  const fetchDiscover = () => $fetch<Record<string, unknown>>("/api/cdc/pgdump/discover")
  const fetchContract = () => $fetch<Record<string, unknown>>("/api/cdc/pgdump/contract")
  const fetchCapability = () => $fetch<Record<string, unknown>>("/api/cdc/pgdump/capability")
  const fetchProfiles = () =>
    $fetch<{ profiles: PgDumpProfile[] }>("/api/cdc/pgdump/profiles").then((r) => r.profiles)
  const createProfile = (body: Omit<PgDumpProfile, "id" | "created_at" | "updated_at" | "last_job_id">) =>
    $fetch<PgDumpProfile>("/api/cdc/pgdump/profiles", { method: "POST", body })
  const updateProfile = (id: string, body: Partial<PgDumpProfile>) =>
    $fetch<PgDumpProfile>(`/api/cdc/pgdump/profiles/${encodeURIComponent(id)}`, { method: "PUT", body })
  const deleteProfile = (id: string) =>
    $fetch(`/api/cdc/pgdump/profiles/${encodeURIComponent(id)}`, { method: "DELETE" })
  const runProfile = (id: string) =>
    $fetch<PgDumpJob>(`/api/cdc/pgdump/profiles/${encodeURIComponent(id)}/run`, { method: "POST" })
  const fetchJobs = () => $fetch<{ jobs: PgDumpJob[] }>("/api/cdc/pgdump/jobs").then((r) => r.jobs)
  const fetchJobLogs = (id: string) =>
    $fetch<{ id: string; log: string }>(`/api/cdc/pgdump/jobs/${encodeURIComponent(id)}/logs`)
  const stopJob = (id: string) => $fetch(`/api/cdc/pgdump/jobs/${encodeURIComponent(id)}/stop`, { method: "POST" })
  const fetchJob = (id: string) => $fetch<PgDumpJob>(`/api/cdc/pgdump/jobs/${encodeURIComponent(id)}`)
  /** Upload a finished job's file to one SFTP server. `profile_id` lets a saved server's `********` secrets be reused (same login only). */
  const sendFile = (id: string, body: { destination: PgDumpDestination; profile_id?: string }) =>
    $fetch<PgDumpJob>(`/api/cdc/pgdump/jobs/${encodeURIComponent(id)}/send`, { method: "POST", body })
  const deleteJob = (id: string) => $fetch(`/api/cdc/pgdump/jobs/${encodeURIComponent(id)}`, { method: "DELETE" })
  const validateConnection = (connection: PgDumpConn) =>
    $fetch<{ valid: boolean; message: string }>("/api/cdc/pgdump/validate", {
      method: "POST",
      body: { connection },
    })

  const testConnection = (body: {
    connection: PgDumpConn
    sslmode?: string
    profile_id?: string
    side?: "source" | "target"
  }) => $fetch<PgDumpTestResult>("/api/cdc/pgdump/test-connection", { method: "POST", body })

  return {
    testConnection,
    fetchDiscover,
    fetchContract,
    fetchCapability,
    fetchProfiles,
    createProfile,
    updateProfile,
    deleteProfile,
    runProfile,
    fetchJobs,
    fetchJobLogs,
    stopJob,
    fetchJob,
    sendFile,
    deleteJob,
    validateConnection,
  }
}
