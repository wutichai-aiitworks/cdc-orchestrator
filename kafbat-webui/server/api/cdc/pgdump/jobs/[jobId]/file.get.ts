// Streams the dump file from the orchestrator (which streams it from the cdc-pgdump sidecar).
// $fetch would buffer multi-GB files in memory, so this uses fetch + a passthrough body.
// Only the orchestrator token is forwarded — never the browser's session cookies.
export default defineEventHandler(async (event) => {
  const jobId = String(getRouterParam(event, "jobId") || "")
  const config = useRuntimeConfig(event)
  const base = String(config.orchestratorUrl || "http://127.0.0.1:9788").replace(/\/$/, "")
  const headers: Record<string, string> = {}
  if (config.orchestratorToken) headers.Authorization = `Bearer ${config.orchestratorToken}`

  let upstream: Response
  try {
    upstream = await fetch(`${base}/cdc/pgdump/jobs/${encodeURIComponent(jobId)}/file`, { headers })
  } catch {
    throw createError({ statusCode: 502, statusMessage: "The orchestrator is unavailable." })
  }
  if (!upstream.ok || !upstream.body) {
    const data = await upstream.json().catch(() => ({}) as Record<string, unknown>)
    const message = typeof data?.error === "string" ? data.error : "Dump file is not available."
    throw createError({ statusCode: upstream.status || 502, statusMessage: message, message })
  }
  for (const name of ["content-type", "content-length", "content-disposition", "x-dump-sha256"]) {
    const value = upstream.headers.get(name)
    if (value) setResponseHeader(event, name, value)
  }
  setResponseHeader(event, "cache-control", "no-store")
  return sendStream(event, upstream.body)
})
