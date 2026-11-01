import { orchestratorFetch } from "../../../utils/orchestratorFetch"
import { buildQuery } from "../../../utils/query"

export default defineEventHandler((event) => {
  const service = encodeURIComponent(getRouterParam(event, "service") || "")
  if (!service) throw createError({ statusCode: 400, statusMessage: "Log service is required" })
  return orchestratorFetch(event, `/cdc/logs/${service}${buildQuery(getQuery(event))}`)
})
