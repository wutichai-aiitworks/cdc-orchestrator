import { isAuthDisabled, isSsoAuth } from "~/utils/authMode"
import { buildSsoStartPath, sanitizeNextPath } from "~/utils/sso"
import type { AuthUser } from "~/composables/useAuth"

export default defineNuxtRouteMiddleware(async (to) => {
  const cfg = useRuntimeConfig()
  if (isAuthDisabled(cfg)) return
  if (to.path.startsWith("/auth/")) return

  const user = useState<AuthUser | null>("cdc-auth-user")
  if (user.value) return

  try {
    user.value = await $fetch<AuthUser>("/api/auth/me")
  } catch {
    const next = sanitizeNextPath(to.fullPath)
    if (isSsoAuth(cfg)) return navigateTo(buildSsoStartPath(next), { external: true })
    return navigateTo(`/auth/login?next=${encodeURIComponent(next)}`)
  }
})
