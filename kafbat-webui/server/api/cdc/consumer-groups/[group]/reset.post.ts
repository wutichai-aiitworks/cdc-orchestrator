import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const group = encodeURIComponent(getRouterParam(event, "group") || "")
  if (!group) throw createError({ statusCode: 400, statusMessage: "group is required" })
  const body = await readBody(event)
  return orchestratorFetch(event, `/cdc/consumer-groups/${group}/reset`, { method: "POST", body })
})
