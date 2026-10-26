import { orchestratorFetch } from "../../../../utils/orchestratorFetch"
import { buildQuery } from "../../../../utils/query"

export default defineEventHandler(async (event) => {
  const name = encodeURIComponent(getRouterParam(event, "name") || "")
  if (!name) throw createError({ statusCode: 400, statusMessage: "name is required" })
  const q = getQuery(event)
  const query = buildQuery({ limit: q.limit })
  return orchestratorFetch(event, `/cdc/connectors/${name}/timeline${query}`)
})
