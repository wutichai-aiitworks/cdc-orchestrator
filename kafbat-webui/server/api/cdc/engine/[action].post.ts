import { orchestratorFetch } from "../../../utils/orchestratorFetch"

export default defineEventHandler((event) => {
  const action = String(getRouterParam(event, "action") || "")
  if (!["up", "down", "restart"].includes(action)) {
    throw createError({ statusCode: 400, statusMessage: "Unsupported engine action" })
  }
  return orchestratorFetch(event, `/cdc/engine/${action}`, { method: "POST" })
})
