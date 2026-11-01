import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const id = encodeURIComponent(getRouterParam(event, "id") || "")
  if (!id) throw createError({ statusCode: 400, statusMessage: "Rule ID is required" })
  return orchestratorFetch(event, `/cdc/notifications/rules/${id}`, {
    method: "PUT",
    body: await readBody(event),
  })
})
