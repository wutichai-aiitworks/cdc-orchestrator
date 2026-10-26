import { orchestratorFetch } from "../../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const subject = encodeURIComponent(getRouterParam(event, "subject") || "")
  const version = getRouterParam(event, "version") || ""
  if (!subject || !version) throw createError({ statusCode: 400, statusMessage: "subject and version are required" })
  return orchestratorFetch(event, `/cdc/schemas/${subject}/versions/${version}`, { method: "DELETE" })
})
