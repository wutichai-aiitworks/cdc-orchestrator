import { isAuthDisabled, isSsoAuth } from "~/utils/authMode"
import { buildSsoStartPath, sanitizeNextPath } from "~/utils/sso"

export default defineNuxtPlugin({
  name: "auth-guard",
  enforce: "pre",
  setup(nuxtApp) {
    const config = useRuntimeConfig()
    if (isAuthDisabled(config)) return

    let redirecting = false
    let verifyInFlight = false
    let lastVerifyAt = 0
    const VERIFY_COOLDOWN_MS = 60_000

    const isAuthRoutePath = () => {
      if (!import.meta.client) return false
      const path = window.location.pathname || ""
      return path.startsWith("/auth/")
    }

    const redirectToLogin = () => {
      if (redirecting || !import.meta.client) return
      if (isAuthRoutePath()) return
      redirecting = true
      if (isSsoAuth(config)) {
        const next = sanitizeNextPath(window.location.pathname + window.location.search)
        window.location.replace(buildSsoStartPath(next))
        return
      }
      window.location.replace("/auth/login")
    }

    const guarded = $fetch.create({
      onResponseError({ response, request }) {
        const url = String(request)
        if (response.status === 401 && !url.includes("/api/auth/")) {
          redirectToLogin()
        }
      },
    })
    ;(globalThis as any).$fetch = guarded

    nuxtApp.hook("app:mounted", () => {
      const verifySession = () => {
        if (isAuthRoutePath()) return
        if (verifyInFlight) return
        const now = Date.now()
        if (now - lastVerifyAt < VERIFY_COOLDOWN_MS) return
        verifyInFlight = true
        lastVerifyAt = now
        window
          .fetch("/api/auth/me", { credentials: "same-origin" })
          .then((r) => {
            if (r.status === 401) redirectToLogin()
          })
          .catch(() => {})
          .finally(() => {
            verifyInFlight = false
          })
      }

      const onVisibilityChange = () => {
        if (document.visibilityState === "visible") verifySession()
      }
      window.addEventListener("focus", verifySession, { passive: true })
      window.addEventListener("online", verifySession, { passive: true })
      document.addEventListener("visibilitychange", onVisibilityChange, { passive: true })
      verifySession()
      nuxtApp.hook("app:beforeUnmount" as any, () => {
        window.removeEventListener("focus", verifySession)
        window.removeEventListener("online", verifySession)
        document.removeEventListener("visibilitychange", onVisibilityChange)
      })
    })

    nuxtApp.hook("vue:error", (err: unknown) => {
      const status = (err as any)?.statusCode ?? (err as any)?.status ?? (err as any)?.response?.status
      if (status === 401) redirectToLogin()
    })

    return { provide: { apiFetch: guarded } }
  },
})
