import { isAuthDisabled } from "~/utils/authMode"
import { getSessionConfig, getSessionSecret } from "~/server/utils/sessionConfig"

export default defineEventHandler(async (event) => {
  const path = event.path || ""
  // /api/ui/mcp: MCP status page data (counts and tool catalog, no secrets, but internal).
  if (!path.startsWith("/api/cdc") && !path.startsWith("/api/ui/mcp")) return

  const cfg = useRuntimeConfig()
  if (isAuthDisabled(cfg)) return
  if (getSessionSecret().length < 32) {
    throw createError({ statusCode: 500, statusMessage: "Session is not configured" })
  }

  const session = await useSession(event, getSessionConfig())
  const data = session.data || {}
  if (!data.userId && !data.email) {
    throw createError({ statusCode: 401, statusMessage: "Not authenticated" })
  }
})
