import { isAuthDisabled, isLocalAuth, isSsoAuth } from "~/utils/authMode"
import { buildSsoStartPath, buildSupertokensLoginUrl, getSsoProvider } from "~/utils/sso"

export type AuthUser = {
  userId: string
  email: string
  groups: string[]
  authProvider?: string
}

type ExchangeResult =
  | { success: true; next?: string }
  | { success: false; error: string; status?: number }

export function useAuth() {
  const user = useState<AuthUser | null>("cdc-auth-user", () => null)
  const ready = useState<boolean>("cdc-auth-ready", () => false)
  const loading = useState<boolean>("cdc-auth-loading", () => false)
  const cfg = useRuntimeConfig()

  const isAuthenticated = computed(() => Boolean(user.value?.userId || user.value?.email))

  async function refresh() {
    if (isAuthDisabled(cfg)) {
      user.value = null
      ready.value = true
      return
    }
    try {
      user.value = await $fetch<AuthUser>("/api/auth/me")
    } catch {
      user.value = null
    } finally {
      ready.value = true
    }
  }

  async function exchange(body: Record<string, string>): Promise<ExchangeResult> {
    loading.value = true
    try {
      const res = await $fetch<{ ok: boolean; next?: string }>("/api/auth/sso/exchange", {
        method: "POST",
        body,
        timeout: 12000,
      })
      await refresh()
      return { success: true, next: res.next }
    } catch (e: any) {
      return {
        success: false,
        error: e?.data?.message || e?.message || "SSO sign-in failed.",
        status: e?.statusCode ?? e?.status ?? e?.response?.status,
      }
    } finally {
      loading.value = false
    }
  }

  const loginWithSso = (ticket: string) => exchange({ ticket })
  const loginWithAutheliaCode = (code: string, state: string) =>
    exchange({ provider: "authelia", code, state })

  async function loginWithPassword(username: string, password: string, next?: string): Promise<ExchangeResult> {
    loading.value = true
    try {
      const res = await $fetch<{ ok: boolean; next?: string }>("/api/auth/login", {
        method: "POST",
        body: { username, password, next },
        timeout: 12000,
      })
      await refresh()
      return { success: true, next: res.next }
    } catch (e: any) {
      return {
        success: false,
        error: e?.data?.message || e?.message || "Sign-in failed.",
        status: e?.statusCode ?? e?.status ?? e?.response?.status,
      }
    } finally {
      loading.value = false
    }
  }

  async function logout() {
    await $fetch("/api/auth/logout", { method: "POST" })
    user.value = null
    useState<number>("cdc-auth-last-verified").value = 0

    if (import.meta.client && isSsoAuth(cfg)) {
      const provider = getSsoProvider(cfg)
      if (provider === "authelia") {
        window.location.replace(buildSsoStartPath("/", { force_consent: "1" }))
        return
      }
      const shared = buildSupertokensLoginUrl(cfg)
      if (shared) {
        const target = new URL(shared)
        target.searchParams.set("force_login", "1")
        target.searchParams.set("logged_out", "1")
        window.location.replace(target.toString())
        return
      }
      window.location.replace(buildSsoStartPath("/", { force_login: "1" }))
      return
    }
    if (isLocalAuth(cfg)) {
      await navigateTo("/auth/login")
      return
    }
    await navigateTo("/")
  }

  return {
    user,
    ready,
    loading,
    isAuthenticated,
    refresh,
    loginWithSso,
    loginWithAutheliaCode,
    loginWithPassword,
    logout,
  }
}
