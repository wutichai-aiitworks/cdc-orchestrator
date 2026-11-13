import { orchestratorFetch } from "../../../../../utils/orchestratorFetch"

const channelId = (event: Parameters<typeof getRouterParam>[0]) => {
  const id = getRouterParam(event, "id")
  if (!id) throw createError({ statusCode: 400, statusMessage: "Channel ID is required" })
  return encodeURIComponent(id)
}

export default defineEventHandler(async (event) =>
  orchestratorFetch(event, `/cdc/notifications/channels/${channelId(event)}`, { method: "PUT", body: await readBody(event) }),
)
