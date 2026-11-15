// MCP pages: mcp-cdc status and tool catalog (GET /api/ui/mcp → mcp-cdc /info). Non-secret: the
// server never returns keys, tokens or client names, only counts by scope.

export type McpTool = { name: string; description: string; read_only: boolean; input_schema?: Record<string, any> }
export type McpPrompt = { name: string; description: string; arguments?: unknown[] }

export type McpInfo = {
  reachable: boolean
  error?: string
  service?: string
  version?: string
  endpoint_path?: string
  write_tools_enabled?: boolean
  clients?: { read: number; write: number }
  active_sessions?: number
  max_sessions?: number
  session_ttl_secs?: number
  rate_limit?: { enabled: boolean; requests_per_minute: number }
  oauth?: { enabled: boolean; public_url?: string | null }
  orchestrator_url?: string
  tools?: McpTool[]
  prompts?: McpPrompt[]
}

const cache = ref<McpInfo | null>(null)
let inflight: Promise<McpInfo> | null = null

export const useMcp = () => {
  const fetchInfo = async (force = false): Promise<McpInfo> => {
    if (cache.value && !force) return cache.value
    inflight ??= $fetch<McpInfo>("/api/ui/mcp")
      .then((r) => (cache.value = r))
      .catch(() => (cache.value = { reachable: false, error: "Could not reach the UI server." }))
      .finally(() => (inflight = null))
    return inflight
  }
  /** The address chat clients use: the public URL when set, else the local port. */
  const endpointUrl = computed(() => {
    const pub = cache.value?.oauth?.public_url?.replace(/\/$/, "")
    return `${pub || "http://127.0.0.1:8410"}${cache.value?.endpoint_path || "/mcp"}`
  })
  return { info: cache, fetchInfo, endpointUrl }
}
