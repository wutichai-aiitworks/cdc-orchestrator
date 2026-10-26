export type RedisShakeConn = {
  type: string
  host: string
  port: string
  database: string
  username: string
  password: string
  schema?: string
}

export type RedisShakeSyncOptions = {
  source_cluster?: boolean
  target_cluster?: boolean
  source_tls?: boolean
  target_tls?: boolean
  sync_rdb?: boolean
  sync_aof?: boolean
  prefer_replica?: boolean
  try_diskless?: boolean
  off_reply?: boolean
  rdb_restore_command_behavior?: string
}

export type RedisShakeProfile = {
  id: string
  name: string
  source: RedisShakeConn
  target: RedisShakeConn
  options: RedisShakeSyncOptions
  last_job_id?: string | null
  created_at: string
  updated_at: string
}

export type RedisShakeJob = {
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

export const defaultRedisShakeOptions = (): RedisShakeSyncOptions => ({
  sync_rdb: true,
  sync_aof: true,
  rdb_restore_command_behavior: "panic",
})

export const useRedisShake = () => {
  const fetchDiscover = () => $fetch<Record<string, unknown>>("/api/cdc/redisshake/discover")
  const fetchContract = () => $fetch<Record<string, unknown>>("/api/cdc/redisshake/contract")
  const fetchCapability = () => $fetch<Record<string, unknown>>("/api/cdc/redisshake/capability")
  const fetchProfiles = () =>
    $fetch<{ profiles: RedisShakeProfile[] }>("/api/cdc/redisshake/profiles").then((r) => r.profiles)
  const createProfile = (
    body: Omit<RedisShakeProfile, "id" | "created_at" | "updated_at" | "last_job_id">,
  ) => $fetch<RedisShakeProfile>("/api/cdc/redisshake/profiles", { method: "POST", body })
  const updateProfile = (id: string, body: Partial<RedisShakeProfile>) =>
    $fetch<RedisShakeProfile>(`/api/cdc/redisshake/profiles/${encodeURIComponent(id)}`, {
      method: "PUT",
      body,
    })
  const deleteProfile = (id: string) =>
    $fetch(`/api/cdc/redisshake/profiles/${encodeURIComponent(id)}`, { method: "DELETE" })
  const runProfile = (id: string) =>
    $fetch<RedisShakeJob>(`/api/cdc/redisshake/profiles/${encodeURIComponent(id)}/run`, {
      method: "POST",
    })
  const fetchJobs = () =>
    $fetch<{ jobs: RedisShakeJob[] }>("/api/cdc/redisshake/jobs").then((r) => r.jobs)
  const fetchJobLogs = (id: string) =>
    $fetch<{ id: string; log: string }>(`/api/cdc/redisshake/jobs/${encodeURIComponent(id)}/logs`)
  const stopJob = (id: string) =>
    $fetch(`/api/cdc/redisshake/jobs/${encodeURIComponent(id)}/stop`, { method: "POST" })
  const validateConnection = (connection: RedisShakeConn) =>
    $fetch<{ valid: boolean; message: string }>("/api/cdc/redisshake/validate", {
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
