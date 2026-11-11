export type RsyncAuth = "key" | "password"
export type RsyncTransport = "auto" | "direct" | "relay"

/** One SSH host (Linux or macOS) and the directory on it. */
export type RsyncProtocol = "ssh" | "daemon"

export type RsyncEndpoint = {
  /** ssh (default): SSH login. daemon: rsync://host:873/module — no shell, relay only, use a tunnel. */
  protocol?: RsyncProtocol
  /** Daemon only: module name (a [name] in the site's rsyncd.conf). */
  module?: string
  name: string
  host: string
  port: string
  username: string
  auth: RsyncAuth
  password: string
  private_key: string
  passphrase: string
  path: string
  host_key_fingerprint: string
  rsync_path: string
  /** WireGuard tunnel to the host's site (ADR 0010); forces the relay transport. */
  tunnel_id?: string | null
  /** Target only: address the source uses to reach it in direct mode. */
  reach_host: string
  reach_port: string
}

export type RsyncOptions = {
  transport?: RsyncTransport
  delete?: boolean
  max_delete?: number | null
  dry_run?: boolean
  excludes?: string[]
  checksum?: boolean
  compress?: boolean
  no_owner?: boolean
  hard_links?: boolean
  acls?: boolean
  xattrs?: boolean
  numeric_ids?: boolean
  one_file_system?: boolean
  bwlimit_kbps?: number | null
  io_timeout_seconds?: number | null
  discard_staging?: boolean
}

export type RsyncProfile = {
  id: string
  name: string
  source: RsyncEndpoint
  target: RsyncEndpoint
  options: RsyncOptions
  last_job_id?: string | null
  created_at: string
  updated_at: string
}

export type RsyncHostFacts = {
  os?: string
  arch?: string
  rsync_path?: string
  rsync_version?: string
  gnu_rsync?: boolean
  path_kind?: string
  free_bytes?: number
}

export type RsyncChange = { op: "create" | "update" | "delete" | "attrs"; kind: string; path: string }

export type RsyncJob = {
  id: string
  name?: string
  profile_id?: string | null
  status: "starting" | "running" | "succeeded" | "completed_with_errors" | "failed" | "stopped" | string
  phase?: string
  source?: string
  target?: string
  created_at?: string
  started_at?: string
  finished_at?: string
  transport_requested?: RsyncTransport
  transport?: "direct" | "relay"
  transport_reason?: string
  dry_run?: boolean
  delete?: boolean
  progress?: { bytes: number; percent: number; rate: string; eta: string; files_transferred?: number; files_to_check?: number; files_total?: number } | null
  stats?: Record<string, number>
  stats_pull?: Record<string, number>
  changes?: { counts: Record<string, number>; items: RsyncChange[]; truncated: boolean }
  hosts?: { source?: RsyncHostFacts; target?: RsyncHostFacts }
  host_keys?: Record<string, { fingerprint: string; verified: boolean; seen_from?: string }>
  staging_discarded?: boolean
  exit_code?: number | null
  error?: string
  first_error?: string
  errors?: string[]
  hint?: string
  last_message?: string
}

/** POST /cdc/rsync/test-connection (logs in from the sidecar). */
export type RsyncTestResult = RsyncHostFacts & {
  ok: boolean
  target: string
  side?: string
  latency_ms?: number
  host_key_fingerprint?: string
  host_key_verified?: boolean
  writable?: boolean
  ssh_client?: boolean
  warning?: string
  error?: string
  hint?: string | null
}

/** POST /cdc/rsync/test-route: would a run go direct? */
export type RsyncRouteResult = {
  direct: boolean
  transport: "direct" | "relay"
  target?: RsyncHostFacts
  target_host_key_fingerprint?: string
  warning?: string | null
  error?: string
  hint?: string | null
}

/** Marker the API returns instead of a stored secret; sending it back means "unchanged". */
export const RSYNC_REDACTED = "********"

export const emptyEndpoint = (): RsyncEndpoint => ({
  protocol: "ssh",
  module: "",
  name: "",
  host: "",
  port: "22",
  username: "",
  auth: "key",
  password: "",
  private_key: "",
  passphrase: "",
  path: "",
  host_key_fingerprint: "",
  rsync_path: "",
  reach_host: "",
  reach_port: "",
})

export const defaultRsyncOptions = (): RsyncOptions => ({
  transport: "auto",
  delete: false,
  max_delete: null,
  dry_run: false,
  excludes: [],
  checksum: false,
  compress: false,
  no_owner: false,
  hard_links: false,
  acls: false,
  xattrs: false,
  numeric_ids: false,
  one_file_system: false,
  bwlimit_kbps: null,
  io_timeout_seconds: null,
  discard_staging: false,
})

const enc = encodeURIComponent

export const useRsync = () => {
  const fetchCapability = () => $fetch<Record<string, unknown>>("/api/cdc/rsync/capability")
  const fetchProfiles = () => $fetch<{ profiles: RsyncProfile[] }>("/api/cdc/rsync/profiles").then((r) => r.profiles)
  const createProfile = (body: any) => $fetch<RsyncProfile>("/api/cdc/rsync/profiles", { method: "POST", body })
  const updateProfile = (id: string, body: any) => $fetch<RsyncProfile>(`/api/cdc/rsync/profiles/${enc(id)}`, { method: "PUT", body })
  const deleteProfile = (id: string) => $fetch(`/api/cdc/rsync/profiles/${enc(id)}`, { method: "DELETE" })
  const runProfile = (id: string) => $fetch<RsyncJob>(`/api/cdc/rsync/profiles/${enc(id)}/run`, { method: "POST" })
  const previewProfile = (id: string) =>
    $fetch<RsyncJob>(`/api/cdc/rsync/profiles/${enc(id)}/run`, { method: "POST", query: { dry_run: "true" } })
  const fetchJobs = () => $fetch<{ jobs: RsyncJob[] }>("/api/cdc/rsync/jobs").then((r) => r.jobs)
  const fetchJobLogs = (id: string) => $fetch<{ id: string; log: string }>(`/api/cdc/rsync/jobs/${enc(id)}/logs`)
  const stopJob = (id: string) => $fetch(`/api/cdc/rsync/jobs/${enc(id)}/stop`, { method: "POST" })
  const deleteJob = (id: string) => $fetch(`/api/cdc/rsync/jobs/${enc(id)}`, { method: "DELETE" })
  const testConnection = (body: { endpoint: RsyncEndpoint; side: "source" | "target"; profile_id?: string }) =>
    $fetch<RsyncTestResult>("/api/cdc/rsync/test-connection", { method: "POST", body })
  const testRoute = (body: { source: RsyncEndpoint; target: RsyncEndpoint; profile_id?: string }) =>
    $fetch<RsyncRouteResult>("/api/cdc/rsync/test-route", { method: "POST", body })

  return {
    fetchCapability,
    fetchProfiles,
    createProfile,
    updateProfile,
    deleteProfile,
    runProfile,
    previewProfile,
    fetchJobs,
    fetchJobLogs,
    stopJob,
    deleteJob,
    testConnection,
    testRoute,
  }
}
