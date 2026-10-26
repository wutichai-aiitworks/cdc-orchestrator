import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const logger = encodeURIComponent(getRouterParam(event, "logger") || "")
  if (!logger) throw createError({ statusCode: 400, statusMessage: "Logger name is required" })
  return orchestratorFetch(event, `/cdc/logs/loggers/${logger}`, {
    method: "PUT",
    body: await readBody(event),
  })
})
