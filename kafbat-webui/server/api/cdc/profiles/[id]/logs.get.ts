import { orchestratorFetch } from "../../../../utils/orchestratorFetch"
import { buildQuery } from "../../../../utils/query"

export default defineEventHandler((event) => {
  const id = getRouterParam(event, "id")
  return orchestratorFetch(
    event,
    `/cdc/profiles/${encodeURIComponent(id || "")}/logs${buildQuery(getQuery(event))}`,
  )
})
