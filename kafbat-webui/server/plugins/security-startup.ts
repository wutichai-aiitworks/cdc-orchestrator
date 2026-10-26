/**
 * Production fail-closed checks. Local/dev may run with AUTH_MODE=none.
 * Production must not stay open unless ALLOW_OPEN_AUTH=true.
 */

const WEAK_SESSION_SECRETS = new Set([
  "change-me-to-strong-random-secret",
  "change-me",
])

function isProduction(): boolean {
  return process.env.NODE_ENV === "production"
}

function isBlank(value: unknown): boolean {
  return typeof value !== "string" || value.trim().length === 0
}

export default defineNitroPlugin(() => {
  if (!isProduction()) return

  const config = useRuntimeConfig()
  const errors: string[] = []
  const authMode = String(process.env.AUTH_MODE || config.public.authMode || "none").trim().toLowerCase()
  const allowOpenAuth = process.env.ALLOW_OPEN_AUTH === "true"

  if ((authMode === "none" || authMode === "disabled" || authMode === "off") && !allowOpenAuth) {
    errors.push("AUTH_MODE=none is not allowed in production unless ALLOW_OPEN_AUTH=true.")
  }

  if (authMode === "sso") {
    const sessionSecret = String(process.env.SESSION_SECRET || config.sessionSecret || "")
    if (isBlank(sessionSecret) || sessionSecret.trim().length < 32 || WEAK_SESSION_SECRETS.has(sessionSecret.trim())) {
      errors.push("SESSION_SECRET must be at least 32 random characters when AUTH_MODE=sso.")
    }
    const provider = String(process.env.SSO_PROVIDER || config.public.ssoProvider || "supertokens_gateway").trim().toLowerCase()
    if (provider === "authelia") {
      if (isBlank(process.env.AUTHELIA_AUTHORIZE_URL || config.public.autheliaAuthorizeUrl) || isBlank(process.env.AUTHELIA_CLIENT_ID || config.public.autheliaClientId) || isBlank(process.env.SSO_CALLBACK_URL || config.public.ssoCallbackUrl)) {
        errors.push("Authelia SSO requires AUTHELIA_AUTHORIZE_URL, AUTHELIA_CLIENT_ID, and SSO_CALLBACK_URL.")
      }
    } else if (
      isBlank(process.env.SSO_APP_ID || config.public.ssoAppId) ||
      isBlank(process.env.SSO_LOGIN_URL || config.public.ssoLoginUrl) ||
      isBlank(process.env.SSO_CALLBACK_URL || config.public.ssoCallbackUrl) ||
      isBlank(process.env.SSO_EXCHANGE_URL || config.ssoExchangeUrl)
    ) {
      errors.push("SuperTokens SSO requires SSO_APP_ID, SSO_LOGIN_URL, SSO_CALLBACK_URL, and SSO_EXCHANGE_URL.")
    }
  }

  if (errors.length > 0) {
    const message = `[security-startup] Refusing to start:\n- ${errors.join("\n- ")}`
    console.error(message)
    throw new Error(message)
  }
})
