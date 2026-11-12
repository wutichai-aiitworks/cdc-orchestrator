import { orchestratorFetch } from "../../../../../../utils/orchestratorFetch"

const KINDS = new Set(["source_only", "target_only", "mismatched"])

export default defineEventHandler(async (event) => {
  const id = encodeURIComponent(getRouterParam(event, "id") || "")
  const jobId = encodeURIComponent(getRouterParam(event, "jobId") || "")
  const q = getQuery(event)
  const kind = String(q.kind || "")
  if (!KINDS.has(kind)) throw createError({ statusCode: 400, statusMessage: "Unknown result kind" })
  const offset = Math.max(0, Math.floor(Number(q.offset) || 0))
  const limit = Math.min(500, Math.max(1, Math.floor(Number(q.limit) || 50)))
  return orchestratorFetch(event, `/cdc/profiles/${id}/record-diff/${jobId}/rows`, {
    query: { kind, offset, limit },
  })
})
