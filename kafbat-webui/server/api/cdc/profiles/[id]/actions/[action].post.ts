import { orchestratorFetch } from "../../../../../utils/orchestratorFetch"

export default defineEventHandler((event) => {
  const id = encodeURIComponent(getRouterParam(event, "id") || "")
  const action = String(getRouterParam(event, "action") || "")
  if (!id) throw createError({ statusCode: 400, statusMessage: "Profile ID is required" })
  if (!["start", "pause", "resume", "restart", "stop", "resync"].includes(action)) {
    throw createError({ statusCode: 400, statusMessage: "Unsupported profile action" })
  }
  const clearTarget = getQuery(event).clear_target === "true"
  const query = action === "resync" && clearTarget ? "?clear_target=true" : ""
  return orchestratorFetch(event, `/cdc/profiles/${id}/actions/${action}${query}`, {
    method: "POST",
  })
})
