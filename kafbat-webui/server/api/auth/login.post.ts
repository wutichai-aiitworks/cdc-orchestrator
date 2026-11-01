import { createHash, timingSafeEqual } from "node:crypto"
import { getRequestIP } from "h3"
import { isLocalAuth } from "~/utils/authMode"
import { sanitizeNextPath } from "~/utils/sso"
import { getSessionConfig } from "~/server/utils/sessionConfig"

// Standalone login for AUTH_MODE=local. Credentials come from ADMIN_USERNAME /
// ADMIN_PASSWORD (default admin / admin).

const MAX_FAILURES = 10
const WINDOW_MS = 5 * 60 * 1000
const failures = new Map<string, { count: number; resetAt: number }>()

function safeEqual(a: string, b: string): boolean {
  const ha = createHash("sha256").update(a, "utf8").digest()
  const hb = createHash("sha256").update(b, "utf8").digest()
  return timingSafeEqual(ha, hb)
}

function isLockedOut(key: string, now: number): boolean {
  const entry = failures.get(key)
  if (!entry) return false
  if (entry.resetAt <= now) {
    failures.delete(key)
    return false
  }
  return entry.count >= MAX_FAILURES
}

function recordFailure(key: string, now: number) {
  const entry = failures.get(key)
  if (!entry || entry.resetAt <= now) {
    failures.set(key, { count: 1, resetAt: now + WINDOW_MS })
    return
  }
  entry.count += 1
}

export default defineEventHandler(async (event) => {
  const cfg = useRuntimeConfig(event)
  if (!isLocalAuth(cfg)) {
    throw createError({ statusCode: 404, statusMessage: "Local login is not enabled" })
  }

  type LoginBody = { username?: string; password?: string; next?: string }
  const body: LoginBody = (await readBody<LoginBody>(event).catch(() => ({}))) || {}
  const username = String(body.username || "").trim()
  const password = String(body.password || "")
  if (!username || !password) {
    throw createError({ statusCode: 400, statusMessage: "Bad Request", message: "Username and password are required." })
  }

  const now = Date.now()
  // Only honour X-Forwarded-For behind a trusted reverse proxy; otherwise a client
  // could rotate the header to dodge the lockout.
  const key = getRequestIP(event, { xForwardedFor: process.env.TRUST_PROXY === "true" }) || "unknown"
  if (isLockedOut(key, now)) {
    throw createError({
      statusCode: 429,
      statusMessage: "Too Many Requests",
      message: "Too many failed sign-in attempts. Try again in a few minutes.",
    })
  }

  const expectedUser = String(cfg.adminUsername || "admin")
  const expectedPass = String(cfg.adminPassword || "admin")
  // Evaluate both comparisons so timing does not reveal which field was wrong.
  const userOk = safeEqual(username, expectedUser)
  const passOk = safeEqual(password, expectedPass)
  if (!userOk || !passOk) {
    recordFailure(key, now)
    throw createError({ statusCode: 401, statusMessage: "Unauthorized", message: "Invalid username or password." })
  }

  failures.delete(key)
  const session = await useSession(event, getSessionConfig())
  await session.update({
    userId: expectedUser,
    email: "",
    groups: ["admin"],
    createdAt: now,
    authProvider: "local",
  })
  return { ok: true, next: sanitizeNextPath(body.next), username: expectedUser }
})
