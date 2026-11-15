// SSH tunnels (ADR 0020): "Connection method = SSH tunnel" for pipeline sources/targets and the
// Copy & Sync tools. A connection with `ssh_tunnel_id` keeps the database address as seen from the
// SSH server in `ssh_remote_host` / `ssh_remote_port`; the orchestrator sets host/port to the stable
// cdc-sshtunnel forward when the connection is saved or tested.

export type SshTunnelForward = { remote_host: string; remote_port: number; listen_port: number; created_at?: string }

export type SshTunnelStatus = {
  connected: boolean
  connected_since?: string | null
  last_error?: string
  last_error_at?: string | null
  observed_host_key_fingerprint?: string
  forwards: { remote_host: string; remote_port: number; listen_port: number; listening: boolean; active_connections: number }[]
}

export type SshProxy = { kind: "none" | "cloudflare_access" | string; hostname: string; service_token_id: string; service_token_secret: string }

export type SshTunnel = {
  id: string
  name: string
  host: string
  port?: number | null
  username: string
  auth_method: "key" | "password" | string
  private_key: string
  passphrase: string
  password: string
  host_key_fingerprint: string
  keepalive?: number | null
  proxy: SshProxy
  forwards: SshTunnelForward[]
  notes: string
  created_at: string
  updated_at: string
  forward_host?: string
  used_by?: string[]
  status?: SshTunnelStatus | null
  gateway_error?: string
}

export type SshTestResult = {
  ok: boolean
  ssh_ok?: boolean
  error?: string
  hint?: string
  host_key_fingerprint?: string
  needs_host_key?: boolean
  host_key_mismatch?: boolean
  server_version?: string
  ssh_ms?: number
  dial_ms?: number
  target?: string
}

/** `error` is set when the list could not be loaded (not the same as "not configured"). */
type ListResponse = { configured: boolean; gateway_reachable: boolean; forward_host?: string; tunnels: SshTunnel[]; error?: string }

const cache = ref<ListResponse | null>(null)
let inflight: Promise<ListResponse> | null = null

export const useSshTunnels = () => {
  const fetchTunnels = async (force = false): Promise<ListResponse> => {
    if (cache.value && !force) return cache.value
    inflight ??= $fetch<ListResponse>("/api/cdc/ssh-tunnels/list")
      .then((r) => (cache.value = r))
      .catch((e: any) => {
        // Keep the last good list; never report a failed request as "not configured".
        const message = String(e?.data?.message || e?.statusMessage || e?.message || "Could not load SSH tunnels")
        cache.value = cache.value ? { ...cache.value, error: message } : { configured: true, gateway_reachable: false, tunnels: [], error: message }
        return cache.value
      })
      .finally(() => (inflight = null))
    return inflight
  }
  const createTunnel = (body: Record<string, any>) =>
    $fetch<SshTunnel>("/api/cdc/ssh-tunnels/list", { method: "POST", body }).finally(() => fetchTunnels(true))
  const updateTunnel = (id: string, body: Record<string, any>) =>
    $fetch<SshTunnel>(`/api/cdc/ssh-tunnels/${encodeURIComponent(id)}`, { method: "PUT", body }).finally(() => fetchTunnels(true))
  const deleteTunnel = (id: string) =>
    $fetch(`/api/cdc/ssh-tunnels/${encodeURIComponent(id)}`, { method: "DELETE" }).finally(() => fetchTunnels(true))
  /** SSH login test of a form (saved or not); with host/port also dials the database through it. */
  const testTunnel = (tunnel: Record<string, any>, id?: string | null, host?: string, port?: number | null) =>
    $fetch<SshTestResult>("/api/cdc/ssh-tunnels/test", { method: "POST", body: { tunnel, id: id || undefined, host: host || "", port: port || undefined } })
  const probe = (id: string, host: string, port: number) =>
    $fetch<{ ok: boolean; ms?: number; error?: string; hint?: string; connected?: boolean; target: string }>(
      `/api/cdc/ssh-tunnels/${encodeURIComponent(id)}/probe`,
      { method: "POST", body: { host, port } },
    )
  const reconnect = (id: string) => $fetch(`/api/cdc/ssh-tunnels/${encodeURIComponent(id)}/reconnect`, { method: "POST" })
  const tunnelName = (id?: string | null) => (id ? cache.value?.tunnels.find((t) => t.id === id)?.name || "a deleted tunnel" : "")
  return { tunnels: cache, fetchTunnels, createTunnel, updateTunnel, deleteTunnel, testTunnel, probe, reconnect, tunnelName }
}
