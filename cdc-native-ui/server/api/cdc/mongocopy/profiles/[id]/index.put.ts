import { orchestratorFetch } from "../../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const id = getRouterParam(event, "id")
  return orchestratorFetch(event, `/cdc/mongocopy/profiles/${encodeURIComponent(String(id))}`, {
    method: "PUT",
    body: await readBody(event),
  })
})
