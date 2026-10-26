import { buildQuery } from "../../../utils/query"
import { orchestratorFetch } from "../../../utils/orchestratorFetch"

export default defineEventHandler((event) => {
  const subject = encodeURIComponent(getRouterParam(event, "subject") || "")
  if (!subject) throw createError({ statusCode: 400, statusMessage: "subject is required" })
  return orchestratorFetch(event, `/cdc/schemas/${subject}${buildQuery(getQuery(event))}`)
})
