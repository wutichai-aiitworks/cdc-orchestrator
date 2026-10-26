import { isAuthDisabled } from "~/utils/authMode"
import { getSessionConfig } from "~/server/utils/sessionConfig"

export default defineEventHandler(async (event) => {
  const cfg = useRuntimeConfig()
  if (isAuthDisabled(cfg)) {
    throw createError({ statusCode: 401, statusMessage: "Not authenticated" })
  }
  const session = await useSession(event, getSessionConfig())
  const data = session.data || {}
  if (!data.userId && !data.email) {
    throw createError({ statusCode: 401, statusMessage: "Not authenticated" })
  }
  return {
    userId: data.userId || "",
    email: data.email || "",
    groups: Array.isArray(data.groups) ? data.groups : [],
    authProvider: data.authProvider || "sso",
  }
})
