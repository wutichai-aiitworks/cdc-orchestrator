import { orchestratorFetch } from "../../../../../utils/orchestratorFetch"

export default defineEventHandler((event) => {
  const id = encodeURIComponent(getRouterParam(event, "id") || "")
  const action = String(getRouterParam(event, "action") || "")
  if (!id) throw createError({ statusCode: 400, statusMessage: "Profile ID is required" })
  if (!["start", "pause", "resume", "restart", "stop", "resync"].includes(action)) {
    throw createError({ statusCode: 400, statusMessage: "Unsupported profile action" })
  }
  const q = getQuery(event)
  const params = new URLSearchParams()
  if (action === "resync" || action === "start" || action === "restart") {
    if (q.clear_target === "true") params.set("clear_target", "true")
    const rowLimit = String(q.row_limit ?? "").trim()
    if (rowLimit) params.set("row_limit", rowLimit)
  }
  const query = params.toString() ? `?${params.toString()}` : ""
  return orchestratorFetch(event, `/cdc/profiles/${id}/actions/${action}${query}`, {
    method: "POST",
  })
})
