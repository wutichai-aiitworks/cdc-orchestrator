export type PgCopyConn = {
  type: string
  host: string
  port: string
  database: string
  username: string
  password: string
  schema?: string
}

export type PgCopyCloneOptions = {
  table_jobs?: number
  index_jobs?: number
  restore_jobs?: number
  large_objects_jobs?: number
  split_tables_larger_than?: number
  split_max_parts?: number
  drop_if_exists?: boolean
  roles?: boolean
  no_role_passwords?: boolean
  no_owner?: boolean
  no_acl?: boolean
  no_comments?: boolean
  no_tablespaces?: boolean
  skip_large_objects?: boolean
  skip_extensions?: boolean
  skip_ext_comments?: boolean
  skip_collations?: boolean
  skip_vacuum?: boolean
  skip_analyze?: boolean
  skip_db_properties?: boolean
  skip_split_by_ctid?: boolean
  fail_fast?: boolean
  restart?: boolean
  resume?: boolean
  not_consistent?: boolean
  follow?: boolean
  create_slot?: boolean
  wal2json_numeric_as_string?: boolean
  use_copy_binary?: boolean
  all_databases?: boolean
  estimate_table_sizes?: boolean
  plugin?: string
  slot_name?: string
  origin?: string
  endpos?: string
  snapshot?: string
  requirements?: string
  filters_file?: string
}

export type PgCopyProfile = {
  id: string
  name: string
  source: PgCopyConn
  target: PgCopyConn
  include_tables: string[]
  options: PgCopyCloneOptions
  last_job_id?: string | null
  created_at: string
  updated_at: string
}

export type PgCopyJob = {
  id: string
  name?: string
  status: string
  log_path?: string
  pid?: number
  exit_code?: number
  error?: string
  created_at?: string
  started_at?: string
  finished_at?: string
}

export const defaultPgCopyOptions = (): PgCopyCloneOptions => ({
  table_jobs: 4,
  index_jobs: 2,
  drop_if_exists: false,
  follow: false,
  create_slot: false,
})

export const usePgCopy = () => {
  const fetchDiscover = () => $fetch<Record<string, unknown>>("/api/cdc/pgcopy/discover")
  const fetchContract = () => $fetch<Record<string, unknown>>("/api/cdc/pgcopy/contract")
  const fetchCapability = () => $fetch<Record<string, unknown>>("/api/cdc/pgcopy/capability")
  const fetchProfiles = () =>
    $fetch<{ profiles: PgCopyProfile[] }>("/api/cdc/pgcopy/profiles").then((r) => r.profiles)
  const createProfile = (body: Omit<PgCopyProfile, "id" | "created_at" | "updated_at" | "last_job_id">) =>
    $fetch<PgCopyProfile>("/api/cdc/pgcopy/profiles", { method: "POST", body })
  const updateProfile = (id: string, body: Partial<PgCopyProfile>) =>
    $fetch<PgCopyProfile>(`/api/cdc/pgcopy/profiles/${encodeURIComponent(id)}`, { method: "PUT", body })
  const deleteProfile = (id: string) =>
    $fetch(`/api/cdc/pgcopy/profiles/${encodeURIComponent(id)}`, { method: "DELETE" })
  const runProfile = (id: string) =>
    $fetch<PgCopyJob>(`/api/cdc/pgcopy/profiles/${encodeURIComponent(id)}/run`, { method: "POST" })
  const fetchJobs = () => $fetch<{ jobs: PgCopyJob[] }>("/api/cdc/pgcopy/jobs").then((r) => r.jobs)
  const fetchJob = (id: string) => $fetch<PgCopyJob>(`/api/cdc/pgcopy/jobs/${encodeURIComponent(id)}`)
  const fetchJobLogs = (id: string) =>
    $fetch<{ id: string; log: string }>(`/api/cdc/pgcopy/jobs/${encodeURIComponent(id)}/logs`)
  const stopJob = (id: string) =>
    $fetch(`/api/cdc/pgcopy/jobs/${encodeURIComponent(id)}/stop`, { method: "POST" })
  const validateConnection = (connection: PgCopyConn) =>
    $fetch<{ valid: boolean; message: string }>("/api/cdc/pgcopy/validate", {
      method: "POST",
      body: { connection },
    })

  return {
    fetchDiscover,
    fetchContract,
    fetchCapability,
    fetchProfiles,
    createProfile,
    updateProfile,
    deleteProfile,
    runProfile,
    fetchJobs,
    fetchJob,
    fetchJobLogs,
    stopJob,
    validateConnection,
  }
}
