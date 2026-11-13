import { orchestratorFetch } from "../../../../utils/orchestratorFetch"
import { buildQuery } from "../../../../utils/query"

const profileId = (event: Parameters<typeof getRouterParam>[0]) => {
  const id = getRouterParam(event, "id")
  if (!id) throw createError({ statusCode: 400, statusMessage: "Pipeline ID is required" })
  return encodeURIComponent(id)
}

export default defineEventHandler((event) =>
  orchestratorFetch(event, `/cdc/profiles/${profileId(event)}/latency${buildQuery(getQuery(event))}`),
)
