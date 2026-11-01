import { isAuthDisabled, isLocalAuth } from "~/utils/authMode"
import { getSessionConfig } from "~/server/utils/sessionConfig"
import { getSsoProvider } from "~/utils/sso"

function envOr(name: string, fallback: unknown): string {
  return String(process.env[name] || fallback || "").trim()
}

export default defineEventHandler(async (event) => {
  // Public SSO URLs are overlaid per request. The global config is the build-time snapshot.
  const cfg = useRuntimeConfig(event)
  if (isAuthDisabled(cfg)) {
    return { ok: true, redirect: "/" }
  }
  const session = await useSession(event, getSessionConfig())
  await session.clear()

  if (isLocalAuth(cfg)) {
    return { ok: true, redirect: "/auth/login" }
  }

  const provider = getSsoProvider(cfg)
  if (provider === "authelia") {
    const issuer = envOr("AUTHELIA_TOKEN_URL", cfg.autheliaTokenUrl)
    try {
      const logout = issuer ? new URL("/logout", issuer).toString() : "/"
      return { ok: true, redirect: logout }
    } catch {
      return { ok: true, redirect: "/" }
    }
  }

  const loginUrl = envOr("SSO_LOGIN_URL", cfg.public.ssoLoginUrl)
  const appId = envOr("SSO_APP_ID", cfg.public.ssoAppId)
  const callbackUrl = envOr("SSO_CALLBACK_URL", cfg.public.ssoCallbackUrl)
  if (loginUrl && appId && callbackUrl) {
    const shared = new URL(loginUrl)
    shared.searchParams.set("app_id", appId)
    shared.searchParams.set("return_url", callbackUrl)
    shared.searchParams.set("force_login", "1")
    shared.searchParams.set("logged_out", "1")
    return { ok: true, redirect: shared.toString() }
  }
  return { ok: true, redirect: "/" }
})
