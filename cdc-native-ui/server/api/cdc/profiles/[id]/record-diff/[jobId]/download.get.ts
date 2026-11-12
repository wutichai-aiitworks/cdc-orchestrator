// Streams a record-diff result file (NDJSON or CSV) from the orchestrator. $fetch would
// buffer millions of rows in memory, so this uses fetch + a passthrough body. Only the
// orchestrator token is forwarded — never the browser's session cookies.
const KINDS = new Set(["source_only", "target_only", "mismatched"])

export default defineEventHandler(async (event) => {
  const id = encodeURIComponent(getRouterParam(event, "id") || "")
  const jobId = encodeURIComponent(getRouterParam(event, "jobId") || "")
  const q = getQuery(event)
  const kind = String(q.kind || "")
  if (!KINDS.has(kind)) throw createError({ statusCode: 400, statusMessage: "Unknown result kind" })
  const format = q.format === "csv" ? "csv" : "ndjson"
  const config = useRuntimeConfig(event)
  const base = String(config.orchestratorUrl || "http://127.0.0.1:9788").replace(/\/$/, "")
  const headers: Record<string, string> = {}
  if (config.orchestratorToken) headers.Authorization = `Bearer ${config.orchestratorToken}`

  let upstream: Response
  try {
    upstream = await fetch(
      `${base}/cdc/profiles/${id}/record-diff/${jobId}/download?kind=${kind}&format=${format}`,
      { headers },
    )
  } catch {
    throw createError({ statusCode: 502, statusMessage: "The orchestrator is unavailable." })
  }
  if (!upstream.ok || !upstream.body) {
    const data = await upstream.json().catch(() => ({}) as Record<string, unknown>)
    const message =
      (typeof data?.message === "string" && data.message) ||
      (typeof data?.error === "string" && data.error) ||
      "Result file is not available."
    throw createError({ statusCode: upstream.status || 502, statusMessage: message, message })
  }
  for (const name of ["content-type", "content-disposition"]) {
    const value = upstream.headers.get(name)
    if (value) setResponseHeader(event, name, value)
  }
  setResponseHeader(event, "cache-control", "no-store")
  return sendStream(event, upstream.body)
})
