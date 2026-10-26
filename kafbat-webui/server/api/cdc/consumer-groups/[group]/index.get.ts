import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler((event) => {
  const group = encodeURIComponent(getRouterParam(event, "group") || "")
  if (!group) throw createError({ statusCode: 400, statusMessage: "group is required" })
  return orchestratorFetch(event, `/cdc/consumer-groups/${group}`)
})
