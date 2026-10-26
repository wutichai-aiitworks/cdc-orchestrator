import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const name = encodeURIComponent(getRouterParam(event, "name") || "")
  const action = String(getRouterParam(event, "action") || "").trim()
  if (!name) throw createError({ statusCode: 400, statusMessage: "name is required" })
  if (!["pause", "resume", "restart", "stop", "reset-offsets"].includes(action)) {
    throw createError({ statusCode: 400, statusMessage: "unsupported action" })
  }
  return orchestratorFetch(event, `/cdc/connectors/${name}/${action}`, {
    method: "POST",
  })
})
