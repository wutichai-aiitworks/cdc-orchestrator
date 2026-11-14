import { orchestratorFetch } from "../../../utils/orchestratorFetch"

// AI vector sync (ADR 0018): one pass-through for /cdc/vectorsync/** (session auth applies via
// server/middleware/admin-auth.ts). Only known sub-paths and methods are forwarded.
const ALLOWED = /^(capability|sources|test|embedding-models|preview|profiles(\/[0-9a-f-]{36}(\/(pause|resume|rebuild|search|dead-letters))?)?)$/
const METHODS = new Set(["GET", "POST", "PUT", "DELETE"])

export default defineEventHandler(async (event) => {
  const path = String(getRouterParam(event, "path") || "")
  const method = event.method.toUpperCase()
  if (!ALLOWED.test(path) || !METHODS.has(method)) throw createError({ statusCode: 404, statusMessage: "Not found" })
  const query = getQuery(event)
  const body = method === "POST" || method === "PUT" ? await readBody(event).catch(() => ({})) : undefined
  return orchestratorFetch(event, `/cdc/vectorsync/${path}`, { method, query, ...(body !== undefined ? { body: body ?? {} } : {}) })
})
