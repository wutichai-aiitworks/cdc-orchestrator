import { randomBytes } from "node:crypto"
import { isLocalAuth } from "~/utils/authMode"

// AUTH_MODE=local works out of the box without SESSION_SECRET: fall back to a
// per-process random secret (sessions are dropped on restart). SSO still requires
// an explicit SESSION_SECRET.
const ephemeralSecret = randomBytes(32).toString("hex")

export function getSessionSecret(): string {
  const config = useRuntimeConfig()
  const configured = String(config.sessionSecret || "").trim()
  if (configured.length >= 32) return configured
  return isLocalAuth(config) ? ephemeralSecret : configured
}

// Cookies are scoped by host, not port. The break-glass admin listener (:4321) runs
// beside the SSO UI (:4320) on the same host, so it needs its own cookie name or
// the two instances would overwrite (or, with a shared secret, accept) each other's
// session. Default stays `cdc-session`.
const COOKIE_NAME_RE = /^[A-Za-z0-9_-]{1,64}$/

export function getSessionCookieName(): string {
  const configured = String(useRuntimeConfig().sessionCookieName || "").trim()
  return COOKIE_NAME_RE.test(configured) ? configured : "cdc-session"
}

export const getSessionConfig = () => {
  const config = useRuntimeConfig()
  return {
    password: getSessionSecret(),
    maxAge: 60 * 60 * 8,
    name: getSessionCookieName(),
    cookie: {
      sameSite: "strict" as const,
      httpOnly: true,
      secure: config.sessionCookieSecure as boolean,
    },
  }
}
