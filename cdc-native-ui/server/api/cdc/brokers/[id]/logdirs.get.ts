import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler((event) => {
  const id = encodeURIComponent(getRouterParam(event, "id") || "")
  if (!id) throw createError({ statusCode: 400, statusMessage: "broker id is required" })
  return orchestratorFetch(event, `/cdc/brokers/${id}/logdirs`)
})
