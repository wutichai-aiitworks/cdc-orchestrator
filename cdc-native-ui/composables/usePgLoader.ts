export type PgLoaderConn = {
  type: string
  host: string
  port: string
  database: string
  username: string
  password: string
  schema?: string
}

export type PgLoaderOptions = {
  drop_if_exists?: boolean
  create_tables?: boolean
  create_indexes?: boolean
  reset_sequences?: boolean
  truncate?: boolean
  workers?: number
  concurrency?: number
  target_table?: string
  csv_delimiter?: string
  csv_skip_header?: number
}

export type PgLoaderProfile = {
  id: string
  name: string
  source: PgLoaderConn
  target: PgLoaderConn
  include_tables: string[]
  options: PgLoaderOptions
  last_job_id?: string | null
  created_at: string
  updated_at: string
}

export type PgLoaderJob = {
  id: string
  name?: string
  status: string
  log_path?: string
  pid?: number
  exit_code?: number
  error?: string
}

export const PGLOADER_SOURCE_TYPES = [
  { value: "mysql", label: "MySQL", port: "3306" },
  { value: "mariadb", label: "MariaDB", port: "3306" },
  { value: "mssql", label: "SQL Server", port: "1433" },
  { value: "sqlite", label: "SQLite file", port: "" },
  { value: "csv", label: "CSV file", port: "" },
] as const

export const defaultPgLoaderOptions = (): PgLoaderOptions => ({
  drop_if_exists: false,
  create_tables: true,
  create_indexes: true,
  reset_sequences: true,
  truncate: false,
  csv_delimiter: ",",
  csv_skip_header: 1,
})

export const usePgLoader = () => {
  const fetchDiscover = () => $fetch<Record<string, unknown>>("/api/cdc/pgloader/discover")
  const fetchContract = () => $fetch<Record<string, unknown>>("/api/cdc/pgloader/contract")
  const fetchCapability = () => $fetch<Record<string, unknown>>("/api/cdc/pgloader/capability")
  const fetchProfiles = () =>
    $fetch<{ profiles: PgLoaderProfile[] }>("/api/cdc/pgloader/profiles").then((r) => r.profiles)
  const createProfile = (body: Omit<PgLoaderProfile, "id" | "created_at" | "updated_at" | "last_job_id">) =>
    $fetch<PgLoaderProfile>("/api/cdc/pgloader/profiles", { method: "POST", body })
  const updateProfile = (id: string, body: Partial<PgLoaderProfile>) =>
    $fetch<PgLoaderProfile>(`/api/cdc/pgloader/profiles/${encodeURIComponent(id)}`, { method: "PUT", body })
  const deleteProfile = (id: string) =>
    $fetch(`/api/cdc/pgloader/profiles/${encodeURIComponent(id)}`, { method: "DELETE" })
  const runProfile = (id: string) =>
    $fetch<PgLoaderJob>(`/api/cdc/pgloader/profiles/${encodeURIComponent(id)}/run`, { method: "POST" })
  const fetchJobs = () => $fetch<{ jobs: PgLoaderJob[] }>("/api/cdc/pgloader/jobs").then((r) => r.jobs)
  const fetchJobLogs = (id: string) =>
    $fetch<{ id: string; log: string }>(`/api/cdc/pgloader/jobs/${encodeURIComponent(id)}/logs`)
  const stopJob = (id: string) =>
    $fetch(`/api/cdc/pgloader/jobs/${encodeURIComponent(id)}/stop`, { method: "POST" })
  const validateConnection = (connection: PgLoaderConn) =>
    $fetch<{ valid: boolean; message: string }>("/api/cdc/pgloader/validate", {
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
    fetchJobLogs,
    stopJob,
    validateConnection,
  }
}
