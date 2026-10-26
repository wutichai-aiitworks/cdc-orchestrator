import { isAuthDisabled } from "~/utils/authMode"
import { getSessionConfig } from "~/server/utils/sessionConfig"
import { getSsoProvider } from "~/utils/sso"

export default defineEventHandler(async (event) => {
  const cfg = useRuntimeConfig()
  if (isAuthDisabled(cfg)) {
    return { ok: true, redirect: "/" }
  }
  const session = await useSession(event, getSessionConfig())
  await session.clear()

  const provider = getSsoProvider(cfg)
  if (provider === "authelia") {
    const issuer = String(cfg.autheliaTokenUrl || "").trim()
    try {
      const logout = issuer ? new URL("/logout", issuer).toString() : "/"
      return { ok: true, redirect: logout }
    } catch {
      return { ok: true, redirect: "/" }
    }
  }

  const loginUrl = String(cfg.public.ssoLoginUrl || "").trim()
  const appId = String(cfg.public.ssoAppId || "").trim()
  const callbackUrl = String(cfg.public.ssoCallbackUrl || "").trim()
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
