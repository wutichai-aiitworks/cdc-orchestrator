import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const subject = encodeURIComponent(getRouterParam(event, "subject") || "")
  if (!subject) throw createError({ statusCode: 400, statusMessage: "subject is required" })
  const body = await readBody(event)
  return orchestratorFetch(event, `/cdc/schemas/${subject}/compatibility`, { method: "PUT", body })
})
