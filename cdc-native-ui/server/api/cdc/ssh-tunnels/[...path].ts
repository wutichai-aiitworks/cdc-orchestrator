import { orchestratorFetch } from "~/server/utils/orchestratorFetch"

// SSH tunnels (ADR 0020). One catch-all proxy for the whole /cdc/ssh-tunnels API.
// `list` stands for the bare /cdc/ssh-tunnels (GET list, POST create): a `**` catch-all does not
// match the base path itself.
// Mirrors /cdc/ssh-tunnels, /capability, /test, /:id, /:id/probe, /:id/reconnect.
const ALLOWED = /^(list|capability|test|[0-9a-f-]{36}(\/(probe|reconnect))?)$/i

export default defineEventHandler(async (event) => {
  const path = String(getRouterParam(event, "path") || "").replace(/^\/+|\/+$/g, "")
  if (!ALLOWED.test(path)) throw createError({ statusCode: 404, statusMessage: "Not found" })
  const method = event.method
  const target = path === "list" ? "/cdc/ssh-tunnels" : `/cdc/ssh-tunnels/${path}`
  const body = method === "POST" || method === "PUT" ? await readBody(event).catch(() => undefined) : undefined
  return orchestratorFetch(event, target, { method, ...(body !== undefined ? { body } : {}) })
})
