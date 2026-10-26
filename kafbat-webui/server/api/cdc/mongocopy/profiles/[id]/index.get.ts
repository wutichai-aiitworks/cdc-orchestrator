import { orchestratorFetch } from "../../../../../utils/orchestratorFetch"

export default defineEventHandler((event) => {
  const id = getRouterParam(event, "id")
  return orchestratorFetch(event, `/cdc/mongocopy/profiles/${encodeURIComponent(String(id))}`)
})
