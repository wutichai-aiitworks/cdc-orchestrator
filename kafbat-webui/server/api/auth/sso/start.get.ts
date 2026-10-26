import { createHash, randomBytes } from "node:crypto"
import { getQuery, getRequestProtocol, sendRedirect, setCookie } from "h3"
import { sanitizeNextPath } from "~/utils/sso"

const AUTHELIA_PKCE_COOKIE = "cdc_atla_pkce"

function base64Url(input: Buffer): string {
  return input.toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "")
}

function randomBase64Url(length = 32): string {
  return base64Url(randomBytes(length))
}

export default defineEventHandler((event) => {
  const cfg = useRuntimeConfig(event)
  const provider = String(cfg.public.ssoProvider || "supertokens_gateway").trim().toLowerCase()
  const next = sanitizeNextPath(String(getQuery(event).next || "/"))
  const forceLoginRaw = String(getQuery(event).force_login || "").trim().toLowerCase()
  const forceLogin = forceLoginRaw === "1" || forceLoginRaw === "true" || forceLoginRaw === "yes"
  const forceConsentRaw = String(getQuery(event).force_consent || "").trim().toLowerCase()
  const forceConsent = forceConsentRaw === "1" || forceConsentRaw === "true" || forceConsentRaw === "yes"

  if (provider === "authelia") {
    const authorizeUrl = String(cfg.public.autheliaAuthorizeUrl || "").trim()
    const callbackUrl = String(cfg.public.ssoCallbackUrl || "").trim()
    const clientId = String(cfg.public.autheliaClientId || "").trim()
    const scope = String(cfg.public.autheliaScopes || "openid profile email groups").trim()
    if (!authorizeUrl || !callbackUrl || !clientId) {
      throw createError({ statusCode: 500, statusMessage: "authelia is not configured correctly" })
    }

    const state = randomBase64Url(24)
    const codeVerifier = randomBase64Url(48)
    const codeChallenge = base64Url(createHash("sha256").update(codeVerifier).digest())
    const payload = Buffer.from(JSON.stringify({ state, codeVerifier, next }), "utf8").toString("base64url")
    const proto = getRequestProtocol(event, { xForwardedProto: true })
    setCookie(event, AUTHELIA_PKCE_COOKIE, payload, {
      httpOnly: true,
      sameSite: "lax",
      secure: proto === "https",
      path: "/",
      maxAge: 60 * 10,
    })

    const auth = new URL(authorizeUrl)
    auth.searchParams.set("client_id", clientId)
    auth.searchParams.set("redirect_uri", callbackUrl)
    auth.searchParams.set("response_type", "code")
    auth.searchParams.set("scope", scope)
    auth.searchParams.set("state", state)
    auth.searchParams.set("code_challenge", codeChallenge)
    auth.searchParams.set("code_challenge_method", "S256")
    if (forceLogin) {
      auth.searchParams.set("prompt", "login")
      auth.searchParams.set("max_age", "0")
    } else if (forceConsent) {
      auth.searchParams.set("prompt", "consent")
    }
    return sendRedirect(event, auth.toString(), 302)
  }

  const appId = String(cfg.public.ssoAppId || "").trim()
  const loginUrl = String(cfg.public.ssoLoginUrl || "").trim()
  const callbackUrl = String(cfg.public.ssoCallbackUrl || "").trim()
  if (!appId || !loginUrl || !callbackUrl) {
    throw createError({ statusCode: 500, statusMessage: "sso is not configured correctly" })
  }
  const returnUrl = new URL(callbackUrl)
  returnUrl.searchParams.set("next", next)
  const shared = new URL(loginUrl)
  shared.searchParams.set("app_id", appId)
  shared.searchParams.set("return_url", returnUrl.toString())
  if (forceLogin) {
    shared.searchParams.set("force_login", "1")
    shared.searchParams.set("logged_out", "1")
  }
  return sendRedirect(event, shared.toString(), 302)
})
