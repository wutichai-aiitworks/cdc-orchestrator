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

export const getSessionConfig = () => {
  const config = useRuntimeConfig()
  return {
    password: getSessionSecret(),
    maxAge: 60 * 60 * 8,
    name: "cdc-session",
    cookie: {
      sameSite: "strict" as const,
      httpOnly: true,
      secure: config.sessionCookieSecure as boolean,
    },
  }
}
