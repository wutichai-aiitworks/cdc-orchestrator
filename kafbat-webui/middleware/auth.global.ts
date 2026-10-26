import { isAuthDisabled, isSsoAuth } from "~/utils/authMode"
import { buildSsoStartPath, sanitizeNextPath } from "~/utils/sso"
import type { AuthUser } from "~/composables/useAuth"

const SESSION_TTL_MS = 5 * 60 * 1000

export default defineNuxtRouteMiddleware(async (to) => {
  const cfg = useRuntimeConfig()
  if (isAuthDisabled(cfg)) return
  if (to.path.startsWith("/api/") || to.path.startsWith("/_nuxt/") || to.path.startsWith("/__nuxt")) return
  if (to.path.startsWith("/auth/login") || to.path.startsWith("/auth/callback")) return

  const user = useState<AuthUser | null>("cdc-auth-user")
  const lastVerified = useState<number>("cdc-auth-last-verified", () => 0)
  const now = Date.now()
  if (user.value && now - lastVerified.value < SESSION_TTL_MS) return

  try {
    user.value = await $fetch<AuthUser>("/api/auth/me")
    lastVerified.value = now
  } catch {
    user.value = null
    lastVerified.value = 0
    const next = sanitizeNextPath(to.fullPath || "/")
    if (isSsoAuth(cfg)) return navigateTo(buildSsoStartPath(next), { external: true })
    return navigateTo(`/auth/login?next=${encodeURIComponent(next)}`)
  }
})
