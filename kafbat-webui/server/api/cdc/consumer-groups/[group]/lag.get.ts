import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const group = encodeURIComponent(getRouterParam(event, "group") || "")
  if (!group) throw createError({ statusCode: 400, statusMessage: "group is required" })
  return orchestratorFetch(event, `/cdc/kafka/groups/${group}/lag`)
})
