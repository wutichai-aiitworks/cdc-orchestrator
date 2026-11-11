// Nuxt/h3 session config with a configurable cookie name (break-glass admin pattern).
// Every useSession() call in the app must go through getSessionConfig().
import { randomBytes } from "node:crypto"

const ephemeralSecret = randomBytes(32).toString("hex")
const COOKIE_NAME_RE = /^[A-Za-z0-9_-]{1,64}$/
const DEFAULT_COOKIE = "<app>-session"

export function getSessionCookieName(): string {
  const configured = String(useRuntimeConfig().sessionCookieName || "").trim()
  return COOKIE_NAME_RE.test(configured) ? configured : DEFAULT_COOKIE
}

export function getSessionSecret(): string {
  const config = useRuntimeConfig()
  const configured = String(config.sessionSecret || "").trim()
  if (configured.length >= 32) return configured
  // local mode may run without a secret (sessions reset on restart); sso must set one.
  const mode = String(process.env.AUTH_MODE || config.public.authMode || "").toLowerCase()
  return mode === "local" ? ephemeralSecret : configured
}

export const getSessionConfig = () => ({
  password: getSessionSecret(),
  maxAge: 60 * 60 * 8,
  name: getSessionCookieName(),
  cookie: {
    sameSite: "strict" as const,
    httpOnly: true,
    secure: useRuntimeConfig().sessionCookieSecure as boolean,
  },
})

// nuxt.config.ts → runtimeConfig:
//   sessionSecret: env("SESSION_SECRET"),
//   sessionCookieName: env("SESSION_COOKIE_NAME") || "<app>-session",
