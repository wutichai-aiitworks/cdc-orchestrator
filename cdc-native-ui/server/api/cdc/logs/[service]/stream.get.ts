export default defineEventHandler(async (event) => {
  const config = useRuntimeConfig(event)
  const service = encodeURIComponent(getRouterParam(event, "service") || "")
  if (!service) {
    throw createError({ statusCode: 400, statusMessage: "Log service is required" })
  }

  const requestedTail = Number(getQuery(event).tail || 200)
  const tail = Number.isFinite(requestedTail)
    ? Math.min(5000, Math.max(1, Math.floor(requestedTail)))
    : 200
  const base = String(config.orchestratorUrl || "http://127.0.0.1:9788").replace(/\/$/, "")
  const headers: Record<string, string> = { Accept: "text/event-stream" }
  if (config.orchestratorToken) {
    headers.Authorization = `Bearer ${config.orchestratorToken}`
  }

  const controller = new AbortController()
  const abortUpstream = () => {
    if (!controller.signal.aborted) controller.abort()
  }
  event.node.res.once("close", abortUpstream)

  let response: Response
  try {
    response = await fetch(`${base}/cdc/logs/${service}/stream?tail=${tail}`, {
      headers,
      signal: controller.signal,
    })
  } catch (error: any) {
    if (controller.signal.aborted) {
      event.node.res.removeListener("close", abortUpstream)
      return
    }
    throw createError({
      statusCode: 502,
      statusMessage: "Unable to connect to the orchestrator log stream",
      message: error?.message,
    })
  }

  if (!response.ok || !response.body) {
    const detail = await response.text().catch(() => "")
    throw createError({
      statusCode: response.status || 502,
      statusMessage: detail || "The orchestrator rejected the log stream",
    })
  }

  setHeader(event, "content-type", "text/event-stream; charset=utf-8")
  setHeader(event, "cache-control", "no-cache, no-transform")
  setHeader(event, "connection", "keep-alive")
  setHeader(event, "x-accel-buffering", "no")
  try {
    await sendStream(event, response.body)
  } catch (error: any) {
    const disconnected =
      controller.signal.aborted ||
      event.node.res.destroyed ||
      error?.name === "AbortError" ||
      error?.code === "ABORT_ERR"
    if (!disconnected) throw error
  } finally {
    event.node.res.removeListener("close", abortUpstream)
    abortUpstream()
  }
})
