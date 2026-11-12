import { orchestratorFetch } from "../../../../../../utils/orchestratorFetch"

export default defineEventHandler(async (event) => {
  const id = encodeURIComponent(getRouterParam(event, "id") || "")
  const jobId = encodeURIComponent(getRouterParam(event, "jobId") || "")
  await orchestratorFetch(event, `/cdc/profiles/${id}/record-diff/${jobId}`, { method: "DELETE" })
  return { ok: true }
})
