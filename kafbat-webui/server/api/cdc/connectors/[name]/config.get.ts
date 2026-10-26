import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const name = encodeURIComponent(getRouterParam(event, "name") || "")
  if (!name) throw createError({ statusCode: 400, statusMessage: "name is required" })
  return orchestratorFetch(event, `/cdc/connectors/${name}/config`)
})
