import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const topic = encodeURIComponent(getRouterParam(event, "topic") || "")
  if (!topic) throw createError({ statusCode: 400, statusMessage: "topic is required" })
  return orchestratorFetch(event, `/cdc/topics/${topic}/clear-messages`, { method: "POST" })
})
