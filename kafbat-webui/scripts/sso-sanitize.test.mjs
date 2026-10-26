import assert from "node:assert/strict"
import { test } from "node:test"

// Keep in sync with utils/sso.ts sanitizeNextPath.
function sanitizeNextPath(raw, fallback = "/") {
  if (typeof raw !== "string") return fallback
  const value = raw.trim()
  if (!value.startsWith("/") || value.startsWith("//") || value.includes("\\")) return fallback
  if (value.startsWith("/api/")) return fallback
  if (value.startsWith("/auth/callback") || value.startsWith("/api/auth/")) return fallback
  if (value === "/auth/login" || value.startsWith("/auth/login?")) return fallback
  if (value === "/login" || value.startsWith("/login?")) return fallback
  if (value.length > 512) return fallback
  return value
}

test("keeps in-app paths", () => {
  assert.equal(sanitizeNextPath("/pipelines"), "/pipelines")
  assert.equal(sanitizeNextPath("/pgcopy?id=1"), "/pgcopy?id=1")
})

test("rejects open redirects and auth loops", () => {
  assert.equal(sanitizeNextPath("https://evil.example"), "/")
  assert.equal(sanitizeNextPath("//evil.example"), "/")
  assert.equal(sanitizeNextPath("/\\evil"), "/")
  assert.equal(sanitizeNextPath("/api/cdc/profiles"), "/")
  assert.equal(sanitizeNextPath("/auth/login"), "/")
  assert.equal(sanitizeNextPath("/auth/callback?ticket=x"), "/")
})
