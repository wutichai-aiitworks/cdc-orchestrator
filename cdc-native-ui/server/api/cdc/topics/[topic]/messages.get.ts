import { buildQuery } from "../../../../utils/query"
import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const topic = encodeURIComponent(getRouterParam(event, "topic") || "")
  if (!topic) throw createError({ statusCode: 400, statusMessage: "topic is required" })
  const q = getQuery(event)
  return orchestratorFetch(event, `/cdc/topics/${topic}/messages${buildQuery(q)}`)
})
