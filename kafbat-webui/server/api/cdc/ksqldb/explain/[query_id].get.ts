import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler((event) => {
  const query_id = getRouterParam(event, "query_id") || ""
  if (!query_id) throw createError({ statusCode: 400, statusMessage: "query_id is required" })
  return orchestratorFetch(event, `/cdc/ksqldb/explain/${encodeURIComponent(query_id)}`)
})
