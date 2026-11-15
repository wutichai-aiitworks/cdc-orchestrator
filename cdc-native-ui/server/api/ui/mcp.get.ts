// MCP pages: mcp-cdc status and tool catalog from its `GET /info` (non-secret; clients counted by
// scope only). Signed-in users only (server/middleware/admin-auth.ts guards /api/ui/mcp).
// Authenticated with the orchestrator token, which mcp-cdc shares (CDC_ORCHESTRATOR_TOKEN).
// mcp-cdc is optional: when it is not running the page says so instead of failing.
export default defineEventHandler(async (event) => {
  const config = useRuntimeConfig(event)
  const base = String(config.mcpUrl || "http://mcp-cdc:8410").replace(/\/$/, "")
  const headers: Record<string, string> = {}
  if (config.orchestratorToken) headers.Authorization = `Bearer ${config.orchestratorToken}`
  try {
    const info = await $fetch<Record<string, unknown>>(`${base}/info`, { headers, timeout: 5000 })
    return { reachable: true, ...info }
  } catch (error: any) {
    const status = Number(error?.response?.status || 0)
    return {
      reachable: false,
      error:
        status === 401
          ? "mcp-cdc rejected the token: ORCHESTRATOR_TOKEN (UI) and CDC_ORCHESTRATOR_TOKEN (mcp-cdc) must match."
          : status === 404
            ? "This mcp-cdc version has no /info endpoint: rebuild mcp-cdc."
            : "mcp-cdc is not running or not reachable.",
    }
  }
})
