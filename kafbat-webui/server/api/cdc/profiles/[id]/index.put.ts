import { orchestratorFetch } from "../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const id = encodeURIComponent(getRouterParam(event, "id") || "")
  if (!id) throw createError({ statusCode: 400, statusMessage: "Profile ID is required" })
  return orchestratorFetch(event, `/cdc/profiles/${id}`, {
    method: "PUT",
    body: await readBody(event),
  })
})
