import { isAuthDisabled } from "~/utils/authMode"

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

  async function logout() {
    const res = await $fetch<{ ok: boolean; redirect?: string }>("/api/auth/logout", { method: "POST" })
    user.value = null
    const redirect = String(res?.redirect || "/").trim()
    if (redirect.startsWith("http")) {
      window.location.href = redirect
      return
    }
    await navigateTo(redirect || "/")
  }

  return {
    user,
    ready,
    loading,
    isAuthenticated,
    refresh,
    loginWithSso,
    loginWithAutheliaCode,
    logout,
  }
}
