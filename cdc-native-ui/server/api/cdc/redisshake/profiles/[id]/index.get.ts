import { orchestratorFetch } from "~/server/utils/orchestratorFetch"

export default defineEventHandler((event) => {
  const id = getRouterParam(event, "id")
  return orchestratorFetch(event, `/cdc/redisshake/profiles/${encodeURIComponent(String(id))}`)
})
