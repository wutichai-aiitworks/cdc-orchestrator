export type MongoCopyConn = {
  type: string
  host: string
  port: string
  database: string
  username: string
  password: string
  schema?: string
}

export type MongoCopyOptions = {
  drop?: boolean
  gzip?: boolean
  auth_source?: string
}

export type MongoCopyProfile = {
  id: string
  name: string
  source: MongoCopyConn
  target: MongoCopyConn
  include_collections: string[]
  options: MongoCopyOptions
  last_job_id?: string | null
  created_at: string
  updated_at: string
}

export type MongoCopyJob = {
  id: string
  name?: string
  status: string
  log_path?: string
  pid?: number
  exit_code?: number
  error?: string
}

export const defaultMongoCopyOptions = (): MongoCopyOptions => ({
  drop: false,
  gzip: true,
  auth_source: "",
})

export const useMongoCopy = () => {
  const fetchDiscover = () => $fetch<Record<string, unknown>>("/api/cdc/mongocopy/discover")
  const fetchContract = () => $fetch<Record<string, unknown>>("/api/cdc/mongocopy/contract")
  const fetchCapability = () => $fetch<Record<string, unknown>>("/api/cdc/mongocopy/capability")
  const fetchProfiles = () =>
    $fetch<{ profiles: MongoCopyProfile[] }>("/api/cdc/mongocopy/profiles").then((r) => r.profiles)
  const createProfile = (body: Omit<MongoCopyProfile, "id" | "created_at" | "updated_at" | "last_job_id">) =>
    $fetch<MongoCopyProfile>("/api/cdc/mongocopy/profiles", { method: "POST", body })
  const updateProfile = (id: string, body: Partial<MongoCopyProfile>) =>
    $fetch<MongoCopyProfile>(`/api/cdc/mongocopy/profiles/${encodeURIComponent(id)}`, { method: "PUT", body })
  const deleteProfile = (id: string) =>
    $fetch(`/api/cdc/mongocopy/profiles/${encodeURIComponent(id)}`, { method: "DELETE" })
  const runProfile = (id: string) =>
    $fetch<MongoCopyJob>(`/api/cdc/mongocopy/profiles/${encodeURIComponent(id)}/run`, { method: "POST" })
  const fetchJobs = () => $fetch<{ jobs: MongoCopyJob[] }>("/api/cdc/mongocopy/jobs").then((r) => r.jobs)
  const fetchJobLogs = (id: string) =>
    $fetch<{ id: string; log: string }>(`/api/cdc/mongocopy/jobs/${encodeURIComponent(id)}/logs`)
  const stopJob = (id: string) =>
    $fetch(`/api/cdc/mongocopy/jobs/${encodeURIComponent(id)}/stop`, { method: "POST" })
  const validateConnection = (connection: MongoCopyConn) =>
    $fetch<{ valid: boolean; message: string }>("/api/cdc/mongocopy/validate", {
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
