import { isAuthDisabled } from "~/utils/authMode"
import { getSessionConfig } from "~/server/utils/sessionConfig"

export default defineEventHandler(async (event) => {
  const path = event.path || ""
  if (!path.startsWith("/api/cdc")) return

  const cfg = useRuntimeConfig()
  if (isAuthDisabled(cfg)) return
  if (String(cfg.sessionSecret || "").trim().length < 32) {
    throw createError({ statusCode: 500, statusMessage: "Session is not configured" })
  }

  const session = await useSession(event, getSessionConfig())
  const data = session.data || {}
  if (!data.userId && !data.email) {
    throw createError({ statusCode: 401, statusMessage: "Not authenticated" })
  }
})
