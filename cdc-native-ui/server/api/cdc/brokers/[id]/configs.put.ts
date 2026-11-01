import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const id = encodeURIComponent(getRouterParam(event, "id") || "")
  if (!id) throw createError({ statusCode: 400, statusMessage: "broker id is required" })
  const body = await readBody(event)
  return orchestratorFetch(event, `/cdc/brokers/${id}/configs`, { method: "PUT", body })
})
